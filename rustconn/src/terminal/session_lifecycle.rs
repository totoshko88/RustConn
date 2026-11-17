//! Session reconnect, disconnect, and status management.
//!
//! Extracted from `terminal/mod.rs` to reduce module complexity.
//! Contains methods for handling session disconnect/reconnect lifecycle,
//! reconnect banners, VTE reset, and connection status tracking.

use super::*;

/// Name of the key controller that gives R, Q and Enter a meaning on an ended
/// session.
const ENDED_SESSION_KEYS: &str = "ended-session-keys";

/// Widget name of the reconnect banner's key hint label.
const KEYS_HINT: &str = "reconnect-keys-hint";

/// Widget name of the banner's "Log In to …" button (see `offer_cloud_login`).
const CLOUD_LOGIN_BUTTON: &str = "cloud-login";

/// Sets the banner's key hint, with the same text as its tooltip: the label
/// ellipsizes in a narrow pane, and the tooltip is where the rest is read.
fn set_keys_hint(label: &gtk4::Label, text: &str) {
    label.set_label(text);
    label.set_tooltip_text(Some(text));
}

/// A key pressed on an ended session, as far as the banner cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndedSessionKey {
    /// R: reconnect.
    Reconnect,
    /// Q: close the session.
    Close,
    /// Return, keypad Enter.
    Enter,
    /// A modifier on its own (Shift before R, say): neither an action nor
    /// "another key".
    Modifier,
    /// Anything else.
    Other,
}

impl EndedSessionKey {
    /// Classifies a key, given its Latin keyval (see [`crate::utils::latin_keyval`]).
    fn from_keyval(keyval: gtk4::gdk::Key) -> Self {
        use gtk4::gdk::Key;
        match keyval.to_lower() {
            Key::r => Self::Reconnect,
            Key::q => Self::Close,
            Key::Return | Key::KP_Enter | Key::ISO_Enter => Self::Enter,
            Key::Shift_L
            | Key::Shift_R
            | Key::Control_L
            | Key::Control_R
            | Key::Alt_L
            | Key::Alt_R
            | Key::Super_L
            | Key::Super_R
            | Key::Meta_L
            | Key::Meta_R
            | Key::Hyper_L
            | Key::Hyper_R
            | Key::ISO_Level3_Shift
            | Key::Caps_Lock
            | Key::Num_Lock => Self::Modifier,
            _ => Self::Other,
        }
    }
}

/// Whether a modifier that makes a key a shortcut is held (Ctrl, Alt, Super,
/// Meta). Shift is not one: Shift+R is still R.
fn has_shortcut_modifier(modifiers: gtk4::gdk::ModifierType) -> bool {
    use gtk4::gdk::ModifierType;
    modifiers.intersects(
        ModifierType::CONTROL_MASK
            | ModifierType::ALT_MASK
            | ModifierType::SUPER_MASK
            | ModifierType::META_MASK,
    )
}

/// Where the banner's two-step Enter stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnterState {
    /// No Enter pressed yet, or the last one was cancelled.
    Idle,
    /// The first Enter is down and not yet released; its key repeat must not
    /// count as the second press.
    Held,
    /// The first Enter was pressed and released: the next Enter runs the
    /// banner's suggested action.
    Armed,
}

/// What the ended-session key controller does with a key press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndedSessionKeyAction {
    /// Not ours: let the terminal have the key.
    Pass,
    /// Another key after a first Enter: drop the highlight, let the key through.
    Disarm,
    /// Reconnect, as the Reconnect button does.
    Reconnect,
    /// Close the session where it is shown.
    Close,
    /// First Enter: highlight the suggested button and say what Enter does next.
    Arm,
    /// The first Enter's key repeat: swallow it, change nothing.
    Hold,
    /// Second Enter: click the suggested button.
    RunSuggested,
}

impl EndedSessionKeyAction {
    /// Whether the key goes on to the terminal.
    fn propagation(self) -> glib::Propagation {
        match self {
            Self::Pass | Self::Disarm => glib::Propagation::Proceed,
            Self::Reconnect | Self::Close | Self::Arm | Self::Hold | Self::RunSuggested => {
                glib::Propagation::Stop
            }
        }
    }

    /// The Enter state after this action, or `None` to keep it.
    fn next_enter_state(self) -> Option<EnterState> {
        match self {
            Self::Pass | Self::Hold => None,
            Self::Arm => Some(EnterState::Held),
            Self::Disarm | Self::Reconnect | Self::Close | Self::RunSuggested => {
                Some(EnterState::Idle)
            }
        }
    }
}

/// Decides what a key press on an ended session does.
///
/// `banner_shown` is whether the session is disconnected and its reconnect
/// banner is up; the keys mean nothing otherwise, so a live session never
/// loses a keystroke. Kept free of GTK so it can be tested without a display.
fn ended_session_key_action(
    key: EndedSessionKey,
    shortcut_modifier: bool,
    banner_shown: bool,
    enter: EnterState,
) -> EndedSessionKeyAction {
    use EndedSessionKeyAction as Action;
    let pending = enter != EnterState::Idle;
    if !banner_shown || key == EndedSessionKey::Modifier {
        return Action::Pass;
    }
    if shortcut_modifier {
        return if pending {
            Action::Disarm
        } else {
            Action::Pass
        };
    }
    match key {
        EndedSessionKey::Reconnect => Action::Reconnect,
        EndedSessionKey::Close => Action::Close,
        EndedSessionKey::Enter => match enter {
            EnterState::Idle => Action::Arm,
            EnterState::Held => Action::Hold,
            EnterState::Armed => Action::RunSuggested,
        },
        EndedSessionKey::Modifier | EndedSessionKey::Other => {
            if pending {
                Action::Disarm
            } else {
                Action::Pass
            }
        }
    }
}

/// Whether R, Q, Enter and the terminal menu's Reconnect act on a session.
///
/// They do only while the session is `disconnected` and `marked` as having a
/// reconnect banner. The mark is cleared while a cloud login runs in the
/// terminal, so `aws login` gets every key and no Reconnect entry.
pub(super) const fn ended_session_actions_available(marked: bool, disconnected: bool) -> bool {
    marked && disconnected
}

/// What to do with a session's reconnect banner when its terminal is mapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BannerReconcile {
    /// Nothing to do.
    Keep,
    /// The banner was dropped with the box around the terminal: show it again.
    Reshow,
    /// The session is live again: the mark is stale.
    ForgetMark,
}

/// Decides [`BannerReconcile`] from whether the session is `marked` as having a
/// banner, is `disconnected`, and has a banner `present` in its own box.
fn reconnect_banner_reconcile(marked: bool, disconnected: bool, present: bool) -> BannerReconcile {
    match (marked, disconnected, present) {
        (true, true, false) => BannerReconcile::Reshow,
        (true, false, _) => BannerReconcile::ForgetMark,
        _ => BannerReconcile::Keep,
    }
}

/// Carries out the ended-session key actions for one session's banner.
///
/// Holds everything weakly or through the notebook's shared cells, so the
/// controller that owns it keeps neither the terminal nor the banner alive.
#[derive(Clone)]
struct EndedSessionKeyActions {
    session_id: Uuid,
    connection_id: Uuid,
    on_reconnect: Rc<RefCell<Option<Box<dyn Fn(Uuid, Uuid)>>>>,
    reconnect_shown: Rc<RefCell<std::collections::HashSet<Uuid>>>,
    split_pane_focuser: Rc<RefCell<Option<Rc<dyn Fn(Uuid) -> bool>>>>,
    terminal: glib::WeakRef<Terminal>,
    banner: glib::WeakRef<GtkBox>,
}

impl EndedSessionKeyActions {
    fn run(&self, action: EndedSessionKeyAction) {
        use EndedSessionKeyAction as Action;
        match action {
            Action::Pass | Action::Hold => {}
            Action::Disarm => self.show_armed(false),
            Action::Arm => self.show_armed(true),
            // Reconnecting or closing resets the terminal or destroys it, and
            // removes this very controller; done on idle, outside the key
            // event's dispatch. The banner may be gone by then (a click on its
            // button, a second queued key), which the guard checks.
            Action::Reconnect | Action::Close | Action::RunSuggested => {
                if action == Action::RunSuggested {
                    self.show_armed(false);
                }
                let this = self.clone();
                glib::idle_add_local_once(move || {
                    if !this.reconnect_shown.borrow().contains(&this.session_id) {
                        return;
                    }
                    match action {
                        Action::Reconnect => this.reconnect(),
                        Action::Close => this.close(),
                        _ => this.run_suggested(),
                    }
                });
            }
        }
    }

    fn reconnect(&self) {
        tracing::info!(session_id = %self.session_id, "ended session: reconnect requested with R");
        if let Some(ref callback) = *self.on_reconnect.borrow() {
            callback(self.session_id, self.connection_id);
        }
    }

    /// Closes the session through the close path of the place it is shown in.
    ///
    /// A pane of a split has no tab of its own: `win.close-pane` ends it and
    /// tidies the layout, but closes the split's *focused* pane, so that pane
    /// is made the focused one first. Everywhere else `win.close-tab` resolves
    /// to the action of the window the terminal sits in: the main window's
    /// closes its selected tab, which is the one holding the focused terminal;
    /// a detached window's ends its own session; a cluster window's closes its
    /// selected tab, again the one holding the focused terminal.
    fn close(&self) {
        let focuser = self.split_pane_focuser.borrow().as_ref().map(Rc::clone);
        let in_split = focuser.is_some_and(|focus| focus(self.session_id));
        let action = if in_split {
            "win.close-pane"
        } else {
            "win.close-tab"
        };
        tracing::info!(session_id = %self.session_id, action, "ended session: close requested with Q");
        let Some(terminal) = self.terminal.upgrade() else {
            return;
        };
        if let Err(error) = terminal.activate_action(action, None) {
            tracing::warn!(session_id = %self.session_id, action, %error, "could not close the ended session");
        }
    }

    /// Clicks the banner's suggested button: what the banner itself offers as
    /// the next step, so Enter and a click can never disagree.
    fn run_suggested(&self) {
        if let Some(button) = self.banner.upgrade().as_ref().and_then(suggested_button) {
            tracing::info!(
                session_id = %self.session_id,
                cloud_login = button.widget_name() == CLOUD_LOGIN_BUTTON,
                "ended session: suggested action requested with Enter"
            );
            button.emit_clicked();
        } else {
            self.reconnect();
        }
    }

    /// Highlights the suggested button and changes the hint for the second
    /// Enter, or puts both back.
    fn show_armed(&self, armed: bool) {
        let Some(banner) = self.banner.upgrade() else {
            return;
        };
        let button = suggested_button(&banner);
        let hint = banner_child_named(&banner, KEYS_HINT).and_downcast::<gtk4::Label>();
        if armed {
            if let Some(ref button) = button {
                button.set_state_flags(gtk4::StateFlags::PRELIGHT, false);
            }
            let text = if button
                .as_ref()
                .is_some_and(|b| b.widget_name() == CLOUD_LOGIN_BUTTON)
            {
                i18n("Press Enter again to log in")
            } else {
                i18n("Press Enter again to reconnect")
            };
            if let Some(hint) = hint {
                set_keys_hint(&hint, &text);
                // Focus stays in the terminal, so the change is said out loud.
                hint.announce(&text, gtk4::AccessibleAnnouncementPriority::Medium);
            }
        } else {
            if let Some(ref button) = button {
                button.unset_state_flags(gtk4::StateFlags::PRELIGHT);
            }
            if let Some(hint) = hint {
                set_keys_hint(&hint, &i18n("Press R to reconnect, Q to close"));
            }
        }
    }
}

/// The banner's direct child with the given widget name.
fn banner_child_named(banner: &GtkBox, name: &str) -> Option<Widget> {
    let mut child = banner.first_child();
    while let Some(widget) = child {
        if widget.widget_name() == name {
            return Some(widget);
        }
        child = widget.next_sibling();
    }
    None
}

/// The banner's suggested-action button: Reconnect, or "Log In to …" once
/// [`TerminalNotebook::offer_cloud_login`] has moved the style there.
fn suggested_button(banner: &GtkBox) -> Option<gtk4::Button> {
    let mut child = banner.first_child();
    while let Some(widget) = child {
        if widget.has_css_class("suggested-action")
            && let Ok(button) = widget.clone().downcast::<gtk4::Button>()
        {
            return Some(button);
        }
        child = widget.next_sibling();
    }
    None
}

/// Decides whether a disconnected session still has a widget home to reconnect
/// into, given the three placements it can occupy one at a time.
///
/// Pure and GTK-free so the branching is unit-testable; the live signals are
/// gathered by [`TerminalNotebook::has_reconnect_home`]. A session is
/// reconnectable in place when it owns a tab page, lives in a detached window
/// (issue #236), or occupies a split pane (issue #328) — the last being the one
/// that has no `TabPage` of its own.
#[must_use]
fn session_has_home(has_tab: bool, is_detached: bool, in_split_pane: bool) -> bool {
    has_tab || is_detached || in_split_pane
}

impl TerminalNotebook {
    // ========================================================================
    // Reconnect Preparation
    // ========================================================================

    /// Prepares a session for reconnection by cleaning up the previous state.
    ///
    /// When a session disconnects and the user clicks Reconnect (or auto-reconnect
    /// fires), instead of closing the tab and opening a fresh one (which loses
    /// tab position, scrollback, and causes visual flicker), this method:
    /// 1. Removes the reconnect banner from the tab container
    /// 2. Resets the VTE terminal (clears screen, resets state)
    /// 3. Clears the disconnected indicator
    /// 4. Removes stale automation sessions
    /// 5. Cancels any background polling
    ///
    /// After calling this, the caller can re-use the same `session_id` to
    /// spawn a new process in the existing terminal via `spawn_ssh()` etc.
    ///
    /// Returns `true` if the session was successfully prepared — in its tab or
    /// in its detached window — and `false` if the session no longer exists
    /// (closed by the user).
    pub fn prepare_for_reconnect(&self, session_id: Uuid) -> bool {
        // Refuse to reconnect a session that is still live. Resetting a live
        // VTE (below) would wipe its screen and the in-place spawn would launch
        // a second process over a working connection — exactly what an
        // accidental Reconnect keypress must never do (issue #328). Every UI
        // entry point already fires only for a disconnected session, but this
        // is the shared invariant: a future caller cannot bypass it, and it is
        // checked before the `disconnected_sessions` removal further down.
        if !self.is_session_disconnected(session_id) {
            tracing::debug!(
                %session_id,
                "prepare_for_reconnect: session is live, refusing to reset it"
            );
            return false;
        }

        // Check that the session still has a place to reconnect into: a tab, a
        // detached window (issue #236), or a split pane (issue #328). All three
        // keep the reconnected session where it is instead of falling back to
        // close+create, which would drop a split guest into a fresh tab and tear
        // the split layout apart.
        if !self.has_reconnect_home(session_id) {
            return false;
        }
        let page = self.sessions.borrow().get(&session_id).cloned();

        // Cancel any background polling (auto-reconnect)
        self.cancel_poll(session_id);

        // Remove the reconnect banner from wherever the session currently lives.
        // This also allows a new banner to be shown if this reconnect fails.
        self.remove_reconnect_banner(session_id);

        // Reset the VTE terminal (clear screen, reset state machine)
        if let Some(terminal) = self.terminals.borrow().get(&session_id) {
            if self.keep_history_on_reconnect.get() {
                self.reset_keeping_history(session_id, terminal);
            } else {
                terminal.reset(true, true);
                // A cleared buffer restarts at row 0, so no baseline is needed
                // until the caller's banner pins one.
                self.cursor_row_base.borrow_mut().remove(&session_id);
            }
        }

        // Clear disconnected indicator (a detached session has no tab to clear)
        if let Some(ref page) = page {
            page.set_indicator_icon(gio::Icon::NONE);
            page.set_indicator_tooltip("");
        }

        // The session is live again, so it becomes focusable by the smart
        // double-click once more (issue #242).
        self.disconnected_sessions.borrow_mut().remove(&session_id);

        // A new process starts in this session now; the expired-sign-in check
        // measures its age from here, not from when the tab was opened.
        if let Some(info) = self.session_info.borrow_mut().get_mut(&session_id) {
            info.reconnected_at = Some(chrono::Utc::now());
        }

        // Remove stale automation session (will be re-created by the caller)
        self.automation_sessions.borrow_mut().remove(&session_id);

        // Remove stale highlight rules (will be re-applied by the caller)
        self.session_highlight_rules
            .borrow_mut()
            .remove(&session_id);

        // Remove stale highlight overlay (will be re-created by set_highlight_rules).
        // Dropping it takes its layer off the terminal's overlay; dropped outside
        // the map borrow because that `Drop` talks to GTK.
        let stale_overlay = self.highlight_overlays.borrow_mut().remove(&session_id);
        drop(stale_overlay);

        // Remove stale VTE child PID entry — the process should have already
        // exited (child-exited removes it), but if reconnect is triggered
        // before child-exited fires (e.g. timeout disconnect), we must clean
        // it to avoid killing a recycled PID later.
        self.vte_child_pids.borrow_mut().remove(&session_id);

        true
    }

    /// Whether a session still has somewhere to reconnect into.
    ///
    /// A reconnect reuses the session's existing VTE widget in place, so it
    /// needs a live home for that widget: its own tab page, a detached window
    /// (issue #236), or a pane of some split layout (issue #328). A split guest
    /// owns no `TabPage` — parking removed its `self.sessions` entry — and is
    /// not detached, but its widget still lives inside the owner's pane and is
    /// still keyed by `session_id` in `self.terminals`, so the split-pane box
    /// provider resolving `Some` is the cheapest "has a pane home" signal.
    ///
    /// The decision itself is [`session_has_home`], kept pure and GTK-free so it
    /// can be unit-tested; this method only gathers the three live signals.
    #[must_use]
    fn has_reconnect_home(&self, session_id: Uuid) -> bool {
        let has_tab = self.sessions.borrow().contains_key(&session_id);
        let in_split_pane = self
            .split_pane_box_provider
            .borrow()
            .as_ref()
            .is_some_and(|provider| provider(session_id).is_some());
        session_has_home(has_tab, self.is_detached(session_id), in_split_pane)
    }

    /// Resets a terminal for reconnect while keeping its scrollback (issue #253).
    ///
    /// VTE only drops the scrollback when `reset()` is called with
    /// `clear_history`, so the preserved output is simply what the terminal
    /// already holds — nothing is copied. Three details:
    ///
    /// - The alternate screen must be left explicitly (see
    ///   [`LEAVE_ALTERNATE_SCREEN`] for rationale).
    /// - The dead session's output may end mid-line, so a separator opens a
    ///   fresh line and marks where the new session begins.
    /// - The user may have scrolled up while reading the dead session; the
    ///   viewport goes back to the bottom so the new output is visible without
    ///   a manual scroll.
    pub(super) fn reset_keeping_history(&self, session_id: Uuid, terminal: &Terminal) {
        // If a cap is set, trim the old scrollback by temporarily lowering VTE's
        // limit. VTE drops the oldest lines when the cap shrinks, then restoring
        // the original value lets the new session grow normally.
        if let Some(max_lines) = self.max_scrollback_on_reconnect.get() {
            let original = terminal.scrollback_lines();
            if original > i64::from(max_lines) {
                terminal.set_scrollback_lines(i64::from(max_lines));
                terminal.set_scrollback_lines(original);
            }
        }

        terminal.reset(true, false);
        terminal.feed(LEAVE_ALTERNATE_SCREEN);

        let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        terminal.feed(reconnect_separator(&i18n_f("Reconnected at {}", &[&stamp])).as_bytes());

        // Everything fed above is processed asynchronously by VTE, so the
        // cursor is still where the preserved history ended. That row is the
        // baseline until the caller's banner pins a better one
        // (`display_connect_banner`); it keeps the kept scrollback out of
        // the count either way.
        self.cursor_row_base.borrow_mut().insert(
            session_id,
            super::output_start::OutputStart::Row(terminal.cursor_position().1),
        );

        if let Some(adjustment) = terminal.vadjustment() {
            adjustment.set_value(adjustment.upper() - adjustment.page_size());
        }
    }

    // ========================================================================
    // Poll Cancellation
    // ========================================================================

    /// Registers a cancel token for a background polling task
    pub fn register_poll_cancel(
        &self,
        key: Uuid,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        self.poll_cancel_tokens.borrow_mut().insert(key, cancel);
    }

    /// Records whether an unattended sweep may reconnect this session.
    ///
    /// Called by the disconnect path with the same verdict it reached for its
    /// own auto-reconnect poll, so the two cannot disagree. Always call it —
    /// passing `false` clears an earlier `true`, which matters for a session
    /// that dropped once from a real failure and later exited cleanly.
    pub fn set_auto_reconnect_eligible(&self, session_id: Uuid, eligible: bool) {
        let mut set = self.auto_reconnect_eligible.borrow_mut();
        if eligible {
            set.insert(session_id);
        } else {
            set.remove(&session_id);
        }
    }

    /// Whether an unattended sweep may reconnect this session.
    ///
    /// Defaults to `false`: a session whose disconnect never reached the
    /// decision point is left alone rather than logged back in on a guess.
    #[must_use]
    pub fn is_auto_reconnect_eligible(&self, session_id: Uuid) -> bool {
        self.auto_reconnect_eligible.borrow().contains(&session_id)
    }

    /// Cancels and removes a background polling task by key
    pub fn cancel_poll(&self, key: Uuid) {
        if let Some(cancel) = self.poll_cancel_tokens.borrow_mut().remove(&key) {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            tracing::debug!(%key, "Cancelled background poll");
        }
    }

    // ========================================================================
    // Connection Status Indicators
    // ========================================================================

    /// Marks a tab as disconnected (changes indicator)
    ///
    /// A detached session has no tab to carry the indicator, so its window is
    /// marked instead: the reconnect banner only covers protocols that can
    /// reconnect in place, which leaves an embedded RDP/VNC session with no
    /// signal at all otherwise (issue #236).
    pub fn mark_tab_disconnected(&self, session_id: Uuid) {
        self.disconnected_sessions.borrow_mut().insert(session_id);
        self.sync_vte_reconnect_item(session_id);
        if self.is_detached(session_id) {
            Self::mark_detached_window_disconnected(session_id, true);
        }
        if let Some(page) = self.sessions.borrow().get(&session_id) {
            page.set_indicator_icon(Some(&gio::ThemedIcon::new("network-offline-symbolic")));
            page.set_indicator_tooltip(&i18n("Session disconnected"));
            page.set_indicator_activatable(false);
        }
        // Reset VTE internal state to prevent use-after-free in libvte/pango
        // during the next GTK snapshot cycle. After the child process exits,
        // VTE may hold stale references to Pango font resources that get
        // invalidated (e.g. on screen lock/unlock or GPU context loss).
        // Calling reset(true, false) forces VTE to release internal state
        // (including Pango layout caches) while preserving scrollback history
        // for reconnect (#171). The preserved history is only readable on the
        // normal screen, hence the explicit switch (#253).
        if let Some(terminal) = self.terminals.borrow().get(&session_id) {
            terminal.reset(true, false);
            terminal.feed(LEAVE_ALTERNATE_SCREEN);
        }
    }

    /// Marks a tab as connected (removes the disconnected indicator).
    ///
    /// A split owner's tab uses the same single `indicator-icon` slot to show
    /// its split color, so preserve that here instead of clearing it — otherwise
    /// connection-state events (RDP fires "connected" on every resolution change)
    /// would wipe the split-color indicator.
    pub fn mark_tab_connected(&self, session_id: Uuid) {
        self.disconnected_sessions.borrow_mut().remove(&session_id);
        self.sync_vte_reconnect_item(session_id);
        if self.is_detached(session_id) {
            Self::mark_detached_window_disconnected(session_id, false);
        }
        if let Some(&color_index) = self.split_session_colors.borrow().get(&session_id) {
            if let Some(page) = self.sessions.borrow().get(&session_id)
                && let Some(icon) = crate::split_view::create_colored_circle_icon(color_index, 16)
            {
                page.set_indicator_icon(Some(&icon));
                page.set_indicator_tooltip("");
                page.set_indicator_activatable(false);
            }
            return;
        }
        if let Some(page) = self.sessions.borrow().get(&session_id) {
            page.set_indicator_icon(gio::Icon::NONE);
            page.set_indicator_tooltip("");
        }
    }

    /// Reveals or hides the disconnect banner of a detached session's window.
    ///
    /// Goes through the thread-local registry rather than a callback, because
    /// the notebook is constructed before any window exists and holds no handle
    /// to one. A session whose window has already gone (its close is what ended
    /// the session) is simply not found.
    pub(super) fn mark_detached_window_disconnected(session_id: Uuid, disconnected: bool) {
        let marked = crate::window::detached_window_registry()
            .is_some_and(|registry| registry.set_session_disconnected(session_id, disconnected));
        tracing::debug!(
            session = %session_id,
            disconnected,
            marked,
            "detached window connection state updated"
        );
    }

    /// Forces every VTE terminal to drop and rebuild its cached font state.
    ///
    /// VTE reads `gtk-fontconfig-timestamp` only when it creates its cached
    /// `FontInfo` (the timestamp is part of the font-cache key) and never
    /// subscribes to changes. After a fontconfig update (font installation,
    /// `fc-cache`, or KDE pushing `Fontconfig/Timestamp` via XSettings on
    /// screen unlock) terminals keep Pango objects that may reference freed
    /// fonts, which crashes with SIGSEGV inside `pango_itemize` during the
    /// next GTK snapshot (#171). Re-applying the current font description
    /// goes through `vte_terminal_set_font`, which deliberately recreates
    /// the font even when the description is unchanged, picking up the new
    /// timestamp and releasing the stale Pango state.
    pub fn refresh_fonts_after_fontconfig_change(&self) {
        for (session_id, terminal) in self.terminals.borrow().iter() {
            let desc = terminal.font_desc();
            terminal.set_font(desc.as_ref());
            tracing::debug!(%session_id, "Refreshed VTE font after fontconfig change");
        }
    }

    // ========================================================================
    // Reconnect Overlay Banner
    // ========================================================================

    /// Shows a reconnect overlay banner at the bottom of a disconnected VTE tab
    ///
    /// Appends a horizontal bar with a "Session disconnected" label and a
    /// "Reconnect" button to the tab's container. The button triggers the
    /// `on_reconnect` callback with the session's connection ID.
    ///
    /// If `auto_reconnect_active` is true, an additional label is shown
    /// indicating that automatic reconnection is in progress.
    pub fn show_reconnect_overlay(&self, session_id: Uuid) {
        self.show_reconnect_overlay_with_status(session_id, false);
    }

    /// Shows a reconnect overlay with optional auto-reconnect status indicator
    pub fn show_reconnect_overlay_with_status(
        &self,
        session_id: Uuid,
        auto_reconnect_active: bool,
    ) {
        // Guard: child-exited can fire twice for the same session; show only one
        // banner. Checked without marking, so a session whose banner could not
        // be placed yet is not locked out of ever showing one (issue #236).
        if self.reconnect_shown.borrow().contains(&session_id) {
            // If banner already shown but auto-reconnect just started, update it
            if auto_reconnect_active {
                self.update_reconnect_banner_status(session_id, true);
            }
            return;
        }

        let Some(info) = self.session_info.borrow().get(&session_id).cloned() else {
            return;
        };

        // Only for VTE-based protocols (SSH, Telnet, Serial, Kubernetes)
        if matches!(info.protocol.as_str(), "rdp" | "vnc" | "spice") {
            return;
        }

        // The session's own split pane, else its tab's content box, else the
        // detached window's one.
        let Some(container) = self.reconnect_banner_box(session_id) else {
            return;
        };
        self.reconnect_shown.borrow_mut().insert(session_id);

        // Build the reconnect banner. A group, so a screen reader presents the
        // label, the key hint and the buttons as one thing.
        let banner = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(6)
            .accessible_role(gtk4::AccessibleRole::Group)
            .build();
        banner.set_margin_start(12);
        banner.set_margin_end(12);
        banner.set_margin_top(6);
        banner.set_margin_bottom(6);
        banner.set_halign(gtk4::Align::Center);
        banner.set_widget_name("reconnect-banner");

        let label = gtk4::Label::new(Some(&i18n("Session disconnected")));
        label.add_css_class("dim-label");

        banner.append(&label);
        banner.update_relation(&[gtk4::accessible::Relation::LabelledBy(
            &[label.upcast_ref()],
        )]);
        // The visible hint names R and Q only, to stay short in a narrow pane;
        // the description names Enter too.
        banner.update_property(&[gtk4::accessible::Property::Description(&i18n(
            "Press R to reconnect, Q to close, or Enter twice for the suggested action",
        ))]);

        // The keys below only work while this banner is up, so it names them.
        // Placed before the status label and the button: the status label is
        // inserted relative to the button, which has to stay the last child.
        let keys_hint = gtk4::Label::new(None);
        keys_hint.set_widget_name(KEYS_HINT);
        keys_hint.add_css_class("dim-label");
        keys_hint.add_css_class("caption");
        // In a narrow split pane the hint gives way first, so the banner never
        // forces the pane wider than its share and the button stays in view.
        // The tooltip carries the full text the ellipsis may cut.
        keys_hint.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        set_keys_hint(&keys_hint, &i18n("Press R to reconnect, Q to close"));
        banner.append(&keys_hint);

        // Auto-reconnect status indicator
        if auto_reconnect_active {
            let status_label = gtk4::Label::new(Some(&i18n("Auto-reconnecting…")));
            status_label.add_css_class("dim-label");
            status_label.set_widget_name("reconnect-status");
            banner.append(&status_label);
        }

        let button = gtk4::Button::with_label(&i18n("Reconnect"));
        button.add_css_class("suggested-action");
        button.set_tooltip_text(Some(&i18n("Reconnect to this session")));

        banner.append(&button);
        container.append(&banner);

        // Wire up the reconnect button
        let on_reconnect = self.on_reconnect.clone();
        let connection_id = info.connection_id;
        button.connect_clicked(move |_| {
            if let Some(ref callback) = *on_reconnect.borrow() {
                callback(session_id, connection_id);
            }
        });

        self.install_ended_session_keys(session_id, connection_id, &banner);
        self.sync_vte_reconnect_item(session_id);

        tracing::info!(
            %session_id,
            protocol = %info.protocol,
            "Reconnect overlay shown for disconnected session"
        );
    }

    /// Gives R, Q and Enter a meaning on a session whose connection has ended.
    ///
    /// A terminal whose child has exited takes no input, so the keys are free:
    /// R does what the banner's Reconnect button does, Q closes the tab, the
    /// split pane or the window tab the session is shown in, and Enter pressed
    /// twice clicks the banner's suggested button — Reconnect, or "Log In to
    /// …" once [`Self::offer_cloud_login`] has moved the suggestion there. The
    /// first Enter only highlights that button and says what the second will
    /// do. What a key means is [`ended_session_key_action`]; this method only
    /// feeds it and carries the answer out.
    ///
    /// The controller lives on the terminal, so it follows the session wherever
    /// its widget is moved, and exists only while the reconnect banner does:
    /// [`Self::remove_reconnect_banner`] takes it away again, which is what
    /// hands the keys back to a session that reconnected in place, and drops
    /// a pending first Enter with it.
    ///
    /// Keys are read through [`crate::utils::latin_keyval`], so they work under
    /// a non-Latin layout, and only without Ctrl, Alt or Super, so no shortcut
    /// is shadowed. A handled key is stopped in the capture phase, before VTE
    /// sees it, so it is never committed and never reaches a broadcast.
    fn install_ended_session_keys(&self, session_id: Uuid, connection_id: Uuid, banner: &GtkBox) {
        let Some(terminal) = self.terminals.borrow().get(&session_id).cloned() else {
            return;
        };
        Self::remove_ended_session_keys(&terminal);

        let controller = gtk4::EventControllerKey::new();
        controller.set_name(Some(ENDED_SESSION_KEYS));
        // Capture phase: VTE would otherwise consume the key press itself.
        controller.set_propagation_phase(gtk4::PropagationPhase::Capture);

        // Per banner: a new banner gets a new controller and a fresh state.
        let enter = Rc::new(std::cell::Cell::new(EnterState::Idle));

        let actions = EndedSessionKeyActions {
            session_id,
            connection_id,
            on_reconnect: self.on_reconnect.clone(),
            reconnect_shown: Rc::clone(&self.reconnect_shown),
            split_pane_focuser: Rc::clone(&self.split_pane_focuser),
            terminal: terminal.downgrade(),
            banner: banner.downgrade(),
        };
        let banner_shown = {
            let reconnect_shown = Rc::clone(&self.reconnect_shown);
            let disconnected = Rc::clone(&self.disconnected_sessions);
            move || {
                ended_session_actions_available(
                    reconnect_shown.borrow().contains(&session_id),
                    disconnected.borrow().contains(&session_id),
                )
            }
        };

        let enter_pressed = Rc::clone(&enter);
        controller.connect_key_pressed(move |_, keyval, keycode, modifiers| {
            let key = EndedSessionKey::from_keyval(crate::utils::latin_keyval(keyval, keycode));
            let action = ended_session_key_action(
                key,
                has_shortcut_modifier(modifiers),
                banner_shown(),
                enter_pressed.get(),
            );
            if let Some(next) = action.next_enter_state() {
                enter_pressed.set(next);
            }
            actions.run(action);
            action.propagation()
        });

        // The second Enter counts only after the first was let go, so holding
        // Enter down (key repeat) never runs the action on its own.
        let enter_released = Rc::clone(&enter);
        controller.connect_key_released(move |_, keyval, keycode, _| {
            let key = EndedSessionKey::from_keyval(crate::utils::latin_keyval(keyval, keycode));
            if key == EndedSessionKey::Enter && enter_released.get() == EnterState::Held {
                enter_released.set(EnterState::Armed);
            }
        });
        terminal.add_controller(controller);
    }

    /// Takes the R / Q / Enter controller off a terminal, if it has one.
    fn remove_ended_session_keys(terminal: &Terminal) {
        let controllers = terminal.observe_controllers();
        let stale: Vec<gtk4::EventController> = (0..controllers.n_items())
            .filter_map(|position| controllers.item(position).and_downcast())
            .filter(|controller: &gtk4::EventController| {
                controller.name().as_deref() == Some(ENDED_SESSION_KEYS)
            })
            .collect();
        for controller in &stale {
            terminal.remove_controller(controller);
        }
    }

    /// Updates the auto-reconnect status label in an existing reconnect banner
    pub fn update_reconnect_banner_status(&self, session_id: Uuid, active: bool) {
        let Some(container) = self.reconnect_banner_box(session_id) else {
            return;
        };

        // Find the reconnect-banner widget
        let mut child = container.first_child();
        while let Some(widget) = child {
            if widget.widget_name() == "reconnect-banner" {
                if let Ok(banner) = widget.downcast::<GtkBox>() {
                    // Check if status label already exists
                    let mut has_status = false;
                    let mut banner_child = banner.first_child();
                    while let Some(bc) = banner_child {
                        if bc.widget_name() == "reconnect-status" {
                            has_status = true;
                            if !active {
                                banner.remove(&bc);
                            }
                            break;
                        }
                        banner_child = bc.next_sibling();
                    }
                    // Add status label if needed and not already present
                    if active && !has_status {
                        let status_label = gtk4::Label::new(Some(&i18n("Auto-reconnecting…")));
                        status_label.add_css_class("dim-label");
                        status_label.set_widget_name("reconnect-status");
                        // Insert before the button (last child)
                        if let Some(button) = banner.last_child() {
                            banner
                                .insert_child_after(&status_label, button.prev_sibling().as_ref());
                        } else {
                            banner.append(&status_label);
                        }
                    }
                }
                break;
            }
            child = widget.next_sibling();
        }
    }

    /// Updates the auto-reconnect status label with attempt progress (N/M)
    pub fn update_reconnect_banner_attempt(
        &self,
        session_id: Uuid,
        attempt: u32,
        max_attempts: u32,
    ) {
        let Some(container) = self.reconnect_banner_box(session_id) else {
            return;
        };

        // Find the reconnect-banner widget
        let mut child = container.first_child();
        while let Some(widget) = child {
            if widget.widget_name() == "reconnect-banner" {
                if let Ok(banner) = widget.downcast::<GtkBox>() {
                    // Find or create the status label
                    let mut banner_child = banner.first_child();
                    while let Some(bc) = banner_child {
                        if bc.widget_name() == "reconnect-status" {
                            if let Ok(label) = bc.downcast::<gtk4::Label>() {
                                label.set_label(&i18n_f(
                                    "Auto-reconnecting (attempt {}/{})",
                                    &[&attempt.to_string(), &max_attempts.to_string()],
                                ));
                            }
                            return;
                        }
                        banner_child = bc.next_sibling();
                    }
                    // Status label not found — create it
                    let status_label = gtk4::Label::new(Some(&i18n_f(
                        "Auto-reconnecting (attempt {}/{})",
                        &[&attempt.to_string(), &max_attempts.to_string()],
                    )));
                    status_label.add_css_class("dim-label");
                    status_label.set_widget_name("reconnect-status");
                    if let Some(button) = banner.last_child() {
                        banner.insert_child_after(&status_label, button.prev_sibling().as_ref());
                    } else {
                        banner.append(&status_label);
                    }
                }
                break;
            }
            child = widget.next_sibling();
        }
    }

    /// Returns the box a session's reconnect banner belongs in.
    ///
    /// A session shown in a split pane, owner or guest, gets the banner in its
    /// own pane: for the owner, [`Self::session_content_box`] is the tab's
    /// box around the whole split, which would put the banner below every
    /// pane. Any other session uses [`Self::session_content_box`].
    fn reconnect_banner_box(&self, session_id: Uuid) -> Option<GtkBox> {
        let provider = self
            .split_pane_box_provider
            .borrow()
            .as_ref()
            .map(Rc::clone);
        if let Some(provider) = provider
            && let Some(pane_box) = provider(session_id)
        {
            return Some(pane_box);
        }
        self.session_content_box(session_id)
    }

    /// Returns the session's reconnect banner, if one is shown.
    fn reconnect_banner(&self, session_id: Uuid) -> Option<GtkBox> {
        let container = self.reconnect_banner_box(session_id)?;
        let mut child = container.first_child();
        while let Some(widget) = child {
            if widget.widget_name() == "reconnect-banner" {
                return widget.downcast::<GtkBox>().ok();
            }
            child = widget.next_sibling();
        }
        None
    }

    /// Takes the session's reconnect banner widgets down, leaving its mark.
    ///
    /// Looks in the session's own pane and in its tab's content box, where an
    /// owner's banner sat before it moved into a split.
    fn remove_reconnect_banner_widgets(&self, session_id: Uuid) {
        let boxes = [
            self.reconnect_banner_box(session_id),
            self.session_content_box(session_id),
        ];
        for container in boxes.iter().flatten() {
            let mut child = container.first_child();
            while let Some(widget) = child {
                let next = widget.next_sibling();
                if widget.widget_name() == "reconnect-banner" {
                    container.remove(&widget);
                }
                child = next;
            }
        }
    }

    /// Removes the reconnect banner and allows a later one to be shown.
    pub fn remove_reconnect_banner(&self, session_id: Uuid) {
        self.remove_reconnect_banner_widgets(session_id);
        self.reconnect_shown.borrow_mut().remove(&session_id);
        self.cloud_login_offers.borrow_mut().remove(&session_id);
        if let Some(terminal) = self.terminals.borrow().get(&session_id) {
            Self::remove_ended_session_keys(terminal);
        }
        self.sync_vte_reconnect_item(session_id);
    }

    /// Offers or withdraws the terminal menu's Reconnect entry to match the
    /// session's state (see [`ended_session_actions_available`]).
    fn sync_vte_reconnect_item(&self, session_id: Uuid) {
        let offered = ended_session_actions_available(
            self.reconnect_shown.borrow().contains(&session_id),
            self.disconnected_sessions.borrow().contains(&session_id),
        );
        if let Some(item) = self.vte_reconnect_items.borrow().get(&session_id) {
            item.set_offered(offered);
        }
    }

    /// Puts a session's reconnect banner back if a move dropped it.
    ///
    /// Runs when the session's terminal is mapped. Moving a session between a
    /// tab, a split pane and a window rebuilds the box around its terminal,
    /// and the banner goes with the old box while the session stays marked.
    /// A marked, disconnected session with no banner in its own box gets one
    /// again, with its "Log In to …" button if it had one. A mark left on a
    /// live session is dropped.
    pub fn reconcile_reconnect_banner(&self, session_id: Uuid) {
        let marked = self.reconnect_shown.borrow().contains(&session_id);
        if !marked {
            return;
        }
        let disconnected = self.disconnected_sessions.borrow().contains(&session_id);
        let present = self.reconnect_banner(session_id).is_some();
        let action = reconnect_banner_reconcile(marked, disconnected, present);
        tracing::debug!(
            %session_id,
            branch = ?action,
            marked,
            disconnected,
            present,
            "reconnect banner reconcile"
        );
        match action {
            BannerReconcile::Keep => {}
            BannerReconcile::Reshow => {
                // A banner left in the tab's box would be a second one.
                self.remove_reconnect_banner_widgets(session_id);
                self.reconnect_shown.borrow_mut().remove(&session_id);
                let polling = self.poll_cancel_tokens.borrow().contains_key(&session_id);
                self.show_reconnect_overlay_with_status(session_id, polling);
                let login = self.cloud_login_offers.borrow().get(&session_id).cloned();
                if let Some(login) = login {
                    self.offer_cloud_login(session_id, login);
                }
            }
            BannerReconcile::ForgetMark => self.remove_reconnect_banner(session_id),
        }
    }

    /// Adds a "Log In to `<provider>`" button to the session's reconnect banner.
    ///
    /// Shown when the session ended because the cloud CLI's credentials
    /// expired, so reconnecting cannot succeed until the user signs in again.
    /// The button goes before "Reconnect" and takes over the suggested-action
    /// style, because it is the step that has to come first. Clicking it hands
    /// the login command to the `on_cloud_login` callback.
    pub fn offer_cloud_login(&self, session_id: Uuid, login: rustconn_core::protocol::CloudLogin) {
        let Some(banner) = self.reconnect_banner(session_id) else {
            return;
        };
        // A doubled `child-exited` reaches here twice for one banner.
        let mut child = banner.first_child();
        while let Some(widget) = child {
            if widget.widget_name() == CLOUD_LOGIN_BUTTON {
                return;
            }
            child = widget.next_sibling();
        }
        let Some(connection_id) = self
            .session_info
            .borrow()
            .get(&session_id)
            .map(|info| info.connection_id)
        else {
            return;
        };

        // The banner's first child is its "Session disconnected" label.
        if let Some(label) = banner
            .first_child()
            .and_then(|w| w.downcast::<gtk4::Label>().ok())
        {
            label.set_label(&i18n("Sign-in expired"));
        }

        let reconnect_button = banner.last_child();
        if let Some(ref reconnect) = reconnect_button {
            reconnect.remove_css_class("suggested-action");
        }

        // Provider names are brand names and stay untranslated.
        let button = gtk4::Button::with_label(&i18n_f("Log In to {}", &[login.provider_name()]));
        button.add_css_class("suggested-action");
        button.set_widget_name(CLOUD_LOGIN_BUTTON);
        button.set_tooltip_text(Some(&i18n_f(
            "Run “{}” to sign in again in the browser",
            &[&login.command_line()],
        )));
        banner.insert_child_after(
            &button,
            reconnect_button.and_then(|r| r.prev_sibling()).as_ref(),
        );

        tracing::info!(
            %session_id,
            provider = login.provider_name(),
            command = %login.command_line(),
            "Offering cloud login for expired credentials"
        );

        // Kept so a banner shown again after a move gets this button back.
        self.cloud_login_offers
            .borrow_mut()
            .insert(session_id, login.clone());

        let on_cloud_login = self.on_cloud_login.clone();
        button.connect_clicked(move |_| {
            if let Some(ref callback) = *on_cloud_login.borrow() {
                callback(session_id, connection_id, login.clone());
            }
        });
    }

    // ========================================================================
    // Reconnect Callback Management
    // ========================================================================

    /// Sets the callback invoked when a banner's cloud login button is clicked.
    ///
    /// The callback receives `(session_id, connection_id, login)`.
    pub fn set_on_cloud_login<F>(&self, callback: F)
    where
        F: Fn(Uuid, Uuid, rustconn_core::protocol::CloudLogin) + 'static,
    {
        *self.on_cloud_login.borrow_mut() = Some(Box::new(callback));
    }

    /// Sets the callback invoked when a reconnect button is clicked
    ///
    /// The callback receives `(session_id, connection_id)`.
    pub fn set_on_reconnect<F>(&self, callback: F)
    where
        F: Fn(Uuid, Uuid) + 'static,
    {
        *self.on_reconnect.borrow_mut() = Some(Box::new(callback));
    }

    /// Sets the resolver for a split guest's pane container box (issue #328).
    ///
    /// The window wires this to look a session up across its per-tab split
    /// bridges, so [`Self::session_content_box`] can attach a reconnect banner
    /// to a pane that has no `TabPage` of its own.
    pub fn set_split_pane_box_provider<F>(&self, provider: F)
    where
        F: Fn(Uuid) -> Option<GtkBox> + 'static,
    {
        *self.split_pane_box_provider.borrow_mut() = Some(Rc::new(provider));
    }

    /// Sets the hook run, on idle, each time a session's terminal is mapped.
    ///
    /// The window wires it to [`Self::reconcile_reconnect_banner`], holding
    /// the notebook weakly: the hook is reachable from every terminal.
    pub fn set_on_terminal_mapped<F>(&self, hook: F)
    where
        F: Fn(Uuid) + 'static,
    {
        *self.on_terminal_mapped.borrow_mut() = Some(Rc::new(hook));
    }

    /// Sets the hook that focuses the split pane showing a session.
    ///
    /// Returns whether the session is shown in a pane. Used by Q on an ended
    /// session so `win.close-pane` closes the pane that got the key, not
    /// whichever pane was last clicked.
    pub fn set_split_pane_focuser<F>(&self, focuser: F)
    where
        F: Fn(Uuid) -> bool + 'static,
    {
        *self.split_pane_focuser.borrow_mut() = Some(Rc::new(focuser));
    }

    /// Sets the resolver for the focused pane's session in a split tab (#371).
    ///
    /// The window wires this to the tab's `SplitViewBridge`, which is the only
    /// thing that knows which pane has focus. `Save Output` uses it so a split
    /// tab saves the focused pane, not the owner. Returns `None` when the tab
    /// has no split.
    pub fn set_focused_session_provider<F>(&self, provider: F)
    where
        F: Fn(Uuid) -> Option<Uuid> + 'static,
    {
        *self.focused_session_provider.borrow_mut() = Some(Rc::new(provider));
    }

    /// Wires the tab context menu's broadcast-membership toggle (issue #329).
    pub fn set_on_tab_broadcast_toggle<F>(&self, callback: F)
    where
        F: Fn(Uuid) + 'static,
    {
        *self.on_tab_broadcast_toggle.borrow_mut() = Some(Box::new(callback));
    }

    /// Wires the broadcast-membership query used to label the tab menu item.
    pub fn set_tab_broadcast_membership_provider<F>(&self, provider: F)
    where
        F: Fn(Uuid) -> bool + 'static,
    {
        *self.tab_broadcast_membership.borrow_mut() = Some(Rc::new(provider));
    }

    /// Wires the query behind the tab menu's Edit Connection and Copy
    /// Wires the predicate the tab menu uses to decide whether the right-clicked
    /// tab has a saved connection, so **Edit Connection…** is offered only when
    /// there is one to edit (issue #357).
    pub(crate) fn set_tab_connection_menu_provider<F>(&self, provider: F)
    where
        F: Fn(Uuid) -> bool + 'static,
    {
        *self.tab_connection_menu.borrow_mut() = Some(Rc::new(provider));
    }

    /// Returns a clone of the reconnect callback reference for use in auto-reconnect polling
    #[must_use]
    pub fn reconnect_callback(&self) -> Rc<RefCell<Option<Box<dyn Fn(Uuid, Uuid)>>>> {
        self.on_reconnect.clone()
    }

    // ========================================================================
    // Session Status Queries
    // ========================================================================

    /// Returns `true` if the session currently has a reconnect banner displayed.
    ///
    /// Used by the network monitor to identify sessions that need immediate
    /// reconnection after a network interface change.
    #[must_use]
    pub fn is_reconnect_shown(&self, session_id: Uuid) -> bool {
        self.reconnect_shown.borrow().contains(&session_id)
    }

    /// Returns `true` if the session's connection has ended but its tab is still
    /// open (issue #242).
    ///
    /// Such a session must not be treated as something to focus or to save for
    /// restore: it is a readable transcript with a Reconnect button, not a live
    /// connection.
    #[must_use]
    pub fn is_session_disconnected(&self, session_id: Uuid) -> bool {
        self.disconnected_sessions.borrow().contains(&session_id)
    }

    /// Returns the sessions that are still live (tab open and connected).
    ///
    /// The counterpart of [`Self::get_all_sessions`] for every caller that means
    /// "sessions I can hand the user" rather than "tabs that exist".
    #[must_use]
    pub fn live_sessions(&self) -> Vec<TerminalSession> {
        let disconnected = self.disconnected_sessions.borrow();
        self.session_info
            .borrow()
            .values()
            .filter(|s| !disconnected.contains(&s.id))
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BannerReconcile, EndedSessionKey as Key, EndedSessionKeyAction as Action, EnterState,
        ended_session_actions_available, ended_session_key_action, has_shortcut_modifier,
        reconnect_banner_reconcile, session_has_home,
    };
    use gtk4::gdk;

    fn decide(key: Key, enter: EnterState) -> Action {
        ended_session_key_action(key, false, true, enter)
    }

    // Only a marked, disconnected session offers the ended-session actions;
    // a cloud login clears the mark, so it never does mid-login.
    #[test]
    fn ended_session_actions_need_mark_and_disconnect() {
        assert!(ended_session_actions_available(true, true));
        assert!(!ended_session_actions_available(true, false));
        assert!(!ended_session_actions_available(false, true));
        assert!(!ended_session_actions_available(false, false));
    }

    #[test]
    fn reconnect_banner_reconcile_reshows_a_dropped_banner() {
        assert_eq!(
            reconnect_banner_reconcile(true, true, false),
            BannerReconcile::Reshow
        );
        assert_eq!(
            reconnect_banner_reconcile(true, true, true),
            BannerReconcile::Keep
        );
    }

    #[test]
    fn reconnect_banner_reconcile_forgets_a_live_sessions_mark() {
        assert_eq!(
            reconnect_banner_reconcile(true, false, false),
            BannerReconcile::ForgetMark
        );
        assert_eq!(
            reconnect_banner_reconcile(true, false, true),
            BannerReconcile::ForgetMark
        );
    }

    // An unmarked session (a live one, or one running `aws login` after its
    // banner was taken down) never gets a banner back.
    #[test]
    fn reconnect_banner_reconcile_leaves_unmarked_sessions_alone() {
        for disconnected in [false, true] {
            for present in [false, true] {
                assert_eq!(
                    reconnect_banner_reconcile(false, disconnected, present),
                    BannerReconcile::Keep
                );
            }
        }
    }

    // R and Q act at once, whatever Enter was doing.
    #[test]
    fn r_reconnects_and_q_closes() {
        for enter in [EnterState::Idle, EnterState::Held, EnterState::Armed] {
            assert_eq!(decide(Key::Reconnect, enter), Action::Reconnect);
            assert_eq!(decide(Key::Close, enter), Action::Close);
        }
    }

    // Enter is two-step: the first arms, its key repeat changes nothing, and
    // only an Enter after release runs the suggested action.
    #[test]
    fn enter_needs_two_separate_presses() {
        assert_eq!(decide(Key::Enter, EnterState::Idle), Action::Arm);
        assert_eq!(Action::Arm.next_enter_state(), Some(EnterState::Held));
        assert_eq!(decide(Key::Enter, EnterState::Held), Action::Hold);
        assert_eq!(Action::Hold.next_enter_state(), None);
        assert_eq!(decide(Key::Enter, EnterState::Armed), Action::RunSuggested);
        assert_eq!(
            Action::RunSuggested.next_enter_state(),
            Some(EnterState::Idle)
        );
    }

    // Another key after the first Enter cancels it and still reaches the
    // terminal; a lone modifier (Shift before R) does not cancel.
    #[test]
    fn another_key_cancels_a_first_enter() {
        assert_eq!(decide(Key::Other, EnterState::Armed), Action::Disarm);
        assert_eq!(decide(Key::Other, EnterState::Held), Action::Disarm);
        assert_eq!(Action::Disarm.next_enter_state(), Some(EnterState::Idle));
        assert_eq!(
            Action::Disarm.propagation(),
            gtk4::glib::Propagation::Proceed
        );
        assert_eq!(decide(Key::Modifier, EnterState::Armed), Action::Pass);
        assert_eq!(decide(Key::Other, EnterState::Idle), Action::Pass);
    }

    // With Ctrl, Alt or Super held a key is a shortcut and is never taken; it
    // still cancels a pending Enter.
    #[test]
    fn shortcuts_pass_through() {
        for key in [Key::Reconnect, Key::Close, Key::Enter] {
            assert_eq!(
                ended_session_key_action(key, true, true, EnterState::Idle),
                Action::Pass
            );
        }
        assert_eq!(
            ended_session_key_action(Key::Enter, true, true, EnterState::Armed),
            Action::Disarm
        );
        assert!(has_shortcut_modifier(gdk::ModifierType::CONTROL_MASK));
        assert!(has_shortcut_modifier(gdk::ModifierType::ALT_MASK));
        assert!(has_shortcut_modifier(gdk::ModifierType::SUPER_MASK));
        assert!(!has_shortcut_modifier(gdk::ModifierType::SHIFT_MASK));
    }

    // Without a banner (a live session) no key is ever taken.
    #[test]
    fn a_live_session_keeps_every_key() {
        for key in [Key::Reconnect, Key::Close, Key::Enter, Key::Other] {
            for enter in [EnterState::Idle, EnterState::Armed] {
                let action = ended_session_key_action(key, false, false, enter);
                assert_eq!(action, Action::Pass);
                assert_eq!(action.propagation(), gtk4::glib::Propagation::Proceed);
            }
        }
    }

    // Every handled key is stopped before VTE, so it is never committed and
    // never broadcast to other panes.
    #[test]
    fn handled_keys_stop_before_the_terminal() {
        for action in [
            Action::Reconnect,
            Action::Close,
            Action::Arm,
            Action::Hold,
            Action::RunSuggested,
        ] {
            assert_eq!(action.propagation(), gtk4::glib::Propagation::Stop);
        }
    }

    // Upper and lower case, the keypad Enter and the modifiers are classified.
    #[test]
    fn keys_are_classified() {
        assert_eq!(Key::from_keyval(gdk::Key::r), Key::Reconnect);
        assert_eq!(Key::from_keyval(gdk::Key::R), Key::Reconnect);
        assert_eq!(Key::from_keyval(gdk::Key::Q), Key::Close);
        assert_eq!(Key::from_keyval(gdk::Key::Return), Key::Enter);
        assert_eq!(Key::from_keyval(gdk::Key::KP_Enter), Key::Enter);
        assert_eq!(Key::from_keyval(gdk::Key::Shift_L), Key::Modifier);
        assert_eq!(Key::from_keyval(gdk::Key::a), Key::Other);
        assert_eq!(Key::from_keyval(gdk::Key::space), Key::Other);
    }

    // A session with its own tab page is reconnectable — the ordinary case.
    #[test]
    fn a_tabbed_session_has_a_home() {
        assert!(session_has_home(true, false, false));
    }

    // A detached session has no tab page but its own window is a valid home
    // (issue #236).
    #[test]
    fn a_detached_session_has_a_home() {
        assert!(session_has_home(false, true, false));
    }

    // A split guest owns no tab page and is not detached, but its widget lives
    // in a pane — the regression that sent it to a fresh tab (issue #328).
    #[test]
    fn a_split_guest_has_a_home() {
        assert!(session_has_home(false, false, true));
    }

    // No placement at all means the session was closed: nothing to reconnect
    // into, so the caller must fall back to close+create.
    #[test]
    fn a_session_with_no_placement_has_no_home() {
        assert!(!session_has_home(false, false, false));
    }
}
