//! Snippet-related methods for the main window
//!
//! This module contains methods for managing and executing command snippets.

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

/// Type alias for shared terminal notebook
pub type SharedNotebook = Rc<TerminalNotebook>;

/// Shows the new snippet dialog
pub fn show_new_snippet_dialog(window: &gtk4::Window, state: SharedAppState) {
    let dialog = SnippetDialog::new(Some(&window.clone().upcast()));

    let window_clone = window.clone();
    dialog.run(move |result| {
        if let Some(snippet) = result
            && let Ok(mut state_mut) = state.try_borrow_mut()
        {
            match state_mut.create_snippet(snippet) {
                Ok(_) => {
                    alert::show_success(
                        &window_clone,
                        &i18n("Snippet Created"),
                        &i18n("Snippet has been saved successfully."),
                    );
                }
                Err(e) => {
                    alert::show_error(&window_clone, &i18n("Error Creating Snippet"), &e);
                }
            }
        }
    });
}

/// Shows the snippets manager window
#[allow(clippy::too_many_lines)]
pub fn show_snippets_manager(
    window: &gtk4::Window,
    state: SharedAppState,
    notebook: SharedNotebook,
) {
    let manager_window = adw::Window::builder()
        .title(i18n("Manage Snippets"))
        .transient_for(window)
        .modal(true)
        .default_width(500)
        .default_height(400)
        .build();

    manager_window.set_size_request(320, 280);

    // Create header bar with Close/Create buttons (GNOME HIG)
    let header = adw::HeaderBar::new();
    let close_btn = Button::builder().label(&i18n("Close")).build();
    let new_btn = Button::builder()
        .label(&i18n("Create"))
        .css_classes(["suggested-action"])
        .build();
    header.pack_start(&close_btn);
    header.pack_end(&new_btn);

    // Close button handler
    let window_clone = manager_window.clone();
    close_btn.connect_clicked(move |_| {
        window_clone.close();
    });

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
        .selection_mode(gtk4::SelectionMode::Single)
        .css_classes(["boxed-list"])
        .build();
    scrolled.set_child(Some(&snippets_list));
    content.append(&scrolled);

    // Action buttons
    let button_box = gtk4::Box::new(Orientation::Horizontal, 8);
    button_box.set_halign(gtk4::Align::End);

    let edit_btn = Button::builder()
        .label(&i18n("Edit"))
        .sensitive(false)
        .build();
    let delete_btn = Button::builder()
        .label(&i18n("Delete"))
        .sensitive(false)
        .build();
    let execute_btn = Button::builder()
        .label(&i18n("Execute"))
        .sensitive(false)
        .css_classes(["suggested-action"])
        .build();

    button_box.append(&edit_btn);
    button_box.append(&delete_btn);
    button_box.append(&execute_btn);
    content.append(&button_box);

    // Use ToolbarView for proper adw::Window layout
    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&content));
    manager_window.set_content(Some(&toolbar_view));

    // Populate snippets list
    populate_snippets_list(&state, &snippets_list, "");

    // Connect search
    let state_clone = state.clone();
    let list_clone = snippets_list.clone();
    search_entry.connect_search_changed(move |entry| {
        let query = entry.text().to_string();
        populate_snippets_list(&state_clone, &list_clone, &query);
    });

    // Connect selection changed
    let edit_clone = edit_btn.clone();
    let delete_clone = delete_btn.clone();
    let execute_clone = execute_btn.clone();
    snippets_list.connect_row_selected(move |_, row| {
        let has_selection = row.is_some();
        edit_clone.set_sensitive(has_selection);
        delete_clone.set_sensitive(has_selection);
        execute_clone.set_sensitive(has_selection);
    });

    // Connect new button
    let state_clone = state.clone();
    let list_clone = snippets_list.clone();
    let manager_clone = manager_window.clone();
    new_btn.connect_clicked(move |_| {
        let dialog = SnippetDialog::new(Some(&manager_clone.clone().upcast()));
        let state_inner = state_clone.clone();
        let list_inner = list_clone.clone();
        dialog.run(move |result| {
            if let Some(snippet) = result
                && let Ok(mut state_mut) = state_inner.try_borrow_mut()
            {
                if let Err(e) = state_mut.create_snippet(snippet) {
                    tracing::warn!(?e, "Failed to create snippet");
                }
                drop(state_mut);
                populate_snippets_list(&state_inner, &list_inner, "");
            }
        });
    });

    // Connect edit button
    let state_clone = state.clone();
    let list_clone = snippets_list.clone();
    let manager_clone = manager_window.clone();
    edit_btn.connect_clicked(move |_| {
        if let Some(row) = list_clone.selected_row()
            && let Some(id_str) = row.widget_name().as_str().strip_prefix("snippet-")
            && let Ok(id) = Uuid::parse_str(id_str)
        {
            let state_ref = state_clone.borrow();
            if let Some(snippet) = state_ref.get_snippet(id).cloned() {
                drop(state_ref);
                let dialog = SnippetDialog::new(Some(&manager_clone.clone().upcast()));
                dialog.set_snippet(&snippet);
                let state_inner = state_clone.clone();
                let list_inner = list_clone.clone();
                dialog.run(move |result| {
                    if let Some(updated) = result
                        && let Ok(mut state_mut) = state_inner.try_borrow_mut()
                    {
                        if let Err(e) = state_mut.update_snippet(id, updated) {
                            tracing::warn!(?e, "Failed to update snippet");
                        }
                        drop(state_mut);
                        populate_snippets_list(&state_inner, &list_inner, "");
                    }
                });
            }
        }
    });

    // Connect delete button
    let state_clone = state.clone();
    let list_clone = snippets_list.clone();
    let manager_clone = manager_window.clone();
    delete_btn.connect_clicked(move |_| {
        if let Some(row) = list_clone.selected_row()
            && let Some(id_str) = row.widget_name().as_str().strip_prefix("snippet-")
            && let Ok(id) = Uuid::parse_str(id_str)
        {
            let state_inner = state_clone.clone();
            let list_inner = list_clone.clone();
            alert::show_confirm(
                &manager_clone,
                &i18n("Delete Snippet?"),
                &i18n("Are you sure you want to delete this snippet?"),
                &i18n("Delete"),
                true,
                move |confirmed| {
                    if confirmed && let Ok(mut state_mut) = state_inner.try_borrow_mut() {
                        if let Err(e) = state_mut.delete_snippet(id) {
                            tracing::warn!(?e, "Failed to delete snippet");
                        }
                        drop(state_mut);
                        populate_snippets_list(&state_inner, &list_inner, "");
                    }
                },
            );
        }
    });

    // Connect execute button
    let state_clone = state;
    let list_clone = snippets_list;
    let notebook_clone = notebook;
    let manager_clone = manager_window.clone();
    execute_btn.connect_clicked(move |_| {
        if let Some(row) = list_clone.selected_row()
            && let Some(id_str) = row.widget_name().as_str().strip_prefix("snippet-")
            && let Ok(id) = Uuid::parse_str(id_str)
        {
            let state_ref = state_clone.borrow();
            if let Some(snippet) = state_ref.get_snippet(id).cloned() {
                drop(state_ref);
                execute_snippet(&manager_clone, &notebook_clone, &snippet);
            }
        }
    });

    manager_window.present();
}

/// Populates the snippets list with filtered results
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

    for snippet in snippets {
        let row = gtk4::ListBoxRow::new();
        row.set_widget_name(&format!("snippet-{}", snippet.id));

        let hbox = gtk4::Box::new(Orientation::Horizontal, 12);
        hbox.set_margin_top(8);
        hbox.set_margin_bottom(8);
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
            format!("{}...", &snippet.command[..50])
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
pub fn show_snippet_picker(window: &gtk4::Window, state: SharedAppState, notebook: SharedNotebook) {
    let picker_window = adw::Window::builder()
        .title(i18n("Execute Snippet"))
        .transient_for(window)
        .modal(true)
        .default_width(400)
        .default_height(400)
        .build();

    let header = adw::HeaderBar::new();
    let cancel_btn = Button::builder().label(&i18n("Cancel")).build();
    header.pack_start(&cancel_btn);

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
    scrolled.set_child(Some(&snippets_list));
    content.append(&scrolled);

    // Use ToolbarView for proper adw::Window layout
    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&content));
    picker_window.set_content(Some(&toolbar_view));

    populate_snippets_list(&state, &snippets_list, "");

    // Connect search
    let state_clone = state.clone();
    let list_clone = snippets_list.clone();
    search_entry.connect_search_changed(move |entry| {
        let query = entry.text().to_string();
        populate_snippets_list(&state_clone, &list_clone, &query);
    });

    // Connect cancel
    let window_clone = picker_window.clone();
    cancel_btn.connect_clicked(move |_| {
        window_clone.close();
    });

    // Connect row activation (double-click or Enter)
    let state_clone = state;
    let notebook_clone = notebook;
    let window_clone = picker_window.clone();
    snippets_list.connect_row_activated(move |_, row| {
        if let Some(id_str) = row.widget_name().as_str().strip_prefix("snippet-")
            && let Ok(id) = Uuid::parse_str(id_str)
        {
            let state_ref = state_clone.borrow();
            if let Some(snippet) = state_ref.get_snippet(id).cloned() {
                drop(state_ref);
                execute_snippet(&window_clone, &notebook_clone, &snippet);
                window_clone.close();
            }
        }
    });

    picker_window.present();
}

/// Executes a snippet in the active terminal
pub fn execute_snippet(
    parent: &impl IsA<gtk4::Window>,
    notebook: &SharedNotebook,
    snippet: &rustconn_core::models::Snippet,
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
        notebook.send_text(&format!("{}\n", snippet.command));
    } else {
        // Show variable input dialog
        show_variable_input_dialog(parent, notebook, snippet);
    }
}

/// Shows a dialog to input variable values for a snippet
pub fn show_variable_input_dialog(
    parent: &impl IsA<gtk4::Window>,
    notebook: &SharedNotebook,
    snippet: &rustconn_core::models::Snippet,
) {
    let var_window = adw::Window::builder()
        .title(i18n("Enter Variable Values"))
        .transient_for(parent)
        .modal(true)
        .default_width(400)
        .build();

    let header = adw::HeaderBar::new();
    let cancel_btn = Button::builder().label(&i18n("Cancel")).build();
    let execute_btn = Button::builder()
        .label(&i18n("Execute"))
        .css_classes(["suggested-action"])
        .build();
    header.pack_start(&cancel_btn);
    header.pack_end(&execute_btn);

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

        // Set default value if available
        if let Some(var_def) = snippet.variables.iter().find(|v| &v.name == var_name) {
            if let Some(ref default) = var_def.default_value {
                entry.set_text(default);
            }
            if let Some(ref desc) = var_def.description {
                entry.set_placeholder_text(Some(desc));
            }
        }

        #[allow(clippy::cast_possible_wrap)]
        let row_idx = i as i32;
        grid.attach(&label, 0, row_idx, 1, 1);
        grid.attach(&entry, 1, row_idx, 1, 1);
        entries.push((var_name.clone(), entry));
    }

    content.append(&grid);

    // Use ToolbarView for proper adw::Window layout
    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&content));
    var_window.set_content(Some(&toolbar_view));

    // Connect cancel
    let window_clone = var_window.clone();
    cancel_btn.connect_clicked(move |_| {
        window_clone.close();
    });

    // Connect execute
    let window_clone = var_window.clone();
    let notebook_clone = notebook.clone();
    let command = snippet.command.clone();
    execute_btn.connect_clicked(move |_| {
        let mut values = std::collections::HashMap::new();
        for (name, entry) in &entries {
            values.insert(name.clone(), entry.text().to_string());
        }

        let substituted =
            rustconn_core::snippet::SnippetManager::substitute_variables(&command, &values);
        notebook_clone.send_text(&format!("{substituted}\n"));
        window_clone.close();
    });

    var_window.present();
}
