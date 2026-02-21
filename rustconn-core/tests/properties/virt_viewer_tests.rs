//! Property-based tests for VirtViewer (.vv) file importer
//!
//! Tests parsing correctness, fuzz resilience, and edge cases for
//! the virt-viewer INI-style file format used by libvirt/Proxmox VE.

use proptest::prelude::*;
use rustconn_core::import::{ImportSource, VirtViewerImporter};
use rustconn_core::models::{PasswordSource, ProtocolConfig};
use std::io::Write;
use tempfile::NamedTempFile;

// ============================================================================
// Helpers
// ============================================================================

/// Writes content to a temp file and returns the handle
fn write_vv_file(content: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().expect("tempfile");
    f.write_all(content.as_bytes()).expect("write");
    f.flush().expect("flush");
    f
}

// ============================================================================
// Strategies
// ============================================================================

fn arb_hostname() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-z][a-z0-9-]{0,15}(\\.[a-z][a-z0-9-]{0,8})*",
        "([0-9]{1,3}\\.){3}[0-9]{1,3}",
    ]
    .prop_filter("hostname must not be empty", |s| !s.is_empty())
}

fn arb_port() -> impl Strategy<Value = u16> {
    1u16..65535
}

fn arb_title() -> impl Strategy<Value = String> {
    "[A-Za-z0-9][A-Za-z0-9 _.-]{0,30}"
        .prop_map(|s| s.trim().to_string())
        .prop_filter("title must not be empty after trim", |s| !s.is_empty())
}

fn arb_password() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9!@#$%^&*]{4,20}"
}

fn arb_proxy() -> impl Strategy<Value = String> {
    arb_hostname().prop_map(|h| format!("http://{}:3128", h))
}

/// Strategy for a valid SPICE .vv file content
fn arb_spice_vv(
    host: String,
    port: u16,
    title: Option<String>,
    password: Option<String>,
    proxy: Option<String>,
    use_tls: bool,
    inline_pem: bool,
) -> String {
    let mut lines = vec!["[virt-viewer]".to_string(), "type=spice".to_string()];
    lines.push(format!("host={host}"));

    if use_tls {
        lines.push(format!("tls-port={port}"));
    } else {
        lines.push(format!("port={port}"));
    }

    if let Some(t) = title {
        lines.push(format!("title={t}"));
    }
    if let Some(pw) = password {
        lines.push(format!("password={pw}"));
    }
    if let Some(px) = proxy {
        lines.push(format!("proxy={px}"));
    }
    if inline_pem {
        lines.push("ca=-----BEGIN CERTIFICATE-----\\nMIIFake...".to_string());
    }

    lines.join("\n")
}

/// Strategy for a valid VNC .vv file content
fn arb_vnc_vv(host: String, port: u16, title: Option<String>) -> String {
    let mut lines = vec!["[virt-viewer]".to_string(), "type=vnc".to_string()];
    lines.push(format!("host={host}"));
    lines.push(format!("port={port}"));

    if let Some(t) = title {
        lines.push(format!("title={t}"));
    }

    lines.join("\n")
}

/// Strategy for arbitrary (potentially invalid) .vv-like content
fn arb_fuzz_content() -> impl Strategy<Value = String> {
    prop_oneof![
        // Completely random text
        "[\\x20-\\x7e]{0,200}",
        // Random INI-like content with wrong section
        Just("[wrong-section]\ntype=spice\nhost=test\n".to_string()),
        // Empty section
        Just("[virt-viewer]\n".to_string()),
        // Section with only comments
        Just("[virt-viewer]\n# comment\n; another comment\n".to_string()),
        // Multiple sections
        Just("[virt-viewer]\ntype=spice\nhost=a\n[other]\nfoo=bar\n".to_string()),
        // Very long values
        prop::string::string_regex("[a-z]{500}")
            .unwrap()
            .prop_map(|s| format!("[virt-viewer]\ntype=spice\nhost={s}\nport=5900\n")),
        // Keys without values
        Just("[virt-viewer]\ntype\nhost\nport\n".to_string()),
        // Empty values
        Just("[virt-viewer]\ntype=\nhost=\nport=\n".to_string()),
    ]
}

// ============================================================================
// Property Tests
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Parser never panics on arbitrary input
    #[test]
    fn prop_vv_parser_never_panics(content in arb_fuzz_content()) {
        let f = write_vv_file(&content);
        let importer = VirtViewerImporter::new();
        // Should not panic — either Ok or Err is fine
        let _ = importer.import_from_path(f.path());
    }

    /// Valid SPICE .vv files produce exactly one connection with correct host/port
    #[test]
    fn prop_spice_vv_imports_correctly(
        host in arb_hostname(),
        port in arb_port(),
        title in prop::option::of(arb_title()),
        password in prop::option::of(arb_password()),
    ) {
        let content = arb_spice_vv(
            host.clone(), port, title.clone(), password.clone(),
            None, true, false,
        );
        let f = write_vv_file(&content);
        let importer = VirtViewerImporter::new();
        let result = importer.import_from_path(f.path()).expect("import");

        prop_assert_eq!(result.connections.len(), 1);
        let conn = &result.connections[0];

        // Host matches
        prop_assert_eq!(&conn.host, &host);
        // Port matches (tls-port used)
        prop_assert_eq!(conn.port, port);
        // Protocol is SPICE with TLS enabled
        prop_assert!(matches!(
            conn.protocol_config,
            ProtocolConfig::Spice(ref s) if s.tls_enabled
        ));
        // Title used as name when present
        if let Some(ref t) = title {
            prop_assert_eq!(&conn.name, t);
        } else {
            prop_assert_eq!(&conn.name, &format!("{}:{}", host, port));
        }
        // Password stored in credentials
        if password.is_some() {
            prop_assert_eq!(&conn.password_source, &PasswordSource::Vault);
            prop_assert!(result.credentials.contains_key(&conn.id));
        }
        // Tagged as imported
        prop_assert!(conn.tags.iter().any(|t| t == "imported:virt-viewer"));
    }

    /// Valid VNC .vv files produce exactly one connection
    #[test]
    fn prop_vnc_vv_imports_correctly(
        host in arb_hostname(),
        port in arb_port(),
        title in prop::option::of(arb_title()),
    ) {
        let content = arb_vnc_vv(host.clone(), port, title.clone());
        let f = write_vv_file(&content);
        let importer = VirtViewerImporter::new();
        let result = importer.import_from_path(f.path()).expect("import");

        prop_assert_eq!(result.connections.len(), 1);
        let conn = &result.connections[0];

        prop_assert_eq!(&conn.host, &host);
        prop_assert_eq!(conn.port, port);
        prop_assert!(matches!(conn.protocol_config, ProtocolConfig::Vnc(_)));

        if let Some(ref t) = title {
            prop_assert_eq!(&conn.name, t);
        }
    }

    /// SPICE with proxy stores proxy in SpiceConfig
    #[test]
    fn prop_spice_proxy_stored_as_tag(
        host in arb_hostname(),
        port in arb_port(),
        proxy in arb_proxy(),
    ) {
        let content = arb_spice_vv(
            host, port, None, None, Some(proxy.clone()), false, false,
        );
        let f = write_vv_file(&content);
        let importer = VirtViewerImporter::new();
        let result = importer.import_from_path(f.path()).expect("import");

        prop_assert_eq!(result.connections.len(), 1);
        let conn = &result.connections[0];
        if let ProtocolConfig::Spice(ref s) = conn.protocol_config {
            prop_assert_eq!(
                s.proxy.as_deref(),
                Some(proxy.as_str()),
                "Expected proxy '{}' in SpiceConfig, got {:?}",
                proxy,
                s.proxy
            );
        } else {
            prop_assert!(false, "Expected SPICE protocol config");
        }
    }

    /// Inline PEM CA produces a skipped entry warning
    #[test]
    fn prop_inline_pem_produces_skipped_entry(
        host in arb_hostname(),
        port in arb_port(),
    ) {
        let content = arb_spice_vv(
            host, port, None, None, None, true, true,
        );
        let f = write_vv_file(&content);
        let importer = VirtViewerImporter::new();
        let result = importer.import_from_path(f.path()).expect("import");

        // Connection should still be imported
        prop_assert_eq!(result.connections.len(), 1);
        // But with a skipped entry for the PEM
        prop_assert!(
            result.skipped.iter().any(|s| s.reason.contains("Inline PEM")),
            "Expected skipped entry for inline PEM CA"
        );
        // And no ca_cert_path set
        if let ProtocolConfig::Spice(ref s) = result.connections[0].protocol_config {
            prop_assert!(s.ca_cert_path.is_none());
        }
    }

    /// Unsupported protocol types produce skipped entries, not errors
    #[test]
    fn prop_unsupported_type_skipped(
        host in arb_hostname(),
        proto in prop_oneof![
            Just("rdp"),
            Just("ssh"),
            Just("telnet"),
            Just("unknown"),
        ],
    ) {
        let content = format!(
            "[virt-viewer]\ntype={proto}\nhost={host}\nport=5900\n"
        );
        let f = write_vv_file(&content);
        let importer = VirtViewerImporter::new();
        let result = importer.import_from_path(f.path()).expect("import");

        prop_assert!(result.connections.is_empty());
        prop_assert!(!result.skipped.is_empty());
    }

    /// Missing host produces skipped entry, not panic
    #[test]
    fn prop_missing_host_skipped(port in arb_port()) {
        let content = format!(
            "[virt-viewer]\ntype=spice\ntls-port={port}\n"
        );
        let f = write_vv_file(&content);
        let importer = VirtViewerImporter::new();
        let result = importer.import_from_path(f.path()).expect("import");

        prop_assert!(result.connections.is_empty());
        prop_assert!(!result.skipped.is_empty());
    }

    /// Missing type field produces skipped entry
    #[test]
    fn prop_missing_type_skipped(host in arb_hostname()) {
        let content = format!(
            "[virt-viewer]\nhost={host}\nport=5900\n"
        );
        let f = write_vv_file(&content);
        let importer = VirtViewerImporter::new();
        let result = importer.import_from_path(f.path()).expect("import");

        prop_assert!(result.connections.is_empty());
        prop_assert!(!result.skipped.is_empty());
    }

    /// Non-TLS SPICE uses plain port and tls_enabled=false
    #[test]
    fn prop_spice_non_tls_port(
        host in arb_hostname(),
        port in arb_port(),
    ) {
        let content = arb_spice_vv(
            host.clone(), port, None, None, None, false, false,
        );
        let f = write_vv_file(&content);
        let importer = VirtViewerImporter::new();
        let result = importer.import_from_path(f.path()).expect("import");

        prop_assert_eq!(result.connections.len(), 1);
        let conn = &result.connections[0];
        prop_assert_eq!(conn.port, port);
        // No tls-port means tls_enabled should be false
        if let ProtocolConfig::Spice(ref s) = conn.protocol_config {
            prop_assert!(!s.tls_enabled);
        }
    }
}

// ============================================================================
// Non-proptest unit tests
// ============================================================================

#[test]
fn test_virt_viewer_importer_metadata() {
    let importer = VirtViewerImporter::new();
    assert_eq!(importer.source_id(), "virt_viewer");
    assert_eq!(importer.display_name(), "Virt-Viewer (.vv)");
    assert!(importer.is_available());
    assert!(importer.default_paths().is_empty());
}

#[test]
fn test_virt_viewer_import_without_path_returns_error() {
    let importer = VirtViewerImporter::new();
    assert!(importer.import().is_err());
}

#[test]
fn test_host_subject_stored_as_tag() {
    let content = "\
[virt-viewer]
type=spice
host=10.0.0.1
port=5900
host-subject=OU=PVE Cluster Node,O=Proxmox,CN=node1
";
    let f = write_vv_file(content);
    let importer = VirtViewerImporter::new();
    let result = importer.import_from_path(f.path()).expect("import");

    assert_eq!(result.connections.len(), 1);
    assert!(
        result.connections[0]
            .tags
            .iter()
            .any(|t| t.starts_with("host-subject:"))
    );
}
