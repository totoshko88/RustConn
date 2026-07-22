//! Network interface change monitoring via `gio::NetworkMonitor`.
//!
//! Subscribes to `network-changed` signals and, upon a real connectivity
//! change, cleans up stale SSH `ControlMaster` sockets and triggers
//! auto-reconnect for affected sessions (both VTE and embedded RDP/VNC).

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk4::prelude::*;
use gtk4::{gio, glib};

use super::SharedToastOverlay;
use super::types::SharedNotebook;
use crate::i18n::i18n;
use crate::state::SharedAppState;

/// Minimum interval between network-change reactions (debounce).
/// GIO may emit multiple signals in quick succession during a single
/// interface switch.
const DEBOUNCE_SECS: u64 = 3;

/// Delay before triggering reconnects, giving the socket health-check thread
/// time to finish `ssh -O check` on each master (each has a 3s timeout,
/// up to 10 concurrent via `buffer_unordered`).
const RECONNECT_DELAY_MS: u64 = 3000;

/// Maximum number of network-change reactions within a 60-second window
/// before entering quiet mode. Prevents toast spam during VPN reconnect loops.
const MAX_REACTIONS_PER_MINUTE: u32 = 3;

/// Sets up the `gio::NetworkMonitor` listener.
///
/// On `network-changed`:
/// 1. Closes all stale SSH `ControlMaster` sockets so new connections
///    don't try to multiplex over a dead master.
/// 2. Shows a toast informing the user.
/// 3. Triggers in-place reconnect for sessions that have `auto_reconnect`
///    enabled and are currently marked as disconnected (banner visible).
/// 4. Triggers reconnect for embedded RDP/VNC sessions in error state.
///
/// # Note
/// `gio::NetworkMonitor::default()` returns a process-wide singleton —
/// the closure attached via `connect_network_changed` lives for the
/// process lifetime. No prevent-GC guard is needed.
pub fn setup_network_monitor(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    _sidebar: &super::types::SharedSidebar,
    toast_overlay: &SharedToastOverlay,
) {
    let monitor = gio::NetworkMonitor::default();

    let state_clone = state.clone();
    let notebook_clone = notebook.clone();
    let toast_overlay_clone = toast_overlay.clone();

    // Track last reaction time for debouncing
    let last_reaction: Rc<Cell<Option<Instant>>> = Rc::new(Cell::new(None));
    // Track whether we were previously online (to distinguish up/down)
    let was_available: Rc<Cell<bool>> = Rc::new(Cell::new(monitor.is_network_available()));
    // Rate-limit counter: (window_start, count_in_window)
    let rate_limit: Rc<Cell<(Instant, u32)>> = Rc::new(Cell::new((Instant::now(), 0)));
    // Cooldown: after quiet mode ends, suppress reactions for 30s extra to
    // prevent immediate re-triggering when a VPN flap cycle is ~60s.
    let quiet_mode_end: Rc<Cell<Option<Instant>>> = Rc::new(Cell::new(None));

    monitor.connect_network_changed(move |mon, available| {
        let now = Instant::now();

        // Debounce: skip if we reacted less than DEBOUNCE_SECS ago
        if let Some(last) = last_reaction.get()
            && now.duration_since(last).as_secs() < DEBOUNCE_SECS
        {
            return;
        }

        let previously_available = was_available.get();
        was_available.set(available);

        // Only react to meaningful transitions:
        // - Network was up, now it's down or changed (interface switch)
        // - Network was down, now it's back up
        // GIO signals "changed" even for same-state events; filter out noise.
        if available == previously_available && available {
            // Still online and no actual change — could be a spurious signal.
            // We still check socket health (ssh -O check) in case the route
            // changed, but healthy sockets are preserved (#230).
        }

        last_reaction.set(Some(now));

        // Rate limiting: if we've fired too often in the last 60s, enter quiet
        // mode (log only, no toast/reconnect) to avoid spam during VPN loops.
        // After quiet mode ends (window reset), a 30s cooldown prevents the
        // next burst from immediately triggering reconnects.
        let (window_start, window_count) = rate_limit.get();
        let in_quiet_mode = if now.duration_since(window_start).as_secs() >= 60 {
            // Window expired — check if we were in quiet mode
            if window_count > MAX_REACTIONS_PER_MINUTE {
                // Exiting quiet mode: start 30s cooldown
                quiet_mode_end.set(Some(now));
            }
            // Reset the window
            rate_limit.set((now, 1));
            false
        } else {
            let new_count = window_count + 1;
            rate_limit.set((window_start, new_count));
            new_count > MAX_REACTIONS_PER_MINUTE
        };

        // During cooldown after quiet mode, still suppress toast/reconnect
        let in_cooldown = quiet_mode_end
            .get()
            .is_some_and(|end| now.duration_since(end).as_secs() < 30);
        let suppress_reactions = in_quiet_mode || in_cooldown;

        let connectivity = mon.connectivity();

        tracing::info!(
            network_available = available,
            previously_available,
            ?connectivity,
            in_quiet_mode,
            in_cooldown,
            "Network interface changed"
        );

        if !available {
            // Network went down — close stale control sockets so they don't
            // block future connections. Show a banner-like toast.
            close_all_sockets_unconditionally();

            if !suppress_reactions {
                toast_overlay_clone.show_warning(&i18n(
                    "Network disconnected — active sessions may be interrupted",
                ));
            }
            return;
        }

        // Network is available (came back or interface switched).
        // Check which sockets are actually dead — only remove those.
        // This avoids killing healthy sessions when a VPN connects/disconnects
        // without affecting the route to the SSH host (#230).
        close_only_dead_sockets();

        // Check connectivity level: if we're behind a captive portal or have
        // only limited connectivity, reconnecting will fail anyway. Just inform
        // the user and skip the reconnect attempt.
        if connectivity != gio::NetworkConnectivity::Full {
            if !suppress_reactions {
                toast_overlay_clone.show_warning(&i18n(
                    "Network limited — full connectivity not yet available",
                ));
            }
            tracing::info!(
                ?connectivity,
                "Skipping reconnect — connectivity is not Full"
            );
            return;
        }

        if suppress_reactions {
            tracing::debug!("Quiet/cooldown mode: skipping toast and reconnect (rate limited)");
            return;
        }

        // Show informational toast
        toast_overlay_clone.show_toast(&i18n("Network changed — reconnecting affected sessions…"));

        // Trigger reconnect after a short delay to let socket cleanup finish.
        // This prevents new connections from attempting to multiplex through
        // a master socket that is still being closed.
        let state_for_reconnect = state_clone.clone();
        let notebook_for_reconnect = notebook_clone.clone();
        glib::timeout_add_local_once(Duration::from_millis(RECONNECT_DELAY_MS), move || {
            trigger_reconnect_for_disconnected_sessions(
                &state_for_reconnect,
                &notebook_for_reconnect,
            );
            trigger_reconnect_for_embedded_sessions(&state_for_reconnect, &notebook_for_reconnect);
        });
    });
}

/// Closes all RustConn SSH `ControlMaster` sockets unconditionally.
///
/// Used when the network went completely down — all masters are assumed dead.
fn close_all_sockets_unconditionally() {
    if let Err(e) = std::thread::Builder::new()
        .name("net-down-socket-cleanup".into())
        .spawn(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            match rt {
                Ok(rt) => {
                    rt.block_on(rustconn_core::close_all_control_sockets());
                    tracing::debug!("Closed all ControlMaster sockets (network down)");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to create runtime for socket cleanup");
                }
            }
        })
    {
        tracing::warn!(error = %e, "Failed to spawn socket-cleanup thread (ulimit reached?)");
    }
}

/// Checks ControlMaster sockets and removes only dead ones.
///
/// Used on network-change events where the network is still available (e.g.
/// VPN connect/disconnect that doesn't affect the default route). Healthy
/// sockets are left untouched — their SSH sessions continue uninterrupted.
fn close_only_dead_sockets() {
    if let Err(e) = std::thread::Builder::new()
        .name("net-change-socket-check".into())
        .spawn(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            match rt {
                Ok(rt) => {
                    let removed = rt.block_on(rustconn_core::close_dead_control_sockets());
                    tracing::debug!(
                        removed,
                        "Checked ControlMaster sockets after network change"
                    );
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to create runtime for socket check");
                }
            }
        })
    {
        tracing::warn!(error = %e, "Failed to spawn socket-check thread (ulimit reached?)");
    }
}

/// Triggers in-place reconnect for VTE sessions currently showing the
/// disconnect overlay.
fn trigger_reconnect_for_disconnected_sessions(state: &SharedAppState, notebook: &SharedNotebook) {
    // Collect sessions that are currently marked as disconnected
    let disconnected_sessions: Vec<(uuid::Uuid, uuid::Uuid)> = notebook
        .get_all_sessions()
        .into_iter()
        .filter(|info| {
            // Check if the reconnect overlay is visible for this session
            notebook.is_reconnect_shown(info.id)
        })
        .map(|info| (info.id, info.connection_id))
        .collect();

    if disconnected_sessions.is_empty() {
        tracing::debug!("No disconnected sessions to reconnect after network change");
        return;
    }

    tracing::info!(
        count = disconnected_sessions.len(),
        "Triggering reconnect for disconnected sessions after network change"
    );

    let on_reconnect = notebook.reconnect_callback();
    let cb = on_reconnect.borrow();
    let Some(ref callback) = *cb else {
        return;
    };

    for (session_id, connection_id) in &disconnected_sessions {
        // Only reconnect if auto-reconnect is enabled for this connection
        let should_reconnect = state
            .try_borrow()
            .ok()
            .and_then(|s| s.get_connection(*connection_id).cloned())
            .map(|conn| conn.retry_config.as_ref().is_none_or(|rc| rc.enabled))
            .unwrap_or(false);

        if should_reconnect {
            tracing::info!(
                %session_id,
                %connection_id,
                "Network-change triggered reconnect"
            );
            // Cancel any existing poll timer for this session
            // (the network is already back — no need to keep polling)
            notebook.cancel_poll(*session_id);
            callback(*session_id, *connection_id);
        }
    }
}

/// Triggers reconnect for embedded RDP/VNC sessions that are in an error
/// or disconnected state and have auto-reconnect enabled.
///
/// Unlike VTE sessions (which show a reconnect banner), embedded sessions
/// manage their own connection state. This function finds embedded sessions
/// with auto-reconnect enabled and calls their `reconnect()` method directly.
fn trigger_reconnect_for_embedded_sessions(state: &SharedAppState, notebook: &SharedNotebook) {
    let all_sessions = notebook.get_all_sessions();

    for info in &all_sessions {
        // Only target embedded sessions (RDP/VNC)
        if !info.is_embedded {
            continue;
        }
        // Skip sessions that already show the VTE reconnect banner (handled above)
        if notebook.is_reconnect_shown(info.id) {
            continue;
        }

        // Only reconnect if auto-reconnect is enabled for this connection
        let should_reconnect = state
            .try_borrow()
            .ok()
            .and_then(|s| s.get_connection(info.connection_id).cloned())
            .map(|conn| conn.retry_config.as_ref().is_none_or(|rc| rc.enabled))
            .unwrap_or(false);

        if !should_reconnect {
            continue;
        }

        match info.protocol.as_str() {
            "rdp" => {
                if let Some(widget) = notebook.get_rdp_widget(info.id)
                    && widget.is_disconnected()
                {
                    tracing::info!(
                        session_id = %info.id,
                        connection_id = %info.connection_id,
                        "Network-change triggered embedded RDP reconnect"
                    );
                    if let Err(e) = widget.reconnect() {
                        tracing::warn!(
                            session_id = %info.id,
                            error = %e,
                            "Embedded RDP reconnect after network change failed"
                        );
                    }
                }
            }
            "vnc" => {
                if let Some(widget) = notebook.get_vnc_widget(info.id) {
                    let vnc_state = widget.state();
                    if vnc_state.is_disconnected() || vnc_state.is_error() {
                        tracing::info!(
                            session_id = %info.id,
                            connection_id = %info.connection_id,
                            "Network-change triggered embedded VNC reconnect"
                        );
                        if let Err(e) = widget.reconnect() {
                            tracing::warn!(
                                session_id = %info.id,
                                error = %e,
                                "Embedded VNC reconnect after network change failed"
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }
}
