//! Tab creation, closing, parking, and reparenting.
//!
//! Extracted from `terminal/mod.rs` to reduce module complexity.
//! Contains methods for creating terminal/VNC/RDP/Web tabs and managing
//! their lifecycle (close, park, restore, reparent).

use super::*;

impl TerminalNotebook {
    // ========================================================================
    // Welcome Tab
    // ========================================================================

    /// Creates the welcome tab content — uses the full welcome screen with features.
    pub(super) fn create_welcome_tab() -> GtkBox {
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);

        // Use the full welcome content from SplitViewBridge for consistency
        let status_page = crate::split_view::SplitViewBridge::create_welcome_content_static();
        container.append(&status_page);
        container
    }

    /// Appends the Welcome tab to an empty `TabView`.
    ///
    /// Shared by both paths that can empty the tab bar: a normal tab close and
    /// parking the last tab into a detached window (issue #236). The caller
    /// checks the user preference and that no pages are left.
    pub(super) fn append_welcome_page(tab_view: &adw::TabView) {
        let welcome = Self::create_welcome_tab();
        let welcome_wrap = TabPageContainer::welcome(&welcome.upcast::<gtk4::Widget>());
        let welcome_page = tab_view.append(welcome_wrap.widget());
        welcome_page.set_title(&i18n("Welcome"));
        welcome_page.set_icon(Some(&gio::ThemedIcon::new("go-home-symbolic")));
    }

    /// Gets the icon name for a protocol.
    pub(super) fn get_protocol_icon(protocol: &str) -> &'static str {
        rustconn_core::get_protocol_icon_by_name(protocol)
    }

    /// Removes the welcome page if it exists.
    ///
    /// Called by every session/tab-creation path *after* the new real page has
    /// been appended **and selected**. The Welcome tab is a placeholder for an
    /// empty notebook (issue #232), so as soon as any real tab exists it must
    /// go. Ordering matters: `AdwTabView::close_page` on the currently-selected
    /// *sole* page defers its reselection to an idle callback, so closing
    /// Welcome *before* the real page was added left it stranded beside the
    /// first tab (`Welcome ⋮ fish1`) while the second tab — where Welcome was no
    /// longer selected — closed synchronously. Running this after the real page
    /// is appended and selected means Welcome is never the selected page at
    /// close time, so it closes immediately on the first tab too. We simply
    /// close any page titled Welcome, independent of the session count.
    pub(super) fn remove_welcome_page(&self) {
        if self.tab_view.n_pages() == 0 {
            return;
        }
        for i in 0..self.tab_view.n_pages() {
            let page = self.tab_view.nth_page(i);
            if page.title() == i18n("Welcome") {
                self.tab_view.close_page(&page);
                break;
            }
        }
    }

    /// Restores the Welcome page when the configured empty-notebook conditions hold.
    pub(super) fn ensure_welcome_page(&self) {
        if self.show_welcome.get()
            && self.sessions.borrow().is_empty()
            && self.tab_view.n_pages() == 0
        {
            Self::append_welcome_page(&self.tab_view);
        }
    }

    // ========================================================================
    // Tab Tooltip Helper
    // ========================================================================

    /// Builds a tab tooltip from a session title, its host, and its group.
    ///
    /// One place decides the layout — title, then the host line the embedded
    /// creation paths add, then the group line `set_tab_group` appends — so a tab
    /// recreated after a park or a rename is indistinguishable from the original
    /// (Requirement 2.3).
    pub(super) fn tab_tooltip(title: &str, host: Option<&str>, group: Option<&str>) -> String {
        use std::fmt::Write;

        let mut tooltip = title.to_owned();
        if let Some(host) = host.filter(|host| !host.is_empty()) {
            tooltip.push('\n');
            tooltip.push_str(host);
        }
        if let Some(group) = group {
            // Writing into a String never fails; the result is discarded the
            // same way the other string builders in the GUI do it.
            let _ = write!(tooltip, "\n[{group}]");
        }
        tooltip
    }

    // ========================================================================
    // Terminal Tab Creation
    // ========================================================================

    /// Creates a new terminal tab for an SSH session with default settings
    pub fn create_terminal_tab(
        &self,
        connection_id: Uuid,
        title: &str,
        protocol: &str,
        automation: Option<&AutomationConfig>,
    ) -> Uuid {
        self.create_terminal_tab_with_settings(
            connection_id,
            title,
            protocol,
            automation,
            &rustconn_core::config::TerminalSettings::default(),
            None,
            &[], // no variables for default tab
        )
    }

    /// Creates a new terminal tab with specific settings
    ///
    /// When `theme_override` is `Some`, the per-connection colors are applied
    /// on top of the global theme. When `None`, the global theme is used as-is.
    ///
    /// `global_variables` are used to substitute `${VAR}` references in
    /// Expect-rule responses before the automation session is created.
    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
    pub fn create_terminal_tab_with_settings(
        &self,
        connection_id: Uuid,
        title: &str,
        protocol: &str,
        automation: Option<&AutomationConfig>,
        settings: &rustconn_core::config::TerminalSettings,
        theme_override: Option<&rustconn_core::models::ConnectionThemeOverride>,
        global_variables: &[rustconn_core::Variable],
    ) -> Uuid {
        let session_id = Uuid::new_v4();

        let terminal = Terminal::new();
        terminal.set_hexpand(true);
        terminal.set_vexpand(true);

        // Focus-based accelerator suspend (#197): when the VTE gains focus,
        // single-Ctrl chords (Ctrl+F/P/N…) must reach the shell instead of the
        // app accelerators; restore them when focus leaves. The actual
        // suspend/restore (and the `terminal_passthrough_ctrl` setting) is
        // decided by the listener wired via `set_on_terminal_focus`.
        self.attach_focus_passthrough(&terminal);

        // Build a VariableManager for substituting ${VAR} in Expect responses
        let var_manager = {
            let mut mgr = rustconn_core::variables::VariableManager::new();
            for var in global_variables {
                mgr.set_global(var.clone());
            }
            mgr
        };

        // Setup automation if configured
        if let Some(cfg) = automation
            && !cfg.expect_rules.is_empty()
        {
            let rules = prepare_rules_from_config(&cfg.expect_rules, &var_manager);

            if !rules.is_empty() {
                let session = AutomationSession::new(terminal.clone(), rules);
                self.automation_sessions
                    .borrow_mut()
                    .insert(session_id, session);
            }
        }

        // Apply user settings
        config::configure_terminal_with_settings(&terminal, settings);

        // Apply per-connection theme override (if present) on top of the global theme
        if let Some(override_colors) = theme_override {
            let base_theme =
                TerminalTheme::resolve(&settings.color_theme, crate::app::system_is_dark());
            config::apply_theme_override_with_base(&terminal, override_colors, &base_theme);
        }

        // VTE implements GtkScrollable natively — no ScrolledWindow needed.
        // Wrapping in ScrolledWindow intercepts mouse events and breaks
        // ncurses apps (mc, htop) that rely on VTE's internal mouse handling.
        // Instead, pair VTE with a standalone GtkScrollbar connected to its
        // vadjustment — the same approach used by GNOME Terminal.
        let terminal_row = GtkBox::new(Orientation::Horizontal, 0);
        terminal_row.set_hexpand(true);
        terminal_row.set_vexpand(true);
        terminal_row.append(&terminal);

        if settings.show_scrollbar {
            let scrollbar =
                gtk4::Scrollbar::new(Orientation::Vertical, terminal.vadjustment().as_ref());
            terminal_row.append(&scrollbar);
        }

        // Wrap terminal_row in an Overlay so the highlight DrawingArea can
        // be layered on top without interfering with VTE input.
        let terminal_overlay = gtk4::Overlay::new();
        terminal_overlay.set_child(Some(&terminal_row));
        terminal_overlay.set_hexpand(true);
        terminal_overlay.set_vexpand(true);

        // Outer vertical container: terminal row on top, monitoring bar below.
        // get_session_container() returns this box so monitoring can append to it.
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        container.append(&terminal_overlay);

        // Right-click context menu actions installed on the terminal widget
        // so they follow it when reparented between TabView and split view.
        // When `right_click_pastes` is enabled (#349) this installs a paste
        // gesture instead of the native menu.
        config::setup_context_menu(
            &terminal,
            &self.snippet_menu_section,
            settings.right_click_pastes,
        );

        // Drag-and-drop: insert shell-escaped file paths when files are
        // dragged from a file manager onto the terminal (GNOME Terminal behavior).
        file_drop::setup_file_drop_target(&terminal);

        // Wrap in TabPageContainer to guarantee non-zero allocation for TabOverview
        let tab_container = TabPageContainer::single(&container);

        // Add page to TabView — child is the TabPageContainer outer box
        let page = self.tab_view.append(tab_container.widget());
        page.set_title(title);
        page.set_icon(Some(&gio::ThemedIcon::new(Self::get_protocol_icon(
            protocol,
        ))));
        page.set_tooltip(title);

        // Store session data
        self.sessions.borrow_mut().insert(session_id, page.clone());
        let terminal_for_focus = terminal.clone();
        self.terminals.borrow_mut().insert(session_id, terminal);
        self.terminal_overlays
            .borrow_mut()
            .insert(session_id, terminal_overlay);
        self.tab_containers
            .borrow_mut()
            .insert(session_id, tab_container);

        self.session_info.borrow_mut().insert(
            session_id,
            TerminalSession {
                id: session_id,
                connection_id,
                name: title.to_string(),
                protocol: protocol.to_string(),
                is_embedded: true,
                host: None,
                log_file: None,
                history_entry_id: None,
                tab_group: None,
                tab_color_index: None,
                connected_at: chrono::Utc::now(),
                reconnected_at: None,
            },
        );

        // Select the new page
        self.tab_view.set_selected_page(&page);

        // Drop the Welcome placeholder now that a real page exists and is
        // selected (see `remove_welcome_page`: doing this after the append is
        // what makes it fire on the very first tab, not only the second).
        self.remove_welcome_page();

        // Auto-focus the terminal so the user can type immediately (#79).
        // Use idle_add_local_once so the focus request runs after the page
        // is fully mapped, and only if this page is still selected (avoids
        // focus-stealing when multiple tabs open in quick succession).
        let tab_view_focus = self.tab_view.clone();
        let page_focus = page.clone();
        let terminal_focus = terminal_for_focus;
        glib::idle_add_local_once(move || {
            if tab_view_focus.selected_page().as_ref() == Some(&page_focus) {
                terminal_focus.grab_focus();
            }
        });

        // Apply protocol color indicator if enabled
        if *self.color_tabs_by_protocol.borrow() {
            self.apply_protocol_color(session_id, protocol);
        }

        // Notify listeners that a new terminal session was created.
        // Single choke point for per-session wiring (activity monitoring):
        // fires for every terminal protocol and for both synchronous and
        // async (port-checked) connection paths, regardless of which connect
        // action started the session.
        if let Some(ref callback) = *self.on_session_created.borrow() {
            callback(session_id, connection_id);
        }

        self.notify_tab_added(session_id, connection_id);

        session_id
    }

    // ========================================================================
    // VNC Tab Creation
    // ========================================================================

    /// Creates a new VNC session tab
    pub fn create_vnc_session_tab(&self, connection_id: Uuid, title: &str) -> Uuid {
        self.create_vnc_session_tab_with_host(connection_id, title, "")
    }

    /// Creates a new VNC session tab with host information
    pub fn create_vnc_session_tab_with_host(
        &self,
        connection_id: Uuid,
        title: &str,
        host: &str,
    ) -> Uuid {
        let session_id = Uuid::new_v4();

        let vnc_widget = Rc::new(VncSessionWidget::new());

        // #197: suspend single-Ctrl accelerators while the viewer has focus.
        self.attach_focus_passthrough(vnc_widget.widget());
        // #356: in passthrough, desktop shortcuts go to the remote desktop too.
        self.attach_shortcut_inhibit(vnc_widget.widget());

        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        container.append(vnc_widget.widget());

        let tab_container = TabPageContainer::single(&container);
        let page = self.tab_view.append(tab_container.widget());
        page.set_title(title);
        page.set_icon(Some(&gio::ThemedIcon::new(
            "video-joined-displays-symbolic",
        )));
        // The host is stored on the session below, so a tab rebuilt after a
        // detach or a rename produces this very tooltip again.
        let session_host = (!host.is_empty()).then(|| host.to_owned());
        page.set_tooltip(&Self::tab_tooltip(title, session_host.as_deref(), None));

        self.sessions.borrow_mut().insert(session_id, page.clone());
        // Register the container so split (switch_tab_to_split) and unsplit /
        // close-pane (reparent_terminal_to_tab) can swap this tab's content.
        self.tab_containers
            .borrow_mut()
            .insert(session_id, tab_container);
        self.session_widgets
            .borrow_mut()
            .insert(session_id, SessionWidgetStorage::Vnc(vnc_widget));

        self.session_info.borrow_mut().insert(
            session_id,
            TerminalSession {
                id: session_id,
                connection_id,
                name: title.to_string(),
                protocol: "vnc".to_string(),
                is_embedded: true,
                host: session_host,
                log_file: None,
                history_entry_id: None,
                tab_group: None,
                tab_color_index: None,
                connected_at: chrono::Utc::now(),
                reconnected_at: None,
            },
        );

        self.tab_view.set_selected_page(&page);
        // Drop the Welcome placeholder after the real page is selected (see
        // `remove_welcome_page`) so it clears on the first tab, not the second.
        self.remove_welcome_page();
        // Apply protocol color indicator if enabled
        if *self.color_tabs_by_protocol.borrow() {
            self.apply_protocol_color(session_id, "vnc");
        }
        self.notify_tab_added(session_id, connection_id);
        session_id
    }

    // ========================================================================
    // Embedded Session Tab Creation (RDP, Web, External)
    // ========================================================================

    /// Adds an embedded RDP tab with the EmbeddedRdpWidget
    pub fn add_embedded_rdp_tab(
        &self,
        session_id: Uuid,
        connection_id: Uuid,
        title: &str,
        widget: Rc<EmbeddedRdpWidget>,
    ) {
        // #197: suspend single-Ctrl accelerators while the viewer has focus.
        self.attach_focus_passthrough(widget.widget());
        // #356: in passthrough, desktop shortcuts go to the remote desktop too.
        self.attach_shortcut_inhibit(widget.widget());

        // Wrap in ToastOverlay for file DnD notifications
        let toast_overlay = libadwaita::ToastOverlay::new();
        toast_overlay.set_child(Some(widget.widget()));
        toast_overlay.set_hexpand(true);
        toast_overlay.set_vexpand(true);
        widget.set_toast_overlay(toast_overlay.clone());

        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        container.append(&toast_overlay);

        let tab_container = TabPageContainer::single(&container);
        let page = self.tab_view.append(tab_container.widget());
        page.set_title(title);
        page.set_icon(Some(&gio::ThemedIcon::new("computer-symbolic")));
        page.set_tooltip(title);

        self.sessions.borrow_mut().insert(session_id, page.clone());
        // Register the container so split (switch_tab_to_split) and unsplit /
        // close-pane (reparent_terminal_to_tab) can swap this tab's content.
        self.tab_containers
            .borrow_mut()
            .insert(session_id, tab_container);
        self.session_widgets
            .borrow_mut()
            .insert(session_id, SessionWidgetStorage::EmbeddedRdp(widget));

        self.session_info.borrow_mut().insert(
            session_id,
            TerminalSession {
                id: session_id,
                connection_id,
                name: title.to_string(),
                protocol: "rdp".to_string(),
                is_embedded: true,
                host: None,
                log_file: None,
                history_entry_id: None,
                tab_group: None,
                tab_color_index: None,
                connected_at: chrono::Utc::now(),
                reconnected_at: None,
            },
        );

        self.tab_view.set_selected_page(&page);
        // Drop the Welcome placeholder after the real page is selected (see
        // `remove_welcome_page`) so it clears on the first tab, not the second.
        self.remove_welcome_page();
        // Apply protocol color indicator if enabled
        if *self.color_tabs_by_protocol.borrow() {
            self.apply_protocol_color(session_id, "rdp");
        }
        self.notify_tab_added(session_id, connection_id);
    }

    /// Adds an embedded Web browser tab with the `EmbeddedWebWidget`.
    ///
    /// Creates a new tab page, stores the widget as
    /// `SessionWidgetStorage::EmbeddedWeb`, and selects the page.
    #[cfg(feature = "web-embedded")]
    pub fn add_embedded_web_tab(
        &self,
        session_id: Uuid,
        connection_id: Uuid,
        title: &str,
        widget: Rc<crate::embedded_web::EmbeddedWebWidget>,
    ) {
        // #197: suspend single-Ctrl accelerators while the viewer has focus.
        self.attach_focus_passthrough(widget.widget());

        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        container.append(widget.widget());

        let tab_container = TabPageContainer::single(&container);
        let page = self.tab_view.append(tab_container.widget());
        page.set_title(title);
        page.set_icon(Some(&gio::ThemedIcon::new("web-browser-symbolic")));
        page.set_tooltip(title);

        self.sessions.borrow_mut().insert(session_id, page.clone());
        self.tab_containers
            .borrow_mut()
            .insert(session_id, tab_container);
        self.session_widgets
            .borrow_mut()
            .insert(session_id, SessionWidgetStorage::EmbeddedWeb(widget));

        self.session_info.borrow_mut().insert(
            session_id,
            TerminalSession {
                id: session_id,
                connection_id,
                name: title.to_string(),
                protocol: "web".to_string(),
                is_embedded: true,
                host: None,
                log_file: None,
                history_entry_id: None,
                tab_group: None,
                tab_color_index: None,
                connected_at: chrono::Utc::now(),
                reconnected_at: None,
            },
        );

        self.tab_view.set_selected_page(&page);
        // Drop the Welcome placeholder after the real page is selected (see
        // `remove_welcome_page`) so it clears on the first tab, not the second.
        self.remove_welcome_page();
        // Apply protocol color indicator if enabled
        if *self.color_tabs_by_protocol.borrow() {
            self.apply_protocol_color(session_id, "web");
        }
        self.notify_tab_added(session_id, connection_id);
    }

    /// Adds an embedded session tab (for RDP/VNC external processes)
    pub fn add_embedded_session_tab(
        &self,
        session_id: Uuid,
        connection_id: Uuid,
        title: &str,
        protocol: &str,
        widget: &GtkBox,
        process: Option<Rc<RefCell<Option<std::process::Child>>>>,
    ) {
        let tab_container = TabPageContainer::single(widget);
        let page = self.tab_view.append(tab_container.widget());
        page.set_title(title);
        page.set_icon(Some(&gio::ThemedIcon::new(Self::get_protocol_icon(
            protocol,
        ))));
        page.set_tooltip(title);

        self.sessions.borrow_mut().insert(session_id, page.clone());

        // Store external process for cleanup on tab close
        if let Some(proc) = process {
            self.session_widgets
                .borrow_mut()
                .insert(session_id, SessionWidgetStorage::ExternalProcess(proc));
        }

        self.session_info.borrow_mut().insert(
            session_id,
            TerminalSession {
                id: session_id,
                connection_id,
                name: title.to_string(),
                protocol: protocol.to_string(),
                is_embedded: false,
                host: None,
                log_file: None,
                history_entry_id: None,
                tab_group: None,
                tab_color_index: None,
                connected_at: chrono::Utc::now(),
                reconnected_at: None,
            },
        );

        self.tab_view.set_selected_page(&page);
        // Drop the Welcome placeholder after the real page is selected (see
        // `remove_welcome_page`) so it clears on the first tab, not the second.
        self.remove_welcome_page();
        // Apply protocol color indicator if enabled
        if *self.color_tabs_by_protocol.borrow() {
            self.apply_protocol_color(session_id, protocol);
        }
        self.notify_tab_added(session_id, connection_id);
    }
}

impl TerminalNotebook {
    // ========================================================================
    // Tab Parking (Split View Support)
    // ========================================================================

    /// Parks (temporarily removes) a session's tab when entering a split pane.
    ///
    /// Closing the tab page frees its place in the tab bar while the session
    /// (its live widget, metadata, terminal/viewer backing, connection state,
    /// history, monitoring state) stays alive. Keeping split guests out of the
    /// tab bar and Tab Overview avoids redundant placeholder tabs. The tab is
    /// recreated by [`Self::restore_session_tab`] when the session leaves the
    /// split. No-op if the session has no tab (already parked).
    pub fn park_session_tab(&self, session_id: Uuid) {
        // A session that is already parked for another reason lives somewhere
        // else entirely — a detached window today. Silently doing nothing would
        // leave it marked detached while its widget moved into a split, so
        // refuse loudly instead (issue #236).
        if self.is_detached(session_id) {
            tracing::warn!(
                session = %session_id,
                "refusing to park a session that lives in a detached window"
            );
            return;
        }
        if !self.sessions.borrow().contains_key(&session_id) {
            tracing::debug!(session = %session_id, "park skipped: session has no tab");
            return;
        }
        // Mark before closing so the close-page handler skips teardown and only
        // removes the tab page (see `setup_tab_view_signals`).
        self.parked_in_split.borrow_mut().insert(session_id);
        if !self.park_tab_page(session_id) {
            // Mirror `take_session_content`: an un-parkable session must not be
            // left marked as parked.
            self.parked_in_split.borrow_mut().remove(&session_id);
            tracing::warn!(session = %session_id, "park failed: no tab page to close");
        }
    }

    /// Closes a session's tab page without running session teardown.
    ///
    /// The shared half of parking: the caller must have already marked the
    /// session in one of the park sets (`parked_in_split` today) so the
    /// `close-page` handler drops only the page and its container mapping.
    /// Returns `false` when the session has no tab page to close.
    pub(super) fn park_tab_page(&self, session_id: Uuid) -> bool {
        let Some(page) = self.sessions.borrow().get(&session_id).cloned() else {
            return false;
        };
        self.tab_view.close_page(&page);
        true
    }

    /// Reports whether a session is currently parked for any reason.
    ///
    /// Read-only counterpart of [`Self::clear_park_marks`], so a caller can
    /// validate before it changes any state.
    pub(super) fn is_parked(&self, session_id: Uuid) -> bool {
        self.parked_in_split.borrow().contains(&session_id)
            || self.detached.borrow().contains(&session_id)
    }

    /// Clears every park marker for a session, returning whether one was set.
    ///
    /// The set arithmetic lives in [`detach::take_park_mark`] so it can be
    /// checked without a display; this method only hands it the two live sets.
    pub(super) fn clear_park_marks(&self, session_id: Uuid) -> bool {
        detach::take_park_mark(
            &mut self.parked_in_split.borrow_mut(),
            &mut self.detached.borrow_mut(),
            session_id,
        )
        .is_some()
    }

    // ========================================================================
    // Tab Restore
    // ========================================================================

    /// Recreates the standalone tab for a session that was parked by
    /// [`Self::park_session_tab`], so its widget has a home again after it
    /// leaves the split. No-op if the session was not parked.
    ///
    /// The fresh tab starts with an empty single-mode container; the caller's
    /// subsequent [`Self::reparent_terminal_to_tab`] moves the live widget in.
    pub(crate) fn restore_session_tab(&self, session_id: Uuid) -> bool {
        if !self.is_parked(session_id) {
            return false;
        }
        // Resolve the metadata *before* touching the park marks: a session
        // without metadata would otherwise lose its mark and gain no tab, which
        // leaves it in no placement at all (issue #236).
        let Some((title, protocol, group, host)) =
            self.session_info.borrow().get(&session_id).map(|info| {
                (
                    info.name.clone(),
                    info.protocol.clone(),
                    info.tab_group.clone(),
                    info.host.clone(),
                )
            })
        else {
            tracing::warn!(
                session = %session_id,
                "cannot restore a tab for a session without metadata; park mark kept"
            );
            return false;
        };

        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        let tab_container = TabPageContainer::single(&container);
        let page = self.tab_view.append(tab_container.widget());
        // A grouped tab keeps its `[group] ` prefix. This used to set the bare
        // name, so a tab that came back from a split pane silently lost its group
        // label — visible only in the tooltip — while still being a member of the
        // group for every operation. Nothing else re-applies it on this path.
        page.set_title(&tab_title(&title, group.as_deref()));
        page.set_icon(Some(&gio::ThemedIcon::new(Self::get_protocol_icon(
            &protocol,
        ))));
        // The creation path sets a tooltip too, and a grouped tab carries the
        // group name as a second line (Requirement 2.3).
        page.set_tooltip(&Self::tab_tooltip(
            &title,
            host.as_deref(),
            group.as_deref(),
        ));

        self.sessions.borrow_mut().insert(session_id, page);
        self.tab_containers
            .borrow_mut()
            .insert(session_id, tab_container);
        // The session has a home again, so the park mark may go.
        self.clear_park_marks(session_id);
        true
    }

    /// Closes (terminates) a session by id, running the standard tab-close
    /// teardown regardless of whether it currently has a standalone tab.
    ///
    /// A split guest has no tab, so its tab is recreated first (unselected, so
    /// the user sees no content switch) and then closed — the `close-page`
    /// handler disconnects the live widget and kills the child process via the
    /// session maps, which hold the widget wherever it currently lives. The
    /// caller is responsible for having removed the session's split panel first
    /// (e.g. via `close_pane`); `on_split_cleanup` clears any remaining split
    /// membership as part of the close.
    pub fn close_session(&self, session_id: Uuid) {
        self.restore_session_tab(session_id);
        let page = self.sessions.borrow().get(&session_id).cloned();
        if let Some(page) = page {
            self.tab_view.close_page(&page);
        }
    }

    // ========================================================================
    // Widget Reparenting
    // ========================================================================

    /// Used by the split close-pane / unsplit paths: when a session leaves a
    /// split panel, the *same* widget instance is reparented back into its
    /// single-session tab. For an embedded RDP/VNC/SPICE viewer this moves the
    /// live viewer widget (never disconnecting or recreating the connection);
    /// for a VTE session it rebuilds the terminal + scrollbar layout.
    pub fn reparent_terminal_to_tab(&self, session_id: Uuid) {
        // Option B: a split guest has no standalone tab (it was parked by
        // `park_session_tab`). Recreate the tab first so the widget has a home;
        // no-op for a session that still has its tab.
        self.restore_session_tab(session_id);

        // Rebuild a fresh single-session content box around the live widget and
        // switch TabPageContainer back to single mode. This correctly handles the
        // case where the tab was previously in split mode (TabPageContainer
        // contained the split bridge widget).
        let Some(content) = self.build_session_content(session_id) else {
            return;
        };

        let mut containers = self.tab_containers.borrow_mut();
        if let Some(tab_container) = containers.get_mut(&session_id) {
            tab_container.switch_to_single(&content);
        }
    }

    /// Builds a fresh single-session content box around a session's live widget.
    ///
    /// The widget instance is unparented from wherever it currently lives (split
    /// panel, tab container) and rewrapped exactly as the creation path does, so
    /// every caller ends up with an identical layout: a VTE terminal goes into a
    /// horizontal `terminal_row` inside a `gtk4::Overlay` (re-registered in
    /// `terminal_overlays` for highlight support), an embedded viewer is
    /// appended directly. The live protocol connection is never touched.
    ///
    /// Returns `None` when the session has neither a terminal nor an embedded
    /// widget, in which case nothing was moved.
    pub(super) fn build_session_content(&self, session_id: Uuid) -> Option<GtkBox> {
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);

        // Embedded viewers have no VTE terminal — they travel as their own
        // widget instance (carrying their in-container toolbar and reconnect
        // banner). Handle them first; fall through to the VTE path otherwise.
        if self.append_embedded_content(session_id, &container) {
            return Some(container);
        }

        let Some(terminal) = self.terminals.borrow().get(&session_id).cloned() else {
            tracing::warn!(
                session = %session_id,
                "no live widget for session, cannot build content"
            );
            return None;
        };

        // Remove terminal from current parent (split pane wrapper, etc.)
        Self::detach_widget_from_parent(terminal.upcast_ref());

        // Re-wrap terminal with scrollbar (matching create_terminal_tab_with_settings layout)
        let terminal_row = GtkBox::new(Orientation::Horizontal, 0);
        terminal_row.set_hexpand(true);
        terminal_row.set_vexpand(true);
        terminal_row.append(&terminal);

        // Re-create overlay for highlight support
        let terminal_overlay = gtk4::Overlay::new();
        terminal_overlay.set_child(Some(&terminal_row));
        terminal_overlay.set_hexpand(true);
        terminal_overlay.set_vexpand(true);
        container.append(&terminal_overlay);

        // Update terminal overlay tracking
        self.terminal_overlays
            .borrow_mut()
            .insert(session_id, terminal_overlay);

        terminal.set_visible(true);

        Some(container)
    }

    /// Appends a session's embedded viewer widget into a fresh content box.
    ///
    /// Returns `true` when `session_id` is an embedded RDP/VNC/Web viewer and
    /// was handled; `false` when it is not embedded (the caller then falls back
    /// to the VTE terminal path). The same widget instance is moved, so the
    /// live protocol connection is preserved — nothing is disconnected.
    pub(super) fn append_embedded_content(&self, session_id: Uuid, container: &GtkBox) -> bool {
        // Resolve the concrete widget while scoping the borrow, so no
        // `session_widgets` borrow is held across GTK reparenting.
        enum Embedded {
            Vnc(Rc<VncSessionWidget>),
            Rdp(Rc<EmbeddedRdpWidget>),
            #[cfg(feature = "web-embedded")]
            Web(Rc<crate::embedded_web::EmbeddedWebWidget>),
        }
        let embedded = {
            let widgets = self.session_widgets.borrow();
            match widgets.get(&session_id) {
                Some(SessionWidgetStorage::Vnc(w)) => Embedded::Vnc(Rc::clone(w)),
                Some(SessionWidgetStorage::EmbeddedRdp(w)) => Embedded::Rdp(Rc::clone(w)),
                #[cfg(feature = "web-embedded")]
                Some(SessionWidgetStorage::EmbeddedWeb(w)) => Embedded::Web(Rc::clone(w)),
                _ => return false,
            }
        };

        // Mirror the creation path so the embedded widget is wrapped exactly as
        // when its tab was first built.
        match embedded {
            Embedded::Vnc(w) => {
                let widget = w.widget();
                Self::detach_widget_from_parent(widget);
                widget.set_hexpand(true);
                widget.set_vexpand(true);
                container.append(widget);
                widget.set_visible(true);
            }
            Embedded::Rdp(w) => {
                let widget = w.widget();
                Self::detach_widget_from_parent(widget.upcast_ref());
                // Append the RDP container directly (mirroring the VNC
                // arm). Wrapping it in a freshly-created `adw::ToastOverlay`
                // here left the reparented `DrawingArea` unable to repaint (its
                // draw func was never re-invoked, so live frames landed in the
                // buffer but never reached the screen — a blank viewer). The
                // file-drop ToastOverlay is only needed while DnD is active and
                // is re-established elsewhere; a plain re-parent restores drawing.
                widget.set_hexpand(true);
                widget.set_vexpand(true);
                container.append(widget);
                widget.set_visible(true);
            }
            #[cfg(feature = "web-embedded")]
            Embedded::Web(w) => {
                let widget = w.widget();
                Self::detach_widget_from_parent(widget.upcast_ref());
                widget.set_hexpand(true);
                widget.set_vexpand(true);
                container.append(widget);
                widget.set_visible(true);
            }
        }

        // Nudge a repaint once the re-parented viewer has settled into its new
        // allocation (the live frame lives in a Rust-side buffer, not GTK's
        // surface cache). The idle runs after the caller has placed the content,
        // so the queue_draw hits the final allocation.
        let content = container.clone();
        glib::idle_add_local_once(move || {
            content.queue_draw();
        });
        true
    }

    /// Detaches a widget from its current parent so the same instance can be
    /// re-attached elsewhere (GTK widgets may only have one parent).
    ///
    /// A `GtkBox` parent uses `remove`; any other parent uses `unparent`.
    pub(super) fn detach_widget_from_parent(widget: &Widget) {
        if let Some(parent) = widget.parent() {
            if let Some(box_widget) = parent.downcast_ref::<GtkBox>() {
                box_widget.remove(widget);
            } else {
                widget.unparent();
            }
        }
    }
}
