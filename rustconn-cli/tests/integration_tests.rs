//! Integration tests for rustconn-cli
//!
//! These tests verify the CLI commands work correctly end-to-end,
//! including list, add, export, import, and error handling.

#![allow(clippy::uninlined_format_args)]

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
    assert!(
        stdout.contains("connect"),
        "Help should mention connect command"
    );
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
    assert!(
        stdout.contains("No connections found") || stdout.is_empty() || stdout.contains("NAME"),
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
// Error Handling Tests
// ============================================================================

#[test]
fn test_connect_nonexistent() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    let output = run_cli(&["connect", "nonexistent"], Some(temp_dir.path()));

    // Should fail with exit code 2 (connection not found)
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
