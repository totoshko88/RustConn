//! VNC session widget for native embedding
//!
//! This module provides the `VncSessionWidget` struct that wraps the VNC FFI
//! display widget with overlay controls and state management.
//!
//! # Requirements Coverage
//!
//! - Requirement 2.1: Native VNC embedding as GTK widget
//! - Requirement 2.2: Keyboard and mouse input forwarding
//! - Requirement 2.3: VNC authentication handling
//! - Requirement 2.5: Connection state management and error handling
//! - Requirement 8.1: VNC viewer detection and launch
//! - Requirement 8.3: Error handling for missing VNC viewer

use super::{SessionError, SessionState};
use crate::embedded_vnc::{EmbeddedVncWidget, VncConfig as EmbeddedVncConfig, VncConnectionState};
use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Label, Orientation, Overlay, Spinner};
use rustconn_core::ffi::{VncCredentialType, VncDisplay};
use rustconn_core::models::{VncClientMode, VncConfig};
use rustconn_core::protocol::{detect_vnc_client, detect_vnc_viewer_name};
use std::cell::RefCell;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;

use crate::i18n::i18n;

/// Callback type for authentication requests
type AuthCallback = Box<dyn Fn(&[VncCredentialType]) + 'static>;

/// Callback type for state change notifications
type StateCallback = Box<dyn Fn(SessionState) + 'static>;

/// VNC session widget with overlay controls
///
/// This widget wraps the VNC FFI display and provides:
/// - Connection lifecycle management
/// - Authentication callback handling
/// - State tracking and error reporting
/// - Overlay controls for session management
/// - External VNC viewer process management
///
/// # Example
///
/// ```ignore
/// use rustconn::session::vnc::VncSessionWidget;
///
/// let widget = VncSessionWidget::new();
///
/// // Set up authentication callback
/// widget.connect_auth_required(|creds| {
///     // Prompt user for credentials
/// });
///
/// // Connect to VNC server
/// widget.connect("192.168.1.100", 5900, None);
/// ```
#[allow(dead_code)] // Fields kept for GTK widget lifecycle
pub struct VncSessionWidget {
    /// The GTK overlay container
    overlay: Overlay,
    /// The VNC display widget (FFI placeholder)
    display: Rc<VncDisplay>,
    /// The embedded VNC widget (native Rust VNC client)
    embedded_widget: Rc<EmbeddedVncWidget>,
    /// Current session state
    state: Rc<RefCell<SessionState>>,
    /// Status label for connection feedback
    status_label: Label,
    /// Spinner for connection progress
    spinner: Spinner,
    /// Status container (kept for preventing premature deallocation and future floating controls)
    status_container: GtkBox,
    /// Authentication callback
    auth_callback: Rc<RefCell<Option<AuthCallback>>>,
    /// State change callback
    state_callback: Rc<RefCell<Option<StateCallback>>>,
    /// External VNC viewer process (when using external mode)
    external_process: Rc<RefCell<Option<Child>>>,
    /// Whether using external viewer mode
    is_external: Rc<RefCell<bool>>,
    /// Whether using embedded native VNC mode
    is_embedded_native: Rc<RefCell<bool>>,
}

impl VncSessionWidget {
    /// Creates a new VNC session widget
    ///
    /// The widget is created in a disconnected state and ready for connection.
    #[must_use]
    pub fn new() -> Self {
        let display = Rc::new(VncDisplay::new());
        let embedded_widget = Rc::new(EmbeddedVncWidget::new());
        let state = Rc::new(RefCell::new(SessionState::Disconnected));
        let auth_callback: Rc<RefCell<Option<AuthCallback>>> = Rc::new(RefCell::new(None));
        let state_callback: Rc<RefCell<Option<StateCallback>>> = Rc::new(RefCell::new(None));
        let external_process: Rc<RefCell<Option<Child>>> = Rc::new(RefCell::new(None));
        let is_external = Rc::new(RefCell::new(false));
        let is_embedded_native = Rc::new(RefCell::new(false));

        // Create the overlay container
        let overlay = Overlay::new();

        // Use the embedded VNC widget as the main display
        let embedded_container = embedded_widget.widget().clone();
        embedded_container.set_hexpand(true);
        embedded_container.set_vexpand(true);
        embedded_container.add_css_class("vnc-display");

        // Create status container for connection feedback
        let status_container = GtkBox::new(Orientation::Vertical, 12);
        status_container.set_halign(Align::Center);
        status_container.set_valign(Align::Center);

        let spinner = Spinner::new();
        spinner.set_spinning(false);
        spinner.set_visible(false);

        let status_label = Label::new(Some(&i18n("Disconnected")));
        status_label.add_css_class("dim-label");

        status_container.append(&spinner);
        status_container.append(&status_label);

        // Set up the overlay with embedded widget as child
        overlay.set_child(Some(&embedded_container));
        overlay.add_overlay(&status_container);

        let widget = Self {
            overlay,
            display,
            embedded_widget,
            state,
            status_label,
            spinner,
            status_container,
            auth_callback,
            state_callback,
            external_process,
            is_external,
            is_embedded_native,
        };

        // Set up VNC display signal handlers
        widget.setup_display_signals();
        // Set up embedded widget state callbacks
        widget.setup_embedded_callbacks();

        widget
    }

    /// Detects if a VNC viewer is installed on the system
    ///
    /// Returns the name of the detected VNC viewer, or None if no viewer is found.
    ///
    /// # Returns
    /// `Some(String)` with the viewer name, or `None` if no viewer is installed
    #[must_use]
    pub fn detect_vnc_viewer() -> Option<String> {
        detect_vnc_viewer_name()
    }

    /// Returns information about the installed VNC client
    ///
    /// This provides detailed information including the path and version
    /// of the detected VNC viewer.
    #[must_use]
    pub fn get_vnc_client_info() -> rustconn_core::protocol::ClientInfo {
        detect_vnc_client()
    }

    /// Sets up signal handlers for the VNC display
    fn setup_display_signals(&self) {
        let state = self.state.clone();
        let status_label = self.status_label.clone();
        let spinner = self.spinner.clone();
        let state_callback = self.state_callback.clone();

        // Connected signal
        let state_clone = state.clone();
        let status_label_clone = status_label.clone();
        let spinner_clone = spinner.clone();
        let state_callback_clone = state_callback.clone();
        self.display.connect_vnc_connected(move |_| {
            *state_clone.borrow_mut() = SessionState::Connected;
            status_label_clone.set_text("Connected");
            spinner_clone.set_spinning(false);
            spinner_clone.set_visible(false);

            if let Some(ref callback) = *state_callback_clone.borrow() {
                callback(SessionState::Connected);
            }
        });

        // Disconnected signal
        let state_clone = state.clone();
        let status_label_clone = status_label.clone();
        let spinner_clone = spinner.clone();
        let state_callback_clone = state_callback.clone();
        self.display.connect_vnc_disconnected(move |_| {
            *state_clone.borrow_mut() = SessionState::Disconnected;
            status_label_clone.set_text(&i18n("Disconnected"));
            spinner_clone.set_spinning(false);
            spinner_clone.set_visible(false);

            if let Some(ref callback) = *state_callback_clone.borrow() {
                callback(SessionState::Disconnected);
            }
        });

        // Auth credential signal
        let state_clone = state.clone();
        let status_label_clone = status_label.clone();
        let auth_callback_clone = self.auth_callback.clone();
        let state_callback_clone = state_callback.clone();
        self.display.connect_vnc_auth_credential(move |_, creds| {
            *state_clone.borrow_mut() = SessionState::Authenticating;
            status_label_clone.set_text("Authenticating...");

            if let Some(ref callback) = *state_callback_clone.borrow() {
                callback(SessionState::Authenticating);
            }

            if let Some(ref callback) = *auth_callback_clone.borrow() {
                callback(creds);
            }
        });

        // Auth failure signal
        let state_clone = state;
        let status_label_clone = status_label;
        let spinner_clone = spinner;
        let state_callback_clone = state_callback;
        self.display.connect_vnc_auth_failure(move |_, msg| {
            let error = SessionError::authentication_failed(msg);
            *state_clone.borrow_mut() = SessionState::Error(error.clone());
            status_label_clone.set_text(&format!("Authentication failed: {msg}"));
            spinner_clone.set_spinning(false);
            spinner_clone.set_visible(false);

            if let Some(ref callback) = *state_callback_clone.borrow() {
                callback(SessionState::Error(error));
            }
        });
    }

    /// Sets up callbacks for the embedded VNC widget
    fn setup_embedded_callbacks(&self) {
        let state = self.state.clone();
        let status_label = self.status_label.clone();
        let spinner = self.spinner.clone();
        let state_callback = self.state_callback.clone();

        // State change callback
        self.embedded_widget
            .connect_state_changed(move |vnc_state| {
                let session_state = match vnc_state {
                    VncConnectionState::Disconnected => SessionState::Disconnected,
                    VncConnectionState::Connecting => SessionState::Connecting,
                    VncConnectionState::Connected => SessionState::Connected,
                    VncConnectionState::Error => {
                        SessionState::Error(SessionError::connection_failed("VNC connection error"))
                    }
                };

                *state.borrow_mut() = session_state.clone();

                // Hide status label when connected to avoid obstructing the view
                if vnc_state == VncConnectionState::Connected {
                    status_label.set_visible(false);
                } else {
                    let status_text = match vnc_state {
                        VncConnectionState::Disconnected => "Disconnected",
                        VncConnectionState::Connecting => "Connecting...",
                        VncConnectionState::Connected => "", // Won't be shown
                        VncConnectionState::Error => "Connection error",
                    };
                    status_label.set_text(status_text);
                    status_label.set_visible(true);
                }

                if vnc_state == VncConnectionState::Connecting {
                    spinner.set_visible(true);
                    spinner.set_spinning(true);
                } else {
                    spinner.set_spinning(false);
                    spinner.set_visible(false);
                }

                if let Some(ref callback) = *state_callback.borrow() {
                    callback(session_state);
                }
            });

        // Error callback
        let state = self.state.clone();
        let status_label = self.status_label.clone();
        let state_callback = self.state_callback.clone();

        self.embedded_widget.connect_error(move |msg| {
            let error = SessionError::connection_failed(msg);
            *state.borrow_mut() = SessionState::Error(error.clone());
            status_label.set_text(&format!("Error: {msg}"));

            if let Some(ref callback) = *state_callback.borrow() {
                callback(SessionState::Error(error));
            }
        });
    }

    /// Connects to a VNC server
    ///
    /// This method first attempts to use native embedded VNC (if available).
    /// If native embedding fails or is not available, it falls back to an
    /// external VNC viewer.
    ///
    /// # Arguments
    ///
    /// * `host` - The hostname or IP address of the VNC server
    /// * `port` - The port number (typically 5900 + display number)
    /// * `password` - Optional password for authentication (note: most viewers
    ///   will prompt for password interactively for security)
    ///
    /// # Errors
    ///
    /// Returns a `SessionError` if:
    /// - No VNC viewer is installed on the system
    /// - The connection cannot be initiated
    /// - The viewer process fails to start
    pub fn connect(
        &self,
        host: &str,
        port: u16,
        password: Option<&str>,
    ) -> Result<(), SessionError> {
        // Check current state
        let current_state = self.state.borrow().clone();
        if !current_state.can_transition_to(&SessionState::Connecting) {
            return Err(SessionError::connection_failed(format!(
                "Cannot connect from state: {current_state}"
            )));
        }

        // Update state to connecting
        *self.state.borrow_mut() = SessionState::Connecting;
        self.status_label
            .set_text(&format!("Connecting to {host}:{port}..."));
        self.spinner.set_visible(true);
        self.spinner.set_spinning(true);

        // Notify state change
        if let Some(ref callback) = *self.state_callback.borrow() {
            callback(SessionState::Connecting);
        }

        // Try native embedded VNC first (if vnc-embedded feature is enabled)
        if rustconn_core::is_embedded_vnc_available() {
            // If password is provided, set it before connecting
            if let Some(pwd) = password {
                let _ = self
                    .display
                    .set_credential(VncCredentialType::Password, pwd);
            }

            // Try to initiate native connection
            if self.display.open_host(host, port).is_ok() {
                *self.is_external.borrow_mut() = false;
                // Connection will be handled by signal callbacks
                return Ok(());
            }
            eprintln!("Native VNC embedding failed, falling back to external viewer...");
        }

        // Fall back to external VNC viewer
        let viewer = Self::detect_vnc_viewer().ok_or_else(|| {
            let client_info = Self::get_vnc_client_info();
            let hint = client_info.install_hint.unwrap_or_else(|| {
                "Install TigerVNC: sudo apt install tigervnc-viewer".to_string()
            });
            SessionError::connection_failed(format!("No VNC viewer installed. {hint}"))
        })?;

        // Try to spawn external VNC viewer
        match self.spawn_external_viewer(&viewer, host, port, password) {
            Ok(()) => {
                *self.is_external.borrow_mut() = true;
                *self.state.borrow_mut() = SessionState::Connected;
                self.status_label
                    .set_text(&format!("Session running in external {viewer} window"));
                self.spinner.set_spinning(false);
                self.spinner.set_visible(false);

                // Notify state change
                if let Some(ref callback) = *self.state_callback.borrow() {
                    callback(SessionState::Connected);
                }

                Ok(())
            }
            Err(e) => {
                *self.state.borrow_mut() = SessionState::Disconnected;
                self.status_label.set_text("Connection failed");
                self.spinner.set_spinning(false);
                self.spinner.set_visible(false);
                Err(e)
            }
        }
    }

    /// Connects to a VNC server using protocol configuration
    ///
    /// This method respects the `client_mode` setting in `VncConfig`:
    /// - `Embedded`: Uses native VNC embedding with dynamic resolution
    /// - `External`: Always uses external VNC viewer application
    ///
    /// # Arguments
    ///
    /// * `host` - The hostname or IP address of the VNC server
    /// * `port` - The port number (typically 5900 + display number)
    /// * `password` - Optional password for authentication
    /// * `config` - VNC protocol configuration with client mode and other settings
    ///
    /// # Errors
    ///
    /// Returns a `SessionError` if connection fails
    pub fn connect_with_config(
        &self,
        host: &str,
        port: u16,
        password: Option<&str>,
        config: &VncConfig,
    ) -> Result<(), SessionError> {
        // Check current state
        let current_state = self.state.borrow().clone();
        if !current_state.can_transition_to(&SessionState::Connecting) {
            return Err(SessionError::connection_failed(format!(
                "Cannot connect from state: {current_state}"
            )));
        }

        // Update state to connecting
        *self.state.borrow_mut() = SessionState::Connecting;
        self.status_label
            .set_text(&format!("Connecting to {host}:{port}..."));
        self.spinner.set_visible(true);
        self.spinner.set_spinning(true);

        // Notify state change
        if let Some(ref callback) = *self.state_callback.borrow() {
            callback(SessionState::Connecting);
        }

        // Check client mode - if External, use external viewer directly
        if config.client_mode == VncClientMode::External {
            return self.connect_external_with_config(host, port, password, config);
        }

        // Embedded mode requested - try native embedded VNC using EmbeddedVncWidget
        if rustconn_core::is_embedded_vnc_available() {
            // Create embedded VNC config from protocol config
            let mut embedded_config = EmbeddedVncConfig::new(host)
                .with_port(port)
                .with_view_only(config.view_only)
                .with_clipboard(config.clipboard_enabled);
            embedded_config.scale_override = config.scale_override;

            let embedded_config = if let Some(pwd) = password {
                embedded_config.with_password(pwd)
            } else {
                embedded_config
            };

            // Try to connect using embedded widget
            match self.embedded_widget.connect(&embedded_config) {
                Ok(()) => {
                    *self.is_external.borrow_mut() = false;
                    *self.is_embedded_native.borrow_mut() = true;
                    // Connection state will be handled by embedded widget callbacks
                    return Ok(());
                }
                Err(e) => {
                    eprintln!(
                        "Native VNC embedding failed: {e}, falling back to external viewer..."
                    );
                }
            }
        } else {
            // Native VNC not available - for Embedded mode, fall back to external
            eprintln!(
                "Native VNC embedding not available (vnc-embedded feature not enabled), \
                 using external viewer..."
            );
        }

        // Fall back to external VNC viewer (for both failed embedded and unavailable native)
        self.connect_external_with_config(host, port, password, config)
    }

    /// Connects using external VNC viewer with configuration options
    fn connect_external_with_config(
        &self,
        host: &str,
        port: u16,
        password: Option<&str>,
        config: &VncConfig,
    ) -> Result<(), SessionError> {
        let viewer = Self::detect_vnc_viewer().ok_or_else(|| {
            let client_info = Self::get_vnc_client_info();
            let hint = client_info.install_hint.unwrap_or_else(|| {
                "Install TigerVNC: sudo apt install tigervnc-viewer".to_string()
            });
            SessionError::connection_failed(format!("No VNC viewer installed. {hint}"))
        })?;

        // Try to spawn external VNC viewer with config
        match self.spawn_external_viewer_with_config(&viewer, host, port, password, config) {
            Ok(()) => {
                *self.is_external.borrow_mut() = true;
                *self.state.borrow_mut() = SessionState::Connected;
                self.status_label
                    .set_text(&format!("Session running in external {viewer} window"));
                self.spinner.set_spinning(false);
                self.spinner.set_visible(false);

                // Notify state change
                if let Some(ref callback) = *self.state_callback.borrow() {
                    callback(SessionState::Connected);
                }

                Ok(())
            }
            Err(e) => {
                *self.state.borrow_mut() = SessionState::Disconnected;
                self.status_label.set_text("Connection failed");
                self.spinner.set_spinning(false);
                self.spinner.set_visible(false);
                Err(e)
            }
        }
    }

    /// Spawns an external VNC viewer process with configuration options
    fn spawn_external_viewer_with_config(
        &self,
        viewer: &str,
        host: &str,
        port: u16,
        _password: Option<&str>,
        config: &VncConfig,
    ) -> Result<(), SessionError> {
        let mut cmd = Command::new(viewer);

        // Build server address based on port and viewer type
        let server = Self::build_server_address(viewer, host, port);

        // Add viewer-specific arguments with config options
        match viewer {
            "vncviewer" | "tigervnc" | "xvnc4viewer" => {
                // TigerVNC/TightVNC/RealVNC style
                // Only add encoding if it's a single valid value (not comma-separated)
                if let Some(ref encoding) = config.encoding {
                    let enc = encoding.trim();
                    if !enc.is_empty() && !enc.contains(',') {
                        cmd.arg("-PreferredEncoding");
                        cmd.arg(enc);
                    }
                }
                if let Some(quality) = config.quality {
                    cmd.arg("-QualityLevel");
                    cmd.arg(quality.to_string());
                }
                if let Some(compression) = config.compression {
                    cmd.arg("-CompressLevel");
                    cmd.arg(compression.to_string());
                }
                if config.view_only {
                    cmd.arg("-ViewOnly");
                }
                cmd.arg(&server);
            }
            "gvncviewer" => {
                // GTK-VNC viewer
                cmd.arg(&server);
            }
            "remmina" => {
                // Remmina uses a different connection format
                cmd.arg("-c");
                cmd.arg(format!("vnc://{host}:{port}"));
            }
            "vinagre" => {
                // Vinagre
                cmd.arg(format!("vnc://{host}:{port}"));
            }
            "krdc" => {
                // KDE Remote Desktop Client
                cmd.arg(format!("vnc://{host}:{port}"));
            }
            _ => {
                // Generic fallback
                cmd.arg(&server);
            }
        }

        // Add custom arguments from config
        for arg in &config.custom_args {
            cmd.arg(arg);
        }

        // Don't capture stdout/stderr so the viewer can run independently
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());

        // Spawn the process
        match cmd.spawn() {
            Ok(child) => {
                *self.external_process.borrow_mut() = Some(child);
                Ok(())
            }
            Err(e) => Err(SessionError::connection_failed(format!(
                "Failed to start VNC viewer '{viewer}': {e}"
            ))),
        }
    }

    /// Spawns an external VNC viewer process
    ///
    /// # Arguments
    ///
    /// * `viewer` - The name of the VNC viewer binary
    /// * `host` - The hostname or IP address
    /// * `port` - The port number
    /// * `password` - Optional password (not passed on command line for security)
    fn spawn_external_viewer(
        &self,
        viewer: &str,
        host: &str,
        port: u16,
        _password: Option<&str>,
    ) -> Result<(), SessionError> {
        let mut cmd = Command::new(viewer);

        // Build server address based on port and viewer type
        let server = Self::build_server_address(viewer, host, port);

        // Add viewer-specific arguments
        match viewer {
            "vncviewer" | "tigervnc" | "xvnc4viewer" => {
                // TigerVNC/TightVNC/RealVNC style
                cmd.arg(&server);
            }
            "gvncviewer" => {
                // GTK-VNC viewer
                cmd.arg(&server);
            }
            "remmina" => {
                // Remmina uses a different connection format
                cmd.arg("-c");
                cmd.arg(format!("vnc://{host}:{port}"));
            }
            "vinagre" => {
                // Vinagre
                cmd.arg(format!("vnc://{host}:{port}"));
            }
            "krdc" => {
                // KDE Remote Desktop Client
                cmd.arg(format!("vnc://{host}:{port}"));
            }
            _ => {
                // Generic fallback
                cmd.arg(&server);
            }
        }

        // Don't capture stdout/stderr so the viewer can run independently
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());

        // Spawn the process
        match cmd.spawn() {
            Ok(child) => {
                *self.external_process.borrow_mut() = Some(child);
                Ok(())
            }
            Err(e) => Err(SessionError::connection_failed(format!(
                "Failed to start VNC viewer '{viewer}': {e}"
            ))),
        }
    }

    /// Builds the server address string based on viewer type and port
    fn build_server_address(viewer: &str, host: &str, port: u16) -> String {
        match viewer {
            "vncviewer" | "tigervnc" | "xvnc4viewer" | "gvncviewer" => {
                // These viewers use display number format for standard ports
                if port == 5900 {
                    format!("{host}:0")
                } else if port > 5900 && port < 6000 {
                    let display = port - 5900;
                    format!("{host}:{display}")
                } else {
                    // Use :: for raw port numbers
                    format!("{host}::{port}")
                }
            }
            _ => {
                // Other viewers typically use host:port format
                format!("{host}:{port}")
            }
        }
    }

    /// Disconnects from the VNC server
    ///
    /// This terminates any external VNC viewer process and cleans up resources.
    pub fn disconnect(&self) {
        // Disconnect embedded widget if using native mode
        if *self.is_embedded_native.borrow() {
            self.embedded_widget.disconnect();
        }

        // Kill external process if running
        if let Some(mut child) = self.external_process.borrow_mut().take() {
            let _ = child.kill();
            let _ = child.wait();
        }

        // Close FFI display (placeholder)
        self.display.close();

        // Reset state
        *self.is_external.borrow_mut() = false;
        *self.is_embedded_native.borrow_mut() = false;
        *self.state.borrow_mut() = SessionState::Disconnected;
        self.status_label.set_text(&i18n("Disconnected"));
        self.spinner.set_spinning(false);
        self.spinner.set_visible(false);

        if let Some(ref callback) = *self.state_callback.borrow() {
            callback(SessionState::Disconnected);
        }
    }

    /// Reconnects using the stored configuration
    ///
    /// This method attempts to reconnect to the VNC server using the
    /// configuration from the previous connection.
    ///
    /// # Errors
    ///
    /// Returns an error if no previous configuration exists or if
    /// the connection fails.
    pub fn reconnect(&self) -> Result<(), SessionError> {
        // Try to reconnect using embedded widget
        self.embedded_widget
            .reconnect()
            .map_err(|e| SessionError::connection_failed(e.to_string()))
    }

    /// Connects a callback for reconnect button clicks
    ///
    /// The callback is invoked when the user clicks the Reconnect button
    /// after a session has disconnected or encountered an error.
    pub fn connect_reconnect<F>(&self, callback: F)
    where
        F: Fn() + 'static,
    {
        self.embedded_widget.connect_reconnect(callback);
    }

    /// Returns whether the session is using an external viewer
    #[must_use]
    pub fn is_external(&self) -> bool {
        *self.is_external.borrow()
    }

    /// Returns the GTK widget for embedding in containers
    #[must_use]
    pub fn widget(&self) -> &gtk4::Widget {
        self.overlay.upcast_ref()
    }

    /// Returns the current session state
    #[must_use]
    pub fn state(&self) -> SessionState {
        self.state.borrow().clone()
    }

    /// Returns whether the session is connected
    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.state.borrow().is_connected()
    }

    /// Provides credentials for authentication
    ///
    /// This should be called in response to the auth_required callback.
    ///
    /// # Arguments
    ///
    /// * `username` - Optional username
    /// * `password` - The password
    ///
    /// # Errors
    ///
    /// Returns a `SessionError` if credentials cannot be set.
    pub fn provide_credentials(
        &self,
        username: Option<&str>,
        password: &str,
    ) -> Result<(), SessionError> {
        if let Some(user) = username {
            self.display
                .set_credential(VncCredentialType::Username, user)
                .map_err(|e| SessionError::authentication_failed(e.to_string()))?;
        }

        self.display
            .set_credential(VncCredentialType::Password, password)
            .map_err(|e| SessionError::authentication_failed(e.to_string()))?;

        Ok(())
    }

    /// Enables or disables display scaling
    pub fn set_scaling(&self, enabled: bool) {
        self.display.set_scaling(enabled);
    }

    /// Returns whether scaling is enabled
    #[must_use]
    pub fn scaling_enabled(&self) -> bool {
        self.display.scaling_enabled()
    }

    /// Connects a callback for authentication requests
    ///
    /// The callback receives a list of credential types that the server requires.
    pub fn connect_auth_required<F>(&self, callback: F)
    where
        F: Fn(&[VncCredentialType]) + 'static,
    {
        *self.auth_callback.borrow_mut() = Some(Box::new(callback));
    }

    /// Connects a callback for state changes
    ///
    /// The callback is invoked whenever the session state changes.
    pub fn connect_state_changed<F>(&self, callback: F)
    where
        F: Fn(SessionState) + 'static,
    {
        *self.state_callback.borrow_mut() = Some(Box::new(callback));
    }

    /// Returns the underlying VNC display (for advanced usage)
    #[must_use]
    pub fn display(&self) -> &VncDisplay {
        &self.display
    }
}

impl Default for VncSessionWidget {
    fn default() -> Self {
        Self::new()
    }
}

// Manual Debug implementation since we can't derive it for callback types
impl std::fmt::Debug for VncSessionWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VncSessionWidget")
            .field("state", &self.state.borrow())
            .field("display", &"VncDisplay { ... }")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These tests require GTK to be initialized, which may not be available
    // in all test environments. The property tests in rustconn-core handle
    // the core logic testing without GTK dependencies.

    #[test]
    fn test_session_state_transitions() {
        // Test that state transitions are properly validated
        let disconnected = SessionState::Disconnected;
        assert!(disconnected.can_transition_to(&SessionState::Connecting));
        assert!(!disconnected.can_transition_to(&SessionState::Connected));

        let connecting = SessionState::Connecting;
        assert!(connecting.can_transition_to(&SessionState::Connected));
        assert!(connecting.can_transition_to(&SessionState::Authenticating));
        assert!(connecting.can_transition_to(&SessionState::Disconnected));

        let connected = SessionState::Connected;
        assert!(connected.can_transition_to(&SessionState::Disconnected));
        assert!(!connected.can_transition_to(&SessionState::Connecting));
    }
}
