//! Shared keyring storage via the Secret Service API.
//!
//! Provides generic store/retrieve/delete operations for all secret backends
//! that need system keyring integration (GNOME Keyring, KDE Wallet, etc.).
//!
//! - On Linux/BSD (`cfg(not(target_os = "macos"))`) this talks to the Secret
//!   Service **in process** via the [`oo7`] crate (`oo7::dbus::Service`), so no
//!   `secret-tool` binary or bundled libsecret C library is required.
//! - On macOS these generic auxiliary operations delegate to the native
//!   Keychain implementation in `super::macos_keychain::keychain_ops`.
//!
//! Linux/BSD entries use the same two attributes — `application` and `key` —
//! and the same labels as the former `secret-tool` implementation.

// oo7 attribute maps are only built on the in-process (non-macOS) path.
#[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
use std::collections::HashMap;

use crate::error::{SecretError, SecretResult};

/// Application identifier used as the `application` attribute in keyring entries
#[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
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
// Call budget — shared by this module and `libsecret.rs`.
// ---------------------------------------------------------------------------

/// Ceiling on a single Secret Service round-trip.
///
/// oo7 builds its zbus connection with a 30 s method timeout, so an ordinary
/// method call was already bounded — but the one that matters is not an ordinary
/// method call. Reading a secret from a locked collection makes the Secret
/// Service raise its own unlock prompt and answer only once the prompt is
/// dismissed, which means waiting on a `Completed` signal with no deadline at
/// all. `rustconn-cli` and any other consumer of this crate could sit there
/// forever; the GUI's one 30 s wrapper around the whole of
/// `resolve_with_hierarchy` is a single budget shared by five lookup keys and
/// every round-trip they make, and it cannot say which step stalled.
///
/// Ten seconds is the project's standard vault budget (`secrets-guide.md`, and
/// `VAULT_RETRIEVE_TIMEOUT` in the GUI's connection dialogs): far longer than a
/// healthy round-trip (~12 ms for the DH handshake), short enough to fail while
/// the user is still watching.
#[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
pub(super) const DBUS_CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Bounds one Secret Service call, reporting `what` if it runs out of time.
///
/// Returns the future's own output on success, so the caller still maps oo7's
/// error itself; the outer `Err` is only ever the timeout.
///
/// `what` is `&'static str` on purpose. It goes into a log line and into a user-
/// visible error, and a `&str` would let a caller interpolate a lookup key — or
/// worse — into either; a literal cannot.
#[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
pub(super) async fn with_dbus_timeout<T>(
    what: &'static str,
    call: impl std::future::Future<Output = T>,
) -> SecretResult<T> {
    if let Ok(value) = tokio::time::timeout(DBUS_CALL_TIMEOUT, call).await {
        return Ok(value);
    }

    tracing::warn!(
        operation = what,
        timeout_secs = DBUS_CALL_TIMEOUT.as_secs(),
        "Secret Service call timed out — is the keyring locked and waiting on a prompt?"
    );
    Err(SecretError::ConnectionFailed(format!(
        "the Secret Service did not answer within {}s while trying to {what}",
        DBUS_CALL_TIMEOUT.as_secs()
    )))
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
    matches!(
        with_dbus_timeout("probe the Secret Service", oo7::dbus::Service::new()).await,
        Ok(Ok(_))
    )
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

    let service = with_dbus_timeout("connect to the Secret Service", oo7::dbus::Service::new())
        .await?
        .map_err(map_oo7_service_error)?;
    let collection = with_dbus_timeout("open the default collection", service.default_collection())
        .await?
        .map_err(map_oo7_service_error)?;

    // `Secret::text` stores the raw UTF-8 string with a `text/plain` content
    // type so values round-trip byte-for-byte like the old secret-tool path.
    with_dbus_timeout(
        "store a keyring value",
        collection.create_item(label, &attrs, oo7::Secret::text(value), true, None),
    )
    .await?
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

    let service = with_dbus_timeout("connect to the Secret Service", oo7::dbus::Service::new())
        .await?
        .map_err(map_oo7_service_error)?;
    let collection = with_dbus_timeout("open the default collection", service.default_collection())
        .await?
        .map_err(map_oo7_service_error)?;

    let items = with_dbus_timeout("search the keyring", collection.search_items(&attrs))
        .await?
        .map_err(map_oo7_retrieve_error)?;

    let Some(item) = items.into_iter().next() else {
        // No matching item is not an error, just an absent value.
        return Ok(None);
    };

    // The call that can wait on a human: a locked collection makes the Secret
    // Service prompt for the keyring password before it answers.
    let secret = with_dbus_timeout("read a keyring value", item.secret())
        .await?
        .map_err(map_oo7_retrieve_error)?;

    // Values were written as UTF-8 text; decode them back the same way. The
    // intermediate copy holds secret material, so it is wiped on drop and the
    // malformed-input buffer is wiped explicitly, matching the macOS path.
    let bytes = zeroize::Zeroizing::new(secret.as_bytes().to_vec());
    let value = match String::from_utf8(bytes.to_vec()) {
        Ok(value) => value,
        Err(e) => {
            drop(zeroize::Zeroizing::new(e.into_bytes()));
            return Err(SecretError::RetrieveFailed(
                "stored secret was not valid UTF-8".to_string(),
            ));
        }
    };

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

    let service = with_dbus_timeout("connect to the Secret Service", oo7::dbus::Service::new())
        .await?
        .map_err(map_oo7_service_error)?;
    let collection = with_dbus_timeout("open the default collection", service.default_collection())
        .await?
        .map_err(map_oo7_service_error)?;

    let items = with_dbus_timeout("search the keyring", collection.search_items(&attrs))
        .await?
        .map_err(map_oo7_retrieve_error)?;

    for item in items {
        with_dbus_timeout("delete a keyring value", item.delete(None))
            .await?
            .map_err(map_oo7_delete_error)?;
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

/// Maximum time callers wait for a native Keychain operation.
#[cfg(all(feature = "system-keyring", target_os = "macos"))]
const KEYCHAIN_OPERATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[cfg(all(feature = "system-keyring", target_os = "macos"))]
async fn run_keychain_operation<T, F>(operation: &'static str, task: F) -> SecretResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> SecretResult<T> + Send + 'static,
{
    let mut handle = tokio::task::spawn_blocking(task);

    match tokio::time::timeout(KEYCHAIN_OPERATION_TIMEOUT, &mut handle).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => Err(SecretError::LibSecret(format!(
            "Keychain {operation} worker failed: {error}"
        ))),
        Err(_) => {
            // A blocking Security.framework call cannot be cancelled, so the
            // timeout only bounds how long the caller waits. Keep observing the
            // task instead of dropping its handle silently: the late outcome is
            // logged, and every Keychain operation here is idempotent per key
            // (store overwrites, clear deletes), so a late completion cannot
            // contradict the failure already reported to the caller.
            tokio::spawn(async move {
                match handle.await {
                    Ok(Ok(_)) => tracing::warn!(
                        operation,
                        "Keychain operation completed after its timeout was reported"
                    ),
                    Ok(Err(error)) => tracing::warn!(
                        operation,
                        %error,
                        "Keychain operation failed after its timeout was reported"
                    ),
                    Err(error) => tracing::warn!(
                        operation,
                        %error,
                        "Keychain worker terminated after its timeout was reported"
                    ),
                }
            });

            Err(SecretError::BackendUnavailable(format!(
                "Keychain {operation} did not finish within {}s; it may still \
                 complete in the background",
                KEYCHAIN_OPERATION_TIMEOUT.as_secs()
            )))
        }
    }
}

/// Checks whether the native macOS Keychain backend is available.
///
/// The function keeps its historical backend-neutral call-site API even though
/// macOS does not use the `secret-tool` executable.
#[cfg(all(feature = "system-keyring", target_os = "macos"))]
#[expect(
    clippy::unused_async,
    reason = "keeps the public async API identical across keyring backends"
)]
pub async fn is_secret_tool_available() -> bool {
    super::macos_keychain::keychain_ops::is_keychain_available()
}

/// Stores a value in the native macOS Keychain.
///
/// The label is accepted for API compatibility; Keychain entries use the
/// application service and key as their stable identity.
///
/// # Errors
/// Returns `SecretError::StoreFailed` if the Keychain operation fails.
/// Returns `SecretError::BackendUnavailable` if the operation times out.
#[cfg(all(feature = "system-keyring", target_os = "macos"))]
pub async fn store(key: &str, value: &str, _label: &str) -> SecretResult<()> {
    let key = key.to_owned();
    let value = zeroize::Zeroizing::new(value.to_owned());
    run_keychain_operation("store", move || {
        super::macos_keychain::keychain_ops::store(&key, &value)
    })
    .await
}

/// Retrieves a value from the native macOS Keychain.
///
/// Returns `Ok(None)` when the key does not exist.
///
/// # Errors
/// Returns `SecretError::LibSecret` if the Keychain operation fails.
/// Returns `SecretError::BackendUnavailable` if the operation times out.
#[cfg(all(feature = "system-keyring", target_os = "macos"))]
pub async fn lookup(key: &str) -> SecretResult<Option<String>> {
    let key = key.to_owned();
    run_keychain_operation("lookup", move || {
        super::macos_keychain::keychain_ops::lookup(&key)
    })
    .await
}

/// Deletes a value from the native macOS Keychain.
///
/// # Errors
/// Returns `SecretError::DeleteFailed` if the Keychain operation fails.
/// Returns `SecretError::BackendUnavailable` if the operation times out.
#[cfg(all(feature = "system-keyring", target_os = "macos"))]
pub async fn clear(key: &str) -> SecretResult<()> {
    let key = key.to_owned();
    run_keychain_operation("clear", move || {
        super::macos_keychain::keychain_ops::clear(&key)
    })
    .await
}
