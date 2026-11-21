//! Cross-tab keystroke broadcast (issue #329).
//!
//! Split-view broadcast (see [`crate::window::navigation_actions`]) mirrors
//! keystrokes across the panes of one tab, whose state lives on a per-tab
//! [`crate::split_view::SplitViewBridge`]. A *group* broadcast spans sessions
//! that each live on their own tab, so its state cannot live on any one bridge —
//! it lives on [`GroupBroadcast`], owned by the window.
//!
//! # Model
//!
//! Membership is an **explicit per-session opt-in**, not derived from an
//! existing tab-group label. The user adds a tab to the broadcast set from the
//! tab context menu; the set is a plain `HashSet<Uuid>` of session ids. This is
//! the iTerm2 model, and it avoids the trap iTerm2 itself hit once
//! (<https://gitlab.com/gnachman/iterm2/-/issues/3671>) where a broadcast whose
//! target set was *inferred* rather than explicit silently reached sessions the
//! user did not intend.
//!
//! # Safety
//!
//! A group broadcast is inherently more dangerous than the split one: the target
//! tabs are not all visible at once, so a `rm -rf` typed once lands on hosts the
//! user cannot see. Three guards, all owned here or by the window that drives
//! this type:
//!
//! - typing only mirrors while [`GroupBroadcast::is_active`] — an explicit,
//!   persistently-bannered mode, never on by accident;
//! - every commit re-checks live membership, because the VTE `commit` handler is
//!   wired once and never removed (a tab dropped from the set keeps its handler);
//! - only terminal-backed sessions participate — an embedded RDP/VNC/SPICE
//!   viewer has no PTY to feed and is skipped by the caller.
//!
//! The persistent banner and the large-group confirmation live in the window
//! layer, not here, but they exist because of this type.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;

use super::*;
use crate::i18n::{i18n, i18n_f};

/// Above this many members, enabling the broadcast asks for confirmation first.
///
/// A handful of hosts is the common, intended case; a mistaken toggle onto a
/// large set is where "the same command on every production box" happens. The
/// threshold is a deliberate speed bump, not a hard limit.
pub const GROUP_BROADCAST_CONFIRM_THRESHOLD: usize = 5;

thread_local! {
    /// The one re-entrancy guard every [`GroupBroadcast`] shares.
    ///
    /// The main window has one broadcast and each cluster window its own
    /// (issue #436). A session can sit in two of them at once — added to the
    /// main window's set, then moved to a cluster window — and a keystroke fed
    /// by one broadcast fires `commit` in the other. With a guard per broadcast
    /// the second would mirror the fed text back, so every character arrived
    /// twice. GTK runs on one thread, so a thread-local is the whole scope.
    static BROADCAST_BUSY: Rc<Cell<bool>> = Rc::new(Cell::new(false));
}

/// Window-owned state for cross-tab keystroke broadcast.
///
/// Cheaply cloneable: every field is an `Rc`, so the value can be cloned into
/// the per-session `commit` closures and the window actions that mutate it. All
/// clones share one set of members and one re-entrancy guard — the guard MUST be
/// shared, or each closure's own flag would let the `feed_child → commit →
/// feed_child` cascade double every character.
#[derive(Clone)]
pub struct GroupBroadcast {
    /// Session ids the user has opted into the broadcast set.
    members: Rc<RefCell<HashSet<Uuid>>>,
    /// Whether mirroring is currently on. `false` makes every wired handler a
    /// no-op regardless of membership.
    active: Rc<Cell<bool>>,
    /// Shared re-entrancy guard across all wired commit handlers, and across
    /// every `GroupBroadcast` (see `BROADCAST_BUSY`). Set while a broadcast pass
    /// feeds the other members, so their own `commit` signals do not
    /// re-broadcast.
    busy: Rc<Cell<bool>>,
    /// Sessions whose `commit` handler is already wired, for idempotency.
    wired: Rc<RefCell<HashSet<Uuid>>>,
}

impl Default for GroupBroadcast {
    fn default() -> Self {
        Self::new()
    }
}

impl GroupBroadcast {
    /// Creates an empty, inactive broadcast group.
    #[must_use]
    pub fn new() -> Self {
        Self {
            members: Rc::new(RefCell::new(HashSet::new())),
            active: Rc::new(Cell::new(false)),
            busy: BROADCAST_BUSY.with(Rc::clone),
            wired: Rc::new(RefCell::new(HashSet::new())),
        }
    }

    /// Returns whether mirroring is currently on.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active.get()
    }

    /// Sets the active flag, returning the new value.
    pub fn set_active(&self, active: bool) -> bool {
        self.active.set(active);
        active
    }

    /// Returns whether `session_id` is in the broadcast set.
    #[must_use]
    pub fn contains(&self, session_id: Uuid) -> bool {
        self.members.borrow().contains(&session_id)
    }

    /// Number of sessions in the broadcast set.
    #[must_use]
    pub fn member_count(&self) -> usize {
        self.members.borrow().len()
    }

    /// Snapshot of the current members (order unspecified).
    #[must_use]
    pub fn members(&self) -> Vec<Uuid> {
        self.members.borrow().iter().copied().collect()
    }

    /// Adds a session to the broadcast set. Returns `true` if it was newly added.
    pub fn add(&self, session_id: Uuid) -> bool {
        self.members.borrow_mut().insert(session_id)
    }

    /// Removes a session from the broadcast set. Returns `true` if it was present.
    ///
    /// Dropping below two members also turns mirroring off: a one-member (or
    /// empty) broadcast has nothing to mirror to, and leaving it "active" would
    /// keep the banner up over a set that does nothing.
    pub fn remove(&self, session_id: Uuid) -> bool {
        let removed = self.members.borrow_mut().remove(&session_id);
        if self.members.borrow().len() < 2 {
            self.active.set(false);
        }
        removed
    }

    /// Toggles membership, returning `true` if the session is now a member.
    pub fn toggle(&self, session_id: Uuid) -> bool {
        if self.contains(session_id) {
            self.remove(session_id);
            false
        } else {
            self.add(session_id)
        }
    }

    /// Forgets a closed session entirely (membership and wired record).
    ///
    /// Called from the page-closed path so a reused `Uuid` can never inherit a
    /// dead session's membership, and the wired set does not grow without bound.
    pub fn forget(&self, session_id: Uuid) {
        self.remove(session_id);
        self.wired.borrow_mut().remove(&session_id);
    }

    /// Records that a session's commit handler has been wired, returning `true`
    /// if this is the first time (so the caller should actually connect it).
    pub fn mark_wired(&self, session_id: Uuid) -> bool {
        self.wired.borrow_mut().insert(session_id)
    }

    /// Runs `pass` with the shared busy guard held, unless it is already held.
    ///
    /// Returns `false` without running `pass` when a broadcast pass is already in
    /// progress — this is the re-entrancy guard that stops fed text from
    /// re-broadcasting.
    pub fn with_busy_guard<F: FnOnce()>(&self, pass: F) -> bool {
        if self.busy.get() {
            return false;
        }
        self.busy.set(true);
        pass();
        self.busy.set(false);
        true
    }
}

/// Resolves the mirror targets for a keystroke originating in `source`.
///
/// Pure and free of GTK so the branching that decides *who receives a
/// broadcast* is unit-testable. Returns the members to feed — everyone in the
/// set except the source — but only when the broadcast is active and the source
/// is itself a member. An inactive broadcast, or input from a non-member tab,
/// resolves to no targets: a non-member tab types only into itself even while a
/// group broadcast is running elsewhere (the iTerm2 asymmetric rule).
#[must_use]
pub fn resolve_broadcast_targets(active: bool, source: Uuid, members: &[Uuid]) -> Vec<Uuid> {
    if !active || !members.contains(&source) {
        return Vec::new();
    }
    members
        .iter()
        .copied()
        .filter(|&sid| sid != source)
        .collect()
}

/// Wires a session's VTE `commit` signal into the group broadcast chain.
///
/// Idempotent: a session is wired at most once (tracked by
/// [`GroupBroadcast::mark_wired`]), so it is safe to call from every path that
/// can add a tab to the set. The handler stays connected for the session's
/// life — there is no unwire step — so it re-checks membership and the active
/// flag on every keystroke via [`resolve_broadcast_targets`], and does nothing
/// unless this session is a member of an active broadcast.
///
/// Only terminal-backed sessions are wired; an embedded RDP/VNC/SPICE viewer
/// has no PTY to mirror to and is skipped.
pub fn wire_group_broadcast_for_session(
    group: &GroupBroadcast,
    notebook: &SharedNotebook,
    sid: Uuid,
) {
    if notebook.get_terminal(sid).is_none() {
        tracing::debug!(%sid, "group broadcast: skipping non-terminal session");
        return;
    }
    if !group.mark_wired(sid) {
        return; // already wired
    }

    let group_for_cb = group.clone();
    let notebook_for_cb = notebook.clone();
    notebook.connect_commit(sid, move |text| {
        let targets =
            resolve_broadcast_targets(group_for_cb.is_active(), sid, &group_for_cb.members());
        if targets.is_empty() {
            return;
        }
        group_for_cb.with_busy_guard(|| {
            for target in targets {
                notebook_for_cb.send_text_to_session(target, text);
            }
        });
    });
}

/// Asks before broadcasting to more than [`GROUP_BROADCAST_CONFIRM_THRESHOLD`]
/// sessions; runs `on_accept` only when the user enables it.
///
/// Shared by the main window's group broadcast and a cluster window's
/// broadcast (issue #436), so both ask the same question the same way.
pub fn confirm_large_group_broadcast<F>(window: &adw::ApplicationWindow, count: usize, on_accept: F)
where
    F: Fn() + 'static,
{
    let dialog = adw::AlertDialog::new(
        Some(&i18n("Broadcast to all these tabs?")),
        Some(&i18n_f(
            "Keystrokes will be sent to {} sessions at once, including tabs that are not visible. A mistyped command runs on all of them.",
            &[&count.to_string()],
        )),
    );
    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.add_response("enable", &i18n("Enable Broadcast"));
    dialog.set_response_appearance("enable", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    dialog.connect_response(None, move |_, response| {
        if response == "enable" {
            on_accept();
        }
    });
    dialog.present(Some(window));
}

impl MainWindow {
    /// Registers the group-broadcast window actions and keeps the header toggle
    /// and the active-broadcast banner in sync (issue #329).
    ///
    /// Three actions: `win.toggle-tab-broadcast` (target = session-id string,
    /// toggles one tab's membership from the tab menu), `win.toggle-group-broadcast`
    /// (the header toggle / keybinding), and `win.group-broadcast-confirmed`
    /// (one-shot, activated by the large-group confirmation on accept).
    pub(crate) fn setup_group_broadcast_actions(
        &self,
        window: &adw::ApplicationWindow,
        notebook: &SharedNotebook,
    ) {
        // Wire the tab context menu → window bridge for membership. The notebook
        // owns the menu; the window owns the group state.
        {
            let group_for_query = self.group_broadcast.clone();
            notebook
                .set_tab_broadcast_membership_provider(move |sid| group_for_query.contains(sid));
        }
        {
            let window_weak = window.downgrade();
            notebook.set_on_tab_broadcast_toggle(move |session_id| {
                if let Some(win) = window_weak.upgrade() {
                    gio::prelude::ActionGroupExt::activate_action(
                        &win,
                        "toggle-tab-broadcast",
                        Some(&session_id.to_string().to_variant()),
                    );
                }
            });
        }

        // Per-tab membership toggle, target = session id string. Captures the
        // handles it needs directly rather than routing back through a
        // `&MainWindow`, which a `'static` action closure cannot hold.
        let toggle_tab_action =
            gio::SimpleAction::new("toggle-tab-broadcast", Some(glib::VariantTy::STRING));
        let group_tab = self.group_broadcast.clone();
        let notebook_tab = notebook.clone();
        let toggle_tab = self.group_broadcast_toggle.clone();
        let banner_tab = self.group_broadcast_banner.clone();
        let toast_tab = self.toast_overlay.clone();
        let window_weak_tab = window.downgrade();
        toggle_tab_action.connect_activate(move |_, param| {
            let Some(sid) = param
                .and_then(glib::Variant::str)
                .and_then(|s| Uuid::parse_str(s).ok())
            else {
                return;
            };
            Self::toggle_tab_broadcast_membership(
                &group_tab,
                &notebook_tab,
                &toggle_tab,
                &banner_tab,
                window_weak_tab.upgrade().as_ref(),
                sid,
            );
            // The toggle appears only with two members, and nothing else says
            // so after the first Add.
            if group_tab.contains(sid) && group_tab.member_count() == 1 {
                toast_tab.show_toast(&crate::i18n::with_accel(
                    &i18n("Add one more tab to start Tab Broadcast"),
                    "win.toggle-group-broadcast",
                ));
            }
        });
        window.add_action(&toggle_tab_action);

        // Mode toggle.
        let action =
            gio::SimpleAction::new_stateful("toggle-group-broadcast", None, &false.to_variant());
        action.set_enabled(false);
        let group = self.group_broadcast.clone();
        let notebook_for_action = notebook.clone();
        let toast = self.toast_overlay.clone();
        let banner = self.group_broadcast_banner.clone();
        let toggle = self.group_broadcast_toggle.clone();
        let window_weak = window.downgrade();
        action.connect_activate(move |action, _| {
            let turning_on = !group.is_active();
            if turning_on && group.member_count() > GROUP_BROADCAST_CONFIRM_THRESHOLD {
                if let Some(win) = window_weak.upgrade() {
                    let win_for_accept = win.downgrade();
                    confirm_large_group_broadcast(&win, group.member_count(), move || {
                        if let Some(win) = win_for_accept.upgrade() {
                            gio::prelude::ActionGroupExt::activate_action(
                                &win,
                                "group-broadcast-confirmed",
                                None,
                            );
                        }
                    });
                }
                return;
            }
            Self::apply_group_broadcast_state(
                turning_on,
                &group,
                &notebook_for_action,
                action,
                &toggle,
                &banner,
                &toast,
            );
        });
        window.add_action(&action);

        // One-shot the confirmation activates on accept, so the enable happens
        // after consent without re-hitting the threshold prompt.
        let confirmed = gio::SimpleAction::new("group-broadcast-confirmed", None);
        let group_c = self.group_broadcast.clone();
        let notebook_c = notebook.clone();
        let toast_c = self.toast_overlay.clone();
        let banner_c = self.group_broadcast_banner.clone();
        let toggle_c = self.group_broadcast_toggle.clone();
        let action_for_confirm = action.clone();
        confirmed.connect_activate(move |_, _| {
            Self::apply_group_broadcast_state(
                true,
                &group_c,
                &notebook_c,
                &action_for_confirm,
                &toggle_c,
                &banner_c,
                &toast_c,
            );
        });
        window.add_action(&confirmed);

        // Recompute the toggle's visibility/state whenever the active tab
        // changes: it is only meaningful when the active tab is a member and the
        // set has two or more members.
        let group_for_switch = self.group_broadcast.clone();
        let notebook_for_switch = notebook.clone();
        let toggle_for_switch = self.group_broadcast_toggle.clone();
        let action_for_switch = action;
        notebook.tab_view().connect_selected_page_notify(move |_| {
            Self::update_group_broadcast_toggle(
                &group_for_switch,
                &notebook_for_switch,
                &toggle_for_switch,
                &action_for_switch,
            );
        });
    }

    /// Applies a new active state: wires members, updates the action/toggle,
    /// shows or hides the banner, and toasts the change.
    fn apply_group_broadcast_state(
        active: bool,
        group: &GroupBroadcast,
        notebook: &SharedNotebook,
        action: &gio::SimpleAction,
        toggle: &gtk4::ToggleButton,
        banner: &adw::Banner,
        toast: &crate::window::SharedToastOverlay,
    ) {
        group.set_active(active);
        action.set_state(&active.to_variant());
        toggle.set_active(active);

        if active {
            for sid in group.members() {
                wire_group_broadcast_for_session(group, notebook, sid);
            }
            toggle.add_css_class("broadcasting");
            banner.set_title(&i18n_f(
                "Tab Broadcast active — keystrokes mirrored to {} sessions",
                &[&group.member_count().to_string()],
            ));
            banner.set_revealed(true);
            // No toast on enable: the persistent banner already announces the
            // active state, and it is the correct surface for a dangerous mode
            // that reaches tabs the user cannot see (GNOME HIG — a state needing
            // attention is a banner, not a transient toast). A toast here would
            // just duplicate the banner and add noise. The disable path below
            // still toasts, because the banner vanishes and a transient
            // confirmation is the only feedback left.
        } else {
            toggle.remove_css_class("broadcasting");
            banner.set_revealed(false);
            toast.show_toast(&i18n(
                "Tab Broadcast disabled — keystrokes go to the focused tab only",
            ));
        }
        // Stopping from a tab outside the set leaves nothing to show.
        Self::update_group_broadcast_toggle(group, notebook, toggle, action);
    }

    /// Toggles a session's membership in the broadcast set and refreshes chrome.
    ///
    /// Called from the tab context menu via the `toggle-tab-broadcast` action.
    /// Adding a member wires its commit handler immediately (idempotent), so
    /// enabling the broadcast later needs no rescan; removing updates the tab
    /// marker and may deactivate the mode if the set drops below two members.
    ///
    /// An associated function rather than a `&self` method because the action
    /// closure that calls it is `'static` and holds only cloned handles.
    fn toggle_tab_broadcast_membership(
        group: &GroupBroadcast,
        notebook: &SharedNotebook,
        toggle: &gtk4::ToggleButton,
        banner: &adw::Banner,
        window: Option<&adw::ApplicationWindow>,
        session_id: Uuid,
    ) {
        let now_member = group.toggle(session_id);
        if now_member {
            wire_group_broadcast_for_session(group, notebook, session_id);
        }
        notebook.set_broadcast_member_marker(session_id, now_member);

        let action = window
            .and_then(|w| w.lookup_action("toggle-group-broadcast"))
            .and_then(|a| a.downcast::<gio::SimpleAction>().ok());

        // Removing may have dropped the set below two members, deactivating it.
        if !group.is_active() {
            banner.set_revealed(false);
            toggle.remove_css_class("broadcasting");
            toggle.set_active(false);
            if let Some(ref action) = action {
                action.set_state(&false.to_variant());
            }
        }

        if let Some(action) = action {
            Self::update_group_broadcast_toggle(group, notebook, toggle, &action);
        }
    }

    /// Shows the group toggle when the active tab is a member and the set has
    /// two or more members, or whenever the broadcast is active; hides it
    /// otherwise. Also reflects the active state.
    fn update_group_broadcast_toggle(
        group: &GroupBroadcast,
        notebook: &SharedNotebook,
        toggle: &gtk4::ToggleButton,
        action: &gio::SimpleAction,
    ) {
        let active_is_member = notebook
            .get_active_session_id()
            .is_some_and(|sid| group.contains(sid));
        let active = group.is_active();
        // An active broadcast stays switchable off from any tab: the banner's
        // Stop button and Ctrl+Shift+A use this action, and the mode reaches
        // tabs the user may not be looking at.
        let show = (active_is_member && group.member_count() >= 2) || active;

        toggle.set_visible(show);
        action.set_enabled(show);
        if show {
            toggle.set_active(active);
            action.set_state(&active.to_variant());
            if active {
                toggle.add_css_class("broadcasting");
            } else {
                toggle.remove_css_class("broadcasting");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: usize) -> Vec<Uuid> {
        (0..n).map(|_| Uuid::new_v4()).collect()
    }

    // A keystroke in a member tab reaches every other member, never the source.
    #[test]
    fn active_member_source_reaches_all_other_members() {
        let m = ids(3);
        let targets = resolve_broadcast_targets(true, m[0], &m);
        assert_eq!(targets.len(), 2);
        assert!(!targets.contains(&m[0]));
        assert!(targets.contains(&m[1]));
        assert!(targets.contains(&m[2]));
    }

    // SHALL CONTINUE TO: an inactive broadcast mirrors nothing, so a member tab
    // types only into itself.
    #[test]
    fn inactive_broadcast_resolves_no_targets() {
        let m = ids(3);
        assert_eq!(
            resolve_broadcast_targets(false, m[0], &m),
            Vec::<uuid::Uuid>::new()
        );
    }

    // SHALL CONTINUE TO: a non-member tab types only into itself even while the
    // broadcast is active (asymmetric rule).
    #[test]
    fn non_member_source_resolves_no_targets() {
        let m = ids(3);
        let outsider = Uuid::new_v4();
        assert_eq!(
            resolve_broadcast_targets(true, outsider, &m),
            Vec::<uuid::Uuid>::new()
        );
    }

    // Removing a member below two participants turns the mode off, so a stale
    // banner never sits over a set that can mirror nothing.
    #[test]
    fn dropping_below_two_members_deactivates() {
        let gb = GroupBroadcast::new();
        let m = ids(2);
        gb.add(m[0]);
        gb.add(m[1]);
        gb.set_active(true);
        assert!(gb.is_active());
        gb.remove(m[1]);
        assert!(!gb.is_active());
    }

    // The busy guard refuses re-entry, which is what stops fed text from
    // re-broadcasting and doubling characters.
    #[test]
    fn busy_guard_blocks_reentry() {
        let gb = GroupBroadcast::new();
        let mut inner_ran = false;
        let ran = gb.with_busy_guard(|| {
            let reentered = gb.with_busy_guard(|| {});
            assert!(
                !reentered,
                "guard must refuse re-entry while a pass is active"
            );
            inner_ran = true;
        });
        assert!(ran);
        assert!(inner_ran);
        assert!(gb.with_busy_guard(|| {}));
    }

    // A cluster window's broadcast and the main window's share one guard
    // (issue #436), so text fed by one is not mirrored back by the other.
    #[test]
    fn separate_broadcasts_share_the_busy_guard() {
        let main = GroupBroadcast::new();
        let cluster_window = GroupBroadcast::new();
        let ran = main.with_busy_guard(|| {
            assert!(!cluster_window.with_busy_guard(|| {}));
        });
        assert!(ran);
        assert!(cluster_window.with_busy_guard(|| {}));
    }

    // toggle() flips membership and reports the resulting state.
    #[test]
    fn toggle_reports_membership() {
        let gb = GroupBroadcast::new();
        let sid = Uuid::new_v4();
        assert!(gb.toggle(sid));
        assert!(gb.contains(sid));
        assert!(!gb.toggle(sid));
        assert!(!gb.contains(sid));
    }
}
