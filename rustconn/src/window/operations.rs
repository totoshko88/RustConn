//! Connection operations for main window
//!
//! This module contains functions for connection operations like delete,
//! duplicate, copy, paste, and reload sidebar.

use std::rc::Rc;

use adw::prelude::*;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use rustconn_core::connection::host_range::{self, HostRangeError};
use uuid::Uuid;

use super::MainWindow;
use crate::alert;
use crate::i18n::{i18n, i18n_f};
use crate::sidebar::ConnectionSidebar;
use crate::state::SharedAppState;

/// Type alias for shared sidebar reference
pub type SharedSidebar = Rc<ConnectionSidebar>;

/// Deletes the selected connection or group
pub fn delete_selected_connection(
    window: &gtk4::Window,
    state: &SharedAppState,
    sidebar: &SharedSidebar,
) {
    // Get selected item using sidebar's method (works in both single and multi-selection modes)
    let Some(conn_item) = sidebar.get_selected_item() else {
        return;
    };

    let id_str = conn_item.id();
    let Ok(id) = Uuid::parse_str(&id_str) else {
        return;
    };
    confirm_and_delete(
        window,
        state,
        sidebar,
        id,
        conn_item.name(),
        conn_item.is_group(),
    );
}

/// Deletes the connection with `id` after confirmation, whatever is selected
/// in the sidebar. Does nothing when no connection has that id.
pub fn delete_connection_by_id(
    window: &gtk4::Window,
    state: &SharedAppState,
    sidebar: &SharedSidebar,
    id: Uuid,
) {
    let Some(name) = state
        .try_borrow()
        .ok()
        .and_then(|s| s.get_connection(id).map(|c| c.name.clone()))
    else {
        return;
    };
    confirm_and_delete(window, state, sidebar, id, name, false);
}

/// Asks for confirmation, then deletes the connection or group `id`.
fn confirm_and_delete(
    window: &gtk4::Window,
    state: &SharedAppState,
    sidebar: &SharedSidebar,
    id: Uuid,
    name: String,
    is_group: bool,
) {
    // Show confirmation dialog with connection count for groups
    let item_type = if is_group {
        i18n("group")
    } else {
        i18n("connection")
    };
    let connection_count = if is_group {
        state
            .try_borrow()
            .map(|s| s.count_connections_in_group(id))
            .unwrap_or(0)
    } else {
        0
    };
    let detail = if is_group {
        if connection_count > 0 {
            i18n_f(
                "This group contains {} connection(s).",
                &[&connection_count.to_string()],
            )
        } else {
            i18n_f(
                "Are you sure you want to delete the empty group '{}'?",
                &[&name],
            )
        }
    } else {
        i18n_f(
            "Are you sure you want to delete the connection '{}'?",
            &[&name],
        )
    };

    let state_clone = state.clone();
    let sidebar_clone = sidebar.clone();
    let window_clone = window.clone();

    if is_group && connection_count > 0 {
        // Group with connections: offer choice between move-to-root and cascade delete
        let dialog =
            adw::AlertDialog::new(Some(&i18n_f("Delete group '{}'?", &[&name])), Some(&detail));
        dialog.add_response("cancel", &i18n("Cancel"));
        dialog.add_response("move", &i18n("Keep Connections"));
        dialog.add_response("delete", &i18n("Delete All"));
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_response_appearance("move", adw::ResponseAppearance::Suggested);

        let name_clone = name.clone();
        dialog.connect_response(None, move |_, response| {
            let cascade = match response {
                "delete" => true,
                "move" => false,
                _ => return,
            };

            if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                let delete_result = if cascade {
                    state_mut.delete_group_cascade(id)
                } else {
                    state_mut.delete_group(id)
                };

                match delete_result {
                    Ok(()) => {
                        drop(state_mut);
                        let state = state_clone.clone();
                        let sidebar = sidebar_clone.clone();
                        let window = window_clone.clone();
                        let name = name_clone.clone();

                        glib::idle_add_local_once(move || {
                            MainWindow::reload_sidebar_preserving_state(&state, &sidebar);
                            let action_target = format!("group:{id}");
                            crate::toast::show_undo_toast_on_window(
                                &window,
                                &i18n_f("Deleted '{}'", &[&name]),
                                &action_target,
                            );
                        });
                    }
                    Err(e) => {
                        alert::show_error(&window_clone, &i18n("Error Deleting"), &e);
                    }
                }
            }
        });

        dialog.present(Some(window));
    } else {
        // Simple confirmation for connections or empty groups
        alert::show_confirm(
            window,
            &i18n_f("Delete {}?", &[&item_type]),
            &detail,
            &i18n("Delete"),
            true,
            move |confirmed| {
                if confirmed && let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                    let delete_result = if is_group {
                        state_mut.delete_group_cascade(id)
                    } else {
                        state_mut.delete_connection(id)
                    };

                    match delete_result {
                        Ok(()) => {
                            drop(state_mut);
                            let state = state_clone.clone();
                            let sidebar = sidebar_clone.clone();
                            let window = window_clone.clone();
                            let name = name.clone();

                            glib::idle_add_local_once(move || {
                                MainWindow::reload_sidebar_preserving_state(&state, &sidebar);
                                let action_target = format!(
                                    "{}:{}",
                                    if is_group { "group" } else { "connection" },
                                    id
                                );
                                crate::toast::show_undo_toast_on_window(
                                    &window,
                                    &i18n_f("Deleted '{}'", &[&name]),
                                    &action_target,
                                );
                            });
                        }
                        Err(e) => {
                            alert::show_error(&window_clone, &i18n("Error Deleting"), &e);
                        }
                    }
                }
            },
        );
    }
}

/// Duplicates the selected connection
pub fn duplicate_selected_connection(
    window: &gtk4::Window,
    state: &SharedAppState,
    sidebar: &SharedSidebar,
) {
    // Get selected item using sidebar's method (works in both single and multi-selection modes)
    let Some(conn_item) = sidebar.get_selected_item() else {
        return;
    };

    // Can only duplicate connections, not groups
    if conn_item.is_group() {
        return;
    }

    let id_str = conn_item.id();
    let Ok(id) = Uuid::parse_str(&id_str) else {
        return;
    };

    let (conn, new_name, settings, groups) = if let Ok(state_ref) = state.try_borrow() {
        let Some(conn) = state_ref.get_connection(id).cloned() else {
            return;
        };
        // Generate unique name for duplicate
        let new_name = state_ref
            .generate_unique_connection_name(&format!("{} (copy)", conn.name), conn.protocol);
        // Needed to copy the vault credential once the duplicate exists; read
        // here so the borrow is released before `create_connection_from`.
        (
            conn,
            new_name,
            state_ref.settings().clone(),
            state_ref.list_groups_owned(),
        )
    } else {
        return;
    };

    // The source connection, kept whole: the vault entry is keyed by name and
    // UUID, so copying the credential needs both sides of the pair.
    let original = conn.clone();

    // New id and name; a duplicate of a favorite is not a favorite itself
    // (see `Connection::duplicate_as`).
    let duplicate = conn.duplicate_as(new_name);

    let duplicate_for_vault = duplicate.clone();

    if let Ok(mut state_mut) = state.try_borrow_mut() {
        match state_mut
            .connection_manager()
            .create_connection_from(duplicate)
        {
            Ok(_) => {
                drop(state_mut);
                // Defer sidebar reload to prevent UI freeze
                let state = state.clone();
                let sidebar = sidebar.clone();
                let window = window.clone();
                glib::idle_add_local_once(move || {
                    MainWindow::reload_sidebar_preserving_state(&state, &sidebar);
                    crate::toast::show_toast_on_window(
                        &window,
                        &i18n("Connection duplicated"),
                        crate::toast::ToastType::Success,
                    );
                });

                // Carry the secret over. A duplicate that keeps
                // `password_source = Vault` without an entry of its own looks
                // configured but has none: connecting prompts for a password or
                // fails outright. Paste, rename and move-to-group all migrate
                // the credential — duplicate was the only name-changing path
                // that did not.
                if duplicate_for_vault.password_source
                    == rustconn_core::models::PasswordSource::Vault
                {
                    crate::utils::spawn_blocking_with_callback(
                        move || {
                            crate::vault_ops::copy_vault_credential(
                                &settings,
                                &groups,
                                &original,
                                &duplicate_for_vault,
                            )
                        },
                        |result| {
                            if let Err(e) = result {
                                tracing::warn!(
                                    error = %e,
                                    "Failed to copy vault credential on duplicate"
                                );
                            }
                        },
                    );
                }
            }
            Err(e) => {
                tracing::error!("Failed to duplicate connection: {e}");
                crate::toast::show_toast_on_window(
                    window,
                    &i18n("Failed to duplicate connection"),
                    crate::toast::ToastType::Error,
                );
            }
        }
    }
}

/// Plans a range duplicate of `source` against the current connection list.
fn plan_range(
    state: &SharedAppState,
    source: &rustconn_core::Connection,
    name_pattern: &str,
    host_pattern: &str,
) -> Result<host_range::RangePlan, HostRangeError> {
    let state_ref = state.borrow();
    host_range::plan_range_duplicates(source, name_pattern, host_pattern, |name| {
        state_ref.connection_exists_by_name(name)
    })
}

/// Describes a range error for the dialog; core's text is untranslated.
fn range_error_text(error: &HostRangeError) -> String {
    match error {
        HostRangeError::InvalidPattern => i18n("Use one number range, such as host[02-10]"),
        HostRangeError::Reversed { .. } => i18n("The range must run from low to high"),
        HostRangeError::TooLarge { max } => i18n_f(
            "A range can create at most this many connections: {}",
            &[&max.to_string()],
        ),
        HostRangeError::SizeMismatch { .. } => {
            i18n("The name and host ranges must be the same size")
        }
        HostRangeError::InvalidConnection { name, .. } => {
            i18n_f("‘{}’ would not be a valid connection", &[name])
        }
    }
}

/// Which of the range dialog's rows to mark invalid, as `(name, host)`.
///
/// The error does not say which pattern it came from: a name pattern that does
/// not parse is the name's fault, and anything else (a host pattern that does
/// not parse, ranges of different sizes, an invalid host) is shown on the host.
fn invalid_range_rows(name_pattern: &str, failed: bool) -> (bool, bool) {
    if !failed {
        return (false, false);
    }
    let name_invalid = host_range::HostRange::parse(name_pattern).is_err();
    (name_invalid, !name_invalid)
}

/// Shows a row as invalid, to the eye and to a screen reader.
fn mark_invalid(row: &adw::EntryRow, invalid: bool) {
    if invalid {
        row.add_css_class("error");
    } else {
        row.remove_css_class("error");
    }
    // These bindings give adw rows no `IsA<Accessible>`; the widget has it.
    let widget = row.upcast_ref::<gtk4::Widget>();
    widget.update_state(&[gtk4::accessible::State::Invalid(if invalid {
        gtk4::AccessibleInvalidState::True
    } else {
        gtk4::AccessibleInvalidState::False
    })]);
}

/// Asks for a name and host range and duplicates the selected connection into
/// one connection per number.
pub fn duplicate_selected_as_range(
    window: &gtk4::Window,
    state: &SharedAppState,
    sidebar: &SharedSidebar,
) {
    let Some(item) = sidebar.get_selected_item() else {
        return;
    };
    if item.is_group() {
        return;
    }
    let Ok(id) = Uuid::parse_str(&item.id()) else {
        return;
    };
    let Some(source) = state
        .try_borrow()
        .ok()
        .and_then(|s| s.get_connection(id).cloned())
    else {
        return;
    };

    let dialog = adw::AlertDialog::new(
        Some(&i18n_f("Duplicate ‘{}’ as a Range", &[&source.name])),
        Some(&i18n(
            "Creates one copy per number. A leading zero in the first number keeps the numbers padded.",
        )),
    );
    let rows = adw::PreferencesGroup::new();
    // Enter in either row answers "Create" when it is enabled.
    let name_row = adw::EntryRow::builder()
        .title(i18n("Name Pattern"))
        .activates_default(true)
        .build();
    let host_row = adw::EntryRow::builder()
        .title(i18n("Host Pattern"))
        .activates_default(true)
        .build();
    rows.add(&name_row);
    rows.add(&host_row);
    let preview = gtk4::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .margin_top(12)
        .build();
    // The preview is the only place an error or the result is spelled out.
    for row in [&name_row, &host_row] {
        row.upcast_ref::<gtk4::Widget>().update_relation(&[
            gtk4::accessible::Relation::DescribedBy(&[preview.upcast_ref()]),
        ]);
    }
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    content.append(&rows);
    content.append(&preview);
    dialog.set_extra_child(Some(&content));

    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.add_response("create", &i18n("Create"));
    dialog.set_response_appearance("create", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("create"));
    dialog.set_close_response("cancel");

    let refresh: Rc<dyn Fn()> = Rc::new({
        let dialog = dialog.clone();
        let state = state.clone();
        let source = source.clone();
        let name_row = name_row.clone();
        let host_row = host_row.clone();
        let preview = preview.clone();
        move || {
            let plan = plan_range(&state, &source, &name_row.text(), &host_row.text());
            let ready = plan.as_ref().is_ok_and(|p| !p.created.is_empty());
            dialog.set_response_enabled("create", ready);
            let (name_invalid, host_invalid) = invalid_range_rows(&name_row.text(), plan.is_err());
            mark_invalid(&name_row, name_invalid);
            mark_invalid(&host_row, host_invalid);
            preview.set_css_classes(if plan.is_err() {
                &["error"]
            } else {
                &["dim-label"]
            });
            preview.set_text(&match &plan {
                Err(error) => range_error_text(error),
                Ok(plan) => match (plan.created.first(), plan.created.last()) {
                    (Some(first), Some(last)) => {
                        let summary = i18n_f(
                            "Connections to create: {} (‘{}’ to ‘{}’)",
                            &[&plan.created.len().to_string(), &first.name, &last.name],
                        );
                        if plan.skipped.is_empty() {
                            summary
                        } else {
                            let skipped = i18n_f(
                                "Already exist, will be skipped: {}",
                                &[&plan.skipped.len().to_string()],
                            );
                            format!("{summary}\n{skipped}")
                        }
                    }
                    _ => i18n("All of these names already exist"),
                },
            });
        }
    });

    // The host pattern tracks the name until the user types their own.
    let last_derived = Rc::new(std::cell::RefCell::new(String::new()));
    name_row.connect_changed({
        let source = source.clone();
        let host_row = host_row.clone();
        let refresh = Rc::clone(&refresh);
        move |row| {
            let derived = host_range::derive_host_pattern(&source, &row.text()).unwrap_or_default();
            let untouched = host_row.text() == last_derived.borrow().as_str();
            if untouched {
                *last_derived.borrow_mut() = derived.clone();
                host_row.set_text(&derived);
            }
            refresh();
        }
    });
    host_row.connect_changed({
        let refresh = Rc::clone(&refresh);
        move |_| refresh()
    });
    name_row.set_text(&host_range::suggest_name_pattern(&source.name).unwrap_or_default());
    refresh();

    let state = state.clone();
    let sidebar = sidebar.clone();
    let parent = window.clone();
    dialog.connect_response(None, move |_, response| {
        if response == "create" {
            create_range_duplicates(
                &parent,
                &state,
                &sidebar,
                &source,
                &name_row.text(),
                &host_row.text(),
            );
        }
    });
    dialog.present(Some(window));
}

/// Stores the copies a range describes, then carries over the saved password.
fn create_range_duplicates(
    window: &gtk4::Window,
    state: &SharedAppState,
    sidebar: &SharedSidebar,
    source: &rustconn_core::Connection,
    name_pattern: &str,
    host_pattern: &str,
) {
    let plan = match plan_range(state, source, name_pattern, host_pattern) {
        Ok(plan) => plan,
        Err(error) => {
            crate::toast::show_toast_on_window(
                window,
                &range_error_text(&error),
                crate::toast::ToastType::Error,
            );
            return;
        }
    };

    let Ok(mut state_mut) = state.try_borrow_mut() else {
        return;
    };
    let mut created = Vec::with_capacity(plan.created.len());
    let mut failed = Vec::new();
    for copy in plan.created {
        match state_mut
            .connection_manager()
            .create_connection_from(copy.clone())
        {
            Ok(_) => created.push(copy),
            Err(e) => {
                tracing::error!(name = %copy.name, "Failed to create range duplicate: {e}");
                failed.push(copy.name);
            }
        }
    }
    let settings = state_mut.settings().clone();
    let groups = state_mut.list_groups_owned();
    drop(state_mut);

    let message = range_result_text(created.len(), &failed, plan.skipped.len());
    {
        let state = state.clone();
        let sidebar = sidebar.clone();
        let window = window.clone();
        glib::idle_add_local_once(move || {
            MainWindow::reload_sidebar_preserving_state(&state, &sidebar);
            // A copy that was never stored is an unfinished action the user
            // has to redo, so it gets a dialog rather than a passing toast.
            if failed.is_empty() {
                crate::toast::show_toast_on_window(
                    &window,
                    &message,
                    crate::toast::ToastType::Success,
                );
            } else {
                alert::show_error(
                    &window,
                    &i18n("Some Connections Were Not Created"),
                    &message,
                );
            }
        });
    }

    // The same credential copy a single duplicate makes, one thread for all.
    if source.password_source == rustconn_core::models::PasswordSource::Vault {
        let source = source.clone();
        let window = window.clone();
        crate::utils::spawn_blocking_with_callback(
            move || {
                created
                    .iter()
                    .filter_map(|copy| {
                        crate::vault_ops::copy_vault_credential(&settings, &groups, &source, copy)
                            .err()
                    })
                    .collect::<Vec<String>>()
            },
            move |errors| {
                for error in &errors {
                    tracing::warn!(%error, "Failed to copy vault credential on range duplicate");
                }
                if !errors.is_empty() {
                    crate::toast::show_toast_on_window(
                        &window,
                        &i18n_f(
                            "The saved password was not copied to {} of the new connections",
                            &[&errors.len().to_string()],
                        ),
                        crate::toast::ToastType::Warning,
                    );
                }
            },
        );
    }
}

/// Describes what a range duplicate stored, skipped and failed to store.
fn range_result_text(created: usize, failed: &[String], skipped: usize) -> String {
    let created = created.to_string();
    if failed.is_empty() {
        return if skipped == 0 {
            i18n_f("Connections created: {}", &[&created])
        } else {
            i18n_f(
                "Connections created: {}, already existed: {}",
                &[&created, &skipped.to_string()],
            )
        };
    }
    let counts = if skipped == 0 {
        i18n_f(
            "Connections created: {}, failed: {}",
            &[&created, &failed.len().to_string()],
        )
    } else {
        i18n_f(
            "Connections created: {}, failed: {}, already existed: {}",
            &[&created, &failed.len().to_string(), &skipped.to_string()],
        )
    };
    let names = i18n_f("Not created: {}", &[&failed.join(", ")]);
    format!("{counts}\n{names}")
}

/// Toggles pin state of the selected connection
pub fn toggle_pin_selected(state: &SharedAppState, sidebar: &SharedSidebar) {
    let Some(conn_item) = sidebar.get_selected_item() else {
        return;
    };

    if conn_item.is_group() {
        return;
    }

    let id_str = conn_item.id();
    let Ok(id) = Uuid::parse_str(&id_str) else {
        return;
    };

    if let Ok(mut state_mut) = state.try_borrow_mut() {
        if let Err(e) = state_mut.toggle_pin(id) {
            tracing::error!(%e, "Failed to update pin state");
            return;
        }
        // Log the persisted state, read back after the write, so the log
        // reflects reality rather than a clone that was never stored.
        let is_pinned = state_mut.get_connection(id).is_some_and(|c| c.is_pinned);
        tracing::info!(
            connection_id = %id,
            is_pinned,
            "Connection pin state toggled"
        );
    }

    // Rebuild sidebar to reflect pin changes
    let state = state.clone();
    let sidebar = sidebar.clone();
    glib::idle_add_local_once(move || {
        MainWindow::reload_sidebar_preserving_state(&state, &sidebar);
    });
}

/// Copies the selected connection to the internal clipboard
pub fn copy_selected_connection(
    window: &gtk4::Window,
    state: &SharedAppState,
    sidebar: &SharedSidebar,
) {
    // Get selected item using sidebar's method
    let Some(conn_item) = sidebar.get_selected_item() else {
        return;
    };

    // Can only copy connections, not groups
    if conn_item.is_group() {
        return;
    }

    let id_str = conn_item.id();
    let Ok(id) = Uuid::parse_str(&id_str) else {
        return;
    };

    if let Ok(mut state_mut) = state.try_borrow_mut() {
        match state_mut.copy_connection(id) {
            Ok(()) => {
                crate::toast::show_toast_on_window(
                    window,
                    &i18n("Connection copied to clipboard"),
                    crate::toast::ToastType::Info,
                );
            }
            Err(e) => {
                tracing::error!("Failed to copy connection: {e}");
                crate::toast::show_toast_on_window(
                    window,
                    &i18n("Failed to copy connection"),
                    crate::toast::ToastType::Error,
                );
            }
        }
    }
}

/// Pastes a connection from the internal clipboard
pub fn paste_connection(window: &gtk4::Window, state: &SharedAppState, sidebar: &SharedSidebar) {
    // Check if clipboard has content
    let has_content = state
        .try_borrow()
        .map(|s| s.has_clipboard_content())
        .unwrap_or(false);

    if !has_content {
        crate::toast::show_toast_on_window(
            window,
            &i18n("Nothing to paste - copy a connection first"),
            crate::toast::ToastType::Warning,
        );
        return;
    }

    if let Ok(mut state_mut) = state.try_borrow_mut() {
        match state_mut.paste_connection() {
            Ok(_) => {
                drop(state_mut);
                // Defer sidebar reload to prevent UI freeze
                let state = state.clone();
                let sidebar = sidebar.clone();
                let window = window.clone();
                glib::idle_add_local_once(move || {
                    MainWindow::reload_sidebar_preserving_state(&state, &sidebar);
                    crate::toast::show_toast_on_window(
                        &window,
                        &i18n("Connection pasted"),
                        crate::toast::ToastType::Success,
                    );
                });
            }
            Err(e) => {
                tracing::error!("Failed to paste connection: {e}");
                crate::toast::show_toast_on_window(
                    window,
                    &i18n("Failed to paste connection"),
                    crate::toast::ToastType::Error,
                );
            }
        }
    }
}

/// Builds a list of items to delete with their display names
fn build_delete_item_list(
    state: &SharedAppState,
    selected_ids: &[Uuid],
) -> Option<(Vec<String>, usize, usize)> {
    let state_ref = state.try_borrow().ok()?;
    let mut names: Vec<String> = Vec::new();
    let mut conn_count = 0;
    let mut grp_count = 0;

    for id in selected_ids {
        if let Some(conn) = state_ref.get_connection(*id) {
            names.push(i18n_f("• {} (connection)", &[&conn.name]));
            conn_count += 1;
        } else if let Some(group) = state_ref.get_group(*id) {
            names.push(i18n_f("• {} (group)", &[&group.name]));
            grp_count += 1;
        }
    }
    Some((names, conn_count, grp_count))
}

/// Creates the bulk delete confirmation dialog UI
fn create_bulk_delete_dialog(
    window: &gtk4::Window,
    item_names: &[String],
    summary: &str,
    on_confirm: impl Fn() + 'static,
) {
    let body = format!(
        "{}\n\n{}\n\n{}",
        i18n_f("Are you sure you want to delete {}?", &[summary]),
        item_names.join("\n"),
        i18n("Connections in deleted groups will become ungrouped.")
    );

    let dialog = adw::AlertDialog::new(Some(&i18n("Delete Selected Items?")), Some(&body));
    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.add_response("delete", &i18n("Delete All"));
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);

    dialog.connect_response(None, move |_, response| {
        if response == "delete" {
            on_confirm();
        }
    });

    dialog.present(Some(window));
}

/// Performs bulk deletion and shows results
fn perform_bulk_delete(
    state: &SharedAppState,
    sidebar: &SharedSidebar,
    window: &gtk4::Window,
    selected_ids: Vec<Uuid>,
) {
    let mut success_count = 0;
    let mut failures: Vec<String> = Vec::new();

    if let Ok(mut state_mut) = state.try_borrow_mut() {
        for id in &selected_ids {
            // Try to delete as connection first, then as group
            let delete_result = state_mut
                .delete_connection(*id)
                .or_else(|_| state_mut.delete_group(*id));

            match delete_result {
                Ok(()) => {
                    success_count += 1;
                    // Bulk delete offers no Undo, so the deletion is permanent
                    // right away: purge the trash entry and its vault credential
                    // instead of leaving the credential behind (issue #263).
                    // Whichever purge does not match the item's kind is a no-op.
                    state_mut.purge_deleted_connection(*id);
                    state_mut.purge_deleted_group(*id);
                }
                Err(e) => failures.push(format!("{id}: {e}")),
            }
        }
    }

    // Defer sidebar reload to prevent UI freeze
    let state = state.clone();
    let sidebar = sidebar.clone();
    let window = window.clone();
    glib::idle_add_local_once(move || {
        MainWindow::reload_sidebar_preserving_state(&state, &sidebar);

        // Show results
        if failures.is_empty() {
            // Non-blocking success feedback (GNOME HIG: use a toast, not a dialog).
            crate::toast::show_toast_on_window(
                &window,
                &i18n_f(
                    "Successfully deleted {} item(s).",
                    &[&success_count.to_string()],
                ),
                crate::toast::ToastType::Success,
            );
        } else {
            alert::show_error(
                &window,
                &i18n("Deletion Partially Complete"),
                &format!(
                    "{}\n\n{}\n{}",
                    i18n_f("Deleted {} item(s).", &[&success_count.to_string()]),
                    i18n_f(
                        "Failed to delete {} item(s):",
                        &[&failures.len().to_string()]
                    ),
                    failures.join("\n")
                ),
            );
        }
    });
}

/// Deletes all selected connections (bulk delete for group operations mode)
pub fn delete_selected_connections(
    window: &gtk4::Window,
    state: &SharedAppState,
    sidebar: &SharedSidebar,
) {
    let selected_ids = sidebar.get_selected_ids();

    if selected_ids.is_empty() {
        alert::show_alert(
            window,
            &i18n("No Selection"),
            &i18n("Select one or more items to delete."),
        );
        return;
    }

    // Build list of items to delete for confirmation
    let Some((item_names, connection_count, group_count)) =
        build_delete_item_list(state, &selected_ids)
    else {
        return;
    };
    let summary = match (connection_count, group_count) {
        (c, 0) => i18n_f("{} connection(s)", &[&c.to_string()]),
        (0, g) => i18n_f("{} group(s)", &[&g.to_string()]),
        (c, g) => i18n_f(
            "{} connection(s) and {} group(s)",
            &[&c.to_string(), &g.to_string()],
        ),
    };

    // Create dialog
    let state_clone = state.clone();
    let sidebar_clone = sidebar.clone();
    let window_clone = window.clone();
    create_bulk_delete_dialog(window, &item_names, &summary, move || {
        perform_bulk_delete(
            &state_clone,
            &sidebar_clone,
            &window_clone,
            selected_ids.clone(),
        );
    });
}

/// Shows dialog to move selected items to a group (supports both connections and groups)
pub fn show_move_selected_to_group_dialog(
    window: &gtk4::Window,
    state: &SharedAppState,
    sidebar: &SharedSidebar,
) {
    let selected_ids = sidebar.get_selected_ids();

    if selected_ids.is_empty() {
        alert::show_alert(
            window,
            &i18n("No Selection"),
            &i18n("Please select one or more items to move."),
        );
        return;
    }

    // Separate connections and groups
    let (connection_ids, group_ids_to_move) = if let Ok(state_ref) = state.try_borrow() {
        let conn_ids: Vec<Uuid> = selected_ids
            .iter()
            .filter(|id| state_ref.get_connection(**id).is_some())
            .copied()
            .collect();
        let grp_ids: Vec<Uuid> = selected_ids
            .iter()
            .filter(|id| state_ref.get_group(**id).is_some())
            .copied()
            .collect();
        (conn_ids, grp_ids)
    } else {
        return;
    };

    let total_items = connection_ids.len() + group_ids_to_move.len();
    if total_items == 0 {
        return;
    }

    // Create dialog
    let move_window = adw::Window::builder()
        .title(i18n("Move"))
        .transient_for(window)
        .modal(true)
        .default_width(450)
        .build();

    // A separate top-level does not inherit the main window's CSS classes, so
    // the macOS overrides have to be tagged here too or this dialog keeps the
    // unscaled Adwaita font while the rest of the app is scaled up.
    #[cfg(target_os = "macos")]
    move_window.add_css_class("macos");

    let header = adw::HeaderBar::new();
    let cancel_btn = gtk4::Button::builder().label(i18n("Cancel")).build();
    let move_btn = gtk4::Button::builder()
        .label(i18n("Move"))
        .css_classes(["suggested-action"])
        .build();
    header.pack_start(&cancel_btn);
    header.pack_end(&move_btn);

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    let info_text = if !connection_ids.is_empty() && !group_ids_to_move.is_empty() {
        i18n_f(
            "Select destination for {} connection(s) and {} group(s):",
            &[
                &connection_ids.len().to_string(),
                &group_ids_to_move.len().to_string(),
            ],
        )
    } else if !connection_ids.is_empty() {
        i18n_f(
            "Select a group for {} connection(s):",
            &[&connection_ids.len().to_string()],
        )
    } else {
        i18n_f(
            "Select parent for {} group(s):",
            &[&group_ids_to_move.len().to_string()],
        )
    };

    let info_label = gtk4::Label::builder()
        .label(&info_text)
        .halign(gtk4::Align::Start)
        .build();
    content.append(&info_label);

    // Build group dropdown with hierarchical sorting
    let mut group_paths: Vec<(Uuid, String)> = if let Ok(state_ref) = state.try_borrow() {
        let groups: Vec<_> = state_ref
            .list_groups()
            .iter()
            .map(|g| (*g).clone())
            .collect();

        // Build paths for all groups, excluding groups being moved and their descendants
        groups
            .iter()
            .filter(|g| {
                // Exclude groups being moved
                if group_ids_to_move.contains(&g.id) {
                    return false;
                }
                // Exclude descendants of groups being moved
                for &moving_id in &group_ids_to_move {
                    if is_descendant_of_group(&state_ref, g.id, moving_id) {
                        return false;
                    }
                }
                true
            })
            .map(|g| {
                let path = state_ref
                    .get_group_path(g.id)
                    .unwrap_or_else(|| g.name.clone());
                (g.id, path)
            })
            .collect()
    } else {
        Vec::new()
    };

    // Sort by path (hierarchical + alphabetical)
    group_paths.sort_by_key(|a| a.1.to_lowercase());

    let mut group_ids: Vec<Option<Uuid>> = vec![None];
    let first_option = if group_ids_to_move.is_empty() {
        i18n("(No Group)")
    } else {
        i18n("(Root Level)")
    };
    let mut strings: Vec<String> = vec![first_option.clone()];

    for (id, path) in &group_paths {
        strings.push(path.clone());
        group_ids.push(Some(*id));
    }

    let string_list = gtk4::StringList::new(
        &strings
            .iter()
            .map(std::string::String::as_str)
            .collect::<Vec<_>>(),
    );
    let group_dropdown = gtk4::DropDown::builder()
        .model(&string_list)
        .selected(0)
        .hexpand(true)
        .build();

    content.append(&group_dropdown);

    // Use ToolbarView for proper adw::Window layout
    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&content));
    move_window.set_content(Some(&toolbar_view));

    // Connect cancel
    let window_clone = move_window.clone();
    cancel_btn.connect_clicked(move |_| {
        window_clone.close();
    });

    // Connect move
    let state_clone = state.clone();
    let sidebar_clone = sidebar.clone();
    let window_clone = move_window.clone();
    let parent_window = window.clone();
    move_btn.connect_clicked(move |_| {
        let choice_idx = group_dropdown.selected() as usize;
        let target_group = if choice_idx < group_ids.len() {
            group_ids[choice_idx]
        } else {
            None
        };

        let mut success_count = 0;
        let mut failures: Vec<String> = Vec::new();

        if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
            // Move connections
            for conn_id in &connection_ids {
                match state_mut.move_connection_to_group(*conn_id, target_group) {
                    Ok(()) => success_count += 1,
                    Err(e) => failures.push(format!("{conn_id}: {e}")),
                }
            }
            // Move groups
            for group_id in &group_ids_to_move {
                match state_mut.move_group_to_parent(*group_id, target_group) {
                    Ok(()) => success_count += 1,
                    Err(e) => failures.push(format!("{group_id}: {e}")),
                }
            }
        }

        // Defer sidebar reload to prevent UI freeze
        let state = state_clone.clone();
        let sidebar = sidebar_clone.clone();
        let window = window_clone.clone();
        let parent = parent_window.clone();
        let failures_clone = failures.clone();
        glib::idle_add_local_once(move || {
            MainWindow::reload_sidebar_preserving_state(&state, &sidebar);
            window.close();

            // Show results if there were failures
            if !failures_clone.is_empty() {
                alert::show_error(
                    &parent,
                    &i18n("Move Partially Complete"),
                    &format!(
                        "{}\n\n{}\n{}",
                        i18n_f("Moved {} item(s).", &[&success_count.to_string()]),
                        i18n_f(
                            "Failed to move {} item(s):",
                            &[&failures_clone.len().to_string()]
                        ),
                        failures_clone.join("\n")
                    ),
                );
            }
        });
    });

    move_window.present();
}

/// Checks if a group is a descendant of another group
fn is_descendant_of_group(
    state: &std::cell::Ref<crate::state::AppState>,
    group_id: Uuid,
    potential_ancestor: Uuid,
) -> bool {
    let mut current_id = state.get_group(group_id).and_then(|g| g.parent_id);
    while let Some(parent_id) = current_id {
        if parent_id == potential_ancestor {
            return true;
        }
        current_id = state.get_group(parent_id).and_then(|g| g.parent_id);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{invalid_range_rows, range_result_text};

    #[test]
    fn invalid_range_rows_blames_the_pattern_that_fails() {
        assert_eq!(invalid_range_rows("web[01-03]", false), (false, false));
        assert_eq!(invalid_range_rows("web", true), (true, false));
        assert_eq!(invalid_range_rows("web[03-01]", true), (true, false));
        // The name parses, so a failure is the host pattern's or the pair's.
        assert_eq!(invalid_range_rows("web[01-03]", true), (false, true));
    }

    #[test]
    fn range_result_counts_only_stored_copies() {
        assert_eq!(range_result_text(3, &[], 0), "Connections created: 3");
        assert_eq!(
            range_result_text(3, &[], 2),
            "Connections created: 3, already existed: 2"
        );
    }

    #[test]
    fn range_result_names_every_failed_copy() {
        let failed = ["web03".to_owned(), "web05".to_owned()];
        assert_eq!(
            range_result_text(1, &failed, 0),
            "Connections created: 1, failed: 2\nNot created: web03, web05"
        );
        assert_eq!(
            range_result_text(0, &failed, 4),
            "Connections created: 0, failed: 2, already existed: 4\nNot created: web03, web05"
        );
    }
}
