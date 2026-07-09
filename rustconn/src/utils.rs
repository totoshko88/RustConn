//! Utility functions for the RustConn GUI
//!
//! This module provides common utility functions used across the application,
//! including safe display access, CSS provider management, and accessibility helpers.

use gtk4::gdk;
use gtk4::prelude::DisplayExtManual;

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

/// Sets a `LabelledBy` accessibility relation from `target` to `label_widget`.
///
/// The labelling widget is upcast through [`gtk4::Widget`] instead of directly
/// to [`gtk4::Accessible`]. Every widget is an `Accessible` via `Widget`, so
/// this holds regardless of whether the concrete row type carries its own
/// `IsA<Accessible>` bound — libadwaita 0.9.2 dropped that bound on
/// `adw::ActionRow`, which would otherwise break a direct `upcast_ref`.
///
/// # Arguments
///
/// * `target` - The widget whose accessible name is provided by the label
/// * `label_widget` - The widget (row or label) that names `target`
pub fn set_labelled_by(
    target: &impl gtk4::prelude::AccessibleExtManual,
    label_widget: &impl gtk4::prelude::IsA<gtk4::Widget>,
) {
    use gtk4::prelude::Cast;
    target.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[label_widget
        .upcast_ref::<gtk4::Widget>()
        .upcast_ref()])]);
}

/// Regex pattern for extracting variable names from templates
///
/// Matches patterns like `${variable_name}` and captures the variable name.
///
/// Re-exported from [`rustconn_core::variables::manager::VARIABLE_REGEX`].
pub use rustconn_core::variables::VARIABLE_REGEX as VARIABLE_PATTERN;

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

    let callback = std::cell::Cell::new(Some(callback));
    // Poll at 16ms (~60fps) for the result from the background thread
    gtk4::glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
        match rx.try_recv() {
            Ok(result) => {
                if let Some(cb) = callback.take() {
                    cb(result);
                }
                gtk4::glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => gtk4::glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => gtk4::glib::ControlFlow::Break,
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

    let callback = std::rc::Rc::new(std::cell::RefCell::new(Some(
        Box::new(callback) as Box<dyn FnOnce(Option<T>)>
    )));

    // Poll at 16ms for result or timeout
    gtk4::glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
        // Check if callback was already consumed
        if callback.borrow().is_none() {
            return gtk4::glib::ControlFlow::Break;
        }

        // Try to receive result from background thread
        match rx.try_recv() {
            Ok(result) => {
                if let Some(cb) = callback.borrow_mut().take() {
                    cb(Some(result));
                }
                return gtk4::glib::ControlFlow::Break;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                // Thread panicked or dropped sender without sending
                if let Some(cb) = callback.borrow_mut().take() {
                    cb(None);
                }
                return gtk4::glib::ControlFlow::Break;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }

        // Check timeout
        if start.elapsed() > timeout {
            tracing::warn!("Background operation timed out after {:?}", timeout);
            if let Some(cb) = callback.borrow_mut().take() {
                cb(None);
            }
            return gtk4::glib::ControlFlow::Break;
        }

        gtk4::glib::ControlFlow::Continue
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
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "value range fits the target type and is non-negative by construction in this code path"
)]
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
#[expect(
    clippy::cast_possible_truncation,
    reason = "value range fits the target type by construction in this code path"
)]
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
#[expect(
    clippy::cast_possible_truncation,
    reason = "value range fits the target type by construction in this code path"
)]
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
pub fn dimension_to_i32(value: u32) -> i32 {
    #[expect(
        clippy::cast_possible_wrap,
        reason = "value range fits the target signed type by construction in this code path"
    )]
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
pub fn stride_to_i32(stride: u32) -> i32 {
    #[expect(
        clippy::cast_possible_wrap,
        reason = "value range fits the target signed type by construction in this code path"
    )]
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
pub fn progress_fraction(current: u64, total: u64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    (current as f64 / total as f64).clamp(0.0, 1.0)
}

/// Maps a key press to a layout-independent (Latin) keyval.
///
/// GDK reports `keyval` according to the *active* keyboard layout, so pressing
/// the physical "F" key under a Cyrillic layout yields `Cyrillic_ef`. Accelerators
/// are stored and registered with Latin keyvals (`<Control>f`), so they would
/// never match under a non-Latin layout. This translates the hardware `keycode`
/// (which is layout-independent) to an ASCII keyval.
///
/// On macOS the GTK Quartz backend does not expose alternate layout groups via
/// `map_keycode` (it returns only the active layout, and often nothing usable
/// when no Latin layout is installed), so the stable Apple virtual keycode is
/// translated directly. On X11/Wayland every installed layout group is walked
/// for an ASCII keyval.
///
/// Returns the original `keyval` when it is already ASCII or when no ASCII
/// mapping exists (e.g. function keys, which are already layout-independent).
#[must_use]
pub fn latin_keyval(keyval: gdk::Key, keycode: u32) -> gdk::Key {
    // Already an ASCII keyval (e.g. Latin layout) — nothing to translate.
    if keyval.to_unicode().is_some_and(|c| c.is_ascii()) {
        return keyval;
    }

    // macOS: translate the stable Apple virtual keycode directly, because the
    // Quartz backend's `map_keycode` does not return a Latin group.
    #[cfg(target_os = "macos")]
    if let Some(kv) = macos_keycode_to_latin(keycode) {
        return kv;
    }

    let Some(display) = gdk::Display::default() else {
        return keyval;
    };
    let Some(entries) = display.map_keycode(keycode) else {
        return keyval;
    };
    // Prefer an ASCII graphic keyval from any layout group (the Latin one),
    // covering letters, digits and punctuation used in accelerators.
    entries
        .iter()
        .map(|(_, kv)| *kv)
        .find(|kv| kv.to_unicode().is_some_and(|c| c.is_ascii_graphic()))
        .unwrap_or(keyval)
}

/// Translates a macOS Apple virtual keycode (`kVK_ANSI_*`) to its Latin GDK
/// keyval, independent of the active keyboard layout.
///
/// Apple virtual keycodes are tied to the physical key position on an ANSI
/// keyboard and never change with the layout, so they are a reliable source for
/// layout-independent accelerator matching. Only the keys that can appear in an
/// accelerator are mapped; anything else returns `None`.
#[cfg(target_os = "macos")]
fn macos_keycode_to_latin(keycode: u32) -> Option<gdk::Key> {
    let name = match keycode {
        0 => "a",
        1 => "s",
        2 => "d",
        3 => "f",
        4 => "h",
        5 => "g",
        6 => "z",
        7 => "x",
        8 => "c",
        9 => "v",
        11 => "b",
        12 => "q",
        13 => "w",
        14 => "e",
        15 => "r",
        16 => "y",
        17 => "t",
        18 => "1",
        19 => "2",
        20 => "3",
        21 => "4",
        22 => "6",
        23 => "5",
        24 => "equal",
        25 => "9",
        26 => "7",
        27 => "minus",
        28 => "8",
        29 => "0",
        30 => "bracketright",
        31 => "o",
        32 => "u",
        33 => "bracketleft",
        34 => "i",
        35 => "p",
        37 => "l",
        38 => "j",
        39 => "apostrophe",
        40 => "k",
        41 => "semicolon",
        42 => "backslash",
        43 => "comma",
        44 => "slash",
        45 => "n",
        46 => "m",
        47 => "period",
        50 => "grave",
        _ => return None,
    };
    gdk::Key::from_name(name)
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
