//! Async credential resolution for non-blocking UI operations
//!
//! This module provides async credential resolution that doesn't block the UI thread.
//! It includes support for cancellation tokens and callback-based resolution.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::oneshot;
use tracing::{debug, instrument, warn};

use crate::config::SecretSettings;
use crate::models::{Connection, Credentials};

use super::manager::SecretManager;
use super::resolver::CredentialResolver;

/// Token for cancelling pending credential resolution requests
///
/// This token can be cloned and shared across threads. When `cancel()` is called,
/// all pending operations using this token will be cancelled.
#[derive(Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Creates a new cancellation token
    #[must_use]
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Cancels all operations using this token
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Checks if the token has been cancelled
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Resets the cancellation state
    pub fn reset(&self) {
        self.cancelled.store(false, Ordering::SeqCst);
    }
}

impl std::fmt::Debug for CancellationToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

/// Result of an async credential resolution operation
#[derive(Debug)]
pub enum AsyncCredentialResult {
    /// Credentials were successfully resolved
    Success(Option<Credentials>),
    /// The operation was cancelled
    Cancelled,
    /// An error occurred during resolution
    Error(String),
    /// The operation timed out
    Timeout,
}

impl AsyncCredentialResult {
    /// Returns true if the operation was successful
    #[must_use]
    pub const fn is_success(&self) -> bool {
        matches!(self, Self::Success(_))
    }

    /// Returns true if the operation was cancelled
    #[must_use]
    pub const fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }

    /// Returns true if the operation resulted in an error
    #[must_use]
    pub const fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }

    /// Returns true if the operation timed out
    #[must_use]
    pub const fn is_timeout(&self) -> bool {
        matches!(self, Self::Timeout)
    }

    /// Converts to Option<Credentials> if successful
    #[must_use]
    pub fn into_credentials(self) -> Option<Credentials> {
        match self {
            Self::Success(creds) => creds,
            _ => None,
        }
    }

    /// Returns the error message if this is an error result
    #[must_use]
    pub fn error_message(&self) -> Option<&str> {
        match self {
            Self::Error(msg) => Some(msg),
            _ => None,
        }
    }
}

/// Async credential resolver that doesn't block the calling thread
///
/// This resolver wraps the synchronous `CredentialResolver` and provides
/// async methods that can be used with callbacks or awaited directly.
pub struct AsyncCredentialResolver {
    resolver: CredentialResolver,
}

impl AsyncCredentialResolver {
    /// Creates a new async credential resolver
    ///
    /// # Arguments
    /// * `secret_manager` - The secret manager with configured backends
    /// * `settings` - Secret settings for configuration
    #[must_use]
    pub const fn new(secret_manager: Arc<SecretManager>, settings: SecretSettings) -> Self {
        Self {
            resolver: CredentialResolver::new(secret_manager, settings),
        }
    }

    /// Resolves credentials asynchronously
    ///
    /// This method resolves credentials without blocking the calling thread.
    ///
    /// # Arguments
    /// * `connection` - The connection to resolve credentials for
    ///
    /// # Returns
    /// `AsyncCredentialResult` indicating success, cancellation, or error
    #[instrument(skip(self, connection), fields(connection_id = %connection.id, async_mode = true))]
    pub async fn resolve_async(&self, connection: &Connection) -> AsyncCredentialResult {
        debug!(
            connection_name = %connection.name,
            "Starting async credential resolution"
        );

        match self.resolver.resolve(connection).await {
            Ok(creds) => {
                debug!("Async credential resolution completed successfully");
                AsyncCredentialResult::Success(creds)
            }
            Err(e) => {
                warn!(error = %e, "Async credential resolution failed");
                AsyncCredentialResult::Error(e.to_string())
            }
        }
    }

    /// Resolves credentials with cancellation support
    ///
    /// This method resolves credentials and can be cancelled using the provided token.
    ///
    /// # Arguments
    /// * `connection` - The connection to resolve credentials for
    /// * `cancel_token` - Token to cancel the operation
    ///
    /// # Returns
    /// `AsyncCredentialResult` indicating success, cancellation, or error
    #[instrument(skip(self, connection, cancel_token), fields(connection_id = %connection.id, cancellable = true))]
    pub async fn resolve_with_cancellation(
        &self,
        connection: &Connection,
        cancel_token: &CancellationToken,
    ) -> AsyncCredentialResult {
        // Check if already cancelled before starting
        if cancel_token.is_cancelled() {
            return AsyncCredentialResult::Cancelled;
        }

        // Use tokio::select! to race between resolution and cancellation check
        tokio::select! {
            result = self.resolver.resolve(connection) => {
                // Check cancellation after resolution completes
                if cancel_token.is_cancelled() {
                    return AsyncCredentialResult::Cancelled;
                }

                match result {
                    Ok(creds) => AsyncCredentialResult::Success(creds),
                    Err(e) => AsyncCredentialResult::Error(e.to_string()),
                }
            }
            () = async {
                // Poll cancellation token periodically
                loop {
                    if cancel_token.is_cancelled() {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            } => {
                AsyncCredentialResult::Cancelled
            }
        }
    }

    /// Resolves credentials with a timeout
    ///
    /// # Arguments
    /// * `connection` - The connection to resolve credentials for
    /// * `timeout` - Maximum time to wait for resolution
    ///
    /// # Returns
    /// `AsyncCredentialResult` indicating success, timeout, or error
    #[allow(clippy::cast_possible_truncation)] // timeout.as_millis() won't exceed u64::MAX
    #[instrument(skip(self, connection), fields(connection_id = %connection.id, timeout_ms = timeout.as_millis() as u64))]
    pub async fn resolve_with_timeout(
        &self,
        connection: &Connection,
        timeout: Duration,
    ) -> AsyncCredentialResult {
        match tokio::time::timeout(timeout, self.resolver.resolve(connection)).await {
            Ok(Ok(creds)) => AsyncCredentialResult::Success(creds),
            Ok(Err(e)) => AsyncCredentialResult::Error(e.to_string()),
            Err(_) => {
                warn!("Credential resolution timed out");
                AsyncCredentialResult::Timeout
            }
        }
    }

    /// Resolves credentials with both cancellation and timeout support
    ///
    /// # Arguments
    /// * `connection` - The connection to resolve credentials for
    /// * `cancel_token` - Token to cancel the operation
    /// * `timeout` - Maximum time to wait for resolution
    ///
    /// # Returns
    /// `AsyncCredentialResult` indicating success, cancellation, timeout, or error
    #[allow(clippy::cast_possible_truncation)] // timeout.as_millis() won't exceed u64::MAX
    #[instrument(skip(self, connection, cancel_token), fields(connection_id = %connection.id, cancellable = true, timeout_ms = timeout.as_millis() as u64))]
    pub async fn resolve_with_cancellation_and_timeout(
        &self,
        connection: &Connection,
        cancel_token: &CancellationToken,
        timeout: Duration,
    ) -> AsyncCredentialResult {
        if cancel_token.is_cancelled() {
            return AsyncCredentialResult::Cancelled;
        }

        tokio::select! {
            result = tokio::time::timeout(timeout, self.resolver.resolve(connection)) => {
                if cancel_token.is_cancelled() {
                    return AsyncCredentialResult::Cancelled;
                }

                match result {
                    Ok(Ok(creds)) => AsyncCredentialResult::Success(creds),
                    Ok(Err(e)) => AsyncCredentialResult::Error(e.to_string()),
                    Err(_) => AsyncCredentialResult::Timeout,
                }
            }
            () = async {
                loop {
                    if cancel_token.is_cancelled() {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            } => {
                AsyncCredentialResult::Cancelled
            }
        }
    }

    /// Gets a reference to the underlying resolver
    #[must_use]
    pub const fn resolver(&self) -> &CredentialResolver {
        &self.resolver
    }
}

/// Handle for a pending credential resolution operation
///
/// This handle can be used to await the result or cancel the operation.
pub struct PendingCredentialResolution {
    /// Receiver for the result
    receiver: oneshot::Receiver<AsyncCredentialResult>,
    /// Cancellation token for this operation
    cancel_token: CancellationToken,
}

impl PendingCredentialResolution {
    /// Creates a new pending resolution with the given receiver and token
    #[must_use]
    pub const fn new(
        receiver: oneshot::Receiver<AsyncCredentialResult>,
        cancel_token: CancellationToken,
    ) -> Self {
        Self {
            receiver,
            cancel_token,
        }
    }

    /// Cancels the pending operation
    pub fn cancel(&self) {
        self.cancel_token.cancel();
    }

    /// Checks if the operation has been cancelled
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancel_token.is_cancelled()
    }

    /// Awaits the result of the operation
    ///
    /// # Returns
    /// The result of the credential resolution, or `Cancelled` if the sender was dropped
    pub async fn await_result(self) -> AsyncCredentialResult {
        self.receiver
            .await
            .unwrap_or(AsyncCredentialResult::Cancelled)
    }

    /// Gets the cancellation token for this operation
    #[must_use]
    pub const fn cancel_token(&self) -> &CancellationToken {
        &self.cancel_token
    }
}

/// Spawns an async credential resolution and returns a handle
///
/// This function spawns the resolution on a tokio runtime and returns
/// a handle that can be used to await or cancel the operation.
///
/// # Arguments
/// * `resolver` - The async resolver to use
/// * `connection` - The connection to resolve credentials for
/// * `timeout` - Optional timeout for the operation
///
/// # Returns
/// A `PendingCredentialResolution` handle
#[must_use]
pub fn spawn_credential_resolution(
    resolver: Arc<AsyncCredentialResolver>,
    connection: Connection,
    timeout: Option<Duration>,
) -> PendingCredentialResolution {
    let (sender, receiver) = oneshot::channel();
    let cancel_token = CancellationToken::new();
    let token_clone = cancel_token.clone();

    tokio::spawn(async move {
        let result = if let Some(timeout_duration) = timeout {
            resolver
                .resolve_with_cancellation_and_timeout(&connection, &token_clone, timeout_duration)
                .await
        } else {
            resolver
                .resolve_with_cancellation(&connection, &token_clone)
                .await
        };

        // Send result, ignoring error if receiver was dropped
        let _ = sender.send(result);
    });

    PendingCredentialResolution::new(receiver, cancel_token)
}

/// Resolves credentials and calls a callback with the result
///
/// This is a convenience function for callback-based async resolution.
///
/// # Arguments
/// * `resolver` - The async resolver to use
/// * `connection` - The connection to resolve credentials for
/// * `callback` - Function to call with the result
///
/// # Returns
/// A `CancellationToken` that can be used to cancel the operation
pub fn resolve_with_callback<F>(
    resolver: Arc<AsyncCredentialResolver>,
    connection: Connection,
    callback: F,
) -> CancellationToken
where
    F: FnOnce(AsyncCredentialResult) + Send + 'static,
{
    let cancel_token = CancellationToken::new();
    let token_clone = cancel_token.clone();

    tokio::spawn(async move {
        let result = resolver
            .resolve_with_cancellation(&connection, &token_clone)
            .await;
        callback(result);
    });

    cancel_token
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cancellation_token_default() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
    }

    #[test]
    fn test_cancellation_token_cancel() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());

        token.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn test_cancellation_token_reset() {
        let token = CancellationToken::new();
        token.cancel();
        assert!(token.is_cancelled());

        token.reset();
        assert!(!token.is_cancelled());
    }

    #[test]
    fn test_cancellation_token_clone() {
        let token1 = CancellationToken::new();
        let token2 = token1.clone();

        assert!(!token1.is_cancelled());
        assert!(!token2.is_cancelled());

        token1.cancel();

        assert!(token1.is_cancelled());
        assert!(token2.is_cancelled());
    }

    #[test]
    fn test_async_credential_result_success() {
        let result = AsyncCredentialResult::Success(None);
        assert!(result.is_success());
        assert!(!result.is_cancelled());
        assert!(!result.is_error());
        assert!(!result.is_timeout());
    }

    #[test]
    fn test_async_credential_result_cancelled() {
        let result = AsyncCredentialResult::Cancelled;
        assert!(!result.is_success());
        assert!(result.is_cancelled());
        assert!(!result.is_error());
        assert!(!result.is_timeout());
    }

    #[test]
    fn test_async_credential_result_error() {
        let result = AsyncCredentialResult::Error("test error".to_string());
        assert!(!result.is_success());
        assert!(!result.is_cancelled());
        assert!(result.is_error());
        assert!(!result.is_timeout());
        assert_eq!(result.error_message(), Some("test error"));
    }

    #[test]
    fn test_async_credential_result_timeout() {
        let result = AsyncCredentialResult::Timeout;
        assert!(!result.is_success());
        assert!(!result.is_cancelled());
        assert!(!result.is_error());
        assert!(result.is_timeout());
    }
}
