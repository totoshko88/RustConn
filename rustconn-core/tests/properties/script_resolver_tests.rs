//! Property-based tests for Script credential resolution
//!
//! Tests cover `PasswordSource::Script` serde round-trip and command string
//! preservation through serialization/deserialization.

use proptest::prelude::*;
use rustconn_core::models::PasswordSource;

// ---------------------------------------------------------------------------
// Proptest 10: Serde round-trip for PasswordSource::Script(command)
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn serde_roundtrip_password_source_script(
        command in "[a-zA-Z0-9 /._-]{1,100}"
    ) {
        let source = PasswordSource::Script(command.clone());
        let json = serde_json::to_string(&source).expect("serialize");
        let deserialized: PasswordSource = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(&deserialized, &source);
        // Verify the command string is preserved
        if let PasswordSource::Script(ref cmd) = deserialized {
            prop_assert_eq!(cmd, &command);
        } else {
            prop_assert!(false, "Expected PasswordSource::Script, got {:?}", deserialized);
        }
    }
}

// ---------------------------------------------------------------------------
// Proptest 11: Arbitrary command string preserved through serialize/deserialize
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn arbitrary_command_string_preserved(
        command in "\\PC{1,200}"
    ) {
        let source = PasswordSource::Script(command.clone());
        let json = serde_json::to_string(&source).expect("serialize");
        let deserialized: PasswordSource = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(&deserialized, &source);
    }
}
