//! Connection Wizard — step-by-step new connection creation
//!
//! A simplified 3-step wizard for creating connections:
//! 1. Protocol selection (grouped logically)
//! 2. Connection details (adaptive per protocol)
//! 3. Authentication + color profile + finish
//!
//! The wizard provides a streamlined experience for new users while
//! offering an "Advanced..." escape hatch to the full ConnectionDialog.

mod auth_page;
mod connection_page;
mod protocol_page;

use crate::i18n::i18n;
use crate::state::SharedAppState;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use rustconn_core::models::{
    Connection, ConnectionThemeOverride, ProtocolType, SshAuthMethod, ZeroTrustProvider,
};
use secrecy::SecretString;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use uuid::Uuid;

use auth_page::AuthPage;
use connection_page::ConnectionPage;
use protocol_page::ProtocolPage;

/// Result from the connection wizard
pub enum WizardResult {
    /// Save the connection without connecting
    Save(Connection),
    /// Save and immediately connect
    SaveAndConnect(Connection),
    /// Open the full ConnectionDialog with pre-filled data
    OpenAdvanced(PartialConnection),
}

/// Callback type for wizard completion
pub type WizardCallback = Rc<RefCell<Option<Box<dyn Fn(WizardResult)>>>>;

/// Partial connection data collected across wizard steps.
/// Used to transfer state between pages and to the full dialog.
#[derive(Debug, Clone, Default)]
pub struct PartialConnection {
    pub protocol: Option<ProtocolType>,
    pub name: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub password: Option<SecretString>,
    pub domain: Option<String>,
    pub auth_method: Option<SshAuthMethod>,
    pub key_path: Option<PathBuf>,
    pub jump_host_id: Option<Uuid>,
    pub theme_override: Option<ConnectionThemeOverride>,
    // Zero Trust
    pub zt_provider: Option<ZeroTrustProvider>,
    pub zt_command: Option<String>,
    // Serial
    pub serial_device: Option<String>,
    pub serial_baud: Option<u32>,
    // Kubernetes
    pub k8s_context: Option<String>,
    pub k8s_namespace: Option<String>,
    pub k8s_pod: Option<String>,
    pub k8s_container: Option<String>,
    // Web
    pub url: Option<String>,
}

impl PartialConnection {
    /// Generate an auto-name based on protocol and host/device/pod
    #[must_use]
    pub fn auto_name(&self) -> String {
        let proto_str = self.protocol.map(|p| p.to_string()).unwrap_or_default();

        if let Some(ref host) = self.host
            && !host.is_empty()
        {
            return format!("{proto_str}: {host}");
        }
        if let Some(ref device) = self.serial_device
            && !device.is_empty()
        {
            return format!("Serial: {device}");
        }
        if let Some(ref pod) = self.k8s_pod
            && !pod.is_empty()
        {
            let ns = self.k8s_namespace.as_deref().unwrap_or("default");
            return format!("k8s: {ns}/{pod}");
        }
        if let Some(ref url) = self.url
            && !url.is_empty()
        {
            let domain = url
                .strip_prefix("https://")
                .or_else(|| url.strip_prefix("http://"))
                .unwrap_or(url)
                .split('/')
                .next()
                .unwrap_or(url);
            return format!("Web: {domain}");
        }
        proto_str
    }

    /// Convert to a full `Connection` for pre-filling the Advanced dialog.
    ///
    /// Uses default values for fields not set in the partial data.
    #[must_use]
    pub fn to_connection(&self) -> Connection {
        use rustconn_core::models::ProtocolConfig;

        let protocol = self.protocol.unwrap_or(ProtocolType::Ssh);
        let host = self.host.clone().unwrap_or_default();
        let port = self.port.unwrap_or_else(|| protocol.default_port());
        let name = self
            .name
            .clone()
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| self.auto_name());

        let protocol_config = match protocol {
            ProtocolType::Ssh => {
                let mut cfg = rustconn_core::models::SshConfig::default();
                if let Some(ref method) = self.auth_method {
                    cfg.auth_method = method.clone();
                }
                if let Some(ref key) = self.key_path {
                    cfg.key_path = Some(key.clone());
                }
                if let Some(jump_id) = self.jump_host_id {
                    cfg.jump_host_id = Some(jump_id);
                }
                ProtocolConfig::Ssh(cfg)
            }
            ProtocolType::Mosh => {
                let cfg = rustconn_core::models::MoshConfig::default();
                ProtocolConfig::Mosh(cfg)
            }
            ProtocolType::Sftp => {
                let mut cfg = rustconn_core::models::SshConfig::default();
                if let Some(ref method) = self.auth_method {
                    cfg.auth_method = method.clone();
                }
                if let Some(ref key) = self.key_path {
                    cfg.key_path = Some(key.clone());
                }
                if let Some(jump_id) = self.jump_host_id {
                    cfg.jump_host_id = Some(jump_id);
                }
                ProtocolConfig::Sftp(cfg)
            }
            ProtocolType::Rdp => {
                let mut cfg = rustconn_core::models::RdpConfig::default();
                if let Some(jump_id) = self.jump_host_id {
                    cfg.jump_host_id = Some(jump_id);
                }
                ProtocolConfig::Rdp(cfg)
            }
            ProtocolType::Vnc => {
                let mut cfg = rustconn_core::models::VncConfig::default();
                if let Some(jump_id) = self.jump_host_id {
                    cfg.jump_host_id = Some(jump_id);
                }
                ProtocolConfig::Vnc(cfg)
            }
            ProtocolType::Spice => {
                let mut cfg = rustconn_core::models::SpiceConfig::default();
                if let Some(jump_id) = self.jump_host_id {
                    cfg.jump_host_id = Some(jump_id);
                }
                ProtocolConfig::Spice(cfg)
            }
            ProtocolType::Telnet => {
                ProtocolConfig::Telnet(rustconn_core::models::TelnetConfig::default())
            }
            ProtocolType::Serial => {
                let cfg = rustconn_core::models::SerialConfig {
                    device: self.serial_device.clone().unwrap_or_default(),
                    ..Default::default()
                };
                ProtocolConfig::Serial(cfg)
            }
            ProtocolType::Kubernetes => {
                let cfg = rustconn_core::models::KubernetesConfig {
                    context: self.k8s_context.clone(),
                    namespace: self.k8s_namespace.clone(),
                    pod: self.k8s_pod.clone(),
                    container: self.k8s_container.clone(),
                    ..Default::default()
                };
                ProtocolConfig::Kubernetes(cfg)
            }
            ProtocolType::Web => {
                // Web protocol stores URL in Connection.host
                ProtocolConfig::Web(rustconn_core::models::WebConfig::default())
            }
            ProtocolType::ZeroTrust => {
                let cfg = rustconn_core::models::ZeroTrustConfig {
                    provider: self.zt_provider.unwrap_or(ZeroTrustProvider::Generic),
                    ..Default::default()
                };
                ProtocolConfig::ZeroTrust(cfg)
            }
        };

        let mut conn = Connection::new(name, host, port, protocol_config);
        conn.username = self.username.clone();
        conn.domain = self.domain.clone();
        if let Some(ref theme) = self.theme_override {
            conn.theme_override = Some(theme.clone());
        }
        // For Web protocol, store URL in host field
        if protocol == ProtocolType::Web
            && let Some(ref url) = self.url
        {
            conn.host = url.clone();
        }
        conn
    }
}

/// The Connection Wizard dialog
#[allow(dead_code)] // Fields kept for GTK widget lifecycle
pub struct ConnectionWizard {
    window: adw::Window,
    nav_view: adw::NavigationView,
    protocol_page: ProtocolPage,
    connection_page: ConnectionPage,
    auth_page: AuthPage,
    selected_protocol: Rc<RefCell<Option<ProtocolType>>>,
    state: SharedAppState,
    on_complete: WizardCallback,
}

impl ConnectionWizard {
    /// Creates a new Connection Wizard
    #[must_use]
    pub fn new(parent: Option<&gtk4::Window>, state: SharedAppState) -> Rc<Self> {
        let window = adw::Window::builder()
            .title(i18n("New Connection"))
            .modal(true)
            .default_width(500)
            .default_height(520)
            .build();

        if let Some(parent_win) = parent {
            window.set_transient_for(Some(parent_win));
        }

        let nav_view = adw::NavigationView::new();

        let toolbar_view = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&nav_view));
        window.set_content(Some(&toolbar_view));

        let protocol_page = ProtocolPage::new();
        let connection_page = ConnectionPage::new(state.clone());
        let auth_page = AuthPage::new();

        nav_view.push(&protocol_page.page);

        let selected_protocol: Rc<RefCell<Option<ProtocolType>>> = Rc::new(RefCell::new(None));
        let on_complete: WizardCallback = Rc::new(RefCell::new(None));

        let wizard = Rc::new(Self {
            window,
            nav_view,
            protocol_page,
            connection_page,
            auth_page,
            selected_protocol,
            state,
            on_complete,
        });

        Self::wire_callbacks(&wizard);
        wizard
    }

    /// Wire up all inter-page callbacks
    fn wire_callbacks(wizard: &Rc<Self>) {
        let w = wizard.clone();
        wizard
            .protocol_page
            .connect_protocol_selected(move |protocol, is_custom_cmd| {
                *w.selected_protocol.borrow_mut() = Some(protocol);
                w.connection_page.configure_for_protocol(protocol);
                if is_custom_cmd {
                    w.connection_page.set_custom_command_mode();
                }
                w.nav_view.push(&w.connection_page.page);
            });

        let w = wizard.clone();
        wizard.connection_page.connect_next(move || {
            if let Some(protocol) = *w.selected_protocol.borrow() {
                let host = w.connection_page.host();
                let port = w.connection_page.port();
                w.auth_page.configure_for_protocol(protocol, &host, port);
                w.nav_view.push(&w.auth_page.page);
            }
        });

        let w = wizard.clone();
        wizard.auth_page.connect_save(move || {
            let conn = w.build_connection();
            w.window.close();
            if let Some(ref cb) = *w.on_complete.borrow() {
                cb(WizardResult::Save(conn));
            }
        });

        let w = wizard.clone();
        wizard.auth_page.connect_save_and_connect(move || {
            let conn = w.build_connection();
            w.window.close();
            if let Some(ref cb) = *w.on_complete.borrow() {
                cb(WizardResult::SaveAndConnect(conn));
            }
        });

        let w = wizard.clone();
        wizard.protocol_page.connect_advanced(move || {
            let partial = w.collect_partial();
            w.window.close();
            if let Some(ref cb) = *w.on_complete.borrow() {
                cb(WizardResult::OpenAdvanced(partial));
            }
        });

        let w = wizard.clone();
        wizard.connection_page.connect_advanced(move || {
            let partial = w.collect_partial();
            w.window.close();
            if let Some(ref cb) = *w.on_complete.borrow() {
                cb(WizardResult::OpenAdvanced(partial));
            }
        });

        let w = wizard.clone();
        wizard.auth_page.connect_advanced(move || {
            let partial = w.collect_partial();
            w.window.close();
            if let Some(ref cb) = *w.on_complete.borrow() {
                cb(WizardResult::OpenAdvanced(partial));
            }
        });
    }

    /// Collect partial connection data from all pages
    fn collect_partial(&self) -> PartialConnection {
        let protocol = *self.selected_protocol.borrow();
        let is_serial = protocol == Some(ProtocolType::Serial);
        let is_k8s = protocol == Some(ProtocolType::Kubernetes);
        let is_zt = protocol == Some(ProtocolType::ZeroTrust);
        let is_web = protocol == Some(ProtocolType::Web);

        PartialConnection {
            protocol,
            name: Some(self.connection_page.name()).filter(|s| !s.is_empty()),
            host: Some(self.connection_page.host()).filter(|s| !s.is_empty()),
            port: Some(self.connection_page.port()),
            username: self.connection_page.username(),
            password: self.auth_page.password(),
            domain: self.connection_page.domain(),
            auth_method: protocol.and_then(|p| {
                if matches!(
                    p,
                    ProtocolType::Ssh | ProtocolType::Mosh | ProtocolType::Sftp
                ) {
                    Some(self.auth_page.auth_method())
                } else {
                    None
                }
            }),
            key_path: self.auth_page.key_path(),
            jump_host_id: self.connection_page.selected_jump_host(),
            theme_override: self.auth_page.theme_override(),
            zt_provider: None,
            zt_command: if is_zt {
                self.connection_page.zt_command()
            } else {
                None
            },
            serial_device: if is_serial {
                Some(self.connection_page.serial_device()).filter(|s| !s.is_empty())
            } else {
                None
            },
            serial_baud: if is_serial {
                Some(self.connection_page.serial_baud())
            } else {
                None
            },
            k8s_context: if is_k8s {
                self.connection_page.k8s_context()
            } else {
                None
            },
            k8s_namespace: if is_k8s {
                Some(self.connection_page.k8s_namespace())
            } else {
                None
            },
            k8s_pod: if is_k8s {
                Some(self.connection_page.k8s_pod()).filter(|s| !s.is_empty())
            } else {
                None
            },
            k8s_container: if is_k8s {
                self.connection_page.k8s_container()
            } else {
                None
            },
            url: if is_web {
                Some(self.connection_page.url()).filter(|s| !s.is_empty())
            } else {
                None
            },
        }
    }

    /// Build a full Connection from wizard data
    fn build_connection(&self) -> Connection {
        use rustconn_core::models::ProtocolConfig;

        let partial = self.collect_partial();
        let protocol = partial.protocol.unwrap_or(ProtocolType::Ssh);
        let name = partial
            .name
            .clone()
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| partial.auto_name());
        let host = if protocol == ProtocolType::Web {
            partial.url.clone().unwrap_or_default()
        } else {
            partial.host.clone().unwrap_or_default()
        };
        let port = partial.port.unwrap_or_else(|| protocol.default_port());

        let protocol_config = match protocol {
            ProtocolType::Ssh | ProtocolType::Mosh | ProtocolType::Sftp => {
                let mut cfg = rustconn_core::models::SshConfig::default();
                if let Some(method) = partial.auth_method {
                    cfg.auth_method = method;
                }
                if let Some(ref key) = partial.key_path {
                    cfg.key_path = Some(key.clone());
                }
                if let Some(jump_id) = partial.jump_host_id {
                    cfg.jump_host_id = Some(jump_id);
                }
                if protocol == ProtocolType::Sftp {
                    ProtocolConfig::Sftp(cfg)
                } else if protocol == ProtocolType::Mosh {
                    ProtocolConfig::Mosh(rustconn_core::models::MoshConfig::default())
                } else {
                    ProtocolConfig::Ssh(cfg)
                }
            }
            ProtocolType::Rdp => {
                let mut cfg = rustconn_core::models::RdpConfig::default();
                if let Some(jump_id) = partial.jump_host_id {
                    cfg.jump_host_id = Some(jump_id);
                }
                ProtocolConfig::Rdp(cfg)
            }
            ProtocolType::Vnc => {
                let mut cfg = rustconn_core::models::VncConfig::default();
                if let Some(jump_id) = partial.jump_host_id {
                    cfg.jump_host_id = Some(jump_id);
                }
                ProtocolConfig::Vnc(cfg)
            }
            ProtocolType::Spice => {
                let mut cfg = rustconn_core::models::SpiceConfig::default();
                if let Some(jump_id) = partial.jump_host_id {
                    cfg.jump_host_id = Some(jump_id);
                }
                ProtocolConfig::Spice(cfg)
            }
            ProtocolType::Telnet => {
                ProtocolConfig::Telnet(rustconn_core::models::TelnetConfig::default())
            }
            ProtocolType::Serial => {
                let mut cfg = rustconn_core::models::SerialConfig::default();
                if let Some(ref device) = partial.serial_device {
                    cfg.device = device.clone();
                }
                if let Some(baud) = partial.serial_baud {
                    cfg.baud_rate = match baud {
                        9600 => rustconn_core::models::SerialBaudRate::B9600,
                        19200 => rustconn_core::models::SerialBaudRate::B19200,
                        38400 => rustconn_core::models::SerialBaudRate::B38400,
                        57600 => rustconn_core::models::SerialBaudRate::B57600,
                        230_400 => rustconn_core::models::SerialBaudRate::B230400,
                        460_800 => rustconn_core::models::SerialBaudRate::B460800,
                        _ => rustconn_core::models::SerialBaudRate::B115200,
                    };
                }
                ProtocolConfig::Serial(cfg)
            }
            ProtocolType::Kubernetes => {
                let cfg = rustconn_core::models::KubernetesConfig {
                    context: partial.k8s_context,
                    namespace: partial.k8s_namespace,
                    pod: partial.k8s_pod,
                    container: partial.k8s_container,
                    ..rustconn_core::models::KubernetesConfig::default()
                };
                ProtocolConfig::Kubernetes(cfg)
            }
            ProtocolType::ZeroTrust => {
                let zt_cfg = if let Some(ref cmd) = partial.zt_command {
                    rustconn_core::models::ZeroTrustConfig {
                        provider: ZeroTrustProvider::Generic,
                        provider_config: rustconn_core::models::ZeroTrustProviderConfig::Generic(
                            rustconn_core::models::GenericZeroTrustConfig {
                                command_template: cmd.clone(),
                            },
                        ),
                        custom_args: Vec::new(),
                        detected_provider: None,
                    }
                } else {
                    rustconn_core::models::ZeroTrustConfig::default()
                };
                ProtocolConfig::ZeroTrust(zt_cfg)
            }
            ProtocolType::Web => {
                // Web stores URL in Connection.host field
                ProtocolConfig::Web(rustconn_core::models::WebConfig::default())
            }
        };

        let mut conn = Connection::new(name, host, port, protocol_config);
        conn.username = partial.username;
        conn.domain = partial.domain;
        conn.theme_override = partial.theme_override;
        conn.icon = self.auth_page.icon();
        conn
    }

    /// Present the wizard window
    pub fn present(&self) {
        self.window.present();
    }

    /// Connect completion callback
    pub fn connect_complete<F: Fn(WizardResult) + 'static>(&self, f: F) {
        *self.on_complete.borrow_mut() = Some(Box::new(f));
    }
}
