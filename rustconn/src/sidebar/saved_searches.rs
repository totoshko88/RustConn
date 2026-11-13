//! "Saved searches" section of the sidebar Filter popover.
//!
//! A saved search is a name plus the exact text of the search entry (protocol
//! filters are part of that text). Applying one puts the text back into the
//! entry, so the list is identical to what typing it would give. The model and
//! its rules live in `rustconn_core::search::saved`.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Button, Label, Orientation, Popover, SearchEntry};
use libadwaita as adw;
use rustconn_core::search::saved::{
    SavedSearch, add_saved_search, remove_saved_search, rename_saved_search,
};
use uuid::Uuid;

use crate::i18n::{i18n, i18n_f};

type ChangeHandler = Box<dyn Fn(&[SavedSearch])>;

/// The section widget plus the list it shows.
#[derive(Clone)]
pub struct SavedSearchesSection {
    widget: GtkBox,
    items: Rc<RefCell<Vec<SavedSearch>>>,
    rows: GtkBox,
    on_change: Rc<RefCell<Option<ChangeHandler>>>,
    entry: SearchEntry,
    popover: Popover,
}

impl SavedSearchesSection {
    /// Builds the section; `popover` is the Filter popover that will hold it.
    pub fn new(entry: &SearchEntry, popover: &Popover) -> Self {
        let widget = GtkBox::new(Orientation::Vertical, 4);
        let heading = Label::builder()
            .label(i18n("Saved searches"))
            .halign(Align::Start)
            .build();
        heading.add_css_class("heading");
        widget.append(&heading);

        let rows = GtkBox::new(Orientation::Vertical, 2);
        widget.append(&rows);

        let save = Button::with_label(&i18n("Save current search…"));
        save.add_css_class("flat");
        widget.append(&save);

        let section = Self {
            widget,
            items: Rc::new(RefCell::new(Vec::new())),
            rows,
            on_change: Rc::new(RefCell::new(None)),
            entry: entry.clone(),
            popover: popover.clone(),
        };

        // Nothing to save while the search entry is empty.
        let save_weak = save.downgrade();
        let entry_for_state = entry.clone();
        popover.connect_visible_notify(move |p| {
            if p.is_visible()
                && let Some(save) = save_weak.upgrade()
            {
                save.set_sensitive(!entry_for_state.text().trim().is_empty());
            }
        });

        let s = section.clone();
        save.connect_clicked(move |_| s.prompt_save());

        section.rebuild();
        section
    }

    pub fn widget(&self) -> &GtkBox {
        &self.widget
    }

    /// Replaces the list (startup load); does not fire the change handler.
    pub fn set_items(&self, items: &[SavedSearch]) {
        *self.items.borrow_mut() = items.to_vec();
        self.rebuild();
    }

    /// Called with the full list after every add, rename or delete.
    pub fn connect_changed<F: Fn(&[SavedSearch]) + 'static>(&self, f: F) {
        *self.on_change.borrow_mut() = Some(Box::new(f));
    }

    fn changed(&self) {
        self.rebuild();
        if let Some(f) = self.on_change.borrow().as_ref() {
            f(&self.items.borrow());
        }
    }

    fn rebuild(&self) {
        while let Some(child) = self.rows.first_child() {
            self.rows.remove(&child);
        }
        let items = self.items.borrow().clone();
        if items.is_empty() {
            let empty = Label::builder()
                .label(i18n("No saved searches yet"))
                .halign(Align::Start)
                .build();
            empty.add_css_class("dim-label");
            self.rows.append(&empty);
            return;
        }
        for item in items {
            self.rows.append(&self.build_row(&item));
        }
    }

    fn build_row(&self, item: &SavedSearch) -> GtkBox {
        let row = GtkBox::new(Orientation::Horizontal, 2);

        let apply = Button::new();
        apply.add_css_class("flat");
        apply.set_hexpand(true);
        let label = Label::builder()
            .label(&item.name)
            .halign(Align::Start)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .max_width_chars(28)
            .build();
        apply.set_child(Some(&label));
        apply.set_tooltip_text(Some(&item.query));
        apply.update_property(&[gtk4::accessible::Property::Label(&item.name)]);
        let (entry, popover, query) =
            (self.entry.clone(), self.popover.clone(), item.query.clone());
        apply.connect_clicked(move |_| {
            entry.set_text(&query);
            entry.set_position(-1);
            popover.popdown();
            entry.grab_focus();
        });
        row.append(&apply);

        let rename = Button::from_icon_name("document-edit-symbolic");
        rename.add_css_class("flat");
        let rename_label = i18n_f("Rename saved search {}", &[&item.name]);
        rename.set_tooltip_text(Some(&rename_label));
        rename.update_property(&[gtk4::accessible::Property::Label(&rename_label)]);
        let (s, id, name) = (self.clone(), item.id, item.name.clone());
        rename.connect_clicked(move |_| s.prompt_rename(id, &name));
        row.append(&rename);

        let delete = Button::from_icon_name("user-trash-symbolic");
        delete.add_css_class("flat");
        let delete_label = i18n_f("Delete saved search {}", &[&item.name]);
        delete.set_tooltip_text(Some(&delete_label));
        delete.update_property(&[gtk4::accessible::Property::Label(&delete_label)]);
        let (s, id, name) = (self.clone(), item.id, item.name.clone());
        delete.connect_clicked(move |_| s.confirm_delete(id, &name));
        row.append(&delete);

        row
    }

    /// Asks for a name; `valid` decides whether the confirm button is enabled.
    fn prompt_name<V, D>(&self, heading: &str, confirm: &str, initial: &str, valid: V, done: D)
    where
        V: Fn(&str) -> bool + 'static,
        D: Fn(String) + 'static,
    {
        let Some(root) = self.popover.root() else {
            return;
        };
        self.popover.popdown();

        let dialog = adw::AlertDialog::new(Some(heading), None);
        dialog.add_response("cancel", &i18n("Cancel"));
        dialog.add_response("ok", confirm);
        dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("ok"));
        dialog.set_close_response("cancel");

        let name = gtk4::Entry::builder()
            .text(initial)
            .activates_default(true)
            .max_length(rustconn_core::search::saved::MAX_SAVED_SEARCH_NAME_CHARS as i32)
            .build();
        name.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Saved search name",
        ))]);
        dialog.set_extra_child(Some(&name));
        dialog.set_response_enabled("ok", valid(initial));

        let dialog_weak = dialog.downgrade();
        name.connect_changed(move |e| {
            if let Some(d) = dialog_weak.upgrade() {
                d.set_response_enabled("ok", valid(e.text().as_str()));
            }
        });
        let name_for_done = name.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "ok" {
                done(name_for_done.text().to_string());
            }
        });
        dialog.present(Some(&root));
        name.grab_focus();
    }

    fn prompt_save(&self) {
        let query = self.entry.text().to_string();
        if query.trim().is_empty() {
            return;
        }
        let (s_valid, q_valid) = (self.items.clone(), query.clone());
        let s = self.clone();
        self.prompt_name(
            &i18n("Save Search"),
            &i18n("Save"),
            "",
            move |name| add_saved_search(&mut s_valid.borrow().clone(), name, &q_valid).is_ok(),
            move |name| {
                let result = add_saved_search(&mut s.items.borrow_mut(), &name, &query);
                if result.is_ok() {
                    s.changed();
                }
            },
        );
    }

    fn prompt_rename(&self, id: Uuid, current: &str) {
        let items_valid = self.items.clone();
        let s = self.clone();
        self.prompt_name(
            &i18n("Rename Saved Search"),
            &i18n("Rename"),
            current,
            move |name| {
                rename_saved_search(&mut items_valid.borrow_mut().clone(), id, name).is_ok()
            },
            move |name| {
                let result = rename_saved_search(&mut s.items.borrow_mut(), id, &name);
                if result.is_ok() {
                    s.changed();
                }
            },
        );
    }

    fn confirm_delete(&self, id: Uuid, name: &str) {
        let Some(root) = self.popover.root() else {
            return;
        };
        self.popover.popdown();
        let s = self.clone();
        crate::alert::show_confirm(
            &root,
            &i18n("Delete saved search?"),
            &i18n_f(
                "“{}” will be removed. Your connections are not affected.",
                &[name],
            ),
            &i18n("Delete"),
            true,
            move |confirmed| {
                if confirmed && remove_saved_search(&mut s.items.borrow_mut(), id) {
                    s.changed();
                }
            },
        );
    }
}
