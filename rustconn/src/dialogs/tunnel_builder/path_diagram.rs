//! Visual tunnel path diagram widget
//!
//! Displays a horizontal chain of styled nodes representing the tunnel path:
//! `[localhost:port] → [bastion] → [target:port]`
//!
//! Each node is a `gtk4::Frame` with icon, host label, port label, and
//! status dot indicator. Arrows connect the nodes.

use gtk4::prelude::*;
use rustconn_core::models::{PortForwardDirection, TunnelStatus};

use crate::i18n::i18n;

// ---------------------------------------------------------------------------
// DiagramNode — a single node in the path diagram
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct DiagramNode {
    frame: gtk4::Frame,
    _icon: gtk4::Image,
    host_label: gtk4::Label,
    port_label: gtk4::Label,
    status_dot: gtk4::Label,
}

impl DiagramNode {
    fn new(icon_name: &str, default_host: &str) -> Self {
        let vbox = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(2)
            .halign(gtk4::Align::Center)
            .build();

        let icon = gtk4::Image::builder()
            .icon_name(icon_name)
            .pixel_size(16)
            .build();

        let host_label = gtk4::Label::builder()
            .label(default_host)
            .css_classes(["caption"])
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .max_width_chars(16)
            .build();

        let port_label = gtk4::Label::builder()
            .label("")
            .css_classes(["caption", "dim-label"])
            .visible(false)
            .build();

        let status_dot = gtk4::Label::builder()
            .label("●")
            .css_classes(["tunnel-status-dot"])
            .visible(false)
            .build();

        vbox.append(&icon);
        vbox.append(&host_label);
        vbox.append(&port_label);
        vbox.append(&status_dot);

        let frame = gtk4::Frame::builder()
            .child(&vbox)
            .css_classes(["tunnel-node"])
            .build();

        Self {
            frame,
            _icon: icon,
            host_label,
            port_label,
            status_dot,
        }
    }

    fn set_host(&self, host: &str) {
        self.host_label.set_label(host);
    }

    fn set_port(&self, port: Option<u16>) {
        if let Some(p) = port {
            self.port_label.set_label(&format!(":{p}"));
            self.port_label.set_visible(true);
        } else {
            self.port_label.set_label("");
            self.port_label.set_visible(false);
        }
    }

    fn set_visible(&self, visible: bool) {
        self.frame.set_visible(visible);
    }

    fn clear_status_classes(&self) {
        self.frame.remove_css_class("success");
        self.frame.remove_css_class("warning");
        self.frame.remove_css_class("error");
        self.frame.remove_css_class("starting");
    }

    fn show_status_dot(&self, visible: bool) {
        self.status_dot.set_visible(visible);
    }
}

// ---------------------------------------------------------------------------
// TunnelPathDiagram — the full horizontal diagram widget
// ---------------------------------------------------------------------------

/// Visual diagram showing the tunnel path: localhost → bastion → target
///
/// Embeddable in any container via `widget()`. Call `update()` to refresh
/// the displayed hosts/ports, and `set_status()` to show status indicators.
#[derive(Clone)]
pub struct TunnelPathDiagram {
    container: gtk4::Box,
    nodes: Vec<DiagramNode>,
    arrows: Vec<gtk4::Label>,
}

impl TunnelPathDiagram {
    /// Creates a new empty tunnel path diagram
    #[must_use]
    pub fn new() -> Self {
        let container = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(0)
            .halign(gtk4::Align::Center)
            .valign(gtk4::Align::Center)
            .css_classes(["tunnel-diagram"])
            .build();

        // Create three nodes: localhost, bastion, target
        let localhost_node = DiagramNode::new("computer-symbolic", &i18n("Localhost"));
        let bastion_node = DiagramNode::new("channel-secure-symbolic", &i18n("Bastion"));
        let target_node = DiagramNode::new("network-server-symbolic", &i18n("Target"));

        // Bastion is hidden by default (shown only when configured)
        bastion_node.set_visible(false);

        // Create arrow labels
        let first_arrow = gtk4::Label::builder()
            .label("→")
            .css_classes(["tunnel-arrow"])
            .build();
        // Mark arrows as decorative for accessibility
        first_arrow.update_property(&[gtk4::accessible::Property::Label("")]);

        let second_arrow = gtk4::Label::builder()
            .label("→")
            .css_classes(["tunnel-arrow"])
            .visible(false)
            .build();
        second_arrow.update_property(&[gtk4::accessible::Property::Label("")]);

        // Assemble: localhost → (bastion →) target
        container.append(&localhost_node.frame);
        container.append(&first_arrow);
        container.append(&bastion_node.frame);
        container.append(&second_arrow);
        container.append(&target_node.frame);

        // Set accessible role for the container (image-like diagram)
        container.set_accessible_role(gtk4::AccessibleRole::Img);
        container.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Tunnel path diagram",
        ))]);

        let nodes = vec![localhost_node, bastion_node, target_node];
        let arrows = vec![first_arrow, second_arrow];

        Self {
            container,
            nodes,
            arrows,
        }
    }

    /// Returns the root widget for embedding in a container
    #[must_use]
    pub fn widget(&self) -> &gtk4::Widget {
        self.container.upcast_ref()
    }

    /// Updates the diagram with current tunnel configuration
    ///
    /// - `local_port`: the local port being forwarded
    /// - `bastion`: optional bastion/jump host name
    /// - `target_host`: the remote target host
    /// - `target_port`: the remote target port
    /// - `direction`: the port forwarding direction (affects label display)
    pub fn update(
        &self,
        local_port: Option<u16>,
        bastion: Option<&str>,
        target_host: Option<&str>,
        target_port: Option<u16>,
        direction: Option<PortForwardDirection>,
    ) {
        // Update localhost node
        self.nodes[0].set_host(&i18n("Localhost"));
        self.nodes[0].set_port(local_port);

        // Update bastion node visibility and content
        let has_bastion = bastion.is_some();
        self.nodes[1].set_visible(has_bastion);
        self.arrows[1].set_visible(has_bastion);

        if let Some(bastion_host) = bastion {
            self.nodes[1].set_host(bastion_host);
            self.nodes[1].set_port(None);
        }

        // Update target node based on direction
        if direction == Some(PortForwardDirection::Dynamic) {
            self.nodes[2].set_host(&i18n("SOCKS proxy"));
            self.nodes[2].set_port(None);
        } else {
            let target_fallback = i18n("Target");
            let host = target_host.unwrap_or(&target_fallback);
            self.nodes[2].set_host(host);
            self.nodes[2].set_port(target_port);
        }

        // Update accessible description
        let desc = self.accessible_description();
        self.container
            .update_property(&[gtk4::accessible::Property::Label(&desc)]);
    }

    /// Updates the status indicators on all nodes (edit mode)
    pub fn set_status(&self, status: &TunnelStatus) {
        // Clear previous status classes from all nodes
        for node in &self.nodes {
            node.clear_status_classes();
            node.show_status_dot(true);
            node.frame.set_sensitive(true);
        }

        match status {
            TunnelStatus::Running => {
                for node in &self.nodes {
                    node.frame.add_css_class("success");
                    node.status_dot.set_label("●");
                    node.status_dot.remove_css_class("error");
                    node.status_dot.remove_css_class("warning");
                    node.status_dot.add_css_class("success");
                }
                // Activate arrows
                for arrow in &self.arrows {
                    arrow.add_css_class("active");
                }
            }
            TunnelStatus::Starting => {
                for node in &self.nodes {
                    node.frame.add_css_class("warning");
                    node.frame.add_css_class("starting");
                    node.status_dot.set_label("●");
                    node.status_dot.remove_css_class("success");
                    node.status_dot.remove_css_class("error");
                    node.status_dot.add_css_class("warning");
                }
                for arrow in &self.arrows {
                    arrow.remove_css_class("active");
                }
            }
            TunnelStatus::Failed(msg) => {
                for node in &self.nodes {
                    node.frame.add_css_class("error");
                    node.status_dot.set_label("●");
                    node.status_dot.remove_css_class("success");
                    node.status_dot.remove_css_class("warning");
                    node.status_dot.add_css_class("error");
                }
                // Set tooltip with error message (truncated to 200 chars, UTF-8 safe)
                let truncated = if msg.chars().count() > 200 {
                    let s: String = msg.chars().take(200).collect();
                    format!("{s}…")
                } else {
                    msg.clone()
                };
                self.container.set_tooltip_text(Some(&truncated));
                for arrow in &self.arrows {
                    arrow.remove_css_class("active");
                }
            }
            TunnelStatus::Stopped => {
                for node in &self.nodes {
                    node.frame.set_sensitive(false);
                    node.status_dot.set_label("●");
                    node.status_dot.remove_css_class("success");
                    node.status_dot.remove_css_class("error");
                    node.status_dot.remove_css_class("warning");
                }
                for arrow in &self.arrows {
                    arrow.remove_css_class("active");
                }
            }
        }

        // Announce status change for assistive technologies (combine path + status)
        let path_desc = self.accessible_description();
        let status_text = match status {
            TunnelStatus::Running => i18n("Status: Running"),
            TunnelStatus::Starting => i18n("Status: Starting"),
            TunnelStatus::Failed(_) => i18n("Status: Failed"),
            TunnelStatus::Stopped => i18n("Status: Stopped"),
        };
        let combined = format!("{path_desc}. {status_text}");
        self.container
            .update_property(&[gtk4::accessible::Property::Label(&combined)]);
    }

    /// Hides all status indicators (used in create mode)
    pub fn hide_status(&self) {
        for node in &self.nodes {
            node.clear_status_classes();
            node.show_status_dot(false);
            node.frame.set_sensitive(true);
        }
        for arrow in &self.arrows {
            arrow.remove_css_class("active");
        }
        self.container.set_tooltip_text(None::<&str>);
    }

    /// Returns accessible description text for the current diagram state
    #[must_use]
    pub fn accessible_description(&self) -> String {
        let mut parts = Vec::new();

        // Localhost node
        let localhost_host = self.nodes[0].host_label.label();
        let localhost_port = self.nodes[0].port_label.label();
        if localhost_port.is_empty() {
            parts.push(localhost_host.to_string());
        } else {
            parts.push(format!("{localhost_host}{localhost_port}"));
        }

        // Bastion node (only if visible)
        if self.nodes[1].frame.is_visible() {
            let bastion_host = self.nodes[1].host_label.label();
            parts.push(bastion_host.to_string());
        }

        // Target node
        let target_host = self.nodes[2].host_label.label();
        let target_port = self.nodes[2].port_label.label();
        if target_port.is_empty() {
            parts.push(target_host.to_string());
        } else {
            parts.push(format!("{target_host}{target_port}"));
        }

        i18n("Tunnel") + ": " + &parts.join(" → ")
    }
}

impl Default for TunnelPathDiagram {
    fn default() -> Self {
        Self::new()
    }
}
