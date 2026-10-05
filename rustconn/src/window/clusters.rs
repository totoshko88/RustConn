//! Cluster management methods for the main window
//!
//! This module contains methods for managing connection clusters,
//! including cluster dialogs and related functionality.

use std::rc::Rc;

use adw::prelude::*;
use gtk4::prelude::*;
use uuid::Uuid;

use super::MainWindow;
use crate::alert;
use crate::dialogs::{ClusterDialog, ClusterListDialog, ClusterSummary};
use crate::i18n::{i18n, i18n_f, ni18n_f};
use crate::sidebar::ConnectionSidebar;
use crate::state::SharedAppState;
use crate::terminal::TerminalNotebook;
use crate::window::SharedToastOverlay;
use libadwaita as adw;

/// Type alias for shared terminal notebook
pub type SharedNotebook = Rc<TerminalNotebook>;

/// Type alias for shared sidebar
pub type SharedSidebar = Rc<ConnectionSidebar>;

/// Shows the new cluster dialog with pre-selected connections from sidebar selection
pub fn show_new_cluster_dialog_with_selection(
    window: &gtk4::Window,
    state: SharedAppState,
    notebook: SharedNotebook,
    selected_ids: Vec<Uuid>,
    toast: SharedToastOverlay,
) {
    let dialog = ClusterDialog::new(Some(&window.clone().upcast()));

    // Populate available connections
    if let Ok(state_ref) = state.try_borrow() {
        let connections: Vec<_> = state_ref
            .list_connections()
            .iter()
            .cloned()
            .cloned()
            .collect();
        dialog.set_connections(&connections);
    }

    // Pre-select the connections chosen in sidebar
    dialog.pre_select_connections(&selected_ids);

    let window_clone = window.clone();
    let state_clone = state.clone();
    let notebook_clone = notebook.clone();
    dialog.run(move |result| {
        if let Some(cluster) = result
            && let Ok(mut state_mut) = state_clone.try_borrow_mut()
        {
            match state_mut.create_cluster(cluster) {
                Ok(_) => {
                    toast.show_success(&i18n("Cluster has been saved successfully."));
                }
                Err(e) => {
                    crate::alert::show_error(&window_clone, &i18n("Error Creating Cluster"), &e);
                }
            }
        }
        // Keep notebook reference alive
        let _ = &notebook_clone;
    });
}

/// Shows the clusters manager dialog
pub fn show_clusters_manager(
    window: &gtk4::Window,
    state: SharedAppState,
    notebook: SharedNotebook,
    sidebar: SharedSidebar,
    monitoring: super::types::SharedMonitoring,
) {
    let dialog = ClusterListDialog::new(Some(&window.clone().upcast()));

    // Set up clusters provider for refresh operations
    let state_for_provider = state.clone();
    dialog.set_clusters_provider(move || {
        if let Ok(state_ref) = state_for_provider.try_borrow() {
            // Count the resolved membership, the set a connect opens — a
            // pattern-only cluster used to read "0 connections" here.
            let connections = state_ref.list_connections();
            state_ref
                .get_all_clusters()
                .into_iter()
                .map(|cluster| ClusterSummary {
                    member_count: cluster.resolve_members(connections.iter().copied()).len(),
                    cluster: cluster.clone(),
                })
                .collect()
        } else {
            Vec::new()
        }
    });

    // Wrap dialog in Rc for shared access across callbacks
    let dialog_ref = std::rc::Rc::new(dialog);

    // Set up all dialog callbacks
    setup_cluster_dialog_callbacks(
        &dialog_ref,
        window,
        &state,
        &notebook,
        &sidebar,
        &monitoring,
    );

    // Populate clusters before showing
    dialog_ref.refresh_list();
    dialog_ref.show();
}

/// Sets up callbacks for the cluster list dialog
fn setup_cluster_dialog_callbacks(
    dialog_ref: &std::rc::Rc<ClusterListDialog>,
    window: &gtk4::Window,
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    monitoring: &super::types::SharedMonitoring,
) {
    // Helper to create refresh callback
    let create_refresh_callback = |dialog_ref: std::rc::Rc<ClusterListDialog>| {
        move || {
            dialog_ref.refresh_list();
        }
    };

    // Connect callback
    let state_clone = state.clone();
    let notebook_clone = notebook.clone();
    let window_clone = window.clone();
    let sidebar_clone = sidebar.clone();
    let monitoring_clone = monitoring.clone();
    dialog_ref.set_on_connect(move |cluster_id| {
        connect_cluster(
            &state_clone,
            &notebook_clone,
            &window_clone,
            &sidebar_clone,
            &monitoring_clone,
            cluster_id,
        );
    });

    // Disconnect callback
    let notebook_clone = notebook.clone();
    dialog_ref.set_on_disconnect(move |cluster_id| {
        disconnect_cluster(&notebook_clone, cluster_id);
    });

    // Edit callback
    let state_clone = state.clone();
    let notebook_clone = notebook.clone();
    let window_clone = window.clone();
    let dialog_ref_edit = dialog_ref.clone();
    let refresh_after_edit = create_refresh_callback(dialog_ref_edit.clone());
    dialog_ref.set_on_edit(move |cluster_id| {
        edit_cluster(
            &window_clone,
            &state_clone,
            &notebook_clone,
            cluster_id,
            Box::new(refresh_after_edit.clone()),
        );
    });

    // Delete callback
    let state_clone = state.clone();
    let window_clone = window.clone();
    let dialog_ref_delete = dialog_ref.clone();
    let refresh_after_delete = create_refresh_callback(dialog_ref_delete.clone());
    dialog_ref.set_on_delete(move |cluster_id| {
        delete_cluster(
            &window_clone,
            &state_clone,
            cluster_id,
            Box::new(refresh_after_delete.clone()),
        );
    });

    // New cluster callback
    let state_clone = state.clone();
    let notebook_clone = notebook.clone();
    let window_clone = window.clone();
    let dialog_ref_new = dialog_ref.clone();
    let refresh_after_new = create_refresh_callback(dialog_ref_new.clone());
    dialog_ref.set_on_new(move || {
        show_new_cluster_dialog_from_manager(
            &window_clone,
            state_clone.clone(),
            notebook_clone.clone(),
            Box::new(refresh_after_new.clone()),
        );
    });
}

/// Shows new cluster dialog from the manager
fn show_new_cluster_dialog_from_manager(
    parent: &gtk4::Window,
    state: SharedAppState,
    _notebook: SharedNotebook,
    on_created: Box<dyn Fn() + 'static>,
) {
    let dialog = ClusterDialog::new(Some(parent));

    // Populate available connections
    if let Ok(state_ref) = state.try_borrow() {
        let connections: Vec<_> = state_ref
            .list_connections()
            .iter()
            .cloned()
            .cloned()
            .collect();
        dialog.set_connections(&connections);
    }

    let state_clone = state.clone();
    let parent_clone = parent.clone();
    dialog.run(move |result| {
        if let Some(cluster) = result {
            let create_result = if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                state_mut.create_cluster(cluster)
            } else {
                Err("Could not access application state".to_string())
            };

            match create_result {
                Ok(_) => {
                    on_created();
                }
                Err(e) => {
                    alert::show_error(
                        &parent_clone,
                        &i18n("Error Creating Cluster"),
                        &i18n_f("Failed to save cluster: {}", &[&e]),
                    );
                }
            }
        }
    });
}

/// Connects to all connections in a cluster.
///
/// Cluster simply means "open these connections together". Broadcast
/// is now an independent split-view feature (see toggle-broadcast action),
/// not a cluster property — tabs do not need to belong to a cluster to use it.
///
/// Sessions are registered into the cluster lazily, when each tab appears, so
/// this works for both synchronously created tabs (Telnet, Serial) and
/// asynchronously created ones (SSH after a TCP port check).
///
/// Each member tab is also put into a tab group named after the cluster, which
/// is what makes an open cluster visible and operable. Before this, a cluster
/// dissolved into anonymous tabs the moment it opened: nothing on screen said
/// which tabs belonged together, and the only way to act on the set as a whole
/// was the Disconnect button in this dialog. With the group in place the tab
/// reads `[cluster] host` and the existing group operations — Close All in
/// Group, Close All Ungrouped — apply to the cluster for free.
///
/// The group name is the cluster's name verbatim, so renaming a cluster changes
/// the label of the next opening, not of tabs already open. A cluster sharing a
/// name with a hand-made group merges into it, which is the same rule two tabs
/// with the same typed name already follow.
fn connect_cluster(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    window: &gtk4::Window,
    sidebar: &SharedSidebar,
    monitoring: &super::types::SharedMonitoring,
    cluster_id: Uuid,
) {
    // Get cluster info. Resolve the EFFECTIVE membership (explicit members plus
    // any regex auto-membership matches) rather than the raw connection_ids, so
    // an auto-membership rule like `^prod-web\d+` actually pulls matching hosts
    // into the mass-connect. resolve_members is read-widening and de-duplicated,
    // and reads the connections in place — no clone of the whole list.
    let (connection_ids, cluster_name, has_pattern) = if let Ok(state_ref) = state.try_borrow() {
        if let Some(cluster) = state_ref.get_cluster(cluster_id) {
            (
                cluster.resolve_members(state_ref.list_connections()),
                cluster.name.clone(),
                cluster.auto_membership.is_some(),
            )
        } else {
            return;
        }
    } else {
        return;
    };

    if connection_ids.is_empty() {
        crate::toast::show_error_toast_on_active_window(&i18n("Cluster has no connections"));
        return;
    }

    // Two questions may stand between the click and the connections, asked in
    // this order and each at most once for the whole cluster: a size check (a
    // broad pattern such as `.` resolves to every connection), then one
    // aggregated jump-host check for every member (issue #345).
    let state = state.clone();
    let notebook = notebook.clone();
    let sidebar = sidebar.clone();
    let monitoring = monitoring.clone();
    let window_for_bastions = window.clone();
    let name_for_question = cluster_name.clone();
    confirm_large_cluster(
        window,
        &name_for_question,
        connection_ids.len(),
        has_pattern,
        move || {
            let state_for_check = state.clone();
            confirm_cluster_bastions(
                &window_for_bastions,
                &state_for_check,
                connection_ids,
                move |connection_ids| {
                    dispatch_cluster_connect(
                        &state,
                        &notebook,
                        &sidebar,
                        &monitoring,
                        cluster_id,
                        &cluster_name,
                        &connection_ids,
                    );
                },
            );
        },
    );
}

/// Clusters resolving to more members than this ask before connecting.
///
/// Ten is a rack or a load-balancer pool — the size clusters are made for. Past
/// that, opening every member at once is more likely an over-broad
/// auto-membership pattern (`.` matches every connection) than intent, and the
/// cost of being wrong is dozens of sessions dialling, prompting for passwords
/// and, for SSH, offering credentials to hosts nobody meant to touch.
const LARGE_CLUSTER_CONFIRM_THRESHOLD: usize = 10;

/// Presents `dialog` and runs `on_accept` once, if the `accept` response is
/// chosen. Any other response — including Escape — does nothing.
fn present_confirmation(
    window: &gtk4::Window,
    dialog: &adw::AlertDialog,
    accept: &'static str,
    on_accept: impl FnOnce() + 'static,
) {
    let pending = std::cell::Cell::new(Some(on_accept));
    dialog.connect_response(None, move |_, response| {
        if response == accept
            && let Some(on_accept) = pending.take()
        {
            on_accept();
        }
    });
    dialog.present(Some(window));
}

/// Asks before opening a cluster of more than
/// [`LARGE_CLUSTER_CONFIRM_THRESHOLD`] members; runs `proceed` directly for a
/// smaller one.
fn confirm_large_cluster(
    window: &gtk4::Window,
    cluster_name: &str,
    member_count: usize,
    has_pattern: bool,
    proceed: impl FnOnce() + 'static,
) {
    if member_count <= LARGE_CLUSTER_CONFIRM_THRESHOLD {
        proceed();
        return;
    }

    let count = member_count.to_string();
    let mut body = ni18n_f(
        "“{}” opens {} connection at once.",
        "“{}” opens {} connections at once.",
        u32::try_from(member_count).unwrap_or(u32::MAX),
        &[cluster_name, &count],
    );
    if has_pattern {
        body.push_str("\n\n");
        body.push_str(&i18n(
            "Most of them may come from its auto-membership pattern. If that is more than you expected, cancel and narrow the pattern.",
        ));
    }

    let dialog = adw::AlertDialog::new(Some(&i18n("Open All Connections?")), Some(&body));
    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.add_response("connect", &i18n("Connect All"));
    dialog.set_response_appearance("connect", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    present_confirmation(window, &dialog, "connect", proceed);
}

/// Checks every member's jump host in one pass and, when any would be skipped,
/// asks once for the whole cluster (issue #345); runs `proceed` with the member
/// list when nothing needs asking or the user confirms.
///
/// One question instead of one toast per member, and one check over borrowed
/// state instead of a full clone of every connection and group per member.
fn confirm_cluster_bastions(
    window: &gtk4::Window,
    state: &SharedAppState,
    connection_ids: Vec<Uuid>,
    proceed: impl FnOnce(Vec<Uuid>) + 'static,
) {
    let affected: Vec<(Uuid, String)> = {
        let Ok(state_ref) = state.try_borrow() else {
            return;
        };
        state_ref
            .skipped_bastions(&connection_ids)
            .into_iter()
            .filter_map(|(id, _)| state_ref.get_connection(id).map(|c| (id, c.name.clone())))
            .collect()
    };
    if affected.is_empty() {
        proceed(connection_ids);
        return;
    }

    let names = affected
        .iter()
        .map(|(_, name)| format!("“{name}”"))
        .collect::<Vec<_>>()
        .join(", ");
    let count = affected.len().to_string();
    let body = format!(
        "{}\n\n{}",
        ni18n_f(
            "{} cluster member has a jump host that no longer exists or points to itself: {}.",
            "{} cluster members have a jump host that no longer exists or points to itself: {}.",
            u32::try_from(affected.len()).unwrap_or(u32::MAX),
            &[&count, &names],
        ),
        i18n(
            "Connecting now skips those jump hosts, so these members may reach their servers directly. Open a member on its own to see the details.",
        ),
    );

    let dialog = adw::AlertDialog::new(Some(&i18n("Jump Host Unavailable")), Some(&body));
    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.add_response("connect", &i18n("Connect Anyway"));
    dialog.set_response_appearance("connect", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    let state_cb = state.clone();
    present_confirmation(window, &dialog, "connect", move || {
        // Same session-scoped acceptance a single connect records, so opening
        // one of these members again does not ask a second time.
        if let Ok(state_ref) = state_cb.try_borrow() {
            for (id, _) in &affected {
                state_ref.confirm_bastion_skip(*id);
            }
        }
        proceed(connection_ids);
    });
}

/// Starts the cluster session and every member connection.
fn dispatch_cluster_connect(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    monitoring: &super::types::SharedMonitoring,
    cluster_id: Uuid,
    cluster_name: &str,
    connection_ids: &[Uuid],
) {
    tracing::info!(
        cluster = %cluster_name,
        cluster_id = %cluster_id,
        connections = connection_ids.len(),
        "Connecting cluster"
    );

    // Start cluster session in state
    if let Ok(mut state_mut) = state.try_borrow_mut()
        && let Err(e) = state_mut.start_cluster_session(cluster_id)
    {
        tracing::error!(?e, cluster = %cluster_name, "Failed to start cluster session");
    }

    // Mark every connection as pending; the central hook in
    // `TerminalNotebook::notify_tab_added` resolves them when each tab actually
    // appears, registering the new session in the cluster's session list and
    // labelling its tab with a tab group named after the cluster.
    for conn_id in connection_ids {
        notebook.mark_cluster_pending(cluster_id, cluster_name, *conn_id);
    }

    // Kick off each connection. We don't care whether `start_connection`
    // returns Started, Pending or Failed — registration is driven by the
    // callback in `create_terminal_tab_with_settings`. The jump hosts were
    // checked for the whole cluster in `confirm_cluster_bastions`, so the
    // members skip the per-connection check (and its dialog).
    let mut sync_started = 0usize;
    for conn_id in connection_ids {
        match MainWindow::start_connection_bastion_prechecked(
            state, notebook, sidebar, monitoring, *conn_id,
        ) {
            super::types::ConnectionStartResult::Started(_) => sync_started += 1,
            super::types::ConnectionStartResult::Pending
            | super::types::ConnectionStartResult::Failed => {}
        }
    }

    tracing::info!(
        cluster = %cluster_name,
        connections = connection_ids.len(),
        sync_started,
        "Cluster connection requests dispatched"
    );
}

/// Disconnects all connections in a cluster
fn disconnect_cluster(notebook: &SharedNotebook, cluster_id: Uuid) {
    let session_ids = notebook.get_cluster_sessions(cluster_id);

    if session_ids.is_empty() {
        return;
    }

    tracing::info!(
        cluster_id = %cluster_id,
        sessions = session_ids.len(),
        "Disconnecting cluster"
    );

    for session_id in &session_ids {
        notebook.close_tab(*session_id);
    }

    notebook.unregister_cluster(cluster_id);
}

/// Edits a cluster
fn edit_cluster(
    parent: &gtk4::Window,
    state: &SharedAppState,
    _notebook: &SharedNotebook,
    cluster_id: Uuid,
    on_updated: Box<dyn Fn() + 'static>,
) {
    let (cluster, connections) = if let Ok(state_ref) = state.try_borrow() {
        let Some(cluster) = state_ref.get_cluster(cluster_id).cloned() else {
            return;
        };
        let connections: Vec<_> = state_ref
            .list_connections()
            .iter()
            .cloned()
            .cloned()
            .collect();
        (cluster, connections)
    } else {
        return;
    };

    let dialog = ClusterDialog::new(Some(parent));
    dialog.set_connections(&connections);
    dialog.set_cluster(&cluster);

    let state_clone = state.clone();
    let parent_clone = parent.clone();
    dialog.run(move |result| {
        if let Some(updated) = result {
            if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                match state_mut.update_cluster(updated) {
                    Ok(()) => {
                        on_updated();
                    }
                    Err(e) => {
                        alert::show_error(
                            &parent_clone,
                            &i18n("Error Updating Cluster"),
                            &i18n_f("Failed to save cluster: {}", &[&e]),
                        );
                    }
                }
            } else {
                alert::show_error(
                    &parent_clone,
                    &i18n("Error"),
                    &i18n("Could not access application state"),
                );
            }
        }
    });
}

/// Deletes a cluster
fn delete_cluster(
    parent: &gtk4::Window,
    state: &SharedAppState,
    cluster_id: Uuid,
    on_deleted: Box<dyn Fn() + 'static>,
) {
    let cluster_name = if let Ok(state_ref) = state.try_borrow() {
        if let Some(cluster) = state_ref.get_cluster(cluster_id) {
            cluster.name.clone()
        } else {
            return;
        }
    } else {
        return;
    };

    let state_clone = state.clone();
    let parent_clone = parent.clone();
    alert::show_confirm(
        parent,
        &i18n("Delete Cluster?"),
        &i18n_f(
            "Are you sure you want to delete the cluster '{}'?\nThis will not delete the connections in the cluster.",
            &[&cluster_name],
        ),
        &i18n("Delete"),
        true,
        move |confirmed| {
            if confirmed {
                let delete_result = if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                    let res = state_mut.delete_cluster(cluster_id);
                    drop(state_mut); // Explicitly drop before calling on_deleted
                    res
                } else {
                    Err("Could not access application state".to_string())
                };

                match delete_result {
                    Ok(()) => {
                        // Refresh the list after successful deletion
                        on_deleted();
                    }
                    Err(e) => {
                        alert::show_error(
                            &parent_clone,
                            &i18n("Error Deleting Cluster"),
                            &i18n_f("Failed to delete cluster: {}", &[&e]),
                        );
                    }
                }
            }
        },
    );
}
