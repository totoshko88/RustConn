//! Serde helpers for secret deserialization.
//!
//! These helpers wrap incoming password fields directly in [`SecretString`]
//! so the plaintext is never materialised as a plain `String` on the heap.
//! Use them on backend response structs (Bitwarden, Passbolt, `KeePassXC`,
//! libvirt XML, RDM imports) to satisfy `secrets-guide.md` rule #6.
//!
//! Example:
//!
//! ```ignore
//! use serde::Deserialize;
//! use crate::secret::serde_helpers::deserialize_optional_secret;
//!
//! #[derive(Deserialize)]
//! struct ApiResponse {
//!     #[serde(default, deserialize_with = "deserialize_optional_secret")]
//!     password: Option<secrecy::SecretString>,
//! }
//! ```

use secrecy::SecretString;
use serde::{Deserialize, Deserializer};

/// Deserializes `Option<String>` into `Option<SecretString>` without
/// keeping the plaintext in a long-lived `String`.
///
/// `serde_json` allocates the `String` for the borrowed JSON text
/// regardless; this helper only ensures the value lives inside a
/// `SecretString` immediately afterwards (which redacts itself in `Debug`
/// and zeroises on drop).
///
/// # Errors
///
/// Returns the underlying deserialization error if the field is not a
/// JSON string or null.
pub fn deserialize_optional_secret<'de, D>(
    deserializer: D,
) -> Result<Option<SecretString>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<String> = Option::deserialize(deserializer)?;
    Ok(opt.map(SecretString::from))
}

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret;
    use serde::Deserialize;

    use super::*;

    #[derive(Deserialize)]
    struct Wrapper {
        #[serde(default, deserialize_with = "deserialize_optional_secret")]
        password: Option<SecretString>,
    }

    #[test]
    fn deserializes_json_string_into_secret() {
        let json = r#"{"password": "hunter2"}"#;
        let parsed: Wrapper = serde_json::from_str(json).expect("parse");
        let secret = parsed.password.expect("Some");
        assert_eq!(secret.expose_secret(), "hunter2");
    }

    #[test]
    fn deserializes_null_as_none() {
        let json = r#"{"password": null}"#;
        let parsed: Wrapper = serde_json::from_str(json).expect("parse");
        assert!(parsed.password.is_none());
    }

    #[test]
    fn deserializes_missing_field_as_none() {
        let json = "{}";
        let parsed: Wrapper = serde_json::from_str(json).expect("parse");
        assert!(parsed.password.is_none());
    }

    #[test]
    fn debug_does_not_leak_secret() {
        let secret = SecretString::from("hunter2");
        let rendered = format!("{secret:?}");
        assert!(
            !rendered.contains("hunter2"),
            "Debug leaked secret: {rendered}"
        );
    }
}
