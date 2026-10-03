//! Secret backend trait definition
//!
//! This module defines the `SecretBackend` trait that all secret storage
//! implementations must implement.

use async_trait::async_trait;

use crate::error::SecretResult;
use crate::models::Credentials;

/// Fine-grained availability state of a secret backend.
///
/// Distinguishes a missing client (binary/library absent) from a present
/// client whose backing service does not respond, so the UI can surface an
/// accurate, actionable signal instead of a single boolean.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendAvailability {
    /// The backend is present and its service answers.
    Available,
    /// The client (binary or library) needed to reach the backend is absent.
    ClientMissing,
    /// The client is present but the backing service does not respond.
    ServiceUnavailable,
}

/// Abstraction over secret storage backends
///
/// This trait defines the interface for storing, retrieving, and deleting
/// credentials from various secret storage backends like `KeePassXC` or libsecret.
#[async_trait]
pub trait SecretBackend: Send + Sync {
    /// Store credentials for a connection
    ///
    /// # Arguments
    /// * `connection_id` - Unique identifier for the connection
    /// * `credentials` - The credentials to store
    ///
    /// # Errors
    /// Returns `SecretError` if the storage operation fails
    async fn store(&self, connection_id: &str, credentials: &Credentials) -> SecretResult<()>;

    /// Retrieve credentials for a connection
    ///
    /// # Arguments
    /// * `connection_id` - Unique identifier for the connection
    ///
    /// # Returns
    /// `Some(Credentials)` if found, `None` if not found
    ///
    /// # Errors
    /// Returns `SecretError` if the retrieval operation fails
    async fn retrieve(&self, connection_id: &str) -> SecretResult<Option<Credentials>>;

    /// Delete credentials for a connection
    ///
    /// # Arguments
    /// * `connection_id` - Unique identifier for the connection
    ///
    /// # Errors
    /// Returns `SecretError` if the deletion operation fails
    async fn delete(&self, connection_id: &str) -> SecretResult<()>;

    /// Check if the backend is available and operational
    ///
    /// # Returns
    /// `true` if the backend is available, `false` otherwise
    async fn is_available(&self) -> bool;

    /// Reports fine-grained backend availability.
    ///
    /// The default implementation derives from [`Self::is_available`], mapping
    /// `true` to [`BackendAvailability::Available`] and `false` to
    /// [`BackendAvailability::ClientMissing`]. Backends that can distinguish a
    /// present-but-unresponsive service should override this to return
    /// [`BackendAvailability::ServiceUnavailable`].
    async fn availability(&self) -> BackendAvailability {
        if self.is_available().await {
            BackendAvailability::Available
        } else {
            BackendAvailability::ClientMissing
        }
    }

    /// Returns the backend identifier
    ///
    /// # Returns
    /// A static string identifying this backend (e.g., "keepassxc", "libsecret")
    fn backend_id(&self) -> &'static str;

    /// Returns a human-readable name for this backend
    ///
    /// # Returns
    /// A static string with the display name (e.g., "`KeePassXC`", "GNOME Keyring")
    fn display_name(&self) -> &'static str;

    /// Whether this backend is in read-only mode.
    ///
    /// A read-only backend refuses every mutating operation ([`Self::store`],
    /// [`Self::delete`]) with [`SecretError::ReadOnly`] and never touches the
    /// vault. Reads ([`Self::retrieve`]) are unaffected. The default is `false`
    /// — writes are allowed — so a backend that cannot be put in read-only mode
    /// (or has not been) keeps its historical behaviour.
    ///
    /// Backends model the flag however they construct themselves (typically a
    /// `bool` field set by a `with_read_only` builder); enforcement is uniform
    /// through [`Self::ensure_writable`].
    fn is_read_only(&self) -> bool {
        false
    }

    /// Returns `Ok(())` when a write may proceed, or
    /// [`SecretError::ReadOnly`] naming this backend when it is read-only.
    ///
    /// Call this at the top of [`Self::store`] and [`Self::delete`] before any
    /// side effect, so a read-only vault is left completely untouched. Provided
    /// so every backend enforces read-only identically and the error message is
    /// consistent.
    ///
    /// # Errors
    /// Returns [`SecretError::ReadOnly`] when [`Self::is_read_only`] is `true`.
    fn ensure_writable(&self) -> SecretResult<()> {
        if self.is_read_only() {
            Err(crate::error::SecretError::ReadOnly(
                self.display_name().to_string(),
            ))
        } else {
            Ok(())
        }
    }

    /// Whether this backend widens credential lookups to the entire vault/store
    /// root instead of only the hardcoded `RustConn` scope.
    ///
    /// By default (`false`) a backend looks for entries only under its
    /// `RustConn` folder / vault / group — the historical behaviour. When
    /// `true`, [`Self::retrieve`] searches from the vault root so credentials
    /// that live outside the `RustConn` subtree (e.g. entries the user created
    /// by hand, or imported from another tool) are found as well.
    ///
    /// This widens **reads only**. Writes continue to target the `RustConn`
    /// scope so that newly stored entries land in one predictable place and the
    /// root-prefix handling that issue #327 fixed is never exercised by a
    /// root-search read.
    fn searches_from_root(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod capability_tests {
    use super::*;
    use crate::error::SecretError;
    use crate::models::Credentials;

    /// A minimal backend whose read-only / root-search state is set at
    /// construction, used to exercise the trait-provided enforcement in
    /// isolation from any real vault.
    struct FakeBackend {
        read_only: bool,
        root_search: bool,
    }

    #[async_trait]
    impl SecretBackend for FakeBackend {
        async fn store(&self, _id: &str, _creds: &Credentials) -> SecretResult<()> {
            self.ensure_writable()?;
            Ok(())
        }

        async fn retrieve(&self, _id: &str) -> SecretResult<Option<Credentials>> {
            Ok(None)
        }

        async fn delete(&self, _id: &str) -> SecretResult<()> {
            self.ensure_writable()?;
            Ok(())
        }

        async fn is_available(&self) -> bool {
            true
        }

        fn backend_id(&self) -> &'static str {
            "fake"
        }

        fn display_name(&self) -> &'static str {
            "Fake Backend"
        }

        fn is_read_only(&self) -> bool {
            self.read_only
        }

        fn searches_from_root(&self) -> bool {
            self.root_search
        }
    }

    #[test]
    fn defaults_are_writable_and_scoped() {
        // A backend that does not override the capability methods must behave
        // exactly as before: writable, scoped to the RustConn subtree.
        struct Bare;
        #[async_trait]
        impl SecretBackend for Bare {
            async fn store(&self, _id: &str, _c: &Credentials) -> SecretResult<()> {
                Ok(())
            }
            async fn retrieve(&self, _id: &str) -> SecretResult<Option<Credentials>> {
                Ok(None)
            }
            async fn delete(&self, _id: &str) -> SecretResult<()> {
                Ok(())
            }
            async fn is_available(&self) -> bool {
                true
            }
            fn backend_id(&self) -> &'static str {
                "bare"
            }
            fn display_name(&self) -> &'static str {
                "Bare"
            }
        }
        let b = Bare;
        assert!(!b.is_read_only());
        assert!(!b.searches_from_root());
        assert!(b.ensure_writable().is_ok());
    }

    #[tokio::test]
    async fn read_only_refuses_store_and_delete_without_touching_vault() {
        let b = FakeBackend {
            read_only: true,
            root_search: false,
        };
        let creds = Credentials::default();

        let store_err = b.store("conn-1", &creds).await.unwrap_err();
        let delete_err = b.delete("conn-1").await.unwrap_err();

        // Both must be the dedicated ReadOnly variant naming the backend, not a
        // generic StoreFailed/DeleteFailed that looks like the write was tried.
        match store_err {
            SecretError::ReadOnly(name) => assert_eq!(name, "Fake Backend"),
            other => panic!("expected ReadOnly, got {other:?}"),
        }
        match delete_err {
            SecretError::ReadOnly(name) => assert_eq!(name, "Fake Backend"),
            other => panic!("expected ReadOnly, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn writable_backend_allows_store_and_delete() {
        let b = FakeBackend {
            read_only: false,
            root_search: true,
        };
        let creds = Credentials::default();
        assert!(b.store("conn-1", &creds).await.is_ok());
        assert!(b.delete("conn-1").await.is_ok());
        assert!(b.searches_from_root());
    }
}
