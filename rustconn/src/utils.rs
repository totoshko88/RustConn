//! Utility functions for the RustConn GUI
//!
//! This module provides common utility functions used across the application,
//! including safe display access, CSS provider management, and accessibility helpers.

use gtk4::gdk;
use gtk4::prelude::DisplayExtManual;

/// Theme-aware colors for a cairo-drawn placeholder/status surface.
///
/// The embedded RDP/VNC viewers paint their "no video yet" status screens
/// directly on a `DrawingArea` with cairo, which cannot use CSS classes. Using
/// a fixed dark fill there renders as a black rectangle on a light desktop
/// (GNOME HIG: do not hardcode colors that ignore the theme). This returns a
/// `(background, foreground)` RGB pair that follows the active light/dark theme,
/// so the status screen matches the rest of the chrome. Status *accent* colors
/// (green/yellow/red/blue state dots) stay fixed by the caller because they
/// encode meaning, not chrome.
///
/// # Returns
/// `((bg_r, bg_g, bg_b), (fg_r, fg_g, fg_b))` in cairo's 0.0–1.0 range.
#[must_use]
pub fn placeholder_surface_colors() -> ((f64, f64, f64), (f64, f64, f64)) {
    let dark = libadwaita::StyleManager::default().is_dark();
    if dark {
        ((0.12, 0.12, 0.14), (0.90, 0.90, 0.90))
    } else {
        ((0.95, 0.95, 0.96), (0.18, 0.18, 0.20))
    }
}

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

/// The scale percentage to report when no window is mapped yet.
///
/// Also FreeRDP's own default, so a session launched before any window exists is
/// requested with no DPI override rather than with a guessed one.
const UNKNOWN_DISPLAY_SCALE_PERCENT: u16 = 100;

/// Returns the scale of the display the application is on, as a percentage.
///
/// `100` means 100%; a 2× HiDPI monitor reports `200` and a fractional 125%
/// monitor reports `125`.
///
/// An external protocol client opens its own top-level window, so unlike an
/// embedded viewer it cannot be measured through the widget it draws into. The
/// application's own window is the best available proxy — the client is placed
/// by the compositor and lands on the focused monitor in practice.
///
/// Reads `gdk::Surface::scale` for the true fractional value rather than the
/// integer `scale_factor()`, which rounds 125% up to 2 and so would report a
/// fractional display as double.
#[must_use]
pub fn active_display_scale_percent() -> u16 {
    use gtk4::gdk::prelude::SurfaceExt;
    use gtk4::prelude::{Cast, GtkApplicationExt, NativeExt, WidgetExt};

    let Some(window) = gtk4::gio::Application::default()
        .and_then(|app| app.downcast::<gtk4::Application>().ok())
        .and_then(|app| app.active_window())
    else {
        return UNKNOWN_DISPLAY_SCALE_PERCENT;
    };

    let scale = window
        .surface()
        .map(|surface| surface.scale())
        .filter(|scale| *scale > 0.0)
        // No surface means the window is not mapped; the integer scale factor is
        // still meaningful and is all X11 without fractional scaling reports.
        .unwrap_or_else(|| f64::from(window.scale_factor()));

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the value is clamped into the u16 range and is positive by construction"
    )]
    let percent = (scale * 100.0).round().clamp(
        f64::from(UNKNOWN_DISPLAY_SCALE_PERCENT),
        f64::from(u16::MAX),
    ) as u16;
    percent
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
/// A vector of unique variable names in first-seen (template) order.
///
/// Thin re-export of [`rustconn_core::variables::extract_variable_names`] — the
/// single canonical extractor the whole workspace shares, so this form and
/// `SnippetManager::extract_variables` can never drift in which names they find
/// or in what order.
#[must_use]
pub fn extract_variables(template: &str) -> Vec<String> {
    rustconn_core::variables::extract_variable_names(template)
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
    // A oneshot future instead of a 16 ms polling timer: the main loop wakes
    // only when the worker finishes. shortcut: if the worker panics the
    // callback is not called (its signature takes `T`, not `Option<T>`), same as
    // before; upgrade by moving callers to `spawn_blocking_with_timeout`.
    let (tx, rx) = tokio::sync::oneshot::channel();

    std::thread::spawn(move || {
        let _ = tx.send(operation());
    });

    gtk4::glib::spawn_future_local(async move {
        if let Ok(result) = rx.await {
            callback(result);
        } else {
            tracing::warn!("Background operation ended without a result (panic?)");
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
    let (tx, rx) = tokio::sync::oneshot::channel();

    std::thread::spawn(move || {
        let _ = tx.send(operation());
    });

    gtk4::glib::spawn_future_local(async move {
        // `Err` from the channel = the worker panicked; `Err` from the timeout
        // = it took too long (the thread keeps running, its result is dropped).
        match gtk4::glib::future_with_timeout(timeout, rx).await {
            Ok(Ok(result)) => callback(Some(result)),
            Ok(Err(_)) => callback(None),
            Err(_) => {
                tracing::warn!("Background operation timed out after {:?}", timeout);
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
        assert_eq!(vars, Vec::<String>::new());
    }

    /// Runs `ctx` until `done` is set or a few seconds pass.
    fn run_until(ctx: &gtk4::glib::MainContext, done: &std::rc::Rc<std::cell::Cell<bool>>) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !done.get() && std::time::Instant::now() < deadline {
            ctx.iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    #[test]
    fn spawn_blocking_callbacks_run_on_main_context() {
        let ctx = gtk4::glib::MainContext::new();
        ctx.with_thread_default(|| {
            let done = std::rc::Rc::new(std::cell::Cell::new(false));
            let got = std::rc::Rc::new(std::cell::Cell::new(None));
            let (d, g) = (done.clone(), got.clone());
            spawn_blocking_with_callback(
                || 41 + 1,
                move |v| {
                    g.set(Some(v));
                    d.set(true);
                },
            );
            run_until(&ctx, &done);
            assert_eq!(got.get(), Some(42));

            // A panicking worker reports `None` through the timeout variant.
            let done = std::rc::Rc::new(std::cell::Cell::new(false));
            let got = std::rc::Rc::new(std::cell::Cell::new(Some(0)));
            let (d, g) = (done.clone(), got.clone());
            spawn_blocking_with_timeout(
                || -> i32 { panic!("worker panic (expected in test)") },
                std::time::Duration::from_secs(3),
                move |v| {
                    g.set(v);
                    d.set(true);
                },
            );
            run_until(&ctx, &done);
            assert!(done.get() && got.get().is_none());
        })
        .expect("set thread-default context");
    }
}
