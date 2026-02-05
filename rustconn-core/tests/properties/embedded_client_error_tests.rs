//! Property tests for `EmbeddedClientError` unified error type.
//!
//! **Validates: Requirements 2.1, 2.2**
//!
//! These tests verify that the unified error type correctly handles all variants
//! and maintains backward compatibility through type aliases.

use proptest::prelude::*;
use rustconn_core::embedded_client_error::{
    EmbeddedClientError, RdpClientError, SpiceClientError, VncClientError,
};

/// Strategy for generating arbitrary error messages
fn error_message_strategy() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-zA-Z0-9 _.-]{0,100}")
        .expect("valid regex")
        .prop_filter("non-empty for meaningful errors", |s| !s.is_empty())
}

/// Strategy for generating arbitrary `EmbeddedClientError` variants
fn embedded_client_error_strategy() -> impl Strategy<Value = EmbeddedClientError> {
    prop_oneof![
        error_message_strategy().prop_map(EmbeddedClientError::ConnectionFailed),
        error_message_strategy().prop_map(EmbeddedClientError::AuthenticationFailed),
        error_message_strategy().prop_map(EmbeddedClientError::ProtocolError),
        error_message_strategy().prop_map(EmbeddedClientError::IoError),
        Just(EmbeddedClientError::NotConnected),
        Just(EmbeddedClientError::AlreadyConnected),
        error_message_strategy().prop_map(EmbeddedClientError::InvalidConfig),
        error_message_strategy().prop_map(EmbeddedClientError::ChannelError),
        Just(EmbeddedClientError::Timeout),
        error_message_strategy().prop_map(EmbeddedClientError::ServerDisconnected),
        error_message_strategy().prop_map(EmbeddedClientError::Unsupported),
        error_message_strategy().prop_map(EmbeddedClientError::TlsError),
        error_message_strategy().prop_map(EmbeddedClientError::UsbRedirectionError),
        error_message_strategy().prop_map(EmbeddedClientError::SharedFolderError),
        Just(EmbeddedClientError::NativeClientNotAvailable),
    ]
}

proptest! {
    /// **Property 2.1: Error Display Consistency**
    ///
    /// All error variants must produce non-empty display strings.
    #[test]
    fn prop_error_display_non_empty(error in embedded_client_error_strategy()) {
        let display = error.to_string();
        prop_assert!(!display.is_empty(), "Error display should not be empty");
    }

    /// **Property 2.2: Type Alias Compatibility**
    ///
    /// Type aliases must be interchangeable with the base type.
    #[test]
    fn prop_type_alias_rdp_compatibility(error in embedded_client_error_strategy()) {
        // RdpClientError should be assignable from EmbeddedClientError
        let rdp_error: RdpClientError = error.clone();
        prop_assert_eq!(rdp_error.to_string(), error.to_string());
    }

    /// **Property 2.3: Type Alias VNC Compatibility**
    #[test]
    fn prop_type_alias_vnc_compatibility(error in embedded_client_error_strategy()) {
        let vnc_error: VncClientError = error.clone();
        prop_assert_eq!(vnc_error.to_string(), error.to_string());
    }

    /// **Property 2.4: Type Alias SPICE Compatibility**
    #[test]
    fn prop_type_alias_spice_compatibility(error in embedded_client_error_strategy()) {
        let spice_error: SpiceClientError = error.clone();
        prop_assert_eq!(spice_error.to_string(), error.to_string());
    }

    /// **Property 2.5: Clone Equality**
    ///
    /// Cloned errors must produce identical display strings.
    #[test]
    fn prop_clone_equality(error in embedded_client_error_strategy()) {
        let cloned = error.clone();
        prop_assert_eq!(error.to_string(), cloned.to_string());
    }

    /// **Property 2.6: IO Error Conversion**
    ///
    /// std::io::Error must convert to IoError variant.
    #[test]
    fn prop_io_error_conversion(kind in prop_oneof![
        Just(std::io::ErrorKind::NotFound),
        Just(std::io::ErrorKind::PermissionDenied),
        Just(std::io::ErrorKind::ConnectionRefused),
        Just(std::io::ErrorKind::ConnectionReset),
        Just(std::io::ErrorKind::TimedOut),
    ], msg in error_message_strategy()) {
        let io_err = std::io::Error::new(kind, msg.clone());
        let embedded_err: EmbeddedClientError = io_err.into();

        match embedded_err {
            EmbeddedClientError::IoError(s) => {
                prop_assert!(s.contains(&msg), "IoError should contain original message");
            }
            _ => prop_assert!(false, "Expected IoError variant"),
        }
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn test_all_variants_have_display() {
        let variants = vec![
            EmbeddedClientError::ConnectionFailed("test".to_string()),
            EmbeddedClientError::AuthenticationFailed("test".to_string()),
            EmbeddedClientError::ProtocolError("test".to_string()),
            EmbeddedClientError::IoError("test".to_string()),
            EmbeddedClientError::NotConnected,
            EmbeddedClientError::AlreadyConnected,
            EmbeddedClientError::InvalidConfig("test".to_string()),
            EmbeddedClientError::ChannelError("test".to_string()),
            EmbeddedClientError::Timeout,
            EmbeddedClientError::ServerDisconnected("test".to_string()),
            EmbeddedClientError::Unsupported("test".to_string()),
            EmbeddedClientError::TlsError("test".to_string()),
            EmbeddedClientError::UsbRedirectionError("test".to_string()),
            EmbeddedClientError::SharedFolderError("test".to_string()),
            EmbeddedClientError::NativeClientNotAvailable,
        ];

        for variant in variants {
            let display = variant.to_string();
            assert!(
                !display.is_empty(),
                "Variant {:?} has empty display",
                variant
            );
        }
    }

    #[test]
    fn test_debug_impl() {
        let error = EmbeddedClientError::ConnectionFailed("timeout".to_string());
        let debug = format!("{:?}", error);
        assert!(debug.contains("ConnectionFailed"));
        assert!(debug.contains("timeout"));
    }
}
