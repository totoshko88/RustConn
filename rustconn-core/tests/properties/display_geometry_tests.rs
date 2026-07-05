//! Property-based tests for `display_geometry::desktop_request_for_area`.
//!
//! **Feature: embedded-split-view**
//!
//! Validates the six correctness properties from the design document:
//! 1. Minimum honored, 2. Aspect preserved, 3. Fill without over-request,
//! 4. Bounded scale, 5. Determinism/idempotence, 6. Never degenerate.

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
fn large_area_matches_area_at_base_scale() {
    // 1920x1080 is well above 640x480 → request the area unchanged at 100%.
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
fn small_area_scales_up_by_two_at_double_scale() {
    // 400x300 < 640x480; factor 2 lifts both (800>=640, 600>=480) → 800x600 @ 200%.
    let req = desktop_request_for_area(400, 300, MIN_W, MIN_H, 100);
    assert_eq!(
        req,
        DesktopRequest {
            width: 800,
            height: 600,
            scale_percent: 200,
        }
    );
}

#[test]
fn tiny_area_is_clamped_to_minimum_and_capped_scale() {
    // 1x1 cannot be lifted by any bounded factor → clamp to the minimum at 300%.
    let req = desktop_request_for_area(1, 1, MIN_W, MIN_H, 100);
    assert_eq!(
        req,
        DesktopRequest {
            width: MIN_W,
            height: MIN_H,
            scale_percent: SCALE_CEILING,
        }
    );
}

#[test]
fn odd_dimensions_round_up_to_even() {
    let req = desktop_request_for_area(1921, 1081, MIN_W, MIN_H, 100);
    assert_eq!(req.width, 1922);
    assert_eq!(req.height, 1082);
    assert_eq!(req.scale_percent, 100);
}

#[test]
fn base_scale_is_capped_at_ceiling() {
    // base 140, factor 3 → 420 capped to 300.
    let req = desktop_request_for_area(1, 1, MIN_W, MIN_H, 140);
    assert_eq!(req.scale_percent, SCALE_CEILING);
}

// ============================================================================
// Strategies
// ============================================================================

/// Minimum resolution within a realistic range.
fn min_strategy() -> impl Strategy<Value = (u32, u32)> {
    (320u32..=1280, 240u32..=1024)
}

// ============================================================================
// Property tests
// ============================================================================

proptest! {
    /// Property 1: the result is always at or above the minimum resolution.
    #[test]
    fn prop_minimum_honored(
        area_w in 1u32..=8000,
        area_h in 1u32..=8000,
        (min_w, min_h) in min_strategy(),
        base in 50u16..=200,
    ) {
        let req = desktop_request_for_area(area_w, area_h, min_w, min_h, base);
        prop_assert!(req.width >= min_w, "width {} < min_w {}", req.width, min_w);
        prop_assert!(req.height >= min_h, "height {} < min_h {}", req.height, min_h);
    }

    /// Property 2: aspect ratio is preserved within rounding tolerance.
    ///
    /// The area range guarantees a bounded factor (<=3) always lifts both
    /// dimensions to the minimum, so no best-effort clamp distorts the aspect;
    /// only even-rounding introduces a sub-percent deviation. (The extreme
    /// tiny-area clamp case is an inherent, documented limitation where
    /// honoring the minimum — Property 1 — necessarily overrides aspect.)
    #[test]
    fn prop_aspect_preserved(
        area_w in 427u32..=8000,
        area_h in 342u32..=8000,
        min_w in 320u32..=1280,
        min_h in 240u32..=1024,
        base in 50u16..=200,
    ) {
        let req = desktop_request_for_area(area_w, area_h, min_w, min_h, base);
        let area_ratio = f64::from(area_w) / f64::from(area_h);
        let result_ratio = f64::from(req.width) / f64::from(req.height);
        let rel_error = (result_ratio - area_ratio).abs() / area_ratio;
        prop_assert!(
            rel_error <= 0.05,
            "aspect drift: area {area_w}x{area_h} ({area_ratio:.4}) -> \
             {}x{} ({result_ratio:.4}), rel_error {rel_error:.4}",
            req.width,
            req.height,
        );
    }

    /// Property 3: when the area meets the minimum in both dimensions, the
    /// request equals the area (rounded to even) at the base scale.
    #[test]
    fn prop_fill_without_over_request(
        area_w in MIN_W..=8000,
        area_h in MIN_H..=8000,
        base in 50u16..=200,
    ) {
        let req = desktop_request_for_area(area_w, area_h, MIN_W, MIN_H, base);
        prop_assert_eq!(req.width, round_up_to_even(area_w));
        prop_assert_eq!(req.height, round_up_to_even(area_h));
        prop_assert_eq!(req.scale_percent, base);
    }

    /// Property 4: scale is one of {base, base*2, base*3} clamped to the
    /// ceiling, and never exceeds the ceiling.
    #[test]
    fn prop_bounded_scale(
        area_w in 1u32..=8000,
        area_h in 1u32..=8000,
        (min_w, min_h) in min_strategy(),
        base in 50u16..=200,
    ) {
        let req = desktop_request_for_area(area_w, area_h, min_w, min_h, base);
        let allowed = [
            base,
            (base.saturating_mul(2)).min(SCALE_CEILING),
            (base.saturating_mul(3)).min(SCALE_CEILING),
        ];
        prop_assert!(
            allowed.contains(&req.scale_percent),
            "scale {} not in {:?}",
            req.scale_percent,
            allowed,
        );
        prop_assert!(req.scale_percent <= SCALE_CEILING);
    }

    /// Property 4 (monotonicity): scale is non-decreasing as the area shrinks
    /// in both dimensions.
    #[test]
    fn prop_scale_non_decreasing_as_area_shrinks(
        big_w in 1u32..=8000,
        big_h in 1u32..=8000,
        shrink_w in 0u32..=8000,
        shrink_h in 0u32..=8000,
        (min_w, min_h) in min_strategy(),
        base in 50u16..=200,
    ) {
        let small_w = big_w.saturating_sub(shrink_w).max(1);
        let small_h = big_h.saturating_sub(shrink_h).max(1);
        let big = desktop_request_for_area(big_w, big_h, min_w, min_h, base);
        let small = desktop_request_for_area(small_w, small_h, min_w, min_h, base);
        prop_assert!(
            small.scale_percent >= big.scale_percent,
            "shrinking {big_w}x{big_h} (scale {}) to {small_w}x{small_h} (scale {}) decreased scale",
            big.scale_percent,
            small.scale_percent,
        );
    }

    /// Property 5: deterministic, and feeding the result's own size back in
    /// (as a large area) is stable.
    #[test]
    fn prop_deterministic_and_idempotent(
        area_w in 1u32..=8000,
        area_h in 1u32..=8000,
        (min_w, min_h) in min_strategy(),
        base in 50u16..=200,
    ) {
        let first = desktop_request_for_area(area_w, area_h, min_w, min_h, base);
        let again = desktop_request_for_area(area_w, area_h, min_w, min_h, base);
        prop_assert_eq!(first, again, "not deterministic");

        // The result is >= min in both dims, so re-feeding its size is the
        // "large area" path and must leave the dimensions unchanged.
        let refed = desktop_request_for_area(first.width, first.height, min_w, min_h, base);
        prop_assert_eq!(refed.width, first.width, "width not idempotent");
        prop_assert_eq!(refed.height, first.height, "height not idempotent");
    }

    /// Property 6: dimensions are even and non-zero for any non-zero area.
    #[test]
    fn prop_never_degenerate(
        area_w in 1u32..=8000,
        area_h in 1u32..=8000,
        (min_w, min_h) in min_strategy(),
        base in 50u16..=200,
    ) {
        let req = desktop_request_for_area(area_w, area_h, min_w, min_h, base);
        prop_assert_eq!(req.width % 2, 0, "width {} not even", req.width);
        prop_assert_eq!(req.height % 2, 0, "height {} not even", req.height);
        prop_assert!(req.width > 0, "width is zero");
        prop_assert!(req.height > 0, "height is zero");
    }
}
