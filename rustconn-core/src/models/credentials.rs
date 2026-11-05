//! Credentials model for secure credential storage.

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Credentials for connection authentication
///
/// Note: Passwords are stored as `SecretString` for in-memory security,
/// but serialization is handled specially to avoid exposing secrets in config files.
/// In practice, credentials should be stored in a secure backend (`KeePassXC`, libsecret).
///
/// There is no SSH key passphrase here, on purpose: RustConn neither stores a
/// key passphrase in a secret backend nor fetches one from it, so SSH login
/// never depends on the vault. A passphrase is asked for when it is needed,
/// used for one `ssh-add` and dropped. Releases before 0.24.6 had a
/// `key_passphrase` field; a value they stored is left where it is and
/// ignored (see `.kiro/steering/secrets-guide.md`).
///
/// `Debug` is implemented manually rather than derived. `SecretString` redacts
/// itself today, but a derived `Debug` would silently start leaking the moment
/// a plain-`String` secret field is added; the manual impl and its test
/// (`credentials_debug_does_not_leak`) make that a compile-and-test guarantee
/// (M-PUBLIC-DEBUG).
#[derive(Clone)]
pub struct Credentials {
    /// Username for authentication
    pub username: Option<String>,
    /// Password (stored securely, not serialized to config files)
    pub password: Option<SecretString>,
    /// Domain for Windows/RDP authentication
    pub domain: Option<String>,
}

impl std::fmt::Debug for Credentials {
    /// Redacts the password; shows only its presence.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("username", &self.username)
            .field(
                "password",
                &if self.password.is_some() {
                    "<set>"
                } else {
                    "<none>"
                },
            )
            .field("domain", &self.domain)
            .finish()
    }
}

/// Serializable representation of credentials (without secrets)
///
/// Unknown fields are ignored, so data written with the legacy
/// `key_passphrase` field still loads; the value is dropped.
#[derive(Serialize, Deserialize)]
struct CredentialsSerde {
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    domain: Option<String>,
    // Passwords are intentionally not serialized for security
    // They should be stored in a secure backend
}

impl Serialize for Credentials {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        CredentialsSerde {
            username: self.username.clone(),
            domain: self.domain.clone(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Credentials {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let serde = CredentialsSerde::deserialize(deserializer)?;
        Ok(Self {
            username: serde.username,
            domain: serde.domain,
            password: None,
        })
    }
}

impl Credentials {
    /// Creates empty credentials
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            username: None,
            domain: None,
            password: None,
        }
    }

    /// Creates credentials with username only
    #[must_use]
    pub fn with_username(username: impl Into<String>) -> Self {
        Self {
            username: Some(username.into()),
            domain: None,
            password: None,
        }
    }

    /// Creates credentials with username and password
    #[must_use]
    pub fn with_password(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: Some(username.into()),
            domain: None,
            password: Some(SecretString::from(password.into())),
        }
    }

    /// Returns true if credentials contain a password
    #[must_use]
    pub const fn has_password(&self) -> bool {
        self.password.is_some()
    }

    /// Returns true if credentials are empty (no username or password)
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.username.is_none() && self.password.is_none()
    }

    /// Exposes the password for use (should be used carefully)
    #[must_use]
    pub fn expose_password(&self) -> Option<&str> {
        self.password
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret)
    }
}

impl Default for Credentials {
    fn default() -> Self {
        Self::empty()
    }
}

// Manual PartialEq implementation since SecretString doesn't implement it
impl PartialEq for Credentials {
    fn eq(&self, other: &Self) -> bool {
        self.username == other.username
            && match (&self.password, &other.password) {
                (Some(a), Some(b)) => a.expose_secret() == b.expose_secret(),
                (None, None) => true,
                _ => false,
            }
            && self.domain == other.domain
    }
}

#[cfg(test)]
mod tests {
    use super::Credentials;
    use secrecy::SecretString;

    /// `Credentials` has a manual `Debug` guarding against a future plain-text
    /// secret field. Prove the password never reaches the output, while
    /// non-secret fields still do (M-PUBLIC-DEBUG).
    #[test]
    fn credentials_debug_does_not_leak() {
        const PASSWORD: &str = "password-sentinel";

        let creds = Credentials {
            username: Some("alice".to_string()),
            password: Some(SecretString::from(PASSWORD)),
            domain: Some("EXAMPLE".to_string()),
        };

        let rendered = format!("{creds:?}");

        assert!(
            !rendered.contains(PASSWORD),
            "password leaked into Debug: {rendered}"
        );
        // Non-secret fields are still useful in a debug dump.
        assert!(rendered.contains("alice"), "username missing: {rendered}");
        assert!(rendered.contains("EXAMPLE"), "domain missing: {rendered}");
        assert!(
            rendered.contains("<set>"),
            "expected a `<set>` presence marker: {rendered}"
        );
    }

    /// Data written by a release that still had the `key_passphrase` field
    /// loads; the passphrase is dropped and never comes back out.
    #[test]
    fn legacy_key_passphrase_field_still_deserializes_and_is_dropped() {
        const PASSPHRASE: &str = "legacy-passphrase-sentinel";
        let json =
            format!(r#"{{"username":"alice","key_passphrase":"{PASSPHRASE}","domain":"EXAMPLE"}}"#);

        let creds: Credentials = serde_json::from_str(&json).unwrap();

        assert_eq!(creds.username.as_deref(), Some("alice"));
        assert_eq!(creds.domain.as_deref(), Some("EXAMPLE"));
        assert!(creds.password.is_none());
        let reserialized = serde_json::to_string(&creds).unwrap();
        assert!(!reserialized.contains("key_passphrase"), "{reserialized}");
        assert!(!reserialized.contains(PASSPHRASE), "{reserialized}");
        assert!(!format!("{creds:?}").contains(PASSPHRASE));
    }
}
