//! Embedded session support for RDP/VNC connections
//!
//! This module provides support for embedding RDP and VNC sessions
//! within the main application window using native protocol clients.
//! On Wayland, sessions fall back to external windows.

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, DrawingArea, Label, Orientation};
use libadwaita as adw;
use std::cell::RefCell;
use std::process::Child;
use std::rc::Rc;
use thiserror::Error;
use uuid::Uuid;

use crate::i18n::i18n;

// Re-export DisplayServer from the unified display module for backward compatibility
pub use crate::display::DisplayServer;

/// Error type for embedding operations
#[derive(Debug, Clone, Error)]
pub enum EmbeddingError {
    /// Embedding not supported on Wayland
    #[error("Embedding not supported on Wayland for {protocol}")]
    WaylandNotSupported {
        /// The protocol that doesn't support embedding
        protocol: String,
    },
    /// Failed to get window ID for embedding
    #[error("Failed to get window ID for embedding")]
    WindowIdNotAvailable,
    /// Client process failed to start
    #[error("Failed to start client process: {0}")]
    ProcessStartFailed(String),
    /// Client exited unexpectedly
    #[error("Client exited with code {code}")]
    ClientExited {
        /// The exit code
        code: i32,
    },
}

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
        container.set_margin_start(12);
        container.set_margin_end(12);
        container.set_margin_top(6);
        container.set_margin_bottom(6);

        let status_label = Label::new(Some(&i18n("Connecting...")));
        status_label.set_hexpand(true);
        status_label.set_halign(gtk4::Align::Start);
        status_label.add_css_class("dim-label");
        container.append(&status_label);

        let fullscreen_button = Button::from_icon_name("view-fullscreen-symbolic");
        fullscreen_button.set_tooltip_text(Some(&i18n("Toggle Fullscreen")));
        fullscreen_button.add_css_class("flat");
        fullscreen_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Toggle Fullscreen",
        ))]);
        container.append(&fullscreen_button);

        let disconnect_button = Button::from_icon_name("process-stop-symbolic");
        disconnect_button.set_tooltip_text(Some(&i18n("Disconnect")));
        disconnect_button.add_css_class("flat");
        disconnect_button.add_css_class("destructive-action");
        disconnect_button
            .update_property(&[gtk4::accessible::Property::Label(&i18n("Disconnect"))]);
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
#[allow(dead_code, reason = "Fields kept for GTK widget lifecycle")]
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
    ///
    /// If `force_external` is `true`, the tab always shows the external-window
    /// StatusPage regardless of display server capabilities.
    #[must_use]
    pub fn new(
        connection_id: Uuid,
        connection_name: &str,
        protocol: &str,
        force_external: bool,
    ) -> (Self, bool) {
        let id = Uuid::new_v4();
        let display_server = DisplayServer::detect();
        let is_embedded = !force_external && display_server.supports_embedding();

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

            // StatusPage for external sessions — shows hotkeys and connection info.
            // adw::StatusPage description already supports Pango markup natively.
            let description = format!(
                "{}\n\n<b>Ctrl+Alt+Enter</b>  —  {}\n<b>Right Ctrl</b>  —  {}\n<b>Ctrl+Alt+C</b>  —  {}\n\n<small>{}</small>",
                glib::markup_escape_text(connection_name),
                i18n("Toggle fullscreen"),
                i18n("Release keyboard/mouse grab"),
                i18n("Toggle remote control (assistance)"),
                i18n("This tab will close automatically when the session ends"),
            );
            let status_page = adw::StatusPage::builder()
                .icon_name("preferences-desktop-remote-desktop-symbolic")
                .title(i18n("Session running in separate window"))
                .description(description)
                .hexpand(true)
                .vexpand(true)
                .build();
            container.append(&status_page);

            tracing::debug!(
                connection = %connection_name,
                "External RDP tab: StatusPage appended to container (children: controls + status_page)"
            );
        }

        if is_embedded {
            container.append(&embed_area);
        }

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

    /// Returns a clone of the process handle for external cleanup
    #[must_use]
    pub fn process_handle(&self) -> Rc<RefCell<Option<Child>>> {
        self.process.clone()
    }
}

/// RDP session launcher for embedded and external sessions
pub struct RdpLauncher;

impl RdpLauncher {
    fn find_freerdp_binary() -> Option<String> {
        let candidates = [
            "sdl-freerdp3", // FreeRDP 3.x SDL3 — versioned (distro packages)
            "sdl-freerdp",  // FreeRDP 3.x SDL3 — unversioned (Flatpak / upstream)
            "xfreerdp3",    // FreeRDP 3.x X11
            "xfreerdp",     // FreeRDP 2.x X11
            "freerdp",      // Generic
        ];
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
    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
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
            false,
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
    /// * `ignore_certificate` - Skip TLS certificate verification
    ///
    /// # Errors
    /// Returns error if the RDP client fails to start
    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
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
        ignore_certificate: bool,
    ) -> Result<(), EmbeddingError> {
        use std::process::Command;

        let binary = Self::find_freerdp_binary().ok_or_else(|| {
            EmbeddingError::ProcessStartFailed(
                "FreeRDP client not found. Install xfreerdp, sdl-freerdp3, sdl-freerdp, or xfreerdp3."
                    .to_string(),
            )
        })?;

        let mut cmd = Command::new(&binary);

        if let Some(dom) = domain
            && !dom.is_empty()
        {
            cmd.arg(format!("/d:{dom}"));
        }

        if let Some(user) = username {
            cmd.arg(format!("/u:{user}"));
        }

        let has_password = password.is_some_and(|p| !p.is_empty());
        if has_password && let Some(pass) = password {
            // Use /p: for password — works for both NLA and non-NLA modes.
            // /from-stdin:force is unreliable with sdl-freerdp3 GUI event loop.
            cmd.arg(format!("/p:{pass}"));
        }

        if let Some((width, height)) = resolution {
            cmd.arg(format!("/w:{width}"));
            cmd.arg(format!("/h:{height}"));
        } else {
            // Default resolution when not specified
            cmd.arg("/w:1920");
            cmd.arg("/h:1080");
        }

        // Security settings — conditional based on connection settings.
        // Default is TOFU (trust-on-first-use), matching SSH known_hosts behavior.
        if ignore_certificate {
            // Remove old certificate file to accept new one (like SSH known_hosts removal)
            Self::remove_known_certificate(host, port);
            cmd.arg("/cert:ignore");
        } else {
            cmd.arg("/cert:tofu");
        }
        // Enable dynamic resolution for better display
        cmd.arg("/dynamic-resolution");

        // Add decorations flag for window controls (Requirement 6.1)
        cmd.arg("/decorations");

        // Add window geometry if saved and remember_window_position is enabled (Requirements 6.2, 6.3, 6.4)
        if remember_window_position && let Some((x, y, _width, _height)) = window_geometry {
            cmd.arg(format!("/x:{x}"));
            cmd.arg(format!("/y:{y}"));
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

        // Capture stderr for error detection
        cmd.stderr(std::process::Stdio::piped());

        match cmd.spawn() {
            Ok(mut child) => {
                // Wait briefly to detect immediate failures (certificate errors, auth failures)
                std::thread::sleep(std::time::Duration::from_millis(1500));

                match child.try_wait() {
                    Ok(Some(status)) if !status.success() => {
                        // Process exited quickly with error — extract stderr
                        let error_msg = child
                            .stderr
                            .take()
                            .and_then(|stderr| {
                                use std::io::Read;
                                let mut buf = String::new();
                                let mut reader = std::io::BufReader::new(stderr);
                                reader.read_to_string(&mut buf).ok()?;
                                Some(buf)
                            })
                            .unwrap_or_default();

                        // Parse meaningful error from FreeRDP output
                        let user_error = Self::parse_freerdp_error(&error_msg);
                        Err(EmbeddingError::ProcessStartFailed(user_error))
                    }
                    _ => {
                        // Process still running or exited successfully — treat as connected
                        tab.set_process(child);
                        tab.set_status(&format!("Connected to {host}"));
                        Ok(())
                    }
                }
            }
            Err(e) => Err(EmbeddingError::ProcessStartFailed(e.to_string())),
        }
    }

    /// Parses FreeRDP stderr output to extract a user-friendly error message
    fn parse_freerdp_error(stderr: &str) -> String {
        if stderr.contains("certificate not trusted")
            || stderr.contains("ERRCONNECT_TLS_CONNECT_FAILED")
        {
            if stderr.contains("NEW HOST IDENTIFICATION") || stderr.contains("has changed") {
                return "RDP certificate has changed. Enable 'Ignore Certificate' or accept the new certificate.".to_string();
            }
            return "TLS certificate verification failed. Enable 'Ignore Certificate' in connection settings.".to_string();
        }
        if stderr.contains("ERRCONNECT_CONNECT_CANCELLED")
            || stderr.contains("nla_client_setup_identity")
        {
            return "NLA authentication failed. Check username/password or disable NLA."
                .to_string();
        }
        if stderr.contains("ERRCONNECT_CONNECT_TRANSPORT_FAILED") {
            return "Connection refused. Check host and port.".to_string();
        }
        if stderr.contains("ERRCONNECT_DNS_NAME_NOT_FOUND") {
            return "Host not found. Check the hostname.".to_string();
        }
        // Fallback: return last ERROR line or generic message
        stderr
            .lines()
            .rev()
            .find(|line| line.contains("[ERROR]"))
            .map(|line| {
                // Extract the message part after the last ]:
                line.rsplit("]: ").next().unwrap_or(line).trim().to_string()
            })
            .unwrap_or_else(|| "FreeRDP exited with error (exit code non-zero)".to_string())
    }

    /// Removes the stored FreeRDP certificate for a host, allowing TOFU to accept a new one.
    /// This is equivalent to removing a line from SSH known_hosts.
    fn remove_known_certificate(host: &str, port: u16) {
        // FreeRDP stores known certificates in ~/.config/freerdp/server/<host>_<port>.pem
        // and also in ~/.config/freerdp/known_hosts2 (FreeRDP 3.x)
        if let Some(config_dir) = dirs::config_dir() {
            let freerdp_dir = config_dir.join("freerdp").join("server");
            let pem_file = if port == 3389 {
                freerdp_dir.join(format!("{host}_3389.pem"))
            } else {
                freerdp_dir.join(format!("{host}_{port}.pem"))
            };
            if pem_file.exists() {
                tracing::debug!(
                    ?pem_file,
                    "Removing old FreeRDP certificate to accept new one"
                );
                let _ = std::fs::remove_file(&pem_file);
            }

            // Also try the known_hosts2 file (FreeRDP 3.x format)
            let known_hosts = config_dir.join("freerdp").join("known_hosts2");
            if known_hosts.exists()
                && let Ok(content) = std::fs::read_to_string(&known_hosts)
            {
                let host_pattern = if port == 3389 {
                    format!("{host} 3389")
                } else {
                    format!("{host} {port}")
                };
                let filtered: Vec<&str> = content
                    .lines()
                    .filter(|line| !line.contains(&host_pattern))
                    .collect();
                if filtered.len() < content.lines().count() {
                    tracing::debug!(
                        ?known_hosts,
                        "Removing host entry from FreeRDP known_hosts2"
                    );
                    let _ = std::fs::write(&known_hosts, filtered.join("\n") + "\n");
                }
            }
        }
    }
}
