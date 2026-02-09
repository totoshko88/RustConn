//! Protocol registry for looking up protocol handlers by ID

use std::collections::HashMap;
use std::sync::Arc;

use crate::models::ProtocolType;

use super::{Protocol, RdpProtocol, SpiceProtocol, SshProtocol, TelnetProtocol, VncProtocol};

/// Registry for protocol handlers
///
/// The `ProtocolRegistry` provides a centralized way to look up protocol handlers
/// by their identifier or protocol type. It is initialized with all supported
/// protocols and provides thread-safe access to protocol implementations.
pub struct ProtocolRegistry {
    protocols: HashMap<&'static str, Arc<dyn Protocol>>,
}

impl ProtocolRegistry {
    /// Creates a new protocol registry with all supported protocols
    #[must_use]
    pub fn new() -> Self {
        let mut protocols: HashMap<&'static str, Arc<dyn Protocol>> = HashMap::new();

        let ssh = Arc::new(SshProtocol::new());
        let rdp = Arc::new(RdpProtocol::new());
        let vnc = Arc::new(VncProtocol::new());
        let spice = Arc::new(SpiceProtocol::new());
        let telnet = Arc::new(TelnetProtocol::new());

        protocols.insert(ssh.protocol_id(), ssh);
        protocols.insert(rdp.protocol_id(), rdp);
        protocols.insert(vnc.protocol_id(), vnc);
        protocols.insert(spice.protocol_id(), spice);
        protocols.insert(telnet.protocol_id(), telnet);

        Self { protocols }
    }

    /// Gets a protocol handler by its identifier
    ///
    /// # Arguments
    /// * `id` - The protocol identifier (e.g., "ssh", "rdp", "vnc")
    ///
    /// # Returns
    /// The protocol handler if found, or `None` if not registered
    #[must_use]
    pub fn get(&self, id: &str) -> Option<Arc<dyn Protocol>> {
        self.protocols.get(id).cloned()
    }

    /// Gets a protocol handler by protocol type
    ///
    /// # Arguments
    /// * `protocol_type` - The protocol type enum variant
    ///
    /// # Returns
    /// The protocol handler for the given type
    ///
    /// # Panics
    /// Panics if the protocol type is not registered (should never happen with built-in types)
    #[must_use]
    pub fn get_by_type(&self, protocol_type: ProtocolType) -> Option<Arc<dyn Protocol>> {
        let id = match protocol_type {
            ProtocolType::Ssh => "ssh",
            ProtocolType::Rdp => "rdp",
            ProtocolType::Vnc => "vnc",
            ProtocolType::Spice => "spice",
            ProtocolType::Telnet => "telnet",
            ProtocolType::ZeroTrust => "zerotrust",
        };
        self.protocols.get(id).cloned()
    }

    /// Returns all registered protocol IDs
    #[must_use]
    pub fn protocol_ids(&self) -> Vec<&'static str> {
        self.protocols.keys().copied().collect()
    }

    /// Returns the number of registered protocols
    #[must_use]
    pub fn len(&self) -> usize {
        self.protocols.len()
    }

    /// Returns true if no protocols are registered
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.protocols.is_empty()
    }
}

impl Default for ProtocolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
