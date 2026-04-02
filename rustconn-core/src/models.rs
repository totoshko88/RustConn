//! Core data models for `RustConn`
//!
//! This module defines the primary data structures used throughout `RustConn`,
//! including connections, groups, credentials, snippets, templates, and history.

mod connection;
mod credentials;
mod custom_property;
mod group;
mod highlight;
mod history;
mod protocol;
mod smart_folder;
mod snippet;
mod template;

pub use connection::{
    AutomationConfig, Connection, ConnectionThemeOverride, PasswordSource, WindowGeometry,
    WindowMode,
};
pub use credentials::Credentials;
pub use custom_property::{CustomProperty, PropertyType};
pub use group::ConnectionGroup;
pub use highlight::HighlightRule;
pub use history::{ConnectionHistoryEntry, ConnectionStatistics, HistorySettings};
pub use protocol::ProtocolType;
pub use protocol::{
    AwsSsmConfig, AzureBastionConfig, AzureSshConfig, BoundaryConfig, CloudflareAccessConfig,
    GcpIapConfig, GenericZeroTrustConfig, HoopDevConfig, KubernetesConfig, MoshConfig,
    MoshPredictMode, OciBastionConfig, PortForward, PortForwardDirection, ProtocolConfig,
    RdpClientMode, RdpConfig, RdpGateway, RdpPerformanceMode, Resolution, ScaleOverride,
    SerialBaudRate, SerialConfig, SerialDataBits, SerialFlowControl, SerialParity, SerialStopBits,
    SharedFolder, SpiceConfig, SpiceImageCompression, SshAuthMethod, SshConfig, SshKeySource,
    TailscaleSshConfig, TeleportConfig, TelnetBackspaceSends, TelnetConfig, TelnetDeleteSends,
    VncClientMode, VncConfig, VncPerformanceMode, ZeroTrustConfig, ZeroTrustProvider,
    ZeroTrustProviderConfig,
};
pub use smart_folder::SmartFolder;
pub use snippet::{Snippet, SnippetVariable};
pub use template::{ConnectionTemplate, TemplateError, group_templates_by_protocol};
