//! Core data models for `RustConn`
//!
//! This module defines the primary data structures used throughout `RustConn`,
//! including connections, groups, credentials, snippets, and templates.

mod connection;
mod credentials;
mod custom_property;
mod group;
mod protocol;
mod snippet;
mod template;

pub use connection::{AutomationConfig, Connection, PasswordSource, WindowGeometry, WindowMode};
pub use credentials::Credentials;
pub use custom_property::{CustomProperty, PropertyType};
pub use group::ConnectionGroup;
pub use protocol::ProtocolType;
pub use protocol::{
    AwsSsmConfig, AzureBastionConfig, AzureSshConfig, BoundaryConfig, CloudflareAccessConfig,
    GcpIapConfig, GenericZeroTrustConfig, OciBastionConfig, ProtocolConfig, RdpClientMode,
    RdpConfig, RdpGateway, Resolution, SharedFolder, SpiceConfig, SpiceImageCompression,
    SshAuthMethod, SshConfig, SshKeySource, TailscaleSshConfig, TeleportConfig, VncClientMode,
    VncConfig, ZeroTrustConfig, ZeroTrustProvider, ZeroTrustProviderConfig,
};
pub use snippet::{Snippet, SnippetVariable};
pub use template::{group_templates_by_protocol, ConnectionTemplate, TemplateError};
