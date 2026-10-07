//! Session restore across restarts (issue #243).
//!
//! Settings → Interface → "Restore sessions on startup" has existed since the
//! settings struct was added, and `rustconn_core::session::restore` has carried a
//! versioned, file-backed state model with its own tests — but nothing ever
//! wrote or read it, so the switch did nothing. This module is the missing
//! wiring: it snapshots the live sessions when the window closes and reopens
//! them on the next start.
//!
//! Scope is deliberately the session *set*, not the layout: split placement and
//! detached windows are what the Workspaces feature is for (`window::workspaces`),
//! and silently rebuilding a layout on every start is more surprising than
//! useful. A detached or split session comes back as an ordinary tab.
//!
//! Tab *state* does come back: the group a tab was in, and whether it was
//! pinned. Both are properties of the tab rather than of the connection, so they
//! travel in the snapshot and are reapplied once the tab exists — the pin
//! included, otherwise a tab the user deliberately kept would come back
//! closable after every restart.

use adw::prelude::*;
use libadwaita as adw;
use rustconn_core::session::{SessionRestoreData, SessionRestoreState, SessionType};

use crate::i18n::{i18n, i18n_f};
use crate::state::SharedAppState;
use crate::terminal::LOCAL_SHELL_PROTOCOL;
use crate::window::types::{SharedNotebook, SharedSidebar};

/// File name of the restore snapshot inside the configuration directory.
const RESTORE_FILE: &str = "session_restore.json";

/// Returns the path of the restore snapshot, or `None` if state is unavailable.
fn restore_path(state: &SharedAppState) -> Option<std::path::PathBuf> {
    state
        .try_borrow()
        .ok()
        .map(|s| s.config_manager().config_dir().join(RESTORE_FILE))
}

/// Snapshots the currently open sessions to disk.
///
/// Called from the window's `close_request` handler, before GTK tears the tabs
/// down. An empty snapshot is written when nothing is open, so quitting with no
/// sessions does not resurrect an earlier set. Does nothing at all when the
/// feature is off, leaving any older snapshot untouched.
pub fn save_snapshot(state: &SharedAppState, notebook: &SharedNotebook) {
    let enabled = state
        .try_borrow()
        .is_ok_and(|s| s.settings().ui.session_restore.enabled);
    if !enabled {
        return;
    }
    let Some(path) = restore_path(state) else {
        return;
    };

    let mut snapshot = SessionRestoreState::new();

    // A detached session has no tab, so it is absent from the ordered list; it is
    // appended and comes back as an ordinary tab (see the module docs).
    let mut ordered = notebook.ordered_session_ids();
    ordered.extend(notebook.detached_session_ids());

    for (index, session_id) in ordered.iter().enumerate() {
        // A tab whose connection already ended is a transcript, not a session —
        // reconnecting it would revive something the user had let go (#242).
        if notebook.is_session_disconnected(*session_id) {
            continue;
        }
        let Some(info) = notebook.get_session_info(*session_id) else {
            continue;
        };
        let session_type = if info.is_embedded {
            SessionType::Embedded
        } else {
            SessionType::External
        };
        let mut data = SessionRestoreData::new(
            info.connection_id,
            info.name.clone(),
            info.protocol.clone(),
            session_type,
        )
        .with_tab_index(index);
        // Carry the tab group structurally so a grouped tab restores under its
        // base label with the `[group]` prefix recomposed, not doubled.
        if let Some(group) = notebook.get_tab_group(*session_id) {
            data = data.with_tab_group(group);
        }
        // Pinning is live TabView state, so the snapshot is the only place a
        // pin can live across a restart. `with_pinned` is unconditional: the
        // value is written even when false, so a snapshot says what the tab was
        // rather than staying silent about it.
        data = data.with_pinned(notebook.is_session_pinned(*session_id));
        snapshot.add_session(data);
    }

    if let Some(active) = notebook.get_active_session_id() {
        snapshot.set_active_session(active);
    }

    match snapshot.save_to_file(&path) {
        Ok(()) => tracing::info!(
            sessions = snapshot.session_count(),
            path = %path.display(),
            "Saved session snapshot for restore"
        ),
        Err(e) => tracing::warn!(error = %e, "Failed to save session snapshot"),
    }
}

/// Loads the snapshot if the feature is on and the snapshot is still fresh.
///
/// Returns `None` when restore is disabled, no snapshot exists, it holds no
/// sessions, or it is older than `max_age_hours` (0 = no limit).
fn load_snapshot(state: &SharedAppState) -> Option<SessionRestoreState> {
    let settings = state
        .try_borrow()
        .ok()
        .map(|s| s.settings().ui.session_restore.clone())?;
    if !settings.enabled {
        return None;
    }

    let path = restore_path(state)?;
    if !path.exists() {
        return None;
    }
    let snapshot = match SessionRestoreState::load_from_file(&path) {
        Ok(snapshot) => snapshot,
        Err(e) => {
            tracing::warn!(error = %e, path = %path.display(), "Failed to read session snapshot");
            return None;
        }
    };
    if !snapshot.has_sessions() {
        return None;
    }

    if settings.max_age_hours > 0 {
        let age = chrono::Utc::now().signed_duration_since(snapshot.saved_at);
        if age.num_hours() >= i64::from(settings.max_age_hours) {
            tracing::info!(
                age_hours = age.num_hours(),
                max_age_hours = settings.max_age_hours,
                "Session snapshot is too old to restore"
            );
            return None;
        }
    }

    Some(snapshot)
}

/// Everything the reopen path needs, bundled because it is handed to a dialog
/// callback that must own each dependency.
pub struct RestoreContext {
    /// Shared application state
    pub state: SharedAppState,
    /// Session notebook that receives the restored tabs
    pub notebook: SharedNotebook,
    /// Split view owner required by the connection-start path
    pub split_view: super::types::SharedSplitView,
    /// Sidebar, for connection status feedback
    pub sidebar: SharedSidebar,
    /// Monitoring coordinator
    pub monitoring: super::types::SharedMonitoring,
    /// Activity coordinator
    pub activity: super::types::SharedActivityCoordinator,
}

/// Restores the previous session set, asking first when configured.
///
/// A no-op when restore is disabled or the snapshot is missing, empty or stale.
pub fn restore_previous_sessions(ctx: RestoreContext, window: &gtk4::Window) {
    let Some(snapshot) = load_snapshot(&ctx.state) else {
        return;
    };
    let prompt = ctx
        .state
        .try_borrow()
        .is_ok_and(|s| s.settings().ui.session_restore.prompt_on_restore);

    if !prompt {
        reopen(&ctx, &snapshot);
        return;
    }

    let count = snapshot.session_count();
    let dialog = adw::AlertDialog::new(
        Some(&i18n("Restore previous session?")),
        Some(&i18n_f(
            "Reopen the {} connections that were open when RustConn last closed.",
            &[&count.to_string()],
        )),
    );
    dialog.add_response("cancel", &i18n("Not Now"));
    dialog.add_response("restore", &i18n("Restore"));
    dialog.set_response_appearance("restore", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("restore"));
    dialog.set_close_response("cancel");

    dialog.connect_response(Some("restore"), move |_, _| {
        reopen(&ctx, &snapshot);
    });
    dialog.present(Some(window));
}

/// Opens every connection listed in the snapshot.
///
/// An entry whose connection has since been deleted is skipped with a warning
/// instead of failing the whole restore.
fn reopen(ctx: &RestoreContext, snapshot: &SessionRestoreState) {
    let mut restored = 0usize;
    let mut missing = 0usize;

    for entry in &snapshot.sessions {
        if entry.connection_id.is_nil() && entry.protocol == LOCAL_SHELL_PROTOCOL {
            // The snapshot carries the tab's title, so a local shell the user
            // relabelled comes back under that name instead of as a second
            // "Local Shell". Its tab group is reapplied so it reopens grouped.
            let session_id = super::MainWindow::open_local_shell_with_split(
                &ctx.notebook,
                &ctx.split_view,
                Some(&ctx.state),
                Some(&entry.connection_name),
                entry.tab_group.as_deref(),
            );
            // A local shell starts immediately, so the id is known here and the
            // pin can be reapplied synchronously.
            ctx.notebook.set_session_pinned(session_id, entry.pinned);
            restored += 1;
            continue;
        }

        let exists = ctx
            .state
            .try_borrow()
            .is_ok_and(|s| s.get_connection(entry.connection_id).is_some());
        if !exists {
            tracing::warn!(
                connection = %entry.connection_name,
                id = %entry.connection_id,
                "Session restore: connection no longer exists, skipping"
            );
            missing += 1;
            continue;
        }

        // A connection start is asynchronous (credential resolution runs first),
        // so the session id is not known here. Observe the exact session the
        // start creates and reapply its tab group and pin once it exists,
        // mirroring the synchronous local-shell path above.
        let observer = tab_observer(&ctx.notebook, entry);

        super::MainWindow::start_connection_with_credential_resolution_observed(
            ctx.state.clone(),
            ctx.notebook.clone(),
            ctx.split_view.clone(),
            ctx.sidebar.clone(),
            ctx.monitoring.clone(),
            entry.connection_id,
            Some(ctx.activity.clone()),
            observer,
        );
        restored += 1;
    }

    tracing::info!(restored, missing, "Session restore finished");
    if missing > 0 {
        crate::toast::show_warning_toast_on_active_window(&i18n_f(
            "{} restored connections no longer exist",
            &[&missing.to_string()],
        ));
    }
}

/// Builds the post-start observer that reapplies a snapshot entry's tab state,
/// or `None` when the entry has no group and no pin to reapply.
///
/// One observer carries both, because a connection start is what decides the
/// session id — a second observer would race the first for the same completion.
/// Hold the notebook weakly, as the reconnect observer does (window-guide): the
/// closure outlives the start, so an `Rc` here would be a cycle back to the
/// notebook that owns the session state.
fn tab_observer(
    notebook: &SharedNotebook,
    entry: &SessionRestoreData,
) -> Option<crate::window::types::SessionStartObserver> {
    let group = entry.tab_group.clone().filter(|group| !group.is_empty());
    if group.is_none() && !entry.pinned {
        return None;
    }
    let pinned = entry.pinned;
    let notebook = std::rc::Rc::downgrade(notebook);
    Some(crate::window::types::SessionStartObserver::new(
        move |session_id| {
            if let Some(notebook) = notebook.upgrade() {
                if let Some(group) = group.as_deref() {
                    notebook.set_tab_group(session_id, group);
                }
                notebook.set_session_pinned(session_id, pinned);
            }
        },
    ))
}
