//! SSH options for Midnight Commander's `sh://` VFS.
//!
//! mc's shell filesystem shells out to `ssh` with a fixed argument list —
//! `ssh -p <port> -l <user> <host> "echo SHELL:; /bin/sh"` — and its own
//! `sh://[user@]host[:options]` syntax only understands compression, `rsh` and
//! a port number. There is no way to hand it `-J`, `-i` or any `-o`, so every
//! SSH setting a connection carries used to be dropped on the mc path: a
//! jump host most visibly (issue #255), but also `HostKeyAlias`, custom
//! options and the identity file.
//!
//! The one injection point mc leaves open is `$PATH`: it invokes `ssh` by name.
//! This module writes a per-session directory holding
//!
//! * `config` — a generated `ssh_config` with the connection's real settings,
//! * `ssh` — a two-line wrapper that `exec`s the real `ssh` with `-F config`,
//!
//! and the caller prepends that directory to `PATH` for the mc child process.
//! The generated config ends with `Match all` + `Include ~/.ssh/config` so the
//! user's own aliases keep working; `ssh_config` is first-match-wins, so the
//! blocks written here still take precedence for the hosts they name.
//!
//! The same mechanism previously existed in `sftp::ensure_flatpak_mc_ssh_wrapper`
//! but only inside Flatpak and only to supply a writable `known_hosts`.
//!
//! # Password delivery
//!
//! mc gives `ssh` no terminal it can answer a password prompt on (it reads the
//! remote shell's output and sends `ssh`'s stderr to `/dev/null`), so a
//! password-authenticated connection used to stop at `shell: Waiting for
//! initial line...`. When the caller passes the connection's password,
//! [`prepare_mc_ssh_env`] writes it to an owner-only file
//! ([`crate::ssh_askpass::create_secret_file`]) and the wrapper hands that file
//! to the prompt-aware askpass helper — but only for the one `ssh` that has
//! mc's exact argument shape (`[-C] [-p PORT] [-l USER] HOST COMMAND`) for this
//! connection's own host, user and port. Any other invocation (another `sh://`
//! link opened in the same mc, an `ssh` with `-J` or `-o`) runs exactly as
//! without delivery. The password and its path never enter mc's environment;
//! only the wrapper script knows the path, and only the delivering `ssh` gets
//! it. Saved jump hosts are then reached through
//! [`crate::jump_route::tunnel_hop_args`], whose hops start without the helper,
//! so no bastion can be sent the target's password (issue #191).

use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use secrecy::SecretString;
use uuid::Uuid;

use crate::connection::JumpChain;
use crate::models::{Connection, ConnectionGroup, ProtocolConfig, SshAuthMethod, SshConfig};
use crate::ssh_tunnel::shell_single_quote;

/// Name of the directory holding all per-session wrapper directories.
const WRAPPER_ROOT: &str = "rustconn-mc-ssh";

/// Age after which an orphaned session directory is removed.
///
/// `XDG_RUNTIME_DIR` is cleared when the session ends, so this only matters on
/// the `TMPDIR` fallback. Twelve hours is comfortably longer than any mc tab
/// anyone leaves open within a login session, and short enough that stale
/// directories do not pile up.
const STALE_AFTER: Duration = Duration::from_hours(12);

/// A prepared `ssh` wrapper for one mc session.
#[derive(Debug, Clone)]
pub struct McSshEnv {
    /// Directory containing the `ssh` wrapper; prepend it to `PATH`.
    wrapper_dir: PathBuf,
    /// The generated `ssh_config` the wrapper passes via `-F`.
    config_path: PathBuf,
    /// The owner-only file holding the connection's password, when the
    /// wrapper delivers one. Holds a path only, never the secret.
    secret_file: Option<PathBuf>,
}

impl McSshEnv {
    /// Whether the wrapper hands the connection's password to mc's `ssh`.
    #[must_use]
    pub fn delivers_password(&self) -> bool {
        self.secret_file.is_some()
    }

    /// The password file the caller must remove when mc exits.
    #[must_use]
    pub fn secret_file(&self) -> Option<&Path> {
        self.secret_file.as_deref()
    }

    /// Returns the `PATH=…` entry to pass to the mc child process.
    ///
    /// Prepends the wrapper directory to the inherited `PATH` so mc's `ssh`
    /// resolves to the wrapper and everything else still resolves normally.
    #[must_use]
    pub fn path_env(&self) -> String {
        let inherited = std::env::var("PATH").unwrap_or_default();
        format!("PATH={}:{inherited}", self.wrapper_dir.display())
    }

    /// Returns the generated `ssh_config` path (useful for logging and tests).
    #[must_use]
    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    /// Returns the wrapper directory.
    #[must_use]
    pub fn wrapper_dir(&self) -> &Path {
        &self.wrapper_dir
    }
}

/// Returns the base directory for wrapper directories.
///
/// `XDG_RUNTIME_DIR` is preferred: it is user-private, on tmpfs, mounted
/// executable (the existing jump-host askpass helper already runs from there)
/// and cleared at logout.
fn wrapper_root() -> PathBuf {
    let base = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|d| !d.is_empty())
        .map_or_else(std::env::temp_dir, PathBuf::from);
    base.join(WRAPPER_ROOT)
}

/// Strips `user@` and `:port` from a hop spec, leaving the bare hostname.
///
/// `ssh_config` `Host` patterns match the name ssh was asked to connect to, so
/// a hop block has to be keyed on the host alone.
///
/// An unbracketed address holding several colons is taken as an IPv6 literal
/// rather than `host:port`, which is the same disambiguation OpenSSH applies:
/// a port after an IPv6 address requires brackets (`[::1]:2222`).
fn hop_hostname(spec: &str) -> &str {
    let host_port = spec.rfind('@').map_or(spec, |at| &spec[at + 1..]);

    if let Some(rest) = host_port.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest);
    }
    if host_port.matches(':').count() > 1 {
        return host_port;
    }
    match host_port.rfind(':') {
        Some(colon)
            if !host_port[colon + 1..].is_empty()
                && host_port[colon + 1..].bytes().all(|b| b.is_ascii_digit()) =>
        {
            &host_port[..colon]
        }
        _ => host_port,
    }
}

/// Whether mc's `ssh` for `connection` will ask for the account password.
///
/// SSH-family, authentication method Password or Keyboard Interactive, and no
/// identity file resolved through the group chain. Key and agent connections
/// return `false`; their mc launch never reads a password.
#[must_use]
pub fn expects_password(connection: &Connection, groups: &[ConnectionGroup]) -> bool {
    ssh_config(connection).is_some_and(|cfg| {
        matches!(
            cfg.auth_method,
            SshAuthMethod::Password | SshAuthMethod::KeyboardInteractive
        )
    }) && identity_file(connection, groups).is_none()
}

/// Whether the wrapper may hand `connection`'s password to mc's `ssh` at all.
///
/// [`expects_password`], and the cheap half of the SSH session's gate
/// ([`crate::ssh_tunnel::target_password_askpass_allowed`]): Keyboard
/// Interactive, a PKCS#11 token, a custom routing or authentication option and
/// an explicit `ProxyCommand` all refuse. Spawns nothing, so a caller can ask
/// it before reading the vault; [`prepare_mc_ssh_env`] adds the route checks.
#[must_use]
pub fn password_may_be_sent(connection: &Connection, groups: &[ConnectionGroup]) -> bool {
    expects_password(connection, groups)
        && ssh_config(connection)
            .is_some_and(|cfg| crate::ssh_tunnel::target_password_askpass_allowed(cfg, false))
}

/// The arguments the delivering `ssh` gets in front of mc's own, or `None`
/// when the password must not be sent along this route.
///
/// With no jump chain, `declares_proxy` (normally
/// [`crate::ssh_tunnel::ssh_config_declares_proxy`]) must report no proxy in
/// the user's ssh configuration. A chain goes through
/// [`crate::jump_route::tunnel_hop_args`], whose `ProxyCommand` starts every
/// hop behind the `env` boundary that clears the helper, and outranks the
/// generated config's `ProxyJump`. Under `ProxyJump` the hops past the first
/// got their keys from the generated config; the nested `ProxyCommand` cannot
/// pass them, so a chain where one of those hops has its own key, or where the
/// first hop's key path is not plain, keeps today's route and gets no password.
fn password_route_args(
    connection: &Connection,
    cfg: &SshConfig,
    chain: &JumpChain,
    connections: &[Connection],
    groups: &[ConnectionGroup],
    known_hosts: Option<&Path>,
    declares_proxy: impl FnOnce(&str, u16, Option<&str>) -> bool,
) -> Option<Vec<String>> {
    if !crate::ssh_tunnel::target_password_askpass_allowed(cfg, false) {
        return None;
    }
    let mut args = if chain.hops.is_empty() {
        Vec::new()
    } else {
        let hop_key = |index: usize| {
            chain
                .hop_ids
                .get(index)
                .copied()
                .flatten()
                .and_then(|id| connections.iter().find(|c| c.id == id))
                .and_then(|hop| identity_file(hop, groups))
        };
        if (1..chain.hops.len()).any(|index| hop_key(index).is_some()) {
            return None;
        }
        let first_key = hop_key(0).map(|key| key.to_string_lossy().into_owned());
        if first_key
            .as_deref()
            .is_some_and(|key| crate::jump_route::plain_path(key).is_none())
        {
            return None;
        }
        crate::jump_route::tunnel_hop_args(&crate::jump_route::TunnelHopsRequest {
            hops: &chain.hops,
            first_hop_identity: first_key.as_deref(),
            deeper_hop_identity: None,
            first_hop_pkcs11: None,
            known_hosts,
            hop_host_keys: crate::ssh_tunnel::HopHostKeys::UserConfig,
        })?
    };
    if !crate::ssh_tunnel::askpass_stays_on_destination(
        &connection.host,
        connection.port,
        connection.username.as_deref(),
        &args,
        declares_proxy,
    ) {
        return None;
    }
    args.push("-o".to_string());
    args.push("NumberOfPasswordPrompts=1".to_string());
    // A first-seen host is accepted, as for an SSH session with a delivered
    // password, unless the connection sets its own policy.
    if !cfg
        .custom_options
        .keys()
        .any(|key| key.trim().eq_ignore_ascii_case("StrictHostKeyChecking"))
    {
        args.push("-o".to_string());
        args.push("StrictHostKeyChecking=accept-new".to_string());
    }
    Some(args)
}

/// What the wrapper needs to hand the password to mc's own `ssh`.
struct Delivery<'a> {
    /// The owner-only file holding the password.
    secret_file: &'a Path,
    /// The prompt-aware askpass helper for the target slot.
    helper: &'a Path,
    /// The connection's host, user (empty for none) and port, which mc's
    /// argv must name exactly.
    host: &'a str,
    user: &'a str,
    port: u16,
    /// The route arguments from [`password_route_args`].
    args: &'a [String],
    /// Where the delivering `ssh`'s stderr goes; mc sends it to `/dev/null`.
    stderr_file: &'a Path,
    /// Created when the delivering `ssh` reports a refused sign-in.
    failure_marker: &'a Path,
}

/// The `ssh` wrapper script for mc.
///
/// Without `delivery` this is the two-line `exec` wrapper. With it, the script
/// first parses its argv against the shape mc uses (`[-C] [-p PORT] [-l USER]
/// HOST COMMAND`); only an exact match for the delivery's host, user and port
/// runs `ssh` with the askpass environment on its own command line. Every
/// embedded value is single-quoted; the password itself is never in the text.
fn wrapper_script(real_ssh: &Path, config: &Path, delivery: Option<&Delivery<'_>>) -> String {
    let ssh = shell_single_quote(&real_ssh.to_string_lossy());
    let config = shell_single_quote(&config.to_string_lossy());
    let mut script = String::from(
        "#!/bin/sh\n\
         # Auto-generated by RustConn so mc's sh:// VFS picks up this\n\
         # connection's SSH options. See rustconn-core/src/mc_ssh.rs.\n",
    );
    if let Some(delivery) = delivery {
        let route = delivery.args.iter().fold(String::new(), |mut route, arg| {
            let _ = write!(route, " {}", shell_single_quote(arg));
            route
        });
        let _ = write!(
            script,
            "# The saved password goes only to mc's own ssh for this connection.\n\
             # OpenSSH keeps the first -p/-l and parses options after HOST too,\n\
             # so a repeat or any option after HOST runs without the password.\n\
             _rc_host= _rc_user= _rc_port=22 _rc_words=0 _rc_ok=1 _rc_want= _rc_seen=\n\
             for _rc_arg do\n\
             \x20 if [ -n \"$_rc_want\" ]; then\n\
             \x20   if [ \"$_rc_want\" = p ]; then _rc_port=$_rc_arg; else _rc_user=$_rc_arg; fi\n\
             \x20   _rc_want=\n\
             \x20 elif [ \"$_rc_words\" -eq 0 ]; then\n\
             \x20   case $_rc_arg in\n\
             \x20     -C) ;;\n\
             \x20     -p|-l)\n\
             \x20       _rc_want=${{_rc_arg#-}}\n\
             \x20       case $_rc_seen in *$_rc_want*) _rc_ok=0 ;; esac\n\
             \x20       _rc_seen=$_rc_seen$_rc_want ;;\n\
             \x20     -*) _rc_ok=0 ;;\n\
             \x20     *) _rc_host=$_rc_arg; _rc_words=1 ;;\n\
             \x20   esac\n\
             \x20 else\n\
             \x20   case $_rc_arg in -*) _rc_ok=0 ;; esac\n\
             \x20   _rc_words=$((_rc_words + 1))\n\
             \x20 fi\n\
             done\n\
             if [ \"$_rc_ok\" = 1 ] && [ -z \"$_rc_want\" ] && [ \"$_rc_words\" -eq 2 ] \\\n\
             \x20 && [ \"$_rc_host\" = {host} ] && [ \"$_rc_user\" = {user} ] \\\n\
             \x20 && [ \"$_rc_port\" = {port} ]; then\n\
             \x20 _RC_TGT_PW_FILE={secret} SSH_ASKPASS={helper} SSH_ASKPASS_REQUIRE=force \\\n\
             \x20   {ssh} -F {config}{route} \"$@\" 2>{stderr}\n\
             \x20 _rc_status=$?\n\
             \x20 if [ \"$_rc_status\" -eq 255 ] && grep -q 'Permission denied' {stderr} 2>/dev/null; then\n\
             \x20   : > {marker}\n\
             \x20 fi\n\
             \x20 rm -f {stderr}\n\
             \x20 exit \"$_rc_status\"\n\
             fi\n",
            host = shell_single_quote(delivery.host),
            user = shell_single_quote(delivery.user),
            port = shell_single_quote(&delivery.port.to_string()),
            secret = shell_single_quote(&delivery.secret_file.to_string_lossy()),
            helper = shell_single_quote(&delivery.helper.to_string_lossy()),
            stderr = shell_single_quote(&delivery.stderr_file.to_string_lossy()),
            marker = shell_single_quote(&delivery.failure_marker.to_string_lossy()),
        );
    }
    let _ = writeln!(script, "exec {ssh} -F {config} \"$@\"");
    script
}

/// The marker the wrapper leaves when `connection_id`'s sign-in was refused.
fn auth_failure_marker(connection_id: Uuid) -> PathBuf {
    wrapper_root().join(format!("{}.auth-failed", connection_id.as_hyphenated()))
}

/// Whether mc's `ssh` reported a refused password for `connection_id` since
/// the last call, clearing the report.
///
/// The caller then forgets its cached password, so the next mc open reads the
/// vault again rather than repeating a stale one.
#[must_use]
pub fn take_auth_failure(connection_id: Uuid) -> bool {
    std::fs::remove_file(auth_failure_marker(connection_id)).is_ok()
}

/// Returns the `SshConfig` of an SSH or SFTP connection.
fn ssh_config(conn: &Connection) -> Option<&crate::models::SshConfig> {
    match &conn.protocol_config {
        ProtocolConfig::Ssh(cfg) | ProtocolConfig::Sftp(cfg) => Some(cfg),
        _ => None,
    }
}

/// Resolves a connection's identity file, following the group inheritance chain
/// and repairing stale portal paths.
fn identity_file(conn: &Connection, groups: &[ConnectionGroup]) -> Option<PathBuf> {
    crate::connection::ssh_inheritance::resolve_ssh_key_path(conn, groups)
        .and_then(|p| crate::resolve_key_path(&p))
}

/// Writes one `ssh_config` option line, quoting the value.
///
/// Values are double-quoted so a path containing spaces survives; OpenSSH
/// accepts quoted values for every keyword used here.
fn push_quoted(out: &mut String, key: &str, value: &str) {
    let _ = writeln!(out, "    {key} \"{}\"", value.replace('"', ""));
}

/// Writes one `ssh_config` option line verbatim.
///
/// Used for values that must not be quoted, such as a comma-separated
/// `ProxyJump` hop list or a bare keyword like `accept-new`.
fn push_raw(out: &mut String, key: &str, value: &str) {
    let _ = writeln!(out, "    {key} {value}");
}

/// Builds the `ssh_config` text for `connection`.
///
/// Emits a `Host` block for the target, one for each reference-based jump hop
/// that has its own identity file (`-J` does not pass `-i` down to a hop —
/// issue #241), and a trailing include of the user's own configuration.
///
/// Returns `None` when the connection is not SSH-family.
fn build_ssh_config(
    connection: &Connection,
    connections: &[Connection],
    groups: &[ConnectionGroup],
    network: &crate::config::NetworkSettings,
) -> Option<String> {
    let cfg = ssh_config(connection)?;
    let chain = crate::connection::resolve_jump_chain(connection, connections, groups, network);
    let known_hosts = crate::flatpak::get_flatpak_known_hosts_path();

    let mut out = String::with_capacity(512);
    out.push_str(
        "# Generated by RustConn for Midnight Commander's sh:// VFS — do not edit.\n\
         # Rewritten on every SFTP open; see rustconn-core/src/mc_ssh.rs.\n\n",
    );

    let _ = writeln!(out, "Host {}", connection.host);

    // An explicit ProxyCommand takes precedence over a jump chain, matching the
    // ssh_config exporter and OpenSSH's own "ProxyCommand instead of a direct
    // connection" wording. Only one of the two is ever emitted.
    if let Some(proxy_command) = cfg
        .proxy_command
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
    {
        push_raw(&mut out, "ProxyCommand", proxy_command);
    } else if let Some(proxy_jump) = chain.proxy_jump_value() {
        push_raw(&mut out, "ProxyJump", &proxy_jump);
    }

    if let Some(key) = identity_file(connection, groups) {
        push_quoted(&mut out, "IdentityFile", &key.to_string_lossy());
        if cfg.identities_only {
            push_raw(&mut out, "IdentitiesOnly", "yes");
        }
    }

    if let Some(provider) = cfg
        .pkcs11_provider
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        push_quoted(&mut out, "PKCS11Provider", provider);
    }

    // Custom options come last so the settings above win a collision; they
    // carry, among other things, the `HostKeyAlias` that the Flatpak mDNS
    // fallback injects when it substitutes a resolved address for a `.local`
    // name.
    let mut custom: Vec<(&String, &String)> = cfg.custom_options.iter().collect();
    custom.sort_by(|a, b| a.0.cmp(b.0));
    for (key, value) in custom {
        if key.trim().is_empty() {
            continue;
        }
        push_raw(&mut out, key.trim(), value.trim());
    }

    if let Some(ref kh) = known_hosts {
        push_quoted(&mut out, "UserKnownHostsFile", &kh.to_string_lossy());
        push_raw(&mut out, "StrictHostKeyChecking", "accept-new");
    }

    // One block per jump hop that needs its own credentials or known_hosts.
    let mut seen: HashSet<String> = HashSet::new();
    seen.insert(connection.host.clone());
    for (spec, hop_id) in chain.hops.iter().zip(chain.hop_ids.iter()) {
        let host = hop_hostname(spec);
        if !seen.insert(host.to_string()) {
            continue;
        }
        let hop_key = hop_id
            .and_then(|id| connections.iter().find(|c| c.id == id))
            .and_then(|hop| identity_file(hop, groups));
        if hop_key.is_none() && known_hosts.is_none() {
            continue;
        }
        let _ = writeln!(out, "\nHost {host}");
        if let Some(key) = hop_key {
            push_quoted(&mut out, "IdentityFile", &key.to_string_lossy());
        }
        if let Some(ref kh) = known_hosts {
            push_quoted(&mut out, "UserKnownHostsFile", &kh.to_string_lossy());
            push_raw(&mut out, "StrictHostKeyChecking", "accept-new");
        }
    }

    // `Match all` resets the block context: an `Include` written straight after
    // a `Host` block belongs to that block and only fires for hosts it matches,
    // which would silently drop the user's configuration.
    out.push_str("\nMatch all\n    Include ~/.ssh/config\n");

    Some(out)
}

/// Locates the real `ssh` binary, skipping our own wrapper directory.
///
/// Scanning `PATH` rather than hardcoding `/usr/bin/ssh` keeps the wrapper
/// working where ssh lives elsewhere (Homebrew on macOS, Nix profiles), and
/// excluding `wrapper_dir` is what stops the wrapper from calling itself.
fn find_real_ssh(wrapper_dir: &Path) -> PathBuf {
    let path = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        if dir == wrapper_dir {
            continue;
        }
        let candidate = dir.join("ssh");
        if candidate.is_file() && is_executable(&candidate) {
            return candidate;
        }
    }
    tracing::warn!(
        fallback = "/usr/bin/ssh",
        "No ssh binary found in PATH (excluding wrapper directory); \
         using hardcoded fallback — this may be wrong on NixOS or Homebrew setups"
    );
    PathBuf::from("/usr/bin/ssh")
}

/// Returns `true` when the file has at least one execute bit set.
///
/// On non-Unix (unlikely for this project) always returns `true` so the
/// fallback does not silently discard every candidate.
fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        true
    }
}

/// Removes wrapper directories left behind by an earlier run.
fn prune_stale(root: &Path, keep: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        if path == keep || !path.is_dir() {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .and_then(|m| now.duration_since(m).map_err(std::io::Error::other))
            .is_ok_and(|age| age > STALE_AFTER);
        if stale {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

/// Prepares an `ssh` wrapper so mc's `sh://` VFS honours the connection's SSH
/// settings — jump host above all (issue #255).
///
/// Writes `<runtime>/rustconn-mc-ssh/<session_id>/{config,ssh}` and returns the
/// handle whose [`McSshEnv::path_env`] the caller passes to the mc process. The
/// directory is keyed on the session so two mc tabs with different jump hosts
/// do not share one wrapper, and is rewritten from scratch on every call.
///
/// With `password`, the wrapper also delivers it to mc's own `ssh` (see the
/// module docs) when [`password_may_be_sent`] holds and the route keeps it on
/// this connection's host. Checking the route may run `ssh -G` (bounded at a
/// few seconds), so call this off the GTK main thread. Whether it delivers is
/// [`McSshEnv::delivers_password`]; the caller removes
/// [`McSshEnv::secret_file`] when mc exits. Any failure on the delivery path is
/// logged and leaves the wrapper as without a password.
///
/// Returns `None` for a non-SSH connection, or if the files cannot be written —
/// in which case the caller should launch mc without a `PATH` override, exactly
/// as it did before this mechanism existed.
#[must_use]
pub fn prepare_mc_ssh_env(
    session_id: Uuid,
    connection: &Connection,
    connections: &[Connection],
    groups: &[ConnectionGroup],
    network: &crate::config::NetworkSettings,
    password: Option<&SecretString>,
) -> Option<McSshEnv> {
    let config_text = build_ssh_config(connection, connections, groups, network)?;

    let root = wrapper_root();
    let wrapper_dir = root.join(session_id.as_hyphenated().to_string());
    // Start from a clean directory so a stale config from a previous open of
    // the same session cannot survive an edit to the connection.
    let _ = std::fs::remove_dir_all(&wrapper_dir);
    if let Err(e) = std::fs::create_dir_all(&wrapper_dir) {
        tracing::warn!(
            dir = %wrapper_dir.display(),
            error = %e,
            "Could not create the mc ssh wrapper directory; \
             launching mc without SSH options"
        );
        return None;
    }
    prune_stale(&root, &wrapper_dir);

    let config_path = wrapper_dir.join("config");
    if let Err(e) = std::fs::write(&config_path, config_text.as_bytes()) {
        tracing::warn!(
            path = %config_path.display(),
            error = %e,
            "Could not write the generated ssh_config for mc"
        );
        return None;
    }

    let real_ssh = find_real_ssh(&wrapper_dir);
    let wrapper_path = wrapper_dir.join("ssh");
    let delivery = password
        .and_then(|password| password_delivery(connection, connections, groups, network, password));
    let stderr_file = wrapper_dir.join("ssh-stderr");
    let failure_marker = auth_failure_marker(connection.id);
    let user = connection.username.as_deref().unwrap_or_default();
    let wrapper = wrapper_script(
        &real_ssh,
        &config_path,
        delivery
            .as_ref()
            .map(|(helper, secret_file, args)| Delivery {
                secret_file,
                helper,
                host: &connection.host,
                user,
                port: connection.port,
                args,
                stderr_file: &stderr_file,
                failure_marker: &failure_marker,
            })
            .as_ref(),
    );
    let secret_file = delivery.map(|(_, secret_file, _)| secret_file);
    if let Err(e) = write_wrapper(&wrapper_path, &wrapper) {
        tracing::warn!(
            path = %wrapper_path.display(),
            error = %e,
            "Could not write the ssh wrapper for mc"
        );
        if let Some(ref path) = secret_file {
            let _ = std::fs::remove_file(path);
        }
        return None;
    }

    tracing::debug!(
        dir = %wrapper_dir.display(),
        real_ssh = %real_ssh.display(),
        delivers_password = secret_file.is_some(),
        "Prepared ssh wrapper for mc"
    );
    Some(McSshEnv {
        wrapper_dir,
        config_path,
        secret_file,
    })
}

/// Writes the wrapper script and makes it executable for the owner only.
fn write_wrapper(path: &Path, text: &str) -> std::io::Result<()> {
    std::fs::write(path, text.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// The askpass helper, the password file and the route arguments for
/// delivering `password` to mc's `ssh`, or `None` when it must not be sent.
fn password_delivery(
    connection: &Connection,
    connections: &[Connection],
    groups: &[ConnectionGroup],
    network: &crate::config::NetworkSettings,
    password: &SecretString,
) -> Option<(PathBuf, PathBuf, Vec<String>)> {
    if !password_may_be_sent(connection, groups) {
        return None;
    }
    let cfg = ssh_config(connection)?;
    let chain = crate::connection::resolve_jump_chain(connection, connections, groups, network);
    let known_hosts = crate::flatpak::get_flatpak_known_hosts_path();
    let Some(args) = password_route_args(
        connection,
        cfg,
        &chain,
        connections,
        groups,
        known_hosts.as_deref(),
        crate::ssh_tunnel::ssh_config_declares_proxy,
    ) else {
        tracing::info!(
            connection = %connection.name,
            "Not giving mc the password: its route could hand it to another host"
        );
        return None;
    };
    let helper = match crate::ssh_askpass::helper_script(crate::ssh_askpass::AskpassSlot::Target) {
        Ok(helper) => helper,
        Err(error) => {
            tracing::warn!(%error, "Could not write the askpass helper for mc");
            return None;
        }
    };
    match crate::ssh_askpass::create_secret_file(password) {
        Ok(secret_file) => Some((helper, secret_file, args)),
        Err(error) => {
            tracing::warn!(%error, "Could not write the password file for mc");
            None
        }
    }
}

/// Removes the wrapper directory for `session_id`.
///
/// Best effort: [`prepare_mc_ssh_env`] also prunes directories left behind, so
/// a missed call costs nothing beyond a few hundred bytes until the login
/// session ends.
pub fn cleanup_mc_ssh_env(session_id: Uuid) {
    let dir = wrapper_root().join(session_id.as_hyphenated().to_string());
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::SshKeySource;

    fn ssh(name: &str, host: &str, port: u16, user: Option<&str>) -> Connection {
        let mut conn = Connection::new_ssh(name.to_string(), host.to_string(), port);
        conn.username = user.map(str::to_string);
        conn
    }

    fn set_jump_host_id(conn: &mut Connection, id: Uuid) {
        if let ProtocolConfig::Ssh(ref mut cfg) | ProtocolConfig::Sftp(ref mut cfg) =
            conn.protocol_config
        {
            cfg.jump_host_id = Some(id);
        }
    }

    fn set_key(conn: &mut Connection, path: &Path) {
        if let ProtocolConfig::Ssh(ref mut cfg) | ProtocolConfig::Sftp(ref mut cfg) =
            conn.protocol_config
        {
            cfg.key_source = SshKeySource::File {
                path: path.to_path_buf(),
            };
        }
    }

    /// Creates an on-disk key file and returns its path.
    ///
    /// `resolve_ssh_key_path` is filtered through `resolve_key_path`, which only
    /// accepts a path that exists — a missing file is treated as a stale portal
    /// path and dropped. Tests therefore need real files, not string literals.
    fn key_file(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"not-a-real-key\n").unwrap();
        path
    }

    #[test]
    fn hop_hostname_strips_user_and_port() {
        assert_eq!(hop_hostname("host.example.com"), "host.example.com");
        assert_eq!(hop_hostname("ops@host.example.com"), "host.example.com");
        assert_eq!(
            hop_hostname("ops@host.example.com:2222"),
            "host.example.com"
        );
        assert_eq!(hop_hostname("host.example.com:2222"), "host.example.com");
    }

    #[test]
    fn hop_hostname_handles_ipv6_literals() {
        assert_eq!(hop_hostname("ops@[2001:db8::1]:2222"), "2001:db8::1");
        assert_eq!(hop_hostname("[::1]"), "::1");
    }

    #[test]
    fn unbracketed_ipv6_is_not_mistaken_for_host_and_port() {
        // The trailing `:1` is part of the address, not a port — a port after an
        // IPv6 literal requires brackets.
        assert_eq!(hop_hostname("2001:db8::1"), "2001:db8::1");
        assert_eq!(hop_hostname("ops@2001:db8::1"), "2001:db8::1");
    }

    #[test]
    fn a_trailing_colon_without_digits_is_kept() {
        assert_eq!(hop_hostname("host.example.com:"), "host.example.com:");
    }

    #[test]
    fn non_ssh_connection_produces_no_config() {
        let conn = Connection::new_rdp("rdp".to_string(), "host".to_string(), 3389);
        assert!(
            build_ssh_config(&conn, &[], &[], &crate::config::NetworkSettings::default()).is_none()
        );
    }

    #[test]
    fn config_carries_the_resolved_jump_host() {
        let bastion = ssh("bastion", "jump.example.com", 22, Some("ops"));
        let mut conn = ssh("target", "target.example.com", 22, Some("me"));
        set_jump_host_id(&mut conn, bastion.id);

        let cfg = build_ssh_config(
            &conn,
            std::slice::from_ref(&bastion),
            &[],
            &crate::config::NetworkSettings::default(),
        )
        .unwrap();
        assert!(cfg.contains("Host target.example.com\n"));
        assert!(cfg.contains("ProxyJump ops@jump.example.com\n"));
    }

    #[test]
    fn config_ends_with_a_reset_before_the_user_include() {
        let conn = ssh("target", "target.example.com", 22, Some("me"));
        let cfg =
            build_ssh_config(&conn, &[], &[], &crate::config::NetworkSettings::default()).unwrap();
        // `Match all` must precede the include, or it would be scoped to the
        // preceding Host block and never apply.
        assert!(cfg.ends_with("Match all\n    Include ~/.ssh/config\n"));
    }

    #[test]
    fn jump_hop_gets_its_own_identity_block() {
        let dir = tempfile::tempdir().unwrap();
        let bastion_key = key_file(dir.path(), "bastion");
        let target_key = key_file(dir.path(), "target");

        let mut bastion = ssh("bastion", "jump.example.com", 22, Some("ops"));
        set_key(&mut bastion, &bastion_key);
        let mut conn = ssh("target", "target.example.com", 22, Some("me"));
        set_key(&mut conn, &target_key);
        set_jump_host_id(&mut conn, bastion.id);

        let cfg = build_ssh_config(
            &conn,
            std::slice::from_ref(&bastion),
            &[],
            &crate::config::NetworkSettings::default(),
        )
        .unwrap();
        assert!(cfg.contains("Host jump.example.com\n"));
        // Each key lands under its own host, not merged into one block.
        let mut blocks = cfg.split("\nHost jump.example.com");
        let target_block = blocks.next().unwrap();
        assert!(target_block.contains(&format!("IdentityFile \"{}\"", target_key.display())));
        let hop_block = blocks.next().unwrap();
        assert!(hop_block.contains(&format!("IdentityFile \"{}\"", bastion_key.display())));
    }

    #[test]
    fn hop_without_a_key_gets_no_block_outside_flatpak() {
        let bastion = ssh("bastion", "jump.example.com", 22, Some("ops"));
        let mut conn = ssh("target", "target.example.com", 22, Some("me"));
        set_jump_host_id(&mut conn, bastion.id);

        let cfg = build_ssh_config(
            &conn,
            std::slice::from_ref(&bastion),
            &[],
            &crate::config::NetworkSettings::default(),
        )
        .unwrap();
        // Nothing to say about the hop, so no empty block is emitted. (Outside
        // Flatpak there is no known_hosts override to add either.)
        if crate::flatpak::get_flatpak_known_hosts_path().is_none() {
            assert!(!cfg.contains("Host jump.example.com\n"));
        }
    }

    #[test]
    fn custom_options_reach_the_config() {
        let mut conn = ssh("target", "10.0.0.5", 22, Some("me"));
        if let ProtocolConfig::Ssh(ref mut cfg) = conn.protocol_config {
            cfg.custom_options
                .insert("HostKeyAlias".to_string(), "printer.local".to_string());
        }

        let cfg =
            build_ssh_config(&conn, &[], &[], &crate::config::NetworkSettings::default()).unwrap();
        assert!(cfg.contains("HostKeyAlias printer.local\n"));
    }

    #[test]
    fn explicit_proxy_command_wins_over_the_jump_chain() {
        let bastion = ssh("bastion", "jump.example.com", 22, Some("ops"));
        let mut conn = ssh("target", "target.example.com", 22, Some("me"));
        set_jump_host_id(&mut conn, bastion.id);
        if let ProtocolConfig::Ssh(ref mut cfg) = conn.protocol_config {
            cfg.proxy_command = Some("ncat --proxy 127.0.0.1:9050 %h %p".to_string());
        }

        let cfg = build_ssh_config(
            &conn,
            std::slice::from_ref(&bastion),
            &[],
            &crate::config::NetworkSettings::default(),
        )
        .unwrap();
        assert!(cfg.contains("ProxyCommand ncat --proxy 127.0.0.1:9050 %h %p\n"));
        assert!(!cfg.contains("ProxyJump "));
    }

    #[test]
    fn identities_only_is_emitted_only_with_a_key() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = ssh("target", "target.example.com", 22, Some("me"));
        if let ProtocolConfig::Ssh(ref mut cfg) = conn.protocol_config {
            cfg.identities_only = true;
        }
        // IdentitiesOnly without an identity file would hide every agent key and
        // break authentication outright, so it is gated on having one.
        let without_key =
            build_ssh_config(&conn, &[], &[], &crate::config::NetworkSettings::default()).unwrap();
        assert!(!without_key.contains("IdentitiesOnly"));

        set_key(&mut conn, &key_file(dir.path(), "target"));
        let with_key =
            build_ssh_config(&conn, &[], &[], &crate::config::NetworkSettings::default()).unwrap();
        assert!(with_key.contains("IdentitiesOnly yes\n"));
    }

    #[test]
    fn generated_config_is_accepted_by_ssh() {
        // `ssh -G` parses the file and prints the effective settings, so this
        // catches a malformed keyword or quoting mistake that unit assertions
        // on the text would miss.
        let dir = tempfile::tempdir().unwrap();
        let bastion_key = key_file(dir.path(), "bastion");
        // A path with a space is exactly what the quoting exists for.
        let target_key = key_file(dir.path(), "with space/target");

        let mut bastion = ssh("bastion", "jump.example.com", 2222, Some("ops"));
        set_key(&mut bastion, &bastion_key);
        let mut conn = ssh("target", "target.example.com", 22, Some("me"));
        set_key(&mut conn, &target_key);
        set_jump_host_id(&mut conn, bastion.id);

        let text = build_ssh_config(
            &conn,
            std::slice::from_ref(&bastion),
            &[],
            &crate::config::NetworkSettings::default(),
        )
        .unwrap();
        let path = dir.path().join("config");
        std::fs::write(&path, &text).unwrap();

        let out = std::process::Command::new("ssh")
            .arg("-F")
            .arg(&path)
            .arg("-G")
            .arg("target.example.com")
            .output();
        let Ok(out) = out else {
            // No ssh on this machine — the text assertions above still ran.
            return;
        };
        assert!(
            out.status.success(),
            "ssh rejected the generated config: {}\n--- config ---\n{text}",
            String::from_utf8_lossy(&out.stderr)
        );
        let rendered = String::from_utf8_lossy(&out.stdout).to_lowercase();
        assert!(rendered.contains("proxyjump ops@jump.example.com:2222"));
        assert!(rendered.contains(&format!(
            "identityfile {}",
            target_key.display().to_string().to_lowercase()
        )));
    }

    #[test]
    fn wrapper_is_written_and_points_at_a_real_ssh() {
        let conn = ssh("target", "target.example.com", 22, Some("me"));
        let session = Uuid::new_v4();
        let Some(env) = prepare_mc_ssh_env(
            session,
            &conn,
            &[],
            &[],
            &crate::config::NetworkSettings::default(),
            None,
        ) else {
            return;
        };
        assert!(!env.delivers_password());

        let wrapper = env.wrapper_dir().join("ssh");
        assert!(wrapper.is_file());
        assert!(env.config_path().is_file());

        let script = std::fs::read_to_string(&wrapper).unwrap();
        assert!(script.starts_with("#!/bin/sh\n"));
        assert!(script.contains("-F "));
        // The wrapper must never resolve to itself.
        assert!(!script.contains(&format!("exec '{}'", wrapper.display())));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&wrapper).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }

        assert!(env.path_env().starts_with("PATH="));
        assert!(
            env.path_env()
                .contains(&env.wrapper_dir().display().to_string())
        );

        cleanup_mc_ssh_env(session);
        assert!(!env.wrapper_dir().exists());
    }

    #[test]
    fn wrapper_hands_ssh_the_jump_host_end_to_end() {
        // The whole point of the mechanism: invoking `ssh` from the wrapper
        // directory — which is what mc does, by name and with no options — must
        // yield a ProxyJump. `-G` makes ssh print the settings it resolved and
        // exit, so this exercises the wrapper and the generated config together
        // rather than asserting on the config text.
        let bastion = ssh("bastion", "jump.example.com", 2222, Some("ops"));
        let mut conn = ssh("target", "target.example.com", 22, Some("me"));
        set_jump_host_id(&mut conn, bastion.id);

        let session = Uuid::new_v4();
        let Some(env) = prepare_mc_ssh_env(
            session,
            &conn,
            std::slice::from_ref(&bastion),
            &[],
            &crate::config::NetworkSettings::default(),
            None,
        ) else {
            return;
        };

        let out = std::process::Command::new(env.wrapper_dir().join("ssh"))
            .arg("-G")
            .arg("target.example.com")
            .output();
        cleanup_mc_ssh_env(session);

        let Ok(out) = out else {
            // No usable ssh on this machine; the config-text tests still ran.
            return;
        };
        assert!(
            out.status.success(),
            "wrapper failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let rendered = String::from_utf8_lossy(&out.stdout).to_lowercase();
        assert!(
            rendered.contains("proxyjump ops@jump.example.com:2222"),
            "wrapper did not apply the jump host; ssh reported:\n{rendered}"
        );
    }

    #[test]
    fn find_real_ssh_skips_the_wrapper_directory() {
        let dir = tempfile::tempdir().unwrap();
        let decoy = dir.path().join("ssh");
        std::fs::write(&decoy, b"#!/bin/sh\n").unwrap();

        // With the directory excluded, the decoy must not be chosen even though
        // it is first on PATH.
        let found = find_real_ssh(dir.path());
        assert_ne!(found, decoy);
    }

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(ToString::to_string).collect()
    }

    fn cfg_of(conn: &mut Connection) -> &mut SshConfig {
        match conn.protocol_config {
            ProtocolConfig::Ssh(ref mut cfg) | ProtocolConfig::Sftp(ref mut cfg) => cfg,
            _ => panic!("not an SSH connection"),
        }
    }

    /// mc's own argv for `sh://me@target.example.com:2222/~`.
    const MC_REMOTE_COMMAND: &str = "echo SHELL:; /bin/sh";

    #[test]
    fn wrapper_without_password_is_unchanged() {
        assert_eq!(
            wrapper_script(
                Path::new("/usr/bin/ssh"),
                Path::new("/run/user/1000/rustconn-mc-ssh/s/config"),
                None
            ),
            "#!/bin/sh\n\
             # Auto-generated by RustConn so mc's sh:// VFS picks up this\n\
             # connection's SSH options. See rustconn-core/src/mc_ssh.rs.\n\
             exec '/usr/bin/ssh' -F '/run/user/1000/rustconn-mc-ssh/s/config' \"$@\"\n"
        );
    }

    /// A delivering wrapper around a stub `ssh` that prints the password file
    /// it was given and its argv, and exits with `$RC_STUB_STATUS`.
    #[cfg(unix)]
    struct StubWrapper {
        dir: tempfile::TempDir,
        wrapper: PathBuf,
        config: PathBuf,
        secret: PathBuf,
        marker: PathBuf,
    }

    #[cfg(unix)]
    impl StubWrapper {
        fn new(port: u16) -> Self {
            use std::os::unix::fs::PermissionsExt;
            let dir = tempfile::tempdir().unwrap();
            let ssh = dir.path().join("real-ssh");
            std::fs::write(
                &ssh,
                "#!/bin/sh\n\
                 printf '%s\\n' \"${_RC_TGT_PW_FILE-unset}\"\n\
                 printf '%s\\n' \"$*\"\n\
                 [ -z \"$RC_STUB_ERR\" ] || printf '%s\\n' \"$RC_STUB_ERR\" >&2\n\
                 exit \"${RC_STUB_STATUS:-0}\"\n",
            )
            .unwrap();
            std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o700)).unwrap();
            let config = dir.path().join("config");
            let secret = dir.path().join("secret");
            let helper = dir.path().join("helper");
            let marker = dir.path().join("auth-failed");
            let args = strs(&["-o", "NumberOfPasswordPrompts=1"]);
            let text = wrapper_script(
                &ssh,
                &config,
                Some(&Delivery {
                    secret_file: &secret,
                    helper: &helper,
                    host: "target.example.com",
                    user: "me",
                    port,
                    args: &args,
                    stderr_file: &dir.path().join("ssh-stderr"),
                    failure_marker: &marker,
                }),
            );
            let wrapper = dir.path().join("ssh");
            write_wrapper(&wrapper, &text).unwrap();
            Self {
                dir,
                wrapper,
                config,
                secret,
                marker,
            }
        }

        /// Runs the wrapper with `argv`; returns (password file seen, argv seen, status).
        fn run(&self, argv: &[&str], status: Option<&str>, stderr: &str) -> (String, String, i32) {
            let mut command = std::process::Command::new("sh");
            command.arg(&self.wrapper).args(argv);
            command.env_remove("_RC_TGT_PW_FILE");
            if let Some(status) = status {
                command.env("RC_STUB_STATUS", status);
            }
            command.env("RC_STUB_ERR", stderr);
            let out = command.output().unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
            let mut lines = stdout.lines();
            let file = lines.next().unwrap_or_default().to_string();
            let seen = lines.next().unwrap_or_default().to_string();
            (file, seen, out.status.code().unwrap_or(-1))
        }

        fn assert_refused(&self, argv: &[&str]) {
            let (file, seen, _) = self.run(argv, None, "");
            assert_eq!(file, "unset", "{argv:?}");
            assert_eq!(
                seen,
                format!("-F {} {}", self.config.display(), argv.join(" ")),
                "{argv:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn delivers_only_for_mc_argv_to_this_host() {
        let stub = StubWrapper::new(2222);
        for argv in [
            [
                "-p",
                "2222",
                "-l",
                "me",
                "target.example.com",
                MC_REMOTE_COMMAND,
            ],
            [
                "-l",
                "me",
                "-p",
                "2222",
                "target.example.com",
                MC_REMOTE_COMMAND,
            ],
        ] {
            let (file, seen, status) = stub.run(&argv, None, "");
            assert_eq!(file, stub.secret.display().to_string(), "{argv:?}");
            assert_eq!(status, 0);
            assert_eq!(
                seen,
                format!(
                    "-F {} -o NumberOfPasswordPrompts=1 {}",
                    stub.config.display(),
                    argv.join(" ")
                )
            );
        }
        // `-C` is the one flag mc adds on its own.
        let (file, _, _) = stub.run(
            &[
                "-C",
                "-p",
                "2222",
                "-l",
                "me",
                "target.example.com",
                MC_REMOTE_COMMAND,
            ],
            None,
            "",
        );
        assert_eq!(file, stub.secret.display().to_string());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_another_host_user_or_port() {
        let stub = StubWrapper::new(2222);
        stub.assert_refused(&[
            "-p",
            "2222",
            "-l",
            "me",
            "other.example.com",
            MC_REMOTE_COMMAND,
        ]);
        stub.assert_refused(&[
            "-p",
            "2222",
            "-l",
            "root",
            "target.example.com",
            MC_REMOTE_COMMAND,
        ]);
        stub.assert_refused(&[
            "-p",
            "22",
            "-l",
            "me",
            "target.example.com",
            MC_REMOTE_COMMAND,
        ]);
        stub.assert_refused(&["-l", "me", "target.example.com", MC_REMOTE_COMMAND]);
        stub.assert_refused(&["-p", "2222", "target.example.com", MC_REMOTE_COMMAND]);
    }

    #[cfg(unix)]
    #[test]
    fn refuses_extra_options() {
        let stub = StubWrapper::new(2222);
        for extra in [
            ["-J", "far.example.com"],
            ["-o", "ProxyJump=far.example.com"],
            ["-qJ", "far.example.com"],
        ] {
            let mut argv = vec!["-p", "2222", "-l", "me"];
            argv.extend(extra);
            argv.extend(["target.example.com", MC_REMOTE_COMMAND]);
            stub.assert_refused(&argv);
        }
    }

    /// OpenSSH parses options after HOST too, so `HOST -Jbastion` would reach
    /// the bastion with the target's askpass environment.
    #[cfg(unix)]
    #[test]
    fn refuses_options_after_host() {
        let stub = StubWrapper::new(2222);
        for extra in [
            &["-Jbastion.example.com"][..],
            &["-oProxyJump=bastion.example.com"][..],
            &["-J", "bastion.example.com"][..],
        ] {
            let mut argv = vec!["-p", "2222", "-l", "me", "target.example.com"];
            argv.extend(extra);
            argv.push(MC_REMOTE_COMMAND);
            stub.assert_refused(&argv);
            let mut argv = vec!["-p", "2222", "-l", "me", "target.example.com"];
            argv.push(MC_REMOTE_COMMAND);
            argv.extend(extra);
            stub.assert_refused(&argv);
        }
    }

    /// OpenSSH keeps the first `-p`/`-l`, so a repeat must not pass the gate
    /// on its last value.
    #[cfg(unix)]
    #[test]
    fn refuses_a_repeated_port_or_user() {
        let stub = StubWrapper::new(2222);
        stub.assert_refused(&[
            "-p",
            "22",
            "-p",
            "2222",
            "-l",
            "me",
            "target.example.com",
            MC_REMOTE_COMMAND,
        ]);
        stub.assert_refused(&[
            "-p",
            "2222",
            "-l",
            "root",
            "-l",
            "me",
            "target.example.com",
            MC_REMOTE_COMMAND,
        ]);
    }

    #[cfg(unix)]
    #[test]
    fn refuses_without_exactly_one_remote_command() {
        let stub = StubWrapper::new(2222);
        stub.assert_refused(&["-p", "2222", "-l", "me", "target.example.com"]);
        stub.assert_refused(&["-p", "2222", "-l", "me", "target.example.com", "id", "-a"]);
        stub.assert_refused(&["-p", "2222", "-l"]);
    }

    #[cfg(unix)]
    #[test]
    fn default_port_matches_without_p() {
        let stub = StubWrapper::new(22);
        let (file, _, _) = stub.run(
            &["-l", "me", "target.example.com", MC_REMOTE_COMMAND],
            None,
            "",
        );
        assert_eq!(file, stub.secret.display().to_string());
        stub.assert_refused(&[
            "-p",
            "2222",
            "-l",
            "me",
            "target.example.com",
            MC_REMOTE_COMMAND,
        ]);
    }

    #[cfg(unix)]
    #[test]
    fn a_refused_sign_in_leaves_an_auth_failure_marker() {
        let stub = StubWrapper::new(2222);
        let argv = [
            "-p",
            "2222",
            "-l",
            "me",
            "target.example.com",
            MC_REMOTE_COMMAND,
        ];

        let (_, _, status) = stub.run(&argv, Some("255"), "Connection closed by 192.0.2.1");
        assert_eq!(status, 255);
        assert!(!stub.marker.exists(), "a lost connection is not a refusal");

        let (_, _, status) = stub.run(
            &argv,
            Some("255"),
            "me@target: Permission denied (password).",
        );
        assert_eq!(status, 255);
        assert!(stub.marker.exists());
        // The captured stderr does not outlive the ssh it came from.
        assert!(!stub.dir.path().join("ssh-stderr").exists());
    }

    #[test]
    fn take_auth_failure_reports_a_marker_once() {
        let id = Uuid::new_v4();
        assert!(!take_auth_failure(id));
        let marker = auth_failure_marker(id);
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
        std::fs::write(&marker, b"").unwrap();
        assert!(take_auth_failure(id));
        assert!(!take_auth_failure(id));
    }

    fn route_args(
        conn: &Connection,
        chain: &JumpChain,
        connections: &[Connection],
        declares_proxy: impl FnOnce(&str, u16, Option<&str>) -> bool,
    ) -> Option<Vec<String>> {
        let cfg = ssh_config(conn).unwrap();
        password_route_args(conn, cfg, chain, connections, &[], None, declares_proxy)
    }

    #[test]
    fn a_direct_route_asks_the_ssh_config_first() {
        let conn = ssh("target", "target.example.com", 2222, Some("me"));
        let mut asked = None;
        let args = route_args(&conn, &JumpChain::default(), &[], |host, port, user| {
            asked = Some((host.to_string(), port, user.map(ToString::to_string)));
            false
        })
        .expect("a direct route keeps the password");
        assert_eq!(
            asked,
            Some((
                "target.example.com".to_string(),
                2222,
                Some("me".to_string())
            ))
        );
        assert!(
            !args.iter().any(|arg| arg.starts_with("ProxyCommand")),
            "{args:?}"
        );
        assert_eq!(
            args,
            strs(&[
                "-o",
                "NumberOfPasswordPrompts=1",
                "-o",
                "StrictHostKeyChecking=accept-new"
            ])
        );

        assert!(route_args(&conn, &JumpChain::default(), &[], |_, _, _| true).is_none());
    }

    #[test]
    fn the_connections_own_host_key_policy_is_kept() {
        let mut conn = ssh("target", "target.example.com", 22, Some("me"));
        cfg_of(&mut conn)
            .custom_options
            .insert("StrictHostKeyChecking".to_string(), "yes".to_string());
        let args = route_args(&conn, &JumpChain::default(), &[], |_, _, _| false).unwrap();
        assert!(
            !args.iter().any(|arg| arg.contains("accept-new")),
            "{args:?}"
        );
    }

    #[test]
    fn a_saved_jump_host_goes_behind_the_env_boundary() {
        let bastion = ssh("bastion", "jump.example.com", 22, Some("ops"));
        let conn = ssh("target", "target.example.com", 22, Some("me"));
        let chain = JumpChain {
            hops: strs(&["ops@jump.example.com"]),
            hop_ids: vec![Some(bastion.id)],
        };
        let args = route_args(&conn, &chain, std::slice::from_ref(&bastion), |_, _, _| {
            panic!("a ProxyCommand on the command line overrides ~/.ssh/config")
        })
        .expect("a plain hop keeps the password");
        assert_eq!(args[0], "-o");
        assert!(
            args[1].starts_with("ProxyCommand=env _RC_TGT_PW_FILE="),
            "{args:?}"
        );
        assert!(args[1].ends_with("ops@jump.example.com"), "{args:?}");
    }

    #[test]
    fn routes_that_could_reach_another_host_get_no_password() {
        let plain = ssh("target", "target.example.com", 22, Some("me"));

        let mut keyboard = plain.clone();
        cfg_of(&mut keyboard).auth_method = SshAuthMethod::KeyboardInteractive;
        let mut proxy_command = plain.clone();
        cfg_of(&mut proxy_command).proxy_command = Some("nc %h %p".to_string());
        let mut custom_jump = plain.clone();
        cfg_of(&mut custom_jump)
            .custom_options
            .insert("ProxyJump".to_string(), "far.example.com".to_string());
        for (case, conn) in [
            ("keyboard interactive", &keyboard),
            ("proxy command", &proxy_command),
            ("custom ProxyJump", &custom_jump),
        ] {
            assert!(
                route_args(conn, &JumpChain::default(), &[], |_, _, _| false).is_none(),
                "{case}"
            );
        }

        let shell_word = JumpChain {
            hops: strs(&["far;reboot"]),
            hop_ids: vec![None],
        };
        assert!(route_args(&plain, &shell_word, &[], |_, _, _| false).is_none());
    }

    #[test]
    fn a_farther_hop_with_its_own_key_keeps_todays_route() {
        let dir = tempfile::tempdir().unwrap();
        let near = ssh("near", "near.example.com", 22, Some("ops"));
        let mut far = ssh("far", "far.example.com", 22, Some("ops"));
        set_key(&mut far, &key_file(dir.path(), "far"));
        let conn = ssh("target", "target.example.com", 22, Some("me"));
        let chain = JumpChain {
            hops: strs(&["ops@near.example.com", "ops@far.example.com"]),
            hop_ids: vec![Some(near.id), Some(far.id)],
        };
        assert!(route_args(&conn, &chain, &[near, far], |_, _, _| false).is_none());
    }

    #[test]
    fn expects_password_only_without_a_key() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = ssh("target", "target.example.com", 22, Some("me"));
        assert!(expects_password(&conn, &[]));
        cfg_of(&mut conn).auth_method = SshAuthMethod::KeyboardInteractive;
        assert!(expects_password(&conn, &[]));
        assert!(!password_may_be_sent(&conn, &[]));

        for method in [SshAuthMethod::PublicKey, SshAuthMethod::Agent] {
            cfg_of(&mut conn).auth_method = method;
            assert!(!expects_password(&conn, &[]));
        }

        cfg_of(&mut conn).auth_method = SshAuthMethod::Password;
        set_key(&mut conn, &key_file(dir.path(), "target"));
        assert!(!expects_password(&conn, &[]));

        let rdp = Connection::new_rdp("rdp".to_string(), "host".to_string(), 3389);
        assert!(!expects_password(&rdp, &[]));
    }

    #[cfg(unix)]
    #[test]
    fn a_direct_password_connection_gets_an_owner_only_secret_file() {
        use secrecy::ExposeSecret as _;
        use std::os::unix::fs::PermissionsExt;

        let conn = ssh("target", "target.example.com", 22, Some("me"));
        // The real `ssh -G` decides; a proxy in this machine's config (or no
        // ssh at all) means no delivery, which other tests cover.
        if crate::ssh_tunnel::ssh_config_declares_proxy("target.example.com", 22, Some("me")) {
            return;
        }
        let password = SecretString::from("hunter2-mc-secret".to_string());
        let session = Uuid::new_v4();
        let Some(env) = prepare_mc_ssh_env(
            session,
            &conn,
            &[],
            &[],
            &crate::config::NetworkSettings::default(),
            Some(&password),
        ) else {
            return;
        };
        let Some(secret) = env.secret_file().map(Path::to_path_buf) else {
            // No runtime directory for secrets on this machine.
            cleanup_mc_ssh_env(session);
            return;
        };
        assert!(env.delivers_password());
        let mode = std::fs::metadata(&secret).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(
            std::fs::read_to_string(&secret)
                .unwrap()
                .trim_end_matches('\n'),
            password.expose_secret()
        );

        let script = std::fs::read_to_string(env.wrapper_dir().join("ssh")).unwrap();
        assert!(!script.contains(password.expose_secret()));
        assert!(script.contains(&secret.display().to_string()));
        // The path goes only to the delivering ssh, never to mc itself.
        assert!(!env.path_env().contains(&secret.display().to_string()));

        let _ = std::fs::remove_file(&secret);
        cleanup_mc_ssh_env(session);
        assert!(!env.wrapper_dir().exists());
    }
}
