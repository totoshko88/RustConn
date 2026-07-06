//! Pure display-geometry math for embedded protocol viewers (GUI-free).
//!
//! Given the on-screen area a panel/widget occupies, [`desktop_request_for_area`]
//! computes the remote desktop resolution and DPI scale to request so that the
//! remote never drops below a minimum resolution and a small window shows a
//! full, dense desktop with a normal-sized cursor.
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

/// Best-effort upscale factor for a logical window so small that even the
/// largest [`UPSCALE_FACTORS`] entry cannot lift it to the minimum resolution.
///
/// The per-dimension `max` against the minimum still guarantees the minimum is
/// honored; only the aspect ratio is sacrificed in this rare degenerate case.
const MAX_UPSCALE_FACTOR: u32 = 3;

/// Integer upscale factors tried (smallest first) when the logical window is
/// below the minimum: `2` ("200%") for the common small case, `3` ("300%") for
/// the very small case. A factor of `3` already reaches most real minimums.
const UPSCALE_FACTORS: [u32; 2] = [2, MAX_UPSCALE_FACTOR];

/// Computes the remote desktop resolution and DPI scale to request for an area.
///
/// `device_w`/`device_h` are the **device-pixel** dimensions of the panel/widget
/// (logical CSS size × display scale factor), `min_w`/`min_h` the smallest
/// resolution the client will request (e.g. `640`x`480` for RDP), and
/// `base_scale_percent` the configured DPI scale as a percentage (e.g. `100`
/// for Auto, `200` on a 2× display).
///
/// The decision hinges on the widget's **logical** size (`device ÷ base`), i.e.
/// the window the user actually sees — a small window on a HiDPI display still
/// has a large device-pixel area, so keying on device pixels would misclassify
/// it.
///
/// - **Comfortable window** (logical ≥ minimum): request the full device
///   resolution at the display DPI, so a HiDPI screen gets a crisp full-scale
///   desktop (the "Native"/retina behaviour from #207). The DPI is capped at
///   [`SCALE_CEILING_PERCENT`].
/// - **Small window** (logical < minimum): request the logical size scaled up by
///   the smallest integer factor in `{2, 3}` that reaches the minimum (the
///   "200% / 300%" request), at a **fixed 100% DPI**. The server then renders a
///   normal-sized cursor and UI on a ≥-minimum desktop, and the viewer
///   downscales that larger frame into the small window, so everything appears
///   small (dense) with a normal cursor — and no reconnect is needed.
///
/// The result is always at or above the minimum resolution.
///
/// # Panics
///
/// Never panics for any input; all arithmetic is saturating and infallible.
#[must_use]
pub fn desktop_request_for_area(
    device_w: u32,
    device_h: u32,
    min_w: u32,
    min_h: u32,
    base_scale_percent: u16,
) -> DesktopRequest {
    let base = u32::from(base_scale_percent).max(1);

    // Recover the widget's logical (CSS) size. The "too small" test is about the
    // logical window the user sees, not the device-pixel count.
    let logical_w = device_w.saturating_mul(100) / base;
    let logical_h = device_h.saturating_mul(100) / base;

    if logical_w >= min_w && logical_h >= min_h {
        // Comfortable window: full device resolution at the display DPI (retina),
        // capped at the ceiling for safety.
        let scale = base.min(u32::from(SCALE_CEILING_PERCENT));
        return DesktopRequest {
            width: round_up_to_even(device_w),
            height: round_up_to_even(device_h),
            scale_percent: u16::try_from(scale).unwrap_or(SCALE_CEILING_PERCENT),
        };
    }

    // Small window: request a >= minimum desktop at 100% DPI (normal cursor/UI),
    // scaled up by the smallest factor in {2, 3} that reaches the minimum. The
    // viewer downscales the larger frame into the window → dense, small content.
    let factor = UPSCALE_FACTORS
        .into_iter()
        .find(|&k| logical_w.saturating_mul(k) >= min_w && logical_h.saturating_mul(k) >= min_h)
        .unwrap_or(MAX_UPSCALE_FACTOR);
    DesktopRequest {
        width: round_up_to_even(logical_w.saturating_mul(factor).max(min_w)),
        height: round_up_to_even(logical_h.saturating_mul(factor).max(min_h)),
        scale_percent: 100,
    }
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
