//! Wake On LAN dialog
//!
//! Standalone dialog for sending WoL magic packets. Accessible from
//! the Tools menu. Allows picking a connection with WoL configured
//! or entering MAC address manually.

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, Orientation};
use libadwaita as adw;
use rustconn_core::models::Connection;
use rustconn_core::wol::{MacAddress, WolConfig};
use std::cell::RefCell;
use std::rc::Rc;

/// Standalone Wake On LAN dialog
pub struct WolDialog {
    window: adw::Window,
    connection_dropdown: adw::ComboRow,
    connections: Rc<RefCell<Vec<Connection>>>,
}

impl WolDialog {
    /// Creates a new WoL dialog
    #[must_use]
    pub fn new(parent: Option<&gtk4::Window>) -> Self {
        let window = adw::Window::builder()
            .title("Wake On LAN")
            .modal(true)
            .default_width(500)
            .default_height(420)
            .build();

        if let Some(p) = parent {
            window.set_transient_for(Some(p));
        }

        let header = adw::HeaderBar::new();
        header.set_show_end_title_buttons(false);
        header.set_show_start_title_buttons(false);

        let cancel_btn = Button::builder().label("Cancel").build();
        let send_btn = Button::builder()
            .label("Send")
            .css_classes(["suggested-action"])
            .build();
        header.pack_start(&cancel_btn);
        header.pack_end(&send_btn);

        let content = GtkBox::new(Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // Connection picker
        let conn_group = adw::PreferencesGroup::builder()
            .title("Connection")
            .description("Pick a connection with WoL configured")
            .build();

        let string_list = gtk4::StringList::new(&["Manual entry"]);
        let connection_dropdown = adw::ComboRow::builder()
            .title("Connection")
            .model(&string_list)
            .build();
        conn_group.add(&connection_dropdown);
        content.append(&conn_group);

        // Manual entry fields
        let manual_group = adw::PreferencesGroup::builder()
            .title("Manual")
            .description("Or enter MAC address manually")
            .build();

        let mac_entry = adw::EntryRow::builder().title("MAC Address").build();
        mac_entry.set_text("AA:BB:CC:DD:EE:FF");
        manual_group.add(&mac_entry);

        let broadcast_entry = adw::EntryRow::builder().title("Broadcast Address").build();
        broadcast_entry.set_text(rustconn_core::wol::DEFAULT_BROADCAST_ADDRESS);
        manual_group.add(&broadcast_entry);

        let port_entry = adw::EntryRow::builder().title("Port").build();
        port_entry.set_text(&rustconn_core::wol::DEFAULT_WOL_PORT.to_string());
        manual_group.add(&port_entry);

        content.append(&manual_group);

        // Status label
        let status_label = gtk4::Label::new(None);
        status_label.set_halign(gtk4::Align::Start);
        status_label.add_css_class("dim-label");
        content.append(&status_label);

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&content));
        window.set_content(Some(&toolbar_view));

        let connections: Rc<RefCell<Vec<Connection>>> = Rc::new(RefCell::new(Vec::new()));

        // Dropdown selection → populate fields
        let mac_c = mac_entry.clone();
        let broadcast_c = broadcast_entry.clone();
        let port_c = port_entry.clone();
        let conns_c = connections.clone();
        connection_dropdown.connect_selected_notify(move |row| {
            let idx = row.selected();
            if idx == 0 {
                return; // "Manual entry"
            }
            let conns = conns_c.borrow();
            if let Some(conn) = conns.get((idx - 1) as usize) {
                if let Some(wol) = conn.get_wol_config() {
                    mac_c.set_text(&wol.mac_address.to_string());
                    broadcast_c.set_text(&wol.broadcast_address);
                    port_c.set_text(&wol.port.to_string());
                }
            }
        });

        // Cancel
        let window_c = window.clone();
        cancel_btn.connect_clicked(move |_| {
            window_c.close();
        });

        // Send
        let mac_e = mac_entry;
        let broadcast_e = broadcast_entry;
        let port_e = port_entry;
        let status_c = status_label;
        send_btn.connect_clicked(move |_| {
            let mac_text = mac_e.text();
            let broadcast = broadcast_e.text();
            let port_text = port_e.text();

            let mac = if let Ok(m) = MacAddress::parse(&mac_text) {
                m
            } else {
                status_c.set_text("Invalid MAC address format");
                status_c.remove_css_class("success");
                status_c.add_css_class("error");
                return;
            };

            let port: u16 = if let Ok(p) = port_text.parse() {
                p
            } else {
                status_c.set_text("Invalid port number");
                status_c.remove_css_class("success");
                status_c.add_css_class("error");
                return;
            };

            let config = WolConfig::new(mac)
                .with_broadcast_address(broadcast.as_str())
                .with_port(port);

            let mac_display = mac_text.to_string();
            let broadcast_display = broadcast.to_string();
            let status_ok = status_c.clone();
            let status_err = status_c.clone();
            status_c.set_text("Sending…");
            status_c.remove_css_class("error");
            status_c.remove_css_class("success");

            crate::utils::spawn_blocking_with_callback(
                move || rustconn_core::wol::send_wol_with_retry(&config, 3, 500),
                move |result| match result {
                    Ok(()) => {
                        tracing::info!(
                            mac = %mac_display,
                            broadcast = %broadcast_display,
                            port,
                            "WoL packet sent from dialog",
                        );
                        status_ok.set_text(&format!("Magic packet sent to {mac_display}"));
                        status_ok.remove_css_class("error");
                        status_ok.add_css_class("success");
                    }
                    Err(e) => {
                        tracing::error!(?e, "WoL send failed from dialog");
                        status_err.set_text(
                            "Failed to send packet. \
                             Check permissions.",
                        );
                        status_err.remove_css_class("success");
                        status_err.add_css_class("error");
                    }
                },
            );
        });

        Self {
            window,
            connection_dropdown,
            connections,
        }
    }

    /// Populates dropdown with connections that have WoL configured
    pub fn set_connections(&self, connections: &[Connection]) {
        let wol_connections: Vec<Connection> = connections
            .iter()
            .filter(|c| c.has_wol_config())
            .cloned()
            .collect();

        let mut items: Vec<String> = vec!["Manual entry".to_string()];
        for conn in &wol_connections {
            items.push(conn.name.clone());
        }

        let string_list =
            gtk4::StringList::new(&items.iter().map(String::as_str).collect::<Vec<_>>());
        self.connection_dropdown.set_model(Some(&string_list));

        *self.connections.borrow_mut() = wol_connections;
    }

    /// Presents the dialog
    pub fn present(&self) {
        self.window.present();
    }
}
