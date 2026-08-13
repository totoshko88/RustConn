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

/// What a `network-changed` signal actually means for the sessions we hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkChange {
    /// The signal carries the monitor's *initial* state rather than a change.
    /// Nothing can be concluded from it, so nothing is done beyond recording it.
    Baseline,
    /// The network is gone; every `ControlMaster` socket is dead.
    Down,
    /// The network is usable and something moved — it came back, or an
    /// interface/route switched underneath a monitor that stayed online.
    Up,
}

/// Classifies a `network-changed` signal.
///
/// `previous` is the availability observed on the previous signal, or `None`
/// when this is the first one to arrive.
///
/// # Why the first signal cannot be trusted
///
/// Inside a Flatpak sandbox `gio::NetworkMonitor::default()` is a
/// `GNetworkMonitorPortal`: it is born reporting `available = false,
/// connectivity = local` and only learns the host's real state asynchronously,
/// announcing it through an ordinary `network-changed` signal a few
/// milliseconds later. Seeding the previous state from `is_network_available()`
/// at construction time therefore records `false`, and that first signal then
/// reads as a `false -> true` transition — an outage that never happened,
/// reported on every single launch. Treating "first signal, network is up" as a
/// baseline removes that whole class of false positive, and does it without a
/// startup timer whose duration would only ever be a guess.
///
/// A first signal reporting the network *down* is not ambiguous — the network
/// is unusable right now whatever preceded it — so it is acted on normally.
#[must_use]
pub fn classify_change(previous: Option<bool>, available: bool) -> NetworkChange {
    match (previous, available) {
        (_, false) => NetworkChange::Down,
        (None, true) => NetworkChange::Baseline,
        (Some(_), true) => NetworkChange::Up,
    }
}

/// Whether this signal is a repeat that the debounce should collapse.
///
/// GIO emits several `network-changed` signals for a single interface switch,
/// and acting on each would mean several socket sweeps for one event. Only
/// repeats are collapsed: a signal classifying differently from the one last
/// acted on carries the transition itself.
///
/// # Why a transition must never be debounced
///
/// The reconnect sweep only runs on the way back up. A flap whose `false` and
/// `true` arrive inside the debounce window — a Wi-Fi roam, a VPN reconnect,
/// switching docking stations — would otherwise be seen as the `false` alone:
/// every `ControlMaster` gets closed, the "network disconnected" warning goes
/// up, and the `true` that should trigger recovery is discarded. Embedded RDP
/// and VNC tabs, whose only automatic recovery path is that sweep, would then
/// stay dead until reconnected by hand.
///
/// `since_last_reaction` is `None` when nothing has been acted on yet.
#[must_use]
pub fn is_debounced_repeat(
    since_last_reaction: Option<Duration>,
    last_change: Option<NetworkChange>,
    change: NetworkChange,
    window: Duration,
) -> bool {
    since_last_reaction.is_some_and(|elapsed| elapsed < window) && last_change == Some(change)
}

/// Sets up the `gio::NetworkMonitor` listener.
///
/// On `network-changed`:
/// 1. Closes all stale SSH `ControlMaster` sockets so new connections
///    don't try to multiplex over a dead master.
/// 2. Triggers in-place reconnect for sessions that have `auto_reconnect`
///    enabled and are currently marked as disconnected (banner visible).
/// 3. Triggers reconnect for embedded RDP/VNC sessions in error state.
/// 4. Shows a toast — but only once steps 2 and 3 have found something to
///    reconnect, so a change that stranded nothing stays silent.
///
/// The very first signal is treated as the monitor's initial state rather than
/// a change; see [`classify_change`].
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
    // Availability seen on the previous signal; `None` until the first arrives.
    // Deliberately *not* seeded from `monitor.is_network_available()` — under
    // the portal implementation that reads `false` before the initial state has
    // been fetched. See `classify_change`.
    let was_available: Rc<Cell<Option<bool>>> = Rc::new(Cell::new(None));
    // How the last acted-on signal classified, so the debounce can tell a
    // repeat apart from a transition. See the debounce below.
    let last_change: Rc<Cell<Option<NetworkChange>>> = Rc::new(Cell::new(None));
    // Rate-limit counter: (window_start, count_in_window)
    let rate_limit: Rc<Cell<(Instant, u32)>> = Rc::new(Cell::new((Instant::now(), 0)));
    // Cooldown: after quiet mode ends, suppress reactions for 30s extra to
    // prevent immediate re-triggering when a VPN flap cycle is ~60s.
    let quiet_mode_end: Rc<Cell<Option<Instant>>> = Rc::new(Cell::new(None));

    monitor.connect_network_changed(move |mon, available| {
        let now = Instant::now();

        // Record what the monitor is telling us *before* anything can return
        // early. A signal that is dropped without updating this is erased from
        // the state machine: the next one is then classified against a stale
        // previous value, and the transition it described is lost for good.
        let previously_available = was_available.replace(Some(available));
        let change = classify_change(previously_available, available);
        let connectivity = mon.connectivity();

        // The monitor's initial state is delivered as a `network-changed`
        // signal too. It is not a change: reacting to it would announce an
        // outage on every launch. Record it and wait for a real one — without
        // consuming the debounce or rate-limit budget, so a genuine change
        // arriving right behind it is still handled.
        if change == NetworkChange::Baseline {
            tracing::debug!(
                ?connectivity,
                "Initial network state received from the monitor; not a change"
            );
            return;
        }

        let since_last_reaction = last_reaction.get().map(|last| now.duration_since(last));
        if is_debounced_repeat(
            since_last_reaction,
            last_change.get(),
            change,
            Duration::from_secs(DEBOUNCE_SECS),
        ) {
            tracing::debug!(?change, "Repeat network signal within debounce; ignored");
            return;
        }

        last_reaction.set(Some(now));
        last_change.set(Some(change));

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

        tracing::info!(
            network_available = available,
            ?previously_available,
            ?change,
            ?connectivity,
            in_quiet_mode,
            in_cooldown,
            "Network interface changed"
        );

        if change == NetworkChange::Down {
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

        // Trigger reconnect after a short delay to let socket cleanup finish.
        // This prevents new connections from attempting to multiplex through
        // a master socket that is still being closed.
        //
        // The toast is raised *here*, once the sweep knows what it actually
        // touched, rather than up front: a network change that stranded no
        // session — the common case, since healthy sockets survive (#230) —
        // must not claim to be reconnecting anything. Same rule the resume
        // detector already follows before reporting a wake-up.
        let state_for_reconnect = state_clone.clone();
        let notebook_for_reconnect = notebook_clone.clone();
        let toast_for_reconnect = toast_overlay_clone.clone();
        glib::timeout_add_local_once(Duration::from_millis(RECONNECT_DELAY_MS), move || {
            let reconnecting =
                reconnect_sessions_after_outage(&state_for_reconnect, &notebook_for_reconnect);
            if reconnecting > 0 {
                toast_for_reconnect
                    .show_toast(&i18n("Network changed — reconnecting affected sessions…"));
            } else {
                tracing::debug!("Network change stranded no session; no toast shown");
            }
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

/// Reconnects every session that an outage left disconnected.
///
/// Shared by the two events that can strand a session: a network interface
/// change (issue #217) and the local machine waking from sleep (issue #248).
/// Both end in the same place — sessions whose transport died while RustConn was
/// unable to do anything about it — so both use one sweep rather than each
/// growing its own copy.
///
/// Only sessions that are *already* known to be disconnected are touched, and
/// only when the connection has auto-reconnect enabled, so a healthy session is
/// never interrupted.
///
/// Returns how many sessions were actually put back into reconnect, so a caller
/// can keep quiet when the answer is none.
pub(super) fn reconnect_sessions_after_outage(
    state: &SharedAppState,
    notebook: &SharedNotebook,
) -> usize {
    trigger_reconnect_for_disconnected_sessions(state, notebook)
        + trigger_reconnect_for_embedded_sessions(state, notebook)
}

/// Triggers in-place reconnect for VTE sessions currently showing the
/// disconnect overlay.
///
/// Returns the number of reconnects triggered.
fn trigger_reconnect_for_disconnected_sessions(
    state: &SharedAppState,
    notebook: &SharedNotebook,
) -> usize {
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
        return 0;
    }

    tracing::info!(
        count = disconnected_sessions.len(),
        "Triggering reconnect for disconnected sessions after network change"
    );

    let on_reconnect = notebook.reconnect_callback();
    let cb = on_reconnect.borrow();
    let Some(ref callback) = *cb else {
        return 0;
    };

    let mut triggered = 0;

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
            triggered += 1;
        }
    }

    triggered
}

/// Triggers reconnect for embedded RDP/VNC sessions that are in an error
/// or disconnected state and have auto-reconnect enabled.
///
/// Unlike VTE sessions (which show a reconnect banner), embedded sessions
/// manage their own connection state. This function finds embedded sessions
/// with auto-reconnect enabled and calls their `reconnect()` method directly.
///
/// Returns the number of reconnects that were successfully started — a widget
/// whose `reconnect()` failed is not reconnecting and must not be counted as if
/// it were.
fn trigger_reconnect_for_embedded_sessions(
    state: &SharedAppState,
    notebook: &SharedNotebook,
) -> usize {
    let all_sessions = notebook.get_all_sessions();
    let mut triggered = 0;

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
                    match widget.reconnect() {
                        Ok(()) => triggered += 1,
                        Err(e) => tracing::warn!(
                            session_id = %info.id,
                            error = %e,
                            "Embedded RDP reconnect after network change failed"
                        ),
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
                        match widget.reconnect() {
                            Ok(()) => triggered += 1,
                            Err(e) => tracing::warn!(
                                session_id = %info.id,
                                error = %e,
                                "Embedded VNC reconnect after network change failed"
                            ),
                        }
                    }
                }
            }
            _ => {}
        }
    }

    triggered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_signal_saying_the_network_is_up_is_only_a_baseline() {
        // GNetworkMonitorPortal delivers its initial state this way, a few
        // milliseconds after the monitor is created. Reacting to it would
        // report an outage on every launch (the toast users saw at startup).
        assert_eq!(classify_change(None, true), NetworkChange::Baseline);
    }

    #[test]
    fn the_first_signal_saying_the_network_is_down_is_still_acted_on() {
        // Nothing ambiguous about it: the network is unusable right now,
        // whatever the monitor believed before it told us anything.
        assert_eq!(classify_change(None, false), NetworkChange::Down);
    }

    #[test]
    fn losing_the_network_is_a_down() {
        assert_eq!(classify_change(Some(true), false), NetworkChange::Down);
    }

    #[test]
    fn getting_the_network_back_is_an_up() {
        assert_eq!(classify_change(Some(false), true), NetworkChange::Up);
    }

    #[test]
    fn a_route_change_while_still_online_is_an_up() {
        // VPN connect/disconnect that leaves the machine online: the sockets
        // still have to be health-checked, healthy ones survive (#230).
        assert_eq!(classify_change(Some(true), true), NetworkChange::Up);
    }

    #[test]
    fn a_repeated_down_stays_a_down() {
        assert_eq!(classify_change(Some(false), false), NetworkChange::Down);
    }

    const WINDOW: Duration = Duration::from_secs(DEBOUNCE_SECS);

    #[test]
    fn the_first_reaction_is_never_debounced() {
        assert!(!is_debounced_repeat(
            None,
            None,
            NetworkChange::Down,
            WINDOW
        ));
    }

    #[test]
    fn a_burst_of_identical_signals_is_collapsed() {
        // One interface switch, several GIO signals: act once.
        assert!(is_debounced_repeat(
            Some(Duration::from_millis(200)),
            Some(NetworkChange::Up),
            NetworkChange::Up,
            WINDOW
        ));
    }

    #[test]
    fn coming_back_up_right_after_going_down_is_never_collapsed() {
        // The flap this whole guard exists for: without the classification
        // check the reconnect sweep would never run and every embedded
        // session would stay dead.
        assert!(!is_debounced_repeat(
            Some(Duration::from_millis(900)),
            Some(NetworkChange::Down),
            NetworkChange::Up,
            WINDOW
        ));
    }

    #[test]
    fn dropping_out_right_after_coming_back_is_never_collapsed() {
        assert!(!is_debounced_repeat(
            Some(Duration::from_millis(900)),
            Some(NetworkChange::Up),
            NetworkChange::Down,
            WINDOW
        ));
    }

    #[test]
    fn an_identical_signal_after_the_window_is_acted_on_again() {
        assert!(!is_debounced_repeat(
            Some(WINDOW),
            Some(NetworkChange::Up),
            NetworkChange::Up,
            WINDOW
        ));
    }
}
