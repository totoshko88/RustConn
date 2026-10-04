//! Moving a session between its tab and a detached window.
//!
//! Extracted from `terminal/mod.rs` following the same `impl TerminalNotebook`
//! extension pattern as `tab_menu.rs`. The notebook stays the single owner of
//! all session state: a detached window only borrows the session's widget
//! subtree, so every per-session map keeps working unchanged while the session
//! lives outside the main window.

use rustconn_core::{DetachContext, DetachVerdict, detach_verdict};

use super::*;

/// Stable monitor preference for a detached fullscreen window.
///
/// The connector is preferred because display-list indices change after
/// hotplug. Manufacturer/model are a fallback for backends that expose no
/// connector; `index` is used only when no identity is available at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetachMonitor {
    /// Monitor-list index when the preference was captured.
    pub index: u32,
    /// Stable display connector, such as `DP-1`, when available.
    pub connector: Option<String>,
    /// Display manufacturer, used with `model` as a fallback identity.
    pub manufacturer: Option<String>,
    /// Display model, used with `manufacturer` as a fallback identity.
    pub model: Option<String>,
}

impl DetachMonitor {
    /// Captures the identity exposed by a currently connected monitor.
    #[must_use]
    pub fn from_monitor(index: u32, monitor: Option<&gtk4::gdk::Monitor>) -> Self {
        Self {
            index,
            connector: monitor
                .and_then(gtk4::gdk::Monitor::connector)
                .map(|value| value.to_string()),
            manufacturer: monitor
                .and_then(gtk4::gdk::Monitor::manufacturer)
                .map(|value| value.to_string()),
            model: monitor
                .and_then(gtk4::gdk::Monitor::model)
                .map(|value| value.to_string()),
        }
    }

    /// Reports whether this preference carries a stable monitor identity.
    #[must_use]
    pub fn has_identity(&self) -> bool {
        self.connector.is_some() || self.manufacturer.is_some() || self.model.is_some()
    }

    /// Matches identity properties from a currently connected monitor.
    #[must_use]
    pub fn matches_identity(
        &self,
        connector: Option<&str>,
        manufacturer: Option<&str>,
        model: Option<&str>,
    ) -> bool {
        if let Some(expected) = self.connector.as_deref() {
            return connector == Some(expected);
        }
        self.has_identity()
            && self.manufacturer.as_deref() == manufacturer
            && self.model.as_deref() == model
    }
}

/// Requested presentation for a detached session window.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DetachPresentation {
    /// Whether the detached window should be fullscreen.
    pub fullscreen: bool,
    /// Preferred monitor identity for fullscreen presentation.
    pub monitor: Option<DetachMonitor>,
}

/// Where a session sits relative to a split layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SplitMembership {
    /// The session is not part of any split.
    Outside,
    /// The session's own tab hosts the split layout.
    Owner,
    /// The session's widget lives in another tab's split layout.
    Guest,
}

impl SplitMembership {
    /// Derives membership from the notebook's two split markers (pure).
    ///
    /// A split color marks every participant, owners and guests alike, while
    /// only a guest is parked out of the tab bar — so a parked session is a
    /// guest and any other colored session is the owner of its own split.
    const fn from_marks(has_split_color: bool, is_parked_in_split: bool) -> Self {
        if is_parked_in_split {
            Self::Guest
        } else if has_split_color {
            Self::Owner
        } else {
            Self::Outside
        }
    }
}

/// Why a session currently has no tab page of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParkMark {
    /// The session's widget lives in another tab's split layout.
    InSplit,
    /// The session's widget lives in a detached window.
    Detached,
}

/// Removes a session from both park sets, reporting the mark it held (pure).
///
/// A session is parked for exactly one reason at a time, so clearing both sets
/// keeps un-parking a single decision point: a session leaving a split can
/// never come back still marked detached, or the other way round. When both
/// marks are somehow set, the split mark wins, because the split layout is what
/// physically holds the widget. Returns `None` for a session that was not
/// parked at all, which is the caller's signal to do nothing.
pub(super) fn take_park_mark(
    parked_in_split: &mut HashSet<Uuid>,
    detached: &mut HashSet<Uuid>,
    session_id: Uuid,
) -> Option<ParkMark> {
    let was_in_split = parked_in_split.remove(&session_id);
    let was_detached = detached.remove(&session_id);
    match (was_in_split, was_detached) {
        (true, _) => Some(ParkMark::InSplit),
        (false, true) => Some(ParkMark::Detached),
        (false, false) => None,
    }
}

/// Builds the placement context from observed notebook facts (pure, GTK-free).
#[must_use]
const fn detach_context_from(
    renders_in_process: bool,
    split: SplitMembership,
    is_detached: bool,
) -> DetachContext {
    DetachContext {
        renders_in_process,
        is_split_owner: matches!(split, SplitMembership::Owner),
        is_split_guest: matches!(split, SplitMembership::Guest),
        is_detached,
    }
}

/// The shared session state a detach decision and a detach request need.
///
/// The tab context menu is wired from `TerminalNotebook::new`, while the
/// notebook is still a plain value, so its handlers cannot hold a back
/// reference to it. They hold this bundle of the very same `Rc` handles
/// instead, which keeps the verdict and the request hook in one place for both
/// the notebook and the menu.
#[derive(Clone)]
pub(super) struct DetachHooks {
    session_info: Rc<RefCell<HashMap<Uuid, TerminalSession>>>,
    parked_in_split: Rc<RefCell<HashSet<Uuid>>>,
    split_session_colors: Rc<RefCell<HashMap<Uuid, usize>>>,
    detached: Rc<RefCell<HashSet<Uuid>>>,
    on_detach_request: Rc<RefCell<Option<Box<dyn Fn(Uuid, DetachPresentation) -> bool>>>>,
}

impl DetachHooks {
    /// Reports whether a session may be moved into a detached window.
    pub(super) fn verdict(&self, session_id: Uuid) -> DetachVerdict {
        detach_verdict(&self.context(session_id))
    }

    /// Collects the placement facts the verdict is derived from.
    fn context(&self, session_id: Uuid) -> DetachContext {
        // Scope every borrow so no two RefCell borrows overlap.
        let renders_in_process = self
            .session_info
            .borrow()
            .get(&session_id)
            .is_some_and(|info| info.is_embedded);
        let is_parked_in_split = self.parked_in_split.borrow().contains(&session_id);
        let has_split_color = self.split_session_colors.borrow().contains_key(&session_id);
        let is_detached = self.detached.borrow().contains(&session_id);
        detach_context_from(
            renders_in_process,
            SplitMembership::from_marks(has_split_color, is_parked_in_split),
            is_detached,
        )
    }

    /// Fires the detach-request callback, if one is installed.
    pub(super) fn notify_detach_request(
        &self,
        session_id: Uuid,
        presentation: DetachPresentation,
    ) -> bool {
        let callback = self.on_detach_request.borrow_mut().take();
        let Some(callback) = callback else {
            tracing::debug!(
                session = %session_id,
                "detach request dropped: no handler installed"
            );
            return false;
        };
        tracing::debug!(session = %session_id, ?presentation, "detach requested");
        let result = callback(session_id, presentation);
        *self.on_detach_request.borrow_mut() = Some(callback);
        result
    }
}

impl TerminalNotebook {
    /// Reports whether a session may be moved into a detached window.
    ///
    /// A session without metadata is treated as an external viewer, which is
    /// the same "not detachable" outcome the Welcome tab gets from its caller.
    #[must_use]
    pub fn detach_verdict(&self, session_id: Uuid) -> DetachVerdict {
        self.detach_hooks().verdict(session_id)
    }

    /// Bundles the handles the tab context menu needs to offer a detach.
    pub(super) fn detach_hooks(&self) -> DetachHooks {
        DetachHooks {
            session_info: Rc::clone(&self.session_info),
            parked_in_split: Rc::clone(&self.parked_in_split),
            split_session_colors: Rc::clone(&self.split_session_colors),
            detached: Rc::clone(&self.detached),
            on_detach_request: Rc::clone(&self.on_detach_request),
        }
    }

    /// Reports whether a session's widget currently lives in a detached window.
    #[must_use]
    pub fn is_detached(&self, session_id: Uuid) -> bool {
        self.detached.borrow().contains(&session_id)
    }

    /// Returns the ids of every session that currently has a detached window.
    ///
    /// Sorted by session id: the park set is a `HashSet`, and its iteration
    /// order changes from run to run, which would reorder the session-manager
    /// rows and the saved workspace entries that chain these ids in.
    #[must_use]
    pub fn detached_session_ids(&self) -> Vec<Uuid> {
        let mut ids: Vec<Uuid> = self.detached.borrow().iter().copied().collect();
        ids.sort_unstable();
        ids
    }

    /// Reports whether a session may be moved into another tab's split layout.
    ///
    /// Answered by [`rustconn_core::may_place_in_split`], so the two split
    /// pickers and the callback that commits their choice cannot disagree about
    /// a detached session (issue #236).
    #[must_use]
    pub fn may_place_in_split(&self, session_id: Uuid) -> bool {
        rustconn_core::may_place_in_split(&self.detach_hooks().context(session_id))
    }

    /// Asks the window layer to move a session into a detached window.
    ///
    /// Returns `true` only when the exact session is detached and registered.
    pub fn request_detach(&self, session_id: Uuid, presentation: DetachPresentation) -> bool {
        self.detach_hooks()
            .notify_detach_request(session_id, presentation)
    }

    /// Returns how many sessions currently live in a detached window.
    ///
    /// [`Self::session_count`] stays "sessions with a tab", so a caller that
    /// needs the true number of live sessions uses
    /// `session_count() + detached_count()`.
    #[must_use]
    pub fn detached_count(&self) -> usize {
        self.detached.borrow().len()
    }

    /// Sets the callback invoked when a detached session is asked to take focus.
    ///
    /// The window layer presents that session's detached window instead of
    /// selecting a tab.
    pub fn set_on_focus_detached<F>(&self, callback: F)
    where
        F: Fn(Uuid) + 'static,
    {
        *self.on_focus_detached.borrow_mut() = Some(Box::new(callback));
    }

    /// Sets the callback invoked when the tab context menu requests a detach.
    pub fn set_on_detach_request<F>(&self, callback: F)
    where
        F: Fn(Uuid, DetachPresentation) -> bool + 'static,
    {
        *self.on_detach_request.borrow_mut() = Some(Box::new(callback));
    }

    /// Sets the callback invoked after a session's teardown has run.
    ///
    /// Fires for every session that ends, whatever ended it, so the window
    /// layer can close a detached window whose session disappeared (remote
    /// disconnect, child exit, terminate from the session manager) instead of
    /// leaving an empty window behind. A session moving between a tab and a
    /// detached window never reaches it: parking skips teardown.
    pub fn set_on_session_ended<F>(&self, callback: F)
    where
        F: Fn(Uuid) + 'static,
    {
        *self.on_session_ended.borrow_mut() = Some(Box::new(callback));
    }

    /// Fires the focus-detached callback, if one is installed.
    pub(super) fn notify_focus_detached(&self, session_id: Uuid) {
        if let Some(ref callback) = *self.on_focus_detached.borrow() {
            callback(session_id);
        }
    }

    /// Hands a session's live content over to a detached window.
    ///
    /// Checks the verdict, marks the session detached so the `close-page`
    /// handler skips teardown, drops its tab page, and rebuilds the content box
    /// around the same live widget. The call is atomic from the caller's point
    /// of view: either it returns a content box holding the live widget with the
    /// session marked detached, or it returns `None` and leaves the session with
    /// its tab.
    #[must_use]
    pub fn take_session_content(&self, session_id: Uuid) -> Option<GtkBox> {
        let verdict = self.detach_verdict(session_id);
        if !verdict.is_allowed() {
            tracing::warn!(
                session = %session_id,
                reason = verdict.reason_key(),
                "detach declined"
            );
            return None;
        }
        if !self.has_live_widget(session_id) {
            tracing::warn!(session = %session_id, "detach declined: no live widget");
            return None;
        }

        // The monitoring bar is a child of the content box about to be rebuilt,
        // so suspend it first and resume it into the new box — exactly what the
        // split path does around its move.
        let monitoring = self.monitoring.borrow().as_ref().map(Rc::clone);
        if let Some(ref coordinator) = monitoring {
            coordinator.suspend_monitoring(session_id);
        }

        self.detached.borrow_mut().insert(session_id);
        if !self.park_tab_page(session_id) {
            self.detached.borrow_mut().remove(&session_id);
            self.resume_monitoring_in_place(monitoring.as_ref(), session_id);
            tracing::warn!(session = %session_id, "detach failed: session has no tab page");
            return None;
        }

        let Some(content) = self.build_session_content(session_id) else {
            // The widget vanished between the guard and the move: give the
            // session its tab back (which also clears the detached mark) so the
            // caller sees no state change.
            let _ = self.restore_session_tab(session_id);
            self.resume_monitoring_in_place(monitoring.as_ref(), session_id);
            tracing::warn!(session = %session_id, "detach failed: could not build content");
            return None;
        };

        if let Some(ref coordinator) = monitoring {
            coordinator.resume_monitoring(session_id, &content);
        }
        tracing::info!(session = %session_id, "session content handed to detached window");
        Some(content)
    }

    /// Moves a detached session's content back into a tab of the main window.
    ///
    /// Rebuilds the content box around the same live widget, re-derives the
    /// tab's group and protocol color, selects the restored page, and only then
    /// drops the Welcome placeholder (so the removal fires after the real page
    /// is selected — see `remove_welcome_page`). Returns `false` when the
    /// session is not detached or its tab could not be rebuilt, in which case
    /// the caller keeps its window open and the Welcome tab is left untouched.
    pub fn attach_session(&self, session_id: Uuid) -> bool {
        if !self.is_detached(session_id) {
            tracing::warn!(session = %session_id, "attach declined: session is not detached");
            return false;
        }
        if !self.has_live_widget(session_id) {
            tracing::warn!(session = %session_id, "attach declined: no live widget");
            return false;
        }

        let monitoring = self.monitoring.borrow().as_ref().map(Rc::clone);
        if let Some(ref coordinator) = monitoring {
            coordinator.suspend_monitoring(session_id);
        }

        if !self.restore_session_tab(session_id) {
            self.resume_monitoring_in_place(monitoring.as_ref(), session_id);
            self.ensure_welcome_page();
            tracing::warn!(session = %session_id, "attach failed: could not recreate tab");
            return false;
        }

        let Some(content) = self.build_session_content(session_id) else {
            // The tab exists but holds nothing: roll all the way back to
            // "detached", so the session is never reachable as both a tab and a
            // window (Requirement 2.7).
            self.detached.borrow_mut().insert(session_id);
            if !self.park_tab_page(session_id) {
                tracing::error!(
                    session = %session_id,
                    "attach rollback could not drop the empty tab"
                );
            }
            self.resume_monitoring_in_place(monitoring.as_ref(), session_id);
            self.ensure_welcome_page();
            tracing::warn!(session = %session_id, "attach failed: could not build content");
            return false;
        };
        self.switch_tab_to_single(session_id, &content);

        // Re-derive the tab indicators from the surviving session metadata.
        if let Some(info) = self.get_session_info(session_id) {
            if let Some(color_index) = info.tab_color_index {
                self.apply_group_color(session_id, color_index);
            }
            if *self.color_tabs_by_protocol.borrow() {
                self.apply_protocol_color(session_id, &info.protocol);
            }
        }

        self.switch_to_tab(session_id);
        // Drop the Welcome placeholder only once the restored tab exists and is
        // selected (see `remove_welcome_page`): if restore had failed above, the
        // early returns leave Welcome untouched so the empty window keeps it.
        self.remove_welcome_page();
        if let Some(ref coordinator) = monitoring {
            coordinator.resume_monitoring(session_id, &content);
        }

        // The re-parented widget lands in a fresh allocation; nudge a repaint
        // once GTK has settled it (embedded viewers keep their frames in a
        // Rust-side buffer, so nothing else triggers the draw).
        glib::idle_add_local_once(move || {
            content.queue_draw();
        });

        tracing::info!(session = %session_id, "session attached back to main window");
        true
    }

    /// Reports whether a session still owns a live widget that can be moved.
    fn has_live_widget(&self, session_id: Uuid) -> bool {
        self.terminals.borrow().contains_key(&session_id)
            || self.session_widgets.borrow().contains_key(&session_id)
    }

    /// Resumes monitoring into wherever a session sits after a failed move.
    ///
    /// Resolves the content box through [`TerminalNotebook::session_content_box`],
    /// so it works for a session rolled back into its tab and for one rolled
    /// back into its detached window.
    fn resume_monitoring_in_place(
        &self,
        monitoring: Option<&Rc<MonitoringCoordinator>>,
        session_id: Uuid,
    ) {
        if let Some(coordinator) = monitoring
            && let Some(container) = self.session_content_box(session_id)
        {
            coordinator.resume_monitoring(session_id, &container);
        }
    }
}

#[cfg(test)]
mod detach_context_tests {
    use super::{SplitMembership, detach_context_from};

    #[test]
    fn colored_but_unparked_session_owns_its_split() {
        assert_eq!(
            SplitMembership::from_marks(true, false),
            SplitMembership::Owner
        );
        let ctx = detach_context_from(true, SplitMembership::Owner, false);
        assert!(ctx.is_split_owner);
        assert!(!ctx.is_split_guest);
    }

    #[test]
    fn parked_session_is_a_guest_not_an_owner() {
        assert_eq!(
            SplitMembership::from_marks(true, true),
            SplitMembership::Guest
        );
        let ctx = detach_context_from(true, SplitMembership::Guest, false);
        assert!(!ctx.is_split_owner);
        assert!(ctx.is_split_guest);
    }

    #[test]
    fn unmarked_session_is_outside_every_split() {
        assert_eq!(
            SplitMembership::from_marks(false, false),
            SplitMembership::Outside
        );
        let ctx = detach_context_from(true, SplitMembership::Outside, false);
        assert!(!ctx.is_split_owner);
        assert!(!ctx.is_split_guest);
        assert!(ctx.renders_in_process);
        assert!(!ctx.is_detached);
    }

    #[test]
    fn remaining_flags_are_carried_through_verbatim() {
        let ctx = detach_context_from(false, SplitMembership::Outside, true);
        assert!(!ctx.renders_in_process);
        assert!(ctx.is_detached);
    }
}

#[cfg(test)]
mod detach_request_tests {
    use super::*;

    fn hooks_without_handler() -> DetachHooks {
        DetachHooks {
            session_info: Rc::new(RefCell::new(HashMap::new())),
            parked_in_split: Rc::new(RefCell::new(HashSet::new())),
            split_session_colors: Rc::new(RefCell::new(HashMap::new())),
            detached: Rc::new(RefCell::new(HashSet::new())),
            on_detach_request: Rc::new(RefCell::new(None)),
        }
    }

    #[test]
    fn request_without_handler_returns_false() {
        let hooks = hooks_without_handler();

        assert!(!hooks.notify_detach_request(Uuid::from_u128(1), DetachPresentation::default(),));
    }

    #[test]
    fn request_returns_the_callback_result() {
        let hooks = hooks_without_handler();
        *hooks.on_detach_request.borrow_mut() = Some(Box::new(|_, _| false));
        assert!(!hooks.notify_detach_request(Uuid::from_u128(1), DetachPresentation::default(),));

        *hooks.on_detach_request.borrow_mut() = Some(Box::new(|_, _| true));
        assert!(hooks.notify_detach_request(Uuid::from_u128(1), DetachPresentation::default(),));
    }

    #[test]
    fn request_forwards_session_and_presentation() {
        let hooks = hooks_without_handler();
        let observed = Rc::new(RefCell::new(None));
        let observed_for_callback = Rc::clone(&observed);
        *hooks.on_detach_request.borrow_mut() = Some(Box::new(move |session_id, presentation| {
            *observed_for_callback.borrow_mut() = Some((session_id, presentation));
            true
        }));
        let session_id = Uuid::from_u128(0xD37A_C4ED);
        let presentation = DetachPresentation {
            fullscreen: true,
            monitor: Some(DetachMonitor {
                index: 2,
                connector: Some("DP-1".to_string()),
                manufacturer: Some("Acme".to_string()),
                model: Some("Panel".to_string()),
            }),
        };
        let expected = presentation.clone();

        assert!(hooks.notify_detach_request(session_id, presentation));
        assert_eq!(observed.borrow().as_ref(), Some(&(session_id, expected)));
    }

    #[test]
    fn monitor_connector_survives_index_reordering() {
        let preference = DetachMonitor {
            index: 2,
            connector: Some("DP-1".to_string()),
            manufacturer: Some("Acme".to_string()),
            model: Some("Panel".to_string()),
        };

        assert!(preference.matches_identity(Some("DP-1"), Some("Other"), Some("Display")));
        assert!(!preference.matches_identity(Some("HDMI-1"), Some("Acme"), Some("Panel")));
    }

    #[test]
    fn monitor_without_identity_uses_index_fallback() {
        let preference = DetachMonitor {
            index: 2,
            connector: None,
            manufacturer: None,
            model: None,
        };

        assert!(!preference.has_identity());
        assert!(!preference.matches_identity(None, None, None));
    }
}

#[cfg(test)]
mod park_mark_tests {
    use std::collections::HashSet;

    use uuid::Uuid;

    use super::{ParkMark, take_park_mark};

    /// Two fixed ids keep the assertions readable.
    fn ids() -> (Uuid, Uuid) {
        (Uuid::from_u128(1), Uuid::from_u128(2))
    }

    #[test]
    fn detach_mark_is_cleared_and_reported() {
        let (id, other) = ids();
        let mut in_split = HashSet::new();
        let mut detached = HashSet::from([id, other]);

        assert_eq!(
            take_park_mark(&mut in_split, &mut detached, id),
            Some(ParkMark::Detached)
        );
        assert!(!detached.contains(&id), "the detach mark must be gone");
        assert!(detached.contains(&other), "other sessions stay marked");
        assert!(in_split.is_empty());
    }

    #[test]
    fn split_mark_is_cleared_and_reported() {
        let (id, _) = ids();
        let mut in_split = HashSet::from([id]);
        let mut detached = HashSet::new();

        assert_eq!(
            take_park_mark(&mut in_split, &mut detached, id),
            Some(ParkMark::InSplit)
        );
        assert!(in_split.is_empty());
        assert!(detached.is_empty());
    }

    #[test]
    fn both_sets_are_cleared_when_both_marks_are_set() {
        // Should not happen in practice — a session is parked for one reason —
        // but un-parking must not leave a stale mark behind if it ever does.
        let (id, _) = ids();
        let mut in_split = HashSet::from([id]);
        let mut detached = HashSet::from([id]);

        assert_eq!(
            take_park_mark(&mut in_split, &mut detached, id),
            Some(ParkMark::InSplit)
        );
        assert!(in_split.is_empty());
        assert!(detached.is_empty(), "the detach mark must be cleared too");
    }

    #[test]
    fn unparked_session_reports_no_mark() {
        let (id, other) = ids();
        let mut in_split = HashSet::from([other]);
        let mut detached = HashSet::from([other]);

        assert_eq!(take_park_mark(&mut in_split, &mut detached, id), None);
        assert!(in_split.contains(&other), "unrelated marks are untouched");
        assert!(detached.contains(&other));
    }
}

#[cfg(test)]
mod placement_invariant_tests {
    use std::collections::HashSet;

    use proptest::prelude::*;
    use uuid::Uuid;

    use super::take_park_mark;
    use crate::window::open_session_count;

    /// A GTK-free mirror of the notebook's session placement bookkeeping.
    ///
    /// Tracks the same three collections the notebook keeps — the tab map
    /// (`sessions`), the split park set and the detach park set — so the
    /// counting invariants can be checked without a display. Un-parking goes
    /// through the production [`take_park_mark`], and the total is computed with
    /// the production [`open_session_count`] the close confirmation uses.
    #[derive(Default)]
    struct PlacementModel {
        live: HashSet<Uuid>,
        tabbed: HashSet<Uuid>,
        parked_in_split: HashSet<Uuid>,
        detached: HashSet<Uuid>,
    }

    impl PlacementModel {
        /// Opens a session, which starts life with a tab of its own.
        fn open(&mut self, id: Uuid) {
            if self.live.insert(id) {
                self.tabbed.insert(id);
            }
        }

        /// Moves a tabbed session into a detached window (`take_session_content`).
        fn detach(&mut self, id: Uuid) {
            if self.tabbed.remove(&id) {
                self.detached.insert(id);
            }
        }

        /// Moves a session into another tab's split (`park_session_tab`).
        ///
        /// The refusal is not the model's own invention: the placement question
        /// goes to the production predicate with the same facts the notebook
        /// derives, so a detached session is rejected here for exactly the
        /// reason the split pickers reject it (issue #236).
        fn park_in_split(&mut self, id: Uuid) {
            if !rustconn_core::may_place_in_split(&self.context(id)) {
                return;
            }
            if self.tabbed.remove(&id) {
                self.parked_in_split.insert(id);
            }
        }

        /// Builds the placement context of a session from the model's sets.
        ///
        /// Every session in this model is an in-process one that owns no split
        /// layout, which is the shape the split picker offers.
        fn context(&self, id: Uuid) -> rustconn_core::DetachContext {
            rustconn_core::DetachContext {
                renders_in_process: true,
                is_split_owner: false,
                is_split_guest: self.parked_in_split.contains(&id),
                is_detached: self.detached.contains(&id),
            }
        }

        /// Gives a parked session its tab back (`restore_session_tab`).
        fn restore(&mut self, id: Uuid) {
            if take_park_mark(&mut self.parked_in_split, &mut self.detached, id).is_some() {
                self.tabbed.insert(id);
            }
        }

        /// Ends a session, wherever it currently lives (`close_session`).
        fn close(&mut self, id: Uuid) {
            take_park_mark(&mut self.parked_in_split, &mut self.detached, id);
            self.tabbed.remove(&id);
            self.live.remove(&id);
        }

        /// Mirrors `TerminalNotebook::session_count` — sessions with a tab.
        fn session_count(&self) -> usize {
            self.tabbed.len()
        }

        /// Mirrors `TerminalNotebook::detached_count`.
        fn detached_count(&self) -> usize {
            self.detached.len()
        }

        /// Asserts the placement invariants after every transition.
        ///
        /// A session sits in exactly one place, and `session_count()` plus
        /// `detached_count()` covers every live session that owns a tab or a
        /// window — the split guests are the deliberate remainder, since their
        /// widget lives inside another session's tab.
        fn assert_invariants(&self) {
            assert!(
                self.tabbed.is_disjoint(&self.detached),
                "a session cannot have a tab and a detached window"
            );
            assert!(
                self.tabbed.is_disjoint(&self.parked_in_split),
                "a parked session cannot also have a tab"
            );
            assert!(
                self.parked_in_split.is_disjoint(&self.detached),
                "a session cannot be parked in a split and detached at once"
            );

            let placed: HashSet<Uuid> = self
                .tabbed
                .union(&self.detached)
                .chain(self.parked_in_split.iter())
                .copied()
                .collect();
            assert_eq!(placed, self.live, "every live session sits somewhere");

            assert_eq!(
                open_session_count(self.session_count(), self.detached_count(), 0),
                self.live.len() - self.parked_in_split.len(),
                "counts must cover every live session outside a split"
            );
        }
    }

    #[test]
    fn counts_cover_every_live_session() {
        let mut model = PlacementModel::default();
        let (a, b, c) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        for id in [a, b, c] {
            model.open(id);
        }
        model.assert_invariants();
        assert_eq!(model.session_count() + model.detached_count(), 3);

        model.detach(b);
        model.assert_invariants();
        assert_eq!(model.session_count(), 2);
        assert_eq!(model.detached_count(), 1);
        assert_eq!(model.session_count() + model.detached_count(), 3);

        model.restore(b);
        model.assert_invariants();
        assert_eq!(model.detached_count(), 0);
        assert_eq!(model.session_count(), 3);
    }

    #[test]
    fn a_split_guest_is_the_only_live_session_outside_the_counts() {
        let mut model = PlacementModel::default();
        let (owner, guest) = (Uuid::from_u128(1), Uuid::from_u128(2));
        model.open(owner);
        model.open(guest);
        model.park_in_split(guest);
        model.assert_invariants();

        // The guest's widget lives in the owner's tab, so one tab covers both —
        // the pre-existing split behaviour detaching must not change.
        assert_eq!(model.session_count() + model.detached_count(), 1);

        model.detach(owner);
        model.assert_invariants();
        assert_eq!(model.session_count(), 0);
        assert_eq!(model.detached_count(), 1);
    }

    #[test]
    fn the_split_picker_refuses_a_detached_session() {
        let mut model = PlacementModel::default();
        let id = Uuid::from_u128(1);
        model.open(id);
        model.detach(id);

        // The transition both split "Select Tab" providers now filter out, and
        // the Select Tab callback refuses before it reparents anything.
        model.park_in_split(id);
        model.assert_invariants();

        assert!(
            model.detached.contains(&id),
            "the session must stay in its window"
        );
        assert!(
            model.parked_in_split.is_empty(),
            "a detached session must never gain a split mark"
        );
        assert_eq!(model.detached_count(), 1);

        // Attaching it first makes the same move legal again.
        model.restore(id);
        model.park_in_split(id);
        model.assert_invariants();
        assert!(model.parked_in_split.contains(&id));
    }

    /// One placement transition, addressed by index into a small id pool.
    #[derive(Debug, Clone, Copy)]
    enum Op {
        Open(usize),
        Detach(usize),
        ParkInSplit(usize),
        Restore(usize),
        Close(usize),
    }

    fn op_strategy() -> impl Strategy<Value = Op> {
        // Four ids are enough to exercise every overlap while keeping the
        // shrunk counterexamples readable.
        let slot = 0_usize..4;
        prop_oneof![
            slot.clone().prop_map(Op::Open),
            slot.clone().prop_map(Op::Detach),
            slot.clone().prop_map(Op::ParkInSplit),
            slot.clone().prop_map(Op::Restore),
            slot.prop_map(Op::Close),
        ]
    }

    proptest! {
        /// Every sequence of moves keeps the placement sets disjoint and the
        /// session counts complete.
        #[test]
        fn placement_invariants_hold_across_arbitrary_move_sequences(
            ops in proptest::collection::vec(op_strategy(), 0..40)
        ) {
            let mut model = PlacementModel::default();
            let pool: Vec<Uuid> = (1..=4).map(Uuid::from_u128).collect();
            for op in ops {
                match op {
                    Op::Open(i) => model.open(pool[i]),
                    Op::Detach(i) => model.detach(pool[i]),
                    Op::ParkInSplit(i) => model.park_in_split(pool[i]),
                    Op::Restore(i) => model.restore(pool[i]),
                    Op::Close(i) => model.close(pool[i]),
                }
                model.assert_invariants();
            }
        }
    }
}

#[cfg(test)]
mod notebook_park_tests {
    use super::*;

    /// Session metadata is a plain struct, so it needs no GTK to build.
    fn stub_session(id: Uuid) -> TerminalSession {
        TerminalSession {
            id,
            connection_id: Uuid::from_u128(0x00C0_FFEE),
            name: "stub".to_owned(),
            protocol: "ssh".to_owned(),
            is_embedded: true,
            host: None,
            log_file: None,
            history_entry_id: None,
            tab_group: None,
            tab_color_index: None,
            connected_at: chrono::Utc::now(),
            reconnected_at: None,
        }
    }

    #[test]
    // GTK can only be initialized from one thread per process; the default
    // multi-threaded harness makes a widget-constructing test unsafe, so this
    // one is opt-in — same convention as the split eligibility tests. Both park
    // sets are checked in one test because a single process must build only one
    // notebook: a second `adw::TabView` in the same run crashes GTK.
    #[ignore = "initialises GTK: needs a display and its own process; run alone with `cargo test -p rustconn --bin rustconn -- --ignored --exact <this test path>`"]
    fn restore_session_tab_clears_the_park_set_the_session_was_in() {
        if gtk4::init().is_err() {
            return;
        }
        let notebook = TerminalNotebook::new(false);
        let detached_id = Uuid::from_u128(1);
        let split_id = Uuid::from_u128(2);
        {
            let mut info = notebook.session_info.borrow_mut();
            info.insert(detached_id, stub_session(detached_id));
            info.insert(split_id, stub_session(split_id));
        }
        notebook.detached.borrow_mut().insert(detached_id);
        notebook.parked_in_split.borrow_mut().insert(split_id);

        // Neither parked session has a tab, so the tab count alone reports none
        // of them; only the detached one is covered by `detached_count`.
        assert_eq!(notebook.session_count(), 0);
        assert_eq!(notebook.detached_count(), 1);

        notebook.restore_session_tab(detached_id);

        assert!(
            !notebook.is_detached(detached_id),
            "the detach mark must be cleared"
        );
        assert!(
            notebook.parked_in_split.borrow().contains(&split_id),
            "the other session's split mark must be untouched"
        );
        assert_eq!(notebook.detached_count(), 0);
        assert_eq!(notebook.session_count(), 1, "the tab is back");
        assert_eq!(
            notebook.session_count() + notebook.detached_count(),
            1,
            "the restored session is counted exactly once"
        );

        notebook.restore_session_tab(split_id);

        assert!(
            !notebook.parked_in_split.borrow().contains(&split_id),
            "the split mark must be cleared"
        );
        assert!(
            !notebook.is_detached(split_id),
            "un-parking a split guest must not mark it detached"
        );
        assert_eq!(notebook.session_count(), 2);
        assert_eq!(notebook.detached_count(), 0);
        assert_eq!(
            notebook.session_count() + notebook.detached_count(),
            2,
            "both live sessions are counted exactly once"
        );
    }
}
