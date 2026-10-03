//! Integration tests for rustconn-cli
//!
//! These tests verify the CLI commands work correctly end-to-end,
//! including list, add, export, import, and error handling.

#![allow(
    clippy::uninlined_format_args,
    reason = "module-wide override for legacy code; refactored case by case"
)]

use std::process::{Command, Output};

use tempfile::TempDir;

/// Helper to run the CLI with given arguments
fn run_cli(args: &[&str], config_dir: Option<&std::path::Path>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rustconn-cli"));

    if let Some(dir) = config_dir {
        cmd.env("RUSTCONN_CONFIG_DIR", dir);
    }

    cmd.args(args).output().expect("Failed to execute CLI")
}

/// Helper to get stdout as string
fn stdout_str(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// Helper to get stderr as string
fn stderr_str(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

// ============================================================================
// Help Command Tests
// ============================================================================

#[test]
fn test_help_command() {
    let output = run_cli(&["--help"], None);

    assert!(output.status.success(), "Help command should succeed");

    let stdout = stdout_str(&output);
    assert!(
        stdout.contains("rustconn-cli"),
        "Help should mention program name"
    );
    assert!(stdout.contains("list"), "Help should mention list command");
    assert!(stdout.contains("add"), "Help should mention add command");
    assert!(
        stdout.contains("export"),
        "Help should mention export command"
    );
    assert!(
        stdout.contains("import"),
        "Help should mention import command"
    );
    assert!(stdout.contains("test"), "Help should mention test command");
}

#[test]
fn test_list_help() {
    let output = run_cli(&["list", "--help"], None);

    assert!(output.status.success(), "List help should succeed");

    let stdout = stdout_str(&output);
    assert!(
        stdout.contains("format"),
        "List help should mention format option"
    );
    assert!(
        stdout.contains("protocol"),
        "List help should mention protocol filter"
    );
}

#[test]
fn test_add_help() {
    let output = run_cli(&["add", "--help"], None);

    assert!(output.status.success(), "Add help should succeed");

    let stdout = stdout_str(&output);
    assert!(
        stdout.contains("name"),
        "Add help should mention name option"
    );
    assert!(
        stdout.contains("host"),
        "Add help should mention host option"
    );
    assert!(
        stdout.contains("port"),
        "Add help should mention port option"
    );
    assert!(
        stdout.contains("protocol"),
        "Add help should mention protocol option"
    );
}

#[test]
fn test_export_help() {
    let output = run_cli(&["export", "--help"], None);

    assert!(output.status.success(), "Export help should succeed");

    let stdout = stdout_str(&output);
    assert!(
        stdout.contains("format"),
        "Export help should mention format option"
    );
    assert!(
        stdout.contains("output"),
        "Export help should mention output option"
    );
}

#[test]
fn test_import_help() {
    let output = run_cli(&["import", "--help"], None);

    assert!(output.status.success(), "Import help should succeed");

    let stdout = stdout_str(&output);
    assert!(
        stdout.contains("format"),
        "Import help should mention format option"
    );
}

// ============================================================================
// List Command Tests
// ============================================================================

#[test]
fn test_list_empty() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    let output = run_cli(&["list"], Some(temp_dir.path()));

    // Should succeed even with no connections
    assert!(
        output.status.success(),
        "List should succeed with empty config"
    );

    let stdout = stdout_str(&output);
    // When stdout is not a terminal, effective format falls back to JSON,
    // so an empty list may render as "[]".
    assert!(
        stdout.contains("No connections found")
            || stdout.is_empty()
            || stdout.contains("NAME")
            || stdout.trim() == "[]",
        "Should show empty message or header. Got: {stdout}"
    );
}

#[test]
fn test_list_json_format() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    let output = run_cli(&["list", "--format", "json"], Some(temp_dir.path()));

    assert!(output.status.success(), "List JSON should succeed");

    let stdout = stdout_str(&output);
    // Empty list should be valid JSON (empty array)
    assert!(
        stdout.trim().is_empty() || stdout.contains('['),
        "JSON output should be valid. Got: {stdout}"
    );
}

#[test]
fn test_list_csv_format() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    let output = run_cli(&["list", "--format", "csv"], Some(temp_dir.path()));

    assert!(output.status.success(), "List CSV should succeed");

    let stdout = stdout_str(&output);
    // CSV should have header even if empty
    if !stdout.is_empty() {
        assert!(
            stdout.contains("name,host,port,protocol") || stdout.contains("No connections"),
            "CSV should have header or empty message. Got: {stdout}"
        );
    }
}

// ============================================================================
// Feature-gated command tests
// ============================================================================

#[cfg(feature = "client-launch")]
#[test]
fn test_connect_present_in_help() {
    let output = run_cli(&["--help"], None);
    assert!(output.status.success());
    let stdout = stdout_str(&output);
    assert!(
        stdout.contains("connect"),
        "Help should mention connect command when client-launch is enabled"
    );
}

#[cfg(feature = "client-launch")]
#[test]
fn test_connect_nonexistent() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    let output = run_cli(&["connect", "nonexistent"], Some(temp_dir.path()));

    assert!(
        !output.status.success(),
        "Connect to nonexistent should fail"
    );

    let exit_code = output.status.code().unwrap_or(-1);
    assert!(
        exit_code == 1 || exit_code == 2,
        "Exit code should be 1 or 2 for connection error. Got: {exit_code}"
    );

    let stderr = stderr_str(&output);
    assert!(
        stderr.contains("not found")
            || stderr.contains("Error")
            || stderr.contains("No connections"),
        "Should show error message. Got: {stderr}"
    );
}

#[cfg(not(feature = "client-launch"))]
#[test]
fn test_connect_absent_from_help() {
    let output = run_cli(&["--help"], None);
    assert!(output.status.success());
    let stdout = stdout_str(&output);
    // Check that "connect" does not appear as a standalone subcommand name.
    // It may appear inside other descriptions (e.g. "Test connectivity").
    let has_connect_subcommand = stdout.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("connect ") || trimmed == "connect"
    });
    assert!(
        !has_connect_subcommand,
        "Help should NOT list connect as a subcommand when client-launch is disabled"
    );
}

#[cfg(any(feature = "secret-management", feature = "keepass-verify"))]
#[test]
fn test_secret_present_in_help() {
    let output = run_cli(&["--help"], None);
    assert!(output.status.success());
    let stdout = stdout_str(&output);
    assert!(
        stdout.contains("secret"),
        "Help should mention secret command when secret-management or keepass-verify is enabled"
    );
}

#[cfg(not(any(feature = "secret-management", feature = "keepass-verify")))]
#[test]
fn test_secret_absent_from_help() {
    let output = run_cli(&["--help"], None);
    assert!(output.status.success());
    let stdout = stdout_str(&output);
    assert!(
        !stdout.contains("secret"),
        "Help should NOT mention secret command when neither secret-management nor keepass-verify is enabled. Got: {stdout}"
    );
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[test]
fn test_import_nonexistent_file() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    let output = run_cli(
        &["import", "--format", "ssh-config", "/nonexistent/file"],
        Some(temp_dir.path()),
    );

    // Should fail with exit code 1 (general error)
    assert!(
        !output.status.success(),
        "Import nonexistent file should fail"
    );

    let exit_code = output.status.code().unwrap_or(-1);
    assert_eq!(exit_code, 1, "Exit code should be 1 for import error");

    let stderr = stderr_str(&output);
    assert!(
        stderr.contains("not found") || stderr.contains("Error") || stderr.contains("No such file"),
        "Should show file not found error. Got: {stderr}"
    );
}

#[test]
fn test_export_invalid_format() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let output_path = temp_dir.path().join("output.txt");

    let output = run_cli(
        &[
            "export",
            "--format",
            "invalid",
            "--output",
            output_path.to_str().unwrap(),
        ],
        Some(temp_dir.path()),
    );

    // Should fail due to invalid format
    assert!(
        !output.status.success(),
        "Export with invalid format should fail"
    );

    let stderr = stderr_str(&output);
    assert!(
        stderr.contains("invalid") || stderr.contains("error") || stderr.contains("Invalid"),
        "Should show invalid format error. Got: {}",
        stderr
    );
}

// ============================================================================
// Native export data-preservation (templates / clusters / variables / snippets)
// ============================================================================

/// Regression guard: `export --format native` used to pass empty vecs for
/// templates, clusters, variables and snippets, so a CLI native export silently
/// dropped all four even though the format (and the GUI export) preserve them.
/// Seed a config with one of each, export through the real binary, read the
/// `.rcn` back, and assert every collection survived.
#[test]
fn native_export_preserves_templates_clusters_variables_snippets() {
    use rustconn_core::cluster::Cluster;
    use rustconn_core::config::ConfigManager;
    use rustconn_core::export::NativeExport;
    use rustconn_core::models::{ConnectionTemplate, Snippet};
    use rustconn_core::variables::Variable;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path();

    // Seed the config dir the CLI will read via RUSTCONN_CONFIG_DIR.
    let manager = ConfigManager::with_config_dir(config_dir.to_path_buf());
    manager
        .save_templates(&[ConnectionTemplate::new_ssh("Edge Router".to_string())])
        .expect("save templates");
    manager
        .save_clusters(&[Cluster::new("DC Fleet".to_string())])
        .expect("save clusters");
    manager
        .save_variables(&[Variable::new("region", "eu-central-1")])
        .expect("save variables");
    manager
        .save_snippets(&[Snippet::new(
            "Tail syslog".to_string(),
            "tail -f /var/log/syslog".to_string(),
        )])
        .expect("save snippets");

    let output_path = config_dir.join("export.rcn");
    let out = run_cli(
        &[
            "export",
            "--format",
            "native",
            "--output",
            output_path.to_str().unwrap(),
        ],
        Some(config_dir),
    );
    assert!(
        out.status.success(),
        "native export should succeed. stderr: {}",
        stderr_str(&out)
    );

    // Read the archive back and assert the four collections survived.
    let export = NativeExport::from_file(&output_path).expect("parse exported .rcn");
    assert_eq!(
        export.templates.len(),
        1,
        "templates must survive native CLI export"
    );
    assert_eq!(export.templates[0].name, "Edge Router");
    assert_eq!(
        export.clusters.len(),
        1,
        "clusters must survive native CLI export"
    );
    assert_eq!(export.clusters[0].name, "DC Fleet");
    assert_eq!(
        export.variables.len(),
        1,
        "variables must survive native CLI export"
    );
    assert_eq!(export.variables[0].name, "region");
    assert_eq!(
        export.snippets.len(),
        1,
        "snippets must survive native CLI export"
    );
    assert_eq!(export.snippets[0].name, "Tail syslog");
}

// ============================================================================
// Add Command Tests
// ============================================================================
#[test]
fn test_add_missing_required_args() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    // Missing --host
    let output = run_cli(&["add", "--name", "test"], Some(temp_dir.path()));

    assert!(!output.status.success(), "Add without host should fail");

    let stderr = stderr_str(&output);
    assert!(
        stderr.contains("host") || stderr.contains("required") || stderr.contains("error"),
        "Should mention missing host. Got: {}",
        stderr
    );
}

#[test]
fn test_add_invalid_protocol() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    let output = run_cli(
        &[
            "add",
            "--name",
            "test",
            "--host",
            "example.com",
            "--protocol",
            "invalid",
        ],
        Some(temp_dir.path()),
    );

    assert!(
        !output.status.success(),
        "Add with invalid protocol should fail"
    );

    let stderr = stderr_str(&output);
    assert!(
        stderr.contains("invalid") || stderr.contains("protocol") || stderr.contains("error"),
        "Should mention invalid protocol. Got: {}",
        stderr
    );
}

// ============================================================================
// Test Command Tests
// ============================================================================

#[test]
fn test_test_nonexistent_connection() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    let output = run_cli(&["test", "nonexistent"], Some(temp_dir.path()));

    // Should fail with exit code 2 (connection failure)
    assert!(!output.status.success(), "Test nonexistent should fail");

    let exit_code = output.status.code().unwrap_or(-1);
    assert!(
        exit_code == 1 || exit_code == 2,
        "Exit code should be 1 or 2 for test error. Got: {}",
        exit_code
    );
}

// ============================================================================
// Version Test
// ============================================================================

#[test]
fn test_version() {
    let output = run_cli(&["--version"], None);

    assert!(output.status.success(), "Version command should succeed");

    let stdout = stdout_str(&output);
    assert!(
        stdout.contains("rustconn-cli") || stdout.contains(env!("CARGO_PKG_VERSION")),
        "Version output should contain program name or version. Got: {}",
        stdout
    );
}

// ============================================================================
// Kerberos (RDP NLA) flags — issue #351
// ============================================================================

/// Loads the single saved RDP connection's config from a CLI config dir.
#[cfg(test)]
fn load_only_rdp(config_dir: &std::path::Path) -> rustconn_core::models::RdpConfig {
    use rustconn_core::config::ConfigManager;
    use rustconn_core::models::ProtocolConfig;
    let manager = ConfigManager::with_config_dir(config_dir.to_path_buf());
    let connections = manager.load_connections().expect("load connections");
    let conn = connections.first().expect("one connection was added");
    match &conn.protocol_config {
        ProtocolConfig::Rdp(cfg) => cfg.clone(),
        other => panic!("expected an RDP connection, got {other:?}"),
    }
}

#[test]
fn add_kerberos_stores_and_normalizes_the_kdc_address() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path();

    let out = run_cli(
        &[
            "add",
            "--name",
            "win-dc",
            "--protocol",
            "rdp",
            "--host",
            "rdp1.example.com",
            "--kerberos",
            "--kdc-address",
            "dc1.example.com",
        ],
        Some(config_dir),
    );
    assert!(
        out.status.success(),
        "add with --kerberos should succeed. stderr: {}",
        stderr_str(&out)
    );

    let cfg = load_only_rdp(config_dir);
    assert!(cfg.kerberos_enabled, "kerberos should be enabled");
    assert_eq!(
        cfg.kdc_proxy_url.as_deref(),
        Some("tcp://dc1.example.com:88"),
        "the KDC address must be stored in normalized form"
    );
}

#[test]
fn add_rejects_a_malformed_kdc_address() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path();

    let out = run_cli(
        &[
            "add",
            "--name",
            "win-dc",
            "--protocol",
            "rdp",
            "--host",
            "rdp1.example.com",
            "--kerberos",
            "--kdc-address",
            "ldap://dc1.example.com",
        ],
        Some(config_dir),
    );
    assert!(
        !out.status.success(),
        "a malformed KDC address must fail the add rather than store a dropped value"
    );
    assert!(
        stderr_str(&out).contains("invalid KDC address"),
        "the error should name the problem. stderr: {}",
        stderr_str(&out)
    );
}

#[test]
fn update_can_disable_kerberos_and_clear_the_kdc_address() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path();

    let add = run_cli(
        &[
            "add",
            "--name",
            "win-dc",
            "--protocol",
            "rdp",
            "--host",
            "rdp1.example.com",
            "--kerberos",
            "--kdc-address",
            "dc1.example.com",
        ],
        Some(config_dir),
    );
    assert!(add.status.success(), "seed add should succeed");

    // Turn Kerberos off and clear the KDC address in one update.
    let upd = run_cli(
        &[
            "update",
            "win-dc",
            "--kerberos",
            "false",
            "--kdc-address",
            "",
        ],
        Some(config_dir),
    );
    assert!(
        upd.status.success(),
        "update should succeed. stderr: {}",
        stderr_str(&upd)
    );

    let cfg = load_only_rdp(config_dir);
    assert!(!cfg.kerberos_enabled, "kerberos should be disabled");
    assert_eq!(
        cfg.kdc_proxy_url, None,
        "an empty --kdc-address must clear the stored value"
    );
}

#[cfg(feature = "client-launch")]
#[test]
fn connect_warns_about_kerberos_with_an_ip_host() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path();

    // Kerberos on, but the host is an IP address — the service principal is
    // TERMSRV/<dns-name>, which the domain does not know for a bare IP, so
    // sign-in would fail. The connect preflight must say so.
    let add = run_cli(
        &[
            "add",
            "--name",
            "win-dc",
            "--protocol",
            "rdp",
            "--host",
            "10.0.0.5",
            "--domain",
            "example.com",
            "--kerberos",
        ],
        Some(config_dir),
    );
    assert!(add.status.success(), "seed add should succeed");

    // --dry-run reaches the preflight (which runs before the dry-run short
    // circuit) without launching a real client.
    let out = run_cli(&["connect", "win-dc", "--dry-run"], Some(config_dir));
    assert!(
        out.status.success(),
        "dry-run connect should succeed. stderr: {}",
        stderr_str(&out)
    );
    assert!(
        stderr_str(&out).contains("Kerberos needs the server's DNS name"),
        "the preflight should warn about the IP host. stderr: {}",
        stderr_str(&out)
    );
}

#[cfg(feature = "client-launch")]
#[test]
fn connect_is_quiet_about_kerberos_when_settings_are_fine() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path();

    let add = run_cli(
        &[
            "add",
            "--name",
            "win-dc",
            "--protocol",
            "rdp",
            "--host",
            "rdp1.example.com",
            "--domain",
            "example.com",
            "--kerberos",
        ],
        Some(config_dir),
    );
    assert!(add.status.success(), "seed add should succeed");

    let out = run_cli(&["connect", "win-dc", "--dry-run"], Some(config_dir));
    assert!(out.status.success(), "dry-run connect should succeed");
    assert!(
        !stderr_str(&out).contains("Kerberos needs"),
        "a DNS host + DNS domain must produce no Kerberos warning. stderr: {}",
        stderr_str(&out)
    );
}
