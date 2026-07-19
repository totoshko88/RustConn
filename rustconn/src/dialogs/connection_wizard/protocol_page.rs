//! Step 1: Protocol selection page
//!
//! Displays protocols in a 4-column grid layout with group headers.
//! Columns: Secure Shell | Remote Desktop | Terminal | Other
//! Each button shows icon + label + subtitle for discoverability.
//! Clicking a protocol advances to Step 2.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, Image, Label, Orientation};
use libadwaita as adw;
use rustconn_core::models::ProtocolType;

use crate::i18n::{i18n, i18n_f};

/// Protocol page — Step 1 of the wizard
pub struct ProtocolPage {
    pub page: adw::NavigationPage,
    on_protocol_selected: Rc<RefCell<Option<Box<dyn Fn(ProtocolType, bool)>>>>,
    on_advanced: Rc<RefCell<Option<Box<dyn Fn()>>>>,
}

/// Protocol button definition
struct ProtocolDef {
    protocol: ProtocolType,
    label: &'static str,
    subtitle: &'static str,
    icon: &'static str,
    /// `true` for the "Custom Command" card, which shares `ProtocolType::ZeroTrust`
    /// with the "Zero Trust" card but opens the wizard in custom-command mode.
    is_custom_command: bool,
}

impl ProtocolPage {
    /// Creates the protocol selection page with true 4-column layout.
    ///
    /// Layout: 4 vertical columns side-by-side, each with a header label
    /// and 3 protocol buttons stacked vertically beneath it.
    #[must_use]
    pub fn new() -> Self {
        let on_protocol_selected: Rc<RefCell<Option<Box<dyn Fn(ProtocolType, bool)>>>> =
            Rc::new(RefCell::new(None));
        let on_advanced: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));

        let content = GtkBox::new(Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        let clamp = adw::Clamp::builder()
            .maximum_size(580)
            .child(&content)
            .build();

        // === Column definitions ===
        // 4 columns × 3 protocols each for balanced visual layout
        let col_ssh = (
            "Secure Shell",
            vec![
                ProtocolDef {
                    protocol: ProtocolType::Ssh,
                    label: "SSH",
                    subtitle: "Secure remote shell",
                    icon: "utilities-terminal-symbolic",
                    is_custom_command: false,
                },
                ProtocolDef {
                    protocol: ProtocolType::Sftp,
                    label: "SFTP",
                    subtitle: "File transfer",
                    icon: "folder-remote-symbolic",
                    is_custom_command: false,
                },
                ProtocolDef {
                    protocol: ProtocolType::Mosh,
                    label: "MOSH",
                    subtitle: "Mobile shell",
                    icon: "network-cellular-signal-excellent-symbolic",
                    is_custom_command: false,
                },
            ],
        );

        let col_desktop = (
            "Remote Desktop",
            vec![
                ProtocolDef {
                    protocol: ProtocolType::Rdp,
                    label: "RDP",
                    subtitle: "Windows desktop",
                    icon: "computer-symbolic",
                    is_custom_command: false,
                },
                ProtocolDef {
                    protocol: ProtocolType::Vnc,
                    label: "VNC",
                    subtitle: "Screen sharing",
                    icon: "preferences-desktop-remote-desktop-symbolic",
                    is_custom_command: false,
                },
                ProtocolDef {
                    protocol: ProtocolType::Spice,
                    label: "SPICE",
                    subtitle: "VM display",
                    icon: "video-display-symbolic",
                    is_custom_command: false,
                },
            ],
        );

        let col_terminal = (
            "Terminal",
            vec![
                ProtocolDef {
                    protocol: ProtocolType::Telnet,
                    label: "Telnet",
                    subtitle: "Unencrypted",
                    icon: "network-wired-symbolic",
                    is_custom_command: false,
                },
                ProtocolDef {
                    protocol: ProtocolType::Serial,
                    label: "Serial",
                    subtitle: "Console port",
                    icon: "media-removable-symbolic",
                    is_custom_command: false,
                },
                ProtocolDef {
                    protocol: ProtocolType::ZeroTrust,
                    label: "Custom Command",
                    subtitle: "Run any CLI tool",
                    icon: "system-run-symbolic",
                    is_custom_command: true,
                },
            ],
        );

        let col_other = (
            "Other",
            vec![
                ProtocolDef {
                    protocol: ProtocolType::Kubernetes,
                    label: "Kubernetes",
                    subtitle: "Pod shell",
                    icon: "application-x-executable-symbolic",
                    is_custom_command: false,
                },
                ProtocolDef {
                    protocol: ProtocolType::ZeroTrust,
                    label: "Zero Trust",
                    subtitle: "Cloud access",
                    icon: "channel-secure-symbolic",
                    is_custom_command: false,
                },
                ProtocolDef {
                    protocol: ProtocolType::Web,
                    label: "Web",
                    subtitle: "Browser URL",
                    icon: "web-browser-symbolic",
                    is_custom_command: false,
                },
            ],
        );

        // Build a true 4-column horizontal layout
        let columns_box = GtkBox::new(Orientation::Horizontal, 12);
        columns_box.set_homogeneous(true);
        columns_box.set_vexpand(true);

        let columns = [&col_ssh, &col_desktop, &col_terminal, &col_other];
        for (title, protocols) in columns {
            let column = GtkBox::new(Orientation::Vertical, 12);

            // Column header (centered)
            let header = Label::new(Some(&i18n(title)));
            header.add_css_class("heading");
            header.set_halign(gtk4::Align::Center);
            header.set_margin_bottom(6);
            column.append(&header);

            // Protocol buttons stacked vertically, expanding to fill space
            for proto_def in protocols {
                let btn = Self::create_protocol_button(proto_def, &on_protocol_selected);
                btn.set_vexpand(true);
                column.append(&btn);
            }

            columns_box.append(&column);
        }

        content.append(&columns_box);

        // Advanced button (sticky bottom bar)
        let footer = GtkBox::new(Orientation::Horizontal, 0);
        footer.set_margin_top(6);
        footer.set_margin_bottom(6);
        footer.set_margin_start(12);
        footer.set_margin_end(12);
        let advanced_btn = Button::with_label(&i18n("Advanced…"));
        advanced_btn.add_css_class("flat");
        advanced_btn.add_css_class("dim-label");
        advanced_btn.set_tooltip_text(Some(&i18n("Open full connection editor")));
        advanced_btn.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Open full connection editor",
        ))]);
        footer.append(&advanced_btn);

        let on_advanced_clone = on_advanced.clone();
        advanced_btn.connect_clicked(move |_| {
            if let Some(ref cb) = *on_advanced_clone.borrow() {
                cb();
            }
        });

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&adw::HeaderBar::new());
        toolbar_view.set_content(Some(&clamp));
        toolbar_view.add_bottom_bar(&footer);

        let page = adw::NavigationPage::builder()
            .title(i18n("New Connection"))
            .child(&toolbar_view)
            .build();

        Self {
            page,
            on_protocol_selected,
            on_advanced,
        }
    }

    /// Connect callback for protocol selection
    /// The bool parameter indicates "custom command mode" (true for Custom Command shortcut)
    pub fn connect_protocol_selected<F: Fn(ProtocolType, bool) + 'static>(&self, f: F) {
        *self.on_protocol_selected.borrow_mut() = Some(Box::new(f));
    }

    /// Connect callback for Advanced button
    pub fn connect_advanced<F: Fn() + 'static>(&self, f: F) {
        *self.on_advanced.borrow_mut() = Some(Box::new(f));
    }

    /// Creates a single protocol button (icon + label + subtitle, vertically stacked)
    fn create_protocol_button(
        proto_def: &ProtocolDef,
        on_selected: &Rc<RefCell<Option<Box<dyn Fn(ProtocolType, bool)>>>>,
    ) -> Button {
        let btn = Button::builder()
            .css_classes(["flat", "protocol-button"])
            .height_request(88)
            .build();

        let btn_content = GtkBox::new(Orientation::Vertical, 2);
        btn_content.set_valign(gtk4::Align::Center);
        btn_content.set_halign(gtk4::Align::Center);

        let icon = Image::from_icon_name(proto_def.icon);
        icon.set_pixel_size(32);
        btn_content.append(&icon);

        let label = Label::new(Some(proto_def.label));
        label.add_css_class("caption-heading");
        btn_content.append(&label);

        let subtitle = Label::new(Some(&i18n(proto_def.subtitle)));
        subtitle.add_css_class("caption");
        subtitle.add_css_class("dim-label");
        btn_content.append(&subtitle);

        btn.set_child(Some(&btn_content));

        let protocol = proto_def.protocol;
        let is_custom_cmd = proto_def.is_custom_command;
        let tooltip = i18n_f("{} connection", &[proto_def.label]);
        btn.set_tooltip_text(Some(&tooltip));
        btn.update_property(&[gtk4::accessible::Property::Label(&tooltip)]);

        let on_selected_clone = on_selected.clone();
        btn.connect_clicked(move |_| {
            if let Some(ref cb) = *on_selected_clone.borrow() {
                cb(protocol, is_custom_cmd);
            }
        });

        btn
    }
}
