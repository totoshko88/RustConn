//! Session reconnect, disconnect, and status management.
//!
//! Extracted from `terminal/mod.rs` to reduce module complexity.
//! Contains methods for handling session disconnect/reconnect lifecycle,
//! reconnect banners, VTE reset, and connection status tracking.

use super::*;

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
        // Check that the session still has a place to reconnect into: a tab, or
        // a detached window (issue #236) — the latter keeps the reconnected
        // session in the same window instead of falling back to close+create.
        let page = self.sessions.borrow().get(&session_id).cloned();
        if page.is_none() && !self.is_detached(session_id) {
            return false;
        }

        // Cancel any background polling (auto-reconnect)
        self.cancel_poll(session_id);

        // Remove the reconnect banner from wherever the session currently lives
        if let Some(container) = self.session_content_box(session_id) {
            // Find and remove the reconnect-banner widget
            let mut child = container.first_child();
            while let Some(widget) = child {
                let next = widget.next_sibling();
                if widget.widget_name() == "reconnect-banner" {
                    container.remove(&widget);
                }
                child = next;
            }
        }

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

        // Allow a new reconnect banner to be shown if this reconnect also fails
        self.reconnect_shown.borrow_mut().remove(&session_id);
        // The session is live again, so it becomes focusable by the smart
        // double-click once more (issue #242).
        self.disconnected_sessions.borrow_mut().remove(&session_id);

        // Remove stale automation session (will be re-created by the caller)
        self.automation_sessions.borrow_mut().remove(&session_id);

        // Remove stale highlight rules (will be re-applied by the caller)
        self.session_highlight_rules
            .borrow_mut()
            .remove(&session_id);

        // Remove stale highlight overlay (will be re-created by set_highlight_rules)
        self.highlight_overlays.borrow_mut().remove(&session_id);

        // Remove stale VTE child PID entry — the process should have already
        // exited (child-exited removes it), but if reconnect is triggered
        // before child-exited fires (e.g. timeout disconnect), we must clean
        // it to avoid killing a recycled PID later.
        self.vte_child_pids.borrow_mut().remove(&session_id);

        true
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

    // ========================================================================
    // Reconnect Callback Management
    // ========================================================================

    /// Sets the callback invoked when a reconnect button is clicked
    ///
    /// The callback receives `(session_id, connection_id)`.
    pub fn set_on_reconnect<F>(&self, callback: F)
    where
        F: Fn(Uuid, Uuid) + 'static,
    {
        *self.on_reconnect.borrow_mut() = Some(Box::new(callback));
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
