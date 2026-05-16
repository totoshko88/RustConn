//! Smart Folder create/edit dialog.
//!
//! Provides a libadwaita dialog for creating and editing smart folders with
//! filter criteria: protocol, host pattern, tags, and group.

use crate::dialogs::widgets::{DropdownRowBuilder, EntryRowBuilder};
use crate::i18n::i18n;
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, DropDown, Entry, Orientation};
use libadwaita as adw;
use rustconn_core::ProtocolType;
use rustconn_core::models::{ConnectionGroup, SmartFolder};
use std::cell::RefCell;
use std::rc::Rc;
use uuid::Uuid;

/// Callback type for smart folder dialog results.
pub type SmartFolderCallback = Rc<RefCell<Option<Box<dyn Fn(Option<SmartFolder>)>>>>;

/// All protocol variants in display order for the dropdown.
/// Index 0 is "Any" (no filter), then each protocol.
const PROTOCOL_VARIANTS: &[ProtocolType] = &[
    ProtocolType::Ssh,
    ProtocolType::Rdp,
    ProtocolType::Vnc,
    ProtocolType::Spice,
    ProtocolType::Telnet,
    ProtocolType::ZeroTrust,
    ProtocolType::Serial,
    ProtocolType::Sftp,
    ProtocolType::Kubernetes,
    ProtocolType::Mosh,
    ProtocolType::Web,
];

/// Dialog for creating or editing a Smart Folder.
pub struct SmartFolderDialog {
    window: adw::Window,
    name_entry: Entry,
    icon_entry: Entry,
    protocol_dropdown: DropDown,
    host_pattern_entry: Entry,
    tags_entry: Entry,
    group_dropdown: DropDown,
    save_btn: gtk4::Button,
    editing_id: Rc<RefCell<Option<Uuid>>>,
    on_save: SmartFolderCallback,
    /// Group IDs corresponding to dropdown indices (index 0 = None / "Any").
    group_ids: Rc<RefCell<Vec<Option<Uuid>>>>,
}

impl SmartFolderDialog {
    /// Creates a new Smart Folder dialog.
    ///
    /// If `existing` is `Some`, the dialog is pre-populated for editing.
    #[must_use]
    pub fn new(parent: Option<&gtk4::Window>, existing: Option<&SmartFolder>) -> Self {
        let is_edit = existing.is_some();
        let title = if is_edit {
            i18n("Edit Smart Folder")
        } else {
            i18n("New Smart Folder")
        };

        let window = adw::Window::builder()
            .title(title)
            .modal(true)
            .default_width(460)
            .build();

        if let Some(p) = parent {
            window.set_transient_for(Some(p));
        }
        window.set_size_request(320, -1);

        // Header bar with Save/Create icon button (GNOME HIG)
        let header = adw::HeaderBar::new();
        let save_btn = if is_edit {
            let btn = Button::from_icon_name("media-floppy-symbolic");
            btn.set_tooltip_text(Some(&i18n("Save")));
            btn.update_property(&[gtk4::accessible::Property::Label(&i18n("Save"))]);
            btn
        } else {
            let btn = Button::from_icon_name("list-add-symbolic");
            btn.set_tooltip_text(Some(&i18n("Create")));
            btn.update_property(&[gtk4::accessible::Property::Label(&i18n("Create"))]);
            btn
        };
        save_btn.add_css_class("suggested-action");
        header.pack_start(&save_btn);

        // Content with clamp (no scroll needed — content fits naturally)
        let clamp = adw::Clamp::builder()
            .maximum_size(600)
            .tightening_threshold(400)
            .build();

        let content = GtkBox::new(Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        clamp.set_child(Some(&content));

        let toast_overlay = adw::ToastOverlay::new();
        toast_overlay.set_child(Some(&clamp));

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&toast_overlay));
        window.set_content(Some(&toolbar_view));

        // === Name section ===
        let name_group = adw::PreferencesGroup::builder()
            .title(i18n("Smart Folder"))
            .build();

        let (name_row, name_entry) = EntryRowBuilder::new(&i18n("Name"))
            .placeholder("Prod SSH Servers")
            .build();
        name_row.set_activatable_widget(Some(&name_entry));
        name_group.add(&name_row);

        let (icon_row, icon_entry) = EntryRowBuilder::new(&i18n("Icon"))
            .placeholder("📁")
            .subtitle(&i18n("Optional emoji"))
            .build();
        icon_row.set_activatable_widget(Some(&icon_entry));
        name_group.add(&icon_row);

        content.append(&name_group);

        // === Filters section ===
        let filter_group = adw::PreferencesGroup::builder()
            .title(i18n("Filters"))
            .description(i18n("Connections must match ALL active filters"))
            .build();

        // Protocol dropdown: "Any" + all protocol types
        let any_label = i18n("Any");
        let protocol_labels: Vec<&str> = std::iter::once(any_label.as_str())
            .chain(PROTOCOL_VARIANTS.iter().map(|p| match p {
                ProtocolType::Ssh => "SSH",
                ProtocolType::Rdp => "RDP",
                ProtocolType::Vnc => "VNC",
                ProtocolType::Spice => "SPICE",
                ProtocolType::Telnet => "Telnet",
                ProtocolType::ZeroTrust => "Zero Trust",
                ProtocolType::Serial => "Serial",
                ProtocolType::Sftp => "SFTP",
                ProtocolType::Kubernetes => "Kubernetes",
                ProtocolType::Mosh => "MOSH",
                ProtocolType::Web => "Web",
            }))
            .collect();

        let (protocol_row, protocol_dropdown) =
            DropdownRowBuilder::new(&i18n("Filter by Protocol"))
                .items(&protocol_labels)
                .selected(0)
                .build();
        filter_group.add(&protocol_row);

        // Host pattern
        let (host_row, host_pattern_entry) = EntryRowBuilder::new(&i18n("Host Pattern"))
            .placeholder("*.example.com")
            .build();
        host_row.set_activatable_widget(Some(&host_pattern_entry));
        filter_group.add(&host_row);

        // Tags
        let (tags_row, tags_entry) = EntryRowBuilder::new(&i18n("Filter by Tags"))
            .subtitle(&i18n("Comma or semicolon separated"))
            .placeholder("web, production")
            .build();
        tags_row.set_activatable_widget(Some(&tags_entry));
        filter_group.add(&tags_row);

        // Group picker — starts with just "Any", populated later via set_groups()
        let any_group_label = i18n("Any");
        let (group_row, group_dropdown) = DropdownRowBuilder::new(&i18n("Filter by Group"))
            .items(&[any_group_label.as_str()])
            .selected(0)
            .build();
        filter_group.add(&group_row);

        content.append(&filter_group);

        let editing_id = Rc::new(RefCell::new(None));
        let on_save: SmartFolderCallback = Rc::new(RefCell::new(None));
        let group_ids: Rc<RefCell<Vec<Option<Uuid>>>> = Rc::new(RefCell::new(vec![None]));

        let dialog = Self {
            window,
            name_entry,
            icon_entry,
            protocol_dropdown,
            host_pattern_entry,
            tags_entry,
            group_dropdown,
            save_btn,
            editing_id,
            on_save,
            group_ids,
        };

        // Pre-populate if editing
        if let Some(folder) = existing {
            dialog.populate(folder);
        }

        dialog
    }

    /// Populates the dialog fields from an existing smart folder.
    fn populate(&self, folder: &SmartFolder) {
        *self.editing_id.borrow_mut() = Some(folder.id);
        self.name_entry.set_text(&folder.name);

        // Icon
        if let Some(ref icon) = folder.icon {
            self.icon_entry.set_text(icon);
        }

        // Protocol
        if let Some(proto) = &folder.filter_protocol
            && let Some(idx) = PROTOCOL_VARIANTS.iter().position(|p| p == proto)
        {
            // +1 because index 0 is "Any"
            self.protocol_dropdown.set_selected(idx as u32 + 1);
        }

        // Host pattern
        if let Some(ref pattern) = folder.filter_host_pattern {
            self.host_pattern_entry.set_text(pattern);
        }

        // Tags
        if !folder.filter_tags.is_empty() {
            self.tags_entry.set_text(&folder.filter_tags.join(", "));
        }

        // Group — will be selected when set_groups() is called
    }

    /// Sets the available groups for the group picker dropdown.
    pub fn set_groups(&self, groups: &[ConnectionGroup]) {
        let mut ids: Vec<Option<Uuid>> = vec![None];
        let mut labels: Vec<String> = vec![i18n("Any")];

        for g in groups {
            ids.push(Some(g.id));
            labels.push(g.name.clone());
        }

        let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let string_list = gtk4::StringList::new(&label_refs);
        self.group_dropdown.set_model(Some(&string_list));
        self.group_dropdown.set_selected(0);

        *self.group_ids.borrow_mut() = ids;
    }

    /// Selects the group in the dropdown matching the given group ID.
    pub fn set_selected_group(&self, group_id: Option<Uuid>) {
        if let Some(gid) = group_id {
            let ids = self.group_ids.borrow();
            if let Some(idx) = ids.iter().position(|id| *id == Some(gid)) {
                self.group_dropdown.set_selected(idx as u32);
            }
        }
    }

    /// Shows the dialog and calls `cb` with the resulting `SmartFolder` on save,
    /// or `None` if cancelled.
    pub fn run<F: Fn(Option<SmartFolder>) + 'static>(&self, cb: F) {
        *self.on_save.borrow_mut() = Some(Box::new(cb));

        let window = self.window.clone();
        let on_save = self.on_save.clone();
        let name_entry = self.name_entry.clone();
        let icon_entry = self.icon_entry.clone();
        let protocol_dropdown = self.protocol_dropdown.clone();
        let host_pattern_entry = self.host_pattern_entry.clone();
        let tags_entry = self.tags_entry.clone();
        let group_dropdown = self.group_dropdown.clone();
        let editing_id = self.editing_id.clone();
        let group_ids = self.group_ids.clone();

        self.save_btn.connect_clicked(move |_| {
            let name = name_entry.text().trim().to_string();
            if name.is_empty() {
                // Inline validation: highlight the Name field with error style
                name_entry.add_css_class("error");
                name_entry.grab_focus();
                return;
            }
            name_entry.remove_css_class("error");

            // Protocol filter
            let proto_idx = protocol_dropdown.selected();
            let filter_protocol = if proto_idx == 0 {
                None
            } else {
                PROTOCOL_VARIANTS.get(proto_idx as usize - 1).copied()
            };

            // Host pattern
            let host_text = host_pattern_entry.text().trim().to_string();
            let filter_host_pattern = if host_text.is_empty() {
                None
            } else {
                Some(host_text)
            };

            // Tags
            let tags_text = tags_entry.text();
            let filter_tags: Vec<String> = tags_text
                .split([',', ';'])
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            // Group
            let selected_group = group_dropdown.selected() as usize;
            let filter_group_id = group_ids.borrow().get(selected_group).copied().flatten();

            // Icon
            let icon_text = icon_entry.text().trim().to_string();
            let icon = if icon_text.is_empty() {
                None
            } else {
                Some(icon_text)
            };

            let id = editing_id.borrow().unwrap_or_else(Uuid::new_v4);

            let folder = SmartFolder {
                id,
                name,
                filter_protocol,
                filter_tags,
                filter_host_pattern,
                filter_group_id,
                sort_order: 0,
                icon,
            };

            if let Some(ref cb) = *on_save.borrow() {
                cb(Some(folder));
            }
            window.close();
        });

        self.window.present();
    }

    /// Returns a reference to the underlying window.
    #[must_use]
    pub const fn window(&self) -> &adw::Window {
        &self.window
    }
}
