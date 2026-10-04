//! Cluster management methods for the main window
//!
//! This module contains methods for managing connection clusters,
//! including cluster dialogs and related functionality.

use std::rc::Rc;

use gtk4::prelude::*;
use uuid::Uuid;

use super::MainWindow;
use crate::alert;
use crate::dialogs::{ClusterDialog, ClusterListDialog};
use crate::i18n::{i18n, i18n_f};
use crate::sidebar::ConnectionSidebar;
use crate::state::SharedAppState;
use crate::terminal::TerminalNotebook;
use crate::window::SharedToastOverlay;

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
            state_ref
                .get_all_clusters()
                .iter()
                .cloned()
                .cloned()
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
    _window: &gtk4::Window,
    sidebar: &SharedSidebar,
    monitoring: &super::types::SharedMonitoring,
    cluster_id: Uuid,
) {
    // Get cluster info. Resolve the EFFECTIVE membership (explicit members plus
    // any regex auto-membership matches) rather than the raw connection_ids, so
    // an auto-membership rule like `^prod-web\d+` actually pulls matching hosts
    // into the mass-connect. resolve_members is read-widening and de-duplicated.
    let (connection_ids, cluster_name) = if let Ok(state_ref) = state.try_borrow() {
        if let Some(cluster) = state_ref.get_cluster(cluster_id) {
            let connections = state_ref.list_connections_owned();
            (cluster.resolve_members(&connections), cluster.name.clone())
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
    for conn_id in &connection_ids {
        notebook.mark_cluster_pending(cluster_id, &cluster_name, *conn_id);
    }

    // Kick off each connection. We don't care whether `start_connection`
    // returns Started, Pending or Failed — registration is driven by the
    // callback in `create_terminal_tab_with_settings`.
    let mut sync_started = 0usize;
    for conn_id in &connection_ids {
        match MainWindow::start_connection(state, notebook, sidebar, monitoring, *conn_id) {
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
