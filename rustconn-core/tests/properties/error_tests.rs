//! Property tests for error types

use rustconn_core::error::{
    ConfigError, ImportError, ProtocolError, RustConnError, SecretError, SessionError,
};
use std::path::PathBuf;

// ============================================================================
// ConfigError Tests
// ============================================================================

#[test]
fn config_error_parse_has_message() {
    let err = ConfigError::Parse("invalid syntax".to_string());
    let msg = err.to_string();
    assert!(msg.contains("parse"));
    assert!(msg.contains("invalid syntax"));
}

#[test]
fn config_error_validation_has_field_and_reason() {
    let err = ConfigError::Validation {
        field: "port".to_string(),
        reason: "must be positive".to_string(),
    };
    let msg = err.to_string();
    assert!(msg.contains("port"));
    assert!(msg.contains("must be positive"));
}

#[test]
fn config_error_not_found_has_path() {
    let err = ConfigError::NotFound(PathBuf::from("/etc/config.toml"));
    let msg = err.to_string();
    assert!(msg.contains("not found"));
    assert!(msg.contains("config.toml"));
}

#[test]
fn config_error_write_has_message() {
    let err = ConfigError::Write("permission denied".to_string());
    let msg = err.to_string();
    assert!(msg.contains("write"));
    assert!(msg.contains("permission denied"));
}

#[test]
fn config_error_serialize_has_message() {
    let err = ConfigError::Serialize("invalid utf-8".to_string());
    let msg = err.to_string();
    assert!(msg.contains("serialize"));
}

#[test]
fn config_error_deserialize_has_message() {
    let err = ConfigError::Deserialize("missing field".to_string());
    let msg = err.to_string();
    assert!(msg.contains("deserialize"));
}

// ============================================================================
// ProtocolError Tests
// ============================================================================

#[test]
fn protocol_error_connection_failed_has_message() {
    let err = ProtocolError::ConnectionFailed("timeout".to_string());
    let msg = err.to_string();
    assert!(msg.contains("Connection failed"));
    assert!(msg.contains("timeout"));
}

#[test]
fn protocol_error_auth_failed_has_message() {
    let err = ProtocolError::AuthFailed("invalid password".to_string());
    let msg = err.to_string();
    assert!(msg.contains("Authentication failed"));
}

#[test]
fn protocol_error_client_not_found_has_path() {
    let err = ProtocolError::ClientNotFound(PathBuf::from("/usr/bin/ssh"));
    let msg = err.to_string();
    assert!(msg.contains("Client not found"));
    assert!(msg.contains("ssh"));
}

#[test]
fn protocol_error_invalid_config_has_message() {
    let err = ProtocolError::InvalidConfig("missing host".to_string());
    let msg = err.to_string();
    assert!(msg.contains("Invalid configuration"));
}

#[test]
fn protocol_error_command_failed_has_message() {
    let err = ProtocolError::CommandFailed("exit code 1".to_string());
    let msg = err.to_string();
    assert!(msg.contains("Command execution failed"));
}

#[test]
fn protocol_error_unsupported_feature_has_message() {
    let err = ProtocolError::UnsupportedFeature("multi-hop".to_string());
    let msg = err.to_string();
    assert!(msg.contains("Unsupported feature"));
}

// ============================================================================
// SecretError Tests
// ============================================================================

#[test]
fn secret_error_connection_failed_has_message() {
    let err = SecretError::ConnectionFailed("dbus error".to_string());
    let msg = err.to_string();
    assert!(msg.contains("connect"));
    assert!(msg.contains("dbus error"));
}

#[test]
fn secret_error_store_failed_has_message() {
    let err = SecretError::StoreFailed("keyring locked".to_string());
    let msg = err.to_string();
    assert!(msg.contains("store"));
}

#[test]
fn secret_error_retrieve_failed_has_message() {
    let err = SecretError::RetrieveFailed("not found".to_string());
    let msg = err.to_string();
    assert!(msg.contains("retrieve"));
}

#[test]
fn secret_error_delete_failed_has_message() {
    let err = SecretError::DeleteFailed("permission denied".to_string());
    let msg = err.to_string();
    assert!(msg.contains("delete"));
}

#[test]
fn secret_error_backend_unavailable_has_message() {
    let err = SecretError::BackendUnavailable("libsecret".to_string());
    let msg = err.to_string();
    assert!(msg.contains("not available"));
}

#[test]
fn secret_error_keepassxc_has_message() {
    let err = SecretError::KeePassXC("database locked".to_string());
    let msg = err.to_string();
    assert!(msg.contains("KeePassXC"));
}

#[test]
fn secret_error_libsecret_has_message() {
    let err = SecretError::LibSecret("service unavailable".to_string());
    let msg = err.to_string();
    assert!(msg.contains("libsecret"));
}

#[test]
fn secret_error_bitwarden_has_message() {
    let err = SecretError::Bitwarden("vault locked".to_string());
    let msg = err.to_string();
    assert!(msg.contains("Bitwarden"));
}

// ============================================================================
// ImportError Tests
// ============================================================================

#[test]
fn import_error_parse_has_source_and_reason() {
    let err = ImportError::ParseError {
        source_name: "SSH config".to_string(),
        reason: "invalid syntax".to_string(),
    };
    let msg = err.to_string();
    assert!(msg.contains("SSH config"));
    assert!(msg.contains("invalid syntax"));
}

#[test]
fn import_error_unsupported_format_has_message() {
    let err = ImportError::UnsupportedFormat("putty".to_string());
    let msg = err.to_string();
    assert!(msg.contains("Unsupported format"));
    assert!(msg.contains("putty"));
}

#[test]
fn import_error_file_not_found_has_path() {
    let err = ImportError::FileNotFound(PathBuf::from("/home/user/.ssh/config"));
    let msg = err.to_string();
    assert!(msg.contains("not found"));
    assert!(msg.contains("config"));
}

#[test]
fn import_error_invalid_entry_has_source_and_reason() {
    let err = ImportError::InvalidEntry {
        source_name: "Remmina".to_string(),
        reason: "missing host".to_string(),
    };
    let msg = err.to_string();
    assert!(msg.contains("Remmina"));
    assert!(msg.contains("missing host"));
}

#[test]
fn import_error_cancelled_has_message() {
    let err = ImportError::Cancelled;
    let msg = err.to_string();
    assert!(msg.contains("cancelled"));
}

#[test]
fn import_error_io_converts_from_std_io() {
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
    let err: ImportError = io_err.into();
    let msg = err.to_string();
    assert!(msg.contains("IO error"));
}

// ============================================================================
// SessionError Tests
// ============================================================================

#[test]
fn session_error_start_failed_has_message() {
    let err = SessionError::StartFailed("process spawn failed".to_string());
    let msg = err.to_string();
    assert!(msg.contains("start session"));
}

#[test]
fn session_error_terminate_failed_has_message() {
    let err = SessionError::TerminateFailed("process not found".to_string());
    let msg = err.to_string();
    assert!(msg.contains("terminate"));
}

#[test]
fn session_error_not_found_has_id() {
    let err = SessionError::NotFound("abc-123".to_string());
    let msg = err.to_string();
    assert!(msg.contains("not found"));
    assert!(msg.contains("abc-123"));
}

#[test]
fn session_error_already_exists_has_id() {
    let err = SessionError::AlreadyExists("xyz-789".to_string());
    let msg = err.to_string();
    assert!(msg.contains("already exists"));
}

#[test]
fn session_error_process_error_has_message() {
    let err = SessionError::ProcessError("zombie process".to_string());
    let msg = err.to_string();
    assert!(msg.contains("Process error"));
}

#[test]
fn session_error_terminal_error_has_message() {
    let err = SessionError::TerminalError("pty allocation failed".to_string());
    let msg = err.to_string();
    assert!(msg.contains("Terminal error"));
}

#[test]
fn session_error_logging_error_has_message() {
    let err = SessionError::LoggingError("disk full".to_string());
    let msg = err.to_string();
    assert!(msg.contains("Logging error"));
}

// ============================================================================
// RustConnError Tests
// ============================================================================

#[test]
fn rustconn_error_from_config_error() {
    let config_err = ConfigError::Parse("test".to_string());
    let err: RustConnError = config_err.into();
    let msg = err.to_string();
    assert!(msg.contains("Configuration error"));
}

#[test]
fn rustconn_error_from_protocol_error() {
    let protocol_err = ProtocolError::ConnectionFailed("test".to_string());
    let err: RustConnError = protocol_err.into();
    let msg = err.to_string();
    assert!(msg.contains("Protocol error"));
}

#[test]
fn rustconn_error_from_secret_error() {
    let secret_err = SecretError::StoreFailed("test".to_string());
    let err: RustConnError = secret_err.into();
    let msg = err.to_string();
    assert!(msg.contains("Secret storage error"));
}

#[test]
fn rustconn_error_from_import_error() {
    let import_err = ImportError::Cancelled;
    let err: RustConnError = import_err.into();
    let msg = err.to_string();
    assert!(msg.contains("Import error"));
}

#[test]
fn rustconn_error_from_session_error() {
    let session_err = SessionError::NotFound("test".to_string());
    let err: RustConnError = session_err.into();
    let msg = err.to_string();
    assert!(msg.contains("Session error"));
}

#[test]
fn rustconn_error_from_io_error() {
    let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "access denied");
    let err: RustConnError = io_err.into();
    let msg = err.to_string();
    assert!(msg.contains("IO error"));
}

// ============================================================================
// Error Debug Trait Tests
// ============================================================================

#[test]
fn config_error_debug_format() {
    let err = ConfigError::Parse("test".to_string());
    let debug = format!("{err:?}");
    assert!(debug.contains("Parse"));
}

#[test]
fn protocol_error_debug_format() {
    let err = ProtocolError::ConnectionFailed("test".to_string());
    let debug = format!("{err:?}");
    assert!(debug.contains("ConnectionFailed"));
}

#[test]
fn secret_error_debug_format() {
    let err = SecretError::KeePassXC("test".to_string());
    let debug = format!("{err:?}");
    assert!(debug.contains("KeePassXC"));
}

#[test]
fn import_error_debug_format() {
    let err = ImportError::Cancelled;
    let debug = format!("{err:?}");
    assert!(debug.contains("Cancelled"));
}

#[test]
fn session_error_debug_format() {
    let err = SessionError::NotFound("test".to_string());
    let debug = format!("{err:?}");
    assert!(debug.contains("NotFound"));
}

#[test]
fn rustconn_error_debug_format() {
    let err: RustConnError = ConfigError::Parse("test".to_string()).into();
    let debug = format!("{err:?}");
    assert!(debug.contains("Config"));
}
