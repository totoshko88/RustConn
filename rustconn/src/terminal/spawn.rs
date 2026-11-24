//! Spawning child processes into terminal tabs (local command, SSH, Telnet, serial).
//!
//! Moved out of `terminal/mod.rs`; behaviour is unchanged.

use super::*;

impl TerminalNotebook {
    /// Spawns a session command on its own PTY and attaches it to the terminal.
    ///
    /// `envv` entries override the inherited environment assembled by
    /// [`build_child_env`]. Returns `false` when the session has no terminal or
    /// the command could not be started; a failure also marks the tab
    /// disconnected and offers a reconnect banner.
    pub fn spawn_command(
        &self,
        session_id: Uuid,
        argv: &[&str],
        envv: Option<&[&str]>,
        working_directory: Option<&str>,
        ssh_agent_socket: Option<&str>,
    ) -> bool {
        self.spawn_command_with_cleanup(
            session_id,
            argv,
            envv,
            working_directory,
            ssh_agent_socket,
            ChildCleanup::default(),
        )
    }

    /// [`Self::spawn_command`], running `cleanup` when the child exits or the
    /// spawn fails, whether or not the tab is still open.
    pub(crate) fn spawn_command_with_cleanup(
        &self,
        session_id: Uuid,
        argv: &[&str],
        envv: Option<&[&str]>,
        working_directory: Option<&str>,
        ssh_agent_socket: Option<&str>,
        cleanup: ChildCleanup,
    ) -> bool {
        let Some(terminal) = self.get_terminal(session_id) else {
            return false;
        };

        // A reconnect starts a new child on the same terminal, so the previous
        // session's relay has to go first: dropping it stops its threads and
        // closes its descriptors, and leaving it would keep feeding the widget
        // from a PTY nobody is on any more.
        self.teardown_relay(session_id);

        let env_vec = build_child_env(envv, ssh_agent_socket);
        let env_refs: Vec<&str> = env_vec.iter().map(|e| e.as_str()).collect();
        // Diagnostics keep naming the session's own command, not `sh`, so a
        // "command not found" message still names the thing the user configured.
        let command_name = (*argv.first().unwrap_or(&"")).to_owned();
        let size = grid_size(&terminal);

        // An output filter turns the command into a shell pipeline. Built here
        // rather than in each launcher because this is the one point every
        // terminal protocol passes through.
        let filtered = self.wrap_in_output_filter(session_id, argv);
        let spawn_argv: Vec<&str> = match filtered.as_deref() {
            Some(script) => vec!["sh", "-c", script],
            None => argv.to_vec(),
        };

        // The session's own argv, not `spawn_argv`: with a filter the latter is
        // the whole `sh -c` script with the session command quoted inside it, so
        // logging it would put the same content in the log twice as much noise.
        // Which filter was chosen is already logged, by name, in
        // `set_output_filter`.
        tracing::debug!(
            command = %command_name,
            %session_id,
            argv = ?argv,
            working_directory = ?working_directory,
            env_count = env_refs.len(),
            rows = size.0,
            cols = size.1,
            filtered = filtered.is_some(),
            "Spawning session command"
        );

        let child = match pty_spawn::spawn_on_pty(&spawn_argv, &env_refs, working_directory, size) {
            Ok(child) => child,
            Err(e) => {
                tracing::error!(
                    command = %command_name,
                    %session_id,
                    %e,
                    "Failed to spawn session command"
                );
                self.show_spawn_failure(session_id, &command_name, &e);
                return false;
            }
        };

        let (relay, output) = match pty_relay::PtyRelay::start(child.master, size) {
            Ok(started) => started,
            Err(e) => {
                // The child is already running on a PTY nothing will read, so
                // it has to go rather than linger on an invisible terminal.
                reap_child(child.pid);
                tracing::error!(
                    command = %command_name,
                    %session_id,
                    %e,
                    "Failed to start the PTY relay"
                );
                self.show_spawn_failure(
                    session_id,
                    &command_name,
                    &pty_spawn::SpawnError::Failed(e.to_string()),
                );
                return false;
            }
        };

        self.pty_relays.borrow_mut().insert(session_id, relay);
        self.vte_child_pids
            .borrow_mut()
            .insert(session_id, child.pid as i32);

        self.deliver_output_to(session_id, &terminal, output);
        self.forward_input_from(session_id, &terminal);
        self.watch_grid_size(session_id, &terminal);
        watch_child_exit(&terminal, child.pid, cleanup);

        true
    }

    /// Marks a session's tab as failed and explains why.
    ///
    /// A missing CLI tool is the common case and gets its own wording, because
    /// "not installed" tells the user what to do while an errno does not. The
    /// banner carries a Reconnect button so the tab stays useful after the tool
    /// has been installed.
    fn show_spawn_failure(
        &self,
        session_id: Uuid,
        command_name: &str,
        error: &pty_spawn::SpawnError,
    ) {
        let not_found = error.is_not_found();
        // A command that never started leaves no child to exit, so the sidebar
        // would stay on "connecting" for good; report the failure here.
        let failed_connection = self
            .session_info
            .borrow()
            .get(&session_id)
            .map(|i| i.connection_id);
        if let Some(connection_id) = failed_connection
            && let Some(ref cb) = *self.on_spawn_failed.borrow()
        {
            cb(connection_id);
        }

        if let Some(page) = self.sessions.borrow().get(&session_id) {
            page.set_indicator_icon(Some(&gio::ThemedIcon::new("network-offline-symbolic")));
            page.set_indicator_tooltip(&i18n("Session disconnected"));
            page.set_indicator_activatable(false);

            if let Ok(outer) = page.child().downcast::<GtkBox>()
                && let Some(inner) = outer.first_child()
                && let Ok(container) = inner.downcast::<GtkBox>()
            {
                let connection_id = self
                    .session_info
                    .borrow()
                    .get(&session_id)
                    .map_or_else(Uuid::nil, |i| i.connection_id);

                let banner = GtkBox::new(Orientation::Horizontal, 6);
                banner.set_margin_start(12);
                banner.set_margin_end(12);
                banner.set_margin_top(6);
                banner.set_margin_bottom(6);
                banner.set_halign(gtk4::Align::Center);
                banner.set_widget_name("reconnect-banner");

                let label = gtk4::Label::new(Some(&if not_found {
                    i18n_f("Command not found: {}", &[command_name])
                } else {
                    i18n_f("Failed to start '{}'", &[command_name])
                }));
                label.add_css_class("dim-label");

                let button = gtk4::Button::with_label(&i18n("Reconnect"));
                button.add_css_class("suggested-action");
                button.set_tooltip_text(Some(&i18n("Reconnect to this session")));

                banner.append(&label);
                banner.append(&button);
                container.append(&banner);

                let on_reconnect = self.on_reconnect.clone();
                button.connect_clicked(move |_| {
                    if let Some(ref cb) = *on_reconnect.borrow() {
                        cb(session_id, connection_id);
                    }
                });
            }
        }

        let msg = if not_found {
            i18n_f("'{}' is not installed", &[command_name])
        } else {
            i18n_f(
                "Failed to start '{}': {}",
                &[command_name, &error.to_string()],
            )
        };
        crate::toast::show_error_toast_on_active_window(&msg);
    }

    /// Spawns an SSH command in the terminal
    ///
    /// Unlike [`Self::spawn_telnet`], this does **not** apply the connection's
    /// erase mode: every caller must call [`Self::set_erase_mode`] itself with
    /// `conn.protocol_config.erase_modes()` before spawning, or the session
    /// silently falls back to the global defaults (issue
    /// [#271](https://github.com/totoshko88/RustConn/issues/271)). The two
    /// bytes are not parameters here because this signature is already at the
    /// point where the project's argument limit is waived below, and one of the
    /// callers (Quick Connect in `window::edit_dialogs`) has no stored
    /// connection to read them from — it wants the defaults.
    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
    pub fn spawn_ssh(
        &self,
        session_id: Uuid,
        host: &str,
        port: u16,
        username: Option<&str>,
        identity_file: Option<&str>,
        extra_args: &[&str],
        use_waypipe: bool,
        ssh_agent_socket: Option<&str>,
        startup_command: Option<&str>,
        extra_env: Option<&[&str]>,
        use_mptcp: bool,
    ) -> bool {
        self.spawn_ssh_with_cleanup(
            session_id,
            host,
            port,
            username,
            identity_file,
            extra_args,
            use_waypipe,
            ssh_agent_socket,
            startup_command,
            extra_env,
            use_mptcp,
            Vec::new(),
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; cleanup ownership is the only additional concern"
    )]
    pub(crate) fn spawn_ssh_with_cleanup(
        &self,
        session_id: Uuid,
        host: &str,
        port: u16,
        username: Option<&str>,
        identity_file: Option<&str>,
        extra_args: &[&str],
        use_waypipe: bool,
        ssh_agent_socket: Option<&str>,
        startup_command: Option<&str>,
        extra_env: Option<&[&str]>,
        use_mptcp: bool,
        cleanup_paths: Vec<PathBuf>,
    ) -> bool {
        let mut argv = if use_waypipe {
            if use_mptcp {
                vec!["mptcpize", "run", "waypipe", "ssh"]
            } else {
                vec!["waypipe", "ssh"]
            }
        } else if use_mptcp {
            vec!["mptcpize", "run", "ssh"]
        } else {
            vec!["ssh"]
        };

        let port_str;
        if port != 22 {
            port_str = port.to_string();
            argv.push("-p");
            argv.push(&port_str);
        }

        if let Some(key) = identity_file {
            argv.push("-i");
            argv.push(key);
        }

        // Always enable ControlMaster so monitoring can multiplex over the
        // same authenticated connection without a second key/passphrase prompt.
        // If the user already set ControlMaster via extra_args (build_command_args),
        // skip to avoid duplicates. But always ensure ControlPath is set to the
        // shared path so monitoring can find the socket.

        let has_control_master = extra_args.iter().any(|a| a.contains("ControlMaster"));
        let has_control_path = extra_args.iter().any(|a| a.contains("ControlPath"));
        let control_path_opt = format!(
            "ControlPath={}",
            rustconn_core::ssh_control_path(host, port)
        );
        if !has_control_master {
            argv.push("-o");
            argv.push("ControlMaster=auto");
            argv.push("-o");
            argv.push(&control_path_opt);
            argv.push("-o");
            // ponytail: 60s persist keeps the master alive briefly for monitoring
            // multiplex, but dies fast after network changes (#217). Was 10m.
            argv.push("ControlPersist=60");
        } else if !has_control_path {
            // User enabled ControlMaster manually but no ControlPath —
            // add our shared path so monitoring can reuse the socket.
            argv.push("-o");
            argv.push(&control_path_opt);
        }

        // In Flatpak, ~/.ssh is read-only — use a writable known_hosts path
        // unless the caller already set UserKnownHostsFile via extra_args
        let kh_option;
        let has_known_hosts_opt = extra_args.iter().any(|a| a.contains("UserKnownHostsFile"));
        if !has_known_hosts_opt
            && let Some(option) = rustconn_core::get_flatpak_known_hosts_path()
                .and_then(|kh_path| rustconn_core::ssh_tunnel::user_known_hosts_option(&kh_path))
        {
            kh_option = option;
            argv.push("-o");
            argv.push(&kh_option);
        }

        // Keep-alive defaults (ServerAliveInterval=15, ServerAliveCountMax=3)
        // are now emitted by `SshConfig::build_command_args` for every SSH
        // command — the GUI terminal, the CLI, and the tunnels — so they already
        // sit in `extra_args`. They used to be injected here, which meant only
        // GUI sessions got them; the fallback moved into the shared builder in
        // 0.21.11 so "all SSH sessions" is finally true (#217).
        argv.extend(extra_args);

        let destination = if let Some(user) = username {
            format!("{user}@{host}")
        } else {
            host.to_string()
        };
        argv.push(&destination);

        // Append startup command after destination — runs the command and then
        // drops into an interactive login shell so the session stays open.
        // Uses `-t` to force PTY allocation (required for interactive shell after command).
        let startup_wrapped;
        if let Some(cmd) = startup_command {
            // Insert -t before destination to force PTY allocation
            // (skip if already present in extra_args to avoid duplicates)
            if !extra_args.contains(&"-t") {
                let dest_idx = argv.len() - 1;
                argv.insert(dest_idx, "-t");
            }
            // Wrap: run the command, then exec the user's login shell
            startup_wrapped = format!("{cmd}; exec $SHELL -l");
            argv.push(&startup_wrapped);
        }

        self.spawn_command_with_cleanup(
            session_id,
            &argv,
            extra_env,
            None,
            ssh_agent_socket,
            cleanup_paths.into(),
        )
    }

    /// Points a session's Backspace and Delete keys at the configured bytes.
    ///
    /// Applies to the session's live VTE widget, so it has to run after the tab
    /// exists and after [`config::configure_terminal_with_settings`] has
    /// installed the defaults — see [`config::apply_erase_mode`]. Re-applying it
    /// on every spawn is what keeps the choice in force across a reconnect into
    /// the same terminal. A session that has already gone away is ignored.
    pub fn set_erase_mode(
        &self,
        session_id: Uuid,
        backspace_sends: BackspaceSends,
        delete_sends: DeleteSends,
    ) {
        if let Some(terminal) = self.terminals.borrow().get(&session_id) {
            config::apply_erase_mode(terminal, backspace_sends, delete_sends);
        }
    }

    /// Spawns a Telnet command in the terminal
    ///
    /// Supports configurable backspace/delete key behavior via VTE
    /// `EraseBinding`. Settings are applied directly on the terminal
    /// widget before spawning the telnet process.
    pub fn spawn_telnet(
        &self,
        session_id: Uuid,
        host: &str,
        port: u16,
        extra_args: &[&str],
        backspace_sends: BackspaceSends,
        delete_sends: DeleteSends,
    ) -> bool {
        self.set_erase_mode(session_id, backspace_sends, delete_sends);

        // Spawn telnet directly — no shell wrapper needed
        let mut argv = vec!["telnet"];
        argv.extend(extra_args);
        argv.push(host);
        let port_str = port.to_string();
        argv.push(&port_str);
        self.spawn_command(session_id, &argv, None, None, None)
    }

    /// Spawns a serial connection using picocom in the terminal tab.
    ///
    /// Builds the picocom command from the `SerialConfig` and spawns it
    /// directly in the VTE terminal (no shell wrapper).
    pub fn spawn_serial(&self, session_id: Uuid, command: &[String]) -> bool {
        let argv: Vec<&str> = command.iter().map(String::as_str).collect();
        self.spawn_command(session_id, &argv, None, None, None)
    }
}
