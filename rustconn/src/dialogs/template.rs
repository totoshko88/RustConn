//! Template dialog for creating and editing connection templates
//!
//! Provides a GTK4 dialog for managing connection templates, including:
//! - Creating new templates
//! - Editing existing templates
//! - Listing templates by protocol
//!
//! Updated for GTK 4.10+ compatibility using Window instead of Dialog.

use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, DropDown, Entry, Grid, HeaderBar, Label, ListBox, ListBoxRow, Notebook,
    Orientation, ScrolledWindow, SpinButton, StringList, Window,
};
use rustconn_core::models::{
    ConnectionTemplate, ProtocolConfig, ProtocolType, RdpConfig, SpiceConfig, SshConfig, VncConfig,
};
use std::cell::RefCell;
use std::rc::Rc;
use uuid::Uuid;

/// Callback type for template dialog
pub type TemplateCallback = Rc<RefCell<Option<Box<dyn Fn(Option<ConnectionTemplate>)>>>>;

/// Template dialog for creating/editing templates
pub struct TemplateDialog {
    window: Window,
    // Basic fields
    name_entry: Entry,
    description_entry: Entry,
    protocol_dropdown: DropDown,
    host_entry: Entry,
    port_spin: SpinButton,
    username_entry: Entry,
    tags_entry: Entry,
    // State
    editing_id: Rc<RefCell<Option<Uuid>>>,
    // Callback
    on_save: TemplateCallback,
}

impl TemplateDialog {
    /// Creates a new template dialog
    #[must_use]
    pub fn new(parent: Option<&Window>) -> Self {
        let window = Window::builder()
            .title("New Template")
            .modal(true)
            .default_width(500)
            .default_height(500)
            .build();

        if let Some(p) = parent {
            window.set_transient_for(Some(p));
        }

        // Create header bar with Cancel/Save buttons
        let header = HeaderBar::new();
        let cancel_btn = Button::builder().label("Cancel").build();
        let save_btn = Button::builder()
            .label("Save")
            .css_classes(["suggested-action"])
            .build();
        header.pack_start(&cancel_btn);
        header.pack_end(&save_btn);
        window.set_titlebar(Some(&header));

        // Create main content area
        let content = GtkBox::new(Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);
        window.set_child(Some(&content));

        // Create notebook for tabs
        let notebook = Notebook::new();
        notebook.set_vexpand(true);
        content.append(&notebook);

        // === Basic Tab ===
        let (
            basic_grid,
            name_entry,
            description_entry,
            protocol_dropdown,
            host_entry,
            port_spin,
            username_entry,
            tags_entry,
        ) = Self::create_basic_tab();
        notebook.append_page(&basic_grid, Some(&Label::new(Some("Basic"))));

        // Connect protocol dropdown to update port
        let port_clone = port_spin.clone();
        protocol_dropdown.connect_selected_notify(move |dropdown| {
            let selected = dropdown.selected() as usize;
            let default_port = match selected {
                1 => 3389.0,     // RDP
                2 | 3 => 5900.0, // VNC, SPICE
                4 => 22.0,       // ZeroTrust (typically SSH-based)
                _ => 22.0,       // SSH
            };
            // Only update if current port is a default port
            let current = port_clone.value();
            if (current - 22.0).abs() < 0.5
                || (current - 3389.0).abs() < 0.5
                || (current - 5900.0).abs() < 0.5
            {
                port_clone.set_value(default_port);
            }
        });

        let on_save: TemplateCallback = Rc::new(RefCell::new(None));
        let editing_id: Rc<RefCell<Option<Uuid>>> = Rc::new(RefCell::new(None));

        // Connect cancel button
        let window_clone = window.clone();
        let on_save_clone = on_save.clone();
        cancel_btn.connect_clicked(move |_| {
            if let Some(ref cb) = *on_save_clone.borrow() {
                cb(None);
            }
            window_clone.close();
        });

        // Connect save button
        let window_save = window.clone();
        let on_save_save = on_save.clone();
        let editing_id_save = editing_id.clone();
        let name_entry_save = name_entry.clone();
        let description_entry_save = description_entry.clone();
        let protocol_dropdown_save = protocol_dropdown.clone();
        let host_entry_save = host_entry.clone();
        let port_spin_save = port_spin.clone();
        let username_entry_save = username_entry.clone();
        let tags_entry_save = tags_entry.clone();

        save_btn.connect_clicked(move |_| {
            // Validate
            let name = name_entry_save.text();
            if name.trim().is_empty() {
                let alert = gtk4::AlertDialog::builder()
                    .message("Validation Error")
                    .detail("Template name is required")
                    .modal(true)
                    .build();
                alert.show(Some(&window_save));
                return;
            }

            // Build template
            let protocol_idx = protocol_dropdown_save.selected() as usize;
            let protocol_config = match protocol_idx {
                1 => ProtocolConfig::Rdp(RdpConfig::default()),
                2 => ProtocolConfig::Vnc(VncConfig::default()),
                3 => ProtocolConfig::Spice(SpiceConfig::default()),
                4 => ProtocolConfig::ZeroTrust(rustconn_core::models::ZeroTrustConfig::default()),
                _ => ProtocolConfig::Ssh(SshConfig::default()),
            };

            let mut template = ConnectionTemplate::new(name.trim().to_string(), protocol_config);

            // Set description
            let desc = description_entry_save.text();
            if !desc.trim().is_empty() {
                template.description = Some(desc.trim().to_string());
            }

            // Set host
            let host = host_entry_save.text();
            if !host.trim().is_empty() {
                template.host = host.trim().to_string();
            }

            // Set port
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let port = port_spin_save.value() as u16;
            template.port = port;

            // Set username
            let username = username_entry_save.text();
            if !username.trim().is_empty() {
                template.username = Some(username.trim().to_string());
            }

            // Set tags
            let tags_text = tags_entry_save.text();
            if !tags_text.trim().is_empty() {
                template.tags = tags_text
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }

            // Preserve ID if editing
            if let Some(id) = *editing_id_save.borrow() {
                template.id = id;
            }

            if let Some(ref cb) = *on_save_save.borrow() {
                cb(Some(template));
            }
            window_save.close();
        });

        Self {
            window,
            name_entry,
            description_entry,
            protocol_dropdown,
            host_entry,
            port_spin,
            username_entry,
            tags_entry,
            editing_id,
            on_save,
        }
    }

    fn create_basic_tab() -> (
        ScrolledWindow,
        Entry,
        Entry,
        DropDown,
        Entry,
        SpinButton,
        Entry,
        Entry,
    ) {
        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .build();

        let grid = Grid::builder()
            .row_spacing(8)
            .column_spacing(12)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();

        let mut row = 0;

        // Name
        let name_label = Label::builder()
            .label("Name:")
            .halign(gtk4::Align::End)
            .build();
        let name_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("Template name")
            .build();
        grid.attach(&name_label, 0, row, 1, 1);
        grid.attach(&name_entry, 1, row, 1, 1);
        row += 1;

        // Description
        let desc_label = Label::builder()
            .label("Description:")
            .halign(gtk4::Align::End)
            .build();
        let description_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("Optional description")
            .build();
        grid.attach(&desc_label, 0, row, 1, 1);
        grid.attach(&description_entry, 1, row, 1, 1);
        row += 1;

        // Protocol
        let protocol_label = Label::builder()
            .label("Protocol:")
            .halign(gtk4::Align::End)
            .build();
        let protocols = StringList::new(&["SSH", "RDP", "VNC", "SPICE", "ZeroTrust"]);
        let protocol_dropdown = DropDown::builder().model(&protocols).hexpand(true).build();
        grid.attach(&protocol_label, 0, row, 1, 1);
        grid.attach(&protocol_dropdown, 1, row, 1, 1);
        row += 1;

        // Host
        let host_label = Label::builder()
            .label("Default Host:")
            .halign(gtk4::Align::End)
            .build();
        let host_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("Leave empty for user to fill in")
            .build();
        grid.attach(&host_label, 0, row, 1, 1);
        grid.attach(&host_entry, 1, row, 1, 1);
        row += 1;

        // Port
        let port_label = Label::builder()
            .label("Default Port:")
            .halign(gtk4::Align::End)
            .build();
        let port_spin = SpinButton::with_range(1.0, 65535.0, 1.0);
        port_spin.set_value(22.0);
        grid.attach(&port_label, 0, row, 1, 1);
        grid.attach(&port_spin, 1, row, 1, 1);
        row += 1;

        // Username
        let user_label = Label::builder()
            .label("Default Username:")
            .halign(gtk4::Align::End)
            .build();
        let username_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("Optional default username")
            .build();
        grid.attach(&user_label, 0, row, 1, 1);
        grid.attach(&username_entry, 1, row, 1, 1);
        row += 1;

        // Tags
        let tags_label = Label::builder()
            .label("Default Tags:")
            .halign(gtk4::Align::End)
            .build();
        let tags_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("tag1, tag2, ...")
            .build();
        grid.attach(&tags_label, 0, row, 1, 1);
        grid.attach(&tags_entry, 1, row, 1, 1);

        scrolled.set_child(Some(&grid));

        (
            scrolled,
            name_entry,
            description_entry,
            protocol_dropdown,
            host_entry,
            port_spin,
            username_entry,
            tags_entry,
        )
    }

    /// Populates the dialog with an existing template for editing
    pub fn set_template(&self, template: &ConnectionTemplate) {
        self.window.set_title(Some("Edit Template"));
        *self.editing_id.borrow_mut() = Some(template.id);

        self.name_entry.set_text(&template.name);
        if let Some(ref desc) = template.description {
            self.description_entry.set_text(desc);
        }

        // Set protocol dropdown
        let protocol_idx: u32 = match template.protocol {
            ProtocolType::Ssh => 0,
            ProtocolType::Rdp => 1,
            ProtocolType::Vnc => 2,
            ProtocolType::Spice => 3,
            ProtocolType::ZeroTrust => 4,
        };
        self.protocol_dropdown.set_selected(protocol_idx);

        self.host_entry.set_text(&template.host);
        self.port_spin.set_value(f64::from(template.port));

        if let Some(ref username) = template.username {
            self.username_entry.set_text(username);
        }

        self.tags_entry.set_text(&template.tags.join(", "));
    }

    /// Runs the dialog and calls the callback with the result
    pub fn run<F: Fn(Option<ConnectionTemplate>) + 'static>(&self, cb: F) {
        *self.on_save.borrow_mut() = Some(Box::new(cb));
        self.window.present();
    }

    /// Returns a reference to the underlying window
    #[must_use]
    pub const fn window(&self) -> &Window {
        &self.window
    }
}

/// Template manager dialog for listing and managing templates
pub struct TemplateManagerDialog {
    window: Window,
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
    pub fn new(parent: Option<&Window>) -> Self {
        let window = Window::builder()
            .title("Manage Templates")
            .modal(true)
            .default_width(600)
            .default_height(500)
            .build();

        if let Some(p) = parent {
            window.set_transient_for(Some(p));
        }

        // Create header bar
        let header = HeaderBar::new();
        let close_btn = Button::builder().label("Close").build();
        let new_btn = Button::builder()
            .label("New Template")
            .css_classes(["suggested-action"])
            .build();
        header.pack_start(&close_btn);
        header.pack_end(&new_btn);
        window.set_titlebar(Some(&header));

        // Create main content
        let content = GtkBox::new(Orientation::Vertical, 8);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // Protocol filter
        let filter_box = GtkBox::new(Orientation::Horizontal, 8);
        let filter_label = Label::new(Some("Filter by protocol:"));
        let protocols = StringList::new(&["All", "SSH", "RDP", "VNC", "SPICE"]);
        let filter_dropdown = DropDown::builder().model(&protocols).build();
        filter_box.append(&filter_label);
        filter_box.append(&filter_dropdown);
        content.append(&filter_box);

        // Templates list
        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .build();

        let templates_list = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .css_classes(["boxed-list"])
            .build();
        scrolled.set_child(Some(&templates_list));
        content.append(&scrolled);

        // Action buttons
        let button_box = GtkBox::new(Orientation::Horizontal, 8);
        button_box.set_halign(gtk4::Align::End);

        let edit_btn = Button::builder().label("Edit").sensitive(false).build();
        let delete_btn = Button::builder().label("Delete").sensitive(false).build();
        let use_btn = Button::builder()
            .label("Use Template")
            .sensitive(false)
            .css_classes(["suggested-action"])
            .build();

        button_box.append(&edit_btn);
        button_box.append(&delete_btn);
        button_box.append(&use_btn);
        content.append(&button_box);

        window.set_child(Some(&content));

        let state_templates: Rc<RefCell<Vec<ConnectionTemplate>>> =
            Rc::new(RefCell::new(Vec::new()));
        let on_template_selected: Rc<RefCell<Option<Box<dyn Fn(Option<ConnectionTemplate>)>>>> =
            Rc::new(RefCell::new(None));
        let on_new: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let on_edit: Rc<RefCell<Option<Box<dyn Fn(ConnectionTemplate)>>>> =
            Rc::new(RefCell::new(None));
        let on_delete: Rc<RefCell<Option<Box<dyn Fn(Uuid)>>>> = Rc::new(RefCell::new(None));

        // Connect selection changed
        let edit_clone = edit_btn.clone();
        let delete_clone = delete_btn.clone();
        let use_clone = use_btn.clone();
        templates_list.connect_row_selected(move |_, row| {
            let has_selection = row.is_some();
            edit_clone.set_sensitive(has_selection);
            delete_clone.set_sensitive(has_selection);
            use_clone.set_sensitive(has_selection);
        });

        // Connect close button
        let window_clone = window.clone();
        close_btn.connect_clicked(move |_| {
            window_clone.close();
        });

        // Connect new button
        let on_new_clone = on_new.clone();
        new_btn.connect_clicked(move |_| {
            if let Some(ref cb) = *on_new_clone.borrow() {
                cb();
            }
        });

        // Connect edit button
        let on_edit_clone = on_edit.clone();
        let state_templates_edit = state_templates.clone();
        let templates_list_edit = templates_list.clone();
        edit_btn.connect_clicked(move |_| {
            if let Some(row) = templates_list_edit.selected_row() {
                if let Some(id_str) = row.widget_name().as_str().strip_prefix("template-") {
                    if let Ok(id) = Uuid::parse_str(id_str) {
                        let templates = state_templates_edit.borrow();
                        if let Some(template) = templates.iter().find(|t| t.id == id) {
                            if let Some(ref cb) = *on_edit_clone.borrow() {
                                cb(template.clone());
                            }
                        }
                    }
                }
            }
        });

        // Connect delete button
        let on_delete_clone = on_delete.clone();
        let templates_list_delete = templates_list.clone();
        delete_btn.connect_clicked(move |_| {
            if let Some(row) = templates_list_delete.selected_row() {
                if let Some(id_str) = row.widget_name().as_str().strip_prefix("template-") {
                    if let Ok(id) = Uuid::parse_str(id_str) {
                        if let Some(ref cb) = *on_delete_clone.borrow() {
                            cb(id);
                        }
                    }
                }
            }
        });

        // Connect use button
        let on_selected_clone = on_template_selected.clone();
        let state_templates_use = state_templates.clone();
        let templates_list_use = templates_list.clone();
        let window_use = window.clone();
        use_btn.connect_clicked(move |_| {
            if let Some(row) = templates_list_use.selected_row() {
                if let Some(id_str) = row.widget_name().as_str().strip_prefix("template-") {
                    if let Ok(id) = Uuid::parse_str(id_str) {
                        let templates = state_templates_use.borrow();
                        if let Some(template) = templates.iter().find(|t| t.id == id) {
                            if let Some(ref cb) = *on_selected_clone.borrow() {
                                cb(Some(template.clone()));
                            }
                            window_use.close();
                        }
                    }
                }
            }
        });

        Self {
            window,
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
        // Clear existing rows
        while let Some(row) = self.templates_list.row_at_index(0) {
            self.templates_list.remove(&row);
        }

        let templates = self.state_templates.borrow();

        // Group templates by protocol
        let mut ssh_templates: Vec<&ConnectionTemplate> = Vec::new();
        let mut rdp_templates: Vec<&ConnectionTemplate> = Vec::new();
        let mut vnc_templates: Vec<&ConnectionTemplate> = Vec::new();
        let mut spice_templates: Vec<&ConnectionTemplate> = Vec::new();

        for template in templates.iter() {
            if let Some(filter) = protocol_filter {
                if template.protocol != filter {
                    continue;
                }
            }
            match template.protocol {
                ProtocolType::Ssh | ProtocolType::ZeroTrust => ssh_templates.push(template),
                ProtocolType::Rdp => rdp_templates.push(template),
                ProtocolType::Vnc => vnc_templates.push(template),
                ProtocolType::Spice => spice_templates.push(template),
            }
        }

        // Add SSH templates
        if !ssh_templates.is_empty() && protocol_filter.is_none() {
            self.add_section_header("SSH Templates");
        }
        for template in ssh_templates {
            self.add_template_row(template);
        }

        // Add RDP templates
        if !rdp_templates.is_empty() && protocol_filter.is_none() {
            self.add_section_header("RDP Templates");
        }
        for template in rdp_templates {
            self.add_template_row(template);
        }

        // Add VNC templates
        if !vnc_templates.is_empty() && protocol_filter.is_none() {
            self.add_section_header("VNC Templates");
        }
        for template in vnc_templates {
            self.add_template_row(template);
        }

        // Add SPICE templates
        if !spice_templates.is_empty() && protocol_filter.is_none() {
            self.add_section_header("SPICE Templates");
        }
        for template in spice_templates {
            self.add_template_row(template);
        }
    }

    fn add_section_header(&self, title: &str) {
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
        self.templates_list.append(&row);
    }

    fn add_template_row(&self, template: &ConnectionTemplate) {
        let hbox = GtkBox::new(Orientation::Horizontal, 8);
        hbox.set_margin_top(8);
        hbox.set_margin_bottom(8);
        hbox.set_margin_start(8);
        hbox.set_margin_end(8);

        // Protocol icon
        let icon_name = match template.protocol {
            ProtocolType::Ssh => "utilities-terminal-symbolic",
            ProtocolType::Rdp => "computer-symbolic",
            ProtocolType::Vnc => "video-display-symbolic",
            ProtocolType::Spice => "video-display-symbolic",
            ProtocolType::ZeroTrust => "cloud-symbolic",
        };
        let icon = gtk4::Image::from_icon_name(icon_name);
        hbox.append(&icon);

        // Template info
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
        self.templates_list.append(&row);
    }

    /// Gets the currently selected template
    #[must_use]
    pub fn get_selected_template(&self) -> Option<ConnectionTemplate> {
        if let Some(row) = self.templates_list.selected_row() {
            if let Some(id_str) = row.widget_name().as_str().strip_prefix("template-") {
                if let Ok(id) = Uuid::parse_str(id_str) {
                    let templates = self.state_templates.borrow();
                    return templates.iter().find(|t| t.id == id).cloned();
                }
            }
        }
        None
    }

    /// Returns a reference to the underlying window
    #[must_use]
    pub const fn window(&self) -> &Window {
        &self.window
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
        self.window.present();
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
