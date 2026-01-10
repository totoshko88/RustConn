//! Cluster management methods for the main window
//!
//! This module contains methods for managing connection clusters,
//! including cluster dialogs and related functionality.

use crate::alert;
use gtk4::prelude::*;
use std::rc::Rc;
use uuid::Uuid;

use crate::dialogs::{ClusterDialog, ClusterListDialog};
use crate::sidebar::ConnectionSidebar;
use crate::state::SharedAppState;
use crate::terminal::TerminalNotebook;
use crate::window::MainWindow;

/// Type alias for shared terminal notebook
pub type SharedNotebook = Rc<TerminalNotebook>;

/// Type alias for shared sidebar
pub type SharedSidebar = Rc<ConnectionSidebar>;

/// Shows the new cluster dialog
pub fn show_new_cluster_dialog(
    window: &gtk4::Window,
    state: SharedAppState,
    notebook: SharedNotebook,
) {
    let dialog = ClusterDialog::new(Some(&window.clone().upcast()));

    // Populate available connections
    {
        let state_ref = state.borrow();
        let connections: Vec<_> = state_ref
            .list_connections()
            .iter()
            .cloned()
            .cloned()
            .collect();
        dialog.set_connections(&connections);
    }

    let window_clone = window.clone();
    let state_clone = state.clone();
    let notebook_clone = notebook.clone();
    dialog.run(move |result| {
        if let Some(cluster) = result {
            if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                match state_mut.create_cluster(cluster) {
                    Ok(_) => {
                        alert::show_success(
                            &window_clone,
                            "Cluster Created",
                            "Cluster has been saved successfully.",
                        );
                    }
                    Err(e) => {
                        alert::show_error(&window_clone, "Error Creating Cluster", &e);
                    }
                }
            }
        }
        // Keep notebook reference alive
        let _ = &notebook_clone;
    });
}

/// Shows the clusters manager dialog
#[allow(clippy::too_many_lines)]
pub fn show_clusters_manager(
    window: &gtk4::Window,
    state: SharedAppState,
    notebook: SharedNotebook,
    sidebar: SharedSidebar,
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

    // Initial population of clusters
    let dialog_for_refresh = dialog_ref.clone();
    dialog_ref.window().connect_show(move |_| {
        dialog_for_refresh.refresh_list();
    });

    // Helper to refresh the cluster list
    let create_refresh_callback = |dialog_ref: std::rc::Rc<ClusterListDialog>| {
        move || {
            dialog_ref.refresh_list();
        }
    };

    // Set up callbacks
    let state_clone = state.clone();
    let notebook_clone = notebook.clone();
    let window_clone = window.clone();
    let sidebar_clone = sidebar.clone();
    dialog_ref.set_on_connect(move |cluster_id| {
        connect_cluster(
            &state_clone,
            &notebook_clone,
            &window_clone,
            &sidebar_clone,
            cluster_id,
        );
    });

    let state_clone = state.clone();
    let notebook_clone = notebook.clone();
    let dialog_window = dialog_ref.window().clone();
    let dialog_ref_edit = dialog_ref.clone();
    let refresh_after_edit = create_refresh_callback(dialog_ref_edit.clone());
    dialog_ref.set_on_edit(move |cluster_id| {
        edit_cluster(
            dialog_window.upcast_ref(),
            &state_clone,
            &notebook_clone,
            cluster_id,
            Box::new(refresh_after_edit.clone()),
        );
    });

    let state_clone = state.clone();
    let dialog_window = dialog_ref.window().clone();
    let dialog_ref_delete = dialog_ref.clone();
    let refresh_after_delete = create_refresh_callback(dialog_ref_delete.clone());
    dialog_ref.set_on_delete(move |cluster_id| {
        delete_cluster(
            dialog_window.upcast_ref(),
            &state_clone,
            cluster_id,
            Box::new(refresh_after_delete.clone()),
        );
    });

    let state_clone = state.clone();
    let notebook_clone = notebook.clone();
    let dialog_window = dialog_ref.window().clone();
    let dialog_ref_new = dialog_ref.clone();
    let refresh_after_new = create_refresh_callback(dialog_ref_new.clone());
    dialog_ref.set_on_new(move || {
        show_new_cluster_dialog_from_manager(
            dialog_window.upcast_ref(),
            state_clone.clone(),
            notebook_clone.clone(),
            Box::new(refresh_after_new.clone()),
        );
    });

    dialog_ref.show();
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
    {
        let state_ref = state.borrow();
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
                        "Error Creating Cluster",
                        &format!("Failed to save cluster: {e}"),
                    );
                }
            }
        }
    });
}

/// Connects to all connections in a cluster
fn connect_cluster(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    _window: &gtk4::Window,
    sidebar: &SharedSidebar,
    cluster_id: Uuid,
) {
    let state_ref = state.borrow();
    let Some(cluster) = state_ref.get_cluster(cluster_id) else {
        return;
    };

    let connection_ids: Vec<Uuid> = cluster.connection_ids.clone();
    drop(state_ref);

    // Connect to each connection in the cluster
    for conn_id in connection_ids {
        MainWindow::start_connection(state, notebook, sidebar, conn_id);
    }
}

/// Edits a cluster
fn edit_cluster(
    parent: &gtk4::Window,
    state: &SharedAppState,
    _notebook: &SharedNotebook,
    cluster_id: Uuid,
    on_updated: Box<dyn Fn() + 'static>,
) {
    let state_ref = state.borrow();
    let Some(cluster) = state_ref.get_cluster(cluster_id).cloned() else {
        return;
    };
    let connections: Vec<_> = state_ref
        .list_connections()
        .iter()
        .cloned()
        .cloned()
        .collect();
    drop(state_ref);

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
                            "Error Updating Cluster",
                            &format!("Failed to save cluster: {e}"),
                        );
                    }
                }
            } else {
                alert::show_error(
                    &parent_clone,
                    "Error",
                    "Could not access application state",
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
    let state_ref = state.borrow();
    let Some(cluster) = state_ref.get_cluster(cluster_id) else {
        return;
    };
    let cluster_name = cluster.name.clone();
    drop(state_ref);

    let state_clone = state.clone();
    let parent_clone = parent.clone();
    alert::show_confirm(
        parent,
        "Delete Cluster?",
        &format!(
            "Are you sure you want to delete the cluster '{cluster_name}'?\n\
            This will not delete the connections in the cluster."
        ),
        "Delete",
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
                            "Error Deleting Cluster",
                            &format!("Failed to delete cluster: {e}"),
                        );
                    }
                }
            }
        },
    );
}
