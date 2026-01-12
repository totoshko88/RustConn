//! Utility functions for the RustConn GUI
//!
//! This module provides common utility functions used across the application,
//! including safe display access, CSS provider management, and accessibility helpers.

use gtk4::gdk;
use std::sync::LazyLock;

/// Gets the default GDK display, returning None if unavailable
///
/// This is safer than using `gdk::Display::default().expect(...)` which
/// would panic in headless environments or during testing.
///
/// # Returns
///
/// `Some(Display)` if a display is available, `None` otherwise.
#[must_use]
pub fn get_display() -> Option<gdk::Display> {
    gdk::Display::default()
}

/// Adds a CSS provider to the default display if available
///
/// This is a safe wrapper around `style_context_add_provider_for_display`
/// that gracefully handles the case where no display is available.
///
/// # Arguments
///
/// * `provider` - The CSS provider to add
/// * `priority` - The priority for the provider
///
/// # Returns
///
/// `true` if the provider was added, `false` if no display was available.
pub fn add_css_provider(provider: &gtk4::CssProvider, priority: u32) -> bool {
    if let Some(display) = get_display() {
        gtk4::style_context_add_provider_for_display(&display, provider, priority);
        true
    } else {
        tracing::warn!("No display available, CSS provider not added");
        false
    }
}

/// Removes a CSS provider from the default display if available
///
/// # Arguments
///
/// * `provider` - The CSS provider to remove
///
/// # Returns
///
/// `true` if the provider was removed, `false` if no display was available.
pub fn remove_css_provider(provider: &gtk4::CssProvider) -> bool {
    if let Some(display) = get_display() {
        gtk4::style_context_remove_provider_for_display(&display, provider);
        true
    } else {
        false
    }
}

/// Sets accessible properties on a widget
///
/// Helper function to set common accessibility properties in a consistent way.
///
/// # Arguments
///
/// * `widget` - The widget to update
/// * `label` - The accessible label (read by screen readers)
/// * `description` - Optional description providing more context
pub fn set_accessible_properties(
    widget: &impl gtk4::prelude::AccessibleExtManual,
    label: &str,
    description: Option<&str>,
) {
    let mut properties = vec![gtk4::accessible::Property::Label(label)];

    if let Some(desc) = description {
        properties.push(gtk4::accessible::Property::Description(desc));
    }

    widget.update_property(&properties);
}

/// Sets an accessible label on a widget
///
/// Shorthand for setting just the accessible label.
///
/// # Arguments
///
/// * `widget` - The widget to update
/// * `label` - The accessible label
pub fn set_accessible_label(widget: &impl gtk4::prelude::AccessibleExtManual, label: &str) {
    widget.update_property(&[gtk4::accessible::Property::Label(label)]);
}

/// Regex pattern for extracting variable names from templates
///
/// Matches patterns like `${variable_name}` and captures the variable name.
pub static VARIABLE_PATTERN: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"\$\{([a-zA-Z_][a-zA-Z0-9_]*)\}").expect("Variable pattern regex is valid")
});

/// Extracts variable names from a template string
///
/// Finds all `${variable_name}` patterns and returns the variable names.
///
/// # Arguments
///
/// * `template` - The template string to search
///
/// # Returns
///
/// A vector of unique variable names found in the template.
#[must_use]
pub fn extract_variables(template: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();

    for cap in VARIABLE_PATTERN.captures_iter(template) {
        if let Some(var_match) = cap.get(1) {
            let var_name = var_match.as_str().to_string();
            if !found.contains(&var_name) {
                found.push(var_name);
            }
        }
    }

    found
}

/// Truncates a string to a maximum length, adding ellipsis if needed
///
/// # Arguments
///
/// * `s` - The string to truncate
/// * `max_len` - Maximum length (including ellipsis)
///
/// # Returns
///
/// The truncated string with "…" appended if it was shortened.
#[must_use]
pub fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else if max_len <= 1 {
        "…".to_string()
    } else {
        let truncated: String = s.chars().take(max_len - 1).collect();
        format!("{truncated}…")
    }
}

/// Formats a duration in a human-readable way
///
/// # Arguments
///
/// * `seconds` - Duration in seconds
///
/// # Returns
///
/// A human-readable string like "2h 30m" or "45s".
#[must_use]
pub fn format_duration(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        let minutes = seconds / 60;
        let secs = seconds % 60;
        if secs == 0 {
            format!("{minutes}m")
        } else {
            format!("{minutes}m {secs}s")
        }
    } else {
        let hours = seconds / 3600;
        let minutes = (seconds % 3600) / 60;
        if minutes == 0 {
            format!("{hours}h")
        } else {
            format!("{hours}h {minutes}m")
        }
    }
}

/// Formats a byte count in a human-readable way
///
/// # Arguments
///
/// * `bytes` - Number of bytes
///
/// # Returns
///
/// A human-readable string like "1.5 MB" or "256 KB".
#[must_use]
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes < KB {
        format!("{bytes} B")
    } else if bytes < MB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else if bytes < GB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    }
}

/// Spawns a blocking operation in a background thread and calls a callback on completion
/// in the GTK main thread.
///
/// This is the standard pattern for running blocking operations (like KeePass access,
/// file I/O, or network requests) without freezing the GTK UI.
///
/// # Type Parameters
///
/// * `T` - The result type from the operation (must be `Send + 'static`)
/// * `F` - The blocking operation to run in the background thread
/// * `C` - The callback to invoke on the GTK main thread with the result
///
/// # Arguments
///
/// * `operation` - A closure that performs the blocking work
/// * `callback` - A closure that handles the result in the GTK main thread
///
/// # Example
///
/// ```ignore
/// spawn_blocking_with_callback(
///     move || {
///         // Blocking operation (runs in background thread)
///         std::thread::sleep(std::time::Duration::from_secs(1));
///         Ok("Done".to_string())
///     },
///     move |result: Result<String, String>| {
///         // Handle result (runs in GTK main thread)
///         match result {
///             Ok(msg) => println!("Success: {}", msg),
///             Err(e) => eprintln!("Error: {}", e),
///         }
///     },
/// );
/// ```
pub fn spawn_blocking_with_callback<T, F, C>(operation: F, callback: C)
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
    C: FnOnce(T) + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        let result = operation();
        let _ = tx.send(result);
    });

    // Use a recursive polling function to check for results
    poll_for_result(rx, callback);
}

/// Internal helper to poll for results from a background thread
fn poll_for_result<T, C>(rx: std::sync::mpsc::Receiver<T>, callback: C)
where
    T: Send + 'static,
    C: FnOnce(T) + 'static,
{
    gtk4::glib::idle_add_local_once(move || match rx.try_recv() {
        Ok(result) => {
            callback(result);
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => {
            // Not ready yet, schedule another check
            poll_for_result(rx, callback);
        }
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            // Thread panicked or was dropped - log and don't call callback
            tracing::error!("Background thread disconnected before sending result");
        }
    });
}

/// Spawns a blocking operation with a timeout
///
/// Similar to `spawn_blocking_with_callback` but includes a timeout. If the operation
/// doesn't complete within the timeout, the callback receives `None`.
///
/// # Arguments
///
/// * `operation` - A closure that performs the blocking work
/// * `timeout` - Maximum time to wait for the operation
/// * `callback` - A closure that handles the result (None if timed out)
pub fn spawn_blocking_with_timeout<T, F, C>(operation: F, timeout: std::time::Duration, callback: C)
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
    C: FnOnce(Option<T>) + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    let start = std::time::Instant::now();

    std::thread::spawn(move || {
        let result = operation();
        let _ = tx.send(result);
    });

    poll_for_result_with_timeout(rx, callback, start, timeout);
}

/// Internal helper to poll for results with timeout
fn poll_for_result_with_timeout<T, C>(
    rx: std::sync::mpsc::Receiver<T>,
    callback: C,
    start: std::time::Instant,
    timeout: std::time::Duration,
) where
    T: Send + 'static,
    C: FnOnce(Option<T>) + 'static,
{
    gtk4::glib::idle_add_local_once(move || {
        if start.elapsed() > timeout {
            tracing::warn!("Background operation timed out after {:?}", timeout);
            callback(None);
            return;
        }

        match rx.try_recv() {
            Ok(result) => {
                callback(Some(result));
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                // Not ready yet, schedule another check
                poll_for_result_with_timeout(rx, callback, start, timeout);
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                tracing::error!("Background thread disconnected before sending result");
                callback(None);
            }
        }
    });
}

// ============================================================================
// Numeric Conversion Utilities
// ============================================================================
//
// These functions provide safe numeric conversions for coordinate handling
// in embedded protocol viewers (VNC, RDP, SPICE). They handle the common
// pattern of converting GTK widget coordinates (f64) to protocol coordinates
// (u16/i32) with proper clamping and rounding.

/// Converts a floating-point coordinate to u16 with clamping
///
/// Used for VNC and RDP protocol coordinates which use u16.
/// Clamps negative values to 0 and values exceeding u16::MAX to u16::MAX.
///
/// # Arguments
///
/// * `value` - The floating-point coordinate value
///
/// # Returns
///
/// The coordinate as u16, safely clamped to valid range.
///
/// # Safety Justification
///
/// The `cast_possible_truncation` and `cast_sign_loss` warnings are suppressed
/// because we explicitly clamp the value to the valid u16 range before casting.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn coord_to_u16(value: f64) -> u16 {
    value.clamp(0.0, f64::from(u16::MAX)).round() as u16
}

/// Converts a floating-point coordinate to i32 with clamping
///
/// Used for FreeRDP protocol coordinates which use i32.
/// Clamps values to the valid i32 range.
///
/// # Arguments
///
/// * `value` - The floating-point coordinate value
///
/// # Returns
///
/// The coordinate as i32, safely clamped to valid range.
///
/// # Safety Justification
///
/// The `cast_possible_truncation` warning is suppressed because we explicitly
/// clamp the value to the valid i32 range before casting.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn coord_to_i32(value: f64) -> i32 {
    value
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX))
        .round() as i32
}

/// Converts a u32 dimension to u16 with clamping
///
/// Used when converting widget dimensions to protocol dimensions.
/// Clamps values exceeding u16::MAX to u16::MAX.
///
/// # Arguments
///
/// * `value` - The u32 dimension value
///
/// # Returns
///
/// The dimension as u16, safely clamped to valid range.
///
/// # Safety Justification
///
/// The `cast_possible_truncation` warning is suppressed because we explicitly
/// clamp the value to u16::MAX before casting.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn dimension_to_u16(value: u32) -> u16 {
    value.min(u32::from(u16::MAX)) as u16
}

/// Converts a u32 dimension to i32 with clamping
///
/// Used for Cairo/GTK APIs that expect i32 dimensions.
/// Clamps values exceeding i32::MAX to i32::MAX.
///
/// # Arguments
///
/// * `value` - The u32 dimension value
///
/// # Returns
///
/// The dimension as i32, safely clamped to valid range.
///
/// # Safety Justification
///
/// The `cast_possible_truncation` and `cast_sign_loss` warnings are suppressed
/// because we explicitly clamp the value to i32::MAX before casting.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn dimension_to_i32(value: u32) -> i32 {
    #[allow(clippy::cast_possible_wrap)]
    let result = value.min(i32::MAX as u32) as i32;
    result
}

/// Converts a u32 stride to i32 for Cairo
///
/// Cairo's `ImageSurface::create_for_data` requires stride as i32.
/// Clamps values exceeding i32::MAX to i32::MAX.
///
/// # Arguments
///
/// * `stride` - The stride value in bytes
///
/// # Returns
///
/// The stride as i32, safely clamped to valid range.
///
/// # Safety Justification
///
/// The `cast_possible_truncation` warning is suppressed because we explicitly
/// clamp the value to i32::MAX before casting.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn stride_to_i32(stride: u32) -> i32 {
    #[allow(clippy::cast_possible_wrap)]
    let result = stride.min(i32::MAX as u32) as i32;
    result
}

/// Calculates the absolute difference between two dimensions
///
/// Used for resize threshold calculations where we need to compare
/// current and target dimensions regardless of which is larger.
///
/// # Arguments
///
/// * `a` - First dimension
/// * `b` - Second dimension
///
/// # Returns
///
/// The absolute difference as u32.
#[must_use]
pub const fn dimension_diff(a: u32, b: u32) -> u32 {
    a.abs_diff(b)
}

/// Converts a progress ratio to percentage (0-100)
///
/// Used for progress bar calculations.
///
/// # Arguments
///
/// * `current` - Current progress value
/// * `total` - Total value (must be > 0)
///
/// # Returns
///
/// Percentage as f64 clamped to 0.0-100.0 range.
///
/// # Safety Justification
///
/// The `cast_precision_loss` warning is suppressed because precision loss
/// is acceptable for progress display purposes.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn progress_percentage(current: u64, total: u64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    ((current as f64 / total as f64) * 100.0).clamp(0.0, 100.0)
}

/// Converts a progress ratio to a 0.0-1.0 fraction
///
/// Used for GTK progress bars which expect a fraction.
///
/// # Arguments
///
/// * `current` - Current progress value
/// * `total` - Total value (must be > 0)
///
/// # Returns
///
/// Fraction as f64 clamped to 0.0-1.0 range.
///
/// # Safety Justification
///
/// The `cast_precision_loss` warning is suppressed because precision loss
/// is acceptable for progress display purposes.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn progress_fraction(current: u64, total: u64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    (current as f64 / total as f64).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coord_to_u16() {
        assert_eq!(coord_to_u16(0.0), 0);
        assert_eq!(coord_to_u16(100.5), 101); // rounds
        assert_eq!(coord_to_u16(-10.0), 0); // clamps negative
        assert_eq!(coord_to_u16(70000.0), u16::MAX); // clamps overflow
        assert_eq!(coord_to_u16(f64::from(u16::MAX)), u16::MAX);
    }

    #[test]
    fn test_coord_to_i32() {
        assert_eq!(coord_to_i32(0.0), 0);
        assert_eq!(coord_to_i32(100.5), 101);
        assert_eq!(coord_to_i32(-100.5), -101);
        assert_eq!(coord_to_i32(f64::from(i32::MAX)), i32::MAX);
        assert_eq!(coord_to_i32(f64::from(i32::MIN)), i32::MIN);
    }

    #[test]
    fn test_dimension_to_u16() {
        assert_eq!(dimension_to_u16(0), 0);
        assert_eq!(dimension_to_u16(1920), 1920);
        assert_eq!(dimension_to_u16(u32::from(u16::MAX)), u16::MAX);
        assert_eq!(dimension_to_u16(100_000), u16::MAX);
    }

    #[test]
    fn test_dimension_to_i32() {
        assert_eq!(dimension_to_i32(0), 0);
        assert_eq!(dimension_to_i32(1920), 1920);
        assert_eq!(dimension_to_i32(u32::MAX), i32::MAX);
    }

    #[test]
    fn test_stride_to_i32() {
        assert_eq!(stride_to_i32(0), 0);
        assert_eq!(stride_to_i32(7680), 7680); // 1920 * 4 bytes
        assert_eq!(stride_to_i32(u32::MAX), i32::MAX);
    }

    #[test]
    fn test_dimension_diff() {
        assert_eq!(dimension_diff(100, 50), 50);
        assert_eq!(dimension_diff(50, 100), 50);
        assert_eq!(dimension_diff(100, 100), 0);
    }

    #[test]
    fn test_progress_percentage() {
        assert!((progress_percentage(50, 100) - 50.0).abs() < f64::EPSILON);
        assert!((progress_percentage(0, 100) - 0.0).abs() < f64::EPSILON);
        assert!((progress_percentage(100, 100) - 100.0).abs() < f64::EPSILON);
        assert!((progress_percentage(0, 0) - 0.0).abs() < f64::EPSILON); // edge case
    }

    #[test]
    fn test_progress_fraction() {
        assert!((progress_fraction(50, 100) - 0.5).abs() < f64::EPSILON);
        assert!((progress_fraction(0, 100) - 0.0).abs() < f64::EPSILON);
        assert!((progress_fraction(100, 100) - 1.0).abs() < f64::EPSILON);
        assert!((progress_fraction(0, 0) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_extract_variables() {
        let template = "Hello ${name}, your ID is ${user_id}";
        let vars = extract_variables(template);
        assert_eq!(vars, vec!["name", "user_id"]);
    }

    #[test]
    fn test_extract_variables_duplicates() {
        let template = "${var} and ${var} again";
        let vars = extract_variables(template);
        assert_eq!(vars, vec!["var"]);
    }

    #[test]
    fn test_extract_variables_empty() {
        let template = "No variables here";
        let vars = extract_variables(template);
        assert!(vars.is_empty());
    }

    #[test]
    fn test_truncate_string() {
        assert_eq!(truncate_string("hello", 10), "hello");
        assert_eq!(truncate_string("hello world", 8), "hello w…");
        assert_eq!(truncate_string("hi", 2), "hi");
        assert_eq!(truncate_string("hi", 1), "…");
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(30), "30s");
        assert_eq!(format_duration(60), "1m");
        assert_eq!(format_duration(90), "1m 30s");
        assert_eq!(format_duration(3600), "1h");
        assert_eq!(format_duration(3660), "1h 1m");
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(1_048_576), "1.0 MB");
    }
}
