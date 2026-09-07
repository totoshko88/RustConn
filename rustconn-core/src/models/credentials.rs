//! Credentials model for secure credential storage.

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Credentials for connection authentication
///
/// Note: Passwords and passphrases are stored as `SecretString` for in-memory security,
/// but serialization is handled specially to avoid exposing secrets in config files.
/// In practice, credentials should be stored in a secure backend (`KeePassXC`, libsecret).
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
    /// SSH key passphrase (stored securely, not serialized to config files)
    pub key_passphrase: Option<SecretString>,
    /// Domain for Windows/RDP authentication
    pub domain: Option<String>,
}

impl std::fmt::Debug for Credentials {
    /// Redacts the password and key passphrase; shows only their presence.
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
            .field(
                "key_passphrase",
                &if self.key_passphrase.is_some() {
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
            key_passphrase: None,
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
            key_passphrase: None,
        }
    }

    /// Creates credentials with username only
    #[must_use]
    pub fn with_username(username: impl Into<String>) -> Self {
        Self {
            username: Some(username.into()),
            domain: None,
            password: None,
            key_passphrase: None,
        }
    }

    /// Creates credentials with username and password
    #[must_use]
    pub fn with_password(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: Some(username.into()),
            domain: None,
            password: Some(SecretString::from(password.into())),
            key_passphrase: None,
        }
    }

    /// Returns true if credentials contain a password
    #[must_use]
    pub const fn has_password(&self) -> bool {
        self.password.is_some()
    }

    /// Returns true if credentials contain a key passphrase
    #[must_use]
    pub const fn has_key_passphrase(&self) -> bool {
        self.key_passphrase.is_some()
    }

    /// Returns true if credentials are empty (no username, password, or passphrase)
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.username.is_none() && self.password.is_none() && self.key_passphrase.is_none()
    }

    /// Exposes the password for use (should be used carefully)
    #[must_use]
    pub fn expose_password(&self) -> Option<&str> {
        self.password
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret)
    }

    /// Exposes the key passphrase for use (should be used carefully)
    #[must_use]
    pub fn expose_key_passphrase(&self) -> Option<&str> {
        self.key_passphrase
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
            && match (&self.key_passphrase, &other.key_passphrase) {
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
    /// secret field. Prove the password and passphrase never reach the output,
    /// while non-secret fields still do (M-PUBLIC-DEBUG).
    #[test]
    fn credentials_debug_does_not_leak() {
        const PASSWORD: &str = "password-sentinel";
        const PASSPHRASE: &str = "passphrase-sentinel";

        let creds = Credentials {
            username: Some("alice".to_string()),
            password: Some(SecretString::from(PASSWORD)),
            key_passphrase: Some(SecretString::from(PASSPHRASE)),
            domain: Some("EXAMPLE".to_string()),
        };

        let rendered = format!("{creds:?}");

        assert!(
            !rendered.contains(PASSWORD),
            "password leaked into Debug: {rendered}"
        );
        assert!(
            !rendered.contains(PASSPHRASE),
            "key passphrase leaked into Debug: {rendered}"
        );
        // Non-secret fields are still useful in a debug dump.
        assert!(rendered.contains("alice"), "username missing: {rendered}");
        assert!(rendered.contains("EXAMPLE"), "domain missing: {rendered}");
        assert!(
            rendered.contains("<set>"),
            "expected a `<set>` presence marker: {rendered}"
        );
    }
}
