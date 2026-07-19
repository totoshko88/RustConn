//! Core data models for `RustConn`
//!
//! This module defines the primary data structures used throughout `RustConn`,
//! including connections, groups, credentials, snippets, templates, and history.

mod connection;
mod credentials;
mod custom_property;
mod dynamic_folder;
mod group;
mod highlight;
mod history;
mod protocol;
mod smart_folder;
mod snippet;
mod template;
mod tunnel;
mod workspace;

pub use connection::{
    AutomationConfig, Connection, ConnectionThemeOverride, PasswordSource, WindowGeometry,
    WindowMode,
};
pub use credentials::Credentials;
pub use custom_property::{CustomProperty, PropertyType};
pub use dynamic_folder::{DynamicConnectionEntry, DynamicFolderConfig, DynamicFolderResult};
pub use group::{ConnectionGroup, collect_descendant_group_ids};
pub use highlight::HighlightRule;
pub use history::{ConnectionHistoryEntry, ConnectionStatistics, HistorySettings};
pub use protocol::{
    AwsSsmConfig, AzureBastionConfig, AzureSshConfig, BoundaryConfig, CloudflareAccessConfig,
    GcpIapConfig, GenericZeroTrustConfig, HoopDevConfig, KubernetesConfig, MoshConfig,
    MoshPredictMode, OciBastionConfig, PortForward, PortForwardDirection, ProtocolConfig,
    ProtocolType, RdpClientMode, RdpConfig, RdpGateway, RdpPerformanceMode, RdpSecurityLayer,
    Resolution, ScaleOverride, SerialBaudRate, SerialConfig, SerialDataBits, SerialFlowControl,
    SerialParity, SerialStopBits, SharedFolder, SpiceConfig, SpiceImageCompression, SshAuthMethod,
    SshConfig, SshKeySource, TailscaleSshConfig, TeleportConfig, TelnetBackspaceSends,
    TelnetConfig, TelnetDeleteSends, VncClientMode, VncConfig, VncPerformanceMode, WebBrowserMode,
    WebConfig, ZeroTrustConfig, ZeroTrustProvider, ZeroTrustProviderConfig,
    build_remote_app_freerdp_args,
};
pub use smart_folder::SmartFolder;
pub use snippet::{ScriptDelivery, Snippet, SnippetTarget, SnippetVariable};
pub use template::{ConnectionTemplate, TemplateError, group_templates_by_protocol};
pub use tunnel::{StandaloneTunnel, TunnelStatus};
pub use workspace::{WorkspaceEntry, WorkspaceProfile, WorkspaceSplitLayout};
