//! System keyring helpers for storing/retrieving secret backend credentials.
//!
//! Every operation — read, write and delete — has a 5-second timeout ceiling to
//! prevent blocking the GTK main thread if the Secret Service daemon is
//! unresponsive. The reads need it as much as the writes: the Bitwarden unlock
//! handler calls its lookup synchronously from a button callback, so an
//! unresponsive daemon would otherwise wedge the UI with no way out.

use std::time::Duration;

/// Timeout for keyring operations (protects GTK main thread).
const KEYRING_SAVE_TIMEOUT: Duration = Duration::from_secs(5);

/// Runs one keyring lookup under the shared timeout ceiling.
///
/// `label` names the entry for the log line only; it never carries the secret.
/// A timeout is reported as "not found" on purpose: the caller's job is to decide
/// whether the keyring can supply the secret right now, and a wedged daemon
/// cannot.
fn lookup_in_keyring<F>(label: &'static str, lookup: F) -> Option<secrecy::SecretString>
where
    F: std::future::Future<
            Output = rustconn_core::error::SecretResult<Option<secrecy::SecretString>>,
        >,
{
    match crate::async_utils::with_runtime(|rt| {
        rt.block_on(async {
            tokio::time::timeout(KEYRING_SAVE_TIMEOUT, lookup)
                .await
                .map_err(|_| "keyring lookup timed out after 5s".to_string())?
                .map_err(|e| e.to_string())
        })
    }) {
        Ok(Ok(Some(secret))) => {
            tracing::debug!(entry = label, "Credential loaded from keyring");
            Some(secret)
        }
        Ok(Ok(None)) => {
            tracing::debug!(entry = label, "No credential found in keyring");
            None
        }
        Ok(Err(e)) => {
            tracing::debug!(entry = label, error = %e, "Failed to load credential from keyring");
            None
        }
        Err(e) => {
            tracing::debug!(entry = label, error = %e, "Runtime error loading credential from keyring");
            None
        }
    }
}

/// Runs one keyring write under the shared timeout ceiling.
///
/// `label` names the entry for the log line only; it never carries the secret.
/// Returns `true` on success, `false` on any failure (including a timeout).
fn save_to_keyring<F>(label: &'static str, store: F) -> bool
where
    F: std::future::Future<Output = rustconn_core::error::SecretResult<()>>,
{
    match crate::async_utils::with_runtime(|rt| {
        rt.block_on(async {
            tokio::time::timeout(KEYRING_SAVE_TIMEOUT, store)
                .await
                .map_err(|_| "keyring save timed out after 5s".to_string())?
                .map_err(|e| e.to_string())
        })
    }) {
        Ok(Ok(())) => {
            tracing::info!(entry = label, "Credential saved to keyring");
            true
        }
        Ok(Err(e)) => {
            tracing::warn!(entry = label, error = %e, "Failed to save credential to keyring");
            false
        }
        Err(e) => {
            tracing::warn!(entry = label, error = %e, "Runtime error saving credential to keyring");
            false
        }
    }
}

/// Saves Bitwarden master password to system keyring via rustconn-core.
///
/// Returns `true` on success, `false` on any failure.
pub(super) fn save_bw_password_to_keyring(password: &str) -> bool {
    let secret = secrecy::SecretString::from(password.to_owned());
    save_to_keyring(
        "Bitwarden master password",
        rustconn_core::secret::store_master_password_in_keyring(&secret),
    )
}

/// Loads Bitwarden master password from system keyring via rustconn-core
pub(super) fn get_bw_password_from_keyring() -> Option<secrecy::SecretString> {
    lookup_in_keyring(
        "bitwarden-master-password",
        rustconn_core::secret::get_master_password_from_keyring(),
    )
}

/// Saves 1Password service account token to system keyring.
///
/// Returns `true` on success, `false` on any failure.
pub(super) fn save_op_token_to_keyring(token: &str) -> bool {
    let secret = secrecy::SecretString::from(token.to_owned());
    save_to_keyring(
        "1Password token",
        rustconn_core::secret::store_token_in_keyring(&secret),
    )
}

/// Loads 1Password service account token from system keyring
pub(super) fn get_op_token_from_keyring() -> Option<secrecy::SecretString> {
    lookup_in_keyring(
        "onepassword-token",
        rustconn_core::secret::get_token_from_keyring(),
    )
}

/// Saves Passbolt GPG passphrase to system keyring.
///
/// Returns `true` on success, `false` on any failure.
pub(super) fn save_pb_passphrase_to_keyring(passphrase: &str) -> bool {
    let secret = secrecy::SecretString::from(passphrase.to_owned());
    save_to_keyring(
        "Passbolt passphrase",
        rustconn_core::secret::store_passphrase_in_keyring(&secret),
    )
}

/// Loads Passbolt GPG passphrase from system keyring
pub(super) fn get_pb_passphrase_from_keyring() -> Option<secrecy::SecretString> {
    lookup_in_keyring(
        "passbolt-passphrase",
        rustconn_core::secret::get_passphrase_from_keyring(),
    )
}

/// Saves the portable credential file passphrase to the system keyring.
///
/// Returns `true` on success, `false` on any failure.
pub(super) fn save_portable_passphrase_to_keyring(passphrase: &str) -> bool {
    let secret = secrecy::SecretString::from(passphrase.to_owned());
    save_to_keyring(
        "Portable file passphrase",
        rustconn_core::secret::store_portable_passphrase_in_keyring(&secret),
    )
}

/// Loads the portable credential file passphrase from the system keyring.
pub(super) fn get_portable_passphrase_from_keyring() -> Option<secrecy::SecretString> {
    lookup_in_keyring(
        "portable-file-passphrase",
        rustconn_core::secret::get_portable_passphrase_from_keyring(),
    )
}

/// Saves KDBX database password to system keyring.
///
/// Returns `true` on success, `false` on any failure.
pub(super) fn save_kdbx_password_to_keyring(password: &str) -> bool {
    let secret = secrecy::SecretString::from(password.to_owned());
    save_to_keyring(
        "KDBX password",
        rustconn_core::secret::store_kdbx_password_in_keyring(&secret),
    )
}

/// Loads KDBX database password from system keyring
pub(super) fn get_kdbx_password_from_keyring() -> Option<secrecy::SecretString> {
    lookup_in_keyring(
        "kdbx-password",
        rustconn_core::secret::get_kdbx_password_from_keyring(),
    )
}

/// Saves the Bitwarden API key pair to the system keyring.
///
/// Takes `SecretString`s rather than `&str` so no intermediate plaintext copy
/// is created — the two values go straight to `rustconn-core`.
///
/// Returns `true` on success, `false` on any failure.
pub(super) fn save_bw_api_credentials_to_keyring(
    client_id: &secrecy::SecretString,
    client_secret: &secrecy::SecretString,
) -> bool {
    let client_id = client_id.clone();
    let client_secret = client_secret.clone();
    save_to_keyring(
        "Bitwarden API credentials",
        rustconn_core::secret::store_api_credentials_in_keyring(&client_id, &client_secret),
    )
}

/// Runs one keyring deletion under the shared timeout ceiling.
///
/// `label` names the entry for the log line only; it never carries the secret.
/// Deletions are bounded by the same 5-second ceiling as the writes because a
/// wedged Secret Service daemon blocks `clear` exactly like `store`.
fn delete_from_keyring<F>(label: &'static str, delete: F) -> bool
where
    F: std::future::Future<Output = rustconn_core::error::SecretResult<()>> + Send,
{
    match crate::async_utils::with_runtime(|rt| {
        rt.block_on(async {
            tokio::time::timeout(KEYRING_SAVE_TIMEOUT, delete)
                .await
                .map_err(|_| "keyring delete timed out after 5s".to_string())?
                .map_err(|e| e.to_string())
        })
    }) {
        Ok(Ok(())) => {
            tracing::info!(entry = label, "Stale keyring entry removed");
            true
        }
        Ok(Err(e)) => {
            tracing::warn!(entry = label, error = %e, "Failed to remove stale keyring entry");
            false
        }
        Err(e) => {
            tracing::warn!(entry = label, error = %e, "Runtime error removing keyring entry");
            false
        }
    }
}

/// Removes the KDBX database password from the system keyring.
pub(super) fn delete_kdbx_password_from_keyring() -> bool {
    delete_from_keyring(
        "kdbx-password",
        rustconn_core::secret::delete_kdbx_password_from_keyring(),
    )
}

/// Removes the Bitwarden master password from the system keyring.
pub(super) fn delete_bw_password_from_keyring() -> bool {
    delete_from_keyring(
        "bitwarden-master-password",
        rustconn_core::secret::delete_master_password_from_keyring(),
    )
}

/// Removes the Bitwarden API key pair from the system keyring.
pub(super) fn delete_bw_api_credentials_from_keyring() -> bool {
    delete_from_keyring(
        "bitwarden-api-credentials",
        rustconn_core::secret::delete_api_credentials_from_keyring(),
    )
}

/// Removes the 1Password service account token from the system keyring.
pub(super) fn delete_op_token_from_keyring() -> bool {
    delete_from_keyring(
        "onepassword-token",
        rustconn_core::secret::delete_token_from_keyring(),
    )
}

/// Removes the Passbolt GPG passphrase from the system keyring.
pub(super) fn delete_pb_passphrase_from_keyring() -> bool {
    delete_from_keyring(
        "passbolt-passphrase",
        rustconn_core::secret::delete_passphrase_from_keyring(),
    )
}

/// Removes the portable credential file passphrase from the system keyring.
pub(super) fn delete_portable_passphrase_from_keyring() -> bool {
    delete_from_keyring(
        "portable-file-passphrase",
        rustconn_core::secret::delete_portable_passphrase_from_keyring(),
    )
}
