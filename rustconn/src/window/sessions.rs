//! Session management methods for the main window
//!
//! This module contains methods for managing active sessions,
//! including the sessions manager dialog and related functionality.

use crate::alert;
use gtk4::prelude::*;
use gtk4::{Button, HeaderBar, Label, Orientation};
use std::rc::Rc;
use uuid::Uuid;

use crate::sidebar::ConnectionSidebar;
use crate::state::SharedAppState;
use crate::terminal::TerminalNotebook;

/// Type alias for shared terminal notebook
pub type SharedNotebook = Rc<TerminalNotebook>;

/// Type alias for shared sidebar
pub type SharedSidebar = Rc<ConnectionSidebar>;

/// Shows the sessions manager window
#[allow(clippy::too_many_lines)]
pub fn show_sessions_manager(
    window: &gtk4::Window,
    state: SharedAppState,
    notebook: SharedNotebook,
    sidebar: SharedSidebar,
) {
    let manager_window = gtk4::Window::builder()
        .title("Active Sessions")
        .transient_for(window)
        .modal(true)
        .default_width(750)
        .default_height(500)
        .build();

    // Create header bar with Close/Refresh buttons (GNOME HIG)
    let header = HeaderBar::new();
    header.set_show_title_buttons(false);
    let close_btn = Button::builder().label("Close").build();
    let refresh_btn = Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text("Refresh")
        .build();
    header.pack_start(&close_btn);
    header.pack_end(&refresh_btn);
    manager_window.set_titlebar(Some(&header));

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

    // Session count label
    let count_label = Label::builder()
        .halign(gtk4::Align::Start)
        .css_classes(["dim-label"])
        .build();
    content.append(&count_label);

    // Sessions list
    let scrolled = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .vexpand(true)
        .build();

    let sessions_list = gtk4::ListBox::builder()
        .selection_mode(gtk4::SelectionMode::Single)
        .css_classes(["boxed-list"])
        .build();
    scrolled.set_child(Some(&sessions_list));
    content.append(&scrolled);

    // Action buttons
    let button_box = gtk4::Box::new(Orientation::Horizontal, 8);
    button_box.set_halign(gtk4::Align::End);

    let switch_btn = Button::builder()
        .label("Switch To")
        .sensitive(false)
        .build();
    let send_text_btn = Button::builder()
        .label("Send Text")
        .sensitive(false)
        .build();
    let terminate_btn = Button::builder()
        .label("Terminate")
        .sensitive(false)
        .css_classes(["destructive-action"])
        .build();

    button_box.append(&switch_btn);
    button_box.append(&send_text_btn);
    button_box.append(&terminate_btn);
    content.append(&button_box);

    manager_window.set_child(Some(&content));

    // Populate sessions list
    populate_sessions_list(&state, &notebook, &sessions_list, &count_label);

    // Connect selection changed
    let switch_clone = switch_btn.clone();
    let send_text_clone = send_text_btn.clone();
    let terminate_clone = terminate_btn.clone();
    sessions_list.connect_row_selected(move |_, row| {
        let has_selection = row.is_some();
        switch_clone.set_sensitive(has_selection);
        send_text_clone.set_sensitive(has_selection);
        terminate_clone.set_sensitive(has_selection);
    });

    // Connect refresh button
    let state_clone = state.clone();
    let notebook_clone = notebook.clone();
    let list_clone = sessions_list.clone();
    let count_clone = count_label.clone();
    refresh_btn.connect_clicked(move |_| {
        populate_sessions_list(&state_clone, &notebook_clone, &list_clone, &count_clone);
    });

    // Connect switch button
    let notebook_clone = notebook.clone();
    let list_clone = sessions_list.clone();
    let window_clone = manager_window.clone();
    switch_btn.connect_clicked(move |_| {
        if let Some(row) = list_clone.selected_row() {
            if let Some(id_str) = row.widget_name().as_str().strip_prefix("session-") {
                if let Ok(id) = Uuid::parse_str(id_str) {
                    notebook_clone.switch_to_tab(id);
                    window_clone.close();
                }
            }
        }
    });

    // Connect send text button
    let notebook_clone = notebook.clone();
    let list_clone = sessions_list.clone();
    let manager_clone = manager_window.clone();
    send_text_btn.connect_clicked(move |_| {
        if let Some(row) = list_clone.selected_row() {
            if let Some(id_str) = row.widget_name().as_str().strip_prefix("session-") {
                if let Ok(session_id) = Uuid::parse_str(id_str) {
                    show_send_text_dialog(&manager_clone, &notebook_clone, session_id);
                }
            }
        }
    });

    // Connect terminate button
    let state_clone = state;
    let notebook_clone = notebook;
    let list_clone = sessions_list;
    let count_clone = count_label;
    let manager_clone = manager_window.clone();
    let sidebar_clone = sidebar;
    terminate_btn.connect_clicked(move |_| {
        if let Some(row) = list_clone.selected_row() {
            if let Some(id_str) = row.widget_name().as_str().strip_prefix("session-") {
                if let Ok(id) = Uuid::parse_str(id_str) {
                    let state_inner = state_clone.clone();
                    let notebook_inner = notebook_clone.clone();
                    let list_inner = list_clone.clone();
                    let count_inner = count_clone.clone();
                    let sidebar_inner = sidebar_clone.clone();
                    alert::show_confirm(
                        &manager_clone,
                        "Terminate Session?",
                        "Are you sure you want to terminate this session?",
                        "Terminate",
                        true,
                        move |confirmed| {
                            if confirmed {
                                // Terminate session in state manager
                                if let Ok(mut state_mut) = state_inner.try_borrow_mut() {
                                    let _ = state_mut.terminate_session(id);
                                }

                                // Decrement session count
                                if let Some(info) = notebook_inner.get_session_info(id) {
                                    sidebar_inner.decrement_session_count(
                                        &info.connection_id.to_string(),
                                        false,
                                    );
                                }

                                // Close the tab
                                notebook_inner.close_tab(id);
                                // Refresh the list
                                populate_sessions_list(
                                    &state_inner,
                                    &notebook_inner,
                                    &list_inner,
                                    &count_inner,
                                );
                            }
                        },
                    );
                }
            }
        }
    });

    manager_window.present();
}

/// Populates the sessions list
pub fn populate_sessions_list(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    list: &gtk4::ListBox,
    count_label: &Label,
) {
    // Clear existing rows
    while let Some(row) = list.row_at_index(0) {
        list.remove(&row);
    }

    // Get sessions from notebook (UI sessions)
    let session_ids = notebook.session_ids();
    let session_count = session_ids.len();

    // Also get active sessions from state manager for additional info
    let state_ref = state.borrow();
    let active_sessions = state_ref.active_sessions();
    let state_session_count = active_sessions.len();
    drop(state_ref);

    count_label.set_text(&format!(
        "{session_count} UI session(s), {state_session_count} tracked session(s)"
    ));

    for session_id in session_ids {
        if let Some(info) = notebook.get_session_info(session_id) {
            let row = gtk4::ListBoxRow::new();
            row.set_widget_name(&format!("session-{session_id}"));

            let hbox = gtk4::Box::new(Orientation::Horizontal, 12);
            hbox.set_margin_top(8);
            hbox.set_margin_bottom(8);
            hbox.set_margin_start(12);
            hbox.set_margin_end(12);

            // Protocol icon
            let icon_name = match info.protocol.as_str() {
                "ssh" | "local" => "utilities-terminal-symbolic",
                "rdp" => "computer-symbolic",
                "vnc" => "video-display-symbolic",
                _ => "network-server-symbolic",
            };
            let icon = gtk4::Image::from_icon_name(icon_name);
            hbox.append(&icon);

            let vbox = gtk4::Box::new(Orientation::Vertical, 4);
            vbox.set_hexpand(true);

            let name_label = Label::builder()
                .label(&info.name)
                .halign(gtk4::Align::Start)
                .css_classes(["heading"])
                .build();
            vbox.append(&name_label);

            // Get connection info if available
            let state_ref = state.borrow();
            let connection_info = if info.connection_id == Uuid::nil() {
                Some(info.protocol.to_uppercase().clone())
            } else {
                state_ref
                    .get_connection(info.connection_id)
                    .map(|c| format!("{} ({})", c.host, info.protocol.to_uppercase()))
            };
            drop(state_ref);

            if let Some(conn_info) = connection_info {
                let info_label = Label::builder()
                    .label(&conn_info)
                    .halign(gtk4::Align::Start)
                    .css_classes(["dim-label"])
                    .build();
                vbox.append(&info_label);
            }

            // Session type indicator
            let type_label = Label::builder()
                .label(info.protocol.to_uppercase())
                .halign(gtk4::Align::Start)
                .css_classes(["dim-label"])
                .build();
            vbox.append(&type_label);

            hbox.append(&vbox);
            row.set_child(Some(&hbox));
            list.append(&row);
        }
    }
}

/// Shows a dialog to send text to a specific session
pub fn show_send_text_dialog(parent: &gtk4::Window, notebook: &SharedNotebook, session_id: Uuid) {
    let dialog = gtk4::Window::builder()
        .title("Send Text to Session")
        .transient_for(parent)
        .modal(true)
        .default_width(400)
        .build();

    let header = HeaderBar::new();
    let cancel_btn = Button::builder().label("Cancel").build();
    let send_btn = Button::builder()
        .label("Send")
        .css_classes(["suggested-action"])
        .build();
    header.pack_start(&cancel_btn);
    header.pack_end(&send_btn);
    dialog.set_titlebar(Some(&header));

    let content = gtk4::Box::new(Orientation::Vertical, 8);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    let label = Label::builder()
        .label("Enter text to send to the session:")
        .halign(gtk4::Align::Start)
        .build();
    content.append(&label);

    let entry = gtk4::Entry::builder()
        .placeholder_text("Text to send...")
        .hexpand(true)
        .build();
    content.append(&entry);

    let newline_check = gtk4::CheckButton::builder()
        .label("Append newline (press Enter)")
        .active(true)
        .build();
    content.append(&newline_check);

    dialog.set_child(Some(&content));

    // Connect cancel button
    let dialog_clone = dialog.clone();
    cancel_btn.connect_clicked(move |_| {
        dialog_clone.close();
    });

    // Connect send button
    let notebook_clone = notebook.clone();
    let dialog_clone = dialog.clone();
    let entry_clone = entry.clone();
    let newline_clone = newline_check.clone();
    send_btn.connect_clicked(move |_| {
        let text = entry_clone.text().to_string();
        if !text.is_empty() {
            let text_to_send = if newline_clone.is_active() {
                format!("{text}\n")
            } else {
                text
            };
            notebook_clone.send_text_to_session(session_id, &text_to_send);
        }
        dialog_clone.close();
    });

    // Also send on Enter key
    let notebook_clone = notebook.clone();
    let dialog_clone = dialog.clone();
    let newline_clone = newline_check;
    entry.connect_activate(move |entry| {
        let text = entry.text().to_string();
        if !text.is_empty() {
            let text_to_send = if newline_clone.is_active() {
                format!("{text}\n")
            } else {
                text
            };
            notebook_clone.send_text_to_session(session_id, &text_to_send);
        }
        dialog_clone.close();
    });

    dialog.present();
}
