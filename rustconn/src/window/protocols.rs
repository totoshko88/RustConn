//! Protocol-specific connection handlers for main window
//!
//! This module contains functions for starting connections for different protocols:
//! SSH, VNC, SPICE, Telnet, Serial, Kubernetes, and Zero Trust.

use std::rc::Rc;

use gtk4::glib;
use gtk4::prelude::*;
use rustconn_core::connection::{automation_inheritance, check_port, ssh_inheritance};
use rustconn_core::models::AutomationConfig;
use rustconn_core::variables::{Variable, VariableManager, VariableScope};
use uuid::Uuid;

pub use super::protocols_ssh::{reconnect_ssh_in_place, start_ssh_connection_observed};
use super::{MainWindow, prompt_autofill};
use crate::i18n::{i18n, i18n_f};
use crate::sidebar::ConnectionSidebar;
use crate::state::SharedAppState;
use crate::terminal::{ConnectionOutput, TerminalNotebook};
use crate::utils::spawn_blocking_with_callback;

/// Type alias for shared sidebar reference
pub type SharedSidebar = Rc<ConnectionSidebar>;

/// Type alias for shared notebook reference
pub type SharedNotebook = Rc<TerminalNotebook>;

/// Resolves the effective automation config for a connection, inheriting from
/// the group hierarchy if the connection has no own expect rules / post-login scripts.
///
/// Elevated credentials are folded in here, after inheritance, because this is
/// the one function every terminal launcher calls to get the rules a session will
/// run with — wiring the sudo rules into each launcher separately is how one of
/// them ends up missing them.
pub(crate) fn resolve_automation_for_connection(
    state: &SharedAppState,
    conn: &rustconn_core::Connection,
) -> AutomationConfig {
    let mut resolved = state
        .try_borrow()
        .ok()
        .map(|s| {
            let groups: Vec<_> = s.list_groups_owned();
            automation_inheritance::resolve_automation(conn, &groups)
        })
        .unwrap_or_else(|| conn.automation.clone());

    append_elevated_rules(conn, &mut resolved);
    resolved
}

/// Appends the generated sudo/su/doas rules to a resolved automation config.
///
/// The rules answer with the `${password}` placeholder, so they resolve through
/// the same path `automation_variables` already uses for a hand-written rule: the
/// credential is copied out of the cache only because a rule asks for it, lands in
/// a secret [`Variable`], and the substituted response is scrubbed when the
/// session ends. Handing the plaintext to `rustconn-core` instead would put an
/// unscrubbed second copy in every rule.
///
/// Appended rather than prepended; ordering inside the engine is by priority, and
/// the generated rules carry a higher one.
fn append_elevated_rules(conn: &rustconn_core::Connection, automation: &mut AutomationConfig) {
    // SFTP shares `SshConfig`, so the flag can be saved on an SFTP connection and
    // has to be honoured there too; matching `Ssh` alone made the switch look
    // enabled while nothing was ever injected.
    let (rustconn_core::ProtocolConfig::Ssh(ssh) | rustconn_core::ProtocolConfig::Sftp(ssh)) =
        &conn.protocol_config
    else {
        return;
    };
    let Some(elevated) = ssh.elevated.as_ref().filter(|e| e.enabled) else {
        return;
    };

    let rules = rustconn_core::elevated_credentials_rules(elevated);
    if rules.is_empty() {
        return;
    }
    tracing::debug!(
        connection_id = %conn.id,
        rules = rules.len(),
        delay_ms = elevated.delay_ms,
        "Elevated credentials enabled; added privilege-escalation expect rules"
    );
    automation.expect_rules.extend(rules);
}

/// Resolves global variables and overlays this connection's interactive
/// `@ask:` answers on top.
///
/// The answers were gathered by the ASK prompt at connect time and stored in
/// [`crate::state::AppState`]. Appending them after the globals makes them
/// shadow a same-named global for this connect, and — because every
/// substitution builder loads this list — makes them resolve everywhere a
/// `${...}` reference is expanded (host, username, custom command, Expect
/// responses). When the connection has no ASK answers this returns exactly what
/// [`crate::state::resolve_global_variables`] returns, so non-ASK connections
/// are unaffected.
pub(super) fn resolve_globals_with_ask(
    state: &SharedAppState,
    connection_id: Uuid,
) -> Vec<Variable> {
    let Ok(state_ref) = state.try_borrow() else {
        return Vec::new();
    };
    let mut vars = crate::state::resolve_global_variables(state_ref.settings());
    vars.extend(state_ref.ask_answers(connection_id).iter().cloned());
    vars
}

/// Collects the interactive `@ask:` prompts a connection needs before it can
/// launch, deduplicated by variable name and skipping any already answered.
///
/// It scans every string that gets `${...}` substitution at connect time — the
/// host, the username, an SSH startup command, a Custom Command template, and
/// each enabled Expect-rule response — resolving references against the same
/// scope chain the launch uses (this connection's local variables over the
/// globals). `answered` carries names already collected in a prior pass (an
/// earlier string, or a previous prompt round) so nothing is asked twice.
///
/// Returns the requests in first-appearance order. An empty result means the
/// connection has no outstanding interactive variables and can connect straight
/// away.
pub(super) fn collect_connection_ask_requests(
    conn: &rustconn_core::Connection,
    global_variables: &[Variable],
) -> Vec<(String, rustconn_core::AskSpec)> {
    // Seed a manager exactly like the launch builders: globals, then this
    // connection's local variables (which hold the `@ask:` directives).
    let mut manager = VariableManager::new();
    for var in global_variables {
        manager.set_global(var.clone());
    }
    let conn_id = conn.id;
    for var in conn.local_variables.values() {
        manager.set_connection(conn_id, Variable::new(&var.name, &var.value));
    }
    let scope = VariableScope::Connection(conn_id);

    // Every string a connection substitutes at launch. Kept in one place so a
    // new substituted field is one line here, not a missed prompt.
    let mut sources: Vec<String> = Vec::new();
    sources.push(conn.host.clone());
    if let Some(user) = &conn.username {
        sources.push(user.clone());
    }
    match &conn.protocol_config {
        rustconn_core::ProtocolConfig::Ssh(cfg) | rustconn_core::ProtocolConfig::Sftp(cfg) => {
            if let Some(startup) = &cfg.startup_command {
                sources.push(startup.clone());
            }
        }
        rustconn_core::ProtocolConfig::ZeroTrust(zt) => {
            if let rustconn_core::models::ZeroTrustProviderConfig::Generic(g) = &zt.provider_config
            {
                sources.push(g.command_template.clone());
            }
        }
        _ => {}
    }
    for rule in &conn.automation.expect_rules {
        if rule.enabled {
            sources.push(rule.response.clone());
        }
    }

    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut requests = Vec::new();
    for source in &sources {
        let Ok(found) = manager.collect_ask_requests(source, scope) else {
            continue;
        };
        for (name, spec) in found {
            if seen.insert(name.clone()) {
                requests.push((name, spec));
            }
        }
    }
    requests
}

/// Substitutes variables in a string using global variables from settings
///
/// Converts `${VAR_NAME}` references to their values from global variables.
/// A reference nothing defines is replaced with the empty string, and a value
/// containing a shell metacharacter makes the whole substitution fail, in which
/// case the input is returned unchanged — both are `substitute_for_command`
/// semantics, which is what a host name or a login name being built into an
/// argument list needs.
pub(super) fn substitute_variables(input: &str, global_variables: &[Variable]) -> String {
    if !input.contains("${") {
        return input.to_string();
    }

    let mut manager = VariableManager::new();
    for var in global_variables {
        manager.set_global(var.clone());
    }

    manager
        .substitute_for_command(input, VariableScope::Global)
        .unwrap_or_else(|_| input.to_string())
}

/// Returns the connection's resolved password, if credential resolution cached one.
///
/// Feeds `${password}` in a Custom Command template (issue #151), an Expect
/// response and a command macro (`macro_dispatch`). `None` when the
/// password source needs no vault lookup or nothing has been resolved yet — the
/// placeholder is then left untouched, like any other unknown reference.
pub(super) fn cached_connection_password(
    state: &SharedAppState,
    connection_id: Uuid,
) -> Option<secrecy::SecretString> {
    use secrecy::ExposeSecret;
    state
        .try_borrow()
        .ok()?
        .get_cached_credentials(connection_id)
        .map(|c| c.password.clone())
        .filter(|p| !p.expose_secret().is_empty())
}

/// Variables an expect-rule response resolves against, built-ins included.
///
/// Returns the global variables plus the four placeholders the Automation tab
/// offers under "Built-in" — `${password}`, `${username}`, `${host}` and
/// `${port}` — taken from the connection itself. Without them those four
/// resolved against nothing and were replaced with the empty string, so the
/// stock "Sudo Password" template answered the prompt with a bare newline
/// (issue #257).
///
/// The built-ins are appended after the globals, so for this session they shadow
/// a global variable that happens to share a name — which is what the labels in
/// the insert-variable menu promise. The list is built fresh per connection, so
/// the shadowing never reaches another one.
///
/// The password is copied out of the credential cache only when an enabled rule
/// actually references `${password}`, and it lands in a variable marked secret so
/// [`rustconn_core::Variable`]'s `Drop` scrubs it. The substituted response
/// itself is a plain `String` for the life of the session; that is pre-existing
/// behaviour for any secret global variable used in a response.
pub(super) fn automation_variables(
    state: &SharedAppState,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
    automation: &AutomationConfig,
    global_variables: &[Variable],
) -> Vec<Variable> {
    use secrecy::ExposeSecret;

    let mut vars = automation_variables_base(conn, global_variables);

    // Materialize the secret only for a rule that asks for it, so an ordinary
    // connection never puts its password into a `Variable`.
    let wants_password = automation
        .expect_rules
        .iter()
        .any(|r| r.enabled && r.response.contains("${password}"));
    if wants_password {
        if let Some(password) = cached_connection_password(state, connection_id) {
            vars.push(Variable::new_secret("password", password.expose_secret()));
        } else {
            tracing::warn!(
                %connection_id,
                placeholder = "password",
                "An expect rule asks for the password placeholder but no password was \
                 resolved for this connection; check its Password Source"
            );
        }
    }

    vars
}

/// Assembles the non-secret variables an Expect response can reference.
///
/// The order encodes precedence, because the caller loads the result into a
/// [`VariableManager`] where a later entry overwrites an earlier one of the same
/// name: globals first, then the connection-local variables (so a local shadows
/// a global — the same precedence the command path gives them via
/// `connection_variable_manager`), then the synthetic `${username}`, `${host}`
/// and `${port}` (so they shadow a same-named local). `${password}` is added by
/// the caller after this, last of all, and so shadows everything here — which
/// preserves the stock "Sudo Password" template's contract (issue #257).
///
/// Pulling the connection-local variables in is what lets `${user2_password}`
/// defined only on the connection resolve in an Expect response instead of being
/// dropped as undefined and leaving the user stuck at the prompt (issue #317).
pub(super) fn automation_variables_base(
    conn: &rustconn_core::Connection,
    global_variables: &[Variable],
) -> Vec<Variable> {
    let mut vars = global_variables.to_vec();

    for var in conn.local_variables.values() {
        vars.push(var.clone());
    }

    if let Some(username) = conn.username.as_deref().filter(|u| !u.trim().is_empty()) {
        vars.push(Variable::new("username", username));
    }
    vars.push(Variable::new("host", &conn.host));
    vars.push(Variable::new("port", conn.port.to_string()));

    vars
}

/// Builds the login auto-fill spec for a terminal-login protocol.
///
/// Telnet and a serial console authenticate by typing into the terminal, so the
/// account name and the password both come from here: the username from the
/// connection (with `${VAR}` references expanded), the password from the
/// credentials the vault resolved before launch. `automation` supplies the
/// optional expected prompt texts, already resolved through the group chain
/// (issue #254).
///
/// The returned spec may be empty — [`prompt_autofill::install_login_autofill`]
/// then does nothing, which is the correct behaviour for a device the user
/// logs into by hand.
fn login_autofill_for(
    state: &SharedAppState,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
    automation: &AutomationConfig,
    global_variables: &[Variable],
    protocol: &'static str,
) -> prompt_autofill::LoginAutofill {
    let username = conn
        .username
        .as_ref()
        .map(|u| substitute_variables(u, global_variables))
        .filter(|u| !u.trim().is_empty());

    prompt_autofill::LoginAutofill {
        username,
        password: cached_connection_password(state, connection_id),
        matcher: rustconn_core::LoginPromptMatcher::new(
            automation.username_prompt.as_deref(),
            automation.password_prompt.as_deref(),
        ),
        protocol,
        deadline_secs: automation.login_timeout_secs,
    }
}

/// Environment variable carrying the connection password to a Custom Command.
///
/// `${password}` in the template expands to a shell reference to this variable
/// rather than to the secret itself, so the value never enters a command line
/// (issue #151). See [`super::command_env`] for how it reaches the child.
const PASSWORD_ENV_VAR: &str = "RUSTCONN_PASSWORD";

/// Metacharacter-free stand-in that `${password}` resolves to during substitution.
///
/// `substitute_defined_for_command` rejects values containing shell
/// metacharacters, so the shell reference cannot be substituted directly. This
/// token passes that guard and is replaced by the reference afterwards. The
/// random suffix keeps a template that happens to contain the literal text from
/// colliding with it.
const PASSWORD_TOKEN: &str = "__RUSTCONN_PASSWORD_REF_8f21c4d7__";

/// A Zero Trust command ready to be spawned in a terminal.
pub(super) struct ZeroTrustLaunch {
    /// argv handed to VTE, already wrapped for Flatpak / login shell
    argv: Vec<String>,
    /// Command line echoed into the terminal, with secret values masked
    display: String,
    /// `KEY=VALUE` entries VTE adds to the child environment.
    ///
    /// Carries [`PASSWORD_ENV_VAR`] when the template references `${password}`
    /// and the connection has a resolved password. Empty on the Flatpak path,
    /// which delivers the same variable through `flatpak-spawn --env-fd`.
    env: Vec<zeroize::Zeroizing<String>>,
    /// Keeps the Flatpak `--env-fd` file alive until the argv has been spawned.
    _env_file: Option<super::command_env::EphemeralCommandEnv>,
}

impl ZeroTrustLaunch {
    /// Returns the extra environment entries as VTE expects them.
    fn env_refs(&self) -> Vec<&str> {
        self.env.iter().map(|e| e.as_str()).collect()
    }
}

/// Strips quotes the user put around `${password}`.
///
/// The placeholder is replaced by an already-quoted shell reference, so a
/// template written as `--password "${password}"` would otherwise end up as
/// `--password ""$RUSTCONN_PASSWORD""`, where the expansion sits *outside* the
/// quotes and word-splits on a password containing spaces.
fn strip_quotes_around_password(template: &str) -> String {
    template
        .replace("\"${password}\"", "${password}")
        .replace("'${password}'", "${password}")
}

/// Replaces [`PASSWORD_TOKEN`] with a quoted shell reference.
///
/// Returns the rewritten string and whether the token was present, i.e. whether
/// the variable actually has to be put into the child environment.
fn link_password_reference(text: &str) -> (String, bool) {
    if !text.contains(PASSWORD_TOKEN) {
        return (text.to_string(), false);
    }
    (
        text.replace(PASSWORD_TOKEN, &format!("\"${PASSWORD_ENV_VAR}\"")),
        true,
    )
}

/// Builds a variable manager for `conn`: synthetic connection fields first,
/// then the connection's own local variables (they win over the synthetic and
/// global ones), with global variables as the outermost fallback.
///
/// With `mask_secrets` every secret value is replaced by `********`, which is
/// what the command line echoed into the terminal (and the session log) uses.
///
/// `has_password` registers `${password}` as [`PASSWORD_TOKEN`]. The password
/// itself never enters the manager: it travels through the child environment, so
/// both the expanded and the masked pass produce the same text and there is
/// nothing here to mask.
pub(super) fn connection_variable_manager(
    conn: &rustconn_core::Connection,
    global_variables: &[Variable],
    has_password: bool,
    mask_secrets: bool,
) -> VariableManager {
    let value_of = |var: &Variable| -> String {
        if mask_secrets && var.is_secret() {
            "********".to_string()
        } else {
            var.value.clone()
        }
    };

    let mut manager = VariableManager::new();
    for var in global_variables {
        manager.set_global(Variable::new(&var.name, value_of(var)));
    }

    // Synthetic connection fields, mirroring the pre-connect task scope.
    // Empty ones are skipped so the placeholder stays available to the shell.
    let conn_id = conn.id;
    if !conn.host.is_empty() {
        manager.set_connection(conn_id, Variable::new("host", &conn.host));
    }
    if conn.port != 0 {
        manager.set_connection(conn_id, Variable::new("port", conn.port.to_string()));
    }
    if let Some(ref user) = conn.username
        && !user.is_empty()
    {
        manager.set_connection(conn_id, Variable::new("username", user));
    }
    if !conn.name.is_empty() {
        manager.set_connection(conn_id, Variable::new("name", &conn.name));
    }
    for var in conn.local_variables.values() {
        manager.set_connection(conn_id, Variable::new(&var.name, value_of(var)));
    }

    // Registered last so it also wins over a local variable named `password`.
    // Precedence is preserved elsewhere: the *value* behind the token already
    // prefers that local variable (see `effective_password`). What must not
    // happen is the local variable's plaintext being substituted directly, which
    // would put it in the `sh -c` argv and bypass the environment indirection.
    if has_password {
        manager.set_connection(conn_id, Variable::new("password", PASSWORD_TOKEN));
    }

    manager
}

/// Name of the local variable that overrides the stored password for `${password}`.
const PASSWORD_VARIABLE: &str = "password";

/// Resolves what `${password}` should expand to, as a secret.
///
/// A connection-local variable named `password` wins over the stored credential,
/// matching how every other local variable overrides its synthetic counterpart.
/// Either way the value leaves through the child environment, never the argv.
fn effective_password(
    conn: &rustconn_core::Connection,
    resolved: Option<&secrecy::SecretString>,
) -> Option<secrecy::SecretString> {
    conn.local_variables
        .values()
        .find(|var| var.name == PASSWORD_VARIABLE)
        .filter(|var| !var.value.is_empty())
        .map(|var| secrecy::SecretString::from(var.value.clone()))
        .or_else(|| resolved.cloned())
}

/// Builds the spawn argv and the echoed command line for a Zero Trust connection.
///
/// For the Generic provider (Custom Command) the `${var}` placeholders of the
/// template are resolved from the connection's local variables, the synthetic
/// connection fields and the global variables; unknown references are left
/// untouched so the shell can still expand them (issue #151).
///
/// `${password}` is special: it becomes a shell reference to
/// [`PASSWORD_ENV_VAR`], and the password is delivered through the child
/// environment instead of the command line. See [`super::command_env`].
///
/// # Errors
///
/// Returns `VariableError::UnsafeValue` when a resolved value contains shell
/// metacharacters or control characters — substituting it into `sh -c` would
/// be a command injection.
pub(super) fn build_zerotrust_launch(
    conn: &rustconn_core::Connection,
    zt_config: &rustconn_core::models::ZeroTrustConfig,
    global_variables: &[Variable],
    password: Option<&secrecy::SecretString>,
) -> Result<ZeroTrustLaunch, rustconn_core::variables::VariableError> {
    let (program, mut args) = zt_config.build_command(conn.username.as_deref());
    let is_generic = matches!(
        zt_config.provider,
        rustconn_core::models::ZeroTrustProvider::Generic
    );
    let password = effective_password(conn, password);

    // Generic yields ("sh", ["-c", <template>]) — expand the template only.
    let mut masked_args = args.clone();
    let mut needs_password_env = false;
    if is_generic && let Some(template) = args.last_mut() {
        let scope = VariableScope::Connection(conn.id);
        let source = strip_quotes_around_password(template);
        let expanded =
            connection_variable_manager(conn, global_variables, password.is_some(), false)
                .substitute_defined_for_command(&source, scope)?;
        // The masked pass must never fall back to `expanded`: that string holds
        // the real secret values and is echoed into the terminal and the session
        // log. If masking cannot be produced, the launch fails instead.
        let masked = connection_variable_manager(conn, global_variables, password.is_some(), true)
            .substitute_defined_for_command(&source, scope)?;
        // The password becomes a shell reference in both passes, so the echoed
        // line and the argv agree and neither carries the secret.
        let (expanded, used) = link_password_reference(&expanded);
        let (masked, _) = link_password_reference(&masked);
        needs_password_env = used;
        *template = expanded;
        if let Some(last) = masked_args.last_mut() {
            *last = masked;
        }
    }

    let join = |parts: &[String]| -> String {
        std::iter::once(program.as_str())
            .chain(parts.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let display = join(&masked_args);

    // Only populated when the template actually references `${password}`, so a
    // command line without it stays byte-identical to before this feature.
    let password_entry = needs_password_env
        .then(|| password.as_ref().map(|value| (PASSWORD_ENV_VAR, value)))
        .flatten();

    let mut env: Vec<zeroize::Zeroizing<String>> = Vec::new();
    let mut env_file = None;

    let spawn_argv = if is_generic && rustconn_core::flatpak::is_flatpak() {
        // The user's custom command refers to host-side binaries that are not
        // installed in the sandbox, so it runs through flatpak-spawn --host
        // (same approach as Local Shell, #122).
        //
        // Run it on the host through a *login* shell (`sh -lc`) so the host PATH
        // resolves the binary. `script` (util-linux) allocates a host PTY for
        // interactive TUI tools, but it is not present on every host (atomic
        // distros, or `script` outside the sandbox PATH) and GUI tools (e.g.
        // WinBox, #190) do not need a PTY at all. So probe the host for `script`
        // and fall back to a plain `sh -c` when it is missing — this fixes both
        // the "Failed to start command: script" portal error and launching GUI
        // programs from a Generic command.
        // ponytail: single login-shell probe per launch; fine for one-shot
        // command spawn (not in a hot path).
        let template = args.last().map_or("", String::as_str);
        // Escape single quotes for safe embedding in '...' shell string:
        // replace ' with '\'' (end quote, escaped quote, start quote)
        let escaped = template.replace('\'', "'\\''");
        let host_runner = "if command -v script >/dev/null 2>&1; then exec script -qfc \"$1\" /dev/null; else exec sh -c \"$1\"; fi";
        // `flatpak-spawn` does not forward the sandbox environment to the host,
        // and its `--env=` option would put the password back into an argv. The
        // secret therefore travels through `--env-fd`: the sandbox shell opens a
        // mode-0600 runtime file as fd 3, unlinks it immediately (the descriptor
        // stays valid), and only then execs `flatpak-spawn`.
        let env_fd_prefix = password_entry
            .and_then(|(name, value)| {
                let file = super::command_env::EphemeralCommandEnv::write(&[(name, value)])?;
                let path = file.path().to_string_lossy().replace('\'', "'\\''");
                env_file = Some(file);
                Some(format!("exec 3<'{path}'; rm -f '{path}'; "))
            })
            .unwrap_or_default();
        let env_fd_flag = if env_file.is_some() {
            " --env-fd=3"
        } else {
            ""
        };
        let spawn_cmd = format!(
            "{env_fd_prefix}exec flatpak-spawn --host{env_fd_flag} --env=TERM=xterm-256color -- sh -lc '{host_runner}' rustconn '{escaped}'"
        );
        vec!["/bin/sh".to_string(), "-c".to_string(), spawn_cmd]
    } else if is_generic {
        // build_command already returns a complete shell invocation
        // ("sh", ["-c", template]). Wrapping it in yet another shell would break
        // argument parsing (e.g. `bash -c 'sh -c aws login'` treats "login" as
        // $0, not part of the command). Spawn it directly.
        //
        // The command runs inside the sandbox (or natively), so VTE can hand the
        // password to the child environment directly — no file, no argv. Goes
        // through the same validation as the `--env-fd` path.
        if let Some(entry) =
            password_entry.and_then(|(name, value)| super::command_env::env_entry(name, value))
        {
            env.push(entry);
        }
        std::iter::once(program.clone()).chain(args).collect()
    } else {
        // Non-Generic providers have no user template, so `${password}` cannot
        // appear and `full_command` never holds a secret.
        let full_command = join(&args);
        let spawn_command = rustconn_core::flatpak::wrap_host_command(&full_command);
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        vec![shell, "-c".to_string(), spawn_command]
    };

    Ok(ZeroTrustLaunch {
        argv: spawn_argv,
        display,
        env,
        _env_file: env_file,
    })
}

/// Known external viewers that hand control to a daemon or a separate process
/// and then exit their initial child (a *detaching viewer*, R5.7).
///
/// ponytail: a static list, fine while the set of detaching viewers is tiny and
/// well-known. If users start reporting other detaching clients, promote this to
/// a setting (a user-editable list) rather than growing the match arm.
const DETACHING_VIEWERS: &[&str] = &["remmina", "krdc", "vinagre"];

/// Returns `true` for a *detaching viewer* (see [`DETACHING_VIEWERS`]).
///
/// RustConn cannot reap or terminate such a process, so it is registered with
/// `child: None` and never auto-closed by the shared poll timer; it ends only
/// via "Stop tracking". Owning viewers (tigervnc, xfreerdp, remote-viewer) keep
/// their `Child` and are watched normally. The `program` may be a bare name or a
/// full path, so it is matched on its file name.
fn external_viewer_detaches(program: &str) -> bool {
    DETACHING_VIEWERS.contains(&external_viewer_name(program))
}

/// Reduces a viewer reference to the bare binary name it will run as.
///
/// Strips both a leading directory and the `host:` marker, so `remote-viewer`,
/// `/usr/bin/remote-viewer` and `host:remote-viewer` all classify alike. Used for
/// the detaching-viewer verdict and for anything the user reads: `host:` is an
/// internal encoding and has no business appearing in an error message.
fn external_viewer_name(program: &str) -> &str {
    let bare = program
        .strip_prefix(rustconn_core::spice_client::HOST_VIEWER_PREFIX)
        .unwrap_or(program);
    std::path::Path::new(bare)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(bare)
}

/// Resolves a viewer reference into the command line that actually starts it.
///
/// A plain name or path is spawned directly. A `host:`-prefixed one comes from
/// [`rustconn_core::spice_client::detect_spice_viewer`] and exists only outside
/// the Flatpak sandbox, so it is run through `flatpak-spawn --host` — the same
/// encoding and the same treatment the RDP launcher gives a host FreeRDP.
/// `--watch-bus` ties the host process to this one, so quitting RustConn does not
/// leave an orphaned viewer behind.
fn external_viewer_command(program: &str) -> (String, Vec<String>) {
    match program.strip_prefix(rustconn_core::spice_client::HOST_VIEWER_PREFIX) {
        Some(host_binary) => (
            "flatpak-spawn".to_owned(),
            vec![
                "--host".to_owned(),
                "--watch-bus".to_owned(),
                host_binary.to_owned(),
            ],
        ),
        None => (program.to_owned(), Vec::new()),
    }
}

/// Spawns an external viewer process and registers it in the external-session
/// registry (issue #209), suppressing the notebook tab.
///
/// Records the connection start in history (R3.1) and passes the entry id into
/// the registry so the shared poll timer records the end exactly once when the
/// viewer exits. `ssh_tunnel`, when present, is stored in the notebook's tunnel
/// map so it stays alive for the session. Returns `true` on success; on spawn
/// failure it shows an error toast, sets the sidebar status to "failed", and
/// returns `false` — without creating a tab (R1.6).
#[expect(
    clippy::too_many_arguments,
    reason = "orchestrates external-viewer spawn + registry handoff; the shared state/notebook/sidebar handles and connection params are all required at this call site"
)]
pub(super) fn spawn_and_register_external_viewer(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
    program: &str,
    args: &[String],
    ssh_tunnel: Option<rustconn_core::ssh_tunnel::SshTunnel>,
) -> bool {
    // A `host:` viewer runs through flatpak-spawn; everything else runs directly.
    let (launcher, mut launch_args) = external_viewer_command(program);
    launch_args.extend(args.iter().cloned());
    let viewer_name = external_viewer_name(program);

    // Log the launcher and its full argument vector before spawning. A SPICE
    // password travels in the single-use `.vv` file, never on the command
    // line, so this is safe to log — and with stdout/stderr nulled below it is
    // the only record of what the viewer was asked to do. Without it, a viewer
    // that rejects an option (a version-sensitive `--spice-*` flag, or the
    // "connection type cannot be detected from URI" case in issue #308) exits
    // silently with nothing in the log to explain it — the same undiagnosable
    // failure fixed for external RDP in issue #339.
    tracing::debug!(
        program,
        launcher = %launcher,
        connection = %conn.name,
        args = ?launch_args,
        "Launching external viewer"
    );

    // Run the viewer independently: don't capture stdout/stderr.
    let child = match std::process::Command::new(&launcher)
        .args(&launch_args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            tracing::error!(%e, program, launcher, connection = %conn.name, "Failed to launch external viewer");
            crate::toast::show_error_toast_on_active_window(&i18n_f(
                "Could not launch external viewer ‘{}’: {}",
                &[viewer_name, &e.to_string()],
            ));
            sidebar.update_connection_status(&connection_id.to_string(), "failed");
            return false;
        }
    };

    // R3.1: record the start; its entry id is replayed to record_connection_end
    // by the registry's on_ended callback when the viewer exits.
    let history_entry_id = if let Ok(mut state_mut) = state.try_borrow_mut() {
        Some(state_mut.record_connection_start(conn, conn.username.as_deref()))
    } else {
        None
    };

    let session_id = Uuid::new_v4();

    // A detaching viewer is registered with child: None — RustConn neither reaps
    // nor kills it (R5.7). Dropping the handle does not terminate the process.
    let tracked_child = if external_viewer_detaches(program) {
        drop(child);
        None
    } else {
        Some(child)
    };

    match super::external_session_registry() {
        Some(registry) => {
            // ponytail: a tunnelled tabless external session keeps its SshTunnel
            // in the notebook map keyed by this synthetic session id; with no
            // tab-close event it is reclaimed at app exit (one ssh proc per
            // tunnelled external session). Move it into the registry entry if
            // this ever grows.
            if let Some(tunnel) = ssh_tunnel {
                notebook.store_ssh_tunnel(session_id, tunnel);
            }
            registry.register(session_id, connection_id, tracked_child, history_entry_id);
        }
        None => {
            tracing::error!(
                connection = %conn.name,
                "External session registry unavailable; viewer left untracked"
            );
        }
    }

    // Update last_connected timestamp.
    if let Ok(mut state_mut) = state.try_borrow_mut()
        && let Err(e) = state_mut.update_last_connected(connection_id)
    {
        tracing::warn!(?e, "Failed to update last_connected");
    }

    true
}

/// SSH failure patterns in terminal output.
///
/// When connecting through a jump host, the terminal cursor may advance
/// past the detection threshold due to jump host banners or SSH error
/// messages, even though the final destination is unreachable. This
/// function checks for known SSH error strings to avoid false positives.
const SSH_FAILURE_PATTERNS: &[&str] = &[
    "Connection timed out",
    "Connection refused",
    "No route to host",
    "Network is unreachable",
    "Host key verification failed",
    // The banner OpenSSH prints for a changed host key. With the default
    // `StrictHostKeyChecking` it is followed by "Host key verification
    // failed."; with `no` ssh goes on with password and keyboard-interactive
    // logins disabled, so only a later shell prompt counts.
    "REMOTE HOST IDENTIFICATION HAS CHANGED",
    "Permission denied",
    "Too many authentication failures",
    "Connection closed by",
    "Connection reset by",
    "ssh: connect to host",
];

/// Returns `true` if the terminal text contains an SSH connection failure pattern
pub(super) fn contains_ssh_failure(text: &str) -> bool {
    let lower = text.to_lowercase();
    SSH_FAILURE_PATTERNS
        .iter()
        .any(|p| lower.contains(&p.to_lowercase()))
}

/// Returns `true` if the terminal text ends on what looks like an interactive
/// shell prompt.
///
/// A bastion prints a static MOTD/banner the instant the *first* hop connects,
/// long before the final hop is even attempted, and that banner alone can push
/// the cursor several rows down — so cursor position cannot tell a live target
/// from a dead one behind a jump host. A shell prompt can: it only appears once
/// the *final* host has accepted the login and handed over a shell. A failed
/// target prints the bastion banner and then the ProxyCommand's
/// `Connection timed out during banner exchange`, and never a prompt.
///
/// The heuristic is deliberately loose — the last non-blank line ending in one of
/// the common prompt terminators (`$`, `#`, `%`, `>`, `❯`, `➜`), optionally followed by a
/// space or cursor. [`output_established`] consults it for every connection; it
/// is the only evidence it accepts through a jump host. It is always used
/// alongside a failure-pattern scan, so a false positive is bounded by that
/// scan and a false negative simply defers the decision to the next output
/// line.
fn ends_with_shell_prompt(text: &str) -> bool {
    last_non_blank_line(text).is_some_and(is_shell_prompt_line)
}

/// The last line of `text` that holds anything but whitespace.
fn last_non_blank_line(text: &str) -> Option<&str> {
    text.lines().rev().find(|l| !l.trim().is_empty())
}

/// Whether one line looks like an interactive shell prompt; see
/// [`ends_with_shell_prompt`].
///
/// Besides `$ # % >`, `❯` (starship, pure, powerlevel10k) and `➜` (several
/// zsh themes) end common prompts. Neither is ASCII, so ordinary output ends
/// in one even more rarely than in `>` or `%`. A prompt that only *starts*
/// with `➜`, such as oh-my-zsh's default `➜  ~ `, still does not count.
fn is_shell_prompt_line(line: &str) -> bool {
    matches!(
        line.trim_end().chars().last(),
        Some('$' | '#' | '%' | '>' | '❯' | '➜')
    )
}

/// Whether one line is a question for a password, a key passphrase or a
/// one-time code: OpenSSH's `user@host's password: `, `Enter passphrase for
/// key '…': `, `Verification code: `, or a telnet `Password:`.
fn is_credential_question(line: &str) -> bool {
    let lower = line.trim_end().to_lowercase();
    lower.ends_with(':')
        && ["password", "passphrase", "verification code"]
            .iter()
            .any(|word| lower.contains(word))
}

/// Whether one line is OpenSSH's host-key question for a host it does not
/// know yet: `Are you sure you want to continue connecting
/// (yes/no/[fingerprint])? `, or `Please type 'yes', 'no' or the fingerprint: `
/// after an answer it did not take.
///
/// It ends in `? `, so [`is_credential_question`] does not catch it. Until
/// the user answers, nothing is connected, and a background probe that
/// started now could record the key the user is still looking at. Matched
/// anywhere in the line, because the answer typed after it is echoed on the
/// same line.
fn is_host_key_question(line: &str) -> bool {
    let lower = line.to_lowercase();
    lower.contains("(yes/no")
        || lower.contains("continue connecting")
        || lower.contains("please type 'yes', 'no'")
}

/// Whether one line is a question the client waits on: a credential
/// ([`is_credential_question`]) or the host-key confirmation
/// ([`is_host_key_question`]).
fn is_waiting_question(line: &str) -> bool {
    is_credential_question(line) || is_host_key_question(line)
}

/// Whether one line is telnet's `Trying <address>...`: the client is about to
/// try one address, and says nothing yet about whether it will answer.
///
/// telnet tries each address of a name in turn and prints this before each
/// one, so a line like it also means that any failure above it was for an
/// earlier address. `Connected to <host>.` and `Escape character is '^]'.`,
/// which follow once the connection is up, are not progress lines: they are
/// what telnet prints for a connected session, and a device that waits for a
/// key press before its login prompt prints nothing else.
fn is_retry_line(line: &str) -> bool {
    let line = line.trim();
    line.starts_with("Trying ") && line.ends_with("...")
}

/// Whether one line is a client's note on the way to a connection, which is
/// not evidence of one: telnet's `Trying …` ([`is_retry_line`]) and OpenSSH's
/// `Warning: Permanently added '<host>' (…) to the list of known hosts.`,
/// printed before authentication.
fn is_progress_line(line: &str) -> bool {
    let line = line.trim();
    is_retry_line(line)
        || (line.starts_with("Warning: Permanently added ")
            && line.ends_with("to the list of known hosts."))
}

/// The part of `text` after its last shell prompt, credential or host-key
/// question, or telnet retry line.
///
/// The output a status verdict reads only grows, so a failure line anywhere
/// in it would be sticky: OpenSSH prints `Permission denied, please try again.`
/// after a rejected password and then asks again, and a successful retry must
/// still count. Every prompt or question means the client went on after what
/// came before it, so only what follows the last one can be the client's final
/// word; the same holds for telnet's `Trying …` before its next address. A
/// client that fails for good prints its error and exits, with no prompt after
/// it. Scanning only this tail also bounds the work the detector repeats on
/// every change.
fn output_after_last_prompt(text: &str) -> &str {
    let mut tail_start = 0;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        offset += line.len();
        if is_shell_prompt_line(line) || is_waiting_question(line) || is_retry_line(line) {
            tail_start = offset;
        }
    }
    &text[tail_start..]
}

/// The one rule behind both status verdicts ([`ssh_output_established`] and
/// [`client_output_established`]), for a fresh connect and an in-place
/// reconnect alike.
///
/// `output` counts from below RustConn's connect banner. `None` fails closed. A
/// failure (`is_failure`) in the output after the last prompt, credential
/// question or telnet retry line means not established. Otherwise a trailing
/// shell prompt means established, on any row. Without a prompt,
/// `row_suffices` decides whether one full line of output (`cursor_row > 0`)
/// is enough, unless the client is waiting on a credential or host-key
/// question ([`is_waiting_question`]) or its last line is only a progress
/// note ([`is_progress_line`]).
fn output_established(
    cursor_row: i64,
    output: Option<&str>,
    row_suffices: bool,
    is_failure: fn(&str) -> bool,
) -> bool {
    let Some(text) = output else {
        return false;
    };
    if is_failure(output_after_last_prompt(text)) {
        return false;
    }
    if ends_with_shell_prompt(text) {
        return true;
    }
    row_suffices
        && cursor_row > 0
        && !last_non_blank_line(text)
            .is_some_and(|line| is_waiting_question(line) || is_progress_line(line))
}

/// Whether an SSH session is established, from what ssh printed below the
/// connect banner (`TerminalNotebook::display_connect_banner`), given whether
/// it is routed through a jump host.
///
/// Shared by the sidebar "connected" flip, the deferred monitoring start and
/// the session-recording start of both the initial connect and the in-place
/// reconnect (`reconnect_ssh_in_place`), so none of them can drift from the
/// others. `cursor_row` and `output` count from just below the banner, so the
/// banner itself never counts, and a quiet login that prints only a prompt
/// sits on row 0.
///
/// The rule is [`output_established`], with [`contains_ssh_failure`]:
///
/// - Neither path is established while `output` is `None` (unreadable, or the
///   banner not processed yet), or while a known SSH failure follows the last
///   prompt or password question; the caller's closure re-runs on the next
///   output. A `Permission denied, please try again.` before a retry that
///   succeeds does not count.
/// - A trailing shell prompt means established, also on row 0.
/// - Direct connection without a prompt: established once the output reaches
///   a full line (`cursor_row > 0`) and is not waiting on a password,
///   passphrase or host-key question or ending in the known-hosts warning.
/// - Jump-host connection: established only on a shell prompt. The pre-connect
///   port check is skipped for it (the target is not directly reachable), and a
///   bastion banner arrives while the final host is still timing out, so only
///   a prompt proves that the final hop handed over a shell. `output` excludes
///   the kept scrollback, so a prompt or an error from the previous session is
///   not read again.
pub(super) fn ssh_output_established(
    cursor_row: i64,
    uses_jump_host: bool,
    output: Option<&str>,
) -> bool {
    output_established(cursor_row, output, !uses_jump_host, contains_ssh_failure)
}

/// Failure messages the non-SSH terminal clients print before they exit: the
/// Zero Trust CLIs, telnet, picocom, kubectl and mosh. Matched
/// case-insensitively anywhere in a line, like [`SSH_FAILURE_PATTERNS`], which
/// also apply (mosh, gcloud IAP and several brokers run OpenSSH underneath).
/// Each one is specific to the client that prints it; a generic word such as
/// `error:` goes in [`CLIENT_FAILURE_LINE_STARTS`] instead, so the first output
/// of a working session cannot match it by accident.
const CLIENT_FAILURE_PATTERNS: &[&str] = &[
    // AWS CLI without the Session Manager plugin.
    "SessionManagerPlugin is not found",
    // telnet: "Unable to connect to remote host", "could not resolve ..."
    "Unable to connect to remote host",
    "could not resolve",
    "Name or service not known",
    "Did not find mosh server startup message",
    // kubectl: "Unable to connect to the server: dial tcp ...: i/o timeout",
    // "The connection to the server localhost:8080 was refused - ..."
    "Unable to connect to the server",
    "The connection to the server",
];

/// Failure messages that count only at the start of a line (after leading
/// whitespace), matched case-insensitively.
const CLIENT_FAILURE_LINE_STARTS: &[&str] = &[
    // AWS CLI: "An error occurred (TargetNotConnected) when calling ..."
    "An error occurred (",
    // kubectl: "Error from server (NotFound): ...", "error: unable to upgrade ..."
    "Error from server",
    // kubectl "error: ...", gcloud and az "ERROR: ..."
    "error:",
    // picocom: "FATAL: cannot open /dev/ttyUSB0: No such file or directory"
    "FATAL:",
    // zsh: "zsh: command not found: aws"
    "zsh: command not found:",
];

/// Failure messages that count only at the end of a line, matched
/// case-insensitively: bash and sh print `bash: aws: command not found`.
const CLIENT_FAILURE_LINE_ENDS: &[&str] = &[": command not found"];

/// Returns `true` if a client's output contains a known connection failure.
fn contains_client_failure(text: &str) -> bool {
    contains_ssh_failure(text)
        || text.lines().any(|line| {
            let lower = line.trim().to_lowercase();
            CLIENT_FAILURE_PATTERNS
                .iter()
                .any(|p| lower.contains(&p.to_lowercase()))
                || CLIENT_FAILURE_LINE_STARTS
                    .iter()
                    .any(|p| lower.starts_with(&p.to_lowercase()))
                || CLIENT_FAILURE_LINE_ENDS
                    .iter()
                    .any(|p| lower.ends_with(&p.to_lowercase()))
        })
}

/// Whether a Zero Trust, Telnet, Serial, Kubernetes or Mosh session is
/// established, from what its client printed below the connect banner.
///
/// `cursor_row` and `output` both count from just below RustConn's connect
/// banner (`TerminalNotebook::display_connect_banner`), so they describe only
/// what the client printed, on a fresh connect and after an in-place reconnect
/// ([`reconnect_generic_vte_in_place`]) alike. The rule is the one the SSH
/// verdict uses ([`output_established`]), with [`contains_client_failure`]:
/// established on a trailing shell prompt (also on row 0, as with
/// `kubectl exec -it`), or once the output is at least one full line that is
/// not telnet's `Trying …`, unless a known failure follows the last prompt,
/// password question or `Trying …` line. An SSM session prints "Starting
/// session…" and a prompt; a failed one prints the CLI's error and exits.
/// `None` output (unreadable, or the banner not processed yet) fails closed;
/// the detector runs again on the next output.
pub(super) fn client_output_established(cursor_row: i64, output: Option<&str>) -> bool {
    output_established(cursor_row, output, true, contains_client_failure)
}

/// Which client a terminal session's status detector judges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StatusClient {
    /// OpenSSH, directly or through a jump host: [`ssh_output_established`].
    Ssh { jump_host: bool },
    /// Any other terminal client: [`client_output_established`].
    Other,
}

/// The verdict every terminal status detector applies, on a fresh connect and
/// after an in-place reconnect: the sidebar "connected" flip, the deferred
/// monitoring start and the session-recording start.
///
/// A client that has taken over the screen
/// ([`ConnectionOutput::screen_replaced`], such as mosh-client on the
/// alternate screen) is established: a client that fails prints its error on
/// the normal screen and exits. Otherwise the client's own verdict decides.
pub(super) fn connection_established(
    client: StatusClient,
    output: Option<&ConnectionOutput>,
) -> bool {
    output.is_some_and(|output| {
        output.screen_replaced
            || match client {
                StatusClient::Ssh { jump_host } => {
                    ssh_output_established(output.row, jump_host, Some(&output.text))
                }
                StatusClient::Other => client_output_established(output.row, Some(&output.text)),
            }
    })
}

/// Runs `on_established` once the session's current process is established
/// by [`connection_established`].
///
/// The detector stops (and its handler disconnects) once it has run
/// `on_established`, or once a later in-place reconnect replaced the process
/// it was installed for; see `TerminalNotebook::watch_connection_output`.
pub(super) fn when_established(
    notebook: &SharedNotebook,
    session_id: Uuid,
    client: StatusClient,
    on_established: impl FnOnce() + 'static,
) {
    let on_established = std::cell::Cell::new(Some(on_established));
    notebook.watch_connection_output(session_id, move |output| {
        let established = connection_established(client, output);
        if established && let Some(on_established) = on_established.take() {
            on_established();
        }
        established
    });
}

/// Starts a session recording once an in-place reconnect is established, for
/// a connection with session recording on.
///
/// The recording detector a fresh connect arms stands down when a reconnect
/// replaces its process (see [`when_established`]), so without this a session
/// whose first attempt failed was never recorded. A session's recording stops
/// when its process exits, so a reconnected session starts a new one; one
/// that is somehow still running is left alone.
pub(super) fn record_reconnect_when_established(
    notebook: &SharedNotebook,
    session_id: Uuid,
    client: StatusClient,
    conn: &rustconn_core::Connection,
    ssh_params: Option<crate::terminal::SshRecordingParams>,
) {
    if !conn.session_recording_enabled {
        return;
    }
    let notebook_clone = notebook.clone();
    let conn_name = conn.name.clone();
    when_established(notebook, session_id, client, move || {
        if notebook_clone.is_recording(session_id) {
            tracing::debug!(%session_id, "Recording still running after the reconnect");
            return;
        }
        if notebook_clone.start_recording(session_id, &conn_name, ssh_params) {
            tracing::info!(
                %session_id,
                "Auto-recording started after an in-place reconnect"
            );
        }
    });
}

/// Delegates to [`rustconn_core::ssh_tunnel::append_proxy_command_destination`].
pub(super) fn append_proxy_command_destination(proxy_parts: &mut Vec<String>, jump_host: &str) {
    rustconn_core::ssh_tunnel::append_proxy_command_destination(proxy_parts, jump_host);
}

/// Parses a jump-host string (`[user@]host[:port]`) for tunnel ControlPath.
fn parse_jump_host_for_tunnel_control(jump_host: &str) -> (String, u16) {
    let host_port = jump_host
        .rfind('@')
        .map_or(jump_host, |at| &jump_host[at + 1..]);

    let (host, port_str) = if host_port.starts_with('[') {
        if let Some(end) = host_port.find(']') {
            let after = &host_port[end + 1..];
            if let Some(p) = after.strip_prefix(':') {
                (&host_port[1..end], Some(p))
            } else {
                (&host_port[1..end], None)
            }
        } else {
            (host_port, None)
        }
    } else if let Some(pos) = host_port.rfind(':') {
        let p = &host_port[pos + 1..];
        if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) {
            (&host_port[..pos], Some(p))
        } else {
            (host_port, None)
        }
    } else {
        (host_port, None)
    };

    let port = port_str.and_then(|p| p.parse().ok()).unwrap_or(22);
    (host.to_string(), port)
}

/// Resolves the recursive jump host chain for a given connection and returns
/// extra SSH args (`-J` or `-o ProxyCommand`) needed to reach it.
///
/// This is used by SSH tunnel creation (RDP, VNC, SPICE) where the jump host
/// itself may require another jump host to be reachable.
///
/// Returns a `Vec<String>` of extra args to pass to the SSH tunnel command.
/// The first bastion hop for `conn`, honouring the group and global tiers.
///
/// **Every launch path must resolve its first hop through here.** Reading
/// `jump_host_id` off the protocol config instead is the bug this function
/// exists to prevent, and it is not hypothetical: 0.20.9 shipped the three-tier
/// resolver, wired the *free-text* ProxyJump to it everywhere, and left all four
/// launchers reading the picker's field directly. So a Jump Host chosen on a
/// group or in Preferences → Network was stored, shown in the editor as
/// inherited, synced between machines — and dropped at connect time (issue
/// [#301]). The release notes said inheritance "reads the same for every
/// protocol", which was not true of either the picker or of RDP/VNC/SPICE at
/// all.
///
/// This is a thin wrapper on purpose. The decision itself belongs to
/// `rustconn_core::connection::ssh_inheritance::resolve_ssh_jump_host_id`, which
/// is tested and which reads the field from every protocol that has one; what
/// this adds is the two pieces of application state that resolver needs, so a
/// launcher cannot get the answer wrong by forgetting one of them.
///
/// Only the *first* hop is resolved this way. Hops further out keep reading
/// their own field, which matches `resolve_jump_chain` and is deliberate there:
/// a bastion's bastion is a property of that bastion, not something the group of
/// the target should redirect.
///
/// Takes `&AppState` rather than `&SharedAppState` because three of the callers
/// already hold a borrow. Handing it the `RefCell` would make it `try_borrow`
/// again, fail, and answer `None` — silently reintroducing the bug it is here to
/// fix.
///
/// [#301]: https://github.com/totoshko88/RustConn/issues/301
pub fn resolve_first_hop_id(
    state_ref: &crate::state::AppState,
    conn: &rustconn_core::Connection,
) -> Option<uuid::Uuid> {
    let groups = state_ref.list_groups_owned();
    // Failover's answer for this launch wins over the configured primary.
    state_ref.chosen_gateway(conn, &groups).or_else(|| {
        rustconn_core::connection::ssh_inheritance::resolve_ssh_jump_host_id(
            conn,
            &groups,
            &state_ref.settings().network,
        )
    })
}

/// Chooses the jump host for a launch of `connection_id` that has fallbacks.
///
/// Returns `false` when the launch can go ahead now. Returns `true` when probing
/// has started in the background: the caller must stop, and `retry` runs once
/// the choice is recorded, on the main thread, and restarts the launch.
pub(super) fn start_gateway_probe(
    state: &SharedAppState,
    connection_id: Uuid,
    retry: impl FnOnce() + 'static,
) -> bool {
    probe_gateway(state, connection_id, true, retry)
}

/// [`start_gateway_probe`] without the failover toasts, for bulk operations that
/// report per host and would otherwise raise one toast per host.
pub(super) fn start_gateway_probe_quiet(
    state: &SharedAppState,
    connection_id: Uuid,
    retry: impl FnOnce() + 'static,
) -> bool {
    probe_gateway(state, connection_id, false, retry)
}

fn probe_gateway(
    state: &SharedAppState,
    connection_id: Uuid,
    announce: bool,
    retry: impl FnOnce() + 'static,
) -> bool {
    use rustconn_core::connection::gateway_failover::{GatewayOutcome, probe_gateways};

    let Ok(state_ref) = state.try_borrow() else {
        return false;
    };
    if state_ref.gateway_choice_is_fresh(connection_id) {
        return false;
    }
    let Some(conn) = state_ref.get_connection(connection_id) else {
        return false;
    };
    let mut probes = state_ref.gateway_probes(conn);
    if probes.is_empty() {
        state_ref.clear_gateway_choice(connection_id);
        return false;
    }
    let timeout = state_ref.settings().connection.port_check_timeout_secs;
    drop(state_ref);

    let globals = resolve_globals_with_ask(state, connection_id);
    for (host, _) in probes.iter_mut().filter_map(|p| p.entry_point.as_mut()) {
        *host = substitute_variables(host, &globals);
    }

    let state = state.clone();
    crate::utils::spawn_blocking_with_callback(
        move || probe_gateways(&probes, timeout),
        move |selection| {
            if let Some(selection) = selection {
                state
                    .borrow()
                    .set_gateway_choice(connection_id, selection.gateway_id);
                match selection.outcome {
                    _ if !announce => {}
                    GatewayOutcome::Primary => {}
                    GatewayOutcome::FailedOver => {
                        tracing::info!(gateway = %selection.gateway_name, "Failing over to a fallback jump host");
                        crate::toast::show_warning_toast_on_active_window(&i18n_f(
                            "Jump host unreachable, connecting through ‘{}’",
                            &[&selection.gateway_name],
                        ));
                    }
                    GatewayOutcome::NoneReachable => {
                        tracing::warn!(gateway = %selection.gateway_name, "No jump host responded to the probe");
                        crate::toast::show_warning_toast_on_active_window(&i18n_f(
                            "No jump host responded, trying ‘{}’",
                            &[&selection.gateway_name],
                        ));
                    }
                }
            }
            retry();
        },
    );
    true
}

/// The `ssh` arguments that take a tunnel to `jump_conn` through the jump hosts
/// past it.
///
/// `tunnel_has_password` is true when the tunnel answers `jump_conn`'s password
/// through its askpass helper. Those hops then go through
/// [`rustconn_core::jump_route::tunnel_hop_args`], so none of them inherits
/// that helper. When they cannot (an address a shell could interpret), the
/// usual route is returned and `create_tunnel` does not send the password.
pub fn resolve_jump_chain_for_tunnel(
    state_ref: &crate::state::AppState,
    jump_conn: &rustconn_core::Connection,
    tunnel_has_password: bool,
    hop_host_keys: rustconn_core::ssh_tunnel::HopHostKeys,
) -> Vec<String> {
    let groups: Vec<rustconn_core::ConnectionGroup> = state_ref.list_groups_owned();
    let network = state_ref.settings().network.clone();

    // Check if the jump host itself has a jump host (recursive chain)
    let ssh_config = match &jump_conn.protocol_config {
        rustconn_core::ProtocolConfig::Ssh(cfg) => cfg,
        _ => return Vec::new(),
    };

    // Collect the chain of jump hosts above the immediate jump host
    let mut chain: Vec<String> = Vec::new();

    // First check for string-based proxy_jump on the jump host
    if let Some(proxy) = rustconn_core::connection::ssh_inheritance::resolve_ssh_proxy_jump(
        jump_conn, &groups, &network,
    ) {
        chain.push(proxy);
    }

    // Then resolve reference-based jump hosts recursively
    // Also resolve the identity file for the first hop (needed for ProxyCommand)
    let mut first_hop_identity: Option<String> = None;
    // PKCS#11 provider of the first hop (not inherited via -J — inject into ProxyCommand)
    let mut first_hop_pkcs11: Option<String> = None;
    if let Some(parent_jump_id) = ssh_config.jump_host_id {
        let mut current_id = Some(parent_jump_id);
        let mut visited = std::collections::HashSet::new();
        visited.insert(jump_conn.id); // Avoid self-reference
        let mut is_first = true;

        for _ in 0..10 {
            if let Some(jid) = current_id {
                if visited.contains(&jid) {
                    break;
                }
                visited.insert(jid);

                if let Some(parent_conn) = state_ref.get_connection(jid) {
                    // Resolve identity file for the first hop
                    if is_first {
                        first_hop_identity =
                            rustconn_core::connection::ssh_inheritance::resolve_ssh_key_path(
                                parent_conn,
                                &groups,
                            )
                            .and_then(|p| rustconn_core::resolve_key_path(&p))
                            .map(|p| p.to_string_lossy().to_string());
                        if let rustconn_core::ProtocolConfig::Ssh(parent_cfg) =
                            &parent_conn.protocol_config
                        {
                            first_hop_pkcs11 = parent_cfg
                                .pkcs11_provider
                                .clone()
                                .filter(|p| !p.trim().is_empty());
                        }
                        is_first = false;
                    }

                    // Format: [user@]host[:port] for -J
                    let mut host_str = parent_conn.host.clone();
                    if let Some(user) = &parent_conn.username {
                        host_str = format!("{user}@{host_str}");
                    }
                    if parent_conn.port != 22 {
                        host_str = format!("{host_str}:{}", parent_conn.port);
                    }
                    chain.push(host_str);

                    // Continue up the chain
                    if let rustconn_core::ProtocolConfig::Ssh(parent_cfg) =
                        &parent_conn.protocol_config
                    {
                        if let Some(p) = &parent_cfg.proxy_jump {
                            chain.insert(chain.len() - 1, p.clone());
                        }
                        current_id = parent_cfg.jump_host_id;
                    } else {
                        current_id = None;
                    }
                } else {
                    current_id = None;
                }
            } else {
                break;
            }
        }
    }

    if chain.is_empty() {
        return Vec::new();
    }

    // In Flatpak, use ProxyCommand so the nested SSH inherits known_hosts
    let flatpak_kh = rustconn_core::get_flatpak_known_hosts_path();
    let proxy_command_route = flatpak_kh.is_some() || first_hop_pkcs11.is_some();
    if tunnel_has_password {
        // The deeper hops get what the routes below give them: the first
        // hop's key only where those use a ProxyCommand.
        let request = rustconn_core::jump_route::TunnelHopsRequest {
            hops: &chain,
            first_hop_identity: first_hop_identity.as_deref(),
            deeper_hop_identity: first_hop_identity
                .as_deref()
                .filter(|_| proxy_command_route),
            first_hop_pkcs11: first_hop_pkcs11.as_deref(),
            known_hosts: flatpak_kh.as_deref(),
            hop_host_keys,
        };
        if let Some(args) = rustconn_core::jump_route::tunnel_hop_args(&request) {
            return args;
        }
        tracing::warn!(
            "Tunnel: a jump host past the first has an address RustConn does not pass to a \
             shell, so the first jump host's password is not sent"
        );
    }
    if proxy_command_route {
        // Build ProxyCommand for the first hop in the chain. `-J` does not pass
        // -o/-i to the nested SSH, so Flatpak known_hosts and PKCS#11 tokens must
        // be injected explicitly here.
        let mut proxy_parts = vec!["ssh".to_string(), "-W".to_string(), "%h:%p".to_string()];
        if let Some(option) = flatpak_kh
            .as_deref()
            .and_then(rustconn_core::jump_route::proxy_known_hosts_option)
        {
            proxy_parts.push("-o".to_string());
            proxy_parts.push(option);
        }

        // Pass identity file for the first hop if available
        if let Some(ref key) = first_hop_identity {
            proxy_parts.push("-i".to_string());
            proxy_parts.push(key.clone());
            proxy_parts.push("-o".to_string());
            proxy_parts.push("IdentitiesOnly=yes".to_string());
        }

        // Pass PKCS#11 provider for the first hop (token also auths the bastion)
        if let Some(ref provider) = first_hop_pkcs11 {
            proxy_parts.push("-o".to_string());
            proxy_parts.push(format!("PKCS11Provider={}", provider.trim()));
        }

        // Reuse the jump host's ControlMaster socket so parallel tunnel
        // establishments share the already-authenticated link. Only set
        // ControlPath — if the master exists, SSH multiplexes; if not, it
        // connects standalone (no auth issues from missing identity).
        {
            let (jh_host, jh_port) = parse_jump_host_for_tunnel_control(&chain[0]);
            let jh_control = rustconn_core::ssh_control_path(&jh_host, jh_port);
            proxy_parts.push("-o".to_string());
            proxy_parts.push(format!("ControlPath={jh_control}"));
        }

        // ponytail: PKCS#11/identity reach only the first hop; deeper hops do
        // not get the bastion's own PKCS#11 token. Fine for the common
        // single-bastion case.
        //
        // Multi-hop: nest a ProxyCommand per remaining hop so EACH inherits the
        // identity file and Flatpak known_hosts. Plain `-J b,c` would drop them
        // and the deeper hops fail in Flatpak (issue #191 follow-up — double jump).
        if chain.len() > 1 {
            let inner_hops: Vec<&str> = chain[1..].iter().map(String::as_str).collect();
            let inner = rustconn_core::ssh_tunnel::build_nested_proxy_command(
                &inner_hops,
                first_hop_identity.as_deref(),
                flatpak_kh.as_deref(),
                rustconn_core::ssh_tunnel::HopHostKeys::UserConfig,
            );
            proxy_parts.push("-o".to_string());
            proxy_parts.push(format!(
                "ProxyCommand={}",
                rustconn_core::ssh_tunnel::shell_single_quote(&inner)
            ));
        }

        // Add the first hop destination with proper -p port parsing
        append_proxy_command_destination(&mut proxy_parts, &chain[0]);

        let proxy_cmd = proxy_parts.join(" ");
        tracing::debug!(
            proxy_command = %proxy_cmd,
            "Tunnel: using ProxyCommand for jump host chain (Flatpak known_hosts or PKCS#11)"
        );
        vec!["-o".to_string(), format!("ProxyCommand={proxy_cmd}")]
    } else {
        // Non-Flatpak: use standard -J. RustConn resolves chains target-first,
        // but OpenSSH `-J` visits hops client-first, so reverse them.
        let j_chain = rustconn_core::ssh_tunnel::proxy_jump_arg(&chain.join(","));
        tracing::debug!(
            jump_chain = %j_chain,
            "Tunnel: using -J for jump host chain"
        );
        vec!["-J".to_string(), j_chain]
    }
}

/// Starts VNC and observes a session created after asynchronous setup.
pub fn start_vnc_connection_observed(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
    observer: Option<super::types::SessionStartObserver>,
) -> Option<Uuid> {
    // Check if port check is needed — skip when jump host is configured
    let settings = state.borrow().settings().clone();
    let should_check = conn.should_pre_connect_check(&settings.connection);

    if should_check {
        let host = conn.host.clone();
        let port = conn.port;
        let timeout = settings.connection.port_check_timeout_secs;
        let state_clone = state.clone();
        let notebook_clone = notebook.clone();
        let sidebar_clone = sidebar.clone();
        let conn_clone = conn.clone();

        // Run port check in background thread
        spawn_blocking_with_callback(
            move || check_port(&host, port, timeout),
            move |result| {
                match result {
                    Ok(_) => {
                        // Port is open, proceed with connection
                        start_vnc_connection_internal(
                            &state_clone,
                            &notebook_clone,
                            &sidebar_clone,
                            connection_id,
                            &conn_clone,
                            observer,
                        );
                    }
                    Err(e) => {
                        // Port check failed, show error with retry
                        tracing::warn!("Port check failed for VNC connection: {e}");
                        sidebar_clone
                            .update_connection_status(&connection_id.to_string(), "failed");
                        if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                            state_mut.record_connection_attempt_failed(
                                &conn_clone,
                                conn_clone.username.as_deref(),
                                &e.to_string(),
                            );
                        }
                        if let Some(root) = notebook_clone.widget().root()
                            && let Some(window) = root.downcast_ref::<gtk4::Window>()
                        {
                            crate::toast::show_retry_toast_on_window(
                                window,
                                &e.to_string(),
                                &connection_id.to_string(),
                            );
                        }
                    }
                }
            },
        );
        // Return None since the actual session will be created asynchronously
        None
    } else {
        // Port check disabled, proceed directly
        start_vnc_connection_internal(state, notebook, sidebar, connection_id, conn, observer)
    }
}

/// Internal function to start VNC connection (after port check)
fn start_vnc_connection_internal(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
    observer: Option<super::types::SessionStartObserver>,
) -> Option<Uuid> {
    use rustconn_core::models::{VncClientMode, WindowMode};

    let conn_name = conn.name.clone();
    let port = conn.port;
    let window_mode = conn.window_mode;

    // Get global variables for substitution (secret values resolved from vault),
    // overlaid with this connection's interactive `@ask:` answers.
    let global_variables = resolve_globals_with_ask(state, connection_id);

    // Apply variable substitution to host
    let host = substitute_variables(&conn.host, &global_variables);

    // Get VNC-specific configuration
    let mut vnc_config = if let rustconn_core::ProtocolConfig::Vnc(config) = &conn.protocol_config {
        config.clone()
    } else {
        rustconn_core::models::VncConfig::default()
    };

    // Apply window_mode: External forces external viewer
    if window_mode == WindowMode::External {
        vnc_config.client_mode = VncClientMode::External;
        tracing::info!(
            protocol = "vnc",
            host = %host,
            "Window mode is External, using external VNC viewer"
        );
    }

    // Issue #209: an external-viewer VNC session gets no notebook tab. Spawn the
    // viewer and register it so the sidebar surfaces it without a dead tab
    // (R1.1). The password is handled by the viewer, never on the command line.
    if conn.uses_external_viewer() {
        let Some(viewer) =
            rustconn_core::protocol::resolve_vnc_viewer(vnc_config.vnc_viewer_override.as_deref())
        else {
            tracing::error!(connection = %conn_name, "No external VNC viewer installed");
            crate::toast::show_error_toast_on_active_window(&i18n(
                "No VNC viewer found. Install TigerVNC or Remmina.",
            ));
            sidebar.update_connection_status(&connection_id.to_string(), "failed");
            return None;
        };
        let (program, args) = rustconn_core::protocol::VncProtocol::build_external_viewer_command(
            &viewer,
            &host,
            port,
            &vnc_config,
        );
        spawn_and_register_external_viewer(
            state,
            notebook,
            sidebar,
            connection_id,
            conn,
            &program,
            &args,
            None,
        );
        return None;
    }

    // Get password from cached credentials (set by credential resolution flow)
    let password: Option<zeroize::Zeroizing<String>> =
        state.try_borrow().ok().and_then(|state_ref| {
            state_ref.get_cached_credentials(connection_id).map(|c| {
                use secrecy::ExposeSecret;
                tracing::debug!("[VNC] Found cached credentials for connection");
                zeroize::Zeroizing::new(c.password.expose_secret().to_string())
            })
        });

    tracing::debug!(
        "[VNC] Password available: {}",
        if password.is_some() { "yes" } else { "no" }
    );

    // Create VNC session tab with native widget
    let session_id = notebook.create_vnc_session_tab(connection_id, &conn_name);
    if let Some(observer) = observer {
        observer.complete(session_id);
    }

    // Record connection start in history
    let history_entry_id = if let Ok(mut state_mut) = state.try_borrow_mut() {
        Some(state_mut.record_connection_start(conn, conn.username.as_deref()))
    } else {
        None
    };

    // Store history entry ID in session for later use
    if let Some(entry_id) = history_entry_id {
        notebook.set_history_entry_id(session_id, entry_id);
    }

    // Get the VNC widget and initiate connection with config
    if let Some(vnc_widget) = notebook.get_vnc_widget(session_id) {
        // Connect state change callback to mark tab as disconnected when session ends
        let notebook_for_state = notebook.clone();
        let sidebar_for_state = sidebar.clone();
        let state_for_callback = state.clone();
        vnc_widget.connect_state_changed(move |vnc_state| {
            if vnc_state == crate::session::SessionState::Disconnected {
                notebook_for_state.stop_recording(session_id);
                notebook_for_state.mark_tab_disconnected(session_id);
                sidebar_for_state.decrement_session_count(
                    &connection_id.to_string(),
                    session_id,
                    false,
                );
                // Record connection end in history
                if let Some(info) = notebook_for_state.get_session_info(session_id)
                    && let Some(entry_id) = info.history_entry_id
                    && let Ok(mut state_mut) = state_for_callback.try_borrow_mut()
                {
                    state_mut.record_connection_end(entry_id);
                }
            } else if vnc_state == crate::session::SessionState::Connected {
                notebook_for_state.mark_tab_connected(session_id);
                sidebar_for_state.increment_session_count(&connection_id.to_string(), session_id);
            }
        });

        // Connect reconnect callback
        let widget_for_reconnect = vnc_widget.clone();
        vnc_widget.connect_reconnect(move || {
            if let Err(e) = widget_for_reconnect.reconnect() {
                tracing::error!(%e, "VNC reconnect failed");
            }
        });

        // Initiate connection with VNC config (respects client_mode setting)
        if let Err(e) = vnc_widget.connect_with_config(
            &host,
            port,
            password.as_ref().map(|p| p.as_str()),
            &vnc_config,
        ) {
            tracing::error!(%e, conn_name, "Failed to connect VNC session");
            sidebar.update_connection_status(&connection_id.to_string(), "failed");
        } else {
            sidebar.update_connection_status(&connection_id.to_string(), "connecting");
        }
    }

    // If Fullscreen mode, maximize the window (same pattern as RDP)
    if matches!(window_mode, WindowMode::Fullscreen)
        && let Some(window) = notebook
            .widget()
            .ancestor(gtk4::ApplicationWindow::static_type())
        && let Some(app_window) = window.downcast_ref::<gtk4::ApplicationWindow>()
    {
        app_window.maximize();
    }

    // Update last_connected timestamp
    if let Ok(mut state_mut) = state.try_borrow_mut()
        && let Err(e) = state_mut.update_last_connected(connection_id)
    {
        tracing::warn!(?e, "Failed to update last_connected");
    }

    Some(session_id)
}

/// Starts a SPICE connection
///
/// Creates a SPICE session tab with native widget and initiates connection.
pub fn start_spice_connection(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
) -> Option<Uuid> {
    // Check if port check is needed — centralized probe-bypass logic
    let settings = state.borrow().settings().clone();
    let should_check = conn.should_pre_connect_check(&settings.connection);

    if should_check {
        let host = conn.host.clone();
        let port = conn.port;
        let timeout = settings.connection.port_check_timeout_secs;
        let state_clone = state.clone();
        let notebook_clone = notebook.clone();
        let sidebar_clone = sidebar.clone();
        let conn_clone = conn.clone();

        // Run port check in background thread
        spawn_blocking_with_callback(
            move || check_port(&host, port, timeout),
            move |result| {
                match result {
                    Ok(_) => {
                        // Port is open, proceed with connection
                        start_spice_connection_internal(
                            &state_clone,
                            &notebook_clone,
                            &sidebar_clone,
                            connection_id,
                            &conn_clone,
                        );
                    }
                    Err(e) => {
                        // Port check failed, show error with retry
                        tracing::warn!("Port check failed for SPICE connection: {e}");
                        sidebar_clone
                            .update_connection_status(&connection_id.to_string(), "failed");
                        if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                            state_mut.record_connection_attempt_failed(
                                &conn_clone,
                                conn_clone.username.as_deref(),
                                &e.to_string(),
                            );
                        }
                        if let Some(root) = notebook_clone.widget().root()
                            && let Some(window) = root.downcast_ref::<gtk4::Window>()
                        {
                            crate::toast::show_retry_toast_on_window(
                                window,
                                &e.to_string(),
                                &connection_id.to_string(),
                            );
                        }
                    }
                }
            },
        );
        // Return None since the actual session will be created asynchronously
        None
    } else {
        // Port check disabled, proceed directly
        start_spice_connection_internal(state, notebook, sidebar, connection_id, conn)
    }
}

/// Internal function to start SPICE connection (after port check)
fn start_spice_connection_internal(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
) -> Option<Uuid> {
    use rustconn_core::spice_client::{
        SpiceClientConfig, build_spice_viewer_args, detect_spice_viewer,
    };

    let conn_name = conn.name.clone();
    let port = conn.port;

    // Get global variables for substitution (secret values resolved from vault),
    // overlaid with this connection's interactive `@ask:` answers.
    let global_variables = resolve_globals_with_ask(state, connection_id);

    // Apply variable substitution to host
    let host = substitute_variables(&conn.host, &global_variables);

    // Get SPICE-specific options from connection config
    let spice_opts = if let rustconn_core::ProtocolConfig::Spice(config) = &conn.protocol_config {
        Some(config.clone())
    } else {
        None
    };

    // --- SSH tunnel for jump host ---
    // Unix-socket mode connects locally, so the jump host is ignored (no tunnel).
    //
    // The hop is resolved through the three tiers rather than read off `opts`, as
    // for RDP and VNC and for the same reason (#301). That is why the borrow has
    // moved out of the inner `if`: resolving needs the group list and the network
    // settings, so it has to happen before the hop id is known rather than after.
    let (effective_host, effective_port, ssh_tunnel) = if let Some(ref opts) = spice_opts
        && opts.unix_socket_path.is_none()
    {
        if let Ok(state_ref) = state.try_borrow()
            && let Some(jump_id) = resolve_first_hop_id(&state_ref, conn)
            && let Some(jump_conn) = state_ref.get_connection(jump_id)
        {
            let mut jump_dest = jump_conn.host.clone();
            if let Some(user) = &jump_conn.username {
                jump_dest = format!("{user}@{}", jump_dest);
            }
            let jump_port = jump_conn.port;
            // Resolve key path via inheritance (connection → group → parent group → root)
            let groups: Vec<rustconn_core::models::ConnectionGroup> = state_ref.list_groups_owned();
            let identity_file = ssh_inheritance::resolve_ssh_key_path(jump_conn, &groups)
                .and_then(|p| rustconn_core::resolve_key_path(&p))
                .map(|p| p.to_string_lossy().to_string());

            let password = state_ref
                .get_cached_credentials(jump_id)
                .filter(|c| {
                    use secrecy::ExposeSecret;
                    !c.password.expose_secret().is_empty()
                })
                .map(|c| c.password.clone());
            // Resolve recursive jump host chain (e.g. jump_conn itself needs a jump host)
            let extra_args = resolve_jump_chain_for_tunnel(
                &state_ref,
                jump_conn,
                password.is_some(),
                rustconn_core::ssh_tunnel::HopHostKeys::UserConfig,
            );

            let params = rustconn_core::ssh_tunnel::SshTunnelParams {
                jump_host: jump_dest,
                jump_port,
                remote_host: host.clone(),
                remote_port: port,
                identity_file,
                password,
                extra_args,
                launcher: rustconn_core::ssh_tunnel::TunnelLauncher::OpenSsh,
            };

            drop(state_ref);

            match rustconn_core::ssh_tunnel::create_tunnel(&params) {
                Ok(mut tunnel) => {
                    let local_port = tunnel.local_port();
                    tracing::info!(
                        %connection_id,
                        local_port,
                        "SSH tunnel established for SPICE connection"
                    );
                    // Wait for tunnel to accept connections
                    if let Err(e) = rustconn_core::ssh_tunnel::wait_for_tunnel_ready(
                        &mut tunnel,
                        40,
                        std::time::Duration::from_millis(250),
                    ) {
                        tracing::error!(%e, "SSH tunnel not ready for SPICE");
                        sidebar.update_connection_status(&connection_id.to_string(), "failed");
                        return None;
                    }

                    // Verify remote SPICE port is reachable through the tunnel
                    if let Err(e) = rustconn_core::ssh_tunnel::probe_tunnel_remote(
                        &mut tunnel,
                        std::time::Duration::from_secs(5),
                    ) {
                        tracing::error!(%e, "Remote SPICE port unreachable through SSH tunnel");
                        sidebar.update_connection_status(&connection_id.to_string(), "failed");
                        return None;
                    }

                    ("127.0.0.1".to_string(), local_port, Some(tunnel))
                }
                Err(e) => {
                    tracing::error!(%e, "Failed to create SSH tunnel for SPICE");
                    sidebar.update_connection_status(&connection_id.to_string(), "failed");
                    return None;
                }
            }
        } else {
            // Reached either because no bastion resolved at any tier — the common
            // case, and not worth a warning — or because one resolved to an id
            // that is no longer in the connection list. The id is not available
            // here now that resolution happens inside the condition; the resolver
            // logs nothing either, so a deleted bastion shows up as a direct
            // connection attempt. Acceptable while the editor cannot store a
            // dangling reference, and the reason this is `debug` rather than
            // `warn`: it fires on every SPICE connection without a bastion.
            tracing::debug!(%connection_id, "No jump host in effect for SPICE");
            (host, port, None)
        }
    } else {
        (host, port, None)
    };

    // Issue #209: SPICE always renders in an external viewer (the embedded
    // client was removed in 0.18.0, so `uses_external_viewer()` is always true
    // here). The launch is therefore unconditional: spawn remote-viewer and
    // register it so the sidebar surfaces the session without a dead notebook
    // tab (R1.1). SPICE arg building lives in rustconn-core (single source of
    // truth); `spawn_and_register_external_viewer` records the history start.
    let Some(viewer) = detect_spice_viewer() else {
        tracing::error!(connection = %conn_name, "No SPICE viewer installed");
        crate::toast::show_error_toast_on_active_window(&i18n(
            "No SPICE viewer found. Install virt-viewer.",
        ));
        sidebar.update_connection_status(&connection_id.to_string(), "failed");
        return None;
    };
    let mut config = SpiceClientConfig::new(&effective_host).with_port(effective_port);
    if let Some(ref opts) = spice_opts {
        config = config.with_tls(opts.tls_enabled);
        if let Some(ca_path) = &opts.ca_cert_path {
            config = config.with_ca_cert(ca_path);
        }
        config = config
            .with_skip_cert_verify(opts.skip_cert_verify)
            .with_usb_redirection(opts.usb_redirection)
            .with_clipboard(opts.clipboard_enabled);
        config.show_local_cursor = opts.show_local_cursor;
        if let Some(ref socket_path) = opts.unix_socket_path {
            config = config.with_unix_socket(socket_path);
        }
        // Proxmox VE proxy tunnelling: the editor stores `proxy`, but the launch
        // mapping dropped it, so `--spice-proxy` was never emitted and the
        // Proxmox-only proxy field was silently dead. Thread it through here.
        if let Some(ref proxy) = opts.proxy {
            config = config.with_proxy(proxy.clone());
        }
        // Shared folders (webdav) were likewise stored but never mapped, so the
        // configured shares never reached the viewer. The persisted `SharedFolder`
        // carries local_path + share_name; the client type defaults read_only=false.
        for folder in &opts.shared_folders {
            config =
                config.with_shared_folder(rustconn_core::spice_client::SpiceSharedFolder::new(
                    folder.local_path.clone(),
                    folder.share_name.clone(),
                ));
        }
    }

    // Issue #308: hand remote-viewer the resolved password so it stops
    // re-prompting on every connect. The credential is resolved and cached
    // before this function runs (see `window/credentials.rs`, where SPICE is
    // handled in the same arm as SSH), but the launch never read it back. The
    // password cannot go on argv — `/proc/<pid>/cmdline` would expose it to
    // every process of the same user — so it travels in a mode-0600 `.vv`
    // connection file the same way RDP uses `/args-from:`. `build_vv_connection_file`
    // returns `None` for a unix socket or an empty password, in which case we
    // fall through to the plain URI args and the viewer prompts as before.
    //
    // Ownership of the file is handed over explicitly below: the spawn only
    // forks the viewer, which opens its connection file some milliseconds later,
    // so removing the file when the spawn call returns would delete it before it
    // is read. A viewer that started owns the deletion through
    // `delete-this-file=1`; a spawn that failed leaves it to the guard's `Drop`.
    if let Some(password) = cached_connection_password(state, connection_id) {
        use secrecy::ExposeSecret;
        config = config.with_password(password.expose_secret());
    }
    let vv_file = rustconn_core::spice_client::build_vv_connection_file(&config)
        .and_then(|contents| super::command_env::EphemeralVvFile::write(&contents));

    // A `host:` viewer runs outside the sandbox, and the sandbox's
    // `$XDG_RUNTIME_DIR` is not the host's despite looking like it, so the path
    // the file was written to means nothing to that process. Resolve the path it
    // *can* open before handing it over; `None` means the file cannot be
    // delivered, and passing an unopenable path is worse than not passing one at
    // all — `remote-viewer` falls back to reading the argument as a URI and dies
    // with "connection type cannot be detected from URI" (issue #308). Dropping
    // to the URI args instead costs a password prompt and keeps the session
    // working, which is how every release before 0.21.2 behaved.
    let vv_argument = vv_file
        .as_ref()
        .and_then(|file| rustconn_core::host_visible_path(file.path()));
    if vv_file.is_some() && vv_argument.is_none() {
        tracing::warn!(
            connection = %conn_name,
            "the SPICE connection file is not reachable by the host viewer; \
             falling back to the URI, so the viewer will ask for the password"
        );
    }

    let args = if let Some(ref path) = vv_argument {
        // The .vv file carries host/port/TLS/password, so it replaces the URI.
        // The connection-independent flags (title, USB, shared folders) still go
        // on argv next to it — remote-viewer accepts both together.
        let mut args = vec![path.to_string_lossy().to_string()];
        args.extend(rustconn_core::spice_client::build_spice_extra_flags(
            &config,
        ));
        args
    } else {
        build_spice_viewer_args(&config)
    };

    let spawned = spawn_and_register_external_viewer(
        state,
        notebook,
        sidebar,
        connection_id,
        conn,
        &viewer,
        &args,
        ssh_tunnel,
    );
    if let Some(vv_file) = vv_file {
        // Ownership passes to the viewer only if it both started *and* was given
        // the file: `delete-this-file=1` is what removes it, and a viewer that
        // never opened it will never act on that. Releasing on `spawned` alone
        // would leave a password on disk for the whole session whenever the path
        // could not be translated for a host viewer.
        if spawned && vv_argument.is_some() {
            // The viewer is running and will remove the file itself.
            vv_file.release_to_viewer();
        } else {
            // Nothing will ever read it — drop removes it now.
            drop(vv_file);
        }
    }
    None
}

/// Applies a terminal session's highlight rules: the built-in rules (unless
/// Settings turned them off), the global rules from Settings, then
/// `per_conn_rules`.
///
/// Every terminal launch and reconnect path comes through here. When this was
/// seven hand-copied blocks, Zero Trust and Quick Connect sessions were the ones
/// left without highlighting at all (issue #343).
pub(crate) fn apply_highlight_rules(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    session_id: Uuid,
    per_conn_rules: &[rustconn_core::models::HighlightRule],
) {
    let (global_rules, include_builtin_defaults) = state
        .try_borrow()
        .ok()
        .map(|s| {
            (
                s.settings().highlight_rules.clone(),
                !s.settings().highlight_builtin_defaults_disabled,
            )
        })
        .unwrap_or((Vec::new(), true));
    notebook.set_highlight_rules(
        session_id,
        &global_rules,
        per_conn_rules,
        include_builtin_defaults,
    );
}

/// Reconnects an SSH session in-place, reusing the existing terminal tab.
///
/// Instead of closing the old tab and creating a new one (which disrupts
/// tab ordering when managing 10+ sessions), this function:
/// 1. Prepares the existing tab (removes banner, resets VTE)
/// 2. Re-applies highlight rules and automation
/// 3. Re-spawns the SSH process in the same terminal
/// 4. Re-wires password injection, status detection, and monitoring
///
/// Returns `true` if reconnect was initiated, `false` if the tab no longer exists.
pub fn reconnect_generic_vte_in_place(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    session_id: Uuid,
    connection_id: Uuid,
) -> bool {
    use rustconn_core::protocol::{
        KubernetesProtocol, MoshProtocol, Protocol, SerialProtocol, format_command_message,
        format_connection_message,
    };

    if !notebook.prepare_for_reconnect(session_id) {
        tracing::warn!(%session_id, "Tab no longer exists, cannot reconnect in-place");
        return false;
    }

    // Show "connecting" status in sidebar immediately
    sidebar.update_connection_status(&connection_id.to_string(), "connecting");

    let conn = {
        let Ok(state_ref) = state.try_borrow() else {
            return false;
        };
        match state_ref.get_connection(connection_id) {
            Some(c) => c.clone(),
            None => return false,
        }
    };

    // Re-apply highlight rules
    apply_highlight_rules(state, notebook, session_id, &conn.highlight_rules);

    // Re-register the output filter, for the same reason the SSH reconnect path
    // does: the map survives a reconnect (it is keyed by session and cleared only
    // when the tab closes), so without this a filter edited between connect and
    // reconnect is honoured for SSH and silently ignored for every protocol that
    // reconnects through here.
    notebook.set_output_filter(session_id, conn.postpend.as_ref());

    // Record connection start in history
    if let Ok(mut state_mut) = state.try_borrow_mut() {
        let entry_id = state_mut.record_connection_start(&conn, conn.username.as_deref());
        notebook.set_history_entry_id(session_id, entry_id);
    }

    // Re-wire child-exited handler
    MainWindow::setup_child_exited_handler(state, notebook, sidebar, session_id, connection_id);

    // Build and spawn command based on protocol
    match &conn.protocol_config {
        rustconn_core::ProtocolConfig::ZeroTrust(zt_config) => {
            let global_variables = resolve_globals_with_ask(state, connection_id);
            // Resolve from the vault on a cache miss, like the SSH reconnect
            // path: a Custom Command referencing `${password}` reconnected after
            // the cache TTL expired used to expand to an empty password (issue
            // #330). No other AppState borrow is held across this blocking call.
            let password = state
                .try_borrow_mut()
                .ok()
                .and_then(|mut s| s.ensure_connection_password(connection_id));
            let launch = match build_zerotrust_launch(
                &conn,
                zt_config,
                &global_variables,
                password.as_ref(),
            ) {
                Ok(launch) => launch,
                Err(e) => {
                    tracing::error!(?e, connection = %conn.name, "Failed to expand custom command");
                    notebook.display_output(
                        session_id,
                        &format!(
                            "\r\n{}\r\n",
                            i18n_f("Invalid variable: {}", &[&e.to_string()])
                        ),
                    );
                    return false;
                }
            };

            let conn_msg = format_connection_message(zt_config.provider.display_name(), &conn.name);
            let cmd_msg = format_command_message(&launch.display);
            notebook
                .display_connect_banner(session_id, &format!("{conn_msg}\r\n{cmd_msg}\r\n\r\n"));

            let argv: Vec<&str> = launch.argv.iter().map(String::as_str).collect();
            let env = launch.env_refs();
            let envv = (!env.is_empty()).then_some(env.as_slice());
            notebook.spawn_command(session_id, &argv, envv, None, None);
        }
        rustconn_core::ProtocolConfig::Telnet(telnet_config) => {
            let conn_msg = format_connection_message("Telnet", &conn.host);
            let cmd_msg = format_command_message(&format!("telnet {} {}", conn.host, conn.port));
            notebook
                .display_connect_banner(session_id, &format!("{conn_msg}\r\n{cmd_msg}\r\n\r\n"));

            notebook.spawn_telnet(
                session_id,
                &conn.host,
                conn.port,
                &[],
                telnet_config.backspace_sends,
                telnet_config.delete_sends,
            );
        }
        rustconn_core::ProtocolConfig::WinRm(_) => {
            let globals = resolve_globals_with_ask(state, connection_id);
            if !launch_winrm(
                state,
                notebook,
                sidebar,
                session_id,
                connection_id,
                &conn,
                &globals,
            ) {
                return false;
            }
        }
        rustconn_core::ProtocolConfig::Serial(_) => {
            let serial = SerialProtocol::new();
            let Some(command) = serial.build_command(&conn) else {
                tracing::warn!(%session_id, "Failed to build Serial command for reconnect");
                return false;
            };
            let serial_config =
                if let rustconn_core::ProtocolConfig::Serial(ref cfg) = conn.protocol_config {
                    cfg
                } else {
                    return false;
                };

            let conn_msg = format_connection_message("Serial", &serial_config.device);
            let cmd_msg = format_command_message(&command.join(" "));
            notebook
                .display_connect_banner(session_id, &format!("{conn_msg}\r\n{cmd_msg}\r\n\r\n"));

            notebook.spawn_serial(session_id, &command);
        }
        rustconn_core::ProtocolConfig::Kubernetes(_) => {
            let k8s = KubernetesProtocol::new();
            let Some(command) = k8s.build_command(&conn) else {
                tracing::warn!(%session_id, "Failed to build Kubernetes command for reconnect");
                return false;
            };

            let conn_msg = format_connection_message("Kubernetes", &conn.name);
            let cmd_msg = format_command_message(&command.join(" "));
            notebook
                .display_connect_banner(session_id, &format!("{conn_msg}\r\n{cmd_msg}\r\n\r\n"));

            // Spawn argv directly (no `sh -c`) so namespace/pod/container fields
            // are never shell-interpreted. This prevents command injection from
            // shell metachars in (possibly imported, untrusted) kubectl configs.
            // kubectl runs in-sandbox under Flatpak (Flatpak Components), so the
            // old flatpak-spawn wrapper is no longer needed — same as Mosh below.
            let argv: Vec<&str> = command.iter().map(String::as_str).collect();
            notebook.spawn_command(session_id, &argv, None, None, None);
        }
        rustconn_core::ProtocolConfig::Mosh(_) => {
            let mosh = MoshProtocol::new();
            let Some(command) = mosh.build_command(&conn) else {
                tracing::warn!(%session_id, "Failed to build MOSH command for reconnect");
                return false;
            };

            let conn_msg = format_connection_message("MOSH", &conn.host);
            let cmd_msg = format_command_message(&command.join(" "));
            notebook
                .display_connect_banner(session_id, &format!("{conn_msg}\r\n{cmd_msg}\r\n\r\n"));

            // Re-assert the erase mode (issue #271): a reconnect spawns into the
            // same terminal, and nothing else restores it after a VTE reset.
            let (backspace_sends, delete_sends) = conn.protocol_config.erase_modes();
            notebook.set_erase_mode(session_id, backspace_sends, delete_sends);

            // Mosh uses direct exec (no shell wrapper needed)
            let argv: Vec<&str> = command.iter().map(String::as_str).collect();
            notebook.spawn_command(session_id, &argv, None, None, None);
        }
        _ => {
            tracing::warn!("Unsupported protocol for generic VTE reconnect");
            return false;
        }
    }

    // Update last_connected
    if let Ok(mut state_mut) = state.try_borrow_mut() {
        let _ = state_mut.update_last_connected(connection_id);
    }

    // Status detection: mark connected once the new process has printed a line
    // that is not a failure. The rows and text both count from below the
    // connect banner pinned above, so an SSM reconnect that prints only
    // "Starting session…" and a prompt — even in the same VTE pass as the
    // banner — turns the sidebar green, and a failed one, whose CLI prints its
    // error and exits, stays on "connecting" until the exit marks it failed.
    let sidebar = sidebar.clone();
    when_established(notebook, session_id, StatusClient::Other, move || {
        sidebar.increment_session_count(&connection_id.to_string(), session_id);
    });

    // Auto-recording for the protocols whose fresh connect records, with
    // the same remote-copy parameters (Zero Trust sessions are not recorded).
    let recording_ssh_params = match &conn.protocol_config {
        rustconn_core::ProtocolConfig::Telnet(_) => Some(crate::terminal::SshRecordingParams {
            host: conn.host.clone(),
            port: conn.port,
            username: conn.username.clone(),
            identity_file: None,
        }),
        rustconn_core::ProtocolConfig::Mosh(cfg) => Some(crate::terminal::SshRecordingParams {
            host: conn.host.clone(),
            port: cfg.ssh_port.unwrap_or(22),
            username: conn.username.clone(),
            identity_file: None,
        }),
        _ => None,
    };
    if !matches!(
        conn.protocol_config,
        rustconn_core::ProtocolConfig::ZeroTrust(_)
    ) {
        record_reconnect_when_established(
            notebook,
            session_id,
            StatusClient::Other,
            &conn,
            recording_ssh_params,
        );
    }

    true
}

/// Starts Telnet and observes a session created after asynchronous setup.
pub fn start_telnet_connection_observed(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
    logging_enabled: bool,
    observer: Option<super::types::SessionStartObserver>,
) -> Option<Uuid> {
    // Check if port check is needed
    let settings = state.borrow().settings().clone();
    let should_check = conn.should_pre_connect_check(&settings.connection);

    if should_check {
        let host = conn.host.clone();
        let port = conn.port;
        let timeout = settings.connection.port_check_timeout_secs;
        let state_clone = state.clone();
        let notebook_clone = notebook.clone();
        let sidebar_clone = sidebar.clone();
        let conn_clone = conn.clone();

        // Run port check in background thread
        spawn_blocking_with_callback(
            move || check_port(&host, port, timeout),
            move |result| match result {
                Ok(_) => {
                    let _ = start_telnet_connection_internal(
                        &state_clone,
                        &notebook_clone,
                        &sidebar_clone,
                        connection_id,
                        &conn_clone,
                        logging_enabled,
                        observer,
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        protocol = "telnet",
                        host = %conn_clone.host,
                        port = conn_clone.port,
                        error = %e,
                        "Port check failed for Telnet connection"
                    );
                    sidebar_clone.update_connection_status(&connection_id.to_string(), "failed");
                    if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                        state_mut.record_connection_attempt_failed(
                            &conn_clone,
                            conn_clone.username.as_deref(),
                            &e.to_string(),
                        );
                    }
                    if let Some(root) = notebook_clone.widget().root()
                        && let Some(window) = root.downcast_ref::<gtk4::Window>()
                    {
                        crate::toast::show_retry_toast_on_window(
                            window,
                            &e.to_string(),
                            &connection_id.to_string(),
                        );
                    }
                }
            },
        );
        None
    } else {
        start_telnet_connection_internal(
            state,
            notebook,
            sidebar,
            connection_id,
            conn,
            logging_enabled,
            observer,
        )
    }
}

/// Spawns `evil-winrm` in an open terminal tab and answers its password prompt.
///
/// The password goes through the prompt, never `-p`, so it stays out of the
/// process list. Returns `false` when the session could not start.
fn launch_winrm(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    session_id: Uuid,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
    global_variables: &[Variable],
) -> bool {
    let Some(client) = rustconn_core::protocol::locate_winrm_client() else {
        let hint = if rustconn_core::flatpak::is_flatpak() {
            i18n(
                "evil-winrm was not found on this computer. Install it outside the Flatpak, from your distribution or with: gem install evil-winrm. It must be on the PATH of your login shell on the host (set in ~/.profile, not only in ~/.bashrc).",
            )
        } else if rustconn_core::snap::is_snap() {
            i18n(
                "evil-winrm cannot run inside the Snap. Connect to Windows over SSH instead, or use the Flatpak or a distribution package of RustConn",
            )
        } else {
            i18n("evil-winrm is not installed. Install it with: gem install evil-winrm")
        };
        notebook.display_connect_banner(session_id, &format!("{hint}\r\n"));
        sidebar.update_connection_status(&connection_id.to_string(), "failed");
        return false;
    };

    let mut resolved = conn.clone();
    resolved.host = substitute_variables(&conn.host, global_variables);
    resolved.username = conn
        .username
        .as_ref()
        .map(|user| substitute_variables(user, global_variables));

    let Some(params) = ssh_tunnel_params_via_jump(
        state,
        conn,
        resolved.host.clone(),
        conn.port,
        rustconn_core::ssh_tunnel::HopHostKeys::UserConfig,
    ) else {
        return spawn_winrm(
            state,
            notebook,
            session_id,
            connection_id,
            &resolved,
            &client,
        );
    };

    // Raised off the GTK thread so a dead jump host cannot freeze the window.
    let state = state.clone();
    let notebook = notebook.clone();
    let sidebar = sidebar.clone();
    spawn_blocking_with_callback(
        move || {
            let mut tunnel = rustconn_core::ssh_tunnel::create_tunnel(&params)?;
            rustconn_core::ssh_tunnel::wait_for_tunnel_ready(
                &mut tunnel,
                50,
                std::time::Duration::from_millis(200),
            )?;
            Ok::<_, rustconn_core::ssh_tunnel::SshTunnelError>(tunnel)
        },
        move |result| match result {
            Ok(tunnel) => {
                resolved.host = "127.0.0.1".to_string();
                resolved.port = tunnel.local_port();
                // The tunnel closes with the tab.
                notebook.store_ssh_tunnel(session_id, tunnel);
                // A host client reaches the tunnel's 127.0.0.1 port because the
                // Flatpak shares the host's network namespace (`--share=network`).
                spawn_winrm(
                    &state,
                    &notebook,
                    session_id,
                    connection_id,
                    &resolved,
                    &client,
                );
            }
            Err(error) => {
                tracing::error!(%error, "SSH tunnel for WinRM failed");
                notebook.display_connect_banner(
                    session_id,
                    &format!(
                        "{}\r\n",
                        i18n_f("Could not open the SSH tunnel: {}", &[&error.to_string()])
                    ),
                );
                sidebar.update_connection_status(&connection_id.to_string(), "failed");
            }
        },
    );
    true
}

/// Starts the client against `resolved`, which already has its final address.
fn spawn_winrm(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    session_id: Uuid,
    connection_id: Uuid,
    resolved: &rustconn_core::Connection,
    client: &rustconn_core::protocol::WinRmClient,
) -> bool {
    use rustconn_core::protocol::{
        Protocol, WINRM_PASSWORD_PROMPT, WinRmProtocol, format_command_message,
        format_connection_message,
    };

    let Some(command) = WinRmProtocol::new().build_command(resolved) else {
        tracing::warn!(%session_id, "Failed to build WinRM command");
        return false;
    };
    let command = client.command(command);

    let conn_msg = format_connection_message("WinRM", &resolved.host);
    let cmd_msg = format_command_message(&command.join(" "));
    notebook.display_connect_banner(session_id, &format!("{conn_msg}\r\n{cmd_msg}\r\n\r\n"));
    let argv: Vec<&str> = command.iter().map(String::as_str).collect();
    notebook.spawn_command(session_id, &argv, None, None, None);

    prompt_autofill::install_login_autofill(
        notebook,
        session_id,
        prompt_autofill::LoginAutofill {
            username: None,
            password: cached_connection_password(state, connection_id),
            matcher: rustconn_core::LoginPromptMatcher::new(None, Some(WINRM_PASSWORD_PROMPT)),
            protocol: "winrm",
            deadline_secs: None,
        },
    );
    true
}

/// The SSH tunnel that reaches `host:port` through `conn`'s jump host, if any.
pub(super) fn ssh_tunnel_params_via_jump(
    state: &SharedAppState,
    conn: &rustconn_core::Connection,
    host: String,
    port: u16,
    hop_host_keys: rustconn_core::ssh_tunnel::HopHostKeys,
) -> Option<rustconn_core::ssh_tunnel::SshTunnelParams> {
    let jump_id = resolve_first_hop_id(&*state.try_borrow().ok()?, conn)?;
    // Before the route: the hops past the jump host are routed differently
    // when the tunnel sends its password.
    let password = state
        .try_borrow_mut()
        .ok()
        .and_then(|mut s| s.ensure_connection_password(jump_id));
    let state_ref = state.try_borrow().ok()?;
    let jump_conn = state_ref.get_connection(jump_id)?;
    let jump_host = jump_conn.username.as_ref().map_or_else(
        || jump_conn.host.clone(),
        |user| format!("{user}@{}", jump_conn.host),
    );
    let jump_port = jump_conn.port;
    let groups = state_ref.list_groups_owned();
    let identity_file = ssh_inheritance::resolve_ssh_key_path(jump_conn, &groups)
        .and_then(|path| rustconn_core::resolve_key_path(&path))
        .map(|path| path.to_string_lossy().to_string());
    let extra_args =
        resolve_jump_chain_for_tunnel(&state_ref, jump_conn, password.is_some(), hop_host_keys);
    drop(state_ref);
    Some(rustconn_core::ssh_tunnel::SshTunnelParams {
        jump_host,
        jump_port,
        remote_host: host,
        remote_port: port,
        identity_file,
        password,
        extra_args,
        launcher: rustconn_core::ssh_tunnel::TunnelLauncher::OpenSsh,
    })
}

/// Internal function to start Telnet or WinRM connection (after port check).
///
/// Creates a terminal tab and spawns the telnet process.
fn start_telnet_connection_internal(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
    logging_enabled: bool,
    observer: Option<super::types::SessionStartObserver>,
) -> Option<Uuid> {
    use rustconn_core::protocol::{format_command_message, format_connection_message};

    let conn_name = conn.name.clone();
    let port = conn.port;

    // Get terminal settings from state
    let terminal_settings = state
        .try_borrow()
        .ok()
        .map(|s| s.settings().terminal.clone())
        .unwrap_or_default();

    // Get global variables for substitution (secret values resolved from vault)
    let global_variables = state
        .try_borrow()
        .ok()
        .map(|s| crate::state::resolve_global_variables(s.settings()))
        .unwrap_or_default();

    // Resolve automation config with group inheritance
    let resolved_automation = resolve_automation_for_connection(state, conn);

    // Create terminal tab for Telnet or WinRM
    let session_id = notebook.create_terminal_tab_with_settings(
        connection_id,
        &conn.name,
        conn.protocol_config.protocol_type().as_str(),
        Some(&resolved_automation),
        &terminal_settings,
        conn.theme_override.as_ref(),
        &automation_variables(
            state,
            connection_id,
            conn,
            &resolved_automation,
            &global_variables,
        ),
    );

    // Pipe this session's output through the configured output filter, when
    // one is set and installed (`PostpendCommand`). Registered before the
    // spawn, which is what reads it.
    notebook.set_output_filter(session_id, conn.postpend.as_ref());
    if let Some(observer) = observer {
        observer.complete(session_id);
    }

    // Apply highlight rules (built-in defaults + global + per-connection)
    apply_highlight_rules(state, notebook, session_id, &conn.highlight_rules);

    // Record connection start in history
    let history_entry_id = if let Ok(mut state_mut) = state.try_borrow_mut() {
        Some(state_mut.record_connection_start(conn, conn.username.as_deref()))
    } else {
        None
    };

    if let Some(entry_id) = history_entry_id {
        notebook.set_history_entry_id(session_id, entry_id);
    }

    let host = substitute_variables(&conn.host, &global_variables);

    // Update last_connected timestamp
    if let Ok(mut state_mut) = state.try_borrow_mut()
        && let Err(e) = state_mut.update_last_connected(connection_id)
    {
        tracing::warn!(?e, "Failed to update last_connected");
    }

    // Set up session logging if enabled
    if logging_enabled {
        MainWindow::setup_session_logging(state, notebook, session_id, connection_id, &conn_name);
    }

    // Wire up child exited callback
    MainWindow::setup_child_exited_handler(state, notebook, sidebar, session_id, connection_id);

    if matches!(
        conn.protocol_config,
        rustconn_core::ProtocolConfig::WinRm(_)
    ) {
        launch_winrm(
            state,
            notebook,
            sidebar,
            session_id,
            connection_id,
            conn,
            &global_variables,
        );
    } else {
        // Get custom args and keyboard settings from TelnetConfig
        let (extra_args, backspace_sends, delete_sends) =
            if let rustconn_core::ProtocolConfig::Telnet(ref config) = conn.protocol_config {
                (
                    config.custom_args.clone(),
                    config.backspace_sends,
                    config.delete_sends,
                )
            } else {
                (
                    Vec::new(),
                    rustconn_core::models::BackspaceSends::Automatic,
                    rustconn_core::models::DeleteSends::Automatic,
                )
            };

        // Build telnet command string for display
        let mut cmd_parts = vec!["telnet".to_string()];
        cmd_parts.extend(extra_args.clone());
        cmd_parts.push(host.clone());
        cmd_parts.push(port.to_string());
        let telnet_command = cmd_parts.join(" ");

        // Display CLI output feedback
        let conn_msg = format_connection_message("Telnet", &host);
        let cmd_msg = format_command_message(&telnet_command);
        let feedback = format!("{conn_msg}\r\n{cmd_msg}\r\n\r\n");
        notebook.display_connect_banner(session_id, &feedback);

        // Spawn telnet
        let extra_refs: Vec<&str> = extra_args.iter().map(String::as_str).collect();
        notebook.spawn_telnet(
            session_id,
            &host,
            port,
            &extra_refs,
            backspace_sends,
            delete_sends,
        );

        // --- Automatic login (issue #254) ---
        // Telnet has no authentication protocol: the device prints its own prompts
        // and expects the account name and the password to be typed. Network gear
        // words those prompts inconsistently (`>>User name:`, `Username:`,
        // `login:`), so the expected text can be overridden per connection or per
        // group; unset falls back to the built-in matchers.
        prompt_autofill::install_login_autofill(
            notebook,
            session_id,
            login_autofill_for(
                state,
                connection_id,
                conn,
                &resolved_automation,
                &global_variables,
                "telnet",
            ),
        );
    }

    // --- Auto-recording for Telnet ---
    if conn.session_recording_enabled {
        let notebook_clone = notebook.clone();
        let recording_conn_name = conn_name.clone();
        let recording_ssh_params = matches!(
            conn.protocol_config,
            rustconn_core::ProtocolConfig::Telnet(_)
        )
        .then(|| crate::terminal::SshRecordingParams {
            host: host.clone(),
            port,
            username: conn.username.clone(),
            identity_file: None,
        });

        when_established(notebook, session_id, StatusClient::Other, move || {
            notebook_clone.start_recording(session_id, &recording_conn_name, recording_ssh_params);
            tracing::info!(
                %session_id,
                "Auto-recording started after Telnet or WinRM connection"
            );
        });
    }

    Some(session_id)
}

/// Starts a Zero Trust connection
///
/// Creates a terminal tab and spawns the Zero Trust provider command.
pub fn start_zerotrust_connection(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
    logging_enabled: bool,
) -> Option<Uuid> {
    use rustconn_core::protocol::{format_command_message, format_connection_message};

    let conn_name = conn.name.clone();

    let rustconn_core::ProtocolConfig::ZeroTrust(zt_config) = &conn.protocol_config else {
        return None;
    };

    // Validate the Zero Trust config and resolve display metadata; the command
    // itself is built further down, once the global variables are resolved.
    let (provider_name, provider_key) = {
        // Validate configuration before launch
        if let Err(e) = zt_config.validate() {
            tracing::error!(?e, "ZeroTrust config validation failed for {}", conn_name);
            if let Some(root) = notebook.widget().root()
                && let Some(window) = root.downcast_ref::<gtk4::Window>()
            {
                crate::toast::show_toast_on_window(
                    window,
                    &crate::i18n::i18n_f("Invalid config: {}", &[&e.to_string()]),
                    crate::toast::ToastType::Error,
                );
            }
            return None;
        }

        let provider = zt_config.provider.display_name();

        // Check CLI tool availability before launch
        let cli = zt_config.provider.cli_command();
        if !cli.is_empty() && !rustconn_core::flatpak::is_host_command_available(cli) {
            tracing::warn!(
                provider = %provider,
                cli,
                flatpak = rustconn_core::flatpak::is_flatpak(),
                "ZeroTrust CLI tool not found"
            );
            if let Some(root) = notebook.widget().root()
                && let Some(window) = root.downcast_ref::<gtk4::Window>()
            {
                crate::toast::show_missing_cli_toast(
                    window,
                    &format!("{provider} requires '{cli}' CLI tool"),
                );
            }
            return None;
        }

        tracing::info!(
            provider = %provider,
            cli,
            connection = %conn_name,
            "Launching ZeroTrust connection"
        );

        // Get provider key for icon matching
        let key = match zt_config.provider {
            rustconn_core::models::ZeroTrustProvider::AwsSsm => "aws",
            rustconn_core::models::ZeroTrustProvider::GcpIap => "gcloud",
            rustconn_core::models::ZeroTrustProvider::AzureBastion => "azure",
            rustconn_core::models::ZeroTrustProvider::AzureSsh => "azure_ssh",
            rustconn_core::models::ZeroTrustProvider::OciBastion => "oci",
            rustconn_core::models::ZeroTrustProvider::CloudflareAccess => "cloudflare",
            rustconn_core::models::ZeroTrustProvider::Teleport => "teleport",
            rustconn_core::models::ZeroTrustProvider::TailscaleSsh => "tailscale",
            rustconn_core::models::ZeroTrustProvider::Boundary => "boundary",
            rustconn_core::models::ZeroTrustProvider::HoopDev => "hoop",
            rustconn_core::models::ZeroTrustProvider::Generic => "generic",
        };
        (provider, key)
    };

    let automation_config = resolve_automation_for_connection(state, conn);

    // Get terminal settings from state
    let terminal_settings = state
        .try_borrow()
        .ok()
        .map(|s| s.settings().terminal.clone())
        .unwrap_or_default();

    // Get global variables for substitution in Expect responses / the Custom
    // Command template, overlaid with this connection's `@ask:` answers.
    let global_variables = resolve_globals_with_ask(state, connection_id);

    // Build the command. For a Custom Command this expands the ${var}
    // placeholders — done before opening a tab so an unusable variable value
    // reports an error instead of leaving a dead terminal behind (#151).
    let password = cached_connection_password(state, connection_id);
    let launch = match build_zerotrust_launch(conn, zt_config, &global_variables, password.as_ref())
    {
        Ok(launch) => launch,
        Err(e) => {
            tracing::error!(?e, connection = %conn_name, "Failed to expand custom command");
            if let Some(root) = notebook.widget().root()
                && let Some(window) = root.downcast_ref::<gtk4::Window>()
            {
                crate::toast::show_toast_on_window(
                    window,
                    &i18n_f("Invalid variable: {}", &[&e.to_string()]),
                    crate::toast::ToastType::Error,
                );
            }
            return None;
        }
    };

    // Create terminal tab for Zero Trust with provider-specific protocol
    let tab_protocol = format!("zerotrust:{provider_key}");
    let session_id = notebook.create_terminal_tab_with_settings(
        connection_id,
        &conn_name,
        &tab_protocol,
        Some(&automation_config),
        &terminal_settings,
        conn.theme_override.as_ref(),
        &automation_variables(
            state,
            connection_id,
            conn,
            &automation_config,
            &global_variables,
        ),
    );

    // Pipe this session's output through the configured output filter, when
    // one is set and installed (`PostpendCommand`). Registered before the
    // spawn, which is what reads it.
    notebook.set_output_filter(session_id, conn.postpend.as_ref());

    // Apply highlight rules (built-in defaults + global + per-connection). A
    // reconnect already did this; the first connect did not (issue #343).
    apply_highlight_rules(state, notebook, session_id, &conn.highlight_rules);

    // Record connection start in history
    let history_entry_id = if let Ok(mut state_mut) = state.try_borrow_mut() {
        Some(state_mut.record_connection_start(conn, conn.username.as_deref()))
    } else {
        None
    };

    // Store history entry ID in session for later use
    if let Some(entry_id) = history_entry_id {
        notebook.set_history_entry_id(session_id, entry_id);
    }

    // Update last_connected timestamp
    if let Ok(mut state_mut) = state.try_borrow_mut()
        && let Err(e) = state_mut.update_last_connected(connection_id)
    {
        tracing::warn!(?e, "Failed to update last_connected");
    }

    // Set up session logging if enabled
    if logging_enabled {
        MainWindow::setup_session_logging(state, notebook, session_id, connection_id, &conn_name);
    }

    // Wire up child exited callback for session cleanup
    MainWindow::setup_child_exited_handler(state, notebook, sidebar, session_id, connection_id);

    // Display CLI output feedback before executing command
    let conn_msg = format_connection_message(provider_name, &conn_name);
    let cmd_msg = format_command_message(&launch.display);
    let feedback = format!("{conn_msg}\r\n{cmd_msg}\r\n\r\n");
    notebook.display_connect_banner(session_id, &feedback);

    let argv: Vec<&str> = launch.argv.iter().map(String::as_str).collect();
    let env = launch.env_refs();
    let envv = (!env.is_empty()).then_some(env.as_slice());
    notebook.spawn_command(session_id, &argv, envv, None, None);

    Some(session_id)
}

/// Starts a Serial connection
///
/// Creates a terminal tab and spawns picocom with the serial configuration.
/// Shows user-friendly toasts when picocom is not found or device access fails.
pub fn start_serial_connection(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
    logging_enabled: bool,
) -> Option<Uuid> {
    use rustconn_core::protocol::{
        Protocol, SerialProtocol, detect_picocom, format_command_message, format_connection_message,
    };

    let conn_name = conn.name.clone();

    // Check picocom availability before attempting to launch
    let picocom_info = detect_picocom();
    if !picocom_info.installed {
        tracing::warn!(
            connection = %conn_name,
            "picocom not found for Serial connection"
        );
        if let Some(root) = notebook.widget().root()
            && let Some(window) = root.downcast_ref::<gtk4::Window>()
        {
            crate::toast::show_missing_cli_toast(
                window,
                &i18n("Install picocom for Serial connections"),
            );
        }
        return None;
    }

    // Build picocom command via SerialProtocol
    let serial = SerialProtocol::new();
    let Some(command) = serial.build_command(conn) else {
        tracing::error!(
            connection = %conn_name,
            "Failed to build picocom command for Serial connection"
        );
        return None;
    };

    tracing::info!(
        connection = %conn_name,
        connection_id = %connection_id,
        "Starting Serial connection"
    );

    // Get terminal settings from state
    let terminal_settings = state
        .try_borrow()
        .ok()
        .map(|s| s.settings().terminal.clone())
        .unwrap_or_default();

    // Get global variables for substitution in Expect responses, overlaid with
    // this connection's interactive `@ask:` answers.
    let global_variables = resolve_globals_with_ask(state, connection_id);

    // Resolve automation config with group inheritance
    let resolved_automation = resolve_automation_for_connection(state, conn);

    // Create terminal tab for Serial
    let session_id = notebook.create_terminal_tab_with_settings(
        connection_id,
        &conn_name,
        "serial",
        Some(&resolved_automation),
        &terminal_settings,
        conn.theme_override.as_ref(),
        &automation_variables(
            state,
            connection_id,
            conn,
            &resolved_automation,
            &global_variables,
        ),
    );

    // Pipe this session's output through the configured output filter, when
    // one is set and installed (`PostpendCommand`). Registered before the
    // spawn, which is what reads it.
    notebook.set_output_filter(session_id, conn.postpend.as_ref());

    // Apply highlight rules (built-in defaults + global + per-connection)
    apply_highlight_rules(state, notebook, session_id, &conn.highlight_rules);

    // Record connection start in history
    let history_entry_id = if let Ok(mut state_mut) = state.try_borrow_mut() {
        Some(state_mut.record_connection_start(conn, conn.username.as_deref()))
    } else {
        None
    };

    if let Some(entry_id) = history_entry_id {
        notebook.set_history_entry_id(session_id, entry_id);
    }

    // Update last_connected timestamp
    if let Ok(mut state_mut) = state.try_borrow_mut()
        && let Err(e) = state_mut.update_last_connected(connection_id)
    {
        tracing::warn!(?e, "Failed to update last_connected");
    }

    // Set up session logging if enabled
    if logging_enabled {
        MainWindow::setup_session_logging(state, notebook, session_id, connection_id, &conn_name);
    }

    // Wire up child exited callback
    MainWindow::setup_child_exited_handler(state, notebook, sidebar, session_id, connection_id);

    // Get device name for display
    let device = if let rustconn_core::ProtocolConfig::Serial(ref cfg) = conn.protocol_config {
        cfg.device.clone()
    } else {
        String::new()
    };

    // Build command string for display
    let serial_command = command.join(" ");
    let conn_msg = format_connection_message("Serial", &device);
    let cmd_msg = format_command_message(&serial_command);
    let feedback = format!("{conn_msg}\r\n{cmd_msg}\r\n\r\n");
    notebook.display_connect_banner(session_id, &feedback);

    // Spawn picocom
    notebook.spawn_serial(session_id, &command);

    // --- Automatic login (issue #254) ---
    // A serial console is the same situation as Telnet: whatever is on the
    // other end of the line prints its own login prompts. Nothing is typed
    // unless the connection actually carries a username or a stored password.
    prompt_autofill::install_login_autofill(
        notebook,
        session_id,
        login_autofill_for(
            state,
            connection_id,
            conn,
            &resolved_automation,
            &global_variables,
            "serial",
        ),
    );

    // --- Auto-recording for Serial ---
    if conn.session_recording_enabled {
        let notebook_clone = notebook.clone();
        let recording_conn_name = conn_name;
        // Serial is local — no SSH params needed
        glib::timeout_add_local_once(std::time::Duration::from_secs(1), move || {
            notebook_clone.start_recording(session_id, &recording_conn_name, None);
            tracing::info!(
                %session_id,
                "Auto-recording started for Serial connection"
            );
        });
    }

    Some(session_id)
}

/// Starts a Kubernetes connection
///
/// Creates a terminal tab and spawns `kubectl exec` or `kubectl run`
/// with the Kubernetes configuration. Uses `Protocol::build_command()`
/// from `KubernetesProtocol` to generate the command.
pub fn start_kubernetes_connection(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
    logging_enabled: bool,
) -> Option<Uuid> {
    use rustconn_core::protocol::{
        KubernetesProtocol, Protocol, detect_kubectl, format_command_message,
        format_connection_message,
    };

    let conn_name = conn.name.clone();

    // Check kubectl availability before attempting to launch
    let kubectl_available = if rustconn_core::is_sandboxed() {
        rustconn_core::flatpak::is_host_command_available("kubectl")
    } else {
        let kubectl_info = detect_kubectl();
        kubectl_info.installed
    };
    if !kubectl_available {
        tracing::warn!(
            connection = %conn_name,
            sandboxed = rustconn_core::is_sandboxed(),
            "kubectl not found for Kubernetes connection"
        );
        if let Some(root) = notebook.widget().root()
            && let Some(window) = root.downcast_ref::<gtk4::Window>()
        {
            crate::toast::show_missing_cli_toast(
                window,
                &i18n("Install kubectl for Kubernetes connections"),
            );
        }
        return None;
    }

    // Build kubectl command via KubernetesProtocol
    let k8s = KubernetesProtocol::new();
    let Some(command) = k8s.build_command(conn) else {
        tracing::error!(
            connection = %conn_name,
            "Failed to build kubectl command for Kubernetes connection"
        );
        if let Some(root) = notebook.widget().root()
            && let Some(window) = root.downcast_ref::<gtk4::Window>()
        {
            crate::toast::show_toast_on_window(
                window,
                &i18n("Configure pod and container for Kubernetes"),
                crate::toast::ToastType::Error,
            );
        }
        return None;
    };

    tracing::info!(
        connection = %conn_name,
        connection_id = %connection_id,
        "Starting Kubernetes connection"
    );

    // Get terminal settings from state
    let terminal_settings = state
        .try_borrow()
        .ok()
        .map(|s| s.settings().terminal.clone())
        .unwrap_or_default();

    // Get global variables for substitution in Expect responses, overlaid with
    // this connection's interactive `@ask:` answers.
    let global_variables = resolve_globals_with_ask(state, connection_id);

    // Resolve automation config with group inheritance
    let resolved_automation = resolve_automation_for_connection(state, conn);

    // Create terminal tab for Kubernetes
    let session_id = notebook.create_terminal_tab_with_settings(
        connection_id,
        &conn_name,
        "kubernetes",
        Some(&resolved_automation),
        &terminal_settings,
        conn.theme_override.as_ref(),
        &automation_variables(
            state,
            connection_id,
            conn,
            &resolved_automation,
            &global_variables,
        ),
    );

    // Pipe this session's output through the configured output filter, when
    // one is set and installed (`PostpendCommand`). Registered before the
    // spawn, which is what reads it.
    notebook.set_output_filter(session_id, conn.postpend.as_ref());

    // Apply highlight rules (built-in defaults + global + per-connection)
    apply_highlight_rules(state, notebook, session_id, &conn.highlight_rules);

    // Record connection start in history
    let history_entry_id = if let Ok(mut state_mut) = state.try_borrow_mut() {
        Some(state_mut.record_connection_start(conn, conn.username.as_deref()))
    } else {
        None
    };

    if let Some(entry_id) = history_entry_id {
        notebook.set_history_entry_id(session_id, entry_id);
    }

    // Update last_connected timestamp
    if let Ok(mut state_mut) = state.try_borrow_mut()
        && let Err(e) = state_mut.update_last_connected(connection_id)
    {
        tracing::warn!(?e, "Failed to update last_connected");
    }

    // Set up session logging if enabled
    if logging_enabled {
        MainWindow::setup_session_logging(state, notebook, session_id, connection_id, &conn_name);
    }

    // Wire up child exited callback
    MainWindow::setup_child_exited_handler(state, notebook, sidebar, session_id, connection_id);

    // Get pod/busybox info for display
    let target = if let rustconn_core::ProtocolConfig::Kubernetes(ref cfg) = conn.protocol_config {
        if cfg.use_busybox {
            format!("busybox ({})", cfg.busybox_image)
        } else {
            cfg.pod.clone().unwrap_or_default()
        }
    } else {
        String::new()
    };

    // Build command string for display
    let kubectl_command = command.join(" ");
    let conn_msg = format_connection_message("Kubernetes", &target);
    let cmd_msg = format_command_message(&kubectl_command);
    let feedback = format!("{conn_msg}\r\n{cmd_msg}\r\n\r\n");
    notebook.display_connect_banner(session_id, &feedback);

    // Spawn argv directly (no `sh -c`) so namespace/pod/container fields are
    // never shell-interpreted — prevents command injection from shell metachars
    // in (possibly imported, untrusted) kubectl configs. kubectl runs in-sandbox
    // under Flatpak (Flatpak Components), so no host wrapper is needed.
    let argv: Vec<&str> = command.iter().map(String::as_str).collect();
    notebook.spawn_command(session_id, &argv, None, None, None);

    // --- Auto-recording for Kubernetes ---
    if conn.session_recording_enabled {
        let notebook_clone = notebook.clone();
        let recording_conn_name = conn_name;

        when_established(notebook, session_id, StatusClient::Other, move || {
            notebook_clone.start_recording(session_id, &recording_conn_name, None);
            tracing::info!(
                %session_id,
                "Auto-recording started after Kubernetes connection"
            );
        });
    }

    Some(session_id)
}

/// Starts MOSH and observes a session created after asynchronous setup.
pub fn start_mosh_connection_observed(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
    logging_enabled: bool,
    observer: Option<super::types::SessionStartObserver>,
) -> Option<Uuid> {
    // Port check uses the SSH port (mosh handshake goes over SSH)
    let settings = state.borrow().settings().clone();
    let should_check = conn.should_pre_connect_check(&settings.connection);

    if should_check {
        let ssh_port = if let rustconn_core::ProtocolConfig::Mosh(ref cfg) = conn.protocol_config {
            cfg.ssh_port.unwrap_or(22)
        } else {
            22
        };
        let host = conn.host.clone();
        let timeout = settings.connection.port_check_timeout_secs;
        let state_clone = state.clone();
        let notebook_clone = notebook.clone();
        let sidebar_clone = sidebar.clone();
        let conn_clone = conn.clone();

        spawn_blocking_with_callback(
            move || check_port(&host, ssh_port, timeout),
            move |result| match result {
                Ok(_) => {
                    let _ = start_mosh_connection_internal(
                        &state_clone,
                        &notebook_clone,
                        &sidebar_clone,
                        connection_id,
                        &conn_clone,
                        logging_enabled,
                        observer,
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        protocol = "mosh",
                        host = %conn_clone.host,
                        error = %e,
                        "Port check failed for MOSH connection"
                    );
                    sidebar_clone.update_connection_status(&connection_id.to_string(), "failed");
                    if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                        state_mut.record_connection_attempt_failed(
                            &conn_clone,
                            conn_clone.username.as_deref(),
                            &e.to_string(),
                        );
                    }
                    if let Some(root) = notebook_clone.widget().root()
                        && let Some(window) = root.downcast_ref::<gtk4::Window>()
                    {
                        crate::toast::show_retry_toast_on_window(
                            window,
                            &e.to_string(),
                            &connection_id.to_string(),
                        );
                    }
                }
            },
        );
        None
    } else {
        start_mosh_connection_internal(
            state,
            notebook,
            sidebar,
            connection_id,
            conn,
            logging_enabled,
            observer,
        )
    }
}

/// Internal function to start MOSH connection (after port check).
fn start_mosh_connection_internal(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
    logging_enabled: bool,
    observer: Option<super::types::SessionStartObserver>,
) -> Option<Uuid> {
    use rustconn_core::protocol::{
        MoshProtocol, Protocol, detect_mosh, format_command_message, format_connection_message,
    };

    let conn_name = conn.name.clone();

    // Check mosh availability
    let mosh_info = detect_mosh();
    if !mosh_info.installed {
        tracing::warn!(
            connection = %conn_name,
            "mosh not found for MOSH connection"
        );
        if let Some(root) = notebook.widget().root()
            && let Some(window) = root.downcast_ref::<gtk4::Window>()
        {
            crate::toast::show_missing_cli_toast(
                window,
                &i18n("Install mosh for MOSH connections"),
            );
        }
        return None;
    }

    // Build mosh command via MoshProtocol
    let mosh = MoshProtocol::new();
    let Some(command) = mosh.build_command(conn) else {
        tracing::error!(
            connection = %conn_name,
            "Failed to build mosh command"
        );
        return None;
    };

    tracing::info!(
        connection = %conn_name,
        connection_id = %connection_id,
        "Starting MOSH connection"
    );

    // Get terminal settings from state
    let terminal_settings = state
        .try_borrow()
        .ok()
        .map(|s| s.settings().terminal.clone())
        .unwrap_or_default();

    // Get global variables for substitution in Expect responses, overlaid with
    // this connection's interactive `@ask:` answers.
    let global_variables = resolve_globals_with_ask(state, connection_id);

    // Resolve automation config with group inheritance
    let resolved_automation = resolve_automation_for_connection(state, conn);

    // Create terminal tab for MOSH
    let session_id = notebook.create_terminal_tab_with_settings(
        connection_id,
        &conn_name,
        "mosh",
        Some(&resolved_automation),
        &terminal_settings,
        conn.theme_override.as_ref(),
        &automation_variables(
            state,
            connection_id,
            conn,
            &resolved_automation,
            &global_variables,
        ),
    );

    // Pipe this session's output through the configured output filter, when
    // one is set and installed (`PostpendCommand`). Registered before the
    // spawn, which is what reads it.
    notebook.set_output_filter(session_id, conn.postpend.as_ref());
    if let Some(observer) = observer {
        observer.complete(session_id);
    }

    // Apply highlight rules (built-in defaults + global + per-connection)
    apply_highlight_rules(state, notebook, session_id, &conn.highlight_rules);

    // Record connection start in history
    let history_entry_id = if let Ok(mut state_mut) = state.try_borrow_mut() {
        Some(state_mut.record_connection_start(conn, conn.username.as_deref()))
    } else {
        None
    };

    if let Some(entry_id) = history_entry_id {
        notebook.set_history_entry_id(session_id, entry_id);
    }

    // Update last_connected timestamp
    if let Ok(mut state_mut) = state.try_borrow_mut()
        && let Err(e) = state_mut.update_last_connected(connection_id)
    {
        tracing::warn!(?e, "Failed to update last_connected");
    }

    // Set up session logging if enabled
    if logging_enabled {
        MainWindow::setup_session_logging(state, notebook, session_id, connection_id, &conn_name);
    }

    // Wire up child exited callback
    MainWindow::setup_child_exited_handler(state, notebook, sidebar, session_id, connection_id);

    // Build command string for display
    let mosh_command = command.join(" ");
    let conn_msg = format_connection_message("MOSH", &conn.host);
    let cmd_msg = format_command_message(&mosh_command);
    let feedback = format!("{conn_msg}\r\n{cmd_msg}\r\n\r\n");
    notebook.display_connect_banner(session_id, &feedback);

    // Backspace/Delete bytes this host expects (issue #271). MOSH draws into a
    // VTE widget like SSH and Telnet do, so the setting applies to the widget
    // rather than to the command line — and after the tab's terminal settings,
    // which would otherwise reinstall the defaults over it.
    {
        let (backspace_sends, delete_sends) = conn.protocol_config.erase_modes();
        notebook.set_erase_mode(session_id, backspace_sends, delete_sends);
    }

    // Spawn mosh — uses exec (no shell wrapper needed)
    let argv: Vec<&str> = command.iter().map(String::as_str).collect();
    notebook.spawn_command(session_id, &argv, None, None, None);

    // --- Auto-recording for MOSH ---
    if conn.session_recording_enabled {
        let notebook_clone = notebook.clone();
        let recording_conn_name = conn_name;
        let ssh_port = if let rustconn_core::ProtocolConfig::Mosh(ref cfg) = conn.protocol_config {
            cfg.ssh_port.unwrap_or(22)
        } else {
            22
        };
        let recording_ssh_params = Some(crate::terminal::SshRecordingParams {
            host: conn.host.clone(),
            port: ssh_port,
            username: conn.username.clone(),
            identity_file: None,
        });

        // mosh-client draws on the alternate screen; see
        // `ConnectionOutput::screen_replaced`.
        when_established(notebook, session_id, StatusClient::Other, move || {
            notebook_clone.start_recording(session_id, &recording_conn_name, recording_ssh_params);
            tracing::info!(
                %session_id,
                "Auto-recording started after MOSH connection"
            );
        });
    }

    Some(session_id)
}

#[cfg(test)]
mod tests {
    //! Tests for the `${password}` rewrite of a Custom Command template
    //! (issue #151). No GTK widgets are involved: both helpers are pure string
    //! transforms, and the property that matters is that the secret never
    //! reaches the rewritten text — only a shell reference does.

    use rustconn_core::variables::{Variable, VariableScope};
    use secrecy::{ExposeSecret, SecretString};

    use super::{
        PASSWORD_ENV_VAR, PASSWORD_TOKEN, automation_variables_base, connection_variable_manager,
        effective_password, link_password_reference, strip_quotes_around_password,
    };

    /// A connection carrying the given local variables.
    ///
    /// Only `local_variables` (plus the synthetic fields) matter to the two
    /// functions under test, so the protocol config is irrelevant here.
    fn connection_with_locals(vars: &[Variable]) -> rustconn_core::Connection {
        use rustconn_core::models::{ProtocolConfig, SshConfig};
        let mut conn = rustconn_core::Connection::new(
            "rustdesk".to_string(),
            "host.example".to_string(),
            0,
            ProtocolConfig::Ssh(SshConfig::default()),
        );
        for var in vars {
            conn.local_variables.insert(var.name.clone(), var.clone());
        }
        conn
    }

    #[test]
    fn token_becomes_a_quoted_shell_reference() {
        let (text, used) =
            link_password_reference(&format!("rustdesk --password {PASSWORD_TOKEN}"));
        assert!(used);
        assert_eq!(text, format!("rustdesk --password \"${PASSWORD_ENV_VAR}\""));
    }

    #[test]
    fn text_without_the_token_is_unchanged() {
        let (text, used) = link_password_reference("rustdesk --connect 123456789");
        assert!(
            !used,
            "no reference means no environment variable is needed"
        );
        assert_eq!(text, "rustdesk --connect 123456789");
    }

    #[test]
    fn user_written_quotes_are_absorbed() {
        // The replacement supplies its own quoting, so keeping the user's would
        // leave the expansion unquoted and word-split a password with spaces.
        assert_eq!(
            strip_quotes_around_password("rustdesk --password \"${password}\""),
            "rustdesk --password ${password}"
        );
        assert_eq!(
            strip_quotes_around_password("rustdesk --password '${password}'"),
            "rustdesk --password ${password}"
        );
    }

    #[test]
    fn unquoted_placeholder_is_left_alone() {
        assert_eq!(
            strip_quotes_around_password("rustdesk --password ${password}"),
            "rustdesk --password ${password}"
        );
    }

    #[test]
    fn every_occurrence_is_linked() {
        let (text, used) =
            link_password_reference(&format!("a {PASSWORD_TOKEN} b {PASSWORD_TOKEN}"));
        assert!(used);
        assert!(!text.contains(PASSWORD_TOKEN));
        assert_eq!(text.matches(PASSWORD_ENV_VAR).count(), 2);
    }

    #[test]
    fn local_password_variable_wins_over_the_stored_credential() {
        let conn = connection_with_locals(&[Variable::new_secret("password", "from-local-var")]);
        let stored = SecretString::from("from-vault".to_string());
        let effective = effective_password(&conn, Some(&stored)).expect("a password is available");
        assert_eq!(effective.expose_secret(), "from-local-var");
    }

    #[test]
    fn stored_credential_is_used_without_a_local_variable() {
        let conn = connection_with_locals(&[Variable::new("id", "123456789")]);
        let stored = SecretString::from("from-vault".to_string());
        let effective = effective_password(&conn, Some(&stored)).expect("a password is available");
        assert_eq!(effective.expose_secret(), "from-vault");
    }

    #[test]
    fn an_empty_local_variable_does_not_mask_the_stored_credential() {
        let conn = connection_with_locals(&[Variable::new("password", "")]);
        let stored = SecretString::from("from-vault".to_string());
        let effective = effective_password(&conn, Some(&stored)).expect("a password is available");
        assert_eq!(effective.expose_secret(), "from-vault");
    }

    #[test]
    fn no_password_anywhere_yields_none() {
        let conn = connection_with_locals(&[]);
        assert!(effective_password(&conn, None).is_none());
    }

    #[test]
    fn a_local_password_variable_never_substitutes_its_plaintext() {
        // Regression guard: local variables normally override their synthetic
        // counterpart, which would have put the plaintext straight into the
        // `sh -c` argv. `${password}` must resolve to the token instead, so the
        // value can only travel through the child environment (issue #151).
        let conn = connection_with_locals(&[Variable::new_secret("password", "from-local-var")]);
        let expanded = connection_variable_manager(&conn, &[], true, false)
            .substitute_defined_for_command(
                "rustdesk --password ${password}",
                VariableScope::Connection(conn.id),
            )
            .expect("substitution succeeds");
        assert!(!expanded.contains("from-local-var"));
        assert_eq!(expanded, format!("rustdesk --password {PASSWORD_TOKEN}"));
    }

    /// Issue #317: a variable defined only on the connection (Local Variables)
    /// must resolve in an Expect response. Before the fix `automation_variables`
    /// carried only the globals and the synthetic fields, so a local variable
    /// resolved against nothing, the rule was dropped as undefined, and the
    /// Expect script stalled at the prompt.
    #[test]
    fn a_local_variable_resolves_in_an_expect_response() {
        use rustconn_core::variables::{VariableManager, VariableScope};

        let conn = connection_with_locals(&[Variable::new("user2_password", "hunter2")]);
        let vars = automation_variables_base(&conn, &[]);

        // Mirror the SSH path: everything is loaded as a global variable and
        // resolution runs in the global scope.
        let mut manager = VariableManager::new();
        for var in &vars {
            manager.set_global(var.clone());
        }
        let substitution = manager
            .substitute_for_terminal_input("${user2_password}\n", VariableScope::Global)
            .expect("substitution succeeds");

        assert!(
            substitution.unresolved.is_empty(),
            "the local variable must resolve, not be reported unresolved: {:?}",
            substitution.unresolved
        );
        assert_eq!(substitution.text.as_str(), "hunter2\n");
    }

    /// A connection-local variable shadows a global of the same name, matching
    /// the precedence the command path gives them.
    #[test]
    fn a_local_variable_shadows_a_global_of_the_same_name() {
        let conn = connection_with_locals(&[Variable::new("shared", "from-local")]);
        let vars = automation_variables_base(&conn, &[Variable::new("shared", "from-global")]);

        // The last entry of a given name wins once loaded via `set_global`, and
        // the local is appended after the globals.
        let last = vars
            .iter()
            .rev()
            .find(|v| v.name == "shared")
            .expect("the variable is present");
        assert_eq!(last.value, "from-local");
    }

    /// The synthetic `${host}`/`${port}`/`${username}` still shadow a local of
    /// the same name, so an accidental local `host` cannot redirect the prompt
    /// answer.
    #[test]
    fn synthetic_fields_shadow_a_same_named_local() {
        let mut conn = connection_with_locals(&[Variable::new("host", "wrong.example")]);
        conn.host = "real.example".to_string();
        let vars = automation_variables_base(&conn, &[]);

        let last = vars
            .iter()
            .rev()
            .find(|v| v.name == "host")
            .expect("host is present");
        assert_eq!(last.value, "real.example");
    }
}

#[cfg(test)]
mod established_tests {
    //! The shared SSH-established verdict (issue: jump-host sessions to
    //! unreachable targets shown as connected + monitoring started), as a fresh
    //! connect applies it. No GTK widgets: the helper is pure, driven by the
    //! cursor row and text counted from below RustConn's connect banner.
    use super::{
        ConnectionOutput, StatusClient, connection_established, ends_with_shell_prompt,
        ssh_output_established,
    };

    #[test]
    fn a_direct_connection_with_a_full_line_is_established() {
        assert!(ssh_output_established(
            1,
            false,
            Some("Last login: today\n")
        ));
        assert!(ssh_output_established(5, false, Some("anything\n")));
    }

    #[test]
    fn the_connect_banner_alone_is_not_established() {
        // The banner sits above the counted rows: nothing printed yet.
        assert!(!ssh_output_established(0, false, Some("")));
        assert!(!ssh_output_established(0, true, Some("")));
        assert!(!ssh_output_established(0, false, None));
    }

    #[test]
    fn jump_host_banner_alone_is_not_established() {
        // The exact repro: the bastion MOTD pushes the cursor to row 7 while the
        // final host is still timing out. No prompt yet, no failure yet — must
        // NOT latch connected.
        let banner = "Welcome to Ubuntu 22.04\n\n  System information as of ...\n\n";
        assert!(!ssh_output_established(7, true, Some(banner)));
    }

    #[test]
    fn jump_host_late_banner_timeout_is_not_established() {
        // The failure line the ProxyCommand's own ssh prints a beat after the
        // banner. "Connection timed out" is a contains_ssh_failure pattern and a
        // substring of the banner-exchange message from the logs.
        let text = "Welcome to Ubuntu\n\nConnection timed out during banner exchange\n";
        assert!(!ssh_output_established(9, true, Some(text)));
    }

    #[test]
    fn jump_host_unreadable_buffer_fails_closed() {
        assert!(!ssh_output_established(9, true, None));
    }

    #[test]
    fn jump_host_established_once_shell_prompt_appears() {
        // The final host handed over a shell — a prompt line appears and no
        // failure pattern is present. Now (and only now) it is established.
        let text = "Welcome to Ubuntu\nLast login: ...\nubuntu@target:~$ ";
        assert!(ssh_output_established(9, true, Some(text)));
    }

    #[test]
    fn jump_host_prompt_then_a_failure_is_not_established() {
        let text = "ubuntu@target:~$ \nConnection reset by 192.0.2.1 port 22\n";
        assert!(!ssh_output_established(2, true, Some(text)));
    }

    #[test]
    fn the_known_hosts_warning_alone_is_not_established() {
        let text =
            "Warning: Permanently added 'web.example' (ED25519) to the list of known hosts.\n";
        assert!(!ssh_output_established(1, false, Some(text)));
        let then_failed = format!("{text}user@web.example: Permission denied (publickey).\n");
        assert!(!ssh_output_established(2, false, Some(&then_failed)));
        let then_logged_in = format!("{text}Last login: today\n");
        assert!(ssh_output_established(2, false, Some(&then_logged_in)));
    }

    /// What OpenSSH prints on a first connect to a host it does not know.
    const HOST_KEY_QUESTION: &str = "The authenticity of host 'web.example (192.0.2.1)' \
         can't be established.\n\
         ED25519 key fingerprint is SHA256:4Kq1nT0vYk2mVjV9b1yQ3m8yS6c3C1yJ8m5f0hJx2aQ.\n\
         This key is not known by any other names.\n\
         Are you sure you want to continue connecting (yes/no/[fingerprint])? ";

    #[test]
    fn an_unanswered_host_key_question_is_not_established() {
        // Four full lines and no failure: before the host-key question was
        // recognised this counted as established, and the monitoring probe
        // could record the key while the user was still reading it.
        assert!(!ssh_output_established(3, false, Some(HOST_KEY_QUESTION)));
        assert!(!ssh_output_established(3, true, Some(HOST_KEY_QUESTION)));
        // The typed answer is echoed on the same line.
        let answered = format!("{HOST_KEY_QUESTION}yes\n");
        assert!(!ssh_output_established(4, false, Some(&answered)));
        let retyped = format!("{HOST_KEY_QUESTION}y\nPlease type 'yes', 'no' or the fingerprint: ");
        assert!(!ssh_output_established(4, false, Some(&retyped)));
        // The same question under a client that runs OpenSSH (mosh).
        assert!(!connection_established(
            StatusClient::Other,
            Some(&output(3, HOST_KEY_QUESTION, false))
        ));
    }

    #[test]
    fn a_host_key_question_answered_no_is_not_established() {
        let declined = format!("{HOST_KEY_QUESTION}no\nHost key verification failed.\n");
        assert!(!ssh_output_established(5, false, Some(&declined)));
    }

    #[test]
    fn a_host_key_question_answered_yes_then_a_login_is_established() {
        let accepted = format!(
            "{HOST_KEY_QUESTION}yes\n\
             Warning: Permanently added 'web.example' (ED25519) to the list of known hosts.\n"
        );
        assert!(
            !ssh_output_established(5, false, Some(&accepted)),
            "the known-hosts warning alone is still progress"
        );
        let password = format!("{accepted}user@web.example's password: ");
        assert!(!ssh_output_established(5, false, Some(&password)));
        let logged_in = format!("{accepted}Last login: today\nuser@web:~$ ");
        assert!(ssh_output_established(6, false, Some(&logged_in)));
    }

    #[test]
    fn a_changed_host_key_is_not_established() {
        let changed = "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\n\
             @    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @\n\
             @@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\n\
             IT IS POSSIBLE THAT SOMEONE IS DOING SOMETHING NASTY!\n";
        assert!(!ssh_output_established(4, false, Some(changed)));
        let refused = format!("{changed}Host key verification failed.\n");
        assert!(!ssh_output_established(5, false, Some(&refused)));
        // `StrictHostKeyChecking=no` goes on with key login; only a shell
        // prompt after the warning counts.
        let went_on = format!(
            "{changed}Password authentication is disabled to avoid man-in-the-middle \
             attacks.\nWelcome to Ubuntu\n"
        );
        assert!(!ssh_output_established(6, false, Some(&went_on)));
        let prompt = format!("{went_on}user@web:~$ ");
        assert!(ssh_output_established(7, false, Some(&prompt)));
    }

    #[test]
    fn shell_prompt_terminators_are_recognised() {
        assert!(ends_with_shell_prompt("ubuntu@host:~$ "));
        assert!(ends_with_shell_prompt("root@host:~# "));
        assert!(ends_with_shell_prompt("zsh %"));
        assert!(ends_with_shell_prompt("PS>"));
        assert!(ends_with_shell_prompt("~/src on main ❯ "));
        assert!(ends_with_shell_prompt("user@web ~ ➜"));
        // Trailing blank lines are ignored — the last non-blank line decides.
        assert!(ends_with_shell_prompt("user@host:~$ \n\n"));
    }

    #[test]
    fn banner_text_is_not_a_shell_prompt() {
        assert!(!ends_with_shell_prompt("Welcome to Ubuntu 22.04"));
        assert!(!ends_with_shell_prompt("System information as of now"));
        assert!(!ends_with_shell_prompt(""));
        // The new terminators count only at the end of the line, as the
        // others do: a MOTD or menu line that merely contains one is not a
        // prompt.
        assert!(!ends_with_shell_prompt("Press ❯ to open the menu"));
        assert!(!ends_with_shell_prompt(
            "➜ Documentation: https://help.example"
        ));
    }

    #[test]
    fn a_quiet_login_with_a_starship_prompt_is_established() {
        // Key login, no MOTD or last-login line, a one-line `❯` prompt: it
        // sits on row 0, so only the prompt can say it is connected.
        assert!(ssh_output_established(0, false, Some("❯ ")));
        assert!(ssh_output_established(0, true, Some("~ ❯ ")));
        assert!(ssh_output_established(0, false, Some("~/src ➜ ")));
    }

    fn output(row: i64, text: &str, screen_replaced: bool) -> ConnectionOutput {
        ConnectionOutput {
            row,
            text: text.to_owned(),
            screen_replaced,
        }
    }

    #[test]
    fn a_client_that_took_over_the_screen_is_established() {
        // mosh-client on the alternate screen: the rows cannot be compared
        // with the base, so the row and text say nothing.
        let replaced = output(0, "", true);
        for client in [
            StatusClient::Other,
            StatusClient::Ssh { jump_host: false },
            StatusClient::Ssh { jump_host: true },
        ] {
            assert!(
                connection_established(client, Some(&replaced)),
                "{client:?}"
            );
        }
    }

    #[test]
    fn the_shared_verdict_follows_the_clients_rule() {
        let bastion = output(3, "Welcome to the bastion\n\n", false);
        assert!(connection_established(
            StatusClient::Ssh { jump_host: false },
            Some(&bastion)
        ));
        assert!(!connection_established(
            StatusClient::Ssh { jump_host: true },
            Some(&bastion)
        ));
        let trying = output(1, "Trying 192.0.2.1...\n", false);
        assert!(!connection_established(StatusClient::Other, Some(&trying)));
        assert!(!connection_established(StatusClient::Other, None));
    }
}

#[cfg(test)]
mod ssh_output_established_tests {
    //! The SSH verdict after an in-place reconnect. Row and output count from
    //! below the connect banner, so they hold only what the new ssh process
    //! printed.
    use super::ssh_output_established;

    #[test]
    fn prompt_only_direct_reconnect_is_established() {
        // Key auth, no MOTD, no last-login line: the prompt alone, on row 0.
        assert!(ssh_output_established(0, false, Some("ubuntu@host:~$ ")));
    }

    #[test]
    fn direct_reconnect_with_a_full_line_is_established() {
        let output = "Last login: Tue Nov  4 14:00:00 2025\nubuntu@host:~$ ";
        assert!(ssh_output_established(1, false, Some(output)));
    }

    #[test]
    fn direct_reconnect_with_nothing_printed_is_not_established() {
        assert!(!ssh_output_established(0, false, Some("")));
        // A password or host-key question is not a prompt.
        assert!(!ssh_output_established(
            0,
            false,
            Some("ubuntu@host's password: ")
        ));
    }

    #[test]
    fn ssh_failure_line_is_not_established() {
        for output in [
            "ssh: connect to host example.com port 22: Connection refused\n",
            "Connection reset by 192.0.2.1 port 22\n",
        ] {
            assert!(
                !ssh_output_established(1, false, Some(output)),
                "treated as established: {output}"
            );
            assert!(
                !ssh_output_established(1, true, Some(output)),
                "treated as established through a jump host: {output}"
            );
        }
    }

    #[test]
    fn unreadable_or_pending_output_fails_closed() {
        assert!(!ssh_output_established(5, false, None));
        assert!(!ssh_output_established(5, true, None));
    }

    #[test]
    fn a_rejected_password_then_a_prompt_is_established() {
        let output = "Permission denied, please try again.\nuser@host:~$ ";
        assert!(ssh_output_established(1, false, Some(output)));
        assert!(ssh_output_established(1, true, Some(output)));
        let retried = "user@host's password: \nPermission denied, please try again.\n\
                       user@host's password: \nLast login: Tue Nov  4 14:00:00 2025\n\
                       user@host:~$ ";
        assert!(ssh_output_established(4, false, Some(retried)));
    }

    #[test]
    fn a_password_asked_again_is_not_established_yet() {
        let output = "user@host's password: \nPermission denied, please try again.\n\
                      user@host's password: ";
        assert!(!ssh_output_established(2, false, Some(output)));
    }

    #[test]
    fn a_failure_after_the_last_password_question_is_not_established() {
        let output = "user@host's password: \n\
                      user@host: Permission denied (publickey,password).\n";
        assert!(!ssh_output_established(2, false, Some(output)));
    }

    #[test]
    fn a_prompt_then_a_later_failure_is_not_established() {
        let output = "user@host:~$ \nConnection reset by 192.0.2.1 port 22\n";
        assert!(!ssh_output_established(2, false, Some(output)));
        assert!(!ssh_output_established(2, true, Some(output)));
    }

    #[test]
    fn a_host_key_question_on_reconnect_is_not_established() {
        // The host was reinstalled (new key, old entry removed) or the
        // known_hosts entry is gone: the reconnect asks again.
        let output = "The authenticity of host 'host (192.0.2.1)' can't be established.\n\
                      ED25519 key fingerprint is SHA256:abc.\n\
                      Are you sure you want to continue connecting (yes/no/[fingerprint])? ";
        assert!(!ssh_output_established(2, false, Some(output)));
        assert!(!ssh_output_established(2, true, Some(output)));
        let failed = format!("{output}no\nHost key verification failed.\n");
        assert!(!ssh_output_established(4, false, Some(&failed)));
        let changed = "@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @\n\
                       Host key verification failed.\n";
        assert!(!ssh_output_established(2, false, Some(changed)));
    }

    #[test]
    fn jump_host_reconnect_needs_a_prompt() {
        let banner = "Welcome to the bastion\n\n";
        assert!(!ssh_output_established(2, true, Some(banner)));
        let output = "Welcome to the bastion\n\nubuntu@target:~$ ";
        assert!(ssh_output_established(2, true, Some(output)));
    }
}

#[cfg(test)]
mod client_output_established_tests {
    //! The verdict for the other terminal clients (Zero Trust, Telnet, Serial,
    //! Kubernetes, Mosh). Row and text count from below the connect banner.
    use super::client_output_established;

    #[test]
    fn ssm_reply_in_one_chunk_is_established() {
        let output = "\nStarting session with SessionId: me-0123abcd\nsh-5.2$ ";
        assert!(client_output_established(2, Some(output)));
    }

    #[test]
    fn nothing_printed_yet_is_not_established() {
        assert!(!client_output_established(0, Some("")));
    }

    #[test]
    fn a_prompt_on_row_zero_is_established() {
        // `kubectl exec -it` or a quiet login: the prompt alone, on row 0.
        assert!(client_output_established(0, Some("sh-5.2$ ")));
    }

    #[test]
    fn a_failure_then_a_prompt_is_established() {
        let output = "Permission denied, please try again.\nuser@host:~$ ";
        assert!(client_output_established(1, Some(output)));
    }

    #[test]
    fn a_prompt_then_a_later_failure_is_not_established() {
        let output = "sh-5.2$ \nAn error occurred (TargetNotConnected) when calling the \
                      StartSession operation\n";
        assert!(!client_output_established(2, Some(output)));
    }

    #[test]
    fn telnet_trying_alone_is_not_established() {
        // telnet prints this in a pass of its own before the TCP connect.
        assert!(!client_output_established(1, Some("Trying 192.0.2.1...\n")));
        assert!(!client_output_established(
            1,
            Some("Trying 2001:db8::1...\n")
        ));
    }

    #[test]
    fn telnet_connected_is_established() {
        // A device that waits for a key press prints nothing after this.
        let output = "Trying 192.0.2.1...\nConnected to router.example.\n\
                      Escape character is '^]'.\n";
        assert!(client_output_established(3, Some(output)));
        assert!(client_output_established(
            2,
            Some("Trying 192.0.2.1...\nConnected to router.example.\n")
        ));
    }

    #[test]
    fn a_failed_telnet_address_before_the_next_one_does_not_count() {
        // telnet tries each address of a name in turn.
        let output = "Trying ::1...\ntelnet: connect to address ::1: Connection refused\n\
                      Trying 127.0.0.1...\nConnected to localhost.\n\
                      Escape character is '^]'.\n";
        assert!(client_output_established(5, Some(output)));
        let still_trying = "Trying ::1...\ntelnet: connect to address ::1: Connection refused\n\
                            Trying 127.0.0.1...\n";
        assert!(!client_output_established(3, Some(still_trying)));
    }

    #[test]
    fn a_waiting_password_question_is_not_established_yet() {
        assert!(!client_output_established(
            2,
            Some("Trying 192.0.2.1...\nConnected to host.\nPassword:")
        ));
    }

    #[test]
    fn generic_words_in_normal_output_are_not_failures() {
        for output in [
            "Welcome. Last run reported error: none\n",
            "motd: /etc/motd.d: No such file or directory\n",
            "the plugin is not found in this list, which is fine\n",
            "Hint: if a command not found message appears, run setup\n",
        ] {
            assert!(
                client_output_established(1, Some(output)),
                "treated as a failure: {output}"
            );
        }
    }

    #[test]
    fn unreadable_or_pending_output_fails_closed() {
        assert!(!client_output_established(3, None));
    }

    #[test]
    fn failed_reconnects_are_not_established() {
        for output in [
            "\nAn error occurred (TargetNotConnected) when calling the StartSession \
             operation: i-0123 is not connected.\n",
            "SessionManagerPlugin is not found. Please refer to SessionManager Documentation\n",
            "ERROR: (gcloud.compute.start-iap-tunnel) Error while connecting [4003]\n",
            "Trying 192.0.2.1...\ntelnet: Unable to connect to remote host: Connection refused\n",
            "FATAL: cannot open /dev/ttyUSB0: No such file or directory\n",
            "Error from server (NotFound): pods \"web-0\" not found\n",
            "Unable to connect to the server: dial tcp 192.0.2.10:6443: i/o timeout\n",
            "The connection to the server localhost:8080 was refused - did you specify \
             the right host or port?\n",
            "ssh: connect to host example.com port 22: No route to host\n",
            "aws: command not found\n",
        ] {
            assert!(
                !client_output_established(2, Some(output)),
                "treated as established: {output}"
            );
        }
    }
}
