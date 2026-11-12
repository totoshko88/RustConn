//! Opening an SFTP connection in Midnight Commander.
//!
//! Every mc entry point (the `open-sftp` action, Connect on an SFTP
//! connection, and the jump host fallback of both) goes through
//! [`open_mc_tab`], so the availability check, the password lookup and the
//! spawn are the same for all of them.
//!
//! A password-authenticated connection gets its password the way Connect
//! reads it: the session cache first, then the vault off the GTK thread
//! (which fills the cache, so a second open or a Connect within the cache
//! lifetime does not ask the vault again); a connection set to ask at connect
//! is asked once. `rustconn_core::mc_ssh` hands it only to the `ssh` mc starts
//! for this connection's own host, user and port.

use adw::prelude::*;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use rustconn_core::models::{Connection, ConnectionGroup, PasswordSource};
use secrecy::{ExposeSecret, SecretString};
use uuid::Uuid;

use super::types::{SharedNotebook, SharedSidebar, SharedSplitView};
use crate::i18n::{i18n, i18n_f};
use crate::state::SharedAppState;
use crate::terminal::ChildCleanup;
use crate::toast::ToastType;

/// One mc tab to open, with what the launch needs from the state.
pub(super) struct McTab {
    /// The stored connection's id (the tab, the cache and the sidebar key).
    pub connection_id: Uuid,
    /// The launch copy of the connection, with any fallback jump host applied.
    pub conn: Connection,
    /// Every connection, to resolve a jump host stored as a reference.
    pub connections: Vec<Connection>,
    /// Every group, for SSH inheritance.
    pub groups: Vec<ConnectionGroup>,
    /// The outermost bastion tier.
    pub network: rustconn_core::NetworkSettings,
    /// The terminal settings for the new tab.
    pub terminal_settings: rustconn_core::config::TerminalSettings,
    /// Whether to show the "Opening mc SFTP…" toast (the `open-sftp` action).
    pub announce: bool,
}

/// Opens `tab` in a new mc terminal tab.
///
/// Shows a warning and opens nothing for a non-SSH connection, and an error
/// when mc is not installed. A password-authenticated connection asked to
/// type its password at connect opens nothing when the user cancels.
pub(super) fn open_mc_tab(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    split_view: Option<&SharedSplitView>,
    sidebar: Option<&SharedSidebar>,
    tab: McTab,
) {
    let Some(mc_args) = rustconn_core::sftp::build_mc_sftp_command(&tab.conn) else {
        toast(
            notebook,
            &i18n("SFTP is only available for SSH connections."),
            ToastType::Warning,
        );
        return;
    };
    if !rustconn_core::which::is_available("mc") {
        toast(
            notebook,
            &i18n("Midnight Commander (mc) is not installed."),
            ToastType::Error,
        );
        return;
    }
    tracing::info!(?mc_args, "Opening SFTP via mc");

    let notebook_for_tab = notebook.clone();
    let split_view = split_view.cloned();
    let sidebar = sidebar.cloned();
    let (connection_id, conn) = (tab.connection_id, tab.conn.clone());
    let state_for_tab = state.clone();
    mc_password(
        state,
        notebook,
        connection_id,
        &conn,
        move |password: McPassword| {
            spawn_mc_tab(
                &state_for_tab,
                &notebook_for_tab,
                split_view.as_ref(),
                sidebar.as_ref(),
                tab,
                mc_args,
                password,
            );
        },
    );
}

/// Creates the tab, prepares the `ssh` wrapper off the GTK thread and starts
/// mc once it is ready.
fn spawn_mc_tab(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    split_view: Option<&SharedSplitView>,
    sidebar: Option<&SharedSidebar>,
    tab: McTab,
    mc_args: Vec<String>,
    password: McPassword,
) {
    let (password, refused) = match password {
        McPassword::Deliver(password) => (Some(password), false),
        McPassword::Absent => (None, false),
        McPassword::Refused => (None, true),
    };
    if refused {
        toast(
            notebook,
            &i18n_f(
                "The saved password for “{}” was refused. Type the password in the mc tab when mc waits for it.",
                &[&tab.conn.name],
            ),
            ToastType::Warning,
        );
    }
    if tab.announce {
        toast(notebook, &i18n("Opening mc SFTP…"), ToastType::Info);
    }
    let connection_id = tab.connection_id;
    if let Some(sb) = sidebar {
        sb.update_connection_status(&connection_id.to_string(), "connecting");
    }

    let tab_name = format!("mc: {}", tab.conn.name);
    let session_id = notebook.create_terminal_tab_with_settings(
        connection_id,
        &tab_name,
        "sftp",
        None,
        &tab.terminal_settings,
        None,
        &[],
    );

    // With a key, mc likely signs in without the password, so saying "type it"
    // when none was handed over would mislead.
    // After a refusal the toast above already said so.
    let warn_without_password = !refused
        && rustconn_core::mc_ssh::expects_password(&tab.conn)
        && !rustconn_core::mc_ssh::has_identity_file(&tab.conn, &tab.groups);
    let downloads = rustconn_core::sftp::get_downloads_dir();
    let notebook_for_spawn = notebook.clone();
    let state_for_exit = state.clone();
    let McTab {
        conn,
        connections,
        groups,
        network,
        ..
    } = tab;
    // mc's `sh://` URI carries only user, host and port; the jump host,
    // identity file, HostKeyAlias, custom options and the password reach the
    // `ssh` it spawns through a generated ssh_config and a PATH wrapper keyed
    // on this session (issue #255). Preparing it may run `ssh -G`.
    crate::utils::spawn_blocking_with_callback(
        move || {
            rustconn_core::prepare_mc_ssh_env(
                session_id,
                &conn,
                &connections,
                &groups,
                &network,
                password.as_ref(),
            )
        },
        move |env: Option<rustconn_core::McSshEnv>| {
            if warn_without_password
                && !env
                    .as_ref()
                    .is_some_and(rustconn_core::McSshEnv::delivers_password)
            {
                toast(
                    &notebook_for_spawn,
                    &i18n(
                        "mc did not get a password from RustConn. Type it in the mc tab when mc waits for it.",
                    ),
                    ToastType::Warning,
                );
            }
            // A refused password must leave the cache when mc exits, not at the
            // next mc open, or a Connect in between sends it once more. This
            // runs from the child watch, so closing the tab, the usual way out
            // of a failed mc, still records the refusal.
            let on_exit = env
                .as_ref()
                .filter(|env| env.delivers_password())
                .map(|env| {
                    let env = env.clone();
                    let state = std::rc::Rc::downgrade(&state_for_exit);
                    Box::new(move || {
                        if env.sign_in_refused()
                            && let Some(state) = state.upgrade()
                        {
                            tracing::info!(
                                %connection_id,
                                "mc's sign-in was refused; dropping the cached password"
                            );
                            crate::state::forget_cached_credentials(&state, connection_id);
                        }
                    }) as Box<dyn FnOnce()>
                });
            let path_env = env.as_ref().map(rustconn_core::McSshEnv::path_env);
            let cleanup = ChildCleanup {
                paths: env
                    .as_ref()
                    .and_then(rustconn_core::McSshEnv::secret_file)
                    .map(std::path::Path::to_path_buf)
                    .into_iter()
                    .collect(),
                on_exit,
            };
            let handover = env.filter(rustconn_core::McSshEnv::delivers_password);
            // Delay mc slightly so GTK allocates the VTE widget's final size
            // before mc reads the terminal dimensions at startup.
            glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
                let argv: Vec<&str> = mc_args.iter().map(String::as_str).collect();
                let envv: Option<Vec<&str>> = path_env.as_ref().map(|e| vec![e.as_str()]);
                let spawned = notebook_for_spawn.spawn_command_with_cleanup(
                    session_id,
                    &argv,
                    envv.as_deref(),
                    Some(&downloads),
                    None,
                    cleanup,
                );
                if spawned && let Some(env) = handover {
                    watch_password_handover(&notebook_for_spawn, session_id, env);
                }
            });
        },
    );

    if let Some(sb) = sidebar {
        sb.update_connection_status(&connection_id.to_string(), "connected");
        sb.increment_session_count(&connection_id.to_string(), session_id);
    }
    if let Some(sv) = split_view {
        if let Some(info) = notebook.get_session_info(session_id) {
            sv.add_session(info);
        }
        sv.widget().set_visible(false);
        sv.widget().set_vexpand(false);
    }
    notebook.widget().set_vexpand(true);
    notebook.show_tab_view_content();
}

/// How often [`watch_password_handover`] looks at the wrapper's gate.
const HANDOVER_POLL: std::time::Duration = std::time::Duration::from_millis(500);

/// How many looks [`watch_password_handover`] takes before it gives up.
///
/// mc starts its `ssh` for the `sh://` panel within a second of starting, so
/// 30 seconds is long enough for a slow machine and short enough to stop.
const HANDOVER_POLLS: u32 = 60;

/// Warns once when the wrapper was ready to hand over the password but mc's
/// first `ssh` did not have the argv shape it delivers to.
///
/// Without this an mc build that adds a flag makes delivery fail closed in
/// silence, and the tab waits with no hint to type the password.
// ponytail: polls a marker file every 500 ms for at most 30 s; a gio
// FileMonitor on the wrapper directory would remove the delay if it matters.
fn watch_password_handover(
    notebook: &SharedNotebook,
    session_id: Uuid,
    env: rustconn_core::McSshEnv,
) {
    use rustconn_core::mc_ssh::PasswordHandover;

    let notebook = std::rc::Rc::downgrade(notebook);
    let mut polls = 0;
    glib::timeout_add_local(HANDOVER_POLL, move || {
        let Some(notebook) = notebook.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if notebook.get_terminal(session_id).is_none() {
            return glib::ControlFlow::Break;
        }
        match env.password_handover() {
            Some(PasswordHandover::Delivered) => glib::ControlFlow::Break,
            Some(PasswordHandover::Withheld) => {
                tracing::warn!(
                    %session_id,
                    "mc's ssh did not have the argument shape RustConn hands the password to; mc got no password"
                );
                toast(
                    &notebook,
                    &i18n(
                        "RustConn could not hand the password to mc. Type it in the mc tab when mc waits for it.",
                    ),
                    ToastType::Warning,
                );
                glib::ControlFlow::Break
            }
            None if polls >= HANDOVER_POLLS => {
                tracing::debug!(%session_id, "mc started no ssh yet; no longer watching the password handover");
                glib::ControlFlow::Break
            }
            None => {
                polls += 1;
                glib::ControlFlow::Continue
            }
        }
    });
}

/// What [`mc_password`] found for mc's `ssh`.
enum McPassword {
    /// Hand this password to mc's `ssh`.
    Deliver(SecretString),
    /// Nothing to hand over: a key or agent connection, a route that would
    /// not get it, or no password found.
    Absent,
    /// mc's last sign-in with the saved password was refused: open without
    /// it so the user can type the right one.
    Refused,
}

/// Finds the password mc's `ssh` should get, then calls `then` with it.
///
/// Whether to look at all is [`rustconn_core::mc_ssh::password_plan`]'s table.
/// A refused sign-in reported by the wrapper also drops the cached entry, as a
/// refused Connect does. A lookup reads the session cache, then the source.
/// For **Prompt** the user is asked once; Cancel ends here without `then`.
fn mc_password(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    connection_id: Uuid,
    conn: &Connection,
    then: impl FnOnce(McPassword) + 'static,
) {
    use rustconn_core::mc_ssh::{McPasswordPlan, password_plan};

    // `take_auth_failure` consumes the report, so ask it only for a connection
    // that could have one.
    let refused = rustconn_core::mc_ssh::expects_password(conn)
        && rustconn_core::mc_ssh::take_auth_failure(connection_id);
    if refused {
        crate::state::forget_cached_credentials(state, connection_id);
    }
    match password_plan(conn, refused) {
        McPasswordPlan::Absent => {
            then(McPassword::Absent);
            return;
        }
        McPasswordPlan::Refused => {
            tracing::info!(
                %connection_id,
                "mc's last sign-in was refused; opening mc without the saved password"
            );
            then(McPassword::Refused);
            return;
        }
        McPasswordPlan::Lookup if refused => {
            tracing::info!(%connection_id, "mc's last sign-in was refused; asking again");
        }
        McPasswordPlan::Lookup => {}
    }
    let then = move |password: Option<SecretString>| {
        then(password.map_or(McPassword::Absent, McPassword::Deliver));
    };

    let Ok(state_ref) = state.try_borrow() else {
        then(None);
        return;
    };
    let cached = state_ref
        .get_cached_credentials(connection_id)
        .map(|credentials| credentials.password.clone())
        .filter(|password| !password.expose_secret().is_empty());
    let backend = state_ref.settings().secrets.preferred_backend;
    drop(state_ref);
    if cached.is_some() {
        then(cached);
        return;
    }

    match &conn.password_source {
        PasswordSource::Prompt => ask_password(notebook, &conn.name, move |answer| match answer {
            PromptAnswer::Cancelled => {}
            PromptAnswer::Typed(password) => then(Some(password)),
            PromptAnswer::Empty => then(None),
        }),
        PasswordSource::Vault if crate::vault_miss_cache::known_missing(connection_id, backend) => {
            then(None);
        }
        source if crate::state::password_source_needs_resolution(source) => {
            crate::state::resolve_credentials_gtk(state, connection_id, move |result| {
                use rustconn_core::sync::CredentialResolutionResult;
                let password = match result {
                    Ok(CredentialResolutionResult::Resolved(credentials)) => credentials
                        .password
                        .filter(|password| !password.expose_secret().is_empty()),
                    Ok(CredentialResolutionResult::VaultEntryMissing { .. }) => {
                        crate::vault_miss_cache::note_missing(connection_id, backend);
                        None
                    }
                    Ok(_) => None,
                    Err(error) => {
                        tracing::warn!(%error, "mc: password lookup failed");
                        None
                    }
                };
                then(password);
            });
        }
        _ => then(None),
    }
}

/// How the password dialog ended.
enum PromptAnswer {
    /// Cancel or Escape: open nothing.
    Cancelled,
    /// A password was typed.
    Typed(SecretString),
    /// Open with an empty field: mc opens and the user types it there.
    Empty,
}

/// Asks for the password of the connection `conn_name` once.
fn ask_password(
    notebook: &SharedNotebook,
    conn_name: &str,
    done: impl FnOnce(PromptAnswer) + 'static,
) {
    let dialog = adw::AlertDialog::new(
        Some(&i18n_f("Password for {}", &[conn_name])),
        Some(&i18n(
            "Midnight Commander signs in to this server with it. RustConn keeps it only while this mc tab is open.",
        )),
    );
    let group = adw::PreferencesGroup::new();
    let password_row = adw::PasswordEntryRow::builder()
        .title(i18n("Password"))
        .activates_default(true)
        .build();
    group.add(&password_row);
    dialog.set_extra_child(Some(&group));

    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.add_response("open", &i18n("Open"));
    dialog.set_response_appearance("open", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("open"));
    dialog.set_close_response("cancel");

    let done = std::cell::Cell::new(Some(done));
    let row = password_row.downgrade();
    dialog.connect_response(None, move |_, response| {
        let typed = row.upgrade().map(|row| {
            // The intermediate copy is wiped on drop; the GString that
            // `text()` returns and the entry's own buffer belong to GTK and
            // cannot be cleared from Rust beyond emptying the entry.
            let text = zeroize::Zeroizing::new(row.text().to_string());
            row.set_text("");
            SecretString::from(text.as_str())
        });
        let Some(done) = done.take() else {
            return;
        };
        let answer = match (response, typed) {
            ("open", Some(text)) if !text.expose_secret().is_empty() => PromptAnswer::Typed(text),
            ("open", _) => PromptAnswer::Empty,
            _ => PromptAnswer::Cancelled,
        };
        done(answer);
    });
    dialog.present(Some(notebook.widget()));
    password_row.grab_focus();
}

/// Shows a toast on the window holding `notebook`.
fn toast(notebook: &SharedNotebook, message: &str, toast_type: ToastType) {
    if let Some(root) = notebook.widget().root()
        && let Some(window) = root.downcast_ref::<gtk4::Window>()
    {
        crate::toast::show_toast_on_window(window, message, toast_type);
    }
}
