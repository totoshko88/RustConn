//! Starting and focusing connection sessions

use super::*;

impl MainWindow {
    /// Opens the selected connection: the sidebar menu's Connect / Show Session.
    ///
    /// A connection that already runs is shown, not duplicated: the newest live
    /// embedded session is focused, the same way a double-click does it, and an
    /// external-viewer session gets the "already running" hint. The menu labels
    /// such a row **Show Session**, so the "new session on every double-click"
    /// preference does not apply here; **Open new session** is the duplicate.
    pub(super) fn connect_selected(
        state: &SharedAppState,
        sidebar: &SharedSidebar,
        notebook: &SharedNotebook,
        monitoring: &types::SharedMonitoring,
        session_split_bridges: &SessionSplitBridges,
    ) {
        // Get selected item from sidebar using the sidebar's method
        let Some(conn_item) = sidebar.get_selected_item() else {
            return;
        };

        // Only connect if it's not a group
        if conn_item.is_group() {
            return;
        }

        let id_str = conn_item.id();
        if let Ok(conn_id) = Uuid::parse_str(&id_str) {
            if Self::show_running_session(notebook, session_split_bridges, conn_id) {
                return;
            }
            Self::start_connection(state, notebook, sidebar, monitoring, conn_id);
        }
    }

    /// Shows a connection's running session instead of starting another.
    ///
    /// Focuses the most recently created *live* embedded session, or, when the
    /// connection runs only in an external viewer, says so in a toast. Returns
    /// `false` when there is nothing running to show (or the session vanished
    /// before it could be focused, R7.6), so the caller launches a new session.
    pub(super) fn show_running_session(
        notebook: &SharedNotebook,
        session_split_bridges: &SessionSplitBridges,
        conn_id: Uuid,
    ) -> bool {
        // `live_sessions` excludes tabs whose connection already ended — those
        // keep their tab for the transcript and the Reconnect button, and
        // focusing one instead of connecting is what made double-click look
        // dead (issue #242). Collect first (an owned `Uuid`) so no notebook
        // borrow is held across the focus call below.
        let newest_live = notebook
            .live_sessions()
            .into_iter()
            .filter(|s| s.connection_id == conn_id)
            .max_by_key(|s| s.connected_at)
            .map(|s| s.id);
        // Only *owned* viewers count — a detaching viewer (remmina/krdc) is
        // tracked without a child handle, so its entry never clears on its own
        // and would block every later open (issue #242).
        let external_running = || {
            external_session_registry()
                .is_some_and(|reg| reg.has_verifiable_active_session(conn_id))
        };
        match open_route(newest_live, external_running) {
            OpenRoute::Focus(target_id) => {
                Self::focus_embedded_session(notebook, session_split_bridges, target_id)
            }
            OpenRoute::AlreadyExternal => {
                // R7 external-only: a foreign viewer window cannot be raised
                // reliably, so inform rather than duplicate.
                crate::toast::show_info_toast_on_active_window(&crate::i18n::i18n(
                    "Already running in an external window",
                ));
                true
            }
            OpenRoute::Launch => false,
        }
    }

    /// Connects to a connection at a specific position with split view support.
    ///
    /// Smart double-click (R7): unless `force_new` is set (a Shift/Ctrl modifier
    /// was held or the "Open New Session" menu item was used), an existing live
    /// session for the connection is focused instead of spawning a duplicate.
    /// An embedded session is focused in place (R7.1/7.2/7.4); an external-only
    /// session shows an informational toast (a foreign OS window cannot be
    /// raised reliably from RustConn); zero sessions launch a new one (R7.3).
    ///
    /// The `double_click_opens_new_session` preference opts out of that
    /// resolution entirely (issue #242): while it is on, every activation
    /// behaves as if `force_new` were set, so the modifier is simply redundant.
    #[expect(
        clippy::too_many_arguments,
        reason = "smart double-click dispatch (R7) needs the full window context: state, sidebar, notebook, split view, monitoring, split bridges plus position/force_new/activity"
    )]
    pub(super) fn connect_at_position_with_split(
        state: &SharedAppState,
        sidebar: &SharedSidebar,
        notebook: &SharedNotebook,
        split_view: &SharedSplitView,
        monitoring: &types::SharedMonitoring,
        session_split_bridges: &SessionSplitBridges,
        position: u32,
        force_new: bool,
        activity: Option<&types::SharedActivityCoordinator>,
    ) {
        // Get the item at position from the tree model (not the flat store)
        let tree_model = sidebar.tree_model();
        if let Some(item) = tree_model.item(position) {
            // TreeListModel returns TreeListRow, need to get the actual item
            if let Some(row) = item.downcast_ref::<gtk4::TreeListRow>()
                && let Some(conn_item) =
                    row.item().and_then(|i| i.downcast::<ConnectionItem>().ok())
                && !conn_item.is_group()
            {
                let id_str = conn_item.id();
                if let Ok(conn_id) = Uuid::parse_str(&id_str) {
                    // Issue #242: the preference restores the pre-0.18.3
                    // behavior for users who keep several concurrent sessions
                    // on one host. Read into a `bool` so no state borrow is
                    // held into the launch path below, which borrows again.
                    let always_new = state.borrow().settings().ui.double_click_opens_new_session;

                    // R7.1/7.4/R7 external-only: show what already runs.
                    if !force_new
                        && !always_new
                        && Self::show_running_session(notebook, session_split_bridges, conn_id)
                    {
                        return;
                    }

                    // force_new, zero live sessions, or the R7.6 race: launch new.
                    sidebar.update_connection_status(&conn_id.to_string(), "connecting");
                    Self::start_connection_with_credential_resolution(
                        state.clone(),
                        notebook.clone(),
                        split_view.clone(),
                        sidebar.clone(),
                        monitoring.clone(),
                        conn_id,
                        activity.cloned(),
                    );
                }
            }
        }
    }

    /// Focuses an existing embedded session for the smart double-click.
    ///
    /// Selects the session's owner tab and, when the session lives in a split,
    /// focuses its pane and grabs input focus (R7.1, R7.2, R7.4). Returns
    /// `false` if the session disappeared before it could be focused (R7.6),
    /// signalling the caller to launch a new session instead.
    pub(super) fn focus_embedded_session(
        notebook: &SharedNotebook,
        session_split_bridges: &SessionSplitBridges,
        target_id: Uuid,
    ) -> bool {
        // R7.6: bail if the session vanished between resolution and focus.
        if notebook.get_session_info(target_id).is_none() {
            tracing::debug!(session = %target_id, "show session: session vanished");
            return false;
        }

        // A split pane is shown through the layout's host tab. The bridge map
        // is keyed by every session a layout shows, so the tab to select is
        // the bridge's owner, never the map key: a guest's own key names a
        // parked session with no tab, and selecting it did nothing.
        if split_view_actions::focus_split_pane(notebook, session_split_bridges, target_id)
            .is_none()
        {
            notebook.switch_to_tab(target_id);
            notebook.show_tab_view_content();
        }
        tracing::debug!(session = %target_id, "show session: focused");

        // Set input focus on the session widget where one exists (terminals);
        // embedded viewers have no VTE widget, so selecting the tab is enough.
        if let Some(terminal) = notebook.get_terminal(target_id) {
            terminal.grab_focus();
        }
        true
    }

    /// Wraps `observer` so a new session is shown in the tab view and, for a
    /// terminal session, registered in the split view for drag-and-drop, once
    /// it exists. `observer` itself runs first.
    ///
    /// New connections always open in a new tab, never in a split pane. VNC,
    /// RDP and SPICE are shown in the tab directly and are not registered.
    pub(super) fn with_split_registration(
        notebook: &SharedNotebook,
        split_view: &SharedSplitView,
        observer: Option<types::SessionStartObserver>,
    ) -> types::SessionStartObserver {
        let notebook = Rc::downgrade(notebook);
        let split_view = Rc::downgrade(split_view);
        types::SessionStartObserver::new(move |session_id| {
            if let Some(observer) = observer {
                observer.complete(session_id);
            }
            let (Some(notebook), Some(split_view)) = (notebook.upgrade(), split_view.upgrade())
            else {
                return;
            };
            let Some(info) = notebook.get_session_info(session_id) else {
                return;
            };
            if !matches!(info.protocol.as_str(), "vnc" | "rdp" | "spice") {
                // The display widget is resolved on demand via the content
                // provider.
                split_view.add_session(info);
            }
            split_view.widget().set_visible(false);
            split_view.widget().set_vexpand(false);
            notebook.widget().set_vexpand(true);
            notebook.show_tab_view_content();
        })
    }

    /// Wraps `observer` so the connection's key sequence is typed into the new
    /// session once it exists, half a second later to let the terminal
    /// initialise. `observer` itself runs first. Without a key sequence,
    /// `observer` is returned unchanged.
    pub(super) fn with_key_sequence(
        notebook: &SharedNotebook,
        conn: &rustconn_core::Connection,
        observer: Option<types::SessionStartObserver>,
    ) -> Option<types::SessionStartObserver> {
        let Some(seq) = conn.key_sequence.clone().filter(|seq| !seq.is_empty()) else {
            return observer;
        };
        let notebook = Rc::downgrade(notebook);
        let name = conn.name.clone();
        Some(types::SessionStartObserver::new(move |session_id| {
            if let Some(observer) = observer {
                observer.complete(session_id);
            }
            let Some(notebook) = notebook.upgrade() else {
                return;
            };
            // Terminal sessions only; an embedded viewer has nothing to type
            // into.
            if notebook.get_terminal(session_id).is_none() {
                return;
            }
            tracing::info!(
                connection = %name,
                elements = seq.len(),
                "Scheduling key sequence after connection"
            );
            glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || {
                notebook.execute_key_sequence(session_id, &seq);
            });
        }))
    }

    /// Starts a connection with split view integration
    pub fn start_connection_with_split(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        split_view: &SharedSplitView,
        sidebar: &SharedSidebar,
        monitoring: &types::SharedMonitoring,
        connection_id: Uuid,
        activity: Option<&types::SharedActivityCoordinator>,
    ) -> Option<Uuid> {
        Self::start_connection_with_split_observed(
            state,
            notebook,
            split_view,
            sidebar,
            monitoring,
            connection_id,
            activity,
            None,
        )
    }

    /// Starts a connection with split integration and an exact-session observer.
    #[expect(
        clippy::too_many_arguments,
        reason = "extends the stable start wrapper with one optional observer"
    )]
    pub fn start_connection_with_split_observed(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        split_view: &SharedSplitView,
        sidebar: &SharedSidebar,
        monitoring: &types::SharedMonitoring,
        connection_id: Uuid,
        activity: Option<&types::SharedActivityCoordinator>,
        observer: Option<types::SessionStartObserver>,
    ) -> Option<Uuid> {
        // Show "connecting" unless a caller already did. Most entry points set
        // it before resolving credentials, which is the moment worth reporting —
        // a vault lookup can take a second and the user should see something in
        // the sidebar meanwhile. Repeating it here walked the whole tree a second
        // time and re-notified the row for no visible change. Anything other than
        // "connecting" (no status, or "failed" from a previous attempt) still gets
        // set, so paths that arrive here without an outer call are unaffected.
        if sidebar
            .get_connection_status(&connection_id.to_string())
            .as_deref()
            != Some("connecting")
        {
            sidebar.update_connection_status(&connection_id.to_string(), "connecting");
        }

        // The tab layout and the split-view registration happen when the
        // session exists, so an asynchronous start (port check, key unlock)
        // gets them too. A synchronous start completes the observer before
        // `start_connection_observed` returns.
        let observer = Self::with_split_registration(notebook, split_view, observer);
        let session_id = match Self::start_connection_observed(
            state,
            notebook,
            sidebar,
            monitoring,
            connection_id,
            Some(observer),
        ) {
            types::ConnectionStartResult::Started(id) => id,
            types::ConnectionStartResult::Pending => {
                // Async port check in progress — keep "connecting" status.
                // The protocol callback will set "connected" or "failed".
                return None;
            }
            types::ConnectionStartResult::Failed => {
                sidebar.update_connection_status(&connection_id.to_string(), "failed");
                // Show connection failure toast with connection name
                if let Ok(state_ref) = state.try_borrow()
                    && let Some(conn) = state_ref.get_connection(connection_id)
                {
                    let name = conn.name.clone();
                    drop(state_ref);
                    crate::toast::show_error_toast_on_active_window(&crate::i18n::i18n_f(
                        "Connection to ‘{}’ failed",
                        &[&name],
                    ));
                }
                return None;
            }
        };

        // Get session info to check protocol
        if let Some(info) = notebook.get_session_info(session_id) {
            // For Zero Trust, detect connection via terminal content changes
            // (SSH status detection is handled inside start_ssh_connection_internal)
            if info.protocol.starts_with("zerotrust") {
                // Set status to connecting initially (only if not already connected)
                if sidebar
                    .get_connection_status(&connection_id.to_string())
                    .is_none()
                {
                    sidebar.update_connection_status(&connection_id.to_string(), "connecting");
                }

                // The verdict an in-place reconnect uses, counted from below
                // the connect banner, so the banner alone no longer turns the
                // session green.
                let sidebar = sidebar.clone();
                protocols::when_established(
                    notebook,
                    session_id,
                    protocols::StatusClient::Other,
                    move || {
                        sidebar.increment_session_count(&connection_id.to_string(), session_id);
                        tracing::info!(
                            protocol = "zerotrust",
                            "Terminal connection detected as established"
                        );
                    },
                );
            }
        }

        // Activity monitoring is wired centrally via the notebook's
        // `on_session_created` hook (see `MainWindow::new`), so it is NOT set
        // up here. The old per-path call only covered synchronous connects and
        // missed the async port-check path; the central hook covers both.
        let _ = activity;

        Some(session_id)
    }

    /// Starts a connection and returns the `session_id`
    pub fn start_connection(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        sidebar: &SharedSidebar,
        monitoring: &types::SharedMonitoring,
        connection_id: Uuid,
    ) -> types::ConnectionStartResult {
        Self::start_connection_observed(state, notebook, sidebar, monitoring, connection_id, None)
    }

    /// Starts a connection and observes the exact session UUID it creates.
    pub fn start_connection_observed(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        sidebar: &SharedSidebar,
        monitoring: &types::SharedMonitoring,
        connection_id: Uuid,
        observer: Option<types::SessionStartObserver>,
    ) -> types::ConnectionStartResult {
        Self::start_connection_inner(
            state,
            notebook,
            sidebar,
            monitoring,
            connection_id,
            observer,
            true,
        )
    }

    /// Starts a connection whose jump host the caller has already checked.
    ///
    /// A cluster connect checks every member's jump host in one pass and asks
    /// once for the whole set (issue #345), so each member must not repeat the
    /// check — and must not raise a dialog of its own.
    pub fn start_connection_bastion_prechecked(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        sidebar: &SharedSidebar,
        monitoring: &types::SharedMonitoring,
        connection_id: Uuid,
    ) -> types::ConnectionStartResult {
        Self::start_connection_inner(
            state,
            notebook,
            sidebar,
            monitoring,
            connection_id,
            None,
            false,
        )
    }

    pub(super) fn start_connection_inner(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        sidebar: &SharedSidebar,
        monitoring: &types::SharedMonitoring,
        connection_id: Uuid,
        observer: Option<types::SessionStartObserver>,
        check_bastion: bool,
    ) -> types::ConnectionStartResult {
        let state_ref = state.borrow();

        let Some(conn) = state_ref.get_connection(connection_id) else {
            return types::ConnectionStartResult::Failed;
        };

        // Unusable-bastion guard (#345): a `jump_host_id` — the connection's
        // own, or one inherited from a group or the global network settings —
        // can point at a connection that has since been deleted, or at the
        // connection itself. The resolve path silently skips such a hop, and
        // for a tunnel protocol or an SSH connection with no other route that
        // means connecting direct: the one outcome a bastion exists to prevent.
        // That is a security-relevant route change made without the user, so it
        // gets a blocking question (default Cancel), the same treatment as the
        // re-point guard below — a transient toast was easy to miss while the
        // session was already dialling. Only protocols that route through a
        // jump host are checked, and a "connect anyway" holds for the session.
        if check_bastion
            && let Some((_, skipped)) = state_ref
                .skipped_bastions(&[connection_id])
                .into_iter()
                .next()
        {
            let body = skipped_bastion_body(&state_ref, conn, &skipped);
            let conn_name = conn.name.clone();
            drop(state_ref);
            Self::show_skipped_bastion_warning(
                state,
                notebook,
                sidebar,
                monitoring,
                connection_id,
                observer,
                &conn_name,
                &body,
                skipped.fallback,
            );
            // The dialog drives the retry; this attempt stops here without
            // marking the sidebar failed.
            return types::ConnectionStartResult::Pending;
        }

        // Re-point guard: if this connection now goes somewhere other than the
        // last time it connected — a different host, account, credential source
        // or jump host — ask before carrying the saved credentials there. This
        // catches a shared connection silently re-pointed by someone who may
        // edit it but not read its password (see routing_memory). The check is
        // skipped once the user has confirmed the change this session.
        if !state_ref.reroute_confirmed(connection_id) {
            let (verdict, current) = state_ref.check_route(conn);
            if let rustconn_core::connection::RouteVerdict::Changed(previous) = verdict {
                let conn_name = conn.name.clone();
                drop(state_ref);
                Self::show_reroute_warning(
                    state,
                    notebook,
                    sidebar,
                    monitoring,
                    connection_id,
                    &conn_name,
                    &previous,
                    &current,
                );
                // The dialog drives the retry; this attempt stops here without
                // marking the sidebar failed.
                return types::ConnectionStartResult::Pending;
            }
        }

        // Jump host failover: the choice is made off the main thread, so a
        // connection with fallbacks comes back through here once it is known.
        // Ahead of Wake-on-LAN and the pre-connect task, which must run once.
        drop(state_ref);
        let retry = {
            let state = state.clone();
            let notebook = notebook.clone();
            let sidebar = sidebar.clone();
            let monitoring = Rc::clone(monitoring);
            let observer = observer.clone();
            move || {
                let result = Self::start_connection_inner(
                    &state,
                    &notebook,
                    &sidebar,
                    &monitoring,
                    connection_id,
                    observer,
                    false,
                );
                if matches!(result, types::ConnectionStartResult::Failed) {
                    sidebar.update_connection_status(&connection_id.to_string(), "failed");
                    crate::toast::show_error_toast_on_active_window(&crate::i18n::i18n(
                        "Connection failed",
                    ));
                }
            }
        };
        if protocols::start_gateway_probe(state, connection_id, retry) {
            return types::ConnectionStartResult::Pending;
        }
        let state_ref = state.borrow();
        let Some(conn) = state_ref.get_connection(connection_id) else {
            return types::ConnectionStartResult::Failed;
        };

        // Auto-WoL: send magic packet before connecting if configured
        // Fire-and-forget on background thread to avoid blocking GTK
        if let Some(wol_config) = conn.get_wol_config() {
            let wol_config = wol_config.clone();
            let conn_name = conn.name.clone();
            tracing::info!(
                mac = %wol_config.mac_address,
                "Sending auto-WoL before connecting to {}",
                conn_name,
            );
            std::thread::spawn(move || {
                if let Err(e) = rustconn_core::wol::send_wol_with_retry(&wol_config, 3, 500) {
                    tracing::warn!(?e, "Auto-WoL failed for {}", conn_name,);
                }
            });
        }

        let protocol = get_protocol_string(&conn.protocol_config);
        // Either switch is enough to arm session logging: the connection's own
        // Logs tab or the global one in Settings → Terminal → Logging. Before
        // 0.19.10 only the global flag was consulted, so a connection that had
        // logging configured never wrote a line (issue #247).
        let logging_enabled = state_ref.settings().logging.enabled
            || conn
                .log_config
                .as_ref()
                .is_some_and(|config| config.enabled);

        // Clone connection data before dropping borrow
        let mut conn_clone = conn.clone();
        // The credential resolution that ran before this cached the identity
        // the vault supplied (already merged with the connection's by
        // `apply_vault_identity`). Every launcher below builds its command line
        // from `conn_clone`, so this one override is what makes a vault user
        // name reach SSH, Telnet, Serial and the rest — for this session only;
        // the stored connection is not changed.
        if let Some(cached) = state_ref.get_cached_credentials(connection_id) {
            if !cached.username.trim().is_empty() {
                conn_clone.username = Some(cached.username.clone());
            }
            if !cached.domain.trim().is_empty() {
                conn_clone.domain = Some(cached.domain.clone());
            }
        }
        let spa_secret_settings = state_ref.settings().secrets.clone();
        let config_dir = state_ref.config_dir();
        drop(state_ref);

        // Issue #241: a `.local` host that only the Flatpak *host* can resolve is
        // rewritten to its address for this launch (SSH keeps the name as
        // HostKeyAlias). A no-op outside Flatpak and for every non-mDNS name.
        rustconn_core::connection::apply_mdns_fallback(&mut conn_clone);
        let conn_clone = conn_clone;

        let needs_prep = conn_clone.pre_connect_task.is_some()
            || conn_clone.knock_sequence.is_some()
            || conn_clone.spa_config.is_some();
        if !needs_prep {
            return Self::launch_prepared(
                state,
                notebook,
                sidebar,
                monitoring,
                connection_id,
                &conn_clone,
                &protocol,
                logging_enabled,
                observer,
            );
        }
        // The pre-connect task, the port knock and the SPA packet (with its
        // vault reads) can take tens of seconds, so they run on a worker
        // thread; the session starts from the callback once they are done.
        let global_variables = state
            .try_borrow()
            .ok()
            .map(|s| crate::state::resolve_global_variables(s.settings()))
            .unwrap_or_default();
        let folder_tracker = state
            .try_borrow()
            .ok()
            .map(|s| Arc::clone(s.folder_tracker()))
            .unwrap_or_default();
        let prep_conn = conn_clone.clone();
        let state = state.clone();
        let notebook = notebook.clone();
        let sidebar = sidebar.clone();
        let monitoring = Rc::clone(monitoring);
        crate::utils::spawn_blocking_with_callback(
            move || {
                Self::run_pre_connect_stage(
                    &prep_conn,
                    global_variables,
                    folder_tracker,
                    &spa_secret_settings,
                    &config_dir,
                )
            },
            move |prepared| {
                let failure = match prepared {
                    Ok(()) => {
                        let result = Self::launch_prepared(
                            &state,
                            &notebook,
                            &sidebar,
                            &monitoring,
                            connection_id,
                            &conn_clone,
                            &protocol,
                            logging_enabled,
                            observer,
                        );
                        if !matches!(result, types::ConnectionStartResult::Failed) {
                            return;
                        }
                        crate::i18n::i18n("Connection failed")
                    }
                    Err(message) => message,
                };
                sidebar.update_connection_status(&connection_id.to_string(), "failed");
                crate::toast::show_error_toast_on_active_window(&failure);
            },
        );
        types::ConnectionStartResult::Pending
    }
    /// Starts the session once the pre-connect stage is done (or not needed).
    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors the launch entry points it must call back into"
    )]
    fn launch_prepared(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        sidebar: &SharedSidebar,
        monitoring: &types::SharedMonitoring,
        connection_id: Uuid,
        conn_clone: &rustconn_core::models::Connection,
        protocol: &str,
        logging_enabled: bool,
        observer: Option<types::SessionStartObserver>,
    ) -> types::ConnectionStartResult {
        // Protocols that use async port check return None when the check is
        // in progress — this is NOT a failure.  We track whether the protocol
        // *may* be pending so we can distinguish Pending from Failed below.
        let may_be_pending = matches!(
            protocol,
            "ssh" | "vnc" | "spice" | "telnet" | "winrm" | "mosh"
        );

        // The key sequence goes out when the session exists, which for an
        // asynchronous start (port check, key unlock) is after this returns.
        let observer = Self::with_key_sequence(notebook, conn_clone, observer);

        let session_id = match protocol {
            "ssh" => protocols::start_ssh_connection_observed(
                state,
                notebook,
                sidebar,
                monitoring,
                connection_id,
                conn_clone,
                logging_enabled,
                observer.clone(),
            ),
            "vnc" => protocols::start_vnc_connection_observed(
                state,
                notebook,
                sidebar,
                connection_id,
                conn_clone,
                observer.clone(),
            ),
            "rdp" => {
                // RDP connections are handled by start_rdp_session_with_credentials
                // which is called from start_connection_with_credential_resolution
                tracing::warn!(
                    "RDP connection reached start_connection without credentials. \
                     Use start_connection_with_credential_resolution instead."
                );
                None
            }
            "spice" => protocols::start_spice_connection(
                state,
                notebook,
                sidebar,
                connection_id,
                conn_clone,
            ),
            "telnet" | "winrm" => protocols::start_telnet_connection_observed(
                state,
                notebook,
                sidebar,
                connection_id,
                conn_clone,
                logging_enabled,
                observer.clone(),
            ),
            "serial" => protocols::start_serial_connection(
                state,
                notebook,
                sidebar,
                connection_id,
                conn_clone,
                logging_enabled,
            ),
            "kubernetes" => protocols::start_kubernetes_connection(
                state,
                notebook,
                sidebar,
                connection_id,
                conn_clone,
                logging_enabled,
            ),
            "mosh" => protocols::start_mosh_connection_observed(
                state,
                notebook,
                sidebar,
                connection_id,
                conn_clone,
                logging_enabled,
                observer.clone(),
            ),
            p if p == "zerotrust" || p.starts_with("zerotrust:") => {
                protocols::start_zerotrust_connection(
                    state,
                    notebook,
                    sidebar,
                    connection_id,
                    conn_clone,
                    logging_enabled,
                )
            }
            "sftp" => {
                // SFTP opens file manager — no terminal session
                Self::handle_sftp_connect(state, notebook, Some(sidebar), None, connection_id);
                None
            }
            "web" => {
                // Web: Embedded mode opens in-tab, System/Custom open externally
                Self::handle_web_connect_observed(
                    state,
                    notebook,
                    sidebar,
                    connection_id,
                    observer.clone(),
                );
                None
            }
            _ => {
                // Unknown protocol
                None
            }
        };

        // A synchronous start; an asynchronous one completes the observer
        // itself. Only the first completion counts.
        if let (Some(session_id), Some(observer)) = (session_id, observer) {
            observer.complete(session_id);
        }

        let result = match session_id {
            Some(sid) => types::ConnectionStartResult::Started(sid),
            None if may_be_pending => types::ConnectionStartResult::Pending,
            None => types::ConnectionStartResult::Failed,
        };

        // Record where this connection went, so a later re-point is noticed.
        // Only for a launch that actually started (or is asynchronously
        // starting) — a failed launch must not overwrite the known-good route,
        // or a re-point followed by a deliberate failure could launder itself in.
        if matches!(
            result,
            types::ConnectionStartResult::Started(_) | types::ConnectionStartResult::Pending
        ) && let Ok(state_ref) = state.try_borrow()
            && let Some(conn) = state_ref.get_connection(connection_id)
        {
            let (_, route) = state_ref.check_route(conn);
            drop(state_ref);
            state.borrow().remember_route(connection_id, route);
        }

        result
    }
    /// The blocking steps before a session starts: the pre-connect task, the
    /// port knock and the fwknop SPA packet. Runs on a worker thread; an `Err`
    /// is the message to show, already translated.
    fn run_pre_connect_stage(
        conn_clone: &rustconn_core::models::Connection,
        global_variables: Vec<rustconn_core::Variable>,
        folder_tracker: Arc<std::sync::Mutex<rustconn_core::automation::FolderConnectionTracker>>,
        secrets: &rustconn_core::config::SecretSettings,
        config_dir: &std::path::Path,
    ) -> Result<(), String> {
        if let Some(ref task) = conn_clone.pre_connect_task {
            tracing::info!(
                connection = %conn_clone.name,
                command = %task.command,
                "Executing pre-connect task"
            );
            let mut var_manager = VariableManager::new();
            for var in &global_variables {
                var_manager.set_global(var.clone());
            }
            let conn_id = conn_clone.id;
            var_manager.set_connection(
                conn_id,
                rustconn_core::Variable::new("host", &conn_clone.host),
            );
            var_manager.set_connection(
                conn_id,
                rustconn_core::Variable::new("port", conn_clone.port.to_string()),
            );
            if let Some(ref user) = conn_clone.username {
                var_manager.set_connection(conn_id, rustconn_core::Variable::new("username", user));
            }
            var_manager.set_connection(
                conn_id,
                rustconn_core::Variable::new("name", &conn_clone.name),
            );
            let executor = TaskExecutor::with_tracker(Arc::new(var_manager), folder_tracker);
            let folder_id = conn_clone.group_id;
            // ponytail: 60s ceiling so a task without a timeout cannot hold the
            // connect attempt forever. The task's own timeout fires first.
            let ceiling = std::time::Duration::from_mins(1);
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| e.to_string())
                .map(|rt| {
                    rt.block_on(async {
                        tokio::time::timeout(
                            ceiling,
                            executor.execute_pre_connect(
                                task,
                                VariableScope::Connection(conn_id),
                                folder_id,
                            ),
                        )
                        .await
                        .unwrap_or(Err(rustconn_core::automation::TaskError::Timeout(60_000)))
                    })
                });
            match result {
                Ok(Ok(_)) => {
                    tracing::info!(
                        connection = %conn_clone.name,
                        "Pre-connect task completed successfully"
                    );
                }
                Ok(Err(e)) => {
                    tracing::error!(
                        connection = %conn_clone.name,
                        command = %task.command,
                        error = %e,
                        "Pre-connect task failed"
                    );
                    if task.abort_on_failure {
                        return Err(crate::i18n::i18n_f(
                            "Pre-connect task failed: {}",
                            &[&e.to_string()],
                        ));
                    }
                }
                Err(runtime_err) => {
                    tracing::error!(
                        connection = %conn_clone.name,
                        error = %runtime_err,
                        "Failed to create async runtime for pre-connect task"
                    );
                    if task.abort_on_failure {
                        return Err(crate::i18n::i18n_f(
                            "Pre-connect task failed: {}",
                            &[&runtime_err],
                        ));
                    }
                }
            }
        }
        if let Some(ref knock_seq) = conn_clone.knock_sequence {
            tracing::info!(
                connection = %conn_clone.name,
                knocks = knock_seq.knocks.len(),
                "Executing port knock sequence"
            );
            match rustconn_core::connection::knock::execute_knock_sequence(
                &conn_clone.host,
                knock_seq,
            ) {
                Ok(result) => {
                    tracing::info!(
                        connection = %conn_clone.name,
                        total_ms = result.total_ms,
                        "Port knock sequence completed"
                    );
                }
                Err(e) => {
                    tracing::error!(
                        connection = %conn_clone.name,
                        error = %e,
                        "Port knock sequence failed"
                    );
                    return Err(crate::i18n::i18n_f(
                        "Port knock failed: {}",
                        &[&e.to_string()],
                    ));
                }
            }
        }
        // The fwknop SPA packet goes out after the knock, before the port
        // check. The keys come from the secret store (F-08); a key still in
        // the file (pre-0.26.0 or not yet moved) is used as is.
        let spa_keys = match conn_clone.spa_config.as_ref() {
            Some(cfg) => match rustconn_core::connection::spa_vault::resolve_spa_keys(
                conn_clone.id,
                cfg,
                &crate::vault_ops::VaultSpaKeyStore(secrets),
            ) {
                Ok(keys) => keys,
                Err(e) => {
                    tracing::error!(connection = %conn_clone.name, error = %e, "SPA keys unavailable");
                    return Err(crate::i18n::i18n_f("SPA failed: {}", &[&e.to_string()]));
                }
            },
            None => None,
        };
        if let Some(ref spa_cfg) = conn_clone.spa_config
            && let Some((rij_secret, hmac_secret)) = spa_keys
        {
            let username = conn_clone.username.as_deref().unwrap_or("root");
            tracing::info!(
                connection = %conn_clone.name,
                dest_port = spa_cfg.dest_port,
                access = %spa_cfg.access,
                "Sending fwknop SPA packet"
            );
            match rustconn_core::connection::spa::send_spa(
                &conn_clone.host,
                spa_cfg.dest_port,
                &rij_secret,
                &hmac_secret,
                &spa_cfg.access,
                username,
            ) {
                Ok(result) => {
                    tracing::info!(
                        connection = %conn_clone.name,
                        packet_size = result.packet_size,
                        elapsed_ms = result.elapsed_ms,
                        "SPA packet sent successfully"
                    );
                    // The keys came out of the vault and worked, so the
                    // pre-move copy of connections.toml (it still holds them
                    // in clear text) has done its job.
                    if (spa_cfg.rijndael_key_in_vault || spa_cfg.hmac_key_in_vault)
                        && rustconn_core::connection::spa_vault::remove_pre_vault_backup(config_dir)
                    {
                        tracing::info!("Removed connections.toml.pre-spa-vault.bak");
                    }
                }
                Err(e) => {
                    tracing::error!(
                        connection = %conn_clone.name,
                        error = %e,
                        "SPA packet send failed"
                    );
                    return Err(crate::i18n::i18n_f("SPA failed: {}", &[&e.to_string()]));
                }
            }
        }
        Ok(())
    }

    /// Shows the "routing changed" warning and, on confirmation, retries the
    /// connection.
    ///
    /// Presents the previous and current route side by side. "Cancel" leaves
    /// the connection untouched and clears the sidebar's "connecting" state;
    /// "Connect anyway" records the acceptance for this session and starts the
    /// connection again, which now passes the guard.
    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors the launch entry points it must call back into"
    )]
    pub(super) fn show_reroute_warning(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        sidebar: &SharedSidebar,
        monitoring: &types::SharedMonitoring,
        connection_id: Uuid,
        conn_name: &str,
        previous: &rustconn_core::connection::Route,
        current: &rustconn_core::connection::Route,
    ) {
        use crate::i18n::{i18n, i18n_f};

        let describe = |r: &rustconn_core::connection::Route| -> String {
            let mut who = if r.username.is_empty() {
                r.host.clone()
            } else {
                format!("{}@{}", r.username, r.host)
            };
            if r.port != 22 {
                who = format!("{who}:{}", r.port);
            }
            if !r.jump.is_empty() {
                who = i18n_f("{} (via {})", &[&who, &r.jump]);
            }
            who
        };

        let body = i18n_f(
            "This connection no longer goes where it did.\n\nPreviously connected to:\n{}\n\nNow points to:\n{}\n\nIf you did not make this change, cancel — connecting could send saved credentials to a different machine.",
            &[&describe(previous), &describe(current)],
        );

        let dialog = adw::AlertDialog::new(
            Some(&i18n_f("Routing changed for “{}”", &[conn_name])),
            Some(&body),
        );
        dialog.add_response("cancel", &i18n("Cancel"));
        dialog.add_response("connect", &i18n("Connect Anyway"));
        dialog.set_response_appearance("connect", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");

        let state_cb = state.clone();
        let notebook_cb = notebook.clone();
        let sidebar_cb = sidebar.clone();
        let monitoring_cb = Rc::clone(monitoring);
        dialog.connect_response(None, move |_, response| {
            if response == "connect" {
                state_cb.borrow().confirm_reroute(connection_id);
                Self::start_connection(
                    &state_cb,
                    &notebook_cb,
                    &sidebar_cb,
                    &monitoring_cb,
                    connection_id,
                );
            } else {
                // Cancelled — undo the "connecting" indication the click set.
                sidebar_cb.update_connection_status(&connection_id.to_string(), "disconnected");
            }
        });

        if let Some(root) = notebook.widget().root() {
            dialog.present(Some(&root));
        } else {
            dialog.present(None::<&gtk4::Widget>);
        }
    }

    /// Asks before connecting without an unusable jump host (issue #345) and,
    /// on confirmation, retries the connection.
    ///
    /// "Cancel" (the default and the Escape response) leaves the connection
    /// untouched and clears the sidebar's "connecting" state. Confirming records
    /// the choice for this session and starts the connection again, which now
    /// passes the guard. The button names the outcome: "Connect Directly" when
    /// nothing else routes the connection, "Connect Anyway" when the rest of its
    /// route still applies.
    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors the launch entry points it must call back into"
    )]
    pub(super) fn show_skipped_bastion_warning(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        sidebar: &SharedSidebar,
        monitoring: &types::SharedMonitoring,
        connection_id: Uuid,
        observer: Option<types::SessionStartObserver>,
        conn_name: &str,
        body: &str,
        fallback: rustconn_core::connection::jump_chain::BastionFallback,
    ) {
        use crate::i18n::{i18n, i18n_f};
        use rustconn_core::connection::jump_chain::BastionFallback;

        let dialog = adw::AlertDialog::new(
            Some(&i18n_f("Jump Host Unavailable for “{}”", &[conn_name])),
            Some(body),
        );
        dialog.add_response("cancel", &i18n("Cancel"));
        let connect_label = match fallback {
            BastionFallback::Direct => i18n("Connect Directly"),
            BastionFallback::RemainingRoute => i18n("Connect Anyway"),
        };
        dialog.add_response("connect", &connect_label);
        dialog.set_response_appearance("connect", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");

        let state_cb = state.clone();
        let notebook_cb = notebook.clone();
        let sidebar_cb = sidebar.clone();
        let monitoring_cb = Rc::clone(monitoring);
        dialog.connect_response(None, move |_, response| {
            if response == "connect" {
                state_cb.borrow().confirm_bastion_skip(connection_id);
                Self::start_connection_observed(
                    &state_cb,
                    &notebook_cb,
                    &sidebar_cb,
                    &monitoring_cb,
                    connection_id,
                    observer.clone(),
                );
            } else {
                // Cancelled — undo the "connecting" indication the click set.
                sidebar_cb.update_connection_status(&connection_id.to_string(), "disconnected");
            }
        });

        if let Some(root) = notebook.widget().root() {
            dialog.present(Some(&root));
        } else {
            dialog.present(None::<&gtk4::Widget>);
        }
    }
}
