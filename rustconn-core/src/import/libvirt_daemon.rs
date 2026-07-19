//! Libvirt daemon importer via `virsh` CLI.
//!
//! Connects to a running libvirtd instance using `virsh` and imports
//! VNC/SPICE/RDP graphics connections from defined virtual machines.
//!
//! This complements `LibvirtXmlImporter` (static XML files) by querying
//! the daemon directly — which resolves autoport assignments and discovers
//! VMs that have no on-disk XML (e.g. transient domains).
//!
//! Supported URIs:
//! - `qemu:///session` — user-level VMs (default, no root required)
//! - `qemu:///system`  — system-level VMs (requires libvirt group or root)
//! - `qemu+ssh://host/system` — remote libvirtd over SSH
//!
//! ## Flatpak
//!
//! Inside Flatpak the sandbox has no access to the libvirt socket by
//! default. Users must grant filesystem access manually:
//!
//! ```bash
//! flatpak override --user --filesystem=/run/libvirt io.github.totoshko88.RustConn
//! ```
//!
//! See the User Guide for details.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::libvirt::LibvirtXmlImporter;
use super::traits::{ImportResult, ImportSource, SkippedEntry};
use crate::error::ImportError;

/// Default libvirt URI for user-session VMs.
const DEFAULT_URI: &str = "qemu:///session";

/// Importer that queries a running libvirtd via `virsh`.
///
/// For each defined/running domain it calls `virsh dumpxml <name>` and
/// feeds the output into the existing `LibvirtXmlImporter` XML parser.
pub struct LibvirtDaemonImporter {
    /// Libvirt connection URI (e.g. `qemu:///session`).
    uri: String,
}

impl Default for LibvirtDaemonImporter {
    fn default() -> Self {
        Self::new()
    }
}

impl LibvirtDaemonImporter {
    /// Creates a new importer with the default URI (`qemu:///session`).
    #[must_use]
    pub fn new() -> Self {
        Self {
            uri: DEFAULT_URI.to_string(),
        }
    }

    /// Creates a new importer with a custom libvirt URI.
    #[must_use]
    pub fn with_uri(uri: impl Into<String>) -> Self {
        Self { uri: uri.into() }
    }

    /// Returns the configured URI.
    #[must_use]
    pub fn uri(&self) -> &str {
        &self.uri
    }

    /// Checks whether `virsh` is available on `$PATH`.
    #[must_use]
    pub fn is_virsh_available() -> bool {
        Command::new("virsh").arg("--version").output().is_ok()
    }

    /// Lists all domain names (defined + running) via `virsh list`.
    ///
    /// # Errors
    ///
    /// Returns `ImportError` if `virsh` is not found or the command fails.
    fn list_domains(&self) -> Result<Vec<String>, ImportError> {
        let output = Command::new("virsh")
            .args(["-c", &self.uri, "list", "--all", "--name"])
            .output()
            .map_err(|e| ImportError::ParseError {
                source_name: "libvirt-daemon".to_string(),
                reason: format!(
                    "Failed to run virsh: {e}. \
                     Is libvirt-client installed?"
                ),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ImportError::ParseError {
                source_name: "libvirt-daemon".to_string(),
                reason: format!(
                    "virsh list failed (exit {}): {stderr}",
                    output.status.code().unwrap_or(-1)
                ),
            });
        }

        let names: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect();

        Ok(names)
    }

    /// Dumps the XML definition of a single domain via `virsh dumpxml`.
    fn dump_domain_xml(&self, domain_name: &str) -> Result<String, ImportError> {
        let output = Command::new("virsh")
            .args(["-c", &self.uri, "dumpxml", domain_name])
            .output()
            .map_err(|e| ImportError::ParseError {
                source_name: "libvirt-daemon".to_string(),
                reason: format!("virsh dumpxml {domain_name}: {e}"),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ImportError::ParseError {
                source_name: "libvirt-daemon".to_string(),
                reason: format!("virsh dumpxml {domain_name} failed: {stderr}"),
            });
        }

        String::from_utf8(output.stdout).map_err(|e| ImportError::ParseError {
            source_name: "libvirt-daemon".to_string(),
            reason: format!("Invalid UTF-8 in XML for {domain_name}: {e}"),
        })
    }

    /// Imports all VMs from the configured libvirt daemon.
    ///
    /// # Errors
    ///
    /// Returns an error if `virsh` is not available or the daemon is
    /// unreachable. Individual VM failures are recorded as skipped entries.
    pub fn import_from_daemon(&self) -> Result<ImportResult, ImportError> {
        let domains = self.list_domains()?;

        if domains.is_empty() {
            return Ok(ImportResult::new());
        }

        let mut result = ImportResult::new();
        let source = format!("virsh -c {}", self.uri);

        for name in &domains {
            match self.dump_domain_xml(name) {
                Ok(xml) => {
                    // Reuse the existing XML parser from LibvirtXmlImporter
                    LibvirtXmlImporter::import_domain_xml(&xml, &source, &mut result);
                }
                Err(e) => {
                    result.add_skipped(SkippedEntry::with_location(
                        name,
                        format!("Failed to dump XML: {e}"),
                        &source,
                    ));
                }
            }
        }

        Ok(result)
    }
}

impl ImportSource for LibvirtDaemonImporter {
    fn source_id(&self) -> &'static str {
        "libvirt_daemon"
    }

    fn display_name(&self) -> &'static str {
        "Libvirt Daemon (virsh)"
    }

    fn is_available(&self) -> bool {
        Self::is_virsh_available()
    }

    fn default_paths(&self) -> Vec<PathBuf> {
        Vec::new()
    }

    fn import(&self) -> Result<ImportResult, ImportError> {
        self.import_from_daemon()
    }

    fn import_from_path(&self, _path: &Path) -> Result<ImportResult, ImportError> {
        self.import_from_daemon()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_uri() {
        let importer = LibvirtDaemonImporter::new();
        assert_eq!(importer.uri(), "qemu:///session");
    }

    #[test]
    fn test_custom_uri() {
        let importer = LibvirtDaemonImporter::with_uri("qemu+ssh://server/system");
        assert_eq!(importer.uri(), "qemu+ssh://server/system");
    }

    #[test]
    fn test_source_id() {
        let importer = LibvirtDaemonImporter::new();
        assert_eq!(importer.source_id(), "libvirt_daemon");
    }
}
