//! Shared keyring storage via the Secret Service API.
//!
//! Provides generic store/retrieve/delete operations for all secret backends
//! that need system keyring integration (GNOME Keyring, KDE Wallet, etc.).
//!
//! - On Linux/BSD (`cfg(not(target_os = "macos"))`) this talks to the Secret
//!   Service **in process** via the [`oo7`] crate (`oo7::dbus::Service`), so no
//!   `secret-tool` binary or bundled libsecret C library is required.
//! - On macOS the legacy `secret-tool` subprocess path is retained purely so the
//!   crate keeps compiling; it is never actually selected at runtime (macOS
//!   routes to the Keychain backend). Task 4.5 removes this macOS path entirely
//!   and gates the whole module behind `cfg(not(macos))`.
//!
//! Both paths use the *same* two attributes — `application` and `key` — and the
//! same labels, so entries written by the old `secret-tool` code remain findable
//! after the switch to oo7 (backward compatibility, R11.1).

use crate::error::{SecretError, SecretResult};

// oo7 attribute maps are only built on the in-process (non-macOS) path.
#[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
use std::collections::HashMap;

// secret-tool subprocess machinery is only used by the retained macOS path.
#[cfg(all(feature = "system-keyring", target_os = "macos"))]
use std::process::Stdio;
#[cfg(all(feature = "system-keyring", target_os = "macos"))]
use tokio::process::Command;

/// Application identifier used as the `application` attribute in keyring entries
#[cfg(feature = "system-keyring")]
const APP_ID: &str = "rustconn";

/// Builds the two-attribute map used to identify a keyring entry.
///
/// The scheme (`application` + `key`) is identical to the one the old
/// `secret-tool` path wrote, so items round-trip across both mechanisms
/// (backward compatibility, R11.1).
#[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
fn build_attributes(key: &str) -> HashMap<String, String> {
    let mut attrs = HashMap::new();
    attrs.insert("application".to_string(), APP_ID.to_string());
    attrs.insert("key".to_string(), key.to_string());
    attrs
}

// ---------------------------------------------------------------------------
// oo7 error mapping (R9.3) — shared by this module and `libsecret.rs`.
// ---------------------------------------------------------------------------

/// Returns `true` when an oo7 DBus error means the Secret Service / session bus
/// is unreachable (a transport/connection failure) rather than an
/// operation-level failure.
///
/// These always map to [`SecretError::BackendUnavailable`] regardless of the
/// operation, because the right recovery is to start or repair the Secret
/// Service, not to retry the store/search/delete. `ZBus`/`IO` are raw
/// wire/socket failures reaching the bus; the two `Service` sub-cases are a
/// broken transport (`ZBus`) or a vanished session (`NoSession`).
#[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
fn is_transport_failure(e: &oo7::dbus::Error) -> bool {
    use oo7::dbus::{Error, ServiceError};
    matches!(
        e,
        Error::ZBus(_)
            | Error::IO(_)
            | Error::Service(ServiceError::ZBus(_) | ServiceError::NoSession(_))
    )
}

/// Maps an oo7 error onto a [`SecretError`], honouring the R9.3 mapping.
///
/// Transport/connection failures win over the operation kind and become
/// `BackendUnavailable`. A `Crypto` failure is neither a transport problem nor
/// a natural store/search/delete failure, so it surfaces as the generic
/// `LibSecret` variant ("other"). Everything else becomes the operation's
/// natural variant supplied via `natural` (e.g. `SecretError::StoreFailed`).
///
/// Messages carry only oo7's `Display` (operation/attribute context) and never
/// secret material.
#[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
#[expect(
    clippy::needless_pass_by_value,
    reason = "mirrors the by-value map_err adapter signature; the three wrappers move the error in"
)]
fn map_oo7_error(
    e: oo7::dbus::Error,
    natural: fn(String) -> SecretError,
    context: &str,
) -> SecretError {
    if is_transport_failure(&e) {
        SecretError::BackendUnavailable(format!("Secret Service unavailable: {e}"))
    } else if matches!(e, oo7::dbus::Error::Crypto(_)) {
        SecretError::LibSecret(format!("{context}: {e}"))
    } else {
        natural(format!("{context}: {e}"))
    }
}

/// Maps a `Service::new()` / `default_collection()` failure onto `SecretError`.
///
/// Establishing the connection is a pure transport/service concern, so any
/// error here is always [`SecretError::BackendUnavailable`] (R9.3).
#[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
#[expect(
    clippy::needless_pass_by_value,
    reason = "passed directly to Result::map_err, which hands the error over by value"
)]
pub(crate) fn map_oo7_service_error(e: oo7::dbus::Error) -> SecretError {
    SecretError::BackendUnavailable(format!("Secret Service unavailable: {e}"))
}

/// Maps a create/update (`Collection::create_item`) failure onto `SecretError`.
#[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
pub(crate) fn map_oo7_store_error(e: oo7::dbus::Error) -> SecretError {
    map_oo7_error(e, SecretError::StoreFailed, "oo7 store failed")
}

/// Maps a search/read (`search_items` / `Item::secret`) failure onto `SecretError`.
#[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
pub(crate) fn map_oo7_retrieve_error(e: oo7::dbus::Error) -> SecretError {
    map_oo7_error(e, SecretError::RetrieveFailed, "oo7 retrieve failed")
}

/// Maps a delete (`Item::delete`) failure onto `SecretError`.
#[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
pub(crate) fn map_oo7_delete_error(e: oo7::dbus::Error) -> SecretError {
    map_oo7_error(e, SecretError::DeleteFailed, "oo7 delete failed")
}

// ---------------------------------------------------------------------------
// Linux/BSD: in-process oo7 Secret Service client.
// ---------------------------------------------------------------------------

/// Checks whether a Secret Service is reachable.
///
/// All keyring operations depend on a running Secret Service. If none answers,
/// callers should fall back to encrypted-settings storage and inform the user.
///
/// On the oo7 path there is no `secret-tool` binary to probe; this now means
/// "a Secret Service responded over D-Bus".
#[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
pub async fn is_secret_tool_available() -> bool {
    oo7::dbus::Service::new().await.is_ok()
}

/// Stores a value in the system keyring via oo7.
///
/// Uses `replace = true` so re-storing the same `key` overwrites the previous
/// item, and `window_id = None` since this runs headless in `rustconn-core`.
///
/// # Errors
/// Returns `SecretError::BackendUnavailable` if no Secret Service answers.
/// Returns `SecretError::StoreFailed` if the item cannot be written.
#[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
pub async fn store(key: &str, value: &str, label: &str) -> SecretResult<()> {
    let attrs = build_attributes(key);

    let service = oo7::dbus::Service::new()
        .await
        .map_err(map_oo7_service_error)?;
    let collection = service
        .default_collection()
        .await
        .map_err(map_oo7_service_error)?;

    // `Secret::text` stores the raw UTF-8 string with a `text/plain` content
    // type so values round-trip byte-for-byte like the old secret-tool path.
    collection
        .create_item(label, &attrs, oo7::Secret::text(value), true, None)
        .await
        .map_err(map_oo7_store_error)?;

    Ok(())
}

/// Retrieves a value from the system keyring via oo7.
///
/// Returns `Ok(None)` when the key does not exist.
///
/// # Errors
/// Returns `SecretError::BackendUnavailable` if no Secret Service answers.
/// Returns `SecretError::RetrieveFailed` if the search or read fails, or the
/// stored value is not valid UTF-8.
#[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
pub async fn lookup(key: &str) -> SecretResult<Option<String>> {
    let attrs = build_attributes(key);

    let service = oo7::dbus::Service::new()
        .await
        .map_err(map_oo7_service_error)?;
    let collection = service
        .default_collection()
        .await
        .map_err(map_oo7_service_error)?;

    let items = collection
        .search_items(&attrs)
        .await
        .map_err(map_oo7_retrieve_error)?;

    let Some(item) = items.into_iter().next() else {
        // No matching item is not an error, just an absent value.
        return Ok(None);
    };

    let secret = item.secret().await.map_err(map_oo7_retrieve_error)?;

    // Values were written as UTF-8 text; decode them back the same way.
    let value = String::from_utf8(secret.as_bytes().to_vec()).map_err(|e| {
        SecretError::RetrieveFailed(format!("stored secret was not valid UTF-8: {e}"))
    })?;

    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}

/// Deletes every keyring item matching a key via oo7.
///
/// # Errors
/// Returns `SecretError::BackendUnavailable` if no Secret Service answers.
/// Returns `SecretError::RetrieveFailed` if the search fails, or
/// `SecretError::DeleteFailed` if an item cannot be removed.
#[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
pub async fn clear(key: &str) -> SecretResult<()> {
    let attrs = build_attributes(key);

    let service = oo7::dbus::Service::new()
        .await
        .map_err(map_oo7_service_error)?;
    let collection = service
        .default_collection()
        .await
        .map_err(map_oo7_service_error)?;

    let items = collection
        .search_items(&attrs)
        .await
        .map_err(map_oo7_retrieve_error)?;

    for item in items {
        item.delete(None).await.map_err(map_oo7_delete_error)?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// macOS: retained secret-tool subprocess path (compile compatibility only;
// removed by task 4.5). Never selected at runtime — macOS uses the Keychain.
// ---------------------------------------------------------------------------

/// Checks whether `secret-tool` binary is available on the system.
///
/// All keyring operations depend on this tool. If it is missing,
/// callers should fall back to encrypted-settings storage and
/// inform the user to install `libsecret-tools`.
#[cfg(not(feature = "system-keyring"))]
#[expect(
    clippy::unused_async,
    reason = "keeps the public async API identical to the feature-enabled keyring implementation"
)]
pub async fn is_secret_tool_available() -> bool {
    false
}

/// Stores a value in the system keyring.
///
/// # Errors
/// Always returns `SecretError::BackendUnavailable` when `system-keyring` is disabled.
#[cfg(not(feature = "system-keyring"))]
#[expect(
    clippy::unused_async,
    reason = "keeps the public async API identical to the feature-enabled keyring implementation"
)]
pub async fn store(_key: &str, _value: &str, _label: &str) -> SecretResult<()> {
    Err(SecretError::BackendUnavailable(
        "system keyring support is not compiled in; enable the \
         rustconn-core/system-keyring feature"
            .to_string(),
    ))
}

/// Retrieves a value from the system keyring.
///
/// # Errors
/// Always returns `SecretError::BackendUnavailable` when `system-keyring` is disabled.
#[cfg(not(feature = "system-keyring"))]
#[expect(
    clippy::unused_async,
    reason = "keeps the public async API identical to the feature-enabled keyring implementation"
)]
pub async fn lookup(_key: &str) -> SecretResult<Option<String>> {
    Err(SecretError::BackendUnavailable(
        "system keyring support is not compiled in; enable the \
         rustconn-core/system-keyring feature"
            .to_string(),
    ))
}

/// Deletes a value from the system keyring.
///
/// # Errors
/// Always returns `SecretError::BackendUnavailable` when `system-keyring` is disabled.
#[cfg(not(feature = "system-keyring"))]
#[expect(
    clippy::unused_async,
    reason = "keeps the public async API identical to the feature-enabled keyring implementation"
)]
pub async fn clear(_key: &str) -> SecretResult<()> {
    Err(SecretError::BackendUnavailable(
        "system keyring support is not compiled in; enable the \
         rustconn-core/system-keyring feature"
            .to_string(),
    ))
}

#[cfg(all(feature = "system-keyring", target_os = "macos"))]
pub async fn is_secret_tool_available() -> bool {
    // secret-tool does not support --version; running it without valid
    // arguments prints usage to stderr and exits with code 1.
    // Use `which` / `command -v` to check binary presence instead.
    Command::new("which")
        .env("PATH", crate::cli_download::get_extended_path())
        .arg("secret-tool")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Stores a value in the system keyring.
///
/// # Errors
/// Returns `SecretError::BackendUnavailable` if `secret-tool` is not installed.
/// Returns `SecretError::LibSecret` if `secret-tool` cannot be spawned
/// or the store command fails.
#[cfg(all(feature = "system-keyring", target_os = "macos"))]
pub async fn store(key: &str, value: &str, label: &str) -> SecretResult<()> {
    use tokio::io::AsyncWriteExt;

    if !is_secret_tool_available().await {
        return Err(SecretError::BackendUnavailable(
            "secret-tool not found. Install libsecret-tools or use \
             encrypted settings storage."
                .into(),
        ));
    }

    let mut child = Command::new("secret-tool")
        .env("PATH", crate::cli_download::get_extended_path())
        .args(["store", "--label", label, "application", APP_ID, "key", key])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| SecretError::LibSecret(format!("Failed to spawn secret-tool: {e}")))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(value.as_bytes())
            .await
            .map_err(|e| SecretError::LibSecret(format!("Failed to write secret: {e}")))?;
    }

    let output = child
        .wait_with_output()
        .await
        .map_err(|e| SecretError::LibSecret(format!("Failed to wait for secret-tool: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SecretError::StoreFailed(format!(
            "secret-tool store failed: {stderr}"
        )));
    }

    Ok(())
}

/// Retrieves a value from the system keyring.
///
/// Returns `Ok(None)` when the key does not exist.
///
/// # Errors
/// Returns `SecretError::LibSecret` if `secret-tool` cannot be spawned.
#[cfg(all(feature = "system-keyring", target_os = "macos"))]
pub async fn lookup(key: &str) -> SecretResult<Option<String>> {
    let output = Command::new("secret-tool")
        .env("PATH", crate::cli_download::get_extended_path())
        .args(["lookup", "application", APP_ID, "key", key])
        .output()
        .await
        .map_err(|e| SecretError::LibSecret(format!("Failed to run secret-tool: {e}")))?;

    if !output.status.success() {
        return Ok(None);
    }

    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}

/// Deletes a value from the system keyring.
///
/// # Errors
/// Returns `SecretError::DeleteFailed` if the clear command fails.
#[cfg(all(feature = "system-keyring", target_os = "macos"))]
pub async fn clear(key: &str) -> SecretResult<()> {
    let output = Command::new("secret-tool")
        .env("PATH", crate::cli_download::get_extended_path())
        .args(["clear", "application", APP_ID, "key", key])
        .output()
        .await
        .map_err(|e| SecretError::LibSecret(format!("Failed to run secret-tool: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SecretError::DeleteFailed(format!(
            "secret-tool clear failed: {stderr}"
        )));
    }

    Ok(())
}
