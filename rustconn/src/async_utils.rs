//! Async utilities for GUI code
//!
//! This module provides helpers for running async code from GTK callbacks
//! and other synchronous GUI contexts.
//!
//! # Architecture
//!
//! GTK4 runs on a single-threaded main loop. Async operations need special
//! handling to avoid blocking the UI. This module provides two approaches:
//!
//! 1. **`spawn_async`** - Spawns async work on the GLib main context (preferred)
//! 2. **`block_on_async`** - Blocks on async work using a thread-local runtime
//!
//! ## When to use each approach
//!
//! | Scenario | Approach | Reason |
//! |----------|----------|--------|
//! | Short async ops (<100ms) | `spawn_async` | Non-blocking, UI stays responsive |
//! | Long async ops | `spawn_async` + loading indicator | User feedback |
//! | Must have result immediately | `block_on_async` | Synchronous return value needed |
//! | In GTK callback | `spawn_async` | Callbacks should return quickly |
//!
//! # Examples
//!
//! ## Non-blocking async (preferred)
//!
//! ```ignore
//! use crate::async_utils::spawn_async;
//!
//! // In a button click handler
//! button.connect_clicked(move |_| {
//!     let state = state.clone();
//!     spawn_async(async move {
//!         let result = some_async_operation().await;
//!         // Update UI with result (runs on main thread)
//!         glib::idle_add_local_once(move || {
//!             update_ui(result);
//!         });
//!     });
//! });
//! ```
//!
//! ## Blocking async (when result is needed immediately)
//!
//! ```ignore
//! use crate::async_utils::block_on_async;
//!
//! let result = block_on_async(async {
//!     secret_manager.get_password(&id).await
//! });
//! ```

use gtk4::glib;
use std::cell::RefCell;
use std::future::Future;

// Thread-local tokio runtime for blocking async operations
thread_local! {
    static TOKIO_RUNTIME: RefCell<Option<tokio::runtime::Runtime>> = const { RefCell::new(None) };
}

/// Spawns an async task on the GLib main context.
///
/// This is the preferred way to run async code from GTK callbacks.
/// The task runs on the main thread and can safely update GTK widgets.
///
/// # Arguments
/// * `future` - The async task to run
///
/// # Example
/// ```ignore
/// spawn_async(async move {
///     let data = fetch_data().await;
///     // Safe to update UI here
///     label.set_text(&data);
/// });
/// ```
pub fn spawn_async<F>(future: F)
where
    F: Future<Output = ()> + 'static,
{
    let ctx = glib::MainContext::default();
    ctx.spawn_local(future);
}

/// Spawns an async task and calls a callback with the result.
///
/// This is useful when you need to process the result of an async operation.
///
/// # Arguments
/// * `future` - The async task to run
/// * `callback` - Called with the result on the main thread
///
/// # Example
/// ```ignore
/// spawn_async_with_callback(
///     async { fetch_connections().await },
///     |connections| {
///         update_sidebar(connections);
///     }
/// );
/// ```
pub fn spawn_async_with_callback<F, T, C>(future: F, callback: C)
where
    F: Future<Output = T> + 'static,
    T: 'static,
    C: FnOnce(T) + 'static,
{
    let ctx = glib::MainContext::default();
    ctx.spawn_local(async move {
        let result = future.await;
        callback(result);
    });
}

/// Blocks on an async operation using a thread-local tokio runtime.
///
/// **Warning**: This blocks the current thread. Use `spawn_async` instead
/// when possible to keep the UI responsive.
///
/// This is useful when you need the result immediately and can't restructure
/// the code to be async-friendly.
///
/// # Arguments
/// * `future` - The async task to run
///
/// # Returns
/// * `Ok(T)` - The result of the async operation
/// * `Err(String)` - If the runtime couldn't be created
///
/// # Example
/// ```ignore
/// let password = block_on_async(async {
///     secret_manager.get_password(&id).await
/// })?;
/// ```
pub fn block_on_async<F, T>(future: F) -> Result<T, String>
where
    F: Future<Output = T>,
{
    TOKIO_RUNTIME.with(|rt| {
        let mut rt_ref = rt.borrow_mut();
        if rt_ref.is_none() {
            *rt_ref = Some(
                tokio::runtime::Runtime::new()
                    .map_err(|e| format!("Failed to create runtime: {e}"))?,
            );
        }
        Ok(rt_ref
            .as_ref()
            .expect("runtime should exist")
            .block_on(future))
    })
}

/// Runs an async operation with a timeout.
///
/// # Arguments
/// * `future` - The async task to run
/// * `timeout` - Maximum time to wait
///
/// # Returns
/// * `Ok(Some(T))` - The result if completed in time
/// * `Ok(None)` - If the operation timed out
/// * `Err(String)` - If the runtime couldn't be created
pub fn block_on_async_with_timeout<F, T>(
    future: F,
    timeout: std::time::Duration,
) -> Result<Option<T>, String>
where
    F: Future<Output = T>,
{
    block_on_async(async move { tokio::time::timeout(timeout, future).await.ok() })
}

/// Checks if we're on the GTK main thread.
///
/// Useful for assertions in code that must run on the main thread.
#[must_use]
pub fn is_main_thread() -> bool {
    glib::MainContext::default().is_owner()
}

/// Ensures code runs on the main thread.
///
/// If already on the main thread, runs immediately.
/// Otherwise, schedules to run on the main thread.
///
/// # Arguments
/// * `f` - The function to run on the main thread
pub fn ensure_main_thread<F>(f: F)
where
    F: FnOnce() + Send + 'static,
{
    if is_main_thread() {
        f();
    } else {
        glib::idle_add_once(f);
    }
}

#[cfg(test)]
mod tests {
    // Note: These tests require a GTK main context, so they're limited
    // Module compilation is verified by the build process itself
}
