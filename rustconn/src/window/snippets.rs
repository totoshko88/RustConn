//! Snippet-related methods for the main window
//!
//! This module contains methods for managing and executing command snippets.
//! Dialogs use `adw::Dialog` for GNOME HIG compliance (bottom-sheet on narrow,
//! auto-close on Escape, drag-to-close).

use std::rc::Rc;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Button, Label, Orientation};
use libadwaita as adw;
use uuid::Uuid;

use crate::alert;
use crate::dialogs::SnippetDialog;
use crate::i18n::{i18n, i18n_f};
use crate::state::SharedAppState;
use crate::terminal::TerminalNotebook;
use crate::window::types::SessionSplitBridges;

/// Type alias for shared terminal notebook
pub type SharedNotebook = Rc<TerminalNotebook>;

/// Longest command shown verbatim in the confirmation dialog.
///
/// The point of the dialog is that the user reads what is about to run, so the
/// command is shown in full rather than summarised. A generated one-liner can be
/// arbitrarily long, though, and an `AlertDialog` grows with its body until it
/// stops being readable — this caps it while still showing far more than the
/// 50-character preview used in the snippet lists.
const CONFIRM_COMMAND_PREVIEW_LIMIT: usize = 400;

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

/// Sends a snippet's resolved command to the focused terminal, asking first when
/// the snippet opts into confirmation.
///
/// Every terminal execution path funnels through this function, so
/// `confirm_before_run` cannot be bypassed by the route the user took: the
/// picker, the manager's Execute button, the variable-input dialog and the
/// inline context-menu item all arrive here. `command` is the substituted
/// command *without* a trailing newline — this function adds the newline that
/// makes the shell run it.
///
/// `after_send` runs only once the command has actually been sent, so a caller
/// that owns a dialog can keep it open when the user cancels.
fn send_snippet_command(
    parent: &gtk4::Widget,
    notebook: &SharedNotebook,
    session_bridges: &SessionSplitBridges,
    snippet: &rustconn_core::models::Snippet,
    command: &str,
    after_send: impl Fn() + 'static,
) {
    // Never log `command` itself. Variable substitution has already happened by
    // this point, so it can carry values resolved from vault-backed global
    // variables. The length is enough to tell "sent something" from "sent
    // nothing" without putting a secret in the log.
    if !snippet.confirm_before_run {
        tracing::debug!(
            snippet = %snippet.name,
            snippet_id = %snippet.id,
            command_bytes = command.len(),
            "Sending snippet to the focused terminal"
        );
        send_text_to_focused(notebook, session_bridges, &format!("{command}\n"));
        after_send();
        return;
    }

    let notebook_confirm = notebook.clone();
    let bridges_confirm = session_bridges.clone();
    let command_confirm = command.to_string();
    let name_confirm = snippet.name.clone();
    let id_confirm = snippet.id;

    tracing::debug!(
        snippet = %snippet.name,
        snippet_id = %snippet.id,
        "Snippet asks for confirmation; waiting for the user"
    );

    alert::show_confirm(
        parent,
        &i18n("Run Snippet?"),
        &i18n_f(
            "This runs “{}” in the active session:\n\n{}",
            &[&snippet.name, &truncate_command(command)],
        ),
        &i18n("Run"),
        // The user marked this snippet as needing confirmation, which is a
        // statement that running it by accident is expensive. Destructive
        // appearance carries that; the default response stays Cancel.
        true,
        move |confirmed| {
            if confirmed {
                tracing::debug!(
                    snippet = %name_confirm,
                    snippet_id = %id_confirm,
                    command_bytes = command_confirm.len(),
                    "Snippet confirmed; sending to the focused terminal"
                );
                send_text_to_focused(
                    &notebook_confirm,
                    &bridges_confirm,
                    &format!("{command_confirm}\n"),
                );
                after_send();
            } else {
                // The state the confirmation flag creates that did not exist
                // before: the user deliberately declined. Without this line it
                // is indistinguishable from a dialog that never appeared.
                tracing::debug!(
                    snippet = %name_confirm,
                    snippet_id = %id_confirm,
                    "Snippet run cancelled at the confirmation dialog"
                );
            }
        },
    );
}

/// Resolves a snippet's variables from global variables and its own defaults.
///
/// Returns what it could resolve and the names it could not, so a caller can
/// pre-fill the variable dialog with the former and ask only for the latter.
///
/// One function because the two callers had a copy each and they had drifted:
/// `execute_snippet` collected every unresolved name, while
/// `execute_snippet_direct` stopped at the first, so the same snippet arrived at
/// the same dialog with different fields pre-filled depending on which menu the
/// user came from. That only became visible once the direct path started opening
/// the dialog at all instead of dropping the snippet.
fn resolve_snippet_variables(
    snippet: &rustconn_core::models::Snippet,
    state: &SharedAppState,
) -> (std::collections::HashMap<String, String>, Vec<String>) {
    use rustconn_core::variables::{VariableManager, VariableScope};

    let variables = rustconn_core::snippet::SnippetManager::extract_variables(&snippet.command);

    let state_ref = state.borrow();
    let global_variables = crate::state::resolve_global_variables(state_ref.settings());
    drop(state_ref);

    let mut var_manager = VariableManager::new();
    for var in &global_variables {
        var_manager.set_global(var.clone());
    }

    let mut resolved = std::collections::HashMap::new();
    let mut unresolved = Vec::new();

    for var_name in &variables {
        match var_manager.resolve(var_name, VariableScope::Global) {
            Ok(value) => {
                resolved.insert(var_name.clone(), value);
            }
            // A global variable is the first choice; the snippet's own default is
            // the fallback. Neither means the user has to be asked.
            Err(_) => {
                if let Some(default) = snippet
                    .variables
                    .iter()
                    .find(|v| &v.name == var_name)
                    .and_then(|v| v.default_value.clone())
                {
                    resolved.insert(var_name.clone(), default);
                } else {
                    unresolved.push(var_name.clone());
                }
            }
        }
    }

    (resolved, unresolved)
}

/// Shortens a command for display, respecting character boundaries.
pub(crate) fn truncate_command(command: &str) -> String {
    match command.char_indices().nth(CONFIRM_COMMAND_PREVIEW_LIMIT) {
        Some((idx, _)) => format!("{}…", &command[..idx]),
        None => command.to_string(),
    }
}

/// Shows the snippets manager dialog
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
    search_entry.set_placeholder_text(Some(&i18n("Search snippets…")));
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
    search_entry.set_placeholder_text(Some(&i18n("Search snippets…")));
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
            &i18n("Open a terminal session first before executing a snippet."),
        );
        return;
    }

    // Check if snippet has variables that need values
    let variables = rustconn_core::snippet::SnippetManager::extract_variables(&snippet.command);

    if variables.is_empty() {
        // No variables, execute directly
        send_snippet_command(
            parent.as_ref().upcast_ref(),
            notebook,
            session_bridges,
            snippet,
            &snippet.command,
            || {},
        );
    } else {
        let (resolved, unresolved) = resolve_snippet_variables(snippet, state);

        if unresolved.is_empty() {
            // All variables resolved — execute directly
            let substituted = rustconn_core::snippet::SnippetManager::substitute_variables(
                &snippet.command,
                &resolved,
            );
            send_snippet_command(
                parent.as_ref().upcast_ref(),
                notebook,
                session_bridges,
                snippet,
                &substituted,
                || {},
            );
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
    let snippet_clone = snippet.clone();
    execute_btn.connect_clicked(move |_| {
        let mut values = std::collections::HashMap::new();
        for (name, entry) in &entries {
            values.insert(name.clone(), entry.text().to_string());
        }

        let substituted = rustconn_core::snippet::SnippetManager::substitute_variables(
            &snippet_clone.command,
            &values,
        );
        // Parented on this dialog, and closed only after the command is sent, so
        // cancelling the confirmation returns to the values the user typed
        // instead of discarding them.
        let dialog_after_send = dialog_clone.clone();
        send_snippet_command(
            dialog_clone.upcast_ref(),
            &notebook_clone,
            &bridges_clone,
            &snippet_clone,
            &substituted,
            move || {
                dialog_after_send.close();
            },
        );
    });

    let parent_widget: gtk4::Widget = parent.as_ref().clone().upcast();
    var_dialog.present(Some(&parent_widget));
}

/// Executes a snippet without opening the picker.
///
/// Used by the inline context menu action `win.run-snippet-direct`, which names a
/// snippet outright, so the picker is skipped. Variables are resolved from global
/// variables and snippet defaults first; when that leaves any unresolved, the
/// variable-input dialog opens rather than nothing happening.
///
/// That dialog used to be unreachable from here and the snippet was dropped in
/// silence, on the stated grounds that a context-menu action has no parent window
/// to present one on. It has had one since the confirmation gate needed somewhere
/// to anchor — the reason in the old comment outlived the constraint it described,
/// which left the one route where picking a snippet from a menu could do nothing
/// at all and log nothing either.
///
/// `parent` is the window the terminal lives in, and is unused on the path where
/// everything resolves and the snippet does not ask for confirmation.
pub fn execute_snippet_direct(
    parent: &impl IsA<gtk4::Window>,
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
        send_snippet_command(
            parent.as_ref().upcast_ref(),
            notebook,
            session_bridges,
            snippet,
            &snippet.command,
            || {},
        );
    } else {
        let (resolved, unresolved) = resolve_snippet_variables(snippet, state);

        if unresolved.is_empty() {
            let substituted = rustconn_core::snippet::SnippetManager::substitute_variables(
                &snippet.command,
                &resolved,
            );
            send_snippet_command(
                parent.as_ref().upcast_ref(),
                notebook,
                session_bridges,
                snippet,
                &substituted,
                || {},
            );
        } else {
            // Ask, rather than drop the snippet. Whatever did resolve is passed
            // through, so only the genuinely missing values need typing.
            tracing::debug!(
                snippet = %snippet.name,
                snippet_id = %snippet.id,
                unresolved = unresolved.len(),
                "Snippet has unresolved variables; asking for them"
            );
            show_variable_input_dialog(parent, notebook, session_bridges, snippet, &resolved);
        }
    }
}
