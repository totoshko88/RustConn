//! Embedded session support for RDP/VNC connections
//!
//! This module provides support for embedding RDP and VNC sessions
//! within the main application window using X11 embedding (`GtkSocket`).
//! On Wayland, sessions fall back to external windows.

use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, DrawingArea, Label, Orientation};
use std::cell::RefCell;
use std::process::Child;
use std::rc::Rc;
use uuid::Uuid;

// Re-export DisplayServer from the unified display module for backward compatibility
pub use crate::display::DisplayServer;

/// Error type for embedding operations
#[derive(Debug, Clone)]
pub enum EmbeddingError {
    /// Embedding not supported on Wayland
    WaylandNotSupported { protocol: String },
    /// Failed to get window ID for embedding
    WindowIdNotAvailable,
    /// Client process failed to start
    ProcessStartFailed(String),
    /// Client exited unexpectedly
    ClientExited { code: i32 },
}

impl std::fmt::Display for EmbeddingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WaylandNotSupported { protocol } => {
                write!(f, "Embedding not supported on Wayland for {protocol}")
            }
            Self::WindowIdNotAvailable => write!(f, "Failed to get window ID for embedding"),
            Self::ProcessStartFailed(msg) => write!(f, "Failed to start client process: {msg}"),
            Self::ClientExited { code } => write!(f, "Client exited with code {code}"),
        }
    }
}

impl std::error::Error for EmbeddingError {}

/// Session controls for embedded sessions
#[derive(Clone)]
pub struct SessionControls {
    container: GtkBox,
    fullscreen_button: Button,
    disconnect_button: Button,
    status_label: Label,
}

impl SessionControls {
    /// Creates new session controls
    #[must_use]
    pub fn new() -> Self {
        let container = GtkBox::new(Orientation::Horizontal, 8);
        container.set_margin_start(8);
        container.set_margin_end(8);
        container.set_margin_top(4);
        container.set_margin_bottom(4);

        let status_label = Label::new(Some("Connecting..."));
        status_label.set_hexpand(true);
        status_label.set_halign(gtk4::Align::Start);
        status_label.add_css_class("dim-label");
        container.append(&status_label);

        let fullscreen_button = Button::from_icon_name("view-fullscreen-symbolic");
        fullscreen_button.set_tooltip_text(Some("Toggle Fullscreen"));
        fullscreen_button.add_css_class("flat");
        container.append(&fullscreen_button);

        let disconnect_button = Button::from_icon_name("process-stop-symbolic");
        disconnect_button.set_tooltip_text(Some("Disconnect"));
        disconnect_button.add_css_class("flat");
        disconnect_button.add_css_class("destructive-action");
        container.append(&disconnect_button);

        Self {
            container,
            fullscreen_button,
            disconnect_button,
            status_label,
        }
    }

    /// Returns the container widget
    #[must_use]
    pub const fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Sets the status text
    pub fn set_status(&self, status: &str) {
        self.status_label.set_text(status);
    }

    /// Connects a callback for the fullscreen button
    pub fn connect_fullscreen<F: Fn() + 'static>(&self, callback: F) {
        self.fullscreen_button.connect_clicked(move |_| callback());
    }

    /// Connects a callback for the disconnect button
    pub fn connect_disconnect<F: Fn() + 'static>(&self, callback: F) {
        self.disconnect_button.connect_clicked(move |_| callback());
    }

    /// Updates the fullscreen button icon based on state
    pub fn set_fullscreen_icon(&self, is_fullscreen: bool) {
        let icon_name = if is_fullscreen {
            "view-restore-symbolic"
        } else {
            "view-fullscreen-symbolic"
        };
        self.fullscreen_button.set_icon_name(icon_name);
    }
}

impl Default for SessionControls {
    fn default() -> Self {
        Self::new()
    }
}

/// Embedded session tab for RDP/VNC connections
#[allow(dead_code)] // Fields kept for GTK widget lifecycle
pub struct EmbeddedSessionTab {
    id: Uuid,
    connection_id: Uuid,
    protocol: String,
    container: GtkBox,
    embed_area: DrawingArea,
    controls: SessionControls,
    process: Rc<RefCell<Option<Child>>>,
    is_embedded: bool,
    is_fullscreen: Rc<RefCell<bool>>,
}

impl EmbeddedSessionTab {
    /// Creates a new embedded session tab
    #[must_use]
    pub fn new(connection_id: Uuid, connection_name: &str, protocol: &str) -> (Self, bool) {
        let id = Uuid::new_v4();
        let display_server = DisplayServer::detect();
        let is_embedded = display_server.supports_embedding();

        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);

        let controls = SessionControls::new();
        container.append(controls.widget());

        let embed_area = DrawingArea::new();
        embed_area.set_hexpand(true);
        embed_area.set_vexpand(true);

        if is_embedded {
            embed_area.set_content_width(800);
            embed_area.set_content_height(600);
            controls.set_status(&format!(
                "{} session - {} (embedded)",
                protocol.to_uppercase(),
                connection_name
            ));
        } else {
            controls.set_status(&format!(
                "{} session - {} (external window)",
                protocol.to_uppercase(),
                connection_name
            ));

            let protocol_clone = protocol.to_string();
            let name_clone = connection_name.to_string();
            embed_area.set_draw_func(move |_area, cr, width, height| {
                // Dark background
                cr.set_source_rgb(0.12, 0.12, 0.14);
                let _ = cr.paint();

                cr.select_font_face(
                    "Sans",
                    gtk4::cairo::FontSlant::Normal,
                    gtk4::cairo::FontWeight::Normal,
                );

                // Icon placeholder (circle with protocol letter)
                let center_y = f64::from(height) / 2.0 - 40.0;
                cr.set_source_rgb(0.3, 0.5, 0.7);
                cr.arc(
                    f64::from(width) / 2.0,
                    center_y,
                    40.0,
                    0.0,
                    2.0 * std::f64::consts::PI,
                );
                let _ = cr.fill();

                cr.set_source_rgb(1.0, 1.0, 1.0);
                cr.set_font_size(32.0);
                let letter = protocol_clone
                    .chars()
                    .next()
                    .unwrap_or('?')
                    .to_uppercase()
                    .to_string();
                if let Ok(extents) = cr.text_extents(&letter) {
                    cr.move_to(
                        f64::from(width) / 2.0 - extents.width() / 2.0,
                        center_y + extents.height() / 2.0,
                    );
                    let _ = cr.show_text(&letter);
                }

                // Connection name
                cr.set_source_rgb(0.9, 0.9, 0.9);
                cr.set_font_size(18.0);
                if let Ok(extents) = cr.text_extents(&name_clone) {
                    cr.move_to((f64::from(width) - extents.width()) / 2.0, center_y + 70.0);
                    let _ = cr.show_text(&name_clone);
                }

                // Status message
                cr.set_font_size(13.0);
                cr.set_source_rgb(0.6, 0.8, 0.6);
                let status = "Session running in separate window";
                if let Ok(extents) = cr.text_extents(status) {
                    cr.move_to((f64::from(width) - extents.width()) / 2.0, center_y + 100.0);
                    let _ = cr.show_text(status);
                }

                // Info text
                cr.set_font_size(11.0);
                cr.set_source_rgb(0.5, 0.5, 0.5);
                let info = "Close this tab to disconnect • Use Disconnect button to terminate";
                if let Ok(extents) = cr.text_extents(info) {
                    cr.move_to((f64::from(width) - extents.width()) / 2.0, center_y + 130.0);
                    let _ = cr.show_text(info);
                }
            });
        }

        container.append(&embed_area);

        let tab = Self {
            id,
            connection_id,
            protocol: protocol.to_string(),
            container,
            embed_area,
            controls,
            process: Rc::new(RefCell::new(None)),
            is_embedded,
            is_fullscreen: Rc::new(RefCell::new(false)),
        };

        tab.setup_controls();

        (tab, is_embedded)
    }

    fn setup_controls(&self) {
        let is_fullscreen = self.is_fullscreen.clone();
        self.controls.connect_fullscreen(move || {
            let mut fs = is_fullscreen.borrow_mut();
            *fs = !*fs;
        });

        let process = self.process.clone();
        self.controls.connect_disconnect(move || {
            if let Some(mut child) = process.borrow_mut().take() {
                let _ = child.kill();
            }
        });
    }

    /// Returns the session UUID
    #[must_use]
    pub const fn id(&self) -> Uuid {
        self.id
    }

    /// Returns the connection UUID
    #[must_use]
    pub const fn connection_id(&self) -> Uuid {
        self.connection_id
    }

    /// Returns the protocol type
    #[must_use]
    pub fn protocol(&self) -> &str {
        &self.protocol
    }

    /// Returns the main container widget
    #[must_use]
    pub const fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Returns whether the session is embedded
    #[must_use]
    pub const fn is_embedded(&self) -> bool {
        self.is_embedded
    }

    /// Sets the status text
    pub fn set_status(&self, status: &str) {
        self.controls.set_status(status);
    }

    /// Sets the child process
    pub fn set_process(&self, child: Child) {
        *self.process.borrow_mut() = Some(child);
    }
}

/// RDP session launcher for embedded and external sessions
pub struct RdpLauncher;

impl RdpLauncher {
    fn find_freerdp_binary() -> Option<String> {
        let candidates = ["xfreerdp3", "xfreerdp", "freerdp"];
        for candidate in candidates {
            if std::process::Command::new("which")
                .arg(candidate)
                .output()
                .is_ok_and(|o| o.status.success())
            {
                return Some(candidate.to_string());
            }
        }
        None
    }

    /// Starts an RDP session
    ///
    /// # Errors
    /// Returns error if the RDP client fails to start
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        tab: &EmbeddedSessionTab,
        host: &str,
        port: u16,
        username: Option<&str>,
        password: Option<&str>,
        domain: Option<&str>,
        resolution: Option<(u32, u32)>,
        extra_args: &[String],
    ) -> Result<(), EmbeddingError> {
        Self::start_with_geometry(
            tab,
            host,
            port,
            username,
            password,
            domain,
            resolution,
            extra_args,
            None,
            true,
            &[],
        )
    }

    /// Starts an RDP session with window geometry support
    ///
    /// # Arguments
    ///
    /// * `tab` - The embedded session tab
    /// * `host` - Target hostname
    /// * `port` - Target port
    /// * `username` - Optional username
    /// * `password` - Optional password
    /// * `domain` - Optional domain
    /// * `resolution` - Optional resolution (width, height)
    /// * `extra_args` - Extra FreeRDP arguments
    /// * `window_geometry` - Optional window geometry (x, y, width, height)
    /// * `remember_window_position` - Whether to apply window geometry
    /// * `shared_folders` - Shared folders for drive redirection (share_name, local_path)
    ///
    /// # Errors
    /// Returns error if the RDP client fails to start
    #[allow(clippy::too_many_arguments)]
    pub fn start_with_geometry(
        tab: &EmbeddedSessionTab,
        host: &str,
        port: u16,
        username: Option<&str>,
        password: Option<&str>,
        domain: Option<&str>,
        resolution: Option<(u32, u32)>,
        extra_args: &[String],
        window_geometry: Option<(i32, i32, i32, i32)>,
        remember_window_position: bool,
        shared_folders: &[(String, std::path::PathBuf)],
    ) -> Result<(), EmbeddingError> {
        use std::process::Command;

        let binary = Self::find_freerdp_binary().ok_or_else(|| {
            EmbeddingError::ProcessStartFailed(
                "FreeRDP client not found. Install xfreerdp or xfreerdp3.".to_string(),
            )
        })?;

        let mut cmd = Command::new(&binary);

        if let Some(dom) = domain {
            if !dom.is_empty() {
                cmd.arg(format!("/d:{dom}"));
            }
        }

        if let Some(user) = username {
            cmd.arg(format!("/u:{user}"));
        }

        if let Some(pass) = password {
            if !pass.is_empty() {
                cmd.arg(format!("/p:{pass}"));
            }
        }

        if let Some((width, height)) = resolution {
            cmd.arg(format!("/w:{width}"));
            cmd.arg(format!("/h:{height}"));
        } else {
            // Default resolution when not specified
            cmd.arg("/w:1920");
            cmd.arg("/h:1080");
        }

        // Security settings - ignore certificate warnings
        cmd.arg("/cert:ignore");
        // Enable dynamic resolution for better display
        cmd.arg("/dynamic-resolution");

        // Add decorations flag for window controls (Requirement 6.1)
        cmd.arg("/decorations");

        // Add window geometry if saved and remember_window_position is enabled (Requirements 6.2, 6.3, 6.4)
        if remember_window_position {
            if let Some((x, y, _width, _height)) = window_geometry {
                cmd.arg(format!("/x:{x}"));
                cmd.arg(format!("/y:{y}"));
            }
        }

        // Add shared folders for drive redirection
        for (share_name, local_path) in shared_folders {
            if local_path.exists() {
                cmd.arg(format!("/drive:{},{}", share_name, local_path.display()));
            }
        }

        for arg in extra_args {
            cmd.arg(arg);
        }

        if port == 3389 {
            cmd.arg(format!("/v:{host}"));
        } else {
            cmd.arg(format!("/v:{host}:{port}"));
        }

        match cmd.spawn() {
            Ok(child) => {
                tab.set_process(child);
                tab.set_status(&format!("Connected to {host}"));
                Ok(())
            }
            Err(e) => Err(EmbeddingError::ProcessStartFailed(e.to_string())),
        }
    }
}

/// VNC session launcher for embedded and external sessions
pub struct VncLauncher;

impl VncLauncher {
    /// Starts a VNC session
    ///
    /// # Errors
    /// Returns error if the VNC client fails to start
    pub fn start(
        tab: &EmbeddedSessionTab,
        host: &str,
        port: u16,
        encoding: Option<&str>,
        quality: Option<u8>,
        extra_args: &[String],
    ) -> Result<(), EmbeddingError> {
        use std::process::Command;

        let mut cmd = Command::new("vncviewer");

        if let Some(enc) = encoding {
            cmd.arg("-PreferredEncoding");
            cmd.arg(enc);
        }

        if let Some(q) = quality {
            cmd.arg("-QualityLevel");
            cmd.arg(q.to_string());
        }

        for arg in extra_args {
            cmd.arg(arg);
        }

        let server = if port == 5900 {
            format!("{host}:0")
        } else if port > 5900 && port < 6000 {
            let display = port - 5900;
            format!("{host}:{display}")
        } else {
            format!("{host}::{port}")
        };
        cmd.arg(&server);

        match cmd.spawn() {
            Ok(child) => {
                tab.set_process(child);
                tab.set_status(&format!("Connected to {host}"));
                Ok(())
            }
            Err(e) => Err(EmbeddingError::ProcessStartFailed(e.to_string())),
        }
    }
}
