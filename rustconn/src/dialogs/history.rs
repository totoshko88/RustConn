//! Connection history dialog
//!
//! This module provides a dialog for viewing connection history.

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, Label, ListBox, ListBoxRow, Orientation, ScrolledWindow};
use libadwaita as adw;
use rustconn_core::models::ConnectionHistoryEntry;
use std::cell::RefCell;
use std::rc::Rc;

/// Connection history dialog
pub struct HistoryDialog {
    window: adw::Window,
    list_box: ListBox,
    entries: Rc<RefCell<Vec<ConnectionHistoryEntry>>>,
    on_connect: Rc<RefCell<Option<Box<dyn Fn(&ConnectionHistoryEntry) + 'static>>>>,
}

impl HistoryDialog {
    /// Creates a new history dialog
    #[must_use]
    pub fn new(parent: Option<&impl IsA<gtk4::Window>>) -> Self {
        let window = adw::Window::builder()
            .title("Connection History")
            .default_width(500)
            .default_height(400)
            .modal(true)
            .build();

        if let Some(p) = parent {
            window.set_transient_for(Some(p));
        }

        // Header bar with Close/Connect buttons (GNOME HIG)
        let header = adw::HeaderBar::new();
        header.set_show_end_title_buttons(false);
        header.set_show_start_title_buttons(false);
        let close_btn = Button::builder().label("Close").build();
        let connect_btn = Button::builder()
            .label("Connect")
            .css_classes(["suggested-action"])
            .sensitive(false)
            .build();
        header.pack_start(&close_btn);
        header.pack_end(&connect_btn);

        // Close button handler
        let window_clone = window.clone();
        close_btn.connect_clicked(move |_| {
            window_clone.close();
        });

        // Main content
        let content = GtkBox::new(Orientation::Vertical, 0);

        // History list in scrolled window
        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .build();

        let list_box = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .css_classes(["boxed-list"])
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();

        list_box.set_placeholder(Some(
            &Label::builder()
                .label("No connection history")
                .css_classes(["dim-label"])
                .margin_top(24)
                .margin_bottom(24)
                .build(),
        ));

        scrolled.set_child(Some(&list_box));
        content.append(&scrolled);

        // Clear history button at bottom
        let bottom_bar = GtkBox::new(Orientation::Horizontal, 0);
        bottom_bar.set_margin_top(6);
        bottom_bar.set_margin_bottom(12);
        bottom_bar.set_margin_start(12);
        bottom_bar.set_margin_end(12);

        let clear_btn = Button::builder()
            .label("Clear History")
            .css_classes(["destructive-action"])
            .build();
        bottom_bar.append(&clear_btn);
        content.append(&bottom_bar);

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&content));
        window.set_content(Some(&toolbar_view));

        let dialog = Self {
            window: window.clone(),
            list_box: list_box.clone(),
            entries: Rc::new(RefCell::new(Vec::new())),
            on_connect: Rc::new(RefCell::new(None)),
        };

        let connect_btn_clone = connect_btn.clone();
        list_box.connect_row_selected(move |_, row| {
            connect_btn_clone.set_sensitive(row.is_some());
        });

        // Connect button
        let entries_clone = dialog.entries.clone();
        let list_box_clone = list_box.clone();
        let on_connect = dialog.on_connect.clone();
        let window_clone = window.clone();
        connect_btn.connect_clicked(move |_| {
            if let Some(row) = list_box_clone.selected_row() {
                let index = row.index();
                if index >= 0 {
                    let entries_ref = entries_clone.borrow();
                    #[allow(clippy::cast_sign_loss)]
                    if let Some(entry) = entries_ref.get(index as usize) {
                        if let Some(ref callback) = *on_connect.borrow() {
                            callback(entry);
                        }
                        window_clone.close();
                    }
                }
            }
        });

        // Clear history button
        let entries_clear = dialog.entries.clone();
        let list_box_clear = list_box;
        clear_btn.connect_clicked(move |_| {
            entries_clear.borrow_mut().clear();
            while let Some(row) = list_box_clear.row_at_index(0) {
                list_box_clear.remove(&row);
            }
        });

        dialog
    }

    /// Sets the history entries to display
    pub fn set_entries(&self, entries: Vec<ConnectionHistoryEntry>) {
        // Clear existing rows
        while let Some(row) = self.list_box.row_at_index(0) {
            self.list_box.remove(&row);
        }

        // Add rows
        for entry in &entries {
            let row = self.create_history_row(entry);
            self.list_box.append(&row);
        }

        *self.entries.borrow_mut() = entries;
    }

    /// Creates a list row for a history entry
    fn create_history_row(&self, entry: &ConnectionHistoryEntry) -> ListBoxRow {
        let row = ListBoxRow::new();

        let content = GtkBox::new(Orientation::Horizontal, 12);
        content.set_margin_top(8);
        content.set_margin_bottom(8);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // Status indicator
        let status_icon = if entry.successful {
            "emblem-ok-symbolic"
        } else {
            "dialog-error-symbolic"
        };
        let status = gtk4::Image::from_icon_name(status_icon);
        if entry.successful {
            status.add_css_class("success");
        } else {
            status.add_css_class("error");
        }
        content.append(&status);

        // Connection info
        let info_box = GtkBox::new(Orientation::Vertical, 2);
        info_box.set_hexpand(true);

        let name_label = Label::builder()
            .label(&entry.connection_name)
            .halign(gtk4::Align::Start)
            .css_classes(["heading"])
            .build();
        info_box.append(&name_label);

        let details = format!(
            "{} • {}:{} • {}",
            entry.protocol.to_uppercase(),
            entry.host,
            entry.port,
            entry.username.as_deref().unwrap_or("(no user)")
        );
        let details_label = Label::builder()
            .label(&details)
            .halign(gtk4::Align::Start)
            .css_classes(["dim-label", "caption"])
            .build();
        info_box.append(&details_label);

        content.append(&info_box);

        // Timestamp
        let time_str = entry.started_at.format("%Y-%m-%d %H:%M").to_string();
        let time_label = Label::builder()
            .label(&time_str)
            .halign(gtk4::Align::End)
            .css_classes(["dim-label", "caption"])
            .build();
        content.append(&time_label);

        row.set_child(Some(&content));
        row
    }

    /// Connects a callback for when user wants to connect to a history entry
    pub fn connect_on_connect<F>(&self, callback: F)
    where
        F: Fn(&ConnectionHistoryEntry) + 'static,
    {
        *self.on_connect.borrow_mut() = Some(Box::new(callback));
    }

    /// Shows the dialog
    pub fn present(&self) {
        self.window.present();
    }
}
