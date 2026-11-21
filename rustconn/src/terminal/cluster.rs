//! Cluster registration and pending-cluster bookkeeping.
//!
//! Moved out of `terminal/mod.rs`; behaviour is unchanged.

use super::*;

impl TerminalNotebook {
    /// Registers a session as part of a cluster, under the cluster's group name.
    pub fn register_cluster_terminal(&self, cluster_id: Uuid, group: &str, session_id: Uuid) {
        self.cluster_sessions
            .borrow_mut()
            .entry(cluster_id)
            .or_insert_with(|| ClusterTabs {
                group: group.to_owned(),
                sessions: Vec::new(),
            })
            .sessions
            .push(session_id);
        self.session_to_cluster
            .borrow_mut()
            .insert(session_id, cluster_id);
    }

    /// Unregisters all tabs of a cluster, retiring its tab group if it is unused.
    ///
    /// Called after the member tabs have been closed, so by now their sessions
    /// are gone from `session_info` and the only sessions that could still be
    /// wearing the name are unrelated ones — a tab a user labelled by hand, or a
    /// second cluster with the same name. Without this, every cluster ever opened
    /// would leave its name in the "Set Group…" chooser for the lifetime of the
    /// window, which is a new problem created by naming groups automatically.
    pub fn unregister_cluster(&self, cluster_id: Uuid) {
        if let Some(tabs) = self.cluster_sessions.borrow_mut().remove(&cluster_id) {
            let mut reverse = self.session_to_cluster.borrow_mut();
            for sid in &tabs.sessions {
                reverse.remove(sid);
            }
            drop(reverse);

            // The cluster's own sessions are excluded rather than assumed gone:
            // `close_page` teardown is not guaranteed to have removed them from
            // `session_info` by the time the caller unregisters, and the question
            // being asked is whether anything *else* wears the name.
            //
            // ponytail: O(members × open tabs); a cluster is a handful of hosts,
            // so a set would cost more than it saves.
            let still_used = {
                let info = self.session_info.borrow();
                group_still_in_use(
                    &tabs.group,
                    info.iter()
                        .filter(|(session_id, _)| !tabs.sessions.contains(session_id))
                        .map(|(_, info)| info.tab_group.as_deref()),
                )
            };
            if still_used {
                tracing::debug!(
                    group = tabs.group,
                    "cluster closed; keeping its group name, other tabs still carry it"
                );
            } else {
                self.tab_group_manager
                    .borrow_mut()
                    .remove_group(&tabs.group);
            }
        }
        // Clear any pending registrations for this cluster
        self.cluster_pending
            .borrow_mut()
            .retain(|_, pending| pending.cluster_id != cluster_id);
    }

    /// Marks a connection as pending cluster registration.
    ///
    /// When the tab for `connection_id` is eventually created — synchronously,
    /// or after an async port check — it is registered as part of `cluster_id`
    /// and labelled with the cluster's `group` name. With `in_window` the tab is
    /// then handed to the callback set by [`Self::set_on_cluster_window_tab`].
    pub fn mark_cluster_pending(
        &self,
        cluster_id: Uuid,
        group: &str,
        connection_id: Uuid,
        in_window: bool,
    ) {
        self.cluster_pending.borrow_mut().insert(
            connection_id,
            PendingCluster {
                cluster_id,
                group: group.to_owned(),
                in_window,
            },
        );
    }

    /// Drops the pending cluster registration of a connection that failed to
    /// start, so its next, unrelated connect does not join the cluster — or,
    /// for a cluster opened in its own window, move into that window.
    pub fn forget_cluster_pending(&self, connection_id: Uuid) {
        let forgotten = self.cluster_pending.borrow_mut().remove(&connection_id);
        if let Some(pending) = forgotten {
            tracing::debug!(
                connection = %connection_id,
                cluster = %pending.cluster_id,
                "cluster member failed to start; pending registration dropped"
            );
        }
    }

    /// Sets the callback that receives the tabs of a cluster opened in its own
    /// window, as `(cluster_id, cluster_name, session_id)`.
    pub fn set_on_cluster_window_tab<F>(&self, callback: F)
    where
        F: Fn(Uuid, &str, Uuid) + 'static,
    {
        *self.on_cluster_window_tab.borrow_mut() = Some(Box::new(callback));
    }

    /// Resolves a pending cluster registration for a freshly created tab.
    ///
    /// Called from [`Self::notify_tab_added`], which every tab-creation path goes
    /// through — terminal, VNC, embedded RDP, embedded Web and external process
    /// alike. It used to be called from the terminal path only, so an RDP or VNC
    /// member of a cluster opened a tab that was never registered: it stayed in
    /// `cluster_pending` forever and "Disconnect all cluster sessions" could not
    /// see it.
    ///
    /// Labelling the tab with the cluster's name is what makes a cluster visible
    /// after it is open: the tab reads `[cluster] host`, and every tab-group
    /// operation — Close All in Group above all — then applies to the cluster
    /// without needing a cluster-specific command.
    pub(super) fn resolve_cluster_pending(&self, connection_id: Uuid, session_id: Uuid) {
        let Some(pending) = self.cluster_pending.borrow_mut().remove(&connection_id) else {
            return;
        };
        self.register_cluster_terminal(pending.cluster_id, &pending.group, session_id);
        if pending.in_window {
            // No tab group: the cluster's own window is the grouping, and a
            // `[cluster]` prefix on every tab of it would only repeat its title.
            if let Some(ref callback) = *self.on_cluster_window_tab.borrow() {
                callback(pending.cluster_id, &pending.group, session_id);
            }
        } else {
            self.set_tab_group(session_id, &pending.group);
        }
    }

    /// Gets all session IDs for a cluster
    pub fn get_cluster_sessions(&self, cluster_id: Uuid) -> Vec<Uuid> {
        self.cluster_sessions
            .borrow()
            .get(&cluster_id)
            .map(|tabs| tabs.sessions.clone())
            .unwrap_or_default()
    }

    // ── Ad-hoc Broadcast ──────────────────────────────────────────────
    // (removed: superseded by the split-view broadcast toggle in the header bar)
}
