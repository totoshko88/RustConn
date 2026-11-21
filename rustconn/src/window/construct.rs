//! Construction of the main window (`MainWindow::new`)

use super::*;

impl MainWindow {
    /// Creates a new main window for the application
    #[must_use]
    pub fn new(app: &adw::Application, state: SharedAppState) -> Self {
        // Register custom icon from assets before creating window
        Self::register_app_icon();

        // Create the main window
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("RustConn")
            .default_width(1200)
            .default_height(800)
            // Initial minimum-width floor; refined to the exact narrow-tier
            // header width (incl. window controls) by the runtime measurement in
            // the connect_map handler below (#204). With breakpoints present,
            // AdwApplicationWindow derives its minimum from width-request.
            .width_request(400)
            // HIG adaptive floor is 360x294 (phone portrait). The width floor
            // is refined down to the measured narrow-tier minimum (>=360) by the
            // connect_map handler below (#204); the height floor is set to 294
            // here so the window can shrink to phone-portrait height.
            .height_request(294)
            .icon_name("io.github.totoshko88.RustConn")
            .build();

        // macOS: tag the window so platform-specific CSS rules (reduced
        // headerbar height, tighter button padding) engage unconditionally.
        #[cfg(target_os = "macos")]
        window.add_css_class("macos");

        // Apply saved window geometry if available
        with_state(&state, |state_ref| {
            let settings = state_ref.settings();
            if settings.ui.remember_window_geometry {
                if let (Some(width), Some(height)) =
                    (settings.ui.window_width, settings.ui.window_height)
                    && width > 0
                    && height > 0
                {
                    window.set_default_size(width, height);
                }
                // Restore maximized state (#202). GTK keeps the unmaximized
                // default size above, so unmaximizing later restores it.
                if settings.ui.window_maximized {
                    window.maximize();
                }
            }
        });

        // Create header bar with busy spinner
        let (
            header_bar,
            busy_spinner,
            passthrough_indicator,
            broadcast_toggle,
            group_broadcast_toggle,
            menu_button,
            header_title,
        ) = ui::create_header_bar();

        // Mirror the window title into the header bar's centre label so the
        // active connection name (issue #211) is visible there too, not only in
        // the WM title read by time-tracking tools. One-way binding: whenever
        // `update_window_title` sets the window title, the label follows.
        window
            .bind_property("title", &header_title, "title")
            .sync_create()
            .build();

        // Create BusyStack that shows/hides the header bar spinner.
        // GTK widgets are !Send, so we bridge via std::sync::mpsc channel.
        // The BusyStack callback (Send+Sync) sends a bool, and a
        // glib::idle_add_local receiver dispatches it on the main thread.
        let (busy_tx, busy_rx) = std::sync::mpsc::channel::<bool>();
        let busy_stack = rustconn_core::BusyStack::new(move |busy| {
            let _ = busy_tx.send(busy);
            // Wake the main loop so it processes the message promptly
            let ctx = glib::MainContext::ref_thread_default();
            ctx.wakeup();
        });
        {
            let spinner = busy_spinner;
            let rx = std::sync::Mutex::new(busy_rx);
            glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                if let Ok(guard) = rx.lock() {
                    // Drain all pending messages, apply the latest state
                    let mut latest = None;
                    while let Ok(busy) = guard.try_recv() {
                        latest = Some(busy);
                    }
                    if let Some(busy) = latest {
                        // The spinner animates whenever it is shown, on both
                        // the libadwaita 1.6 and the 1.5 path — see
                        // `crate::spinner`.
                        spinner.set_visible(busy);
                    }
                }
                glib::ControlFlow::Continue
            });
        }

        // Create the main layout with OverlaySplitView (GNOME HIG)
        let overlay_split_view = adw::OverlaySplitView::new();

        // One sidebar width (issue #3): the saved value if it lies in 180..=500,
        // else the 320 default — values > 500 date from the old 360 px minimum.
        let sidebar_width = with_state(&state, |s| s.settings().ui.effective_sidebar_width());
        overlay_split_view.set_max_sidebar_width(f64::from(sidebar_width));
        overlay_split_view
            .set_min_sidebar_width(f64::from(rustconn_core::config::SIDEBAR_WIDTH_MIN));
        // High fraction so the configured sidebar width (max-sidebar-width) is the
        // effective width whenever the sidebar is shown, instead of being capped
        // below the setting by a small proportion (#204: "sidebar width not
        // adjustable"). The sidebar is only shown at ≥ 820 sp, where
        // 0.7 × width already exceeds the 500 sp maximum, so max wins.
        overlay_split_view.set_sidebar_width_fraction(0.7);
        overlay_split_view.set_enable_show_gesture(true);
        overlay_split_view.set_enable_hide_gesture(true);
        overlay_split_view.set_pin_sidebar(true);

        // Create sidebar
        let sidebar = Rc::new(ConnectionSidebar::new());
        // Wrap the sidebar in the opt-in drag-resize handle (issue #3). The
        // handle is hidden entirely while the setting is off or the split view
        // is collapsed, so the default layout is unchanged.
        let sidebar_wrapper =
            sidebar_resize::wrap_with_handle(sidebar.widget(), &overlay_split_view, &state);
        overlay_split_view.set_sidebar(Some(&sidebar_wrapper));

        // Load persisted search history
        with_state(&state, |s| {
            sidebar.load_search_history(&s.settings().ui.search_history);
            sidebar.load_saved_searches(&s.settings().ui.saved_searches);
        });
        {
            let state_for_saved = state.clone();
            sidebar.connect_saved_searches_changed(move |items| {
                if let Ok(mut state_mut) = state_for_saved.try_borrow_mut() {
                    state_mut.settings_mut().ui.saved_searches = items.to_vec();
                    if let Err(e) = state_mut.save_settings() {
                        tracing::warn!(?e, "Failed to save settings");
                    }
                } else {
                    tracing::warn!("State busy; the saved searches change was not stored");
                }
            });
        }

        // Create global color pool shared across all split containers
        // This ensures different split containers get different colors
        let global_color_pool: SharedColorPool = Rc::new(RefCell::new(ColorPool::new()));

        // Cross-tab broadcast group state (issue #329). Created here so a clone
        // can be captured by the page-closed handler below to forget a closed
        // session, before the window struct that owns it is built.
        let group_broadcast = group_broadcast::GroupBroadcast::new();

        // Create split terminal view as the main terminal container
        // Uses the global color pool for consistent color allocation
        let mut split_bridge = SplitViewBridge::with_color_pool(Rc::clone(&global_color_pool));
        with_state(&state, |s| {
            split_bridge.set_show_scrollbar(s.settings().terminal.show_scrollbar);
        });
        let split_view = Rc::new(split_bridge);

        // Create per-session split bridges map
        // Each tab maintains its own independent split layout
        let session_split_bridges: SessionSplitBridges =
            Rc::new(RefCell::new(std::collections::HashMap::new()));

        // Create container for split views - we swap which bridge is visible based on active session
        let split_container = gtk4::Box::new(Orientation::Vertical, 0);
        split_container.set_vexpand(true);
        split_container.set_hexpand(true);

        // Create terminal notebook for tab management (using adw::TabView)
        // Show Welcome tab only when the user hasn't disabled it AND no startup
        // action will immediately open a session (issue #232).
        let show_welcome = if crate::has_cli_startup_override() {
            // CLI --shell / --connect will open a session immediately
            false
        } else {
            state
                .try_borrow()
                .ok()
                .map(|s| {
                    let ui = &s.settings().ui;
                    ui.show_welcome_on_startup
                        && ui.startup_action == rustconn_core::config::StartupAction::None
                })
                .unwrap_or(true)
        };
        let terminal_notebook = Rc::new(TerminalNotebook::new(show_welcome));
        let auto_capture_shortcuts = with_state(&state, |s| {
            s.settings().ui.remote_desktop_auto_capture_shortcuts
        });
        terminal_notebook.set_auto_capture_shortcuts(auto_capture_shortcuts);

        // Refresh VTE font state when fontconfig changes (#171).
        // VTE reads `gtk-fontconfig-timestamp` only when it creates its
        // cached FontInfo and never subscribes to changes, so after a
        // fontconfig update (font install, fc-cache, KDE pushing
        // Fontconfig/Timestamp via XSettings on screen unlock) terminals
        // keep stale Pango font references and crash with SIGSEGV in
        // pango_itemize during the next snapshot.
        if let Some(gtk_settings) = gtk4::Settings::default() {
            let notebook_weak = Rc::downgrade(&terminal_notebook);
            gtk_settings.connect_notify_local(Some("gtk-fontconfig-timestamp"), move |_, _| {
                if let Some(notebook) = notebook_weak.upgrade() {
                    tracing::info!(
                        "fontconfig timestamp changed; refreshing fonts on all terminals"
                    );
                    notebook.refresh_fonts_after_fontconfig_change();
                }
            });
        }

        // Repaint terminals when the desktop flips light/dark.
        //
        // `AdwStyleManager` already drives the GTK chrome; what it does not touch is
        // VTE's palette, which RustConn sets by name. With the "Follow System" theme
        // the resolved palette changes while `settings.toml` does not, so no other
        // path repaints — the chrome went light and the terminal stayed dark.
        //
        // Weak on both handles, mirroring the fontconfig handler above: this lives on
        // the process-wide StyleManager, which outlives every window, so a strong
        // reference here would leak the window and its sessions.
        {
            let notebook_weak = Rc::downgrade(&terminal_notebook);
            let state_weak = Rc::downgrade(&state);
            adw::StyleManager::default().connect_dark_notify(move |style_manager| {
                let Some(notebook) = notebook_weak.upgrade() else {
                    return;
                };
                let Some(state) = state_weak.upgrade() else {
                    return;
                };
                // try_borrow, not borrow: this fires from a settings-portal signal
                // that can land while another handler already holds the state.
                let Ok(state_ref) = state.try_borrow() else {
                    tracing::debug!("state busy on color-scheme change; skipping repaint");
                    return;
                };
                let theme_name = state_ref.settings().terminal.color_theme.clone();
                if theme_name != rustconn_core::terminal_themes::FOLLOW_SYSTEM_THEME {
                    // A theme chosen by name is meant to stay put.
                    return;
                }
                tracing::debug!(
                    dark = style_manager.is_dark(),
                    "desktop color scheme changed; repainting terminals"
                );
                notebook.reapply_colors(&theme_name, |connection_id| {
                    state_ref
                        .get_connection(connection_id)
                        .and_then(|c| c.theme_override.clone())
                });
            });
        }

        // Apply initial reconnect-history and sidebar settings
        if let Ok(state_ref) = state.try_borrow() {
            terminal_notebook.set_keep_history_on_reconnect(
                state_ref.settings().terminal.keep_history_on_reconnect,
            );
            terminal_notebook.set_max_scrollback_on_reconnect(
                state_ref.settings().terminal.max_scrollback_on_reconnect,
            );
            sidebar.set_smart_folders_visible(state_ref.settings().ui.show_smart_folders);
            terminal_notebook
                .set_show_directional_close(state_ref.settings().ui.show_directional_tab_close);
        }

        // Set up callback for when SSH tabs are closed via TabView
        // This ensures sidebar status is cleared when tabs are closed
        // Note: Split view cleanup is handled in connect_signals() where we have access to session_bridges
        let sidebar_for_close = sidebar.clone();
        let monitoring = Rc::new(MonitoringCoordinator::new());
        let monitoring_for_close = monitoring.clone();

        // Publish script-runner handles. `.rsc` scripts run from the launch
        // path and the scripts manager, which do not thread every handle
        // through, so they fetch these from a thread-local.
        crate::script_runner::publish_script_runtime(crate::script_runner::ScriptRuntime::new(
            session_split_bridges.clone(),
            split_view.clone(),
            window.downgrade(),
            sidebar.clone(),
            monitoring.clone(),
        ));
        // Write the bundled example scripts on first run, so the auto-run
        // pickers list them before the Scripts manager has ever been opened.
        crate::dialogs::script::ensure_bundled_examples();
        let activity_coordinator = Rc::new(ActivityCoordinator::new());
        let activity_for_close = activity_coordinator.clone();
        terminal_notebook.set_activity_coordinator(activity_coordinator.clone());
        // The detach/attach paths suspend and resume the monitoring bar around
        // the widget move, mirroring the split path.
        terminal_notebook.set_monitoring_coordinator(Rc::clone(&monitoring));
        let group_broadcast_for_close = group_broadcast.clone();
        // Auto-reveal-on-last-close (issue #401). Captured from the locals before
        // the struct literal moves them: the split view and app state to read the
        // live setting, and a weak notebook handle for the live session counts.
        // The external-viewer count comes from the thread-local registry accessor
        // (`external_session_registry`), which is published after construction and
        // is populated by the time any close fires.
        let split_for_close = overlay_split_view.clone();
        let state_for_close = state.clone();
        let notebook_for_autohide_close = Rc::downgrade(&terminal_notebook);
        terminal_notebook.set_on_page_closed(move |session_id, connection_id| {
            monitoring_for_close.stop_monitoring(session_id);
            activity_for_close.stop(session_id);
            // A script still running on the closed tab would keep its timer and
            // its `ACTIVE_SCRIPTS` entry alive against a terminal that is gone.
            crate::script_runner::cancel_script_for_session(session_id);
            // Drop the closed session from the broadcast set so a reused UUID
            // can never inherit its membership, and the wired set stays bounded.
            group_broadcast_for_close.forget(session_id);
            sidebar_for_close.decrement_session_count(
                &connection_id.to_string(),
                session_id,
                false,
            );

            // Read the setting into a local FIRST — no borrow held across the GTK
            // call (NFR3). At this point the closing session is STILL in the
            // notebook's map (remove() runs later in connect_close_page), so the
            // remainder after this close is (current live count - 1). The count
            // spans all session kinds (window-guide.md). A failed weak upgrade
            // means the window is tearing down — skip the reveal silently.
            let enabled = with_state(&state_for_close, |s| {
                s.settings().ui.auto_hide_sidebar_on_connect
            });
            if let Some(notebook) = notebook_for_autohide_close.upgrade() {
                let external = external_session_registry().map_or(0, |reg| reg.active_count());
                let live_now = notebook.session_count() + notebook.detached_count() + external;
                let remaining = live_now.saturating_sub(1);
                if let Some(action) =
                    rustconn_core::config::sidebar_action_on_session_closed(enabled, remaining)
                {
                    Self::apply_sidebar_auto_action(&split_for_close, action);
                }
            }
        });

        // Auto-hide-on-connect (issue #401). Fires for every protocol via the
        // notebook's all-protocol "session shown" choke point. Session restore
        // reopens tabs through `open_local_shell_with_split` /
        // `start_connection_with_credential_resolution_observed`, both of which
        // end in `notify_tab_added`, so a restored session auto-hides too — the
        // intended "a session was shown" behaviour.
        let split_for_shown = overlay_split_view.clone();
        let state_for_shown = state.clone();
        terminal_notebook.set_on_session_shown(move |_session_id, _connection_id| {
            // Read the bool into a local FIRST — no borrow held across the GTK
            // call (NFR3).
            let enabled = with_state(&state_for_shown, |s| {
                s.settings().ui.auto_hide_sidebar_on_connect
            });
            if let Some(action) = rustconn_core::config::sidebar_action_on_session_shown(enabled) {
                Self::apply_sidebar_auto_action(&split_for_shown, action);
            }
        });

        // A renamed session's split pane header (issue #277) follows the rename,
        // for a guest and an owner alike (issue #421). The map is held weakly:
        // each bridge's title listener holds the notebook, so a strong
        // reference from the notebook's hook would close a cycle.
        {
            let bridges_weak = Rc::downgrade(&session_split_bridges);
            terminal_notebook.set_on_session_relabelled(move |session_id, name| {
                let Some(bridges) = bridges_weak.upgrade() else {
                    return;
                };
                let bridge = bridges.borrow().get(&session_id).cloned();
                if let Some(bridge) = bridge {
                    bridge.relabel_session(session_id, name);
                }
            });
        }

        // Wire activity monitoring from the single session-creation choke point.
        // This covers every terminal protocol and both synchronous and async
        // (port-checked) connection paths, regardless of which connect action
        // was used (sidebar Connect, command palette, cluster, double-click).
        {
            let state_for_activity = state.clone();
            let activity_for_setup = activity_coordinator.clone();
            let notebook_weak = Rc::downgrade(&terminal_notebook);
            let state_for_scripts = state.clone();
            let sidebar_for_scripts = sidebar.clone();
            let monitoring_for_scripts = monitoring.clone();
            terminal_notebook.set_on_session_created(move |session_id, connection_id| {
                let Some(notebook) = notebook_weak.upgrade() else {
                    return;
                };
                Self::setup_activity_monitoring(
                    &state_for_activity,
                    &notebook,
                    &activity_for_setup,
                    session_id,
                    connection_id,
                );
                // Auto-run scripts belong to this same choke point: a
                // connection started after an asynchronous port check never
                // returns a session id to its caller, so hooking the launch
                // path directly missed exactly those connections.
                crate::script_runner::maybe_run_auto_scripts(
                    state_for_scripts.clone(),
                    notebook.clone(),
                    sidebar_for_scripts.clone(),
                    monitoring_for_scripts.clone(),
                    connection_id,
                    session_id,
                );
            });

            // Group-path subtitle. When the active tab changes, set the content
            // header's subtitle to the active connection's group path (e.g.
            // "AWS Test Lab / Prod"), gated on the `window_title_shows_path`
            // setting. The WM window title (issue #211) is untouched — this only
            // drives the AdwWindowTitle subtitle. The handler lives on the
            // notebook's own TabView, so it holds the notebook weakly.
            {
                let header_title_for_sub = header_title.clone();
                let state_for_sub = state.clone();
                let notebook_for_sub = Rc::downgrade(&terminal_notebook);
                terminal_notebook
                    .tab_view()
                    .connect_selected_page_notify(move |_tab_view| {
                        let Some(notebook) = notebook_for_sub.upgrade() else {
                            return;
                        };
                        Self::refresh_header_subtitle(
                            &header_title_for_sub,
                            &notebook,
                            &state_for_sub,
                        );
                    });

                // A split tab names its focused pane (issue #421), so the
                // subtitle follows pane focus too, not only tab switches.
                let header_title_for_split = header_title.clone();
                let state_for_split = state.clone();
                let notebook_for_split = Rc::downgrade(&terminal_notebook);
                terminal_notebook.set_on_split_title_changed(move || {
                    let Some(notebook) = notebook_for_split.upgrade() else {
                        return;
                    };
                    Self::refresh_header_subtitle(
                        &header_title_for_split,
                        &notebook,
                        &state_for_split,
                    );
                });
            }
        }

        // Focus-based accelerator suspend (#197): when the VTE gains focus,
        // suspend the single-Ctrl accelerators that collide with readline
        // chords; restore them from settings when focus leaves. Honors the
        // `terminal_passthrough_ctrl` setting live (off → leave accels as-is,
        // composing with the global passthrough). A weak `app` ref avoids a
        // reference cycle (the application owns the window).
        {
            let state_for_focus = state.clone();
            let app_weak = app.downgrade();
            let window_for_focus = window.downgrade();
            terminal_notebook.set_on_terminal_focus(move |focused| {
                // Command-macro accels live only while a terminal has focus, and
                // follow focus between split panes; the dispatcher re-reads the
                // focused session on the next idle.
                if let Some(window) = window_for_focus.upgrade() {
                    macro_dispatch::request_refresh(&window);
                }
                let passthrough = with_state(&state_for_focus, |s| {
                    s.settings().ui.terminal_passthrough_ctrl
                });
                if !passthrough {
                    return;
                }
                let Some(app) = app_weak.upgrade() else {
                    return;
                };
                if focused {
                    crate::app::suspend_terminal_accels(&app, &state_for_focus);
                } else {
                    crate::app::restore_terminal_accels(&app, &state_for_focus);
                }
            });
        }

        // Resolve a split guest's pane container so its reconnect banner can be
        // shown inside its own pane, not only for the split owner (issue #328).
        {
            let bridges_for_pane_box = session_split_bridges.clone();
            terminal_notebook.set_split_pane_box_provider(move |session_id| {
                let bridges = bridges_for_pane_box.borrow();
                bridges
                    .values()
                    .find_map(|bridge| bridge.pane_container_for_session(session_id))
            });
        }

        // Put a disconnected session's reconnect banner back after a move
        // between a tab, a split pane and a window dropped it. Weak: every
        // terminal holds this hook.
        {
            let notebook_for_mapped = Rc::downgrade(&terminal_notebook);
            terminal_notebook.set_on_terminal_mapped(move |session_id| {
                if let Some(notebook) = notebook_for_mapped.upgrade() {
                    notebook.reconcile_reconnect_banner(session_id);
                }
            });
        }

        // Focus the pane that shows a session, so Q on an ended session closes
        // that pane through `win.close-pane` rather than the pane last clicked:
        // the bridge follows clicks, not keyboard focus.
        {
            let bridges_for_focuser = session_split_bridges.clone();
            terminal_notebook.set_split_pane_focuser(move |session_id| {
                // Resolved under the borrow, focused after it: focusing notifies
                // the title source, which reads the bridges map again.
                let target = bridges_for_focuser.borrow().values().find_map(|bridge| {
                    bridge
                        .pane_ids()
                        .into_iter()
                        .find(|&pane| bridge.get_pane_session(pane) == Some(session_id))
                        .map(|pane| (Rc::clone(bridge), pane))
                });
                let Some((bridge, pane)) = target else {
                    return false;
                };
                bridge.set_focused_pane(Some(pane));
                true
            });
        }

        // Resolve the focused pane's session for a split tab, so "Save Output"
        // saves the pane the user is looking at rather than the tab owner
        // (issue #371). Keyed by the owner session id — the tab's bridge is the
        // only one that knows which of its panes has focus.
        {
            let bridges_for_focus = session_split_bridges.clone();
            terminal_notebook.set_focused_session_provider(move |owner_id| {
                let bridges = bridges_for_focus.borrow();
                bridges
                    .get(&owner_id)
                    .and_then(|bridge| bridge.get_focused_session())
            });
        }

        // Set up reconnect callback for VTE sessions
        // When user clicks "Reconnect" in a disconnected tab, reuse the
        // existing terminal tab instead of closing and creating a new one.
        // This preserves tab position, avoids visual flicker, and keeps
        // the user's tab arrangement intact (#89).
        {
            let state_for_reconnect = state.clone();
            let notebook_for_reconnect = Rc::downgrade(&terminal_notebook);
            let split_view_for_reconnect = split_view.clone();
            let sidebar_for_reconnect = sidebar.clone();
            let monitoring_for_reconnect = monitoring.clone();
            let activity_for_reconnect = activity_coordinator.clone();
            terminal_notebook.set_on_reconnect(move |session_id, connection_id| {
                let Some(notebook_for_reconnect) = notebook_for_reconnect.upgrade() else {
                    return;
                };
                tracing::info!(
                    %session_id,
                    %connection_id,
                    "Reconnecting session in-place"
                );

                // Determine the protocol of the disconnected session
                let protocol = notebook_for_reconnect
                    .get_session_info(session_id)
                    .map(|info| info.protocol.clone());

                // A local shell has no saved connection to dial again: start a
                // new shell in the same terminal.
                if protocol.as_deref() == Some(LOCAL_SHELL_PROTOCOL) {
                    if notebook_for_reconnect.prepare_for_reconnect(session_id) {
                        let terminal_settings = state_for_reconnect
                            .try_borrow()
                            .map(|s| s.settings().terminal.clone())
                            .unwrap_or_default();
                        Self::start_local_shell_process(
                            &notebook_for_reconnect,
                            session_id,
                            &terminal_settings,
                        );
                    }
                    return;
                }

                // For VTE-based sessions, reconnect in-place (reuse existing tab)
                let is_vte_protocol = protocol.as_deref().is_some_and(|p| {
                    p == "ssh"
                        || p == "telnet"
                        || p == "winrm"
                        || p == "serial"
                        || p == "kubernetes"
                        || p == "mosh"
                        || p.starts_with("zerotrust")
                });
                if is_vte_protocol {
                    let success = if protocol.as_deref() == Some("ssh") {
                        protocols::reconnect_ssh_in_place(
                            &state_for_reconnect,
                            &notebook_for_reconnect,
                            &sidebar_for_reconnect,
                            &monitoring_for_reconnect,
                            session_id,
                            connection_id,
                        )
                    } else {
                        protocols::reconnect_generic_vte_in_place(
                            &state_for_reconnect,
                            &notebook_for_reconnect,
                            &sidebar_for_reconnect,
                            session_id,
                            connection_id,
                        )
                    };
                    if success {
                        // In-place reconnect reuses the terminal, so the VTE
                        // signal handlers persist; only the coordinator session
                        // entry must be recreated so monitoring/menu resume.
                        Self::reactivate_activity_monitoring(
                            &state_for_reconnect,
                            &activity_for_reconnect,
                            connection_id,
                            session_id,
                        );
                        return;
                    }
                    tracing::warn!(
                        %session_id,
                        "In-place reconnect failed, falling back to close+create"
                    );
                }

                // Snapshot placement before close tears down the old tab/window.
                let was_detached = notebook_for_reconnect.is_detached(session_id);
                let presentation = was_detached.then(|| {
                    detached_window_registry()
                        .and_then(|registry| registry.presentation(session_id))
                        .unwrap_or_default()
                });
                let tab_position = if was_detached {
                    None
                } else {
                    let sessions = notebook_for_reconnect.sessions_map();
                    let sessions_ref = sessions.borrow();
                    sessions_ref
                        .get(&session_id)
                        .map(|page| notebook_for_reconnect.tab_view().page_position(page))
                };

                notebook_for_reconnect.close_tab(session_id);

                let notebook_for_observer = Rc::downgrade(&notebook_for_reconnect);
                let observer = types::SessionStartObserver::new(move |new_session_id| {
                    let Some(notebook) = notebook_for_observer.upgrade() else {
                        return;
                    };
                    if let Some(presentation) = presentation {
                        if !notebook.request_detach(new_session_id, presentation) {
                            tracing::warn!(
                                session = %new_session_id,
                                "could not restore detached placement after reconnect"
                            );
                        }
                    } else if let Some(original_pos) = tab_position {
                        let page = notebook
                            .sessions_map()
                            .borrow()
                            .get(&new_session_id)
                            .cloned();
                        if let Some(page) = page {
                            notebook.tab_view().reorder_page(&page, original_pos);
                        }
                    }
                });

                Self::start_connection_with_credential_resolution_observed(
                    state_for_reconnect.clone(),
                    notebook_for_reconnect,
                    split_view_for_reconnect.clone(),
                    sidebar_for_reconnect.clone(),
                    monitoring_for_reconnect.clone(),
                    connection_id,
                    Some(activity_for_reconnect.clone()),
                    Some(observer),
                );
            });
        }

        // "Log In to <provider>" on a Zero Trust session whose cloud
        // credentials expired: run the CLI login in the tab, then reconnect.
        {
            let notebook_for_login = Rc::downgrade(&terminal_notebook);
            terminal_notebook.set_on_cloud_login(move |session_id, connection_id, login| {
                if let Some(notebook) = notebook_for_login.upgrade() {
                    Self::start_cloud_login(&notebook, session_id, connection_id, login);
                }
            });
        }

        // TabView/TabBar configuration is handled internally
        // TabView is always visible — content lives inside TabPages
        terminal_notebook.widget().set_vexpand(true);
        // Ensure notebook is visible
        terminal_notebook.widget().set_visible(true);
        terminal_notebook.show_tab_view_content();

        // Create a container for the terminal area
        let terminal_container = gtk4::Box::new(Orientation::Vertical, 0);
        terminal_container.set_vexpand(true);
        terminal_container.set_hexpand(true);

        // Add notebook tabs at top for session switching (tabs only, content hidden by size)
        terminal_container.append(terminal_notebook.widget());

        // Add split view as the main content area - takes full space
        // With per-tab split architecture, this is hidden by default
        // (content lives inside TabPages, not in a global split view)
        split_view.widget().set_vexpand(false);
        split_view.widget().set_hexpand(true);
        split_view.widget().set_visible(false);
        terminal_container.append(split_view.widget());

        // Add split_container for per-session split views (initially hidden)
        split_container.set_visible(false);
        terminal_container.append(&split_container);

        // Note: drag-and-drop is set up in connect_signals after we have access to notebook

        // Per-panel headerbars (GNOME Files/Settings style): the window has NO
        // single full-width titlebar. The content side is an AdwToolbarView
        // whose top bar is the header + banners and whose content is the
        // terminal container; it becomes the OverlaySplitView's content, so the
        // split renders as two distinct panels with their own headers and a
        // vertical divider between them. (The sidebar carries its own headerbar
        // from §1.) `terminal_container` is wired into this toolbar view below,
        // once its top bars are attached.

        // Create toast overlay — wraps the whole split view (set below).
        let toast_overlay = Rc::new(ToastOverlay::new());

        // Content-panel layout: adw::ToolbarView carrying the header + banners
        // above the terminal container. This is the content side of the split
        // (NOT a window-level full-width bar).
        let toolbar_view = adw::ToolbarView::new();
        // The header goes in through the fullscreen chrome, which hides it — and
        // the tab bar it adopts in fullscreen — as one block (issue #354). It
        // must stay the first top bar, above the banners.
        let fullscreen_chrome = fullscreen_header::Chrome::new(&header_bar);
        toolbar_view.add_top_bar(fullscreen_chrome.widget());

        // Persistent banner for config files startup could not read, or found
        // written by a newer RustConn (GNOME HIG: a state that needs attention
        // belongs in a banner). Hidden until `show_config_file_notice` fills it.
        // Every banner here is a `plain_banner`: their text carries paths, names
        // and error strings, which are not Pango markup.
        let config_banner = crate::toast::plain_banner("");
        config_banner.set_button_label(Some(&crate::i18n::i18n("Dismiss")));
        config_banner.connect_button_clicked(|banner| {
            banner.set_revealed(false);
        });
        toolbar_view.add_top_bar(&config_banner);

        // Persistent banner for cloud sync failures (GNOME HIG: a state
        // that needs attention belongs in a banner, not a transient toast).
        // Hidden by default; shown via show_sync_error_banner(), hidden on
        // the next successful sync.
        let sync_banner = crate::toast::plain_banner("");
        sync_banner.set_button_label(Some(&crate::i18n::i18n("Dismiss")));
        sync_banner.connect_button_clicked(|banner| {
            banner.set_revealed(false);
        });
        toolbar_view.add_top_bar(&sync_banner);

        // Persistent banner warning that the preferred secret backend cannot
        // store passwords (GNOME HIG: keyring-unavailable is a state that
        // needs attention, so it belongs in a banner, not a transient toast).
        // Hidden by default; shown via check_secret_backend_available(). Its
        // action button opens Settings, where the user picks a working backend
        // in the Secrets section (#201).
        let secret_banner = crate::toast::plain_banner("");
        secret_banner.set_button_label(Some(&crate::i18n::i18n("Open Settings")));
        secret_banner.connect_button_clicked(|banner| {
            // The banner lives inside the window that owns the `win.settings`
            // action; activating it from the banner opens Settings → Secrets.
            let _ = gtk4::prelude::WidgetExt::activate_action(banner, "win.settings", None);
            banner.set_revealed(false);
        });
        toolbar_view.add_top_bar(&secret_banner);

        // Persistent banner shown at startup when the previous run left a Rust
        // panic crash log (W5). GNOME HIG: a past failure the user should know
        // about is a banner, not a transient toast. Its button opens the log
        // (portal-aware FileLauncher, which also escapes the path for the URI);
        // the report is a local file that is never sent anywhere automatically,
        // so the banner only points at it. The logs are cleared only once the
        // log actually opened: a failed open keeps the file and the banner, and
        // names the path in a toast so the user can still find it. adw::Banner
        // has no dismiss of its own, so until then it returns on each launch.
        let crash_banner = crate::toast::plain_banner("");
        crash_banner.set_button_label(Some(&crate::i18n::i18n("Open Log")));
        if let Some(report) = rustconn_core::crash_report::take_pending_crash() {
            crash_banner.set_title(&crate::i18n::i18n(
                "RustConn closed unexpectedly the last time it ran. A crash log was saved locally.",
            ));
            let log_path = report.path.clone();
            crash_banner.connect_button_clicked(move |banner| {
                let file = gtk4::gio::File::for_path(&log_path);
                let launcher = gtk4::FileLauncher::new(Some(&file));
                let win = banner
                    .root()
                    .and_then(|r| r.downcast::<gtk4::Window>().ok());
                let banner_weak = banner.downgrade();
                let win_for_toast = win.clone();
                let shown_path = log_path.display().to_string();
                launcher.launch(
                    win.as_ref(),
                    gtk4::gio::Cancellable::NONE,
                    move |res| match res {
                        Ok(()) => {
                            rustconn_core::crash_report::clear_crash_reports();
                            if let Some(banner) = banner_weak.upgrade() {
                                banner.set_revealed(false);
                            }
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "could not open the crash log");
                            let message = crate::i18n::i18n_f(
                                "Could not open the crash log. It is saved at {}",
                                &[&shown_path],
                            );
                            if let Some(win) = win_for_toast {
                                crate::toast::show_toast_on_window(
                                    &win,
                                    &message,
                                    crate::toast::ToastType::Error,
                                );
                            } else {
                                crate::toast::show_error_toast_on_active_window(&message);
                            }
                        }
                    },
                );
            });
            crash_banner.set_revealed(true);
        } else {
            crash_banner.set_revealed(false);
        }
        toolbar_view.add_top_bar(&crash_banner);

        // Persistent banner shown while a `keepassxc-cli` run blocks on a
        // hardware-key touch (issue #350). GTK widgets are !Send, so the
        // worker-thread touch observer bridges to the main thread the same way
        // BusyStack does: it sends a +1/-1 delta over an mpsc channel, and a
        // glib::timeout_add_local drain tracks the in-flight count and reveals
        // the banner whenever it is above zero. Informational — no button.
        let touch_banner = crate::toast::plain_banner(&crate::i18n::i18n(
            "Touch your hardware key to unlock the password database",
        ));
        toolbar_view.add_top_bar(&touch_banner);
        {
            // true = a touch-waiting run started, false = it ended. One run can
            // be challenged twice (open + save), and several may overlap, so we
            // net the deltas into a count rather than treating each as a toggle.
            let (touch_tx, touch_rx) = std::sync::mpsc::channel::<bool>();
            rustconn_core::secret::set_touch_observer(Some(std::sync::Arc::new(
                BannerTouchObserver { tx: touch_tx },
            )));
            let banner = touch_banner.clone();
            let rx = std::sync::Mutex::new(touch_rx);
            let in_flight = std::cell::Cell::new(0_i32);
            glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                if let Ok(guard) = rx.lock() {
                    let mut count = in_flight.get();
                    while let Ok(started) = guard.try_recv() {
                        count += if started { 1 } else { -1 };
                    }
                    // Deltas are matched one-to-one by the TouchGuard, but clamp
                    // defensively so a dropped message can never pin the banner on.
                    count = count.max(0);
                    in_flight.set(count);
                    banner.set_revealed(count > 0);
                }
                glib::ControlFlow::Continue
            });
        }

        // Persistent banner shown while cross-tab group broadcast is active
        // (issue #329). Unlike a split broadcast, whose panels are all on screen,
        // a group broadcast reaches tabs the user cannot see, so its active state
        // must be unmissable — a banner, not a transient toast. Its button turns
        // the broadcast off. Hidden until group broadcast is enabled.
        let group_broadcast_banner = crate::toast::plain_banner("");
        group_broadcast_banner.set_button_label(Some(&crate::i18n::i18n("Stop")));
        group_broadcast_banner.add_css_class("accent");
        group_broadcast_banner.connect_button_clicked(|banner| {
            if let Some(win) = banner
                .root()
                .and_then(|r| r.downcast::<adw::ApplicationWindow>().ok())
            {
                gio::prelude::ActionGroupExt::activate_action(&win, "toggle-group-broadcast", None);
            }
        });
        toolbar_view.add_top_bar(&group_broadcast_banner);

        // The content panel's body is the terminal container (header + banners
        // sit above it in this same toolbar view).
        toolbar_view.set_content(Some(&terminal_container));

        // The content toolbar view IS the split's content side; the sidebar
        // side carries its own headerbar. The split then wraps in the toast
        // overlay, which wraps in the tab overview — no window-level full-width
        // header anywhere, so the two panels render with their own headers and a
        // divider between them (GNOME Files/Settings).
        overlay_split_view.set_content(Some(&toolbar_view));
        toast_overlay.set_child(Some(&overlay_split_view));

        // Wrap everything with TabOverview — must be the outermost widget
        // so it can overlay the entire window content (GNOME Web pattern)
        let tab_overview = terminal_notebook.tab_overview();
        tab_overview.set_child(Some(toast_overlay.widget()));
        // Clip overflow to prevent the TabOverview from requesting more space
        // than the window provides when embedded RDP sessions have large framebuffers
        tab_overview.set_overflow(gtk4::Overflow::Hidden);

        window.set_content(Some(tab_overview));

        // Fullscreen hides the header bar, the tab bar and the sidebar — the
        // banners stay — and brings the bars back on a top-edge hover, F10 or a tab
        // switch (issue #354).
        fullscreen_header::install(
            &window,
            &toolbar_view,
            &overlay_split_view,
            fullscreen_chrome,
            &menu_button,
            &terminal_notebook,
        );

        // Adaptive layout breakpoints (#204).
        //
        // CRITICAL: AdwApplicationWindow applies only ONE breakpoint at a time —
        // the last-added whose condition currently holds — NOT a cumulative
        // stack. So each tier must repeat every setter of the wider tiers, and
        // tiers must be added widest→narrowest so the narrowest matching one wins
        // (e.g. at 500 sp both tiers match, the narrow tier added last applies).
        //
        // Why these two things move together:
        // - The header packs many fixed 44×44 icon buttons whose combined minimum
        //   otherwise squeezes the GtkWindowControls (min/max/close) off the end.
        // - The sidebar's own minimum (~368 px — its filter/bulk revealers demand
        //   width even while hidden) pins the window wide unless it is overlaid
        //   first, so auto-hide can never engage.
        //
        // Thresholds are chosen so each tier's resulting minimum width is BELOW
        // the next (narrower) tier's threshold — otherwise the window's minimum
        // plateaus at a tier boundary and a single drag "sticks" there, needing
        // a second drag to continue (the reported jank). The content header
        // holds only session/window actions since 0.23 (the list actions moved
        // to the sidebar and the primary menu):
        // - medium ≤ 820sp: collapse + hide the sidebar (F9-style) and hide the
        //   two split buttons (`MEDIUM_HIDDEN`).
        // - narrow ≤ 600sp: everything above, plus Settings and the Shell pill
        //   (`NARROW_HIDDEN`), leaving the Sidebar toggle and the primary menu
        //   beside the window controls. Both stay reachable from the menu.
        // The sidebar is hidden (show-sidebar = false), not shown as an overlay,
        // when collapsed; F9 / the edge gesture still reveals it as an overlay.
        // Growing the window past a threshold restores the hidden setters.
        let collapsed_val = true.to_value();
        let hide_flag = false.to_value();
        // The centre title ("RustConn") is hidden in both tiers so the freed
        // space guarantees room for the window controls at the minimum width.
        let title_widget = header_bar.title_widget();

        let bp_medium = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            820.0,
            adw::LengthUnit::Sp,
        ));
        bp_medium.add_setter(&overlay_split_view, "collapsed", Some(&collapsed_val));
        bp_medium.add_setter(&overlay_split_view, "pin-sidebar", Some(&hide_flag));
        bp_medium.add_setter(&overlay_split_view, "show-sidebar", Some(&hide_flag));
        if let Some(title) = title_widget.as_ref() {
            bp_medium.add_setter(title, "visible", Some(&hide_flag));
        }
        for action in MEDIUM_HIDDEN {
            if let Some(btn) = Self::header_button(&header_bar, action) {
                bp_medium.add_setter(&btn, "visible", Some(&hide_flag));
            }
        }
        window.add_breakpoint(bp_medium);

        let bp_narrow = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            600.0,
            adw::LengthUnit::Sp,
        ));
        bp_narrow.add_setter(&overlay_split_view, "collapsed", Some(&collapsed_val));
        bp_narrow.add_setter(&overlay_split_view, "pin-sidebar", Some(&hide_flag));
        bp_narrow.add_setter(&overlay_split_view, "show-sidebar", Some(&hide_flag));
        if let Some(title) = title_widget.as_ref() {
            bp_narrow.add_setter(title, "visible", Some(&hide_flag));
        }
        for action in NARROW_HIDDEN {
            if let Some(btn) = Self::header_button(&header_bar, action) {
                bp_narrow.add_setter(&btn, "visible", Some(&hide_flag));
            }
        }
        // The Shell button stays at narrow width (removed from NARROW_HIDDEN),
        // but its "Shell" text must give way to icon-only there. The label
        // collapse lives in CSS keyed on `.compact`, which only engages when
        // auto-compact is on — so mark the window `.narrow` on this breakpoint
        // too, and CSS collapses `.shell-button-label` under either class. This
        // is width-driven and independent of the compact preference (user
        // request, 0.23.5 audit).
        {
            let win = window.clone();
            bp_narrow.connect_apply(move |_| win.add_css_class("narrow"));
            let win = window.clone();
            bp_narrow.connect_unapply(move |_| win.remove_css_class("narrow"));
        }
        window.add_breakpoint(bp_narrow);

        // Pin the window's minimum width to the *measured* narrow-tier layout so
        // the window controls (min/max/close) are never clipped, regardless of
        // theme, font, locale or the compositor's decoration layout (#204).
        // Guessing this from button counts proved unreliable, so once the window
        // is mapped (and the window controls are realized) we temporarily hide
        // exactly what the narrow tier hides, measure the real minimum width of
        // the content, restore visibility, and set width-request to it. Runs
        // once. `AdwApplicationWindow` derives its minimum from width-request
        // when breakpoints are present, so this becomes the true floor.
        let narrow_hidden: Vec<gtk4::Widget> = {
            let mut widgets = Vec::new();
            if let Some(title) = header_bar.title_widget() {
                widgets.push(title);
            }
            for action in NARROW_HIDDEN {
                if let Some(btn) = Self::header_button(&header_bar, action) {
                    widgets.push(btn.upcast());
                }
            }
            widgets
        };
        let overview_for_measure = tab_overview.clone();
        let split_for_measure = overlay_split_view.clone();
        let measured = std::cell::Cell::new(false);
        window.connect_map(move |win| {
            if measured.replace(true) {
                return;
            }
            // Mirror the narrow tier *exactly*: besides hiding the shed header
            // buttons, collapse the sidebar. When the split view is not
            // collapsed its minimum is sidebar-width + content-width, and the
            // sidebar's filter/search/bulk-action labels are locale-dependent —
            // wider in German than in English — which pinned the window far too
            // wide for tiling (#204 follow-up). Collapsing here (as the narrow
            // breakpoint does) makes the sidebar overlay instead, so only the
            // content contributes and the measured floor is locale-independent.
            let was_collapsed = split_for_measure.is_collapsed();
            let was_showing = split_for_measure.shows_sidebar();
            split_for_measure.set_collapsed(true);
            split_for_measure.set_show_sidebar(false);
            for widget in &narrow_hidden {
                widget.set_visible(false);
            }
            let (min_width, _, _, _) =
                overview_for_measure.measure(gtk4::Orientation::Horizontal, -1);
            for widget in &narrow_hidden {
                widget.set_visible(true);
            }
            split_for_measure.set_collapsed(was_collapsed);
            split_for_measure.set_show_sidebar(was_showing);
            // Small margin absorbs sub-pixel rounding; floor guards a degenerate
            // zero measurement before first layout.
            win.set_width_request((min_width + 4).max(360));
        });

        // External viewer session registry (issue #209): tracks VNC/RDP/SPICE
        // sessions delegated to a separate viewer process, surfaced in the
        // sidebar without a notebook tab. The callbacks bridge the registry into
        // the sidebar session count and the connection history; a single shared
        // 2 s poll timer (inside the registry) reaps closed viewers.
        let external_sessions = {
            let sidebar_registered = sidebar.clone();
            let on_registered: Box<dyn Fn(Uuid, Uuid)> =
                Box::new(move |session_id, connection_id| {
                    // R2.1: show the green "connected" status via the session count.
                    sidebar_registered
                        .increment_session_count(&connection_id.to_string(), session_id);
                    // R2.2: show the `window-new-symbolic` emblem alongside the
                    // connected icon. `record_connection_start` (R3.1) is done at the
                    // launch site (task 6); its entry id is passed into `register`.
                    sidebar_registered.set_external_session(&connection_id.to_string(), true);
                });

            let sidebar_ended = sidebar.clone();
            let state_ended = state.clone();
            let on_ended: Box<dyn Fn(Uuid, Uuid, Option<Uuid>)> = Box::new(
                move |session_id, connection_id, history_entry_id| {
                    // R4.3/4.5: clear the connected state when the viewer exits.
                    sidebar_ended.decrement_session_count(
                        &connection_id.to_string(),
                        session_id,
                        false,
                    );
                    // R3.2/4.4: record the end exactly once; skip if there is no
                    // start entry and log a warning (R3.5). A failed borrow is
                    // logged and does not interrupt the session (R3.4).
                    if let Some(entry_id) = history_entry_id {
                        if let Ok(mut state_mut) = state_ended.try_borrow_mut() {
                            state_mut.record_connection_end(entry_id);
                        } else {
                            tracing::warn!(
                                %connection_id,
                                "Could not borrow AppState to record external session end"
                            );
                        }
                    } else {
                        tracing::warn!(
                            %connection_id,
                            "External session ended without a history start entry; skipping record_connection_end"
                        );
                    }
                    // R2.5/2.6: the emblem is visible iff the connection still
                    // has at least one active external session. `on_ended` fires
                    // after the ending session is removed from the registry, so
                    // querying it here reflects the remaining sessions — the
                    // emblem is cleared only once the count reaches zero (a
                    // connection may have more than one external viewer).
                    let still_external = external_session_registry()
                        .is_some_and(|reg| reg.has_active_session(connection_id));
                    sidebar_ended.set_external_session(&connection_id.to_string(), still_external);
                },
            );

            crate::external_session::ExternalSessionRegistry::new(
                crate::external_session::ExternalSessionCallbacks {
                    on_registered,
                    on_ended,
                },
            )
        };

        // Create tunnel manager for standalone SSH tunnels
        let tunnel_manager: SharedTunnelManager = Rc::new(RefCell::new(
            rustconn_core::tunnel_manager::TunnelManager::new(),
        ));

        let main_window = Self {
            window,
            sidebar,
            terminal_notebook,
            split_view,
            session_split_bridges,
            global_color_pool,
            split_container,
            state: state.clone(),
            overlay_split_view,
            header_title,
            external_sessions,
            detached_windows: Rc::new(crate::detached_window::DetachedWindowRegistry::new()),
            toast_overlay,
            monitoring,
            activity_coordinator,
            tunnel_manager,
            busy_stack,
            quick_connect_history: types::load_quick_connect_history(&state),
            passthrough_indicator,
            menu_button,
            broadcast_toggle,
            broadcast_hint_shown: Rc::new(std::cell::Cell::new(false)),
            group_broadcast,
            group_broadcast_toggle,
            group_broadcast_banner,
            config_banner,
            sync_banner,
            secret_banner,
        };

        // Set up window actions
        main_window.setup_actions();

        // Publish BusyStack to thread-local so static methods can acquire guards
        BUSY_STACK.with(|cell| {
            *cell.borrow_mut() = Some(main_window.busy_stack.clone());
        });

        // Publish the external viewer session registry to a thread-local so the
        // static `start_*_connection` launch paths can register external viewers
        // (issue #209) without threading the registry through every call site.
        EXTERNAL_SESSIONS.with(|cell| {
            *cell.borrow_mut() = Some(Rc::clone(&main_window.external_sessions));
        });
        // Publish the detached window registry (issue #236) so the lifecycle
        // paths reach it while every action closure keeps only a `Weak` handle.
        DETACHED_WINDOWS.with(|cell| {
            *cell.borrow_mut() = Some(Rc::clone(&main_window.detached_windows));
        });

        // The sidebar and smart-folder "Copy" submenus (issue #357). Weak, so
        // the thread-local provider does not keep the application state alive.
        {
            let state_weak = Rc::downgrade(&main_window.state);
            crate::sidebar_ui::set_copy_items_provider(move |conn_id| {
                let Some(state) = state_weak.upgrade() else {
                    return Vec::new();
                };
                let Ok(id) = Uuid::parse_str(conn_id) else {
                    return Vec::new();
                };
                let Ok(state_ref) = state.try_borrow() else {
                    return Vec::new();
                };
                state_ref
                    .get_connection(id)
                    .map(copy_field_actions::sidebar_copy_items)
                    .unwrap_or_default()
            });

            // The tab menu only needs to know whether the tab's connection is a
            // saved one, to decide whether to offer Edit Connection…. Copy lives
            // in the sidebar menu, so no copy entries are built here.
            let state_weak = Rc::downgrade(&main_window.state);
            main_window
                .terminal_notebook
                .set_tab_connection_menu_provider(move |connection_id| {
                    let Some(state) = state_weak.upgrade() else {
                        return false;
                    };
                    let Ok(state_ref) = state.try_borrow() else {
                        return false;
                    };
                    state_ref.get_connection(connection_id).is_some()
                });
        }

        // Set up recording checker for sidebar context menu
        {
            let notebook = main_window.terminal_notebook.clone();
            main_window
                .sidebar
                .set_recording_checker(move |conn_id_str| {
                    if let Ok(conn_id) = Uuid::parse_str(conn_id_str) {
                        notebook
                            .get_all_sessions()
                            .iter()
                            .any(|s| s.connection_id == conn_id && notebook.is_recording(s.id))
                    } else {
                        false
                    }
                });
        }

        // Set up multi-session checker for the sidebar context menu (#374):
        // true when a connection has 2+ live sessions, so the ambiguous
        // per-session controls (Disconnect / Move to Own Tab / Move to New
        // Window) are hidden — they target a single session, which is
        // undefined when several are open for one connection.
        {
            let notebook = main_window.terminal_notebook.clone();
            main_window
                .sidebar
                .set_multi_session_checker(move |conn_id_str| {
                    if let Ok(conn_id) = Uuid::parse_str(conn_id_str) {
                        notebook
                            .get_all_sessions()
                            .iter()
                            .filter(|s| s.connection_id == conn_id)
                            .count()
                            >= 2
                    } else {
                        false
                    }
                });
        }

        // Tells the sidebar menu whether a connection's session owns its split,
        // so that row offers Remove Split instead of Move to Own Tab. Asked only
        // when the row has one live session (the menu hides the split item
        // otherwise), so the first session found is that session. Weak handles:
        // the checker lives in a thread-local for the life of the process.
        {
            let notebook = Rc::downgrade(&main_window.terminal_notebook);
            let bridges = Rc::downgrade(&main_window.session_split_bridges);
            crate::sidebar_ui::set_split_owner_checker(move |conn_id_str| {
                let (Some(notebook), Some(bridges), Ok(conn_id)) = (
                    notebook.upgrade(),
                    bridges.upgrade(),
                    Uuid::parse_str(conn_id_str),
                ) else {
                    return false;
                };
                let Some(session_id) = notebook
                    .get_all_sessions()
                    .into_iter()
                    .find(|s| s.connection_id == conn_id)
                    .map(|s| s.id)
                else {
                    return false;
                };
                bridges
                    .borrow()
                    .get(&session_id)
                    .is_some_and(|bridge| bridge.is_owned_by(session_id))
            });
        }

        // Drive the sidebar recording indicator from recording start/stop.
        // Use a weak notebook reference — the notebook owns this callback,
        // so a strong clone would create an Rc cycle.
        {
            let sidebar = main_window.sidebar.clone();
            let notebook_weak = Rc::downgrade(&main_window.terminal_notebook);
            main_window.terminal_notebook.set_on_recording_changed(
                move |connection_id, recording| {
                    // A connection may have several sessions; keep the dot
                    // while ANY of them is still being recorded.
                    let still_recording = recording
                        || notebook_weak.upgrade().is_some_and(|nb| {
                            nb.get_all_sessions()
                                .iter()
                                .any(|s| s.connection_id == connection_id && nb.is_recording(s.id))
                        });
                    sidebar
                        .update_connection_recording(&connection_id.to_string(), still_recording);
                },
            );
        }

        // Drive the sidebar split-membership marker (R6.2) from the notebook's
        // split-color map. A single re-sync handler recomputes the full desired
        // per-connection state on every split mutation (join / leave / close),
        // which is simpler and more robust than tracking individual deltas.
        {
            let sidebar = main_window.sidebar.clone();
            let notebook_weak = Rc::downgrade(&main_window.terminal_notebook);
            // Connections we last marked as in-a-split, so we can clear the
            // marker when a connection's last split session leaves.
            let marked: Rc<RefCell<std::collections::HashSet<Uuid>>> =
                Rc::new(RefCell::new(std::collections::HashSet::new()));
            main_window
                .terminal_notebook
                .set_on_split_colors_changed(move || {
                    let Some(notebook) = notebook_weak.upgrade() else {
                        return;
                    };
                    // Snapshot the split map first, then drop the borrow before
                    // touching the sidebar to avoid BorrowMutError / nested borrows.
                    let pairs: Vec<(Uuid, usize)> = notebook
                        .split_colors()
                        .borrow()
                        .iter()
                        .map(|(session_id, color)| (*session_id, *color))
                        .collect();

                    // Resolve session_id → connection_id. A connection may host
                    // several split sessions; the last one wins for the single
                    // per-connection marker.
                    // ponytail: last-writer-wins per connection is fine for a
                    // one-square summary; index by session if per-session markers
                    // are ever needed.
                    let mut desired: std::collections::HashMap<Uuid, usize> =
                        std::collections::HashMap::new();
                    for (session_id, color) in pairs {
                        if let Some(info) = notebook.get_session_info(session_id) {
                            desired.insert(info.connection_id, color);
                        }
                    }

                    // Show / refresh markers for connections currently in a split.
                    for (connection_id, color) in &desired {
                        sidebar.set_split_color(&connection_id.to_string(), Some(*color));
                    }
                    // Clear markers for connections that left every split.
                    let mut marked_ref = marked.borrow_mut();
                    for connection_id in marked_ref.iter() {
                        if !desired.contains_key(connection_id) {
                            sidebar.set_split_color(&connection_id.to_string(), None);
                        }
                    }
                    *marked_ref = desired.keys().copied().collect();
                });
        }

        // Load initial data
        main_window.load_connections();
        crate::vault_ops::schedule_spa_key_move(&main_window.state);

        // Initialize KeePass button status
        main_window.update_keepass_button_status();

        // Report a config file startup kept aside or found from a newer RustConn
        main_window.show_config_file_notice();

        // Connect signals
        main_window.connect_signals();

        // Monitor network interface changes (VPN, WiFi/Ethernet switch) to
        // clean up stale ControlMaster sockets and trigger auto-reconnect (#217)
        network_monitor::setup_network_monitor(
            &main_window.state,
            &main_window.terminal_notebook,
            &main_window.sidebar,
            &main_window.toast_overlay,
        );

        // Notice when the machine wakes from sleep, so a session whose socket
        // died during the suspend is reported at once rather than after the
        // keepalive timeout — and never shown as a live frozen desktop (#248)
        resume_monitor::setup_resume_monitor(
            &main_window.state,
            &main_window.terminal_notebook,
            &main_window.toast_overlay,
        );

        main_window
    }
}
