//! Libvirt domain XML importer.
//!
//! Parses libvirt domain XML files to extract VNC, SPICE, and RDP graphics
//! connections from virtual machines. Covers both system-level libvirt
//! (`/etc/libvirt/qemu/`) and user-session VMs (`~/.config/libvirt/qemu/`),
//! including GNOME Boxes which uses the same format.
//!
//! Also supports importing a single XML file (e.g. output of `virsh dumpxml`).
//!
//! Reference: <https://libvirt.org/formatdomain.html#graphical-framebuffers>

use std::path::{Path, PathBuf};

use quick_xml::Reader;
use quick_xml::events::Event;
use secrecy::SecretString;

use crate::error::ImportError;
use crate::models::{
    Connection, ConnectionGroup, Credentials, PasswordSource, ProtocolConfig, SpiceConfig,
    VncConfig,
};

use super::traits::{ImportResult, ImportSource, SkippedEntry, read_import_file};

/// Default VNC port used by libvirt when autoport is enabled.
const DEFAULT_VNC_PORT: u16 = 5900;

/// Default SPICE port used by libvirt when autoport is enabled.
const DEFAULT_SPICE_PORT: u16 = 5900;

/// Default RDP port.
const DEFAULT_RDP_PORT: u16 = 3389;

/// Maximum file size for libvirt XML (10 MB).
const MAX_XML_SIZE: u64 = 10 * 1024 * 1024;

/// A single `<graphics>` element parsed from a libvirt domain XML.
#[derive(Debug, Clone)]
struct GraphicsEntry {
    /// Graphics type: "vnc", "spice", or "rdp"
    graphics_type: String,
    /// TCP port (-1 means autoport / not yet allocated)
    port: i32,
    /// TLS port for SPICE
    tls_port: Option<i32>,
    /// Listen address (from attribute or nested `<listen>` element)
    listen_address: Option<String>,
    /// Password (clear-text in XML, stored as `SecretString`)
    password: Option<String>,
    /// Whether autoport is enabled
    autoport: bool,
}

/// Parsed domain metadata from a libvirt XML file.
#[derive(Debug, Clone)]
struct DomainInfo {
    /// VM name (`<name>` element)
    name: String,
    /// VM UUID (`<uuid>` element)
    uuid: Option<String>,
    /// VM description (`<description>` element)
    description: Option<String>,
    /// Graphics devices from `<devices><graphics>` elements
    graphics: Vec<GraphicsEntry>,
}

/// Importer for libvirt domain XML files.
///
/// Scans standard libvirt directories for domain definitions and extracts
/// VNC/SPICE/RDP graphics connections. Also covers GNOME Boxes VMs which
/// use the same libvirt XML format under `~/.config/libvirt/qemu/`.
pub struct LibvirtXmlImporter;

impl Default for LibvirtXmlImporter {
    fn default() -> Self {
        Self::new()
    }
}

impl LibvirtXmlImporter {
    /// Creates a new libvirt XML importer.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Returns the standard directories where libvirt stores domain XMLs.
    fn libvirt_xml_dirs() -> Vec<PathBuf> {
        let mut dirs = Vec::new();

        // User session (GNOME Boxes, user-level QEMU/KVM)
        if let Some(config) = dirs::config_dir() {
            dirs.push(config.join("libvirt/qemu"));
        }

        // System-level (requires read access, may need root)
        dirs.push(PathBuf::from("/etc/libvirt/qemu"));

        dirs
    }

    /// Parses a single libvirt domain XML string into `DomainInfo`.
    fn parse_domain_xml(content: &str) -> Option<DomainInfo> {
        let content = content.trim_start_matches('\u{feff}');
        let mut reader = Reader::from_str(content);
        reader.config_mut().trim_text(true);

        let mut domain = DomainInfo {
            name: String::new(),
            uuid: None,
            description: None,
            graphics: Vec::new(),
        };

        let mut depth: Vec<String> = Vec::new();
        let mut text_buf = String::new();

        loop {
            match reader.read_event() {
                Ok(Event::Start(e)) => {
                    let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();

                    if tag == "graphics" {
                        let entry = Self::parse_graphics_attributes(&e);
                        domain.graphics.push(entry);
                    }

                    // Track <listen> inside <graphics> for address extraction
                    if tag == "listen"
                        && depth.last().is_some_and(|d| d == "graphics")
                        && let Some(last_gfx) = domain.graphics.last_mut()
                    {
                        Self::parse_listen_element(&e, last_gfx);
                    }

                    depth.push(tag);
                    text_buf.clear();
                }
                Ok(Event::Empty(e)) => {
                    let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();

                    // Self-closing <graphics .../> (common in libvirt XML)
                    if tag == "graphics" {
                        let entry = Self::parse_graphics_attributes(&e);
                        domain.graphics.push(entry);
                    }

                    // Self-closing <listen .../> inside <graphics>
                    if tag == "listen"
                        && depth.last().is_some_and(|d| d == "graphics")
                        && let Some(last_gfx) = domain.graphics.last_mut()
                    {
                        Self::parse_listen_element(&e, last_gfx);
                    }
                }
                Ok(Event::Text(e)) => {
                    text_buf = String::from_utf8_lossy(&e).to_string();
                }
                Ok(Event::End(_)) => {
                    if let Some(tag) = depth.pop() {
                        // Only capture direct children of <domain>
                        let parent_is_domain = depth.last().is_some_and(|p| p == "domain");
                        match tag.as_str() {
                            "name" if parent_is_domain => {
                                domain.name = text_buf.trim().to_string();
                            }
                            "uuid" if parent_is_domain => {
                                let val = text_buf.trim().to_string();
                                if !val.is_empty() {
                                    domain.uuid = Some(val);
                                }
                            }
                            "description" if parent_is_domain => {
                                let val = text_buf.trim().to_string();
                                if !val.is_empty() {
                                    domain.description = Some(val);
                                }
                            }
                            _ => {}
                        }
                    }
                    text_buf.clear();
                }
                Ok(Event::Eof) => break,
                Err(_) => break,
                _ => {} // Skip CData, Decl, PI, Comment, DocType
            }
        }

        if domain.name.is_empty() && domain.graphics.is_empty() {
            return None;
        }

        Some(domain)
    }

    /// Extracts attributes from a `<graphics>` element.
    fn parse_graphics_attributes(e: &quick_xml::events::BytesStart<'_>) -> GraphicsEntry {
        let mut entry = GraphicsEntry {
            graphics_type: String::new(),
            port: -1,
            tls_port: None,
            listen_address: None,
            password: None,
            autoport: false,
        };

        for attr in e.attributes().flatten() {
            let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
            let val = String::from_utf8_lossy(&attr.value).to_string();
            match key.as_str() {
                "type" => entry.graphics_type = val.to_lowercase(),
                "port" => entry.port = val.parse().unwrap_or(-1),
                "tlsPort" | "tls-port" => {
                    entry.tls_port = val.parse().ok();
                }
                "autoport" => entry.autoport = val.eq_ignore_ascii_case("yes"),
                "listen" => {
                    if !val.is_empty() {
                        entry.listen_address = Some(val);
                    }
                }
                "passwd" | "password" => {
                    if !val.is_empty() {
                        entry.password = Some(val);
                    }
                }
                _ => {}
            }
        }

        entry
    }

    /// Extracts the address from a `<listen>` sub-element of `<graphics>`.
    fn parse_listen_element(e: &quick_xml::events::BytesStart<'_>, gfx: &mut GraphicsEntry) {
        let mut listen_type = String::new();
        let mut address = String::new();

        for attr in e.attributes().flatten() {
            let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
            let val = String::from_utf8_lossy(&attr.value).to_string();
            match key.as_str() {
                "type" => listen_type = val,
                "address" => address = val,
                _ => {}
            }
        }

        // Only use address-type listeners
        if listen_type == "address" && !address.is_empty() {
            gfx.listen_address = Some(address);
        }
    }

    /// Converts a parsed `DomainInfo` into one or more `Connection` objects.
    ///
    /// Each `<graphics>` element becomes a separate connection. If a VM has
    /// both VNC and SPICE, two connections are created (e.g. "myvm (VNC)",
    /// "myvm (SPICE)").
    fn domain_to_connections(
        domain: &DomainInfo,
        source_path: &str,
        result: &mut ImportResult,
    ) -> Vec<Connection> {
        let mut connections = Vec::new();

        if domain.graphics.is_empty() {
            result.add_skipped(SkippedEntry::with_location(
                &domain.name,
                "No <graphics> element found — VM has no graphical console",
                source_path,
            ));
            return connections;
        }

        let multi_graphics = domain.graphics.len() > 1;

        for gfx in &domain.graphics {
            let Some(conn) =
                Self::graphics_to_connection(domain, gfx, multi_graphics, source_path, result)
            else {
                continue;
            };
            connections.push(conn);
        }

        connections
    }

    /// Converts a single `GraphicsEntry` into a `Connection`.
    fn graphics_to_connection(
        domain: &DomainInfo,
        gfx: &GraphicsEntry,
        multi_graphics: bool,
        source_path: &str,
        result: &mut ImportResult,
    ) -> Option<Connection> {
        let (protocol_config, default_port, proto_label) = match gfx.graphics_type.as_str() {
            "vnc" => (
                ProtocolConfig::Vnc(VncConfig::default()),
                DEFAULT_VNC_PORT,
                "VNC",
            ),
            "spice" => {
                let tls_port = gfx
                    .tls_port
                    .and_then(|p| if p > 0 { Some(p as u16) } else { None });
                let has_tls = tls_port.is_some();

                (
                    ProtocolConfig::Spice(SpiceConfig {
                        tls_enabled: has_tls,
                        ..SpiceConfig::default()
                    }),
                    DEFAULT_SPICE_PORT,
                    "SPICE",
                )
            }
            "rdp" => (
                ProtocolConfig::Rdp(crate::models::RdpConfig::default()),
                DEFAULT_RDP_PORT,
                "RDP",
            ),
            other => {
                result.add_skipped(SkippedEntry::with_location(
                    &domain.name,
                    format!("Unsupported graphics type: {other}"),
                    source_path,
                ));
                return None;
            }
        };

        // Resolve listen address — default to localhost for VMs
        let host = gfx
            .listen_address
            .as_deref()
            .filter(|a| !a.is_empty() && *a != "0.0.0.0")
            .unwrap_or("127.0.0.1")
            .to_string();

        // Resolve port: prefer tls_port for SPICE, then port, then default
        let port = if gfx.graphics_type == "spice" {
            gfx.tls_port
                .and_then(|p| if p > 0 { Some(p as u16) } else { None })
                .or(if gfx.port > 0 {
                    Some(gfx.port as u16)
                } else {
                    None
                })
                .unwrap_or(default_port)
        } else if gfx.port > 0 {
            gfx.port as u16
        } else {
            default_port
        };

        // Warn about autoport with unresolved port
        if gfx.autoport && gfx.port <= 0 {
            result.add_skipped(SkippedEntry::with_location(
                &domain.name,
                format!(
                    "{proto_label} uses autoport — actual port is assigned at VM startup. \
                     Using default port {default_port}. Edit after starting the VM."
                ),
                source_path,
            ));
        }

        // Build connection name
        let name = if multi_graphics {
            format!("{} ({proto_label})", domain.name)
        } else {
            domain.name.clone()
        };

        let mut conn = Connection::new(name, host, port, protocol_config);

        // Store VM UUID as tag for cross-reference
        if let Some(ref uuid) = domain.uuid {
            conn.tags.push(format!("libvirt-uuid:{uuid}"));
        }

        // Store description
        if let Some(ref desc) = domain.description {
            conn.description = Some(desc.clone());
        }

        // Handle password
        if let Some(ref pw) = gfx.password {
            conn.password_source = PasswordSource::Vault;
            let creds = Credentials {
                username: None,
                password: Some(SecretString::from(pw.clone())),
                key_passphrase: None,
                domain: None,
            };
            result.credentials.insert(conn.id, creds);
        }

        conn.tags.push("imported:libvirt".to_string());

        Some(conn)
    }

    /// Imports all domain XMLs from a directory.
    fn import_from_directory(dir: &Path, result: &mut ImportResult) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return, // Directory not readable — skip silently
        };

        for entry in entries.flatten() {
            let path = entry.path();

            // Only process .xml files (libvirt stores domains as <name>.xml)
            if path.extension().and_then(|e| e.to_str()) != Some("xml") {
                continue;
            }

            // Skip autostart symlinks directory and network XMLs
            if path.is_symlink() || path.is_dir() {
                continue;
            }

            // Check file size
            if let Ok(meta) = std::fs::metadata(&path)
                && meta.len() > MAX_XML_SIZE
            {
                result.add_skipped(SkippedEntry::with_location(
                    &path.display().to_string(),
                    "File too large (>10 MB)",
                    &dir.display().to_string(),
                ));
                continue;
            }

            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let source_path = path.display().to_string();

            let Some(domain) = Self::parse_domain_xml(&content) else {
                continue;
            };

            let conns = Self::domain_to_connections(&domain, &source_path, result);
            for conn in conns {
                result.add_connection(conn);
            }
        }
    }
}

impl ImportSource for LibvirtXmlImporter {
    fn source_id(&self) -> &'static str {
        "libvirt"
    }

    fn display_name(&self) -> &'static str {
        "Libvirt / GNOME Boxes"
    }

    fn is_available(&self) -> bool {
        Self::libvirt_xml_dirs().iter().any(|dir| dir.is_dir())
    }

    fn default_paths(&self) -> Vec<PathBuf> {
        Self::libvirt_xml_dirs()
            .into_iter()
            .filter(|dir| dir.is_dir())
            .collect()
    }

    fn import(&self) -> Result<ImportResult, ImportError> {
        let dirs = self.default_paths();
        if dirs.is_empty() {
            return Err(ImportError::FileNotFound(PathBuf::from(
                "No libvirt configuration directories found",
            )));
        }

        let mut result = ImportResult::new();

        // Create a group for imported VMs
        let group = ConnectionGroup::new("Libvirt VMs".to_string());
        let group_id = group.id;
        result.add_group(group);

        for dir in &dirs {
            Self::import_from_directory(dir, &mut result);
        }

        // Assign all connections to the group
        for conn in &mut result.connections {
            conn.group_id = Some(group_id);
        }

        Ok(result)
    }

    fn import_from_path(&self, path: &Path) -> Result<ImportResult, ImportError> {
        let mut result = ImportResult::new();

        if path.is_dir() {
            // Scan directory for XML files
            let group = ConnectionGroup::new("Libvirt VMs".to_string());
            let group_id = group.id;
            result.add_group(group);

            Self::import_from_directory(path, &mut result);

            for conn in &mut result.connections {
                conn.group_id = Some(group_id);
            }
        } else {
            // Single XML file (e.g. virsh dumpxml output)
            let content = read_import_file(path, "libvirt")?;
            let source_path = path.display().to_string();

            let Some(domain) = Self::parse_domain_xml(&content) else {
                return Err(ImportError::ParseError {
                    source_name: "libvirt".to_string(),
                    reason: "No valid <domain> element found in XML".to_string(),
                });
            };

            let conns = Self::domain_to_connections(&domain, &source_path, &mut result);
            for conn in conns {
                result.add_connection(conn);
            }
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_xml_file(content: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().expect("tempfile");
        f.write_all(content.as_bytes()).expect("write");
        f.flush().expect("flush");
        f
    }

    #[test]
    fn test_parse_vnc_domain() {
        let xml = r"
<domain type='kvm'>
  <name>ubuntu-dev</name>
  <uuid>ab953e2f-9d16-4955-bb43-1178230ee625</uuid>
  <description>Ubuntu development VM</description>
  <devices>
    <graphics type='vnc' port='5901' autoport='no' listen='127.0.0.1'>
      <listen type='address' address='127.0.0.1'/>
    </graphics>
  </devices>
</domain>";

        let f = write_xml_file(xml);
        let importer = LibvirtXmlImporter::new();
        let result = importer.import_from_path(f.path()).expect("import");

        assert_eq!(result.connections.len(), 1);
        let conn = &result.connections[0];
        assert_eq!(conn.name, "ubuntu-dev");
        assert_eq!(conn.host, "127.0.0.1");
        assert_eq!(conn.port, 5901);
        assert!(matches!(conn.protocol_config, ProtocolConfig::Vnc(_)));
        assert_eq!(conn.description.as_deref(), Some("Ubuntu development VM"));
        assert!(conn.tags.iter().any(|t| t.starts_with("libvirt-uuid:")));
        assert!(conn.tags.iter().any(|t| t == "imported:libvirt"));
    }

    #[test]
    fn test_parse_spice_with_tls() {
        let xml = r"
<domain type='kvm'>
  <name>win10</name>
  <devices>
    <graphics type='spice' port='5900' tlsPort='5901' autoport='no'>
      <listen type='address' address='192.168.1.100'/>
    </graphics>
  </devices>
</domain>";

        let f = write_xml_file(xml);
        let importer = LibvirtXmlImporter::new();
        let result = importer.import_from_path(f.path()).expect("import");

        assert_eq!(result.connections.len(), 1);
        let conn = &result.connections[0];
        assert_eq!(conn.name, "win10");
        assert_eq!(conn.host, "192.168.1.100");
        // Should prefer tls_port
        assert_eq!(conn.port, 5901);
        assert!(matches!(
            conn.protocol_config,
            ProtocolConfig::Spice(ref s) if s.tls_enabled
        ));
    }

    #[test]
    fn test_parse_multiple_graphics() {
        let xml = r"
<domain type='kvm'>
  <name>multivm</name>
  <devices>
    <graphics type='vnc' port='5902' autoport='no'/>
    <graphics type='spice' port='5903' autoport='no'/>
  </devices>
</domain>";

        let f = write_xml_file(xml);
        let importer = LibvirtXmlImporter::new();
        let result = importer.import_from_path(f.path()).expect("import");

        assert_eq!(result.connections.len(), 2);
        assert_eq!(result.connections[0].name, "multivm (VNC)");
        assert_eq!(result.connections[0].port, 5902);
        assert_eq!(result.connections[1].name, "multivm (SPICE)");
        assert_eq!(result.connections[1].port, 5903);
    }

    #[test]
    fn test_autoport_warning() {
        let xml = r"
<domain type='kvm'>
  <name>autovm</name>
  <devices>
    <graphics type='vnc' port='-1' autoport='yes'/>
  </devices>
</domain>";

        let f = write_xml_file(xml);
        let importer = LibvirtXmlImporter::new();
        let result = importer.import_from_path(f.path()).expect("import");

        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.connections[0].port, DEFAULT_VNC_PORT);
        // Should have a warning about autoport
        assert!(
            result.skipped.iter().any(|s| s.reason.contains("autoport")),
            "Expected autoport warning in skipped entries"
        );
    }

    #[test]
    fn test_no_graphics_skipped() {
        let xml = r"
<domain type='kvm'>
  <name>headless-server</name>
  <devices>
    <disk type='file' device='disk'/>
  </devices>
</domain>";

        let f = write_xml_file(xml);
        let importer = LibvirtXmlImporter::new();
        let result = importer.import_from_path(f.path()).expect("import");

        assert!(result.connections.is_empty());
        assert!(
            result
                .skipped
                .iter()
                .any(|s| s.reason.contains("No <graphics>")),
            "Expected skipped entry for headless VM"
        );
    }

    #[test]
    fn test_password_stored_as_credential() {
        let xml = r"
<domain type='kvm'>
  <name>secured-vm</name>
  <devices>
    <graphics type='vnc' port='5900' passwd='s3cret'/>
  </devices>
</domain>";

        let f = write_xml_file(xml);
        let importer = LibvirtXmlImporter::new();
        let result = importer.import_from_path(f.path()).expect("import");

        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.connections[0].password_source, PasswordSource::Vault);
        assert!(result.credentials.contains_key(&result.connections[0].id));
    }

    #[test]
    fn test_self_closing_graphics() {
        let xml = r"
<domain type='kvm'>
  <name>compact-vm</name>
  <devices>
    <graphics type='spice' port='5910' autoport='no'/>
  </devices>
</domain>";

        let f = write_xml_file(xml);
        let importer = LibvirtXmlImporter::new();
        let result = importer.import_from_path(f.path()).expect("import");

        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.connections[0].port, 5910);
        assert!(matches!(
            result.connections[0].protocol_config,
            ProtocolConfig::Spice(_)
        ));
    }

    #[test]
    fn test_listen_address_0000_defaults_to_localhost() {
        let xml = r"
<domain type='kvm'>
  <name>anyhost-vm</name>
  <devices>
    <graphics type='vnc' port='5900' listen='0.0.0.0'/>
  </devices>
</domain>";

        let f = write_xml_file(xml);
        let importer = LibvirtXmlImporter::new();
        let result = importer.import_from_path(f.path()).expect("import");

        assert_eq!(result.connections[0].host, "127.0.0.1");
    }

    #[test]
    fn test_invalid_xml_returns_error() {
        let xml = "this is not xml at all";
        let f = write_xml_file(xml);
        let importer = LibvirtXmlImporter::new();
        let err = importer.import_from_path(f.path()).unwrap_err();
        assert!(matches!(err, ImportError::ParseError { .. }));
    }

    #[test]
    fn test_directory_import() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Write two VM XMLs
        let vm1 = r"
<domain type='kvm'>
  <name>vm1</name>
  <devices>
    <graphics type='vnc' port='5901' autoport='no'/>
  </devices>
</domain>";

        let vm2 = r"
<domain type='kvm'>
  <name>vm2</name>
  <devices>
    <graphics type='spice' port='5902' autoport='no'/>
  </devices>
</domain>";

        std::fs::write(dir.path().join("vm1.xml"), vm1).expect("write vm1");
        std::fs::write(dir.path().join("vm2.xml"), vm2).expect("write vm2");
        // Non-XML file should be ignored
        std::fs::write(dir.path().join("networks"), "not xml").expect("write junk");

        let importer = LibvirtXmlImporter::new();
        let result = importer.import_from_path(dir.path()).expect("import");

        assert_eq!(result.connections.len(), 2);
        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.groups[0].name, "Libvirt VMs");

        // All connections should be in the group
        let group_id = result.groups[0].id;
        assert!(
            result
                .connections
                .iter()
                .all(|c| c.group_id == Some(group_id))
        );
    }

    #[test]
    fn test_rdp_graphics() {
        let xml = r"
<domain type='kvm'>
  <name>rdp-vm</name>
  <devices>
    <graphics type='rdp' port='3389' autoport='no'/>
  </devices>
</domain>";

        let f = write_xml_file(xml);
        let importer = LibvirtXmlImporter::new();
        let result = importer.import_from_path(f.path()).expect("import");

        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.connections[0].port, 3389);
        assert!(matches!(
            result.connections[0].protocol_config,
            ProtocolConfig::Rdp(_)
        ));
    }
}
