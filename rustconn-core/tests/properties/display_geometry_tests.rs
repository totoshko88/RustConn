//! Property-based tests for `display_geometry::desktop_request_for_area`.
//!
//! **Feature: embedded-split-view**
//!
//! The helper keys on the widget's *logical* size (`device ÷ base`):
//! - comfortable window (logical ≥ minimum) → full device resolution at the
//!   display DPI (HiDPI/retina), DPI capped at 300%;
//! - small window (logical < minimum) → logical size × {2, 3} to reach the
//!   minimum, at a fixed **100% DPI**, so the server renders a normal-sized
//!   cursor/UI and the viewer downscales the larger frame into the window
//!   (dense, small content, normal cursor, no reconnect).

use proptest::prelude::*;
use rustconn_core::display_geometry::{DesktopRequest, desktop_request_for_area};

/// RDP-style minimum desktop resolution used across the tests.
const MIN_W: u32 = 640;
const MIN_H: u32 = 480;
const SCALE_CEILING: u16 = 300;

/// Rounds up to the nearest even number, mirroring the module's private helper,
/// so the tests can assert exact expected dimensions.
fn round_up_to_even(value: u32) -> u32 {
    if value == 0 {
        0
    } else {
        value.saturating_add(1) & !1
    }
}

// ============================================================================
// Concrete unit tests
// ============================================================================

#[test]
fn comfortable_window_matches_device_at_base_scale() {
    // 1920x1080 logical at 100% is well above 640x480 → request it unchanged.
    let req = desktop_request_for_area(1920, 1080, MIN_W, MIN_H, 100);
    assert_eq!(
        req,
        DesktopRequest {
            width: 1920,
            height: 1080,
            scale_percent: 100,
        }
    );
}

#[test]
fn comfortable_hidpi_window_keeps_full_scale() {
    // 3840x2160 device at 200% → logical 1920x1080 (comfortable) → full retina.
    let req = desktop_request_for_area(3840, 2160, MIN_W, MIN_H, 200);
    assert_eq!(
        req,
        DesktopRequest {
            width: 3840,
            height: 2160,
            scale_percent: 200,
        }
    );
}

#[test]
fn comfortable_scale_is_capped_at_ceiling() {
    // 3840x2160 device at 400% → logical 960x540 (comfortable) → DPI capped 300.
    let req = desktop_request_for_area(3840, 2160, MIN_W, MIN_H, 400);
    assert_eq!(req.width, 3840);
    assert_eq!(req.height, 2160);
    assert_eq!(req.scale_percent, SCALE_CEILING);
}

#[test]
fn small_hidpi_window_requests_larger_desktop_at_100_dpi() {
    // Reported bug: a ~373x270 CSS window on a 2× display -> 746x540 device px.
    // Logical 373x270 < 640x480 → small mode: request logical×2 (746x540, which
    // reaches the minimum) at a fixed 100% DPI. The server then renders a
    // normal-sized cursor/UI and the viewer downscales the frame into the small
    // window, so everything appears small — not a huge 200%-DPI cursor.
    let req = desktop_request_for_area(746, 540, MIN_W, MIN_H, 200);
    assert_eq!(
        req,
        DesktopRequest {
            width: 746,
            height: 540,
            scale_percent: 100,
        }
    );
}

#[test]
fn small_window_scales_up_by_two_at_100_dpi() {
    // 400x300 logical at 100% < 640x480; factor 2 lifts both → 800x600 @ 100%.
    let req = desktop_request_for_area(400, 300, MIN_W, MIN_H, 100);
    assert_eq!(
        req,
        DesktopRequest {
            width: 800,
            height: 600,
            scale_percent: 100,
        }
    );
}

#[test]
fn tiny_window_is_clamped_to_minimum_at_100_dpi() {
    // 1x1 cannot be lifted by any bounded factor → clamp to the minimum at 100%.
    let req = desktop_request_for_area(1, 1, MIN_W, MIN_H, 100);
    assert_eq!(
        req,
        DesktopRequest {
            width: MIN_W,
            height: MIN_H,
            scale_percent: 100,
        }
    );
}

#[test]
fn odd_comfortable_dimensions_round_up_to_even() {
    let req = desktop_request_for_area(1921, 1081, MIN_W, MIN_H, 100);
    assert_eq!(req.width, 1922);
    assert_eq!(req.height, 1082);
    assert_eq!(req.scale_percent, 100);
}

// ============================================================================
// Property tests
// ============================================================================

proptest! {
    /// The result is always at or above the minimum resolution.
    #[test]
    fn prop_minimum_honored(
        device_w in 1u32..=8000,
        device_h in 1u32..=8000,
        base in 100u16..=300,
    ) {
        let req = desktop_request_for_area(device_w, device_h, MIN_W, MIN_H, base);
        prop_assert!(req.width >= MIN_W, "width {} < {}", req.width, MIN_W);
        prop_assert!(req.height >= MIN_H, "height {} < {}", req.height, MIN_H);
    }

    /// Dimensions are even and non-zero for any non-zero device area.
    #[test]
    fn prop_never_degenerate(
        device_w in 1u32..=8000,
        device_h in 1u32..=8000,
        base in 100u16..=300,
    ) {
        let req = desktop_request_for_area(device_w, device_h, MIN_W, MIN_H, base);
        prop_assert_eq!(req.width % 2, 0, "width {} not even", req.width);
        prop_assert_eq!(req.height % 2, 0, "height {} not even", req.height);
        prop_assert!(req.width > 0 && req.height > 0, "zero dimension");
    }

    /// Scale is always within [100, 300].
    #[test]
    fn prop_scale_within_bounds(
        device_w in 1u32..=8000,
        device_h in 1u32..=8000,
        base in 100u16..=300,
    ) {
        let req = desktop_request_for_area(device_w, device_h, MIN_W, MIN_H, base);
        prop_assert!(req.scale_percent >= 100, "scale {} < 100", req.scale_percent);
        prop_assert!(req.scale_percent <= SCALE_CEILING, "scale {} > 300", req.scale_percent);
    }

    /// The implied logical desktop (resolution ÷ scale) never drops below the
    /// minimum — a small HiDPI window can no longer ask for an unusably tiny
    /// logical desktop with a giant cursor.
    #[test]
    fn prop_logical_desktop_not_below_minimum(
        device_w in 1u32..=8000,
        device_h in 1u32..=8000,
        base in 100u16..=300,
    ) {
        let req = desktop_request_for_area(device_w, device_h, MIN_W, MIN_H, base);
        let logical_w = req.width * 100 / u32::from(req.scale_percent);
        let logical_h = req.height * 100 / u32::from(req.scale_percent);
        prop_assert!(logical_w >= MIN_W, "logical width {logical_w} < {MIN_W}");
        prop_assert!(logical_h >= MIN_H, "logical height {logical_h} < {MIN_H}");
    }

    /// Branch behaviour: a small logical window uses a fixed 100% DPI; a
    /// comfortable one keeps the (capped) display DPI and the device resolution.
    #[test]
    fn prop_branch_matches_logical_size(
        device_w in 1u32..=8000,
        device_h in 1u32..=8000,
        base in 100u16..=300,
    ) {
        let req = desktop_request_for_area(device_w, device_h, MIN_W, MIN_H, base);
        let logical_w = device_w * 100 / u32::from(base);
        let logical_h = device_h * 100 / u32::from(base);
        if logical_w >= MIN_W && logical_h >= MIN_H {
            prop_assert_eq!(req.width, round_up_to_even(device_w));
            prop_assert_eq!(req.height, round_up_to_even(device_h));
            prop_assert_eq!(u32::from(req.scale_percent), u32::from(base).min(u32::from(SCALE_CEILING)));
        } else {
            prop_assert_eq!(req.scale_percent, 100, "small window must use 100% DPI");
        }
    }

    /// Deterministic: identical inputs yield identical requests.
    #[test]
    fn prop_deterministic(
        device_w in 1u32..=8000,
        device_h in 1u32..=8000,
        base in 100u16..=300,
    ) {
        let first = desktop_request_for_area(device_w, device_h, MIN_W, MIN_H, base);
        let again = desktop_request_for_area(device_w, device_h, MIN_W, MIN_H, base);
        prop_assert_eq!(first, again);
    }

    /// Comfortable windows preserve the device aspect ratio (within even-rounding
    /// tolerance), so a HiDPI desktop is not distorted.
    #[test]
    fn prop_comfortable_aspect_preserved(
        // device sizes that stay comfortable for any base in range (>= 640*300/100).
        device_w in 1920u32..=8000,
        device_h in 1440u32..=8000,
        base in 100u16..=300,
    ) {
        let req = desktop_request_for_area(device_w, device_h, MIN_W, MIN_H, base);
        let area_ratio = f64::from(device_w) / f64::from(device_h);
        let result_ratio = f64::from(req.width) / f64::from(req.height);
        let rel_error = (result_ratio - area_ratio).abs() / area_ratio;
        prop_assert!(rel_error <= 0.02, "aspect drift {rel_error:.4}");
    }
}
