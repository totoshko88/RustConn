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
use crate::models::{
    Connection, ConnectionGroup, PasswordSource, ProtocolConfig, SshAuthMethod, SshConfig,
};
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
    /// The connection the wrapper was prepared for, which keys its
    /// auth-failure marker.
    connection_id: Uuid,
}

impl McSshEnv {
    /// What the wrapper's gate has done with the password, or `None` before
    /// mc started an `ssh`.
    ///
    /// [`PasswordHandover::Delivered`] wins as soon as one `ssh` got it.
    /// Meaningful only when [`Self::delivers_password`]: a wrapper without a
    /// password notes nothing.
    #[must_use]
    pub fn password_handover(&self) -> Option<PasswordHandover> {
        if self.wrapper_dir.join(HANDOVER_DELIVERED).exists() {
            Some(PasswordHandover::Delivered)
        } else if self.wrapper_dir.join(HANDOVER_WITHHELD).exists() {
            Some(PasswordHandover::Withheld)
        } else {
            None
        }
    }

    /// Whether this mc session's `ssh` reported a refused sign-in.
    ///
    /// Reads the delivering `ssh`'s per-run logs in this session's wrapper
    /// directory, records a refusal as the report [`take_auth_failure`]
    /// takes, and removes the logs that hold one. Leaves the report in place,
    /// so the next mc open still opens without the password. Only this
    /// directory's own logs count: a report another mc session of the
    /// connection left behind does not make this one's exit a refusal. Call
    /// it when mc exits, so the caller can drop its cached password before a
    /// Connect sends it again.
    #[must_use]
    pub fn sign_in_refused(&self) -> bool {
        let Some(root) = self.wrapper_dir.parent() else {
            return false;
        };
        let marker = auth_failure_marker(root, self.connection_id);
        record_refused_sign_in(&self.wrapper_dir, self.connection_id, &marker)
    }

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
    /// Prepends the wrapper directory to
    /// [`crate::cli_download::get_extended_path`] so mc's `ssh` resolves to the
    /// wrapper and everything else resolves as in any other session. The
    /// extended path, not the inherited `PATH`: this entry replaces the child's
    /// whole `PATH`, and the child is started by that `PATH`. A macOS app
    /// launched from the Dock or Finder inherits launchd's
    /// `/usr/bin:/bin:/usr/sbin:/sbin`, so a Homebrew `mc` that the
    /// availability check found could not be started (issue #422).
    #[must_use]
    pub fn path_env(&self) -> String {
        self.path_env_over(&crate::cli_download::get_extended_path())
    }

    /// `PATH=<wrapper dir>:<search_path>`.
    fn path_env_over(&self, search_path: &str) -> String {
        format!("PATH={}:{search_path}", self.wrapper_dir.display())
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
///
/// Without it the root falls back to the temp directory, which on Linux is
/// the shared `/tmp`, so [`prepare_mc_ssh_env`] uses the root only after
/// [`ensure_private_dir`] has made sure it is this user's own.
fn wrapper_root() -> PathBuf {
    let base = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|d| !d.is_empty())
        .map_or_else(std::env::temp_dir, PathBuf::from);
    base.join(WRAPPER_ROOT)
}

/// Why the wrapper root is not used.
#[derive(Debug, thiserror::Error)]
enum UnsafeRoot {
    /// A symlink could send the wrapper's files anywhere.
    #[error("it is a symbolic link")]
    Symlink,
    /// Something other than a directory sits at the path.
    #[error("it is not a directory")]
    NotDirectory,
    /// Another account could replace the `ssh` script, which runs as this
    /// user and names the password file.
    #[error("another user owns it")]
    ForeignOwner,
    /// The directory could not be created, read or made owner-only.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Creates `dir` with mode 0700, or checks an existing one: it must be a
/// real directory owned by `uid`, and is made owner-only if it is not.
///
/// The root falls back to the shared `/tmp` when `XDG_RUNTIME_DIR` is unset,
/// where another local user could have created it first (issue #255).
#[cfg(unix)]
fn ensure_private_dir(dir: &Path, uid: u32) -> Result<(), UnsafeRoot> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};

    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Err(error) if error.kind() != std::io::ErrorKind::AlreadyExists => {
            return Err(error.into());
        }
        _ => {}
    }
    let metadata = std::fs::symlink_metadata(dir)?;
    if metadata.file_type().is_symlink() {
        return Err(UnsafeRoot::Symlink);
    }
    if !metadata.is_dir() {
        return Err(UnsafeRoot::NotDirectory);
    }
    if metadata.uid() != uid {
        return Err(UnsafeRoot::ForeignOwner);
    }
    // An older RustConn created it with the default mode.
    if metadata.mode() & 0o077 != 0 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Off Unix there is no shared `/tmp` to defend against; just create it.
#[cfg(not(unix))]
fn ensure_private_dir(dir: &Path, _uid: u32) -> Result<(), UnsafeRoot> {
    std::fs::create_dir_all(dir)?;
    Ok(())
}

/// This process's real user id.
fn current_uid() -> u32 {
    #[cfg(unix)]
    {
        nix::unistd::getuid().as_raw()
    }
    #[cfg(not(unix))]
    {
        0
    }
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

/// Whether mc's `ssh` for `connection` may ask for the account password.
///
/// SSH-family with authentication method Password or Keyboard Interactive.
/// Key and agent connections return `false`; their mc launch never reads a
/// password.
///
/// An identity file, the connection's own or one inherited from a group, is
/// deliberately not a veto, as for an SSH session since 0.21.2 (see
/// [`crate::ssh_tunnel::target_password_askpass_allowed`]): the helper answers
/// only the account password prompt, so a key that authenticates means it is
/// never asked.
#[must_use]
pub fn expects_password(connection: &Connection) -> bool {
    ssh_config(connection).is_some_and(|cfg| {
        matches!(
            cfg.auth_method,
            SshAuthMethod::Password | SshAuthMethod::KeyboardInteractive
        )
    })
}

/// Whether an identity file resolves for `connection`, its own or a group's.
///
/// The mc launch uses it only to decide whether "type the password" is worth
/// saying when no password was handed over: with a key, mc likely signs in
/// without one.
#[must_use]
pub fn has_identity_file(connection: &Connection, groups: &[ConnectionGroup]) -> bool {
    identity_file(connection, groups).is_some()
}

/// Whether the wrapper may hand `connection`'s password to mc's `ssh` at all.
///
/// [`expects_password`], and the cheap half of the SSH session's gate
/// ([`crate::ssh_tunnel::target_password_askpass_allowed`]): Keyboard
/// Interactive, a PKCS#11 token, a custom routing or authentication option and
/// an explicit `ProxyCommand` all refuse. Spawns nothing, so a caller can ask
/// it before reading the vault; [`prepare_mc_ssh_env`] adds the route checks.
#[must_use]
pub fn password_may_be_sent(connection: &Connection) -> bool {
    expects_password(connection)
        && ssh_config(connection)
            .is_some_and(|cfg| crate::ssh_tunnel::target_password_askpass_allowed(cfg, false))
}

/// What an mc launch does about `connection`'s password, before any lookup.
///
/// This is the one place the policy lives; the GUI follows it and only does
/// the lookup. In order, the first row that applies wins:
///
/// | Connection | Plan |
/// |---|---|
/// | Not Password / Keyboard Interactive authentication ([`expects_password`]) | [`Absent`](McPasswordPlan::Absent) |
/// | mc's last sign-in with the saved password was refused, and the password is saved | [`Refused`](McPasswordPlan::Refused) |
/// | A route or option that must not get the password ([`password_may_be_sent`]) | [`Absent`](McPasswordPlan::Absent) |
/// | Anything else, a prompt-at-connect connection after a refusal included | [`Lookup`](McPasswordPlan::Lookup) |
///
/// A `Lookup` password is still delivered only when the wrapper's gate in
/// [`prepare_mc_ssh_env`] agrees: the route check (`ssh -G`) and mc's exact
/// argument shape for this host, user and port.
#[must_use]
pub fn password_plan(connection: &Connection, sign_in_refused: bool) -> McPasswordPlan {
    if !expects_password(connection) {
        return McPasswordPlan::Absent;
    }
    if sign_in_refused && !matches!(connection.password_source, PasswordSource::Prompt) {
        return McPasswordPlan::Refused;
    }
    if !password_may_be_sent(connection) {
        return McPasswordPlan::Absent;
    }
    McPasswordPlan::Lookup
}

/// The outcome of [`password_plan`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McPasswordPlan {
    /// Hand mc no password and look nothing up.
    Absent,
    /// The saved password was refused: open without it so the user can type
    /// the right one. Looks nothing up.
    Refused,
    /// Look the password up (session cache, then its source) and hand it over.
    Lookup,
}

/// The arguments the delivering `ssh` gets in front of mc's own, or `None`
/// when the password must not be sent along this route.
///
/// With no jump chain, `declares_proxy` (normally
/// [`crate::ssh_tunnel::ssh_config_declares_proxy_with`] on the wrapper's own
/// `ssh`) must report no proxy in
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
    /// The delivering `ssh`'s own log (`ssh -E`), with `.<pid>` appended per
    /// run so concurrent `ssh` processes each have their own file. Holds
    /// only ssh's messages, not the remote stderr; mc sends stderr to
    /// `/dev/null`. Named by [`ssh_log_prefix`]; read by
    /// [`McSshEnv::sign_in_refused`] and [`take_auth_failure`].
    log_prefix: &'a Path,
    /// Where the gate notes each decision for [`McSshEnv::password_handover`].
    handover_dir: &'a Path,
}

/// Left in the wrapper directory when an `ssh` got the password.
const HANDOVER_DELIVERED: &str = "handover-delivered";

/// Left in the wrapper directory when an `ssh` ran without it.
const HANDOVER_WITHHELD: &str = "handover-withheld";

/// What the wrapper's gate has done with the password so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordHandover {
    /// An `ssh` with mc's argv shape for this connection got the password.
    Delivered,
    /// Only `ssh` processes without that shape ran, so none got it: another
    /// `sh://` link, or an mc build whose argv the gate does not know.
    Withheld,
}

/// File name prefix of the delivering `ssh`'s per-run log.
const SSH_LOG_PREFIX: &str = "ssh-log";

/// The file name every per-run log of `connection_id`'s delivering `ssh`
/// starts with (the wrapper appends `.<pid>`).
///
/// It names the connection so that an mc open can find the refusals of this
/// connection's other mc sessions without knowing their session ids.
fn ssh_log_prefix(connection_id: Uuid) -> String {
    format!("{SSH_LOG_PREFIX}.{}", connection_id.as_hyphenated())
}

/// What ssh logs when the server refuses every method it tried
/// (`user@host: Permission denied (password).`). The parenthesis keeps out
/// local errors such as `Load key "…": Permission denied`.
const REFUSED_SIGN_IN: &str = "Permission denied (";

/// The `ssh` wrapper script for mc.
///
/// Without `delivery` this is the two-line `exec` wrapper. With it, the script
/// first parses its argv against the shape mc uses (`[-C] [-p PORT] [-l USER]
/// HOST COMMAND`); only an exact match for the delivery's host, user and port
/// gets the askpass environment, exported just before it `exec`s `ssh`. Both
/// branches end in `exec`, so the process mc started is `ssh` itself and a
/// signal mc sends reaches it. Every embedded value is single-quoted; the
/// password itself is never in the text.
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
             \x20 : > {delivered}\n\
             \x20 _RC_TGT_PW_FILE={secret} SSH_ASKPASS={helper} SSH_ASKPASS_REQUIRE=force\n\
             \x20 export _RC_TGT_PW_FILE SSH_ASKPASS SSH_ASKPASS_REQUIRE\n\
             \x20 exec {ssh} -F {config}{route} -E {log}.$$ \"$@\"\n\
             fi\n\
             : > {withheld}\n",
            host = shell_single_quote(delivery.host),
            user = shell_single_quote(delivery.user),
            port = shell_single_quote(&delivery.port.to_string()),
            secret = shell_single_quote(&delivery.secret_file.to_string_lossy()),
            helper = shell_single_quote(&delivery.helper.to_string_lossy()),
            log = shell_single_quote(&delivery.log_prefix.to_string_lossy()),
            delivered = shell_single_quote(
                &delivery
                    .handover_dir
                    .join(HANDOVER_DELIVERED)
                    .to_string_lossy()
            ),
            withheld = shell_single_quote(
                &delivery
                    .handover_dir
                    .join(HANDOVER_WITHHELD)
                    .to_string_lossy()
            ),
        );
    }
    let _ = writeln!(script, "exec {ssh} -F {config} \"$@\"");
    script
}

/// The marker left in the wrapper `root` when `connection_id`'s sign-in from
/// mc was refused.
fn auth_failure_marker(root: &Path, connection_id: Uuid) -> PathBuf {
    root.join(format!("{}.auth-failed", connection_id.as_hyphenated()))
}

/// Creates `marker` when a log of `connection_id`'s delivering `ssh` in
/// `wrapper_dir` reports a refused sign-in, and removes the logs that do.
/// Returns whether one did.
///
/// The logs are written through `ssh -E`, after the wrapper has `exec`ed
/// `ssh`, so nothing in the wrapper can check them; this runs when mc exits
/// and again at the next mc open. A log without a refusal is left alone,
/// because its `ssh` may still be signing in; it goes when the directory is
/// rewritten or pruned.
fn record_refused_sign_in(wrapper_dir: &Path, connection_id: Uuid, marker: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(wrapper_dir) else {
        return false;
    };
    let log_name = format!("{}.", ssh_log_prefix(connection_id));
    let mut refused = false;
    for path in entries.flatten().map(|entry| entry.path()) {
        if !path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with(&log_name))
        {
            continue;
        }
        if std::fs::read_to_string(&path).is_ok_and(|log| log.contains(REFUSED_SIGN_IN)) {
            refused = true;
            let _ = std::fs::remove_file(&path);
        }
    }
    if refused && let Err(error) = std::fs::write(marker, b"") {
        tracing::warn!(%error, "Could not record mc's refused sign-in");
    }
    refused
}

/// Whether mc's `ssh` reported a refused password for `connection_id` since
/// the last call, clearing the report.
///
/// Besides the report [`McSshEnv::sign_in_refused`] left when an mc exited,
/// this reads the logs of every mc session of the connection, so a refusal
/// counts even when that mc's exit was never seen (RustConn quit first) or
/// that mc is still open. The caller then opens mc without the saved password
/// once. A wrapper root that is not this user's own reports nothing.
#[must_use]
pub fn take_auth_failure(connection_id: Uuid) -> bool {
    take_auth_failure_in(&wrapper_root(), current_uid(), connection_id)
}

/// [`take_auth_failure`] in the wrapper `root`, which must belong to `uid`.
fn take_auth_failure_in(root: &Path, uid: u32, connection_id: Uuid) -> bool {
    // In a shared `/tmp` another user could plant a marker or a log; neither
    // says anything about this user's sign-in.
    if let Err(reason) = ensure_private_dir(root, uid) {
        tracing::debug!(dir = %root.display(), %reason, "Not reading mc sign-in reports");
        return false;
    }
    let marker = auth_failure_marker(root, connection_id);
    if let Ok(entries) = std::fs::read_dir(root) {
        for dir in entries.flatten().map(|entry| entry.path()) {
            if dir.is_dir() {
                record_refused_sign_in(&dir, connection_id, &marker);
            }
        }
    }
    std::fs::remove_file(&marker).is_ok()
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

/// Why a value is not written into the generated `ssh_config`.
///
/// The file is line-based: a line break, or a quote that ends a quoted value
/// early, lets a host name, a custom option or a free-text `ProxyJump` from an
/// import or a Dynamic Folder script add directives of its own. The value is
/// not logged; a `ProxyCommand` can hold a secret.
fn unsafe_value(key: &str) {
    tracing::warn!(
        option = key,
        "Not building the mc ssh_config: a value has a line break, control character or quote"
    );
}

/// Writes one `ssh_config` option line, quoting the value.
///
/// Values are double-quoted so a path containing spaces survives; OpenSSH
/// accepts quoted values for every keyword used here. Returns `None`, writing
/// nothing, for a value with a control character, a `"` or a `\`, which
/// would end the quotes early, or a `${`, which ssh expands from the
/// environment.
fn push_quoted(out: &mut String, key: &str, value: &str) -> Option<()> {
    if value.contains("${")
        || value
            .chars()
            .any(|c| c.is_control() || c == '"' || c == '\\')
    {
        unsafe_value(key);
        return None;
    }
    let _ = writeln!(out, "    {key} \"{value}\"");
    Some(())
}

/// Writes a path-valued option, with each `%` doubled: `IdentityFile` and
/// `UserKnownHostsFile` read `%d`, `%u` and the like as tokens, and `%%` as a
/// literal `%`.
fn push_path(out: &mut String, key: &str, path: &Path) -> Option<()> {
    push_quoted(out, key, &path.to_string_lossy().replace('%', "%%"))
}

/// Writes one `ssh_config` option line verbatim.
///
/// Used for values that must not be quoted, such as a comma-separated
/// `ProxyJump` hop list or a bare keyword like `accept-new`. Returns `None`,
/// writing nothing, for a value with a control character, and for a `key` that
/// is not one bare word or that starts a new block.
fn push_raw(out: &mut String, key: &str, value: &str) -> Option<()> {
    let bad_key = key.is_empty()
        || key
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '=')
        || key.eq_ignore_ascii_case("Host")
        || key.eq_ignore_ascii_case("Match");
    if bad_key || value.chars().any(char::is_control) {
        unsafe_value(if bad_key { "custom option" } else { key });
        return None;
    }
    let _ = writeln!(out, "    {key} {value}");
    Some(())
}

/// Writes a `Host` line for `host`, which must be one literal name: no
/// whitespace or control character (a second pattern or a new line), no
/// pattern or negation character, no quote, and no leading `-`.
fn push_host(out: &mut String, host: &str) -> Option<()> {
    let bad = host.is_empty()
        || host.starts_with('-')
        || host
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || "*?!\"'#".contains(c));
    if bad {
        unsafe_value("Host");
        return None;
    }
    let _ = writeln!(out, "Host {host}");
    Some(())
}

/// Builds the `ssh_config` text for `connection`.
///
/// Emits a `Host` block for the target, one for each reference-based jump hop
/// that has its own identity file (`-J` does not pass `-i` down to a hop —
/// issue #241), and a trailing include of the user's own configuration.
///
/// Returns `None` when the connection is not SSH-family, or when a host name
/// or option value could add directives of its own (see [`push_raw`]); the
/// caller then launches mc without the wrapper.
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

    push_host(&mut out, &connection.host)?;

    // An explicit ProxyCommand takes precedence over a jump chain, matching the
    // ssh_config exporter and OpenSSH's own "ProxyCommand instead of a direct
    // connection" wording. Only one of the two is ever emitted.
    if let Some(proxy_command) = cfg
        .proxy_command
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
    {
        push_raw(&mut out, "ProxyCommand", proxy_command)?;
    } else if let Some(proxy_jump) = chain.proxy_jump_value() {
        push_raw(&mut out, "ProxyJump", &proxy_jump)?;
    }

    if let Some(key) = identity_file(connection, groups) {
        push_path(&mut out, "IdentityFile", &key)?;
        if cfg.identities_only {
            push_raw(&mut out, "IdentitiesOnly", "yes")?;
        }
    }

    if let Some(provider) = cfg
        .pkcs11_provider
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        push_quoted(&mut out, "PKCS11Provider", provider)?;
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
        push_raw(&mut out, key.trim(), value.trim())?;
    }

    if let Some(ref kh) = known_hosts {
        push_path(&mut out, "UserKnownHostsFile", kh)?;
        push_raw(&mut out, "StrictHostKeyChecking", "accept-new")?;
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
        out.push('\n');
        push_host(&mut out, host)?;
        if let Some(key) = hop_key {
            push_path(&mut out, "IdentityFile", &key)?;
        }
        if let Some(ref kh) = known_hosts {
            push_path(&mut out, "UserKnownHostsFile", kh)?;
            push_raw(&mut out, "StrictHostKeyChecking", "accept-new")?;
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
/// Scanning the search path rather than hardcoding `/usr/bin/ssh` keeps the
/// wrapper working where ssh lives elsewhere (Homebrew on macOS, Nix
/// profiles), and excluding `wrapper_dir` is what stops the wrapper from
/// calling itself. The search path is the one mc gets ([`McSshEnv::path_env`]),
/// so a Dock launch and a terminal launch pick the same `ssh`.
fn find_real_ssh(wrapper_dir: &Path) -> PathBuf {
    find_real_ssh_in(wrapper_dir, &crate::cli_download::get_extended_path())
}

/// [`find_real_ssh`] over an explicit `:`-separated search path.
fn find_real_ssh_in(wrapper_dir: &Path, search_path: &str) -> PathBuf {
    for dir in search_path
        .split(':')
        .filter(|d| !d.is_empty())
        .map(Path::new)
    {
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
/// Returns `None` for a non-SSH connection, if the wrapper root is not this
/// user's own owner-only directory (a symlink, or one another user created
/// in a shared `/tmp`; no password is delivered there), or if the files
/// cannot be written — in which case the
/// caller should launch mc without a `PATH` override, exactly as it did
/// before this mechanism existed.
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
    if let Err(reason) = ensure_private_dir(&root, current_uid()) {
        tracing::warn!(
            dir = %root.display(),
            %reason,
            "Not using the mc ssh wrapper directory; launching mc without SSH options or a password"
        );
        return None;
    }
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
    let delivery = password.and_then(|password| {
        password_delivery(
            connection,
            connections,
            groups,
            network,
            password,
            &real_ssh,
        )
    });
    let log_prefix = wrapper_dir.join(ssh_log_prefix(connection.id));
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
                log_prefix: &log_prefix,
                handover_dir: &wrapper_dir,
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
        connection_id: connection.id,
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
///
/// `real_ssh` is the binary the wrapper runs; the proxy check asks that same
/// binary, so it judges the route of the `ssh` that gets the password.
fn password_delivery(
    connection: &Connection,
    connections: &[Connection],
    groups: &[ConnectionGroup],
    network: &crate::config::NetworkSettings,
    password: &SecretString,
    real_ssh: &Path,
) -> Option<(PathBuf, PathBuf, Vec<String>)> {
    if !password_may_be_sent(connection) {
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
        |host, port, user| {
            crate::ssh_tunnel::ssh_config_declares_proxy_with(real_ssh, host, port, user)
        },
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

    /// Every row of the [`password_plan`] table, over the inputs that decide it.
    #[test]
    fn the_password_plan_follows_the_policy_table() {
        use McPasswordPlan::{Absent, Lookup, Refused};
        let build = |auth: SshAuthMethod, source: PasswordSource, proxy: Option<&str>| {
            let mut conn = ssh("t", "h.example.com", 22, Some("me"));
            conn.password_source = source;
            let cfg = ssh_cfg(&mut conn);
            cfg.auth_method = auth;
            cfg.proxy_command = proxy.map(str::to_string);
            conn
        };
        for source in [
            PasswordSource::None,
            PasswordSource::Vault,
            PasswordSource::Prompt,
            PasswordSource::Inherit,
            PasswordSource::Variable("v".into()),
            PasswordSource::Script("s".into()),
        ] {
            for refused in [false, true] {
                // Key and agent: never a password, refusal or not.
                for auth in [
                    SshAuthMethod::PublicKey,
                    SshAuthMethod::Agent,
                    SshAuthMethod::SecurityKey,
                ] {
                    let conn = build(auth.clone(), source.clone(), None);
                    assert_eq!(password_plan(&conn, refused), Absent, "{auth:?} {source:?}");
                }
                // Keyboard Interactive is `expects_password` but the gate refuses it.
                let kbd = build(SshAuthMethod::KeyboardInteractive, source.clone(), None);
                let expected = if refused && !matches!(source, PasswordSource::Prompt) {
                    Refused
                } else {
                    Absent
                };
                assert_eq!(password_plan(&kbd, refused), expected, "kbd {source:?}");
                // A custom ProxyCommand takes the route over.
                let proxied = build(SshAuthMethod::Password, source.clone(), Some("nc %h %p"));
                let expected = if refused && !matches!(source, PasswordSource::Prompt) {
                    Refused
                } else {
                    Absent
                };
                assert_eq!(
                    password_plan(&proxied, refused),
                    expected,
                    "proxy {source:?}"
                );
                // A plain password connection.
                let plain = build(SshAuthMethod::Password, source.clone(), None);
                let expected = match (refused, &source) {
                    (true, PasswordSource::Prompt) | (false, _) => Lookup,
                    (true, _) => Refused,
                };
                assert_eq!(password_plan(&plain, refused), expected, "plain {source:?}");
            }
        }
    }

    /// A non-SSH connection never expects one.
    #[test]
    fn a_non_ssh_connection_gets_no_password_plan() {
        let rdp = Connection::new_rdp("r".to_string(), "h".to_string(), 3389);
        assert_eq!(password_plan(&rdp, false), McPasswordPlan::Absent);
        assert_eq!(password_plan(&rdp, true), McPasswordPlan::Absent);
    }

    fn config_for(conn: &Connection, hops: &[Connection]) -> Option<String> {
        build_ssh_config(conn, hops, &[], &crate::config::NetworkSettings::default())
    }

    fn ssh_cfg(conn: &mut Connection) -> &mut SshConfig {
        let (ProtocolConfig::Ssh(cfg) | ProtocolConfig::Sftp(cfg)) = &mut conn.protocol_config
        else {
            unreachable!("test connection is SSH");
        };
        cfg
    }

    /// Hosts that would end the `Host` line or add a directive to the file.
    #[test]
    fn a_host_that_could_add_directives_gets_no_config() {
        for host in [
            "a\nProxyCommand touch /tmp/x",
            "a\r\nLocalCommand x",
            "a b",
            "a\tb",
            "a\0b",
            "-oProxyCommand=x",
            "*",
            "!a",
            "a?",
            "a\"b",
            "",
            " ",
        ] {
            let conn = ssh("t", host, 22, None);
            assert!(
                config_for(&conn, &[]).is_none(),
                "host {host:?} was accepted"
            );
        }
    }

    #[test]
    fn ordinary_hosts_still_get_a_config() {
        for host in [
            "server.example.com",
            "10.0.0.5",
            "::1",
            "fe80::1%eth0",
            "сервер.укр",
            "host_1",
        ] {
            let conn = ssh("t", host, 22, None);
            let cfg = config_for(&conn, &[]).unwrap_or_else(|| panic!("host {host:?} refused"));
            assert!(cfg.contains(&format!("Host {host}\n")));
        }
    }

    #[test]
    fn a_line_break_in_any_option_value_gets_no_config() {
        for value in ["x\nProxyCommand y", "x\r\nProxyCommand y", "x\0"] {
            let mut conn = ssh("t", "h.example.com", 22, None);
            ssh_cfg(&mut conn).proxy_command = Some(value.to_string());
            assert!(config_for(&conn, &[]).is_none(), "proxy_command {value:?}");

            let mut conn = ssh("t", "h.example.com", 22, None);
            ssh_cfg(&mut conn).proxy_jump = Some(value.to_string());
            assert!(config_for(&conn, &[]).is_none(), "proxy_jump {value:?}");

            let mut conn = ssh("t", "h.example.com", 22, None);
            ssh_cfg(&mut conn)
                .custom_options
                .insert("HostKeyAlias".to_string(), value.to_string());
            assert!(config_for(&conn, &[]).is_none(), "custom value {value:?}");

            let mut conn = ssh("t", "h.example.com", 22, None);
            ssh_cfg(&mut conn).pkcs11_provider = Some(value.to_string());
            assert!(config_for(&conn, &[]).is_none(), "pkcs11 {value:?}");
        }
    }

    #[test]
    fn a_custom_option_cannot_open_a_new_block() {
        for key in ["Host", "match", "Match exec", "Foo\nHost *", "A B"] {
            let mut conn = ssh("t", "h.example.com", 22, None);
            ssh_cfg(&mut conn)
                .custom_options
                .insert(key.to_string(), "x".to_string());
            assert!(config_for(&conn, &[]).is_none(), "option key {key:?}");
        }
    }

    #[test]
    fn a_percent_in_a_path_is_not_a_token() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = ssh("t", "h.example.com", 22, None);
        set_key(&mut conn, &key_file(dir.path(), "100%d key"));
        let cfg = config_for(&conn, &[]).unwrap();
        assert!(cfg.contains("100%%d key\"\n"), "{cfg}");

        // ssh expands `${VAR}` in these options, which cannot be escaped.
        assert!(push_quoted(&mut String::new(), "IdentityFile", "/k/${HOME}").is_none());
    }

    #[test]
    fn a_key_path_cannot_leave_its_quotes() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["k\"ey", "k\\", "k\ney"] {
            let mut conn = ssh("t", "h.example.com", 22, None);
            set_key(&mut conn, &key_file(dir.path(), name));
            assert!(config_for(&conn, &[]).is_none(), "key path {name:?}");
        }
        let mut conn = ssh("t", "h.example.com", 22, None);
        set_key(&mut conn, &key_file(dir.path(), "my key ключ"));
        assert!(config_for(&conn, &[]).is_some());
    }

    /// A jump host reaches the file through `Host <hop>`, from the hop's
    /// saved connection or from a free-text `ProxyJump`.
    #[test]
    fn a_jump_hop_that_could_add_directives_gets_no_config() {
        let dir = tempfile::tempdir().unwrap();
        for host in ["jump\nHost *", "jump host", "-oProxyCommand=x"] {
            let mut bastion = ssh("b", host, 22, Some("ops"));
            set_key(&mut bastion, &key_file(dir.path(), "b"));
            let mut conn = ssh("t", "h.example.com", 22, None);
            set_jump_host_id(&mut conn, bastion.id);
            assert!(
                config_for(&conn, std::slice::from_ref(&bastion)).is_none(),
                "hop {host:?}"
            );
        }
        let mut conn = ssh("t", "h.example.com", 22, None);
        ssh_cfg(&mut conn).proxy_jump = Some("ops@jump\nHost *".to_string());
        assert!(config_for(&conn, &[]).is_none());
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

    fn env_in(wrapper_dir: &Path) -> McSshEnv {
        McSshEnv {
            wrapper_dir: wrapper_dir.to_path_buf(),
            config_path: wrapper_dir.join("ssh_config"),
            secret_file: None,
            connection_id: Uuid::new_v4(),
        }
    }

    #[test]
    fn path_env_prepends_the_wrapper_to_the_extended_path() {
        let env = env_in(Path::new("/tmp/rustconn-mc-ssh/session"));
        assert_eq!(
            env.path_env(),
            format!(
                "PATH=/tmp/rustconn-mc-ssh/session:{}",
                crate::cli_download::get_extended_path()
            )
        );
    }

    /// Issue #422: the child is started by the `PATH` it is given, so a program
    /// only on the extended search path (a Homebrew `mc` under a Dock launch)
    /// is found with `path_env_over` and is "not installed" with launchd's PATH.
    #[cfg(unix)]
    #[test]
    fn a_program_only_on_the_search_path_is_found_by_the_child() {
        use std::os::unix::fs::PermissionsExt;

        let tools = tempfile::tempdir().unwrap();
        let probe = tools.path().join("rc-issue422-probe");
        std::fs::write(&probe, b"#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&probe, std::fs::Permissions::from_mode(0o700)).unwrap();
        let wrapper = tempfile::tempdir().unwrap();
        let env = env_in(wrapper.path());

        let search_path = format!("{}:/usr/bin:/bin", tools.path().display());
        let path = env.path_env_over(&search_path);
        let status = std::process::Command::new("rc-issue422-probe")
            .env_clear()
            .env("PATH", path.trim_start_matches("PATH="))
            .status()
            .unwrap();
        assert!(status.success());

        let launchd = std::process::Command::new("rc-issue422-probe")
            .env_clear()
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .status()
            .unwrap_err();
        assert_eq!(launchd.kind(), std::io::ErrorKind::NotFound);
    }

    #[cfg(unix)]
    #[test]
    fn find_real_ssh_in_skips_the_wrapper_and_uses_the_given_path() {
        use std::os::unix::fs::PermissionsExt;

        let wrapper = tempfile::tempdir().unwrap();
        let decoy = wrapper.path().join("ssh");
        std::fs::write(&decoy, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&decoy, std::fs::Permissions::from_mode(0o700)).unwrap();
        let tools = tempfile::tempdir().unwrap();
        let real = tools.path().join("ssh");
        std::fs::write(&real, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o700)).unwrap();

        // The wrapper directory comes first and must be skipped; the next
        // directory of the given path wins over anything on the process PATH.
        let search_path = format!(
            "{}::{}:/usr/bin",
            wrapper.path().display(),
            tools.path().display()
        );
        assert_eq!(find_real_ssh_in(wrapper.path(), &search_path), real);
        // Nothing usable on the given path: the hardcoded fallback.
        let only_wrapper = wrapper.path().display().to_string();
        assert_eq!(
            find_real_ssh_in(wrapper.path(), &only_wrapper),
            PathBuf::from("/usr/bin/ssh")
        );
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
    /// it was given, its argv and its pid, writes `$RC_STUB_ERR` to its `-E`
    /// log (stderr without one), and exits with `$RC_STUB_STATUS`.
    #[cfg(unix)]
    struct StubWrapper {
        dir: tempfile::TempDir,
        wrapper: PathBuf,
        config: PathBuf,
        secret: PathBuf,
        /// Where [`record_refused_sign_in`] records a refusal in these tests.
        marker: PathBuf,
        /// The connection the logs are named after.
        id: Uuid,
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
                 printf '%s\\n' \"$$\"\n\
                 _log= _prev=\n\
                 for _a do [ \"$_prev\" = -E ] && _log=$_a; _prev=$_a; done\n\
                 [ -z \"$_log\" ] || : >> \"$_log\"\n\
                 if [ -n \"$RC_STUB_ERR\" ]; then\n\
                 \x20 if [ -n \"$_log\" ]; then printf '%s\\n' \"$RC_STUB_ERR\" >> \"$_log\";\n\
                 \x20 else printf '%s\\n' \"$RC_STUB_ERR\" >&2; fi\n\
                 fi\n\
                 [ -z \"$RC_STUB_SLEEP\" ] || sleep \"$RC_STUB_SLEEP\"\n\
                 exit \"${RC_STUB_STATUS:-0}\"\n",
            )
            .unwrap();
            std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o700)).unwrap();
            let config = dir.path().join("config");
            let secret = dir.path().join("secret");
            let helper = dir.path().join("helper");
            let marker = dir.path().join("auth-failed");
            let id = Uuid::new_v4();
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
                    log_prefix: &dir.path().join(ssh_log_prefix(id)),
                    handover_dir: dir.path(),
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
                id,
            }
        }

        /// The wrapper with `argv`, the stub exiting with `status` after
        /// logging `stderr`.
        fn command(
            &self,
            argv: &[&str],
            status: Option<&str>,
            stderr: &str,
        ) -> std::process::Command {
            let mut command = std::process::Command::new("sh");
            command.arg(&self.wrapper).args(argv);
            command.env_remove("_RC_TGT_PW_FILE");
            if let Some(status) = status {
                command.env("RC_STUB_STATUS", status);
            }
            command.env("RC_STUB_ERR", stderr);
            command
        }

        /// The delivering `ssh` logs in the wrapper directory.
        fn log_files(&self) -> Vec<PathBuf> {
            let mut logs: Vec<PathBuf> = std::fs::read_dir(self.dir.path())
                .unwrap()
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .is_some_and(|name| name.to_string_lossy().starts_with(SSH_LOG_PREFIX))
                })
                .collect();
            logs.sort();
            logs
        }

        /// The `-E` arguments the delivering `ssh` with `pid` gets.
        fn log_args(&self, pid: u32) -> String {
            format!(
                "-E {}.{pid}",
                self.dir.path().join(ssh_log_prefix(self.id)).display()
            )
        }

        /// Runs the wrapper with `argv`; returns (password file seen, argv seen, status).
        fn run(&self, argv: &[&str], status: Option<&str>, stderr: &str) -> (String, String, i32) {
            let out = self.command(argv, status, stderr).output().unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
            let mut lines = stdout.lines();
            let file = lines.next().unwrap_or_default().to_string();
            let seen = lines.next().unwrap_or_default().to_string();
            (file, seen, out.status.code().unwrap_or(-1))
        }

        /// Runs the wrapper with `argv` and no error; returns (password file
        /// seen, argv seen, pid of the process started, pid the stub ran as).
        fn run_with_pid(&self, argv: &[&str]) -> (String, String, u32, String) {
            let child = self
                .command(argv, None, "")
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            let started = child.id();
            let out = child.wait_with_output().unwrap();
            assert!(out.status.success());
            let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
            let mut lines = stdout.lines().map(ToString::to_string);
            let file = lines.next().unwrap_or_default();
            let seen = lines.next().unwrap_or_default();
            let ran_as = lines.next().unwrap_or_default();
            (file, seen, started, ran_as)
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
            let (file, seen, pid, _) = stub.run_with_pid(&argv);
            assert_eq!(file, stub.secret.display().to_string(), "{argv:?}");
            assert_eq!(
                seen,
                format!(
                    "-F {} -o NumberOfPasswordPrompts=1 {} {}",
                    stub.config.display(),
                    stub.log_args(pid),
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
            // No remote command at all: the shape that leaked before af92b6ca.
            let mut argv = vec!["-p", "2222", "-l", "me", "target.example.com"];
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
        record_refused_sign_in(stub.dir.path(), stub.id, &stub.marker);
        assert!(!stub.marker.exists(), "a lost connection is not a refusal");

        let (_, _, status) = stub.run(
            &argv,
            Some("255"),
            "Load key \"/home/me/.ssh/id_ed25519\": Permission denied",
        );
        assert_eq!(status, 255);
        record_refused_sign_in(stub.dir.path(), stub.id, &stub.marker);
        assert!(!stub.marker.exists(), "an unreadable key is not a refusal");

        let (_, _, status) = stub.run(
            &argv,
            Some("255"),
            "me@target: Permission denied (password).",
        );
        assert_eq!(status, 255);
        assert!(
            !stub.marker.exists(),
            "recorded only when RustConn reads the logs"
        );
        record_refused_sign_in(stub.dir.path(), stub.id, &stub.marker);
        assert!(stub.marker.exists());
        // The refusal does not outlive the reading; the other two logs, whose
        // ssh could still be running, are left alone.
        let logs = stub.log_files();
        assert_eq!(logs.len(), 2);
        assert!(logs.iter().all(|log| {
            !std::fs::read_to_string(log)
                .unwrap()
                .contains(REFUSED_SIGN_IN)
        }));
    }

    /// Review item mc-n7: the gate notes what it did, so the tab can say when
    /// mc's `ssh` never got the password.
    #[cfg(unix)]
    #[test]
    fn the_gate_notes_whether_the_password_was_handed_over() {
        let mc_argv = [
            "-p",
            "2222",
            "-l",
            "me",
            "target.example.com",
            MC_REMOTE_COMMAND,
        ];

        let stub = StubWrapper::new(2222);
        let env = env_in(stub.dir.path());
        assert_eq!(env.password_handover(), None);
        // An mc build with a flag the gate does not know.
        stub.assert_refused(&["-x", "-p", "2222", "-l", "me", "target.example.com"]);
        assert_eq!(env.password_handover(), Some(PasswordHandover::Withheld));
        stub.run(&mc_argv, None, "");
        assert_eq!(env.password_handover(), Some(PasswordHandover::Delivered));

        let stub = StubWrapper::new(2222);
        let env = env_in(stub.dir.path());
        stub.run(&mc_argv, None, "");
        assert_eq!(env.password_handover(), Some(PasswordHandover::Delivered));
        stub.assert_refused(&[
            "-p",
            "2222",
            "-l",
            "me",
            "other.example.com",
            MC_REMOTE_COMMAND,
        ]);
        assert_eq!(
            env.password_handover(),
            Some(PasswordHandover::Delivered),
            "a later sh:// link does not undo the handover"
        );

        // The plain wrapper notes nothing.
        let dir = tempfile::tempdir().unwrap();
        let script = wrapper_script(Path::new("/bin/true"), &dir.path().join("config"), None);
        assert!(!script.contains(HANDOVER_WITHHELD));
        assert!(!script.contains(HANDOVER_DELIVERED));
    }

    /// Review item mc-n6: the delivering branch `exec`s ssh, so the process
    /// mc started, and every signal mc sends it, is ssh itself.
    #[cfg(unix)]
    #[test]
    fn the_delivering_wrapper_becomes_ssh() {
        let stub = StubWrapper::new(2222);
        let (file, _, started, ran_as) = stub.run_with_pid(&[
            "-p",
            "2222",
            "-l",
            "me",
            "target.example.com",
            MC_REMOTE_COMMAND,
        ]);
        assert_eq!(file, stub.secret.display().to_string());
        assert_eq!(ran_as, started.to_string());
        // The refusing branch has always exec'd.
        let (file, _, started, ran_as) = stub.run_with_pid(&["-p", "2222", "other.example.com"]);
        assert_eq!(file, "unset");
        assert_eq!(ran_as, started.to_string());
    }

    /// Review items mc-n5 and mc-n6: two delivering `ssh` processes at once
    /// each log to their own file, so the one that ends first neither
    /// truncates nor deletes the other's refusal.
    #[cfg(unix)]
    #[test]
    fn concurrent_delivering_ssh_runs_keep_their_own_logs() {
        let stub = StubWrapper::new(2222);
        let argv = [
            "-p",
            "2222",
            "-l",
            "me",
            "target.example.com",
            MC_REMOTE_COMMAND,
        ];
        let mut refused = stub.command(
            &argv,
            Some("255"),
            "me@target: Permission denied (password).",
        );
        refused.env("RC_STUB_SLEEP", "1");
        let mut refused = refused.stdout(std::process::Stdio::null()).spawn().unwrap();
        // Let the refused run write its stderr before the second one starts.
        std::thread::sleep(Duration::from_millis(300));
        let (_, _, status) = stub.run(&argv, None, "");
        assert_eq!(status, 0);

        assert_eq!(refused.wait().unwrap().code(), Some(255));
        assert_eq!(stub.log_files().len(), 2, "one log per delivering ssh");
        record_refused_sign_in(stub.dir.path(), stub.id, &stub.marker);
        assert!(
            stub.marker.exists(),
            "the refusal was lost to the other run"
        );
        assert_eq!(stub.log_files().len(), 1, "only the refused log goes");
    }

    /// Review item mc-r1: the root is created owner-only, an old one is
    /// tightened, and one that is a symlink or someone else's is refused
    /// without being touched.
    #[cfg(unix)]
    #[test]
    fn the_wrapper_root_must_be_this_users_own() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let base = tempfile::tempdir().unwrap();
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        let me = std::fs::metadata(base.path()).unwrap().uid();
        assert_eq!(me, current_uid());

        let fresh = base.path().join("fresh");
        ensure_private_dir(&fresh, me).unwrap();
        assert_eq!(mode(&fresh), 0o700);
        // A second call accepts its own directory.
        ensure_private_dir(&fresh, me).unwrap();

        let old = base.path().join("old");
        std::fs::create_dir(&old).unwrap();
        std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o755)).unwrap();
        ensure_private_dir(&old, me).unwrap();
        assert_eq!(mode(&old), 0o700);

        let link = base.path().join("link");
        std::os::unix::fs::symlink(&fresh, &link).unwrap();
        assert!(matches!(
            ensure_private_dir(&link, me),
            Err(UnsafeRoot::Symlink)
        ));

        let file = base.path().join("file");
        std::fs::write(&file, b"").unwrap();
        assert!(matches!(
            ensure_private_dir(&file, me),
            Err(UnsafeRoot::NotDirectory)
        ));

        // Owned by "another user": refused, and its mode is left alone.
        std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(
            ensure_private_dir(&old, me.wrapping_add(1)),
            Err(UnsafeRoot::ForeignOwner)
        ));
        assert_eq!(mode(&old), 0o755);
    }

    #[test]
    fn take_auth_failure_reports_a_marker_once() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("root");
        let uid = current_uid();
        let id = Uuid::new_v4();
        assert!(!take_auth_failure_in(&root, uid, id));
        std::fs::write(auth_failure_marker(&root, id), b"").unwrap();
        assert!(take_auth_failure_in(&root, uid, id));
        assert!(!take_auth_failure_in(&root, uid, id));
    }

    /// Review item mc-n2: a refusal still in a session's logs, because that
    /// mc's exit was never seen (RustConn quit first) or that mc is still
    /// open, is taken by the next open of the same connection only.
    #[test]
    fn take_auth_failure_reads_the_logs_of_every_session_of_the_connection() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("root");
        let uid = current_uid();
        let (id, other) = (Uuid::new_v4(), Uuid::new_v4());
        ensure_private_dir(&root, uid).unwrap();
        let log = |session: &str, connection: Uuid, text: &str| {
            let dir = root.join(session);
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join(format!("{}.4242", ssh_log_prefix(connection)));
            std::fs::write(&path, text).unwrap();
            path
        };

        let signing_in = log("a", id, "");
        assert!(!take_auth_failure_in(&root, uid, id));
        assert!(signing_in.exists(), "a running ssh keeps its log");

        log("b", id, "me@target: Permission denied (password).\n");
        let others = log("c", other, "me@other: Permission denied (password).\n");
        assert!(take_auth_failure_in(&root, uid, id));
        assert!(!take_auth_failure_in(&root, uid, id), "taken once");
        assert!(others.exists(), "another connection's refusal is its own");
        assert!(take_auth_failure_in(&root, uid, other));
    }

    /// Review item mc-r1: on a root that is not this user's own, a planted
    /// marker or log reports nothing and is left where it is.
    #[cfg(unix)]
    #[test]
    fn take_auth_failure_ignores_a_foreign_root() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("root");
        let uid = current_uid();
        let id = Uuid::new_v4();
        ensure_private_dir(&root, uid).unwrap();
        let marker = auth_failure_marker(&root, id);
        std::fs::write(&marker, b"").unwrap();
        assert!(!take_auth_failure_in(&root, uid.wrapping_add(1), id));
        assert!(marker.exists());

        let link = base.path().join("link");
        std::os::unix::fs::symlink(&root, &link).unwrap();
        assert!(!take_auth_failure_in(&link, uid, id));
        assert!(marker.exists());
    }

    /// Review item mc-n4: mc's exit reads the report without taking it, so
    /// the next mc open still sees it.
    #[test]
    fn sign_in_refused_leaves_the_report_for_the_next_open() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("root");
        let uid = current_uid();
        ensure_private_dir(&root, uid).unwrap();
        let session = root.join("session");
        std::fs::create_dir(&session).unwrap();
        let mut env = env_in(&session);
        let id = Uuid::new_v4();
        env.connection_id = id;
        assert!(!env.sign_in_refused());
        std::fs::write(
            session.join(format!("{}.4242", ssh_log_prefix(id))),
            "me@target: Permission denied (password).\n",
        )
        .unwrap();
        assert!(env.sign_in_refused());
        assert!(
            !env.sign_in_refused(),
            "the refusal was read with its log; nothing new since"
        );
        assert!(take_auth_failure_in(&root, uid, id));
        assert!(!env.sign_in_refused());
    }

    /// Review item cleanup-mc-refused: a report that another mc session of
    /// the connection left, and no open has taken yet, does not make an mc
    /// whose own logs hold no refusal report one at its exit, so that exit
    /// does not drop the cached password.
    #[test]
    fn sign_in_refused_ignores_another_sessions_report() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("root");
        let uid = current_uid();
        ensure_private_dir(&root, uid).unwrap();
        let session = root.join("session");
        std::fs::create_dir(&session).unwrap();
        let mut env = env_in(&session);
        let id = Uuid::new_v4();
        env.connection_id = id;
        std::fs::write(auth_failure_marker(&root, id), b"").unwrap();
        std::fs::write(
            session.join(format!("{}.4242", ssh_log_prefix(id))),
            "me@target: Authenticated to target ([192.0.2.1]:22) using \"password\".\n",
        )
        .unwrap();
        assert!(!env.sign_in_refused());
        assert!(
            take_auth_failure_in(&root, uid, id),
            "the other session's report is still there for the next open"
        );
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
    fn expects_password_follows_the_auth_method() {
        let mut conn = ssh("target", "target.example.com", 22, Some("me"));
        assert!(expects_password(&conn));
        cfg_of(&mut conn).auth_method = SshAuthMethod::KeyboardInteractive;
        assert!(expects_password(&conn));
        assert!(!password_may_be_sent(&conn));

        for method in [SshAuthMethod::PublicKey, SshAuthMethod::Agent] {
            cfg_of(&mut conn).auth_method = method;
            assert!(!expects_password(&conn));
        }

        let rdp = Connection::new_rdp("rdp".to_string(), "host".to_string(), 3389);
        assert!(!expects_password(&rdp));
    }

    /// Review item mc-n3: a key, the connection's own or a group's, does not
    /// stop the password from being sent, as for an SSH session since 0.21.2.
    #[test]
    fn a_resolved_key_does_not_veto_the_password() {
        let dir = tempfile::tempdir().unwrap();
        let mut own = ssh("target", "target.example.com", 22, Some("me"));
        set_key(&mut own, &key_file(dir.path(), "own"));
        assert!(has_identity_file(&own, &[]));
        assert!(expects_password(&own));
        assert!(password_may_be_sent(&own));
        assert!(route_args(&own, &JumpChain::default(), &[], |_, _, _| false).is_some());

        let mut group = ConnectionGroup::new("G".into());
        group.ssh_key_path = Some(key_file(dir.path(), "group"));
        let mut inherited = ssh("target", "target.example.com", 22, Some("me"));
        inherited.group_id = Some(group.id);
        cfg_of(&mut inherited).key_source = SshKeySource::Inherit;
        let groups = [group];
        assert!(has_identity_file(&inherited, &groups));
        assert!(expects_password(&inherited));
        assert!(password_may_be_sent(&inherited));
        let cfg = ssh_config(&inherited).unwrap();
        assert!(
            password_route_args(
                &inherited,
                cfg,
                &JumpChain::default(),
                &[],
                &groups,
                None,
                |_, _, _| false
            )
            .is_some()
        );
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
