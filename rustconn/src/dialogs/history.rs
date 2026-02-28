//! Connection history dialog
//!
//! This module provides a dialog for viewing, searching, and managing
//! connection history with per-entry deletion.

use crate::i18n::i18n;
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, Label, ListBox, ListBoxRow, Orientation, ScrolledWindow};
use libadwaita as adw;
use rustconn_core::models::ConnectionHistoryEntry;
use std::cell::RefCell;
use std::rc::Rc;

/// Connection history dialog
pub struct HistoryDialog {
    dialog: adw::Dialog,
    list_box: ListBox,
    entries: Rc<RefCell<Vec<ConnectionHistoryEntry>>>,
    on_connect: Rc<RefCell<Option<Box<dyn Fn(&ConnectionHistoryEntry) + 'static>>>>,
    on_delete_entry: Rc<RefCell<Option<Box<dyn Fn(&ConnectionHistoryEntry) + 'static>>>>,
    on_clear_all: Rc<RefCell<Option<Box<dyn Fn() + 'static>>>>,
    parent: Option<gtk4::Widget>,
}

impl HistoryDialog {
    /// Creates a new history dialog
    #[must_use]
    pub fn new(parent: Option<&impl IsA<gtk4::Window>>) -> Self {
        let dialog = adw::Dialog::builder()
            .title(i18n("Connection History"))
            .content_width(500)
            .content_height(400)
            .build();

        // Header bar (GNOME HIG)
        let (header, close_btn, connect_btn) = super::widgets::dialog_header("Close", "Connect");
        connect_btn.set_sensitive(false);

        // Close button handler
        let dialog_clone = dialog.clone();
        close_btn.connect_clicked(move |_| {
            dialog_clone.close();
        });

        // Main content
        let content = GtkBox::new(Orientation::Vertical, 0);

        // Search entry
        let search_entry = gtk4::SearchEntry::builder()
            .placeholder_text(i18n("Search history…"))
            .hexpand(true)
            .margin_top(12)
            .margin_start(12)
            .margin_end(12)
            .margin_bottom(6)
            .build();
        search_entry.set_tooltip_text(Some(&i18n("Filter history by name, host, or protocol")));
        content.append(&search_entry);

        // History list in scrolled window with clamp
        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .build();

        let clamp = adw::Clamp::builder()
            .maximum_size(600)
            .tightening_threshold(400)
            .build();

        let list_box = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .css_classes(["boxed-list"])
            .margin_top(6)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();

        list_box.set_placeholder(Some(
            &Label::builder()
                .label(i18n("No connection history"))
                .css_classes(["dim-label"])
                .margin_top(24)
                .margin_bottom(24)
                .build(),
        ));

        clamp.set_child(Some(&list_box));
        scrolled.set_child(Some(&clamp));
        content.append(&scrolled);

        // Clear history button at bottom
        let bottom_bar = GtkBox::new(Orientation::Horizontal, 0);
        bottom_bar.set_margin_top(6);
        bottom_bar.set_margin_bottom(12);
        bottom_bar.set_margin_start(12);
        bottom_bar.set_margin_end(12);

        let clear_btn = Button::builder()
            .label(i18n("Clear History"))
            .css_classes(["destructive-action"])
            .build();
        bottom_bar.append(&clear_btn);
        content.append(&bottom_bar);

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&content));
        dialog.set_child(Some(&toolbar_view));

        let stored_parent: Option<gtk4::Widget> =
            parent.map(|p| p.clone().upcast::<gtk4::Window>().upcast::<gtk4::Widget>());

        let hist = Self {
            dialog: dialog.clone(),
            list_box: list_box.clone(),
            entries: Rc::new(RefCell::new(Vec::new())),
            on_connect: Rc::new(RefCell::new(None)),
            on_delete_entry: Rc::new(RefCell::new(None)),
            on_clear_all: Rc::new(RefCell::new(None)),
            parent: stored_parent,
        };

        // Selection → enable Connect button
        let connect_btn_clone = connect_btn.clone();
        list_box.connect_row_selected(move |_, row| {
            connect_btn_clone.set_sensitive(row.is_some());
        });

        // Connect button
        let entries_clone = hist.entries.clone();
        let list_box_clone = list_box.clone();
        let on_connect = hist.on_connect.clone();
        let dialog_clone = dialog.clone();
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
                        dialog_clone.close();
                    }
                }
            }
        });

        // Search filtering
        let list_box_search = hist.list_box.clone();
        let entries_search = hist.entries.clone();
        search_entry.connect_search_changed(move |entry| {
            let query = entry.text().to_string().to_lowercase();
            let entries_ref = entries_search.borrow();
            #[allow(clippy::cast_possible_wrap)]
            for (idx, e) in entries_ref.iter().enumerate() {
                if let Some(row) = list_box_search.row_at_index(idx as i32) {
                    let visible = query.is_empty()
                        || e.connection_name.to_lowercase().contains(&query)
                        || e.host.to_lowercase().contains(&query)
                        || e.protocol.to_lowercase().contains(&query)
                        || e.username
                            .as_deref()
                            .is_some_and(|u| u.to_lowercase().contains(&query));
                    row.set_visible(visible);
                }
            }
        });

        // Clear history button — show confirmation dialog before destructive action
        let entries_clear = hist.entries.clone();
        let list_box_clear = list_box;
        let on_clear_all = hist.on_clear_all.clone();
        let dialog_weak = dialog.downgrade();
        clear_btn.connect_clicked(move |_| {
            let alert = adw::AlertDialog::builder()
                .heading(i18n("Clear History?"))
                .body(i18n(
                    "All connection history entries will be permanently removed.",
                ))
                .build();
            alert.add_response("cancel", &i18n("Cancel"));
            alert.add_response("clear", &i18n("Clear"));
            alert.set_response_appearance("clear", adw::ResponseAppearance::Destructive);
            alert.set_default_response(Some("cancel"));
            alert.set_close_response("cancel");

            let entries_ref = entries_clear.clone();
            let list_ref = list_box_clear.clone();
            let on_clear = on_clear_all.clone();
            alert.connect_response(None, move |_, response| {
                if response == "clear" {
                    entries_ref.borrow_mut().clear();
                    while let Some(row) = list_ref.row_at_index(0) {
                        list_ref.remove(&row);
                    }
                    if let Some(ref callback) = *on_clear.borrow() {
                        callback();
                    }
                }
            });

            if let Some(dlg) = dialog_weak.upgrade() {
                alert.present(Some(&dlg));
            }
        });

        hist
    }

    /// Sets the history entries to display
    pub fn set_entries(&self, mut entries: Vec<ConnectionHistoryEntry>) {
        // Sort by started_at descending (newest first)
        entries.sort_by(|a, b| b.started_at.cmp(&a.started_at));

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

    /// Creates a list row for a history entry with a delete button
    fn create_history_row(&self, entry: &ConnectionHistoryEntry) -> ListBoxRow {
        let row = ListBoxRow::new();

        let content = GtkBox::new(Orientation::Horizontal, 12);
        content.set_margin_top(8);
        content.set_margin_bottom(8);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // Status indicator
        let status_icon = if entry.successful {
            "object-select-symbolic"
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
            .ellipsize(gtk4::pango::EllipsizeMode::End)
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
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .css_classes(["dim-label", "caption"])
            .build();
        info_box.append(&details_label);

        content.append(&info_box);

        // Timestamp
        let time_str = entry.started_at.format("%Y-%m-%d %H:%M").to_string();
        let time_label = Label::builder()
            .label(&time_str)
            .halign(gtk4::Align::End)
            .valign(gtk4::Align::Center)
            .css_classes(["dim-label", "caption"])
            .build();
        content.append(&time_label);

        // Delete button for individual entry
        let delete_btn = Button::builder()
            .icon_name("edit-delete-symbolic")
            .css_classes(["flat", "circular"])
            .valign(gtk4::Align::Center)
            .tooltip_text(i18n("Remove from history"))
            .build();

        let entries_ref = self.entries.clone();
        let list_box_ref = self.list_box.clone();
        let on_delete = self.on_delete_entry.clone();
        let row_weak = row.downgrade();
        delete_btn.connect_clicked(move |_| {
            if let Some(r) = row_weak.upgrade() {
                let index = r.index();
                if index >= 0 {
                    #[allow(clippy::cast_sign_loss)]
                    let idx = index as usize;
                    let mut entries = entries_ref.borrow_mut();
                    if idx < entries.len() {
                        let removed = entries.remove(idx);
                        list_box_ref.remove(&r);
                        drop(entries);
                        if let Some(ref callback) = *on_delete.borrow() {
                            callback(&removed);
                        }
                    }
                }
            }
        });
        content.append(&delete_btn);

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

    /// Connects a callback for when a single history entry is deleted
    pub fn connect_on_delete_entry<F>(&self, callback: F)
    where
        F: Fn(&ConnectionHistoryEntry) + 'static,
    {
        *self.on_delete_entry.borrow_mut() = Some(Box::new(callback));
    }

    /// Connects a callback for when all history is cleared
    pub fn connect_on_clear_all<F>(&self, callback: F)
    where
        F: Fn() + 'static,
    {
        *self.on_clear_all.borrow_mut() = Some(Box::new(callback));
    }

    /// Shows the dialog
    pub fn present(&self) {
        self.dialog
            .present(self.parent.as_ref().map(|w| w as &gtk4::Widget));
    }
}
