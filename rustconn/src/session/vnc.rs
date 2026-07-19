//! VNC session widget for native embedding
//!
//! `VncSessionWidget` wraps the native Rust VNC client (`EmbeddedVncWidget`,
//! backed by the `vnc-rs` crate) with overlay controls and state management,
//! and falls back to an external VNC viewer when embedding is unavailable or
//! the connection requests `VncClientMode::External`.

use std::cell::RefCell;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Label, Orientation, Overlay};
#[cfg(feature = "adw-1-6")]
use libadwaita as adw;
use rustconn_core::models::{VncClientMode, VncConfig};
use rustconn_core::protocol::{VncProtocol, detect_vnc_client, detect_vnc_viewer_name};

use super::{SessionError, SessionState};
use crate::embedded_vnc::{EmbeddedVncWidget, VncConfig as EmbeddedVncConfig, VncConnectionState};
use crate::i18n::i18n;

/// Callback type for state change notifications
type StateCallback = Box<dyn Fn(SessionState) + 'static>;

/// VNC session widget with overlay controls
///
/// This widget wraps the native embedded VNC client and provides:
/// - Connection lifecycle management
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
/// // Connect to VNC server
/// widget.connect_with_config("192.168.1.100", 5900, None, &config);
/// ```
#[expect(dead_code, reason = "status_container kept for GTK widget lifecycle")]
pub struct VncSessionWidget {
    /// The GTK overlay container
    overlay: Overlay,
    /// The embedded VNC widget (native Rust VNC client)
    embedded_widget: Rc<EmbeddedVncWidget>,
    /// Current session state
    state: Rc<RefCell<SessionState>>,
    /// Status label for connection feedback
    status_label: Label,
    /// Spinner for connection progress
    #[cfg(feature = "adw-1-6")]
    spinner: adw::Spinner,
    #[cfg(not(feature = "adw-1-6"))]
    spinner: gtk4::Spinner,
    /// Status container (kept for preventing premature deallocation and future floating controls)
    status_container: GtkBox,
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
        let embedded_widget = Rc::new(EmbeddedVncWidget::new());
        let state = Rc::new(RefCell::new(SessionState::Disconnected));
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

        #[cfg(feature = "adw-1-6")]
        let spinner = {
            let s = adw::Spinner::new();
            s.set_visible(false);
            s
        };
        #[cfg(not(feature = "adw-1-6"))]
        let spinner = {
            let s = gtk4::Spinner::new();
            s.set_spinning(false);
            s.set_visible(false);
            s
        };

        let status_label = Label::new(Some(&i18n("Disconnected")));
        status_label.add_css_class("dim-label");

        status_container.append(&spinner);
        status_container.append(&status_label);

        // Set up the overlay with embedded widget as child
        overlay.set_child(Some(&embedded_container));
        overlay.add_overlay(&status_container);

        let widget = Self {
            overlay,
            embedded_widget,
            state,
            status_label,
            spinner,
            status_container,
            state_callback,
            external_process,
            is_external,
            is_embedded_native,
        };

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
                        VncConnectionState::Disconnected => i18n("Disconnected"),
                        VncConnectionState::Connecting => i18n("Connecting..."),
                        VncConnectionState::Connected => String::new(), // Won't be shown
                        VncConnectionState::Error => i18n("Connection error"),
                    };
                    status_label.set_text(&status_text);
                    status_label.set_visible(true);
                }

                if vnc_state == VncConnectionState::Connecting {
                    spinner.set_visible(true);
                    #[cfg(not(feature = "adw-1-6"))]
                    spinner.set_spinning(true);
                } else {
                    #[cfg(not(feature = "adw-1-6"))]
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
            status_label.set_text(&i18n("Connection error"));

            if let Some(ref callback) = *state_callback.borrow() {
                callback(SessionState::Error(error));
            }
        });
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
        self.status_label.set_text(&i18n("Connecting..."));
        self.spinner.set_visible(true);
        #[cfg(not(feature = "adw-1-6"))]
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
            embedded_config.show_local_cursor = config.show_local_cursor;
            embedded_config.accept_certificate = config.accept_certificate;

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
                    tracing::warn!(%e, "Native VNC embedding failed, falling back to external viewer");
                }
            }
        } else {
            // Native VNC not available - for Embedded mode, fall back to external
            tracing::info!(
                "Native VNC embedding not available (vnc-embedded feature not enabled), \
                 using external viewer"
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
                    .set_text(&i18n("Session running in external window"));
                #[cfg(not(feature = "adw-1-6"))]
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
                self.status_label.set_text(&i18n("Connection failed"));
                #[cfg(not(feature = "adw-1-6"))]
                self.spinner.set_spinning(false);
                self.spinner.set_visible(false);
                Err(e)
            }
        }
    }

    /// Spawns an external VNC viewer process with configuration options
    ///
    /// Argument construction is delegated to
    /// [`rustconn_core::protocol::VncProtocol::build_external_viewer_command`]
    /// (single source of truth); this method only builds and spawns the
    /// process. Passwords are handled by the viewer, never on the command line.
    fn spawn_external_viewer_with_config(
        &self,
        viewer: &str,
        host: &str,
        port: u16,
        _password: Option<&str>,
        config: &VncConfig,
    ) -> Result<(), SessionError> {
        let (program, args) =
            VncProtocol::build_external_viewer_command(viewer, host, port, config);

        let mut cmd = Command::new(&program);
        cmd.args(&args);

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

        // Reset state
        *self.is_external.borrow_mut() = false;
        *self.is_embedded_native.borrow_mut() = false;
        *self.state.borrow_mut() = SessionState::Disconnected;
        self.status_label.set_text(&i18n("Disconnected"));
        #[cfg(not(feature = "adw-1-6"))]
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

    /// Connects a callback for state changes
    ///
    /// The callback is invoked whenever the session state changes.
    pub fn connect_state_changed<F>(&self, callback: F)
    where
        F: Fn(SessionState) + 'static,
    {
        *self.state_callback.borrow_mut() = Some(Box::new(callback));
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
