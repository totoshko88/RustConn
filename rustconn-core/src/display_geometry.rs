//! Pure display-geometry math for embedded protocol viewers (GUI-free).
//!
//! Given the on-screen area a panel/widget occupies, [`desktop_request_for_area`]
//! computes the remote desktop resolution and DPI scale to request so that the
//! area is fully filled and the remote never drops below a minimum resolution.
//!
//! This module is deliberately free of GTK/libadwaita/VTE so the logic stays
//! testable and the `rustconn-core` crate boundary holds (see project rules).

/// Remote desktop resolution and DPI scale to request for a given on-screen area.
///
/// `width`/`height` are always even (RDP requires even dimensions) and non-zero
/// for any non-zero input area.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DesktopRequest {
    /// Requested remote desktop width in device pixels (even, non-zero).
    pub width: u32,
    /// Requested remote desktop height in device pixels (even, non-zero).
    pub height: u32,
    /// Requested remote DPI scale as a percentage (`100` == 100%).
    pub scale_percent: u16,
}

/// Largest DPI scale percentage the embedded clients support.
///
/// The Display Control (MS-RDPEDISP) / client scaling path is validated up to
/// 300%; requesting more offers no legibility gain and risks server rejection.
const SCALE_CEILING_PERCENT: u16 = 300;

/// Best-effort upscale factor used when even the largest [`UPSCALE_FACTORS`]
/// entry cannot lift an extreme-aspect tiny area to the minimum resolution.
///
/// The per-dimension `max` against the minimum still guarantees the minimum is
/// honored; only the aspect ratio is sacrificed in this rare degenerate case.
const MAX_UPSCALE_FACTOR: u32 = 3;

/// Integer upscale factors tried (smallest first) when the area is below the
/// minimum resolution.
///
/// Bounded to `{2, 3}`: `2` covers the common small-panel case, `3` the very
/// small case. A factor of `3` at a 100% base already reaches the 300% DPI
/// ceiling, so larger factors are pointless.
const UPSCALE_FACTORS: [u32; 2] = [2, MAX_UPSCALE_FACTOR];

/// Computes the remote desktop resolution and DPI scale to request for an area.
///
/// `area_w`/`area_h` are the device-pixel dimensions of the panel/widget,
/// `min_w`/`min_h` the smallest resolution the client will request (e.g.
/// `640`x`480` for RDP), and `base_scale_percent` the configured DPI scale
/// (e.g. `100`, `140`).
///
/// When the area is at least the minimum in both dimensions, the request
/// matches the area (rounded up to even) at the base scale. When the area is
/// smaller, the area is scaled up by the smallest integer factor in `{2, 3}`
/// that lifts both dimensions to the minimum, the DPI scale is raised by that
/// same factor (capped at 300%), and the local view downscales the frame to
/// fill the area. The result is always at or above the minimum resolution.
///
/// # Panics
///
/// Never panics for any input; all arithmetic is saturating and infallible.
#[must_use]
pub fn desktop_request_for_area(
    area_w: u32,
    area_h: u32,
    min_w: u32,
    min_h: u32,
    base_scale_percent: u16,
) -> DesktopRequest {
    let factor = upscale_factor(area_w, area_h, min_w, min_h);

    // Scale by the integer factor (aspect-preserving), then floor at the
    // minimum per dimension so the minimum is always honored, then round each
    // dimension up to an even number.
    let width = round_up_to_even(area_w.saturating_mul(factor).max(min_w));
    let height = round_up_to_even(area_h.saturating_mul(factor).max(min_h));

    let scaled = u32::from(base_scale_percent)
        .saturating_mul(factor)
        .min(u32::from(SCALE_CEILING_PERCENT));
    // `scaled` is clamped to `SCALE_CEILING_PERCENT`, so it always fits in u16.
    let scale_percent = u16::try_from(scaled).unwrap_or(SCALE_CEILING_PERCENT);

    DesktopRequest {
        width,
        height,
        scale_percent,
    }
}

/// Picks the smallest integer upscale factor that lifts the area to the minimum.
///
/// Returns `1` when the area already meets the minimum in both dimensions, the
/// smallest matching entry from [`UPSCALE_FACTORS`] otherwise, or
/// [`MAX_UPSCALE_FACTOR`] as a best-effort fallback for extreme-aspect tiny
/// areas that no bounded factor can lift.
fn upscale_factor(area_w: u32, area_h: u32, min_w: u32, min_h: u32) -> u32 {
    if area_w >= min_w && area_h >= min_h {
        return 1;
    }
    UPSCALE_FACTORS
        .into_iter()
        .find(|&k| area_w.saturating_mul(k) >= min_w && area_h.saturating_mul(k) >= min_h)
        .unwrap_or(MAX_UPSCALE_FACTOR)
}

/// Rounds `value` up to the nearest even number.
///
/// Returns `0` only for a `0` input; any non-zero input yields at least `2`.
/// RDP requires even desktop dimensions, and a zero dimension is never valid.
fn round_up_to_even(value: u32) -> u32 {
    if value == 0 {
        return 0;
    }
    value.saturating_add(1) & !1
}
