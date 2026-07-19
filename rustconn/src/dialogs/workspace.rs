//! Workspace profiles dialog
//!
//! Provides UI for managing workspace profiles — saved sets of connections
//! that can be opened together to restore a working context.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Button, Label, ListBox, Orientation, SelectionMode};
use libadwaita as adw;
use uuid::Uuid;

use crate::i18n::{i18n, i18n_f};

/// Callback type for workspace operations
pub type WorkspaceCallback = Box<dyn Fn(Uuid)>;

/// Callback type for workspace rename
pub type WorkspaceRenameCallback = Box<dyn Fn(Uuid, String)>;

/// Dialog for managing workspace profiles
pub struct WorkspaceManagerDialog {
    dialog: adw::Dialog,
    list_box: ListBox,
    open_button: Button,
    rename_button: Button,
    delete_button: Button,
    /// Currently selected workspace ID
    selected_id: Rc<RefCell<Option<Uuid>>>,
    /// (id, name, entry_count) tuples for display
    items: Rc<RefCell<Vec<(Uuid, String, usize)>>>,
    /// Provider to fetch current workspace list
    provider: Rc<RefCell<Option<Box<dyn Fn() -> Vec<(Uuid, String, usize)>>>>>,
    /// Callback when "Open" is clicked
    on_open: Rc<RefCell<Option<WorkspaceCallback>>>,
    /// Callback when "Delete" is clicked
    on_delete: Rc<RefCell<Option<WorkspaceCallback>>>,
    /// Callback when "Rename" is confirmed
    on_rename: Rc<RefCell<Option<WorkspaceRenameCallback>>>,
    /// Callback when "Save current" is clicked
    on_save_current: Rc<RefCell<Option<Box<dyn Fn()>>>>,
}

impl WorkspaceManagerDialog {
    /// Creates a new workspace manager dialog
    #[must_use]
    pub fn new(_parent: Option<&gtk4::Widget>) -> Self {
        let dialog = adw::Dialog::builder()
            .title(i18n("Workspace Profiles"))
            .content_width(400)
            .content_height(360)
            .build();

        // Header bar with close button
        let header = adw::HeaderBar::new();

        // Save current button in header
        let save_current_btn = Button::builder()
            .label(i18n("Save Current"))
            .css_classes(["suggested-action"])
            .build();
        save_current_btn
            .set_tooltip_text(Some(&i18n("Save currently open sessions as a workspace")));
        save_current_btn.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Save currently open sessions as a workspace",
        ))]);
        header.pack_start(&save_current_btn);

        // Content
        let content = GtkBox::new(Orientation::Vertical, 0);

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&content));
        dialog.set_child(Some(&toolbar_view));

        // Clamp for consistent width
        let clamp = adw::Clamp::new();
        clamp.set_maximum_size(600);
        clamp.set_margin_top(12);
        clamp.set_margin_bottom(12);
        clamp.set_margin_start(12);
        clamp.set_margin_end(12);
        content.append(&clamp);

        let inner = GtkBox::new(Orientation::Vertical, 12);
        clamp.set_child(Some(&inner));

        // Description label
        let desc = Label::new(Some(&i18n(
            "Open a saved workspace to restore all its connections at once.",
        )));
        desc.set_wrap(true);
        desc.set_xalign(0.0);
        desc.add_css_class("dim-label");
        inner.append(&desc);

        // List box in a frame
        let frame = gtk4::Frame::new(None);
        let scrolled = gtk4::ScrolledWindow::builder()
            .vexpand(true)
            .min_content_height(180)
            .build();
        let list_box = ListBox::builder()
            .selection_mode(SelectionMode::Single)
            .build();
        list_box.add_css_class("boxed-list");
        scrolled.set_child(Some(&list_box));
        frame.set_child(Some(&scrolled));
        inner.append(&frame);

        // Action buttons
        let button_box = GtkBox::new(Orientation::Horizontal, 6);
        button_box.set_halign(Align::End);

        let delete_button = Button::with_label(&i18n("Delete"));
        delete_button.add_css_class("destructive-action");
        delete_button.set_sensitive(false);
        delete_button.set_tooltip_text(Some(&i18n("Delete selected workspace")));
        delete_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Delete selected workspace",
        ))]);

        let rename_button = Button::with_label(&i18n("Rename"));
        rename_button.set_sensitive(false);
        rename_button.set_tooltip_text(Some(&i18n("Rename selected workspace")));
        rename_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Rename selected workspace",
        ))]);

        let open_button = Button::with_label(&i18n("Open"));
        open_button.add_css_class("suggested-action");
        open_button.set_sensitive(false);
        open_button.set_tooltip_text(Some(&i18n("Open selected workspace")));
        open_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Open selected workspace",
        ))]);

        button_box.append(&delete_button);
        button_box.append(&rename_button);
        button_box.append(&open_button);
        inner.append(&button_box);

        let selected_id: Rc<RefCell<Option<Uuid>>> = Rc::new(RefCell::new(None));
        let items: Rc<RefCell<Vec<(Uuid, String, usize)>>> = Rc::new(RefCell::new(Vec::new()));

        // Selection changed → update button sensitivity
        let sel_id = selected_id.clone();
        let open_btn_clone = open_button.clone();
        let rename_btn_clone = rename_button.clone();
        let del_btn_clone = delete_button.clone();
        let items_clone = items.clone();
        list_box.connect_row_selected(move |_, row| {
            // GTK emits this during list teardown/removal; use try_borrow so a
            // re-entrant borrow never turns into a non-unwinding abort.
            if let Some(row) = row {
                let idx = row.index() as usize;
                let id = items_clone.borrow().get(idx).map(|(id, _, _)| *id);
                if let Some(id) = id
                    && let Ok(mut sel) = sel_id.try_borrow_mut()
                {
                    *sel = Some(id);
                    open_btn_clone.set_sensitive(true);
                    rename_btn_clone.set_sensitive(true);
                    del_btn_clone.set_sensitive(true);
                }
            } else if let Ok(mut sel) = sel_id.try_borrow_mut() {
                *sel = None;
                open_btn_clone.set_sensitive(false);
                rename_btn_clone.set_sensitive(false);
                del_btn_clone.set_sensitive(false);
            }
        });

        let on_open: Rc<RefCell<Option<WorkspaceCallback>>> = Rc::new(RefCell::new(None));
        let on_delete: Rc<RefCell<Option<WorkspaceCallback>>> = Rc::new(RefCell::new(None));
        let on_rename: Rc<RefCell<Option<WorkspaceRenameCallback>>> = Rc::new(RefCell::new(None));
        let on_save_current: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));

        // Open button
        let sel_for_open = selected_id.clone();
        let cb_open = on_open.clone();
        let dialog_for_open = dialog.clone();
        open_button.connect_clicked(move |_| {
            // Copy the id and release the borrow before invoking the callback
            // or closing the dialog — closing tears down the list box, which
            // emits `row-selected` and re-borrows `selected_id`.
            let selected = *sel_for_open.borrow();
            if let Some(id) = selected {
                if let Some(ref cb) = *cb_open.borrow() {
                    cb(id);
                }
                dialog_for_open.close();
            }
        });

        // Activating a row (double-click / Enter) triggers the primary action
        // (Open), matching the activatable rows in the list.
        let items_for_activate = items.clone();
        let cb_open_activate = on_open.clone();
        let dialog_for_activate = dialog.clone();
        list_box.connect_row_activated(move |_, row| {
            let idx = row.index() as usize;
            let id = items_for_activate.borrow().get(idx).map(|(id, _, _)| *id);
            if let Some(id) = id {
                if let Some(ref cb) = *cb_open_activate.borrow() {
                    cb(id);
                }
                dialog_for_activate.close();
            }
        });

        // Rename button
        let sel_for_rename = selected_id.clone();
        let cb_rename = on_rename.clone();
        let items_for_rename = items.clone();
        let dialog_for_rename = dialog.clone();
        rename_button.connect_clicked(move |_| {
            let id = match *sel_for_rename.borrow() {
                Some(id) => id,
                None => return,
            };
            let current_name = items_for_rename
                .borrow()
                .iter()
                .find(|(ws_id, _, _)| *ws_id == id)
                .map(|(_, name, _)| name.clone())
                .unwrap_or_default();

            let alert = adw::AlertDialog::new(
                Some(&i18n("Rename Workspace")),
                Some(&i18n("Enter a new name:")),
            );
            alert.add_response("cancel", &i18n("Cancel"));
            alert.add_response("rename", &i18n("Rename"));
            alert.set_response_appearance("rename", adw::ResponseAppearance::Suggested);
            alert.set_default_response(Some("rename"));
            alert.set_close_response("cancel");

            let entry = gtk4::Entry::builder()
                .text(&current_name)
                .activates_default(true)
                .build();
            alert.set_extra_child(Some(&entry));

            let cb_clone = cb_rename.clone();
            alert.connect_response(None, move |_, response| {
                if response != "rename" {
                    return;
                }
                let new_name = entry.text().trim().to_string();
                if new_name.is_empty() || new_name == current_name {
                    return;
                }
                if let Some(ref cb) = *cb_clone.borrow() {
                    cb(id, new_name);
                }
            });

            alert.present(Some(&dialog_for_rename));
        });

        // Delete button — confirm first (GNOME HIG: destructive actions),
        // matching the tunnel/cluster managers.
        let sel_for_del = selected_id.clone();
        let cb_del = on_delete.clone();
        let items_for_del = items.clone();
        let dialog_for_del = dialog.clone();
        delete_button.connect_clicked(move |_| {
            // Copy the id and release the borrow before showing the dialog.
            let Some(id) = *sel_for_del.borrow() else {
                return;
            };
            let name = items_for_del
                .borrow()
                .iter()
                .find(|(ws_id, _, _)| *ws_id == id)
                .map(|(_, name, _)| name.clone())
                .unwrap_or_default();

            let confirm = adw::AlertDialog::new(
                Some(&i18n("Delete Workspace?")),
                Some(&i18n_f(
                    "Workspace \"{}\" will be permanently removed.",
                    &[&name],
                )),
            );
            confirm.add_response("cancel", &i18n("Cancel"));
            confirm.add_response("delete", &i18n("Delete"));
            confirm.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
            confirm.set_default_response(Some("cancel"));
            confirm.set_close_response("cancel");

            let cb_clone = cb_del.clone();
            confirm.connect_response(None, move |_, response| {
                if response != "delete" {
                    return;
                }
                if let Some(ref cb) = *cb_clone.borrow() {
                    cb(id);
                }
            });

            confirm.present(Some(&dialog_for_del));
        });

        // Save current
        let cb_save = on_save_current.clone();
        save_current_btn.connect_clicked(move |_| {
            if let Some(ref cb) = *cb_save.borrow() {
                cb();
            }
        });

        Self {
            dialog,
            list_box,
            open_button,
            rename_button,
            delete_button,
            selected_id,
            items,
            provider: Rc::new(RefCell::new(None)),
            on_open,
            on_delete,
            on_rename,
            on_save_current,
        }
    }

    /// Sets the provider that fetches the current workspace list
    pub fn set_provider(&self, provider: impl Fn() -> Vec<(Uuid, String, usize)> + 'static) {
        *self.provider.borrow_mut() = Some(Box::new(provider));
    }

    /// Sets the callback for opening a workspace
    pub fn set_on_open(&self, cb: impl Fn(Uuid) + 'static) {
        *self.on_open.borrow_mut() = Some(Box::new(cb));
    }

    /// Sets the callback for deleting a workspace
    pub fn set_on_delete(&self, cb: impl Fn(Uuid) + 'static) {
        *self.on_delete.borrow_mut() = Some(Box::new(cb));
    }

    /// Sets the callback for renaming a workspace
    pub fn set_on_rename(&self, cb: impl Fn(Uuid, String) + 'static) {
        *self.on_rename.borrow_mut() = Some(Box::new(cb));
    }

    /// Sets the callback for "Save current" button
    pub fn set_on_save_current(&self, cb: impl Fn() + 'static) {
        *self.on_save_current.borrow_mut() = Some(Box::new(cb));
    }

    /// Refreshes the list from the provider
    pub fn refresh_list(&self) {
        // Clear list
        while let Some(row) = self.list_box.row_at_index(0) {
            self.list_box.remove(&row);
        }
        *self.selected_id.borrow_mut() = None;
        self.open_button.set_sensitive(false);
        self.rename_button.set_sensitive(false);
        self.delete_button.set_sensitive(false);

        // Fetch items
        let new_items = if let Some(ref provider) = *self.provider.borrow() {
            provider()
        } else {
            Vec::new()
        };

        if new_items.is_empty() {
            // Empty state
            let row = adw::ActionRow::builder()
                .title(i18n("No workspace profiles saved"))
                .subtitle(i18n("Save your current sessions to create one"))
                .sensitive(false)
                .build();
            self.list_box.append(&row);
        } else {
            for (_, name, count) in &new_items {
                let subtitle = i18n_f("{} connections", &[&count.to_string()]);
                let row = adw::ActionRow::builder()
                    .title(name)
                    .subtitle(subtitle)
                    .activatable(true)
                    .build();
                row.add_prefix(&gtk4::Image::from_icon_name("view-grid-symbolic"));
                self.list_box.append(&row);
            }
        }

        *self.items.borrow_mut() = new_items;
    }

    /// Shows the dialog
    pub fn show(&self, parent: &impl IsA<gtk4::Widget>) {
        self.dialog.present(Some(parent));
    }
}
