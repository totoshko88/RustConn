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

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use auth_page::AuthPage;
use connection_page::ConnectionPage;
use gtk4::prelude::*;
use libadwaita as adw;
use protocol_page::ProtocolPage;
use rustconn_core::models::{
    Connection, ConnectionThemeOverride, ProtocolType, SshAuthMethod, ZeroTrustProvider,
};
use secrecy::SecretString;
use uuid::Uuid;

use crate::i18n::i18n;
use crate::state::SharedAppState;

/// Result from the connection wizard
pub enum WizardResult {
    /// Save the connection without connecting.
    ///
    /// The optional `SecretString` is the password typed in the wizard; the
    /// handler must persist it to the vault (the connection's
    /// `password_source` is already set to `Vault` when this is `Some`).
    Save(Connection, Option<SecretString>),
    /// Save and immediately connect. See [`WizardResult::Save`] for the password.
    SaveAndConnect(Connection, Option<SecretString>),
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
    pub zt_field1: Option<String>,
    pub zt_field2: Option<String>,
    pub zt_field3: Option<String>,
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
    // SPICE unix socket
    pub spice_unix_socket_path: Option<PathBuf>,
}

impl PartialConnection {
    /// Create a `PartialConnection` from an existing `Connection`.
    ///
    /// Extracts wizard-relevant fields from the full connection for
    /// "clone & modify" / "Duplicate via Wizard…" workflows.
    #[must_use]
    pub fn from_connection(conn: &Connection) -> Self {
        use rustconn_core::models::ProtocolConfig;

        let protocol = Some(conn.protocol);
        let name = Some(conn.name.clone());
        let host = Some(conn.host.clone());
        let port = Some(conn.port);
        let username = conn.username.clone();
        let domain = conn.domain.clone();
        let theme_override = conn.theme_override.clone();

        let mut partial = Self {
            protocol,
            name,
            host,
            port,
            username,
            domain,
            theme_override,
            ..Default::default()
        };

        match &conn.protocol_config {
            ProtocolConfig::Ssh(cfg) | ProtocolConfig::Sftp(cfg) => {
                partial.auth_method = Some(cfg.auth_method.clone());
                partial.key_path = cfg.key_path.clone();
                partial.jump_host_id = cfg.jump_host_id;
            }
            ProtocolConfig::Mosh(cfg) => {
                partial.port = cfg.ssh_port.or(partial.port);
            }
            ProtocolConfig::Rdp(cfg) => {
                partial.jump_host_id = cfg.jump_host_id;
            }
            ProtocolConfig::Vnc(cfg) => {
                partial.jump_host_id = cfg.jump_host_id;
            }
            ProtocolConfig::Spice(cfg) => {
                partial.jump_host_id = cfg.jump_host_id;
                partial.spice_unix_socket_path = cfg.unix_socket_path.clone();
            }
            ProtocolConfig::Serial(cfg) => {
                partial.serial_device = Some(cfg.device.clone());
                partial.serial_baud = Some(cfg.baud_rate.value());
            }
            ProtocolConfig::Kubernetes(cfg) => {
                partial.k8s_context = cfg.context.clone();
                partial.k8s_namespace = cfg.namespace.clone();
                partial.k8s_pod = cfg.pod.clone();
                partial.k8s_container = cfg.container.clone();
            }
            ProtocolConfig::ZeroTrust(cfg) => {
                partial.zt_provider = Some(cfg.provider);
                match &cfg.provider_config {
                    rustconn_core::models::ZeroTrustProviderConfig::Generic(g) => {
                        partial.zt_command = Some(g.command_template.clone());
                    }
                    rustconn_core::models::ZeroTrustProviderConfig::AwsSsm(c) => {
                        partial.zt_field1 = Some(c.target.clone());
                        partial.zt_field2 = c.region.clone();
                        partial.zt_field3 = Some(c.profile.clone());
                    }
                    rustconn_core::models::ZeroTrustProviderConfig::GcpIap(c) => {
                        partial.zt_field1 = Some(c.instance.clone());
                        partial.zt_field2 = Some(c.zone.clone());
                        partial.zt_field3 = c.project.clone();
                    }
                    rustconn_core::models::ZeroTrustProviderConfig::AzureBastion(c) => {
                        partial.zt_field1 = Some(c.target_resource_id.clone());
                        partial.zt_field2 = Some(c.resource_group.clone());
                        partial.zt_field3 = Some(c.bastion_name.clone());
                    }
                    rustconn_core::models::ZeroTrustProviderConfig::AzureSsh(c) => {
                        partial.zt_field1 = Some(c.vm_name.clone());
                        partial.zt_field2 = Some(c.resource_group.clone());
                    }
                    rustconn_core::models::ZeroTrustProviderConfig::CloudflareAccess(c) => {
                        partial.zt_field1 = Some(c.hostname.clone());
                    }
                    rustconn_core::models::ZeroTrustProviderConfig::Teleport(c) => {
                        partial.zt_field1 = Some(c.host.clone());
                        partial.zt_field2 = c.cluster.clone();
                    }
                    rustconn_core::models::ZeroTrustProviderConfig::TailscaleSsh(c) => {
                        partial.zt_field1 = Some(c.host.clone());
                    }
                    rustconn_core::models::ZeroTrustProviderConfig::Boundary(c) => {
                        partial.zt_field1 = Some(c.target.clone());
                        partial.zt_field2 = c.addr.clone();
                    }
                    rustconn_core::models::ZeroTrustProviderConfig::HoopDev(c) => {
                        partial.zt_field1 = Some(c.connection_name.clone());
                        partial.zt_field2 = c.gateway_url.clone();
                    }
                    rustconn_core::models::ZeroTrustProviderConfig::OciBastion(_) => {
                        // OCI Bastion has too many fields for wizard
                    }
                }
            }
            ProtocolConfig::Web(_) => {
                partial.url = Some(conn.host.clone());
            }
            ProtocolConfig::Telnet(_) => {}
        }

        partial
    }

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
                let cfg = rustconn_core::models::MoshConfig {
                    ssh_port: Some(port),
                    ..Default::default()
                };
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
                cfg.unix_socket_path = self.spice_unix_socket_path.clone();
                ProtocolConfig::Spice(cfg)
            }
            ProtocolType::Telnet => {
                ProtocolConfig::Telnet(rustconn_core::models::TelnetConfig::default())
            }
            ProtocolType::Serial => {
                let mut cfg = rustconn_core::models::SerialConfig {
                    device: self.serial_device.clone().unwrap_or_default(),
                    ..Default::default()
                };
                if let Some(baud) = self.serial_baud {
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
                let provider = self.zt_provider.unwrap_or(ZeroTrustProvider::Generic);
                let provider_config = Self::build_zt_provider_config(
                    provider,
                    self.zt_command.as_deref(),
                    self.zt_field1.as_deref(),
                    self.zt_field2.as_deref(),
                    self.zt_field3.as_deref(),
                );
                let cfg = rustconn_core::models::ZeroTrustConfig {
                    provider,
                    provider_config,
                    custom_args: Vec::new(),
                    detected_provider: None,
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
        // Mark the source as Vault when a usable password was typed, so the
        // Advanced dialog opens with the correct source pre-selected. The
        // password value itself is transferred separately by the caller.
        if self.has_storable_password() {
            conn.password_source = rustconn_core::models::PasswordSource::Vault;
        }
        // For Web protocol, store URL in host field
        if protocol == ProtocolType::Web
            && let Some(ref url) = self.url
        {
            conn.host = url.clone();
        }
        conn
    }

    /// Returns the typed password if it should be stored (non-empty and the
    /// auth method is not key/agent/security-key based).
    #[must_use]
    pub fn storable_password(&self) -> Option<SecretString> {
        if matches!(
            self.auth_method,
            Some(SshAuthMethod::PublicKey | SshAuthMethod::Agent | SshAuthMethod::SecurityKey)
        ) {
            return None;
        }
        self.password.clone().filter(|p| {
            use secrecy::ExposeSecret;
            !p.expose_secret().is_empty()
        })
    }

    /// Whether a storable password is present (see [`storable_password`]).
    ///
    /// [`storable_password`]: Self::storable_password
    #[must_use]
    pub fn has_storable_password(&self) -> bool {
        self.storable_password().is_some()
    }

    /// Build a `ZeroTrustProviderConfig` from wizard fields
    ///
    /// Delegates to `ZeroTrustProviderConfig::from_wizard_fields` in rustconn-core.
    #[must_use]
    pub fn build_zt_provider_config(
        provider: ZeroTrustProvider,
        command: Option<&str>,
        field1: Option<&str>,
        field2: Option<&str>,
        field3: Option<&str>,
    ) -> rustconn_core::models::ZeroTrustProviderConfig {
        rustconn_core::models::ZeroTrustProviderConfig::from_wizard_fields(
            provider, command, field1, field2, field3,
        )
    }
}

/// The Connection Wizard dialog
#[expect(dead_code, reason = "Fields kept for GTK widget lifecycle")]
pub struct ConnectionWizard {
    dialog: adw::Dialog,
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
    pub fn new(state: SharedAppState) -> Rc<Self> {
        let dialog = adw::Dialog::builder()
            .title(i18n("New Connection"))
            .content_width(600)
            .content_height(580)
            .build();

        let nav_view = adw::NavigationView::new();

        // NavigationView as direct child — each NavigationPage gets its own
        // header bar with automatic back button (GNOME HIG)
        // Set minimum size to avoid AdwDialog warnings
        nav_view.set_width_request(360);
        nav_view.set_height_request(400);
        dialog.set_child(Some(&nav_view));

        let protocol_page = ProtocolPage::new();
        let connection_page = ConnectionPage::new(state.clone());
        let auth_page = AuthPage::new();

        nav_view.push(&protocol_page.page);

        let selected_protocol: Rc<RefCell<Option<ProtocolType>>> = Rc::new(RefCell::new(None));
        let on_complete: WizardCallback = Rc::new(RefCell::new(None));

        let wizard = Rc::new(Self {
            dialog,
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
            let (conn, password) = w.build_connection();
            w.dialog.close();
            if let Some(ref cb) = *w.on_complete.borrow() {
                cb(WizardResult::Save(conn, password));
            }
        });

        let w = wizard.clone();
        wizard.auth_page.connect_save_and_connect(move || {
            let (conn, password) = w.build_connection();
            w.dialog.close();
            if let Some(ref cb) = *w.on_complete.borrow() {
                cb(WizardResult::SaveAndConnect(conn, password));
            }
        });

        let w = wizard.clone();
        wizard.protocol_page.connect_advanced(move || {
            let partial = w.collect_partial();
            w.dialog.close();
            if let Some(ref cb) = *w.on_complete.borrow() {
                cb(WizardResult::OpenAdvanced(partial));
            }
        });

        let w = wizard.clone();
        wizard.connection_page.connect_advanced(move || {
            let partial = w.collect_partial();
            w.dialog.close();
            if let Some(ref cb) = *w.on_complete.borrow() {
                cb(WizardResult::OpenAdvanced(partial));
            }
        });

        let w = wizard.clone();
        wizard.auth_page.connect_advanced(move || {
            let partial = w.collect_partial();
            w.dialog.close();
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
        let zt_fields = if is_zt {
            self.connection_page.zt_fields()
        } else {
            (None, None, None)
        };

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
            zt_provider: if is_zt {
                use rustconn_core::models::ZeroTrustProvider;
                match self.connection_page.zt_provider_index() {
                    0 => Some(ZeroTrustProvider::Generic),
                    1 => Some(ZeroTrustProvider::AwsSsm),
                    2 => Some(ZeroTrustProvider::GcpIap),
                    3 => Some(ZeroTrustProvider::AzureBastion),
                    4 => Some(ZeroTrustProvider::AzureSsh),
                    5 => Some(ZeroTrustProvider::CloudflareAccess),
                    6 => Some(ZeroTrustProvider::Teleport),
                    7 => Some(ZeroTrustProvider::TailscaleSsh),
                    8 => Some(ZeroTrustProvider::Boundary),
                    9 => Some(ZeroTrustProvider::HoopDev),
                    _ => Some(ZeroTrustProvider::Generic),
                }
            } else {
                None
            },
            zt_command: if is_zt {
                self.connection_page.zt_command()
            } else {
                None
            },
            zt_field1: zt_fields.0,
            zt_field2: zt_fields.1,
            zt_field3: zt_fields.2,
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
            spice_unix_socket_path: None, // ponytail: wizard doesn't expose socket path yet; user opens Advanced for this
        }
    }

    /// Build a full Connection from wizard data.
    ///
    /// Returns the connection plus the password typed in the wizard (if any).
    /// When a password is present it is stored in the vault by the caller and
    /// `password_source` is set to `Vault`, mirroring the full dialog. Without
    /// this the password was dropped and the source stayed `None` (issue #188).
    fn build_connection(&self) -> (Connection, Option<SecretString>) {
        use rustconn_core::models::{PasswordSource, ProtocolConfig};

        let partial = self.collect_partial();

        // A typed password (when not key/agent-based) is persisted to the vault
        // by the caller; mark the source accordingly. Without this the password
        // was dropped and password_source stayed None (issue #188).
        let stored_password = partial.storable_password();

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
                    let mosh_cfg = rustconn_core::models::MoshConfig {
                        ssh_port: Some(port),
                        ..Default::default()
                    };
                    ProtocolConfig::Mosh(mosh_cfg)
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
                cfg.unix_socket_path = partial.spice_unix_socket_path.clone();
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
                let provider = partial.zt_provider.unwrap_or(ZeroTrustProvider::Generic);
                let provider_config = PartialConnection::build_zt_provider_config(
                    provider,
                    partial.zt_command.as_deref(),
                    partial.zt_field1.as_deref(),
                    partial.zt_field2.as_deref(),
                    partial.zt_field3.as_deref(),
                );
                let zt_cfg = rustconn_core::models::ZeroTrustConfig {
                    provider,
                    provider_config,
                    custom_args: Vec::new(),
                    detected_provider: None,
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
        // A typed password is persisted to the configured vault by the caller.
        if stored_password.is_some() {
            conn.password_source = PasswordSource::Vault;
        }
        // Use auth_page icon if set, otherwise inherit from selected template
        let icon = self.auth_page.icon();
        conn.icon = if icon.is_some() {
            icon
        } else {
            self.connection_page.selected_template_icon()
        };
        (conn, stored_password)
    }

    /// Pre-fill the wizard from a `PartialConnection` (e.g. "Duplicate via Wizard").
    ///
    /// Sets the protocol, navigates to the connection page (step 2), and
    /// populates host/port/username/domain/name fields.
    pub fn set_partial(&self, partial: &PartialConnection) {
        let Some(protocol) = partial.protocol else {
            return;
        };

        // Set selected protocol
        *self.selected_protocol.borrow_mut() = Some(protocol);

        // Configure connection page for this protocol
        self.connection_page.configure_for_protocol(protocol);

        // Pre-fill connection page fields
        if let Some(ref name) = partial.name {
            self.connection_page.name_row.set_text(name);
        }
        if let Some(ref host) = partial.host {
            self.connection_page.host_row.set_text(host);
        }
        if let Some(port) = partial.port {
            self.connection_page.port_row.set_value(f64::from(port));
        }
        if let Some(ref username) = partial.username {
            self.connection_page.username_row.set_text(username);
        }
        if let Some(ref domain) = partial.domain {
            self.connection_page.domain_row.set_text(domain);
        }

        // Navigate directly to connection page (skip protocol selection)
        self.nav_view.push(&self.connection_page.page);

        // Update dialog title to indicate duplication
        self.dialog.set_title(&i18n("Duplicate Connection"));
    }

    /// Present the wizard dialog
    pub fn present(&self, parent: &impl IsA<gtk4::Widget>) {
        self.dialog.present(Some(parent));
    }

    /// Connect completion callback
    pub fn connect_complete<F: Fn(WizardResult) + 'static>(&self, f: F) {
        *self.on_complete.borrow_mut() = Some(Box::new(f));
    }
}
