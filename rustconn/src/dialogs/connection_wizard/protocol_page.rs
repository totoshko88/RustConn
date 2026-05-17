//! Step 1: Protocol selection page
//!
//! Displays protocols in a 4-column grid layout with group headers.
//! Columns: Secure Shell | Remote Desktop | Terminal | Other
//! Clicking a protocol advances to Step 2.

use crate::i18n::{i18n, i18n_f};
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, Image, Label, Orientation};
use libadwaita as adw;
use rustconn_core::models::ProtocolType;
use std::cell::RefCell;
use std::rc::Rc;

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
    icon: &'static str,
}

impl ProtocolPage {
    /// Creates the protocol selection page with 4-column grid layout
    #[must_use]
    pub fn new() -> Self {
        let on_protocol_selected: Rc<RefCell<Option<Box<dyn Fn(ProtocolType, bool)>>>> =
            Rc::new(RefCell::new(None));
        let on_advanced: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));

        let content = GtkBox::new(Orientation::Vertical, 16);
        content.set_margin_top(16);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        let clamp = adw::Clamp::builder()
            .maximum_size(480)
            .child(&content)
            .build();

        // === Column definitions ===
        // Each column: [header_label, protocol_buttons...]
        let col_ssh = vec![
            ProtocolDef {
                protocol: ProtocolType::Ssh,
                label: "SSH",
                icon: "utilities-terminal-symbolic",
            },
            ProtocolDef {
                protocol: ProtocolType::Sftp,
                label: "SFTP",
                icon: "folder-remote-symbolic",
            },
            ProtocolDef {
                protocol: ProtocolType::Mosh,
                label: "MOSH",
                icon: "network-cellular-signal-excellent-symbolic",
            },
        ];

        let col_desktop = vec![
            ProtocolDef {
                protocol: ProtocolType::Rdp,
                label: "RDP",
                icon: "computer-symbolic",
            },
            ProtocolDef {
                protocol: ProtocolType::Vnc,
                label: "VNC",
                icon: "preferences-desktop-remote-desktop-symbolic",
            },
            ProtocolDef {
                protocol: ProtocolType::Spice,
                label: "SPICE",
                icon: "video-display-symbolic",
            },
        ];

        let col_terminal = vec![
            ProtocolDef {
                protocol: ProtocolType::Telnet,
                label: "Telnet",
                icon: "network-wired-symbolic",
            },
            ProtocolDef {
                protocol: ProtocolType::Serial,
                label: "Serial",
                icon: "media-removable-symbolic",
            },
            ProtocolDef {
                protocol: ProtocolType::ZeroTrust,
                label: "Custom Cmd",
                icon: "system-run-symbolic",
            },
        ];

        let col_other = vec![
            ProtocolDef {
                protocol: ProtocolType::Kubernetes,
                label: "Kubernetes",
                icon: "application-x-executable-symbolic",
            },
            ProtocolDef {
                protocol: ProtocolType::ZeroTrust,
                label: "Zero Trust",
                icon: "channel-secure-symbolic",
            },
            ProtocolDef {
                protocol: ProtocolType::Web,
                label: "Web",
                icon: "web-browser-symbolic",
            },
        ];

        // Build the 4-column grid using a horizontal box of vertical columns
        let grid = GtkBox::new(Orientation::Horizontal, 8);
        grid.set_homogeneous(true);

        let columns = [
            (&i18n("Secure Shell"), &col_ssh),
            (&i18n("Remote Desktop"), &col_desktop),
            (&i18n("Terminal"), &col_terminal),
            (&i18n("Other"), &col_other),
        ];

        for (title, protocols) in &columns {
            let column = Self::create_column(title, protocols, &on_protocol_selected);
            grid.append(&column);
        }

        content.append(&grid);

        // Advanced button (sticky bottom bar)
        let footer = GtkBox::new(Orientation::Horizontal, 0);
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

        let on_advanced_clone = on_advanced.clone();
        advanced_btn.connect_clicked(move |_| {
            if let Some(ref cb) = *on_advanced_clone.borrow() {
                cb();
            }
        });

        let toolbar_view = adw::ToolbarView::new();
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
    /// The bool parameter indicates "custom command mode" (true for Custom Cmd shortcut)
    pub fn connect_protocol_selected<F: Fn(ProtocolType, bool) + 'static>(&self, f: F) {
        *self.on_protocol_selected.borrow_mut() = Some(Box::new(f));
    }

    /// Connect callback for Advanced button
    pub fn connect_advanced<F: Fn() + 'static>(&self, f: F) {
        *self.on_advanced.borrow_mut() = Some(Box::new(f));
    }

    /// Creates a single column: header label + vertical stack of protocol buttons
    fn create_column(
        title: &str,
        protocols: &[ProtocolDef],
        on_selected: &Rc<RefCell<Option<Box<dyn Fn(ProtocolType, bool)>>>>,
    ) -> GtkBox {
        let column = GtkBox::new(Orientation::Vertical, 8);
        column.set_valign(gtk4::Align::Start);

        // Column header
        let header = Label::builder()
            .label(title)
            .css_classes(["heading"])
            .halign(gtk4::Align::Center)
            .build();
        column.append(&header);

        // Protocol buttons
        for proto_def in protocols {
            let btn = Self::create_protocol_button(proto_def, on_selected);
            column.append(&btn);
        }

        column
    }

    /// Creates a single protocol button (icon + label, vertically stacked)
    fn create_protocol_button(
        proto_def: &ProtocolDef,
        on_selected: &Rc<RefCell<Option<Box<dyn Fn(ProtocolType, bool)>>>>,
    ) -> Button {
        let btn = Button::builder()
            .css_classes(["flat", "protocol-button"])
            .height_request(72)
            .build();

        let btn_content = GtkBox::new(Orientation::Vertical, 4);
        btn_content.set_valign(gtk4::Align::Center);
        btn_content.set_halign(gtk4::Align::Center);

        let icon = Image::from_icon_name(proto_def.icon);
        icon.set_pixel_size(32);
        btn_content.append(&icon);

        let label = Label::new(Some(proto_def.label));
        label.add_css_class("caption");
        btn_content.append(&label);

        btn.set_child(Some(&btn_content));

        let protocol = proto_def.protocol;
        let is_custom_cmd = proto_def.label == "Custom Cmd";
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
