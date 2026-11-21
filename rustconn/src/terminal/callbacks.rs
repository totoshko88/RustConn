//! Callback registration and notification: `set_on_*`, `connect_*` signal hooks and coordinators.
//!
//! Moved out of `terminal/mod.rs`; behaviour is unchanged.

use super::*;

impl TerminalNotebook {
    /// Connects a callback for when a terminal child exits.
    ///
    /// `hook` identifies the caller. Registering the same hook twice for a
    /// session replaces the earlier handler; different hooks coexist. Both
    /// halves matter: an in-place reconnect re-registers
    /// [`ChildExitHook::Disconnect`] on the same VTE widget and a leftover
    /// handler would run the whole disconnect path once per reconnect the
    /// session ever had (two post-disconnect tasks, two host-probe polls, the
    /// sidebar's active-session count decremented twice for one exit) — while
    /// evicting a *different* caller's handler is what left the reconnect
    /// banner unreachable whenever session logging was enabled (issue #297).
    pub fn connect_child_exited<F>(&self, session_id: Uuid, hook: ChildExitHook, callback: F)
    where
        F: Fn(i32) + 'static,
    {
        if let Some(terminal) = self.get_terminal(session_id) {
            if let Some((previous_terminal, handler)) = self
                .child_exited_handlers
                .borrow_mut()
                .remove(&(session_id, hook))
                && let Some(previous_terminal) = previous_terminal.upgrade()
            {
                previous_terminal.disconnect(handler);
            }

            let pids = self.vte_child_pids.clone();
            let handler = terminal.connect_child_exited(move |_terminal, status| {
                // Remove PID — process already exited, no need to kill on tab close
                pids.borrow_mut().remove(&session_id);
                callback(status);
            });

            self.child_exited_handlers
                .borrow_mut()
                .insert((session_id, hook), (terminal.downgrade(), handler));
        } else {
            // The signal is attached to the widget, so with no terminal there
            // is nothing to attach to and the caller's cleanup would never run.
            // Silent before, and silence here is how issue #297 stayed
            // invisible for two releases.
            tracing::warn!(
                %session_id,
                ?hook,
                "No terminal for session — child-exited handler not connected"
            );
        }
    }

    /// Connects a callback for terminal output (for logging)
    pub fn connect_contents_changed<F>(&self, session_id: Uuid, callback: F)
    where
        F: Fn() + 'static,
    {
        if let Some(terminal) = self.get_terminal(session_id) {
            terminal.connect_contents_changed(move |_terminal| {
                callback();
            });
        }
    }

    /// Connects a callback for cursor movement in terminal.
    ///
    /// `cursor-moved` fires more reliably than `contents-changed` for output
    /// that uses cursor positioning escape sequences without a trailing newline
    /// (e.g. SSH password prompts in no-echo mode). See issue #194.
    pub fn connect_cursor_moved<F>(&self, session_id: Uuid, callback: F)
    where
        F: Fn() + 'static,
    {
        if let Some(terminal) = self.get_terminal(session_id) {
            terminal.connect_cursor_moved(move |_terminal| {
                callback();
            });
        }
    }

    /// Connects a callback for user input (commit signal - data sent to PTY)
    pub fn connect_commit<F>(&self, session_id: Uuid, callback: F)
    where
        F: Fn(&str) + 'static,
    {
        if let Some(terminal) = self.get_terminal(session_id) {
            terminal.connect_commit(move |_terminal, text, _size| {
                callback(text);
            });
        }
    }

    /// Sets the callback to be invoked when a page is closed.
    ///
    /// The callback receives the session ID and connection ID of the closed page.
    /// This is used to update the sidebar status when SSH tabs are closed via TabView.
    ///
    /// # Arguments
    ///
    /// * `callback` - A closure that takes (session_id, connection_id) as parameters
    pub fn set_on_page_closed<F>(&self, callback: F)
    where
        F: Fn(Uuid, Uuid) + 'static,
    {
        *self.on_page_closed.borrow_mut() = Some(Box::new(callback));
    }

    /// Sets the persistent all-protocol callback fired when a session is shown.
    ///
    /// The callback receives `(session_id, connection_id)` and fires from
    /// [`Self::notify_tab_added`], which every protocol's tab-creation path
    /// calls. Unlike the one-shot `on_tab_added`, this slot is restored after
    /// firing. Wired from the window to drive auto-hide-on-connect (issue #401).
    pub fn set_on_session_shown<F>(&self, callback: F)
    where
        F: Fn(Uuid, Uuid) + 'static,
    {
        *self.on_session_shown.borrow_mut() = Some(Box::new(callback));
    }

    /// Sets the callback invoked when a new terminal session tab is created.
    ///
    /// The callback receives `(session_id, connection_id)`. It fires from
    /// [`Self::create_terminal_tab_with_settings`] — the single choke point
    /// for all terminal protocols and for both synchronous and async
    /// (port-checked) connection paths. Used to wire activity monitoring.
    pub fn set_on_session_created<F>(&self, callback: F)
    where
        F: Fn(Uuid, Uuid) + 'static,
    {
        *self.on_session_created.borrow_mut() = Some(Box::new(callback));
    }

    /// Sets a callback fired when ANY tab is added (all protocols).
    ///
    /// Unlike `on_session_created` (terminal-only), this fires for VNC, SPICE,
    /// embedded RDP, and external-process tabs too. Designed for one-shot use
    /// by workspace restore: the callback should clear itself once the target
    /// session is detected.
    pub fn set_on_tab_added<F>(&self, callback: F)
    where
        F: Fn(Uuid, Uuid) + 'static,
    {
        *self.on_tab_added.borrow_mut() = Some(Box::new(callback));
    }

    /// Fires the `on_tab_added` callback if set.
    pub(super) fn notify_tab_added(&self, session_id: Uuid, connection_id: Uuid) {
        // Cluster membership is resolved here rather than in each creation path
        // so that every protocol is covered, and before the callback so an
        // observer sees the tab already labelled with its cluster.
        self.resolve_cluster_pending(connection_id, session_id);

        // Take the callback out to avoid holding a borrow across the call —
        // the callback may set `on_tab_added` itself, which borrows it too.
        let callback = self.on_tab_added.borrow_mut().take();
        if let Some(cb) = callback {
            cb(session_id, connection_id);
            // Restore the callback for future tab creations UNLESS it was
            // consumed. Convention: the callback sets the `on_tab_added` slot
            // to a new value (or the slot stays None if consumed). If the slot
            // is still None after the call, the callback was NOT self-clearing
            // from within (because take already emptied it), so we restore.
            // If the workspace callback wants to signal "done", it must NOT
            // clear the slot — instead it signals via a shared flag
            // captured in the closure. We simply always restore here; the
            // workspace code uses an `Rc<Cell<bool>>` to stop re-firing.
            let mut slot = self.on_tab_added.borrow_mut();
            if slot.is_none() {
                *slot = Some(cb);
            }
        }

        // Auto-hide-on-connect and any future "a session appeared" observers.
        // Taken out of the slot so the callback may re-enter the notebook
        // without a BorrowMutError; restored afterwards because, unlike
        // on_tab_added, this one is persistent (issue #401).
        let shown_cb = self.on_session_shown.borrow_mut().take();
        if let Some(cb) = shown_cb {
            cb(session_id, connection_id);
            let mut slot = self.on_session_shown.borrow_mut();
            if slot.is_none() {
                *slot = Some(cb);
            }
        }
    }

    /// Sets the callback invoked when terminal (VTE) focus changes.
    ///
    /// The callback receives `true` when focus enters the terminal and `false`
    /// when it leaves. Wired from the window to suspend/restore the single-Ctrl
    /// accelerators that collide with readline chords (issue #197).
    pub fn set_on_terminal_focus<F>(&self, callback: F)
    where
        F: Fn(bool) + 'static,
    {
        *self.on_terminal_focus.borrow_mut() = Some(Box::new(callback));
    }

    /// Sets the callback invoked when session recording starts or stops.
    ///
    /// Receives the connection ID and the new recording state; used to
    /// drive the sidebar recording indicator.
    pub fn set_on_recording_changed<F>(&self, callback: F)
    where
        F: Fn(Uuid, bool) + 'static,
    {
        *self.on_recording_changed.borrow_mut() = Some(Box::new(callback));
    }

    /// Sets the callback invoked after the split-color map changes.
    ///
    /// Fired when a session joins or leaves a split, or a split tab closes.
    /// The handler re-syncs the sidebar split-membership marker from
    /// [`Self::split_colors`].
    pub fn set_on_split_colors_changed<F>(&self, callback: F)
    where
        F: Fn() + 'static,
    {
        *self.on_split_colors_changed.borrow_mut() = Some(Box::new(callback));
    }

    /// Fires the split-colors-changed callback, if one is registered.
    ///
    /// Callers must not hold a borrow of `split_session_colors`, `sessions`,
    /// or `session_info` when calling this — the handler re-reads them.
    pub(super) fn notify_split_colors_changed(&self) {
        if let Some(ref callback) = *self.on_split_colors_changed.borrow() {
            callback();
        }
    }

    /// Sets the callback to be invoked for split view cleanup when a page is about to close.
    ///
    /// The callback receives the session ID of the page being closed.
    /// This is used to clear the session from split view panels before the tab is closed.
    ///
    /// # Arguments
    ///
    /// * `callback` - A closure that takes session_id as parameter
    pub fn set_on_split_cleanup<F>(&self, callback: F)
    where
        F: Fn(Uuid) + 'static,
    {
        *self.on_split_cleanup.borrow_mut() = Some(Box::new(callback));
    }

    // === Highlight rules integration ===

    /// Sets the activity coordinator for tab context menu integration.
    ///
    /// Must be called after construction to enable the "Monitor: ..." context menu action.
    pub fn set_activity_coordinator(&self, coordinator: Rc<ActivityCoordinator>) {
        *self.activity_coordinator.borrow_mut() = Some(coordinator);
    }

    /// Sets the monitoring coordinator used by the detach and attach paths.
    ///
    /// Must be called after construction so moving a session between its tab
    /// and a detached window suspends the monitoring bar before the widget move
    /// and resumes it into the new content box afterwards. Without it the
    /// detach paths simply leave monitoring untouched.
    pub fn set_monitoring_coordinator(&self, coordinator: Rc<MonitoringCoordinator>) {
        *self.monitoring.borrow_mut() = Some(coordinator);
    }
}
