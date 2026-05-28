//! Step 2: Connection details page
//!
//! Adaptive form that changes fields based on the selected protocol.
//! Includes Jump Host dropdown for protocols that support SSH tunneling.

use crate::i18n::{i18n, i18n_f};
use crate::state::SharedAppState;
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, Orientation, ScrolledWindow, StringList};
use libadwaita as adw;
use rustconn_core::models::ProtocolType;
use std::cell::RefCell;
use std::rc::Rc;
use uuid::Uuid;

/// Connection details page — Step 2 of the wizard
#[allow(dead_code, reason = "Fields kept for GTK widget lifecycle")]
pub struct ConnectionPage {
    pub page: adw::NavigationPage,
    state: SharedAppState,
    // Shared widgets
    pub(super) name_row: adw::EntryRow,
    pub(super) host_row: adw::EntryRow,
    pub(super) port_row: adw::SpinRow,
    pub(super) username_row: adw::EntryRow,
    pub(super) domain_row: adw::EntryRow,
    jump_host_row: adw::ComboRow,
    // Serial-specific
    device_row: adw::EntryRow,
    baud_row: adw::ComboRow,
    // Kubernetes-specific
    k8s_context_row: adw::EntryRow,
    k8s_namespace_row: adw::EntryRow,
    k8s_pod_row: adw::EntryRow,
    k8s_container_row: adw::EntryRow,
    // Zero Trust-specific
    zt_provider_row: adw::ComboRow,
    zt_command_row: adw::EntryRow,
    zt_field1_row: adw::EntryRow,
    zt_field2_row: adw::EntryRow,
    zt_field3_row: adw::EntryRow,
    // Web-specific
    url_row: adw::EntryRow,
    // Info label
    info_label: gtk4::Label,
    // Groups
    connection_group: adw::PreferencesGroup,
    serial_group: adw::PreferencesGroup,
    k8s_group: adw::PreferencesGroup,
    zt_group: adw::PreferencesGroup,
    web_group: adw::PreferencesGroup,
    // Template grid for Custom Command mode
    templates_group: GtkBox,
    templates_flow: gtk4::FlowBox,
    // Navigation
    next_button: Button,
    on_next: Rc<RefCell<Option<Box<dyn Fn()>>>>,
    on_advanced: Rc<RefCell<Option<Box<dyn Fn()>>>>,
    // Jump host data
    jump_host_ids: Rc<RefCell<Vec<Option<Uuid>>>>,
    // Current protocol
    current_protocol: Rc<RefCell<Option<ProtocolType>>>,
}

impl ConnectionPage {
    /// Creates the connection details page
    #[must_use]
    pub fn new(state: SharedAppState) -> Self {
        let on_next: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let on_advanced: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let current_protocol: Rc<RefCell<Option<ProtocolType>>> = Rc::new(RefCell::new(None));

        let content_box = GtkBox::new(Orientation::Vertical, 12);
        content_box.set_margin_top(12);
        content_box.set_margin_bottom(12);
        content_box.set_margin_start(12);
        content_box.set_margin_end(12);

        let clamp = adw::Clamp::builder()
            .maximum_size(520)
            .child(&content_box)
            .build();

        // Info label (for MOSH/SFTP subtitles)
        let info_label = gtk4::Label::builder()
            .wrap(true)
            .css_classes(["dim-label"])
            .halign(gtk4::Align::Start)
            .visible(false)
            .build();
        content_box.append(&info_label);

        // === Connection group ===
        let connection_group = adw::PreferencesGroup::builder()
            .title(i18n("Connection"))
            .build();

        let name_row = adw::EntryRow::builder().title(i18n("Name")).build();
        name_row.set_tooltip_text(Some(&i18n("Optional \u{2014} auto-generated if empty")));
        connection_group.add(&name_row);

        let host_row = adw::EntryRow::builder().title(i18n("Host")).build();
        connection_group.add(&host_row);

        let port_adj = gtk4::Adjustment::new(22.0, 1.0, 65535.0, 1.0, 10.0, 0.0);
        let port_row = adw::SpinRow::builder()
            .title(i18n("Port"))
            .adjustment(&port_adj)
            .build();
        connection_group.add(&port_row);

        let current_user = std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .unwrap_or_default();
        let username_row = adw::EntryRow::builder().title(i18n("Username")).build();
        if !current_user.is_empty() {
            username_row.set_tooltip_text(Some(&i18n_f("Default: {}", &[&current_user])));
        }
        username_row.set_visible(false);
        connection_group.add(&username_row);

        let domain_row = adw::EntryRow::builder()
            .title(i18n("Domain"))
            .visible(false)
            .build();
        connection_group.add(&domain_row);

        let jump_host_row = adw::ComboRow::builder()
            .title(i18n("Jump Host"))
            .subtitle(i18n("Connect via SSH tunnel"))
            .visible(false)
            .build();
        connection_group.add(&jump_host_row);

        content_box.append(&connection_group);

        // === Serial group ===
        let serial_group = adw::PreferencesGroup::builder()
            .title(i18n("Serial Connection"))
            .visible(false)
            .build();

        let device_row = adw::EntryRow::builder()
            .title(i18n("Device"))
            .text("/dev/ttyUSB0")
            .build();
        serial_group.add(&device_row);

        let baud_model = StringList::new(&[
            "9600", "19200", "38400", "57600", "115200", "230400", "460800",
        ]);
        let baud_row = adw::ComboRow::builder()
            .title(i18n("Baud Rate"))
            .model(&baud_model)
            .selected(4)
            .build();
        serial_group.add(&baud_row);

        content_box.append(&serial_group);

        // === Kubernetes group ===
        let k8s_group = adw::PreferencesGroup::builder()
            .title(i18n("Kubernetes"))
            .visible(false)
            .build();

        let k8s_context_row = adw::EntryRow::builder().title(i18n("Context")).build();
        k8s_context_row.set_tooltip_text(Some(&i18n("Leave empty for current context")));
        k8s_group.add(&k8s_context_row);

        let k8s_namespace_row = adw::EntryRow::builder()
            .title(i18n("Namespace"))
            .text("default")
            .build();
        k8s_group.add(&k8s_namespace_row);

        let k8s_pod_row = adw::EntryRow::builder().title(i18n("Pod")).build();
        k8s_group.add(&k8s_pod_row);

        let k8s_container_row = adw::EntryRow::builder().title(i18n("Container")).build();
        k8s_container_row.set_tooltip_text(Some(&i18n("Leave empty for first container")));
        k8s_group.add(&k8s_container_row);

        content_box.append(&k8s_group);

        // === Zero Trust group ===
        let zt_group = adw::PreferencesGroup::builder()
            .title(i18n("Zero Trust"))
            .visible(false)
            .build();

        let zt_provider_strs: Vec<String> = vec![
            i18n("Custom Command"),
            "AWS SSM".to_string(),
            "GCP IAP".to_string(),
            "Azure Bastion".to_string(),
            "Azure SSH".to_string(),
            "Cloudflare Access".to_string(),
            "Teleport".to_string(),
            "Tailscale SSH".to_string(),
            "Boundary".to_string(),
            "Hoop.dev".to_string(),
        ];
        let zt_refs: Vec<&str> = zt_provider_strs.iter().map(String::as_str).collect();
        let zt_provider_model = StringList::new(&zt_refs);
        let zt_provider_row = adw::ComboRow::builder()
            .title(i18n("Provider"))
            .subtitle(i18n("Run any command for connection"))
            .model(&zt_provider_model)
            .selected(0)
            .build();
        zt_group.add(&zt_provider_row);

        let zt_command_row = adw::EntryRow::builder().title(i18n("Command")).build();
        zt_command_row.set_tooltip_text(Some("cloudflared access ssh --hostname ..."));
        zt_group.add(&zt_command_row);

        let zt_field1_row = adw::EntryRow::builder().visible(false).build();
        zt_group.add(&zt_field1_row);
        let zt_field2_row = adw::EntryRow::builder().visible(false).build();
        zt_group.add(&zt_field2_row);
        let zt_field3_row = adw::EntryRow::builder().visible(false).build();
        zt_group.add(&zt_field3_row);

        content_box.append(&zt_group);

        // === Web group ===
        let web_group = adw::PreferencesGroup::builder()
            .title(i18n("Web Bookmark"))
            .visible(false)
            .build();

        let url_row = adw::EntryRow::builder()
            .title(i18n("URL"))
            .text("https://")
            .build();
        web_group.add(&url_row);

        content_box.append(&web_group);

        // Create next_button early so template buttons can reference it
        let next_button = Button::with_label(&i18n("Next"));
        next_button.add_css_class("suggested-action");
        next_button.set_sensitive(false);

        // === Templates grid (for Custom Command mode) ===
        #[expect(
            clippy::items_after_statements,
            reason = "local helper introduced inline next to its only call site; hoisting would scatter related logic"
        )]
        const MAX_GRID_SLOTS: usize = 7;
        // Use a plain GtkBox instead of PreferencesGroup to avoid ListBoxRow
        // wrapping which intercepts button clicks.
        let templates_group = GtkBox::new(Orientation::Vertical, 6);
        templates_group.set_visible(false);
        templates_group.set_margin_top(12);

        let templates_header = gtk4::Label::builder()
            .label(&i18n("Templates"))
            .halign(gtk4::Align::Start)
            .css_classes(["heading"])
            .build();
        templates_group.append(&templates_header);

        let templates_flow = gtk4::FlowBox::builder()
            .homogeneous(true)
            .min_children_per_line(2)
            .max_children_per_line(4)
            .selection_mode(gtk4::SelectionMode::None)
            .row_spacing(6)
            .column_spacing(6)
            .activate_on_single_click(false)
            .build();

        // Populate: user ZeroTrust templates first, then predefined to fill 7 slots
        let zt_cmd_for_flow = zt_command_row.clone();
        let name_row_for_flow = name_row.clone();
        let next_btn_for_flow = next_button.clone();

        let mut slots_used = 0usize;

        // 1) User templates (ZeroTrust protocol = Custom Command)
        {
            let state_ref = state.borrow();
            let user_zt_templates: Vec<_> = state_ref
                .get_all_templates()
                .into_iter()
                .filter(|t| t.protocol == ProtocolType::ZeroTrust)
                .collect();

            for user_tpl in user_zt_templates.iter().take(MAX_GRID_SLOTS) {
                let icon_str = user_tpl.icon.clone().unwrap_or_default();
                let tpl_name = user_tpl.name.clone();
                // Extract command from ZeroTrust config
                let cmd_str = if let rustconn_core::models::ProtocolConfig::ZeroTrust(ref zt) =
                    user_tpl.protocol_config
                {
                    if let rustconn_core::models::ZeroTrustProviderConfig::Generic(ref g) =
                        zt.provider_config
                    {
                        g.command_template.clone()
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };

                if cmd_str.is_empty() {
                    continue;
                }

                let btn = Self::create_user_template_button(&icon_str, &tpl_name);
                let zt_cmd_clone = zt_cmd_for_flow.clone();
                let name_row_clone = name_row_for_flow.clone();
                let next_btn_clone = next_btn_for_flow.clone();
                let icon_for_cb = icon_str.clone();
                let name_for_cb = tpl_name.clone();
                btn.connect_clicked(move |_| {
                    zt_cmd_clone.set_text(&cmd_str);
                    if name_row_clone.text().is_empty() {
                        name_row_clone.set_text(&name_for_cb);
                    }
                    if !icon_for_cb.is_empty() {
                        name_row_clone.set_widget_name(&format!("tpl-icon:{}", icon_for_cb));
                    }
                    next_btn_clone.set_sensitive(true);
                });
                templates_flow.append(&btn);
                slots_used += 1;
            }
        }

        // 2) Fill remaining slots with predefined templates
        let remaining = MAX_GRID_SLOTS.saturating_sub(slots_used);
        for predefined in rustconn_core::PREDEFINED_TEMPLATES.iter().take(remaining) {
            let btn = Self::create_template_button(predefined);
            let cmd = predefined.command;
            let tpl_name = predefined.name;
            let tpl_icon = predefined.icon;
            let zt_cmd_clone = zt_cmd_for_flow.clone();
            let name_row_clone = name_row_for_flow.clone();
            let next_btn_clone = next_btn_for_flow.clone();
            btn.connect_clicked(move |_| {
                zt_cmd_clone.set_text(cmd);
                if name_row_clone.text().is_empty() {
                    name_row_clone.set_text(tpl_name);
                }
                name_row_clone.set_widget_name(&format!("tpl-icon:{tpl_icon}"));
                next_btn_clone.set_sensitive(true);
            });
            templates_flow.append(&btn);
        }

        // 3) "More…" button — shows popover with all templates
        let more_btn = Self::create_more_templates_button();
        let zt_cmd_for_more = zt_command_row.clone();
        let name_row_for_more = name_row.clone();
        let next_btn_for_more = next_button.clone();
        let state_for_more = state.clone();
        more_btn.connect_clicked(move |btn| {
            Self::show_all_templates_popover(
                btn,
                &zt_cmd_for_more,
                &name_row_for_more,
                &next_btn_for_more,
                &state_for_more,
            );
        });
        templates_flow.append(&more_btn);

        templates_group.append(&templates_flow);
        content_box.append(&templates_group);

        // === Footer (sticky bottom bar) ===
        let footer = GtkBox::new(Orientation::Horizontal, 12);
        footer.set_margin_top(6);
        footer.set_margin_bottom(6);
        footer.set_margin_start(12);
        footer.set_margin_end(12);

        let advanced_btn = Button::with_label(&i18n("Advanced\u{2026}"));
        advanced_btn.add_css_class("flat");
        advanced_btn.add_css_class("dim-label");
        advanced_btn.set_tooltip_text(Some(&i18n("Open full connection editor")));
        advanced_btn.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Open full connection editor",
        ))]);
        footer.append(&advanced_btn);

        let spacer = GtkBox::new(Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        footer.append(&spacer);

        footer.append(&next_button);

        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .child(&clamp)
            .vexpand(true)
            .build();

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&adw::HeaderBar::new());
        toolbar_view.set_content(Some(&scrolled));
        toolbar_view.add_bottom_bar(&footer);

        let page = adw::NavigationPage::builder()
            .title(i18n("Connection Details"))
            .child(&toolbar_view)
            .build();

        // Wire Next button
        let on_next_clone = on_next.clone();
        next_button.connect_clicked(move |_| {
            if let Some(ref cb) = *on_next_clone.borrow() {
                cb();
            }
        });

        // Wire Advanced button
        let on_advanced_clone = on_advanced.clone();
        advanced_btn.connect_clicked(move |_| {
            if let Some(ref cb) = *on_advanced_clone.borrow() {
                cb();
            }
        });

        // Validation
        let next_btn_v = next_button.clone();
        let host_row_v = host_row.clone();
        let current_protocol_v = current_protocol.clone();
        host_row.connect_changed(move |_| {
            let proto = *current_protocol_v.borrow();
            let valid = match proto {
                Some(
                    ProtocolType::Serial
                    | ProtocolType::Kubernetes
                    | ProtocolType::Web
                    | ProtocolType::ZeroTrust,
                ) => true,
                _ => !host_row_v.text().trim().is_empty(),
            };
            next_btn_v.set_sensitive(valid);
        });

        // URL validation for Web protocol
        let next_btn_url = next_button.clone();
        let current_protocol_url = current_protocol.clone();
        url_row.connect_changed(move |row| {
            let proto = *current_protocol_url.borrow();
            if proto == Some(ProtocolType::Web) {
                let url = row.text();
                let url_trimmed = url.trim();
                let valid = url_trimmed
                    .strip_prefix("https://")
                    .or_else(|| url_trimmed.strip_prefix("http://"))
                    .is_some_and(|rest| !rest.is_empty());
                next_btn_url.set_sensitive(valid);
                if valid || url_trimmed.is_empty() {
                    row.remove_css_class("error");
                } else {
                    row.add_css_class("error");
                }
            }
        });

        // ZT provider change
        let zt_cmd = zt_command_row.clone();
        let zt_f1 = zt_field1_row.clone();
        let zt_f2 = zt_field2_row.clone();
        let zt_f3 = zt_field3_row.clone();
        zt_provider_row.connect_selected_notify(move |row| {
            Self::update_zt_fields(row.selected(), &zt_cmd, &zt_f1, &zt_f2, &zt_f3);
        });

        let jump_host_ids = Rc::new(RefCell::new(Vec::new()));

        Self {
            page,
            state,
            name_row,
            host_row,
            port_row,
            username_row,
            domain_row,
            jump_host_row,
            device_row,
            baud_row,
            k8s_context_row,
            k8s_namespace_row,
            k8s_pod_row,
            k8s_container_row,
            zt_provider_row,
            zt_command_row,
            zt_field1_row,
            zt_field2_row,
            zt_field3_row,
            url_row,
            info_label,
            connection_group,
            serial_group,
            k8s_group,
            zt_group,
            web_group,
            templates_group,
            templates_flow,
            next_button,
            on_next,
            on_advanced,
            jump_host_ids,
            current_protocol,
        }
    }

    /// Configure the page for a specific protocol
    pub fn configure_for_protocol(&self, protocol: ProtocolType) {
        *self.current_protocol.borrow_mut() = Some(protocol);

        // Hide all groups
        self.connection_group.set_visible(false);
        self.serial_group.set_visible(false);
        self.k8s_group.set_visible(false);
        self.zt_group.set_visible(false);
        self.web_group.set_visible(false);
        self.templates_group.set_visible(false);
        self.info_label.set_visible(false);
        self.username_row.set_visible(false);
        self.domain_row.set_visible(false);
        self.jump_host_row.set_visible(false);

        match protocol {
            ProtocolType::Ssh | ProtocolType::Mosh | ProtocolType::Sftp => {
                self.connection_group.set_visible(true);
                self.username_row.set_visible(true);
                self.jump_host_row.set_visible(true);
                self.port_row.set_value(22.0);
                if protocol == ProtocolType::Mosh {
                    self.info_label.set_visible(true);
                    self.info_label
                        .set_label(&i18n("Uses SSH for handshake, then switches to UDP"));
                } else if protocol == ProtocolType::Sftp {
                    self.info_label.set_visible(true);
                    self.info_label
                        .set_label(&i18n("File browser over SSH connection"));
                }
                self.populate_jump_hosts();
            }
            ProtocolType::Rdp => {
                self.connection_group.set_visible(true);
                self.username_row.set_visible(true);
                self.domain_row.set_visible(true);
                self.jump_host_row.set_visible(true);
                self.port_row.set_value(3389.0);
                self.populate_jump_hosts();
            }
            ProtocolType::Vnc | ProtocolType::Spice => {
                self.connection_group.set_visible(true);
                self.jump_host_row.set_visible(true);
                self.port_row.set_value(5900.0);
                self.populate_jump_hosts();
            }
            ProtocolType::Telnet => {
                self.connection_group.set_visible(true);
                self.port_row.set_value(23.0);
            }
            ProtocolType::Serial => {
                self.connection_group.set_visible(true);
                self.host_row.set_visible(false);
                self.port_row.set_visible(false);
                self.serial_group.set_visible(true);
                self.next_button.set_sensitive(true);
            }
            ProtocolType::Kubernetes => {
                self.connection_group.set_visible(true);
                self.host_row.set_visible(false);
                self.port_row.set_visible(false);
                self.k8s_group.set_visible(true);
                self.next_button.set_sensitive(true);
            }
            ProtocolType::ZeroTrust => {
                self.connection_group.set_visible(true);
                self.host_row.set_visible(false);
                self.port_row.set_visible(false);
                self.zt_group.set_visible(true);
                self.next_button.set_sensitive(true);
                Self::update_zt_fields(
                    0,
                    &self.zt_command_row,
                    &self.zt_field1_row,
                    &self.zt_field2_row,
                    &self.zt_field3_row,
                );
            }
            ProtocolType::Web => {
                self.connection_group.set_visible(true);
                self.host_row.set_visible(false);
                self.port_row.set_visible(false);
                self.web_group.set_visible(true);
                // Validate current URL value
                let url = self.url_row.text();
                let url_trimmed = url.trim();
                let valid = url_trimmed
                    .strip_prefix("https://")
                    .or_else(|| url_trimmed.strip_prefix("http://"))
                    .is_some_and(|rest| !rest.is_empty());
                self.next_button.set_sensitive(valid);
                if valid || url_trimmed.is_empty() {
                    self.url_row.remove_css_class("error");
                } else {
                    self.url_row.add_css_class("error");
                }
            }
        }

        let title = i18n_f("{} Connection", &[&protocol.to_string()]);
        self.page.set_title(&title);
    }

    /// Configure for "Custom Command" shortcut — shows Name + Command fields
    /// without the provider dropdown. Called after `configure_for_protocol(ZeroTrust)`.
    pub fn set_custom_command_mode(&self) {
        // Show Name field from connection group
        self.connection_group.set_visible(true);
        self.host_row.set_visible(false);
        self.port_row.set_visible(false);
        self.username_row.set_visible(false);
        self.domain_row.set_visible(false);
        self.jump_host_row.set_visible(false);
        // ZT group: only command row, rename to "Custom Command"
        self.zt_group.set_title(&i18n("Custom Command"));
        self.zt_group.set_description(None);
        self.zt_provider_row.set_visible(false);
        self.zt_command_row.set_visible(true);
        self.zt_field1_row.set_visible(false);
        self.zt_field2_row.set_visible(false);
        self.zt_field3_row.set_visible(false);
        // Show templates grid
        self.templates_group.set_visible(true);
        self.page.set_title(&i18n("Custom Command"));
    }

    fn populate_jump_hosts(&self) {
        let state_ref = self.state.borrow();
        let mut ids: Vec<Option<Uuid>> = vec![None];
        let mut names: Vec<String> = vec![i18n("(None)")];

        let mut ssh_conns: Vec<_> = state_ref
            .list_connections()
            .iter()
            .filter(|c| c.protocol == ProtocolType::Ssh)
            .cloned()
            .collect();
        ssh_conns.sort_by_key(|c| c.name.to_lowercase());

        for conn in &ssh_conns {
            ids.push(Some(conn.id));
            names.push(format!("{} ({})", conn.name, conn.host));
        }
        drop(state_ref);

        let strings: Vec<&str> = names.iter().map(String::as_str).collect();
        let model = StringList::new(&strings);
        self.jump_host_row.set_model(Some(&model));
        self.jump_host_row.set_selected(0);
        *self.jump_host_ids.borrow_mut() = ids;
    }

    fn update_zt_fields(
        idx: u32,
        command_row: &adw::EntryRow,
        field1: &adw::EntryRow,
        field2: &adw::EntryRow,
        field3: &adw::EntryRow,
    ) {
        command_row.set_visible(false);
        field1.set_visible(false);
        field2.set_visible(false);
        field3.set_visible(false);

        match idx {
            0 => command_row.set_visible(true),
            1 => {
                field1.set_visible(true);
                field1.set_title(&i18n("Target ID"));
                field2.set_visible(true);
                field2.set_title(&i18n("Region"));
                field3.set_visible(true);
                field3.set_title(&i18n("Profile"));
            }
            2 => {
                field1.set_visible(true);
                field1.set_title(&i18n("Instance"));
                field2.set_visible(true);
                field2.set_title(&i18n("Zone"));
                field3.set_visible(true);
                field3.set_title(&i18n("Project"));
            }
            3 => {
                field1.set_visible(true);
                field1.set_title(&i18n("Resource ID"));
                field2.set_visible(true);
                field2.set_title(&i18n("Resource Group"));
                field3.set_visible(true);
                field3.set_title(&i18n("Bastion Name"));
            }
            4 => {
                field1.set_visible(true);
                field1.set_title(&i18n("VM Name"));
                field2.set_visible(true);
                field2.set_title(&i18n("Resource Group"));
            }
            5 => {
                field1.set_visible(true);
                field1.set_title(&i18n("Hostname"));
            }
            6 => {
                field1.set_visible(true);
                field1.set_title(&i18n("Host"));
                field2.set_visible(true);
                field2.set_title(&i18n("Cluster"));
            }
            7 => {
                field1.set_visible(true);
                field1.set_title(&i18n("Host"));
            }
            8 => {
                field1.set_visible(true);
                field1.set_title(&i18n("Target ID"));
                field2.set_visible(true);
                field2.set_title(&i18n("Address"));
            }
            9 => {
                field1.set_visible(true);
                field1.set_title(&i18n("Connection Name"));
                field2.set_visible(true);
                field2.set_title(&i18n("Gateway URL"));
            }
            _ => {}
        }
    }

    #[must_use]
    pub fn selected_jump_host(&self) -> Option<Uuid> {
        let idx = self.jump_host_row.selected() as usize;
        self.jump_host_ids.borrow().get(idx).copied().flatten()
    }

    #[must_use]
    pub fn host(&self) -> String {
        self.host_row.text().trim().to_string()
    }

    #[must_use]
    pub fn port(&self) -> u16 {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "value range fits the target type and is non-negative by construction in this code path"
        )]
        let p = self.port_row.value() as u16;
        p
    }

    #[must_use]
    pub fn name(&self) -> String {
        self.name_row.text().trim().to_string()
    }

    #[must_use]
    pub fn username(&self) -> Option<String> {
        let t = self.username_row.text().trim().to_string();
        if t.is_empty() { None } else { Some(t) }
    }

    #[must_use]
    pub fn domain(&self) -> Option<String> {
        let t = self.domain_row.text().trim().to_string();
        if t.is_empty() { None } else { Some(t) }
    }

    #[must_use]
    pub fn serial_device(&self) -> String {
        self.device_row.text().trim().to_string()
    }

    #[must_use]
    pub fn serial_baud(&self) -> u32 {
        const BAUD_RATES: [u32; 7] = [9600, 19_200, 38_400, 57_600, 115_200, 230_400, 460_800];
        BAUD_RATES
            .get(self.baud_row.selected() as usize)
            .copied()
            .unwrap_or(115_200)
    }

    #[must_use]
    pub fn k8s_context(&self) -> Option<String> {
        let t = self.k8s_context_row.text().trim().to_string();
        if t.is_empty() { None } else { Some(t) }
    }

    #[must_use]
    pub fn k8s_namespace(&self) -> String {
        let t = self.k8s_namespace_row.text().trim().to_string();
        if t.is_empty() {
            "default".to_string()
        } else {
            t
        }
    }

    #[must_use]
    pub fn k8s_pod(&self) -> String {
        self.k8s_pod_row.text().trim().to_string()
    }

    #[must_use]
    pub fn k8s_container(&self) -> Option<String> {
        let t = self.k8s_container_row.text().trim().to_string();
        if t.is_empty() { None } else { Some(t) }
    }

    #[must_use]
    pub fn zt_provider_index(&self) -> u32 {
        self.zt_provider_row.selected()
    }

    #[must_use]
    pub fn zt_command(&self) -> Option<String> {
        let t = self.zt_command_row.text().trim().to_string();
        if t.is_empty() { None } else { Some(t) }
    }

    #[must_use]
    pub fn zt_fields(&self) -> (Option<String>, Option<String>, Option<String>) {
        let f = |row: &adw::EntryRow| -> Option<String> {
            let t = row.text().trim().to_string();
            if t.is_empty() { None } else { Some(t) }
        };
        (
            f(&self.zt_field1_row),
            f(&self.zt_field2_row),
            f(&self.zt_field3_row),
        )
    }

    #[must_use]
    pub fn url(&self) -> String {
        self.url_row.text().trim().to_string()
    }

    pub fn connect_next<F: Fn() + 'static>(&self, f: F) {
        *self.on_next.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_advanced<F: Fn() + 'static>(&self, f: F) {
        *self.on_advanced.borrow_mut() = Some(Box::new(f));
    }

    /// Creates a button for a user-defined template (from Manage Templates)
    fn create_user_template_button(icon: &str, name: &str) -> Button {
        let vbox = GtkBox::new(Orientation::Vertical, 4);
        vbox.set_halign(gtk4::Align::Center);
        vbox.set_margin_top(12);
        vbox.set_margin_bottom(12);

        if !icon.is_empty()
            && icon.chars().count() <= 2
            && icon.chars().next().is_some_and(|c| !c.is_ascii())
        {
            // Emoji icon
            let emoji_label = gtk4::Label::builder()
                .label(icon)
                .css_classes(["title-2"])
                .build();
            vbox.append(&emoji_label);
        } else if !icon.is_empty() {
            // GTK icon name
            let img = gtk4::Image::from_icon_name(icon);
            img.set_pixel_size(24);
            vbox.append(&img);
        } else {
            // Fallback: system-run
            let img = gtk4::Image::from_icon_name("system-run-symbolic");
            img.set_pixel_size(24);
            vbox.append(&img);
        }

        let name_label = gtk4::Label::builder()
            .label(name)
            .css_classes(["caption"])
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .max_width_chars(12)
            .build();
        vbox.append(&name_label);

        Button::builder()
            .child(&vbox)
            .css_classes(["flat"])
            .tooltip_text(name)
            .width_request(90)
            .height_request(70)
            .build()
    }

    /// Creates a template button with emoji icon + name for the flow grid
    fn create_template_button(predefined: &rustconn_core::PredefinedTemplate) -> Button {
        let vbox = GtkBox::new(Orientation::Vertical, 4);
        vbox.set_halign(gtk4::Align::Center);
        vbox.set_margin_top(12);
        vbox.set_margin_bottom(12);

        let emoji_label = gtk4::Label::builder()
            .label(predefined.icon)
            .css_classes(["title-2"])
            .build();
        vbox.append(&emoji_label);

        let name_label = gtk4::Label::builder()
            .label(predefined.name)
            .css_classes(["caption"])
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .max_width_chars(12)
            .build();
        vbox.append(&name_label);

        let btn = Button::builder()
            .child(&vbox)
            .css_classes(["flat"])
            .tooltip_text(&i18n(predefined.description))
            .width_request(90)
            .height_request(70)
            .build();
        btn.update_property(&[gtk4::accessible::Property::Label(&i18n(
            predefined.description,
        ))]);
        btn
    }

    /// Creates the "More…" button for the flow grid — shows all predefined templates
    fn create_more_templates_button() -> Button {
        let vbox = GtkBox::new(Orientation::Vertical, 4);
        vbox.set_halign(gtk4::Align::Center);
        vbox.set_margin_top(12);
        vbox.set_margin_bottom(12);

        let icon = gtk4::Image::from_icon_name("view-more-symbolic");
        icon.set_pixel_size(24);
        vbox.append(&icon);

        let label = gtk4::Label::builder()
            .label(&i18n("More\u{2026}"))
            .css_classes(["caption"])
            .build();
        vbox.append(&label);

        let btn = Button::builder()
            .child(&vbox)
            .css_classes(["flat"])
            .tooltip_text(&i18n("Browse all predefined templates"))
            .width_request(90)
            .height_request(70)
            .build();
        btn.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Browse all predefined templates",
        ))]);
        btn
    }

    /// Returns the template icon stored by a template button click, if any.
    /// Format: "tpl-icon:🐳" stored in name_row widget_name.
    #[must_use]
    pub fn selected_template_icon(&self) -> Option<String> {
        let wname = self.name_row.widget_name();
        wname
            .strip_prefix("tpl-icon:")
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
    }

    /// Shows a popover with all predefined templates grouped by category
    fn show_all_templates_popover(
        parent_btn: &Button,
        zt_cmd_row: &adw::EntryRow,
        name_row: &adw::EntryRow,
        next_btn: &Button,
        state: &SharedAppState,
    ) {
        let popover = gtk4::Popover::new();
        popover.set_parent(parent_btn);

        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .max_content_height(350)
            .propagate_natural_height(true)
            .build();

        let content = GtkBox::new(Orientation::Vertical, 8);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // === User templates section ===
        let state_ref = state.borrow();
        let user_zt_templates: Vec<_> = state_ref
            .get_all_templates()
            .into_iter()
            .filter(|t| t.protocol == ProtocolType::ZeroTrust)
            .collect();
        drop(state_ref);

        if !user_zt_templates.is_empty() {
            let user_header = gtk4::Label::builder()
                .label(&i18n("Your Templates"))
                .halign(gtk4::Align::Start)
                .css_classes(["heading"])
                .margin_top(4)
                .build();
            content.append(&user_header);

            let user_flow = gtk4::FlowBox::builder()
                .homogeneous(true)
                .min_children_per_line(3)
                .max_children_per_line(5)
                .selection_mode(gtk4::SelectionMode::None)
                .activate_on_single_click(false)
                .row_spacing(4)
                .column_spacing(4)
                .build();

            for user_tpl in &user_zt_templates {
                let icon_str = user_tpl.icon.clone().unwrap_or_default();
                let tpl_name = user_tpl.name.clone();
                let cmd_str = if let rustconn_core::models::ProtocolConfig::ZeroTrust(ref zt) =
                    user_tpl.protocol_config
                {
                    if let rustconn_core::models::ZeroTrustProviderConfig::Generic(ref g) =
                        zt.provider_config
                    {
                        g.command_template.clone()
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };

                if cmd_str.is_empty() {
                    continue;
                }

                let btn = Self::create_user_template_button(&icon_str, &tpl_name);
                let zt_clone = zt_cmd_row.clone();
                let name_clone = name_row.clone();
                let next_clone = next_btn.clone();
                let popover_clone = popover.clone();
                let icon_for_cb = icon_str.clone();
                let name_for_cb = tpl_name.clone();
                btn.connect_clicked(move |_| {
                    zt_clone.set_text(&cmd_str);
                    if name_clone.text().is_empty() {
                        name_clone.set_text(&name_for_cb);
                    }
                    if !icon_for_cb.is_empty() {
                        name_clone.set_widget_name(&format!("tpl-icon:{}", icon_for_cb));
                    }
                    next_clone.set_sensitive(true);
                    popover_clone.popdown();
                });
                user_flow.append(&btn);
            }

            content.append(&user_flow);
        }

        // === Predefined templates by category ===
        for category in rustconn_core::TemplateCategory::all() {
            let templates = rustconn_core::templates_by_category(*category);
            if templates.is_empty() {
                continue;
            }

            // Category header
            let header = gtk4::Label::builder()
                .label(&i18n(category.display_name()))
                .halign(gtk4::Align::Start)
                .css_classes(["heading"])
                .margin_top(4)
                .build();
            content.append(&header);

            // Template buttons in a flow
            let flow = gtk4::FlowBox::builder()
                .homogeneous(true)
                .min_children_per_line(3)
                .max_children_per_line(5)
                .selection_mode(gtk4::SelectionMode::None)
                .activate_on_single_click(false)
                .row_spacing(4)
                .column_spacing(4)
                .build();

            for tpl in &templates {
                let btn = Self::create_template_button(tpl);
                let cmd = tpl.command;
                let tpl_name = tpl.name;
                let tpl_icon = tpl.icon;
                let zt_clone = zt_cmd_row.clone();
                let name_clone = name_row.clone();
                let next_clone = next_btn.clone();
                let popover_clone = popover.clone();
                btn.connect_clicked(move |_| {
                    zt_clone.set_text(cmd);
                    if name_clone.text().is_empty() {
                        name_clone.set_text(tpl_name);
                    }
                    name_clone.set_widget_name(&format!("tpl-icon:{tpl_icon}"));
                    next_clone.set_sensitive(true);
                    popover_clone.popdown();
                });
                flow.append(&btn);
            }

            content.append(&flow);
        }

        scrolled.set_child(Some(&content));
        popover.set_child(Some(&scrolled));
        popover.popup();
    }
}
