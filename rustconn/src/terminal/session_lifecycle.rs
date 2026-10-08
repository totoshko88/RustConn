//! Session reconnect, disconnect, and status management.
//!
//! Extracted from `terminal/mod.rs` to reduce module complexity.
//! Contains methods for handling session disconnect/reconnect lifecycle,
//! reconnect banners, VTE reset, and connection status tracking.

use super::*;

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
                // A cleared buffer restarts at row 0, so no baseline is needed.
                self.cursor_row_base.borrow_mut().remove(&session_id);
            }
        }

        // Clear disconnected indicator (a detached session has no tab to clear)
        if let Some(ref page) = page {
            page.set_indicator_icon(gio::Icon::NONE);
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
        // cursor row is not final yet — mark the baseline as pending and let
        // `get_terminal_cursor_row` capture it once the output has landed.
        self.cursor_row_base.borrow_mut().insert(session_id, None);

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
        if self.is_detached(session_id) {
            Self::mark_detached_window_disconnected(session_id, true);
        }
        if let Some(page) = self.sessions.borrow().get(&session_id) {
            page.set_indicator_icon(Some(&gio::ThemedIcon::new("network-offline-symbolic")));
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
        if self.is_detached(session_id) {
            Self::mark_detached_window_disconnected(session_id, false);
        }
        if let Some(&color_index) = self.split_session_colors.borrow().get(&session_id) {
            if let Some(page) = self.sessions.borrow().get(&session_id)
                && let Some(icon) = crate::split_view::create_colored_circle_icon(color_index, 16)
            {
                page.set_indicator_icon(Some(&icon));
                page.set_indicator_activatable(false);
            }
            return;
        }
        if let Some(page) = self.sessions.borrow().get(&session_id) {
            page.set_indicator_icon(gio::Icon::NONE);
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

        // Resolves the tab's content box, or the detached window's one for a
        // session that currently lives outside the main window.
        let Some(container) = self.session_content_box(session_id) else {
            return;
        };
        self.reconnect_shown.borrow_mut().insert(session_id);

        // Build the reconnect banner
        let banner = GtkBox::new(Orientation::Horizontal, 6);
        banner.set_margin_start(12);
        banner.set_margin_end(12);
        banner.set_margin_top(6);
        banner.set_margin_bottom(6);
        banner.set_halign(gtk4::Align::Center);
        banner.set_widget_name("reconnect-banner");

        let label = gtk4::Label::new(Some(&i18n("Session disconnected")));
        label.add_css_class("dim-label");

        banner.append(&label);

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

        tracing::info!(
            %session_id,
            protocol = %info.protocol,
            "Reconnect overlay shown for disconnected session"
        );
    }

    /// Updates the auto-reconnect status label in an existing reconnect banner
    pub fn update_reconnect_banner_status(&self, session_id: Uuid, active: bool) {
        let Some(container) = self.session_content_box(session_id) else {
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
        let Some(container) = self.session_content_box(session_id) else {
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

    /// Returns the session's reconnect banner, if one is shown.
    fn reconnect_banner(&self, session_id: Uuid) -> Option<GtkBox> {
        let container = self.session_content_box(session_id)?;
        let mut child = container.first_child();
        while let Some(widget) = child {
            if widget.widget_name() == "reconnect-banner" {
                return widget.downcast::<GtkBox>().ok();
            }
            child = widget.next_sibling();
        }
        None
    }

    /// Removes the reconnect banner and allows a later one to be shown.
    pub fn remove_reconnect_banner(&self, session_id: Uuid) {
        if let Some(container) = self.session_content_box(session_id) {
            let mut child = container.first_child();
            while let Some(widget) = child {
                let next = widget.next_sibling();
                if widget.widget_name() == "reconnect-banner" {
                    container.remove(&widget);
                }
                child = next;
            }
        }
        self.reconnect_shown.borrow_mut().remove(&session_id);
    }

    /// Adds a "Log In to <provider>" button to the session's reconnect banner.
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
            if widget.widget_name() == "cloud-login" {
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
        button.set_widget_name("cloud-login");
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

    /// Resolves the session whose terminal `Save Output` should dump for the
    /// right-clicked tab (#371).
    ///
    /// For a tab that hosts a split this is the focused pane's session (via
    /// [`Self::set_focused_session_provider`]); for a normal tab, or when the
    /// provider is unwired or reports no focused pane, it is the tab's own
    /// `owner` session.
    #[must_use]
    pub fn output_target_session(&self, owner: Uuid) -> Uuid {
        self.focused_session_provider
            .borrow()
            .as_ref()
            .and_then(|resolve| resolve(owner))
            .unwrap_or(owner)
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
    use super::session_has_home;

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
