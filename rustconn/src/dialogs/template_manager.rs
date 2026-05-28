//! Template manager dialog for listing and managing templates.
//!
//! Extracted from `dialogs/template.rs` to reduce module complexity.

use crate::i18n::i18n;
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, DropDown, Label, ListBox, ListBoxRow, Orientation, ScrolledWindow,
    StringList,
};
use libadwaita as adw;
use rustconn_core::models::{ConnectionTemplate, ProtocolType};
use std::cell::RefCell;
use std::rc::Rc;
use uuid::Uuid;

/// Template manager dialog for listing and managing templates
pub struct TemplateManagerDialog {
    dialog: adw::Dialog,
    parent: Option<gtk4::Widget>,
    templates_list: ListBox,
    state_templates: Rc<RefCell<Vec<ConnectionTemplate>>>,
    on_template_selected: Rc<RefCell<Option<Box<dyn Fn(Option<ConnectionTemplate>)>>>>,
    on_new: Rc<RefCell<Option<Box<dyn Fn()>>>>,
    on_edit: Rc<RefCell<Option<Box<dyn Fn(ConnectionTemplate)>>>>,
    on_delete: Rc<RefCell<Option<Box<dyn Fn(Uuid)>>>>,
}

impl TemplateManagerDialog {
    /// Creates a new template manager dialog
    #[must_use]
    pub fn new(parent: Option<&impl IsA<gtk4::Widget>>) -> Self {
        let dialog = adw::Dialog::builder()
            .title(i18n("Manage Templates"))
            .content_width(600)
            .content_height(500)
            .build();

        let parent_widget: Option<gtk4::Widget> =
            parent.map(|p| p.clone().upcast::<gtk4::Widget>());

        // Header bar with Add button and standard window buttons (GNOME HIG)
        let header = adw::HeaderBar::new();
        let add_btn = Button::from_icon_name("list-add-symbolic");
        add_btn.set_tooltip_text(Some(&i18n("New Template")));
        add_btn.update_property(&[gtk4::accessible::Property::Label(&i18n("New Template"))]);
        header.pack_start(&add_btn);

        let clamp = adw::Clamp::builder()
            .maximum_size(600)
            .tightening_threshold(400)
            .build();

        let content = GtkBox::new(Orientation::Vertical, 8);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        clamp.set_child(Some(&content));

        let filter_box = GtkBox::new(Orientation::Horizontal, 8);
        let filter_label = Label::new(Some(&i18n("Filter by protocol:")));
        let filter_items: Vec<String> = vec![
            i18n("All"),
            i18n("SSH"),
            i18n("RDP"),
            i18n("VNC"),
            i18n("SPICE"),
        ];
        let filter_refs: Vec<&str> = filter_items.iter().map(String::as_str).collect();
        let protocols = StringList::new(&filter_refs);
        let filter_dropdown = DropDown::builder().model(&protocols).build();
        filter_box.append(&filter_label);
        filter_box.append(&filter_dropdown);
        content.append(&filter_box);

        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .build();

        let templates_list = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .css_classes(["boxed-list"])
            .build();

        // Empty state placeholder (GNOME HIG)
        let placeholder = adw::StatusPage::builder()
            .icon_name("document-page-setup-symbolic")
            .title(i18n("No Templates"))
            .description(i18n(
                "Create a template to quickly set up new connections with predefined settings.",
            ))
            .build();
        templates_list.set_placeholder(Some(&placeholder));

        scrolled.set_child(Some(&templates_list));
        content.append(&scrolled);

        let button_box = GtkBox::new(Orientation::Horizontal, 8);
        button_box.set_halign(gtk4::Align::End);

        let edit_btn = Button::builder()
            .label(i18n("Edit"))
            .sensitive(false)
            .build();
        let delete_btn = Button::builder()
            .label(i18n("Delete"))
            .sensitive(false)
            .build();
        let create_conn_btn = Button::builder()
            .label(i18n("Use Template"))
            .sensitive(false)
            .css_classes(["suggested-action"])
            .build();

        button_box.append(&edit_btn);
        button_box.append(&delete_btn);
        button_box.append(&create_conn_btn);
        content.append(&button_box);

        // Use ToolbarView for adw::Dialog
        let main_box = GtkBox::new(Orientation::Vertical, 0);
        main_box.append(&header);
        main_box.append(&clamp);
        dialog.set_child(Some(&main_box));

        let state_templates: Rc<RefCell<Vec<ConnectionTemplate>>> =
            Rc::new(RefCell::new(Vec::new()));
        let on_template_selected: Rc<RefCell<Option<Box<dyn Fn(Option<ConnectionTemplate>)>>>> =
            Rc::new(RefCell::new(None));
        let on_new: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let on_edit: Rc<RefCell<Option<Box<dyn Fn(ConnectionTemplate)>>>> =
            Rc::new(RefCell::new(None));
        let on_delete: Rc<RefCell<Option<Box<dyn Fn(Uuid)>>>> = Rc::new(RefCell::new(None));

        let edit_clone = edit_btn.clone();
        let delete_clone = delete_btn.clone();
        let create_conn_clone = create_conn_btn.clone();
        templates_list.connect_row_selected(move |_, row| {
            let has_selection = row.is_some();
            edit_clone.set_sensitive(has_selection);
            delete_clone.set_sensitive(has_selection);
            create_conn_clone.set_sensitive(has_selection);
        });

        // "Add" button in header - creates a new template
        let on_new_clone = on_new.clone();
        add_btn.connect_clicked(move |_| {
            if let Some(ref cb) = *on_new_clone.borrow() {
                cb();
            }
        });

        let on_edit_clone = on_edit.clone();
        let state_templates_edit = state_templates.clone();
        let templates_list_edit = templates_list.clone();
        edit_btn.connect_clicked(move |_| {
            if let Some(row) = templates_list_edit.selected_row()
                && let Some(id_str) = row.widget_name().as_str().strip_prefix("template-")
                && let Ok(id) = Uuid::parse_str(id_str)
            {
                let templates = state_templates_edit.borrow();
                if let Some(template) = templates.iter().find(|t| t.id == id)
                    && let Some(ref cb) = *on_edit_clone.borrow()
                {
                    cb(template.clone());
                }
            }
        });

        let on_delete_clone = on_delete.clone();
        let templates_list_delete = templates_list.clone();
        let window_weak_delete = dialog.downgrade();
        delete_btn.connect_clicked(move |_| {
            if let Some(row) = templates_list_delete.selected_row()
                && let Some(id_str) = row.widget_name().as_str().strip_prefix("template-")
                && let Ok(id) = Uuid::parse_str(id_str)
                && let Some(win) = window_weak_delete.upgrade()
            {
                let alert = adw::AlertDialog::builder()
                    .heading(i18n("Delete Template?"))
                    .body(i18n("This template will be permanently removed."))
                    .build();
                alert.add_response("cancel", &i18n("Cancel"));
                alert.add_response("delete", &i18n("Delete"));
                alert.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
                alert.set_default_response(Some("cancel"));
                alert.set_close_response("cancel");
                let on_delete_inner = on_delete_clone.clone();
                alert.connect_response(None, move |_, response| {
                    if response == "delete"
                        && let Some(ref cb) = *on_delete_inner.borrow()
                    {
                        cb(id);
                    }
                });
                alert.present(Some(&win));
            }
        });

        // "Create Connection" button in header - creates connection from selected template
        let on_selected_clone = on_template_selected.clone();
        let state_templates_use = state_templates.clone();
        let templates_list_use = templates_list.clone();
        let window_use = dialog.clone();
        create_conn_btn.connect_clicked(move |_| {
            if let Some(row) = templates_list_use.selected_row()
                && let Some(id_str) = row.widget_name().as_str().strip_prefix("template-")
                && let Ok(id) = Uuid::parse_str(id_str)
            {
                let templates = state_templates_use.borrow();
                if let Some(template) = templates.iter().find(|t| t.id == id) {
                    if let Some(ref cb) = *on_selected_clone.borrow() {
                        cb(Some(template.clone()));
                    }
                    window_use.close();
                }
            }
        });

        // Double-click on template row - creates connection from template
        let gesture = gtk4::GestureClick::new();
        gesture.set_button(1); // Left mouse button
        let on_selected_dblclick = on_template_selected.clone();
        let state_templates_dblclick = state_templates.clone();
        let templates_list_dblclick = templates_list.clone();
        let window_dblclick = dialog.clone();
        gesture.connect_pressed(move |gesture, n_press, _x, y| {
            if n_press == 2 {
                // Double-click
                if let Some(row) = templates_list_dblclick.row_at_y(y as i32)
                    && let Some(id_str) = row.widget_name().as_str().strip_prefix("template-")
                    && let Ok(id) = Uuid::parse_str(id_str)
                {
                    let templates = state_templates_dblclick.borrow();
                    if let Some(template) = templates.iter().find(|t| t.id == id) {
                        if let Some(ref cb) = *on_selected_dblclick.borrow() {
                            cb(Some(template.clone()));
                        }
                        window_dblclick.close();
                    }
                }
                gesture.set_state(gtk4::EventSequenceState::Claimed);
            }
        });
        templates_list.add_controller(gesture);

        // Connect filter dropdown to refresh the list when protocol filter changes
        let templates_list_filter = templates_list.clone();
        let state_templates_filter = state_templates.clone();
        filter_dropdown.connect_selected_notify(move |dropdown| {
            let protocol_filter = match dropdown.selected() {
                1 => Some(ProtocolType::Ssh),
                2 => Some(ProtocolType::Rdp),
                3 => Some(ProtocolType::Vnc),
                4 => Some(ProtocolType::Spice),
                _ => None, // 0 = "All"
            };
            refresh_templates_list(
                &templates_list_filter,
                &state_templates_filter.borrow(),
                protocol_filter,
            );
        });

        Self {
            dialog,
            parent: parent_widget,
            templates_list,
            state_templates,
            on_template_selected,
            on_new,
            on_edit,
            on_delete,
        }
    }

    /// Sets the templates to display
    pub fn set_templates(&self, templates: Vec<ConnectionTemplate>) {
        *self.state_templates.borrow_mut() = templates;
        self.refresh_list(None);
    }

    /// Refreshes the templates list with optional protocol filter
    pub fn refresh_list(&self, protocol_filter: Option<ProtocolType>) {
        refresh_templates_list(
            &self.templates_list,
            &self.state_templates.borrow(),
            protocol_filter,
        );
    }

    /// Gets the currently selected template
    #[must_use]
    pub fn get_selected_template(&self) -> Option<ConnectionTemplate> {
        if let Some(row) = self.templates_list.selected_row()
            && let Some(id_str) = row.widget_name().as_str().strip_prefix("template-")
            && let Ok(id) = Uuid::parse_str(id_str)
        {
            let templates = self.state_templates.borrow();
            return templates.iter().find(|t| t.id == id).cloned();
        }
        None
    }

    /// Returns a reference to the underlying dialog
    #[must_use]
    pub const fn dialog(&self) -> &adw::Dialog {
        &self.dialog
    }

    /// Returns a reference to the templates list
    #[must_use]
    pub const fn templates_list(&self) -> &ListBox {
        &self.templates_list
    }

    /// Returns a reference to the state templates
    #[must_use]
    pub fn state_templates(&self) -> &Rc<RefCell<Vec<ConnectionTemplate>>> {
        &self.state_templates
    }

    /// Presents the dialog
    pub fn present(&self) {
        self.dialog.present(self.parent.as_ref());
    }

    /// Sets the callback for creating a new template
    pub fn set_on_new<F: Fn() + 'static>(&self, cb: F) {
        *self.on_new.borrow_mut() = Some(Box::new(cb));
    }

    /// Sets the callback for editing a template
    pub fn set_on_edit<F: Fn(ConnectionTemplate) + 'static>(&self, cb: F) {
        *self.on_edit.borrow_mut() = Some(Box::new(cb));
    }

    /// Sets the callback for deleting a template
    pub fn set_on_delete<F: Fn(Uuid) + 'static>(&self, cb: F) {
        *self.on_delete.borrow_mut() = Some(Box::new(cb));
    }

    /// Sets the callback for selecting a template to use
    pub fn set_on_template_selected<F: Fn(Option<ConnectionTemplate>) + 'static>(&self, cb: F) {
        *self.on_template_selected.borrow_mut() = Some(Box::new(cb));
    }
}

/// Refreshes the templates list widget with optional protocol filter.
///
/// Extracted as a free function so both `TemplateManagerDialog::refresh_list`
/// and the filter dropdown closure can reuse the same logic.
fn refresh_templates_list(
    list: &ListBox,
    templates: &[ConnectionTemplate],
    protocol_filter: Option<ProtocolType>,
) {
    while let Some(row) = list.row_at_index(0) {
        list.remove(&row);
    }

    let mut ssh_templates: Vec<&ConnectionTemplate> = Vec::new();
    let mut rdp_templates: Vec<&ConnectionTemplate> = Vec::new();
    let mut vnc_templates: Vec<&ConnectionTemplate> = Vec::new();
    let mut spice_templates: Vec<&ConnectionTemplate> = Vec::new();

    for template in templates {
        if let Some(filter) = protocol_filter
            && template.protocol != filter
        {
            continue;
        }
        match template.protocol {
            ProtocolType::Ssh | ProtocolType::ZeroTrust | ProtocolType::Telnet => {
                ssh_templates.push(template);
            }
            ProtocolType::Rdp => rdp_templates.push(template),
            ProtocolType::Vnc => vnc_templates.push(template),
            ProtocolType::Spice => spice_templates.push(template),
            ProtocolType::Serial | ProtocolType::Sftp => {
                ssh_templates.push(template);
            }
            ProtocolType::Kubernetes | ProtocolType::Mosh => {
                ssh_templates.push(template);
            }
            ProtocolType::Web => {} // Web bookmarks don't use templates
        }
    }

    if !ssh_templates.is_empty() && protocol_filter.is_none() {
        append_section_header(list, &i18n("SSH Templates"));
    }
    for template in ssh_templates {
        append_template_row(list, template);
    }

    if !rdp_templates.is_empty() && protocol_filter.is_none() {
        append_section_header(list, &i18n("RDP Templates"));
    }
    for template in rdp_templates {
        append_template_row(list, template);
    }

    if !vnc_templates.is_empty() && protocol_filter.is_none() {
        append_section_header(list, &i18n("VNC Templates"));
    }
    for template in vnc_templates {
        append_template_row(list, template);
    }

    if !spice_templates.is_empty() && protocol_filter.is_none() {
        append_section_header(list, &i18n("SPICE Templates"));
    }
    for template in spice_templates {
        append_template_row(list, template);
    }
}

/// Appends a section header row to the templates list.
fn append_section_header(list: &ListBox, title: &str) {
    let label = Label::builder()
        .label(title)
        .halign(gtk4::Align::Start)
        .css_classes(["heading"])
        .margin_top(8)
        .margin_bottom(4)
        .margin_start(8)
        .build();
    let row = ListBoxRow::builder()
        .child(&label)
        .selectable(false)
        .activatable(false)
        .build();
    list.append(&row);
}

/// Appends a template row to the templates list.
fn append_template_row(list: &ListBox, template: &ConnectionTemplate) {
    let hbox = GtkBox::new(Orientation::Horizontal, 8);
    hbox.set_margin_top(12);
    hbox.set_margin_bottom(12);
    hbox.set_margin_start(12);
    hbox.set_margin_end(12);

    let icon_name = rustconn_core::get_protocol_icon(template.protocol);
    let icon = gtk4::Image::from_icon_name(icon_name);
    hbox.append(&icon);

    let info_box = GtkBox::new(Orientation::Vertical, 2);
    info_box.set_hexpand(true);

    let name_label = Label::builder()
        .label(&template.name)
        .halign(gtk4::Align::Start)
        .css_classes(["heading"])
        .build();
    info_box.append(&name_label);

    let details = if let Some(ref desc) = template.description {
        desc.clone()
    } else {
        let mut parts = Vec::new();
        if !template.host.is_empty() {
            parts.push(format!("Host: {}", template.host));
        }
        parts.push(format!("Port: {}", template.port));
        if let Some(ref user) = template.username {
            parts.push(format!("User: {user}"));
        }
        parts.join(" | ")
    };

    let details_label = Label::builder()
        .label(&details)
        .halign(gtk4::Align::Start)
        .css_classes(["dim-label"])
        .build();
    info_box.append(&details_label);

    hbox.append(&info_box);

    let row = ListBoxRow::builder().child(&hbox).build();
    row.set_widget_name(&format!("template-{}", template.id));
    list.append(&row);
}
