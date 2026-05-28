//! Snippet-related methods for the main window
//!
//! This module contains methods for managing and executing command snippets.
//! Dialogs use `adw::Dialog` for GNOME HIG compliance (bottom-sheet on narrow,
//! auto-close on Escape, drag-to-close).

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Button, Label, Orientation};
use libadwaita as adw;
use std::rc::Rc;
use uuid::Uuid;

use crate::alert;
use crate::dialogs::SnippetDialog;
use crate::i18n::i18n;
use crate::state::SharedAppState;
use crate::terminal::TerminalNotebook;
use crate::window::SharedToastOverlay;
use crate::window::types::SessionSplitBridges;

/// Type alias for shared terminal notebook
pub type SharedNotebook = Rc<TerminalNotebook>;

/// Sends text to the focused terminal, respecting split view focus.
///
/// If the active tab has a per-session split bridge, sends text to the
/// focused pane's session. Otherwise falls back to the tab's active terminal.
fn send_text_to_focused(
    notebook: &SharedNotebook,
    session_bridges: &SessionSplitBridges,
    text: &str,
) {
    // Check if the active tab has a per-session split bridge
    if let Some(active_session_id) = notebook.get_active_session_id() {
        let bridges = session_bridges.borrow();
        if let Some(bridge) = bridges.get(&active_session_id) {
            // Tab is split — send to the focused pane's session
            if let Some(focused_session_id) = bridge.get_focused_session() {
                notebook.send_text_to_session(focused_session_id, text);
                return;
            }
        }
    }
    // Fallback: send to the active tab's terminal
    notebook.send_text(text);
}

/// Shows the new snippet dialog
pub fn show_new_snippet_dialog(
    window: &gtk4::Window,
    state: SharedAppState,
    toast: SharedToastOverlay,
    notebook: SharedNotebook,
) {
    let dialog = SnippetDialog::new(Some(&window.clone().upcast()));

    let window_clone = window.clone();
    dialog.run(move |result| {
        if let Some(snippet) = result
            && let Ok(mut state_mut) = state.try_borrow_mut()
        {
            match state_mut.create_snippet(snippet) {
                Ok(_) => {
                    toast.show_success(&i18n("Snippet has been saved successfully."));
                    drop(state_mut);
                    notebook.rebuild_snippet_menu(&state);
                }
                Err(e) => {
                    crate::alert::show_error(&window_clone, &i18n("Error Creating Snippet"), &e);
                }
            }
        }
    });
}

/// Shows the snippets manager dialog
#[allow(
    clippy::too_many_lines,
    reason = "long match/dispatch over many enum variants; splitting per variant only relocates the boilerplate"
)]
pub fn show_snippets_manager(
    window: &gtk4::Window,
    state: SharedAppState,
    notebook: SharedNotebook,
    session_bridges: SessionSplitBridges,
) {
    let manager_dialog = adw::Dialog::builder()
        .title(i18n("Manage Snippets"))
        .content_width(600)
        .content_height(500)
        .build();

    // Header bar with Add button (GNOME HIG)
    let header = adw::HeaderBar::new();
    let new_btn = Button::from_icon_name("list-add-symbolic");
    new_btn.set_tooltip_text(Some(&i18n("New Snippet")));
    new_btn.update_property(&[gtk4::accessible::Property::Label(&i18n("New Snippet"))]);
    header.pack_start(&new_btn);

    // Create main content
    let content = gtk4::Box::new(Orientation::Vertical, 8);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    // Search entry
    let search_entry = gtk4::SearchEntry::new();
    search_entry.set_placeholder_text(Some(&i18n("Search snippets...")));
    content.append(&search_entry);

    // Snippets list
    let scrolled = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .vexpand(true)
        .build();

    let snippets_list = gtk4::ListBox::builder()
        .selection_mode(gtk4::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    scrolled.set_child(Some(&snippets_list));
    content.append(&scrolled);

    // Use ToolbarView for adw::Dialog layout
    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&content));
    manager_dialog.set_child(Some(&toolbar_view));

    // Populate snippets list with inline action buttons
    populate_snippets_manager_list(
        &state,
        &snippets_list,
        "",
        window,
        &manager_dialog,
        &notebook,
        &session_bridges,
    );

    // Connect search
    let state_clone = state.clone();
    let list_clone = snippets_list.clone();
    let window_clone = window.clone();
    let manager_clone = manager_dialog.clone();
    let notebook_clone = notebook.clone();
    let bridges_clone = session_bridges.clone();
    search_entry.connect_search_changed(move |entry| {
        let query = entry.text().to_string();
        populate_snippets_manager_list(
            &state_clone,
            &list_clone,
            &query,
            &window_clone,
            &manager_clone,
            &notebook_clone,
            &bridges_clone,
        );
    });

    // Connect new button
    let state_clone = state.clone();
    let list_clone = snippets_list.clone();
    let window_clone = window.clone();
    let manager_clone = manager_dialog.clone();
    let notebook_clone = notebook;
    let bridges_clone = session_bridges;
    new_btn.connect_clicked(move |_| {
        let dialog = SnippetDialog::new(Some(&window_clone));
        let state_inner = state_clone.clone();
        let list_inner = list_clone.clone();
        let window_inner = window_clone.clone();
        let manager_inner = manager_clone.clone();
        let notebook_inner = notebook_clone.clone();
        let bridges_inner = bridges_clone.clone();
        dialog.run(move |result| {
            if let Some(snippet) = result
                && let Ok(mut state_mut) = state_inner.try_borrow_mut()
            {
                if let Err(e) = state_mut.create_snippet(snippet) {
                    tracing::warn!(?e, "Failed to create snippet");
                }
                drop(state_mut);
                notebook_inner.rebuild_snippet_menu(&state_inner);
                populate_snippets_manager_list(
                    &state_inner,
                    &list_inner,
                    "",
                    &window_inner,
                    &manager_inner,
                    &notebook_inner,
                    &bridges_inner,
                );
            }
        });
    });

    manager_dialog.present(Some(window));
}

/// Populates the snippets manager list with inline action buttons per row
fn populate_snippets_manager_list(
    state: &SharedAppState,
    list: &gtk4::ListBox,
    query: &str,
    parent_window: &gtk4::Window,
    manager_dialog: &adw::Dialog,
    notebook: &SharedNotebook,
    session_bridges: &SessionSplitBridges,
) {
    // Clear existing rows
    while let Some(row) = list.row_at_index(0) {
        list.remove(&row);
    }

    let state_ref = state.borrow();
    let snippets = if query.is_empty() {
        state_ref.list_snippets()
    } else {
        state_ref.search_snippets(query)
    };

    for snippet in snippets {
        let row = gtk4::ListBoxRow::new();
        row.set_activatable(false);
        row.set_widget_name(&format!("snippet-{}", snippet.id));

        let hbox = gtk4::Box::new(Orientation::Horizontal, 8);
        hbox.set_margin_top(12);
        hbox.set_margin_bottom(12);
        hbox.set_margin_start(12);
        hbox.set_margin_end(12);

        let vbox = gtk4::Box::new(Orientation::Vertical, 2);
        vbox.set_hexpand(true);

        let name_label = Label::builder()
            .label(&snippet.name)
            .halign(gtk4::Align::Start)
            .css_classes(["heading"])
            .build();
        vbox.append(&name_label);

        let cmd_preview = if snippet.command.len() > 50 {
            let end = snippet
                .command
                .char_indices()
                .nth(50)
                .map_or(snippet.command.len(), |(i, _)| i);
            format!("{}…", &snippet.command[..end])
        } else {
            snippet.command.clone()
        };
        let cmd_label = Label::builder()
            .label(&cmd_preview)
            .halign(gtk4::Align::Start)
            .css_classes(["dim-label", "monospace"])
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        vbox.append(&cmd_label);

        hbox.append(&vbox);

        // Inline action buttons (GNOME HIG)
        let snippet_id = snippet.id;

        let execute_btn = Button::from_icon_name("media-playback-start-symbolic");
        execute_btn.add_css_class("flat");
        execute_btn.set_tooltip_text(Some(&i18n("Execute")));
        execute_btn.update_property(&[gtk4::accessible::Property::Label(&i18n("Execute snippet"))]);
        execute_btn.set_valign(gtk4::Align::Center);

        let edit_btn = Button::from_icon_name("document-edit-symbolic");
        edit_btn.add_css_class("flat");
        edit_btn.set_tooltip_text(Some(&i18n("Edit")));
        edit_btn.update_property(&[gtk4::accessible::Property::Label(&i18n("Edit snippet"))]);
        edit_btn.set_valign(gtk4::Align::Center);

        let delete_btn = Button::from_icon_name("user-trash-symbolic");
        delete_btn.add_css_class("flat");
        delete_btn.set_tooltip_text(Some(&i18n("Delete")));
        delete_btn.update_property(&[gtk4::accessible::Property::Label(&i18n("Delete snippet"))]);
        delete_btn.set_valign(gtk4::Align::Center);

        hbox.append(&execute_btn);
        hbox.append(&edit_btn);
        hbox.append(&delete_btn);

        row.set_child(Some(&hbox));
        list.append(&row);

        // Connect execute
        let state_exec = state.clone();
        let notebook_exec = notebook.clone();
        let parent_exec = parent_window.clone();
        let bridges_exec = session_bridges.clone();
        execute_btn.connect_clicked(move |_| {
            let state_ref = state_exec.borrow();
            if let Some(snippet) = state_ref.get_snippet(snippet_id).cloned() {
                drop(state_ref);
                execute_snippet(
                    &parent_exec,
                    &notebook_exec,
                    &bridges_exec,
                    &snippet,
                    &state_exec,
                );
            }
        });

        // Connect edit
        let state_edit = state.clone();
        let list_edit = list.clone();
        let parent_edit = parent_window.clone();
        let manager_edit = manager_dialog.clone();
        let notebook_edit = notebook.clone();
        let bridges_edit = session_bridges.clone();
        edit_btn.connect_clicked(move |_| {
            let state_ref = state_edit.borrow();
            if let Some(snippet) = state_ref.get_snippet(snippet_id).cloned() {
                drop(state_ref);
                let dialog = SnippetDialog::new(Some(&parent_edit));
                dialog.set_snippet(&snippet);
                let state_inner = state_edit.clone();
                let list_inner = list_edit.clone();
                let parent_inner = parent_edit.clone();
                let manager_inner = manager_edit.clone();
                let notebook_inner = notebook_edit.clone();
                let bridges_inner = bridges_edit.clone();
                dialog.run(move |result| {
                    if let Some(updated) = result
                        && let Ok(mut state_mut) = state_inner.try_borrow_mut()
                    {
                        if let Err(e) = state_mut.update_snippet(snippet_id, updated) {
                            tracing::warn!(?e, "Failed to update snippet");
                        }
                        drop(state_mut);
                        notebook_inner.rebuild_snippet_menu(&state_inner);
                        populate_snippets_manager_list(
                            &state_inner,
                            &list_inner,
                            "",
                            &parent_inner,
                            &manager_inner,
                            &notebook_inner,
                            &bridges_inner,
                        );
                    }
                });
            }
        });

        // Connect delete
        let state_del = state.clone();
        let list_del = list.clone();
        let parent_del = parent_window.clone();
        let manager_del = manager_dialog.clone();
        let notebook_del = notebook.clone();
        let bridges_del = session_bridges.clone();
        delete_btn.connect_clicked(move |_| {
            let state_inner = state_del.clone();
            let list_inner = list_del.clone();
            let parent_inner = parent_del.clone();
            let manager_inner = manager_del.clone();
            let notebook_inner = notebook_del.clone();
            let bridges_inner = bridges_del.clone();
            alert::show_confirm(
                &manager_del,
                &i18n("Delete Snippet?"),
                &i18n("Are you sure you want to delete this snippet?"),
                &i18n("Delete"),
                true,
                move |confirmed| {
                    if confirmed && let Ok(mut state_mut) = state_inner.try_borrow_mut() {
                        if let Err(e) = state_mut.delete_snippet(snippet_id) {
                            tracing::warn!(?e, "Failed to delete snippet");
                        }
                        drop(state_mut);
                        notebook_inner.rebuild_snippet_menu(&state_inner);
                        populate_snippets_manager_list(
                            &state_inner,
                            &list_inner,
                            "",
                            &parent_inner,
                            &manager_inner,
                            &notebook_inner,
                            &bridges_inner,
                        );
                    }
                },
            );
        });
    }
}

/// Populates the snippets list with filtered results.
///
/// Only shows snippets compatible with VTE terminals (`Terminal` or `Any` target).
pub fn populate_snippets_list(state: &SharedAppState, list: &gtk4::ListBox, query: &str) {
    // Clear existing rows
    while let Some(row) = list.row_at_index(0) {
        list.remove(&row);
    }

    let state_ref = state.borrow();
    let snippets = if query.is_empty() {
        state_ref.list_snippets()
    } else {
        state_ref.search_snippets(query)
    };

    for snippet in snippets
        .iter()
        .filter(|s| s.target.is_terminal_compatible())
    {
        let row = gtk4::ListBoxRow::new();
        row.set_widget_name(&format!("snippet-{}", snippet.id));

        let hbox = gtk4::Box::new(Orientation::Horizontal, 12);
        hbox.set_margin_top(12);
        hbox.set_margin_bottom(12);
        hbox.set_margin_start(12);
        hbox.set_margin_end(12);

        let vbox = gtk4::Box::new(Orientation::Vertical, 4);
        vbox.set_hexpand(true);

        let name_label = Label::builder()
            .label(&snippet.name)
            .halign(gtk4::Align::Start)
            .css_classes(["heading"])
            .build();
        vbox.append(&name_label);

        let cmd_preview = if snippet.command.len() > 50 {
            let end = snippet
                .command
                .char_indices()
                .nth(50)
                .map_or(snippet.command.len(), |(i, _)| i);
            format!("{}…", &snippet.command[..end])
        } else {
            snippet.command.clone()
        };
        let cmd_label = Label::builder()
            .label(&cmd_preview)
            .halign(gtk4::Align::Start)
            .css_classes(["dim-label", "monospace"])
            .build();
        vbox.append(&cmd_label);

        if let Some(ref cat) = snippet.category {
            let cat_label = Label::builder()
                .label(cat)
                .halign(gtk4::Align::Start)
                .css_classes(["dim-label"])
                .build();
            vbox.append(&cat_label);
        }

        hbox.append(&vbox);
        row.set_child(Some(&hbox));
        list.append(&row);
    }
}

/// Shows a snippet picker for quick execution
pub fn show_snippet_picker(
    window: &gtk4::Window,
    state: SharedAppState,
    notebook: SharedNotebook,
    session_bridges: SessionSplitBridges,
) {
    let picker_dialog = adw::Dialog::builder()
        .title(i18n("Execute Snippet"))
        .content_width(600)
        .content_height(500)
        .build();

    let header = adw::HeaderBar::new();

    let content = gtk4::Box::new(Orientation::Vertical, 8);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    let search_entry = gtk4::SearchEntry::new();
    search_entry.set_placeholder_text(Some(&i18n("Search snippets...")));
    content.append(&search_entry);

    let scrolled = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .vexpand(true)
        .build();

    let snippets_list = gtk4::ListBox::builder()
        .selection_mode(gtk4::SelectionMode::Single)
        .css_classes(["boxed-list"])
        .build();
    snippets_list.set_placeholder(Some(
        &adw::StatusPage::builder()
            .icon_name("edit-paste-symbolic")
            .title(i18n("No snippets available"))
            .description(i18n("Create snippets in the Manage Snippets dialog"))
            .build(),
    ));
    scrolled.set_child(Some(&snippets_list));
    content.append(&scrolled);

    // Use ToolbarView for adw::Dialog layout
    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&content));
    picker_dialog.set_child(Some(&toolbar_view));

    populate_snippets_list(&state, &snippets_list, "");

    // Connect search
    let state_clone = state.clone();
    let list_clone = snippets_list.clone();
    search_entry.connect_search_changed(move |entry| {
        let query = entry.text().to_string();
        populate_snippets_list(&state_clone, &list_clone, &query);
    });

    // Connect row activation (double-click or Enter)
    let state_clone = state;
    let notebook_clone = notebook;
    let dialog_clone = picker_dialog.clone();
    let window_clone = window.clone();
    let bridges_clone = session_bridges;
    snippets_list.connect_row_activated(move |_, row| {
        if let Some(id_str) = row.widget_name().as_str().strip_prefix("snippet-")
            && let Ok(id) = Uuid::parse_str(id_str)
        {
            let state_ref = state_clone.borrow();
            if let Some(snippet) = state_ref.get_snippet(id).cloned() {
                drop(state_ref);
                execute_snippet(
                    &window_clone,
                    &notebook_clone,
                    &bridges_clone,
                    &snippet,
                    &state_clone,
                );
                dialog_clone.close();
            }
        }
    });

    picker_dialog.present(Some(window));
}

/// Executes a snippet in the active terminal
pub fn execute_snippet(
    parent: &impl IsA<gtk4::Window>,
    notebook: &SharedNotebook,
    session_bridges: &SessionSplitBridges,
    snippet: &rustconn_core::models::Snippet,
    state: &SharedAppState,
) {
    // Check if there's an active terminal
    if notebook.get_active_terminal().is_none() {
        let window: &gtk4::Window = parent.upcast_ref();
        alert::show_error(
            window,
            &i18n("No Active Terminal"),
            &i18n("Please open a terminal session first before executing a snippet."),
        );
        return;
    }

    // Check if snippet has variables that need values
    let variables = rustconn_core::snippet::SnippetManager::extract_variables(&snippet.command);

    if variables.is_empty() {
        // No variables, execute directly
        send_text_to_focused(notebook, session_bridges, &format!("{}\n", snippet.command));
    } else {
        // Try to resolve variables from Global Variables first
        let state_ref = state.borrow();
        let global_variables = crate::state::resolve_global_variables(state_ref.settings());
        drop(state_ref);

        let mut var_manager = rustconn_core::variables::VariableManager::new();
        for var in &global_variables {
            var_manager.set_global(var.clone());
        }

        let mut resolved: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut unresolved: Vec<String> = Vec::new();

        for var_name in &variables {
            match var_manager.resolve(var_name, rustconn_core::variables::VariableScope::Global) {
                Ok(value) => {
                    resolved.insert(var_name.clone(), value);
                }
                Err(_) => {
                    // Check snippet-defined defaults as fallback
                    if let Some(var_def) = snippet.variables.iter().find(|v| &v.name == var_name)
                        && let Some(ref default) = var_def.default_value
                    {
                        resolved.insert(var_name.clone(), default.clone());
                    } else {
                        unresolved.push(var_name.clone());
                    }
                }
            }
        }

        if unresolved.is_empty() {
            // All variables resolved — execute directly
            let substituted = rustconn_core::snippet::SnippetManager::substitute_variables(
                &snippet.command,
                &resolved,
            );
            send_text_to_focused(notebook, session_bridges, &format!("{substituted}\n"));
        } else {
            // Some variables unresolved — show dialog with pre-filled values
            show_variable_input_dialog(parent, notebook, session_bridges, snippet, &resolved);
        }
    }
}

/// Shows a dialog to input variable values for a snippet
pub fn show_variable_input_dialog(
    parent: &impl IsA<gtk4::Window>,
    notebook: &SharedNotebook,
    session_bridges: &SessionSplitBridges,
    snippet: &rustconn_core::models::Snippet,
    prefilled: &std::collections::HashMap<String, String>,
) {
    let var_dialog = adw::Dialog::builder()
        .title(i18n("Enter Variable Values"))
        .content_width(450)
        .build();

    let (header, execute_btn) = crate::dialogs::widgets::dialog_header("Execute");

    let content = gtk4::Box::new(Orientation::Vertical, 8);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    let grid = gtk4::Grid::builder()
        .row_spacing(8)
        .column_spacing(12)
        .build();

    let mut entries: Vec<(String, gtk4::Entry)> = Vec::new();
    let variables = rustconn_core::snippet::SnippetManager::extract_variables(&snippet.command);

    for (i, var_name) in variables.iter().enumerate() {
        let label = Label::builder()
            .label(format!("{var_name}:"))
            .halign(gtk4::Align::End)
            .build();

        let entry = gtk4::Entry::builder().hexpand(true).build();

        // Set default value if available (prefilled from Global Variables takes priority)
        if let Some(prefilled_value) = prefilled.get(var_name) {
            entry.set_text(prefilled_value);
        } else if let Some(var_def) = snippet.variables.iter().find(|v| &v.name == var_name)
            && let Some(ref default) = var_def.default_value
        {
            entry.set_text(default);
        }

        // Set placeholder from snippet variable description
        if let Some(var_def) = snippet.variables.iter().find(|v| &v.name == var_name)
            && let Some(ref desc) = var_def.description
        {
            entry.set_placeholder_text(Some(desc));
        }

        #[expect(
            clippy::cast_possible_wrap,
            reason = "value range fits the target signed type by construction in this code path"
        )]
        let row_idx = i as i32;
        grid.attach(&label, 0, row_idx, 1, 1);
        grid.attach(&entry, 1, row_idx, 1, 1);
        entries.push((var_name.clone(), entry));
    }

    content.append(&grid);

    // Use ToolbarView for adw::Dialog layout
    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&content));
    var_dialog.set_child(Some(&toolbar_view));

    // Connect execute
    let dialog_clone = var_dialog.clone();
    let notebook_clone = notebook.clone();
    let bridges_clone = session_bridges.clone();
    let command = snippet.command.clone();
    execute_btn.connect_clicked(move |_| {
        let mut values = std::collections::HashMap::new();
        for (name, entry) in &entries {
            values.insert(name.clone(), entry.text().to_string());
        }

        let substituted =
            rustconn_core::snippet::SnippetManager::substitute_variables(&command, &values);
        send_text_to_focused(&notebook_clone, &bridges_clone, &format!("{substituted}\n"));
        dialog_clone.close();
    });

    let parent_widget: gtk4::Widget = parent.as_ref().clone().upcast();
    var_dialog.present(Some(&parent_widget));
}

/// Executes a snippet directly without a parent window dialog.
///
/// Used by the inline context menu action `win.run-snippet-direct`.
/// If the snippet has unresolved variables, falls back to the full
/// `execute_snippet` flow (which requires a parent window — not available
/// from a context menu action, so variables are resolved from globals/defaults only).
pub fn execute_snippet_direct(
    notebook: &SharedNotebook,
    session_bridges: &SessionSplitBridges,
    snippet: &rustconn_core::models::Snippet,
    state: &SharedAppState,
) {
    // Check if there's an active terminal
    if notebook.get_active_terminal().is_none() {
        return;
    }

    let variables = rustconn_core::snippet::SnippetManager::extract_variables(&snippet.command);

    if variables.is_empty() {
        send_text_to_focused(notebook, session_bridges, &format!("{}\n", snippet.command));
    } else {
        // Resolve variables from Global Variables and snippet defaults
        let state_ref = state.borrow();
        let global_variables = crate::state::resolve_global_variables(state_ref.settings());
        drop(state_ref);

        let mut var_manager = rustconn_core::variables::VariableManager::new();
        for var in &global_variables {
            var_manager.set_global(var.clone());
        }

        let mut resolved: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut has_unresolved = false;

        for var_name in &variables {
            match var_manager.resolve(var_name, rustconn_core::variables::VariableScope::Global) {
                Ok(value) => {
                    resolved.insert(var_name.clone(), value);
                }
                Err(_) => {
                    if let Some(var_def) = snippet.variables.iter().find(|v| &v.name == var_name)
                        && let Some(ref default) = var_def.default_value
                    {
                        resolved.insert(var_name.clone(), default.clone());
                    } else {
                        has_unresolved = true;
                        break;
                    }
                }
            }
        }

        if !has_unresolved {
            let substituted = rustconn_core::snippet::SnippetManager::substitute_variables(
                &snippet.command,
                &resolved,
            );
            send_text_to_focused(notebook, session_bridges, &format!("{substituted}\n"));
        }
        // If unresolved variables remain, silently skip — the user should
        // use the full "Execute Snippet…" picker which shows the variable dialog.
    }
}
