//! Async utilities for GUI code
//!
//! This module provides helpers for running async code from GTK callbacks
//! and other synchronous GUI contexts.
//!
//! # Architecture
//!
//! GTK4 runs on a single-threaded main loop. Async operations need special
//! handling to avoid blocking the UI:
//!
//! 1. **`spawn_async`** - Spawns async work on the GLib main context (preferred)
//! 2. **`with_runtime`** - Thread-local tokio runtime for code that already runs
//!    on a worker thread (never call it from the GTK main thread)
//!
//! | Scenario | Approach | Reason |
//! |----------|----------|--------|
//! | Short async ops (<100ms) | `spawn_async` | Non-blocking, UI stays responsive |
//! | Long async ops | `spawn_async` + loading indicator | User feedback |
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

use std::cell::RefCell;
use std::future::Future;

use gtk4::glib;

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

/// Gets or creates the thread-local tokio runtime and executes a closure with it.
///
/// **Warning**: this blocks the current thread. Call it only from worker
/// threads, never from the GTK main thread.
///
/// # Arguments
/// * `f` - Closure that receives a reference to the tokio runtime
///
/// # Returns
/// * `Ok(R)` - The result of the closure
/// * `Err(String)` - If the runtime couldn't be created
///
/// # Example
/// ```ignore
/// let result = with_runtime(|rt| {
///     rt.block_on(async {
///         secret_manager.store(&id, &creds).await
///     })
/// })?;
/// ```
pub fn with_runtime<F, R>(f: F) -> Result<R, String>
where
    F: FnOnce(&tokio::runtime::Runtime) -> R,
{
    TOKIO_RUNTIME.with(|rt| {
        let mut rt_ref = rt.borrow_mut();
        if rt_ref.is_none() {
            *rt_ref = Some(
                tokio::runtime::Runtime::new()
                    .map_err(|e| format!("Failed to create runtime: {e}"))?,
            );
        }
        rt_ref
            .as_ref()
            .map(f)
            .ok_or_else(|| "thread-local Tokio runtime unavailable".to_string())
    })
}

#[cfg(test)]
mod tests {
    // Note: These tests require a GTK main context, so they're limited
    // Module compilation is verified by the build process itself
}
