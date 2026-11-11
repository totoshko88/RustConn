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
fn show_prints_the_winrm_jump_host_and_options() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path();

    let bastion = run_cli(
        &[
            "add",
            "--name",
            "bastion",
            "--protocol",
            "ssh",
            "--host",
            "bastion.example.com",
        ],
        Some(config_dir),
    );
    assert!(bastion.status.success(), "{}", stderr_str(&bastion));
    let win = run_cli(
        &[
            "add",
            "--name",
            "win1",
            "--protocol",
            "winrm",
            "--host",
            "win1.example.com",
            "--jump-host",
            "bastion",
            "--winrm-accept-invalid-certs",
        ],
        Some(config_dir),
    );
    assert!(win.status.success(), "{}", stderr_str(&win));

    // A secret option can reach the stored arguments only by editing the
    // configuration by hand; the client never gets it, so `show` must not print it.
    {
        use rustconn_core::config::ConfigManager;
        use rustconn_core::models::ProtocolConfig;
        let manager = ConfigManager::with_config_dir(config_dir.to_path_buf());
        let mut connections = manager.load_connections().expect("load connections");
        let win1 = connections
            .iter_mut()
            .find(|c| c.name == "win1")
            .expect("win1 was added");
        let ProtocolConfig::WinRm(config) = &mut win1.protocol_config else {
            panic!("win1 is a WinRM connection");
        };
        config.custom_args = ["-n", "-p", "hunter2", "--spn", "HTTP"]
            .map(String::from)
            .to_vec();
        manager
            .save_connections(&connections)
            .expect("save connections");
    }

    let json = run_cli(&["show", "win1", "--format", "json"], Some(config_dir));
    assert!(json.status.success(), "{}", stderr_str(&json));
    let json_text = stdout_str(&json);
    let value: serde_json::Value = serde_json::from_str(&json_text).expect("show prints JSON");
    assert_eq!(value["jump_host"], "bastion");
    assert_eq!(value["accept_invalid_certs"], true);
    assert_eq!(
        value["custom_args"],
        serde_json::json!(["-n", "--spn", "HTTP"])
    );
    assert!(!json_text.contains("hunter2"), "{json_text}");

    let table = run_cli(&["show", "win1", "--format", "table"], Some(config_dir));
    let text = stdout_str(&table);
    assert!(text.contains("Jump Host: bastion"), "{text}");
    assert!(text.contains("Accept Invalid TLS: yes"), "{text}");
    assert!(text.contains("Custom Args: -n --spn HTTP"), "{text}");
    assert!(!text.contains("hunter2"), "{text}");
    assert!(!text.contains(" -p "), "{text}");
}

#[test]
fn show_prints_the_socks_login_of_a_web_connection() {
    use rustconn_core::config::ConfigManager;
    use rustconn_core::variables::Variable;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path();
    ConfigManager::with_config_dir(config_dir.to_path_buf())
        .save_variables(&[Variable::new_secret("proxy_pw", "hunter2")])
        .expect("save variables");

    let web = run_cli(
        &[
            "add",
            "--name",
            "intranet",
            "--protocol",
            "web",
            "--host",
            "https://example.com",
            "--socks-proxy",
            "127.0.0.1:1080",
            "--socks-proxy-user",
            "alice",
            "--socks-proxy-password-var",
            "proxy_pw",
        ],
        Some(config_dir),
    );
    assert!(web.status.success(), "{}", stderr_str(&web));

    let json = run_cli(&["show", "intranet", "--format", "json"], Some(config_dir));
    assert!(json.status.success(), "{}", stderr_str(&json));
    let json_text = stdout_str(&json);
    let value: serde_json::Value = serde_json::from_str(&json_text).expect("show prints JSON");
    assert_eq!(value["socks_proxy_username"], "alice");
    assert_eq!(value["socks_proxy_password_variable"], "proxy_pw");
    assert!(!json_text.contains("hunter2"), "{json_text}");

    let table = run_cli(&["show", "intranet", "--format", "table"], Some(config_dir));
    let text = stdout_str(&table);
    assert!(text.contains("SOCKS User:   alice"), "{text}");
    assert!(text.contains("SOCKS Password Var: proxy_pw"), "{text}");
    assert!(!text.contains("hunter2"), "{text}");
}

/// Saves `variables` as the global variables of the config dir.
fn save_variables(config_dir: &std::path::Path, variables: &[rustconn_core::variables::Variable]) {
    rustconn_core::config::ConfigManager::with_config_dir(config_dir.to_path_buf())
        .save_variables(variables)
        .expect("save variables");
}

/// The names of the saved connections.
fn saved_connection_names(config_dir: &std::path::Path) -> Vec<String> {
    rustconn_core::config::ConfigManager::with_config_dir(config_dir.to_path_buf())
        .load_connections()
        .expect("load connections")
        .into_iter()
        .map(|c| c.name)
        .collect()
}

/// `add` of a Web connection through a SOCKS proxy, with `extra` arguments.
fn add_socks_web(config_dir: &std::path::Path, extra: &[&str]) -> Output {
    let mut args = vec![
        "add",
        "--name",
        "intranet",
        "--protocol",
        "web",
        "--host",
        "https://example.com",
        "--socks-proxy",
        "127.0.0.1:1080",
    ];
    args.extend_from_slice(extra);
    run_cli(&args, Some(config_dir))
}

/// Asserts that `out` failed with `expected` on stderr and saved no `intranet`.
fn assert_socks_refused(config_dir: &std::path::Path, out: &Output, expected: &str) {
    assert!(!out.status.success(), "must be refused");
    let stderr = stderr_str(out);
    assert!(stderr.contains(expected), "{stderr}");
    assert!(!stderr.contains("hunter2"), "{stderr}");
    assert!(
        !saved_connection_names(config_dir).contains(&"intranet".to_string()),
        "a refused connection must not be saved"
    );
}

/// Adds an SSH connection named `bastion` for `--tunnel-via`.
fn add_tunnel_bastion(config_dir: &std::path::Path) {
    let bastion = run_cli(
        &[
            "add",
            "--name",
            "bastion",
            "--protocol",
            "ssh",
            "--host",
            "bastion.lan",
        ],
        Some(config_dir),
    );
    assert!(bastion.status.success(), "{}", stderr_str(&bastion));
}

/// On `add` the argument parser refuses the pair before `check_web_socks_settings`.
#[test]
fn add_refuses_a_socks_proxy_with_a_tunnel() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path();
    add_tunnel_bastion(config_dir);

    let out = add_socks_web(config_dir, &["--tunnel-via", "bastion"]);
    assert_socks_refused(
        config_dir,
        &out,
        "'--socks-proxy <HOST:PORT>' cannot be used with '--tunnel-via <NAME|UUID>'",
    );
}

/// A proxy added by `update` to a connection that already tunnels is refused by
/// `check_web_socks_settings`.
#[test]
fn update_refuses_a_socks_proxy_beside_a_saved_tunnel() {
    use rustconn_core::models::ProtocolConfig;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path();
    add_tunnel_bastion(config_dir);
    let add = run_cli(
        &[
            "add",
            "--name",
            "intranet",
            "--protocol",
            "web",
            "--host",
            "https://example.com",
            "--tunnel-via",
            "bastion",
        ],
        Some(config_dir),
    );
    assert!(add.status.success(), "{}", stderr_str(&add));

    let out = run_cli(
        &["update", "intranet", "--socks-proxy", "127.0.0.1:1080"],
        Some(config_dir),
    );
    assert!(
        !out.status.success(),
        "a proxy beside a tunnel must be refused"
    );
    let stderr = stderr_str(&out);
    assert!(
        stderr.contains("Choose either an SSH tunnel or a SOCKS proxy, not both"),
        "{stderr}"
    );
    let connections =
        rustconn_core::config::ConfigManager::with_config_dir(config_dir.to_path_buf())
            .load_connections()
            .expect("load connections");
    let web = connections
        .iter()
        .find(|c| c.name == "intranet")
        .expect("the Web connection is still saved");
    let ProtocolConfig::Web(cfg) = &web.protocol_config else {
        panic!("the saved connection is a Web connection");
    };
    assert_eq!(cfg.socks_proxy, None, "a refused update must not be saved");
}

#[test]
fn add_refuses_a_socks_login_without_a_password_variable() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path();

    let out = add_socks_web(config_dir, &["--socks-proxy-user", "alice"]);
    assert_socks_refused(
        config_dir,
        &out,
        "A SOCKS proxy login needs a proxy, a username and a password variable",
    );
}

#[test]
fn add_refuses_a_missing_socks_password_variable() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path();

    let out = add_socks_web(
        config_dir,
        &[
            "--socks-proxy-user",
            "alice",
            "--socks-proxy-password-var",
            "absent_pw",
        ],
    );
    assert_socks_refused(config_dir, &out, "no global variable is named 'absent_pw'");
}

#[test]
fn add_refuses_a_plain_socks_password_variable() {
    use rustconn_core::variables::Variable;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path();
    save_variables(config_dir, &[Variable::new("plain_pw", "hunter2")]);

    let out = add_socks_web(
        config_dir,
        &[
            "--socks-proxy-user",
            "alice",
            "--socks-proxy-password-var",
            "plain_pw",
        ],
    );
    assert_socks_refused(config_dir, &out, "the variable 'plain_pw' is not secret");
}

#[test]
fn add_accepts_a_secret_socks_password_variable() {
    use rustconn_core::variables::Variable;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path();
    save_variables(config_dir, &[Variable::new_secret("proxy_pw", "hunter2")]);

    let out = add_socks_web(
        config_dir,
        &[
            "--socks-proxy-user",
            "alice",
            "--socks-proxy-password-var",
            "proxy_pw",
        ],
    );
    assert!(out.status.success(), "{}", stderr_str(&out));
    assert!(!stdout_str(&out).contains("hunter2"));
    assert_eq!(saved_connection_names(config_dir), ["intranet"]);
}

#[test]
fn update_refuses_a_plain_socks_password_variable() {
    use rustconn_core::models::ProtocolConfig;
    use rustconn_core::variables::Variable;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path();
    save_variables(
        config_dir,
        &[
            Variable::new_secret("proxy_pw", "hunter2"),
            Variable::new("plain_pw", "hunter2"),
        ],
    );
    let add = add_socks_web(
        config_dir,
        &[
            "--socks-proxy-user",
            "alice",
            "--socks-proxy-password-var",
            "proxy_pw",
        ],
    );
    assert!(add.status.success(), "{}", stderr_str(&add));

    let out = run_cli(
        &[
            "update",
            "intranet",
            "--socks-proxy-password-var",
            "plain_pw",
        ],
        Some(config_dir),
    );
    assert!(!out.status.success(), "a plain variable must be refused");
    let stderr = stderr_str(&out);
    assert!(
        stderr.contains("the variable 'plain_pw' is not secret"),
        "{stderr}"
    );
    assert!(!stderr.contains("hunter2"), "{stderr}");

    let connections =
        rustconn_core::config::ConfigManager::with_config_dir(config_dir.to_path_buf())
            .load_connections()
            .expect("load connections");
    let ProtocolConfig::Web(cfg) = &connections[0].protocol_config else {
        panic!("the saved connection is a Web connection");
    };
    assert_eq!(
        cfg.socks_proxy_password_variable.as_deref(),
        Some("proxy_pw"),
        "a refused update must leave the saved variable alone"
    );
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

/// `connect` does not fail over, so a deleted primary jump host is reported
/// even when a fallback jump host still exists.
#[cfg(feature = "client-launch")]
#[test]
fn connect_warns_about_a_deleted_jump_host_even_with_a_fallback() {
    use rustconn_core::config::ConfigManager;
    use rustconn_core::models::Connection;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path();

    let bastion = Connection::new_ssh("bastion".to_string(), "bastion.lan".to_string(), 22);
    let mut target = Connection::new_ssh("web".to_string(), "web.lan".to_string(), 22);
    let deleted = uuid::Uuid::new_v4();
    target.set_jump_host_id(deleted);
    target.fallback_jump_host_ids = vec![bastion.id];
    ConfigManager::with_config_dir(config_dir.to_path_buf())
        .save_connections(&[bastion, target])
        .expect("save connections");

    let out = run_cli(&["connect", "web", "--dry-run"], Some(config_dir));
    assert!(
        out.status.success(),
        "dry-run connect should succeed. stderr: {}",
        stderr_str(&out)
    );
    let stderr = stderr_str(&out);
    assert!(
        stderr.contains(&format!(
            "jump host {deleted} (referenced by its own Jump Host setting) no longer exists; \
             this hop is skipped"
        )),
        "{stderr}"
    );
}

// ============================================================================
// CLI usability (0.24.3): explicit --format, group feedback, aws_ssm validation,
// bulk --from-file, --password-stdin
// ============================================================================

/// #2 — an explicit `--format table` is honoured even when stdout is piped.
/// The test harness runs the CLI with a non-terminal stdout, so the bare
/// default downgrades to JSON while an explicit `--format table` must still
/// print a table.
#[test]
fn explicit_format_table_is_honored_when_piped() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path();

    let add = run_cli(
        &[
            "add",
            "--name",
            "web1",
            "--host",
            "web1.example.com",
            "--protocol",
            "ssh",
        ],
        Some(config_dir),
    );
    assert!(add.status.success(), "seed add should succeed");

    // No --format: piped stdout downgrades to JSON (clig.dev default).
    let defaulted = run_cli(&["list"], Some(config_dir));
    assert!(defaulted.status.success());
    let defaulted_out = stdout_str(&defaulted);
    assert!(
        defaulted_out.contains('[') || defaulted_out.contains('{'),
        "a bare piped `list` must default to JSON. Got: {defaulted_out}"
    );

    // Explicit --format table: honoured verbatim even though stdout is piped.
    let explicit = run_cli(&["list", "--format", "table"], Some(config_dir));
    assert!(explicit.status.success());
    let explicit_out = stdout_str(&explicit);
    assert!(
        explicit_out.contains("NAME") && !explicit_out.trim_start().starts_with('['),
        "an explicit --format table must stay a table when piped. Got: {explicit_out}"
    );
}

/// #1/#6 — `add --group` reports whether the group was created or already
/// existed, and prints the group's ID.
#[test]
fn add_group_reports_created_then_existing_with_id() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path();

    let first = run_cli(
        &[
            "add",
            "--name",
            "a1",
            "--host",
            "a1.example.com",
            "--group",
            "Ops",
        ],
        Some(config_dir),
    );
    assert!(first.status.success(), "first add should succeed");
    let first_out = stdout_str(&first);
    assert!(
        first_out.contains("(created, ID "),
        "the first --group use must report the group as created with its ID. Got: {first_out}"
    );

    let second = run_cli(
        &[
            "add",
            "--name",
            "a2",
            "--host",
            "a2.example.com",
            "--group",
            "Ops",
        ],
        Some(config_dir),
    );
    assert!(second.status.success(), "second add should succeed");
    let second_out = stdout_str(&second);
    assert!(
        second_out.contains("(existing group, ID "),
        "reusing the group must report it as existing with its ID. Got: {second_out}"
    );
}

/// #5 — `add --provider aws_ssm` accepts an EC2 instance ID in --host and
/// rejects a plain hostname before the connection is written.
#[test]
fn add_aws_ssm_requires_an_instance_id_in_host() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path();

    let ok = run_cli(
        &[
            "add",
            "--name",
            "ec2a",
            "--protocol",
            "zt",
            "--provider",
            "aws_ssm",
            "--host",
            "i-0123456789abcdef0",
        ],
        Some(config_dir),
    );
    assert!(
        ok.status.success(),
        "a valid i-… instance ID must be accepted. stderr: {}",
        stderr_str(&ok)
    );

    let bad = run_cli(
        &[
            "add",
            "--name",
            "ec2b",
            "--protocol",
            "zt",
            "--provider",
            "aws_ssm",
            "--host",
            "ec2.example.com",
        ],
        Some(config_dir),
    );
    assert!(
        !bad.status.success(),
        "a plain hostname must be rejected for aws_ssm"
    );
    assert_eq!(
        bad.status.code().unwrap_or(-1),
        1,
        "aws_ssm validation failure uses the general error exit code"
    );
    assert!(
        stderr_str(&bad).contains("EC2 instance ID"),
        "the error should name the expected form. stderr: {}",
        stderr_str(&bad)
    );
}

/// #4 — `add --from-file` bulk-adds every connection in a JSON document and
/// reports the count. Uses the serde shape of `Connection` the CLI already
/// reads, built through the public API so the fixture stays valid as the model
/// evolves.
#[test]
fn add_from_file_adds_every_connection() {
    use rustconn_core::models::Connection;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path();

    let conns = vec![
        Connection::new_ssh("bulk1".to_string(), "h1.example.com".to_string(), 22),
        Connection::new_ssh("bulk2".to_string(), "h2.example.com".to_string(), 22),
        Connection::new_rdp("bulk3".to_string(), "h3.example.com".to_string(), 3389),
    ];
    let doc = serde_json::json!({ "connections": conns });
    let file = config_dir.join("bulk.json");
    std::fs::write(&file, serde_json::to_string_pretty(&doc).unwrap()).expect("write fixture");

    let out = run_cli(
        &["add", "--from-file", file.to_str().unwrap()],
        Some(config_dir),
    );
    assert!(
        out.status.success(),
        "bulk add should succeed. stderr: {}",
        stderr_str(&out)
    );
    assert!(
        stdout_str(&out).contains("Added 3 connection(s)"),
        "it should report three added. Got: {}",
        stdout_str(&out)
    );

    // All three are now listed.
    let listed = run_cli(&["list", "--format", "json"], Some(config_dir));
    let listed_out = stdout_str(&listed);
    for name in ["bulk1", "bulk2", "bulk3"] {
        assert!(
            listed_out.contains(name),
            "'{name}' should be in the list. Got: {listed_out}"
        );
    }
}

/// #4 — a duplicate name in the bulk file is skipped with a reason, the rest
/// still added.
#[test]
fn add_from_file_skips_duplicates_with_a_reason() {
    use rustconn_core::models::Connection;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path();

    // Seed an existing connection whose name collides with one in the file.
    let seed = run_cli(
        &["add", "--name", "dup", "--host", "existing.example.com"],
        Some(config_dir),
    );
    assert!(seed.status.success(), "seed add should succeed");

    let conns = vec![
        Connection::new_ssh("dup".to_string(), "h.example.com".to_string(), 22),
        Connection::new_ssh("fresh".to_string(), "h2.example.com".to_string(), 22),
    ];
    let doc = serde_json::json!({ "connections": conns });
    let file = config_dir.join("bulk.json");
    std::fs::write(&file, serde_json::to_string(&doc).unwrap()).expect("write fixture");

    let out = run_cli(
        &["add", "--from-file", file.to_str().unwrap()],
        Some(config_dir),
    );
    assert!(out.status.success(), "bulk add should succeed overall");
    let out_s = stdout_str(&out);
    assert!(
        out_s.contains("Added 1 connection(s)"),
        "only the fresh one is added. Got: {out_s}"
    );
    assert!(
        out_s.contains("Skipped") && out_s.contains("'dup'"),
        "the duplicate must be reported as skipped. Got: {out_s}"
    );
}

/// #3 — `--password-stdin` never echoes the password, regardless of build. On a
/// build without `secret-management` it fails before writing anything and names
/// the feature; on a build with it, the password is stored (keyring permitting)
/// and still never appears in the output.
#[test]
fn add_password_stdin_never_echoes_the_secret() {
    use std::io::Write;
    use std::process::Stdio;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path();

    let secret = "sup3r-s3cret-value";
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustconn-cli"))
        .env("RUSTCONN_CONFIG_DIR", config_dir)
        .args([
            "add",
            "--name",
            "pw1",
            "--host",
            "pw.example.com",
            "--password-stdin",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn CLI");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(format!("{secret}\n").as_bytes())
        .expect("write password");
    let output = child.wait_with_output().expect("wait CLI");

    let combined = format!("{}{}", stdout_str(&output), stderr_str(&output));
    assert!(
        !combined.contains(secret),
        "the password must never appear in stdout or stderr. Got: {combined}"
    );

    if cfg!(feature = "secret-management") {
        // With a backend the add succeeds (or fails only because the test
        // environment has no Secret Service); either way the secret stays out
        // of the output, which is asserted above.
    } else {
        assert!(
            !output.status.success(),
            "without secret-management, --password-stdin must fail rather than drop the password"
        );
        assert!(
            stderr_str(&output).contains("secret-management"),
            "the error should name the required feature. stderr: {}",
            stderr_str(&output)
        );
    }
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
