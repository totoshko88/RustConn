//! UI helpers for embedded VNC widget
//!
//! This module contains utility functions for:
//! - Coordinate transformation (widget ↔ VNC)
//! - Button mask conversion

/// Transforms widget coordinates to VNC framebuffer coordinates
///
/// This function handles the coordinate transformation needed when the widget
/// size differs from the VNC framebuffer size. It maintains aspect ratio and
/// centers the content.
///
/// # Arguments
/// * `x`, `y` - Widget coordinates (from mouse event)
/// * `widget_w`, `widget_h` - Current widget dimensions
/// * `vnc_w`, `vnc_h` - VNC framebuffer dimensions
///
/// # Returns
/// Tuple of (vnc_x, vnc_y) clamped to valid framebuffer coordinates
#[must_use]
pub fn transform_widget_to_vnc(
    x: f64,
    y: f64,
    widget_w: f64,
    widget_h: f64,
    vnc_w: f64,
    vnc_h: f64,
) -> (f64, f64) {
    // Calculate scale factor maintaining aspect ratio
    let scale = (widget_w / vnc_w).min(widget_h / vnc_h);

    // Calculate centering offsets
    let offset_x = vnc_w.mul_add(-scale, widget_w) / 2.0;
    let offset_y = vnc_h.mul_add(-scale, widget_h) / 2.0;

    // Transform and clamp to valid range
    let vnc_x = ((x - offset_x) / scale).clamp(0.0, vnc_w - 1.0);
    let vnc_y = ((y - offset_y) / scale).clamp(0.0, vnc_h - 1.0);

    (vnc_x, vnc_y)
}

/// Converts GTK button number to VNC button mask bit
///
/// GTK buttons: 1=left, 2=middle, 3=right
/// VNC mask bits: 0x01=left, 0x02=middle, 0x04=right
#[must_use]
pub const fn gtk_button_to_vnc_mask(gtk_button: u32) -> u8 {
    match gtk_button {
        1 => 0x01, // Left
        2 => 0x02, // Middle
        3 => 0x04, // Right
        _ => 0x00,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transform_widget_to_vnc_centered() {
        // Widget and VNC same size - no transformation needed
        let (x, y) = transform_widget_to_vnc(100.0, 100.0, 1920.0, 1080.0, 1920.0, 1080.0);
        assert!((x - 100.0).abs() < 0.001);
        assert!((y - 100.0).abs() < 0.001);
    }

    #[test]
    fn test_transform_widget_to_vnc_scaled() {
        // Widget is 2x larger than VNC
        let (x, y) = transform_widget_to_vnc(200.0, 200.0, 3840.0, 2160.0, 1920.0, 1080.0);
        assert!((x - 100.0).abs() < 0.001);
        assert!((y - 100.0).abs() < 0.001);
    }

    #[test]
    fn test_transform_widget_to_vnc_clamped() {
        // Coordinates outside VNC area should be clamped
        let (x, y) = transform_widget_to_vnc(-100.0, -100.0, 1920.0, 1080.0, 1920.0, 1080.0);
        assert!(x >= 0.0);
        assert!(y >= 0.0);

        let (x, y) = transform_widget_to_vnc(10000.0, 10000.0, 1920.0, 1080.0, 1920.0, 1080.0);
        assert!(x <= 1919.0);
        assert!(y <= 1079.0);
    }

    #[test]
    fn test_gtk_button_to_vnc_mask() {
        assert_eq!(gtk_button_to_vnc_mask(1), 0x01); // Left
        assert_eq!(gtk_button_to_vnc_mask(2), 0x02); // Middle
        assert_eq!(gtk_button_to_vnc_mask(3), 0x04); // Right
        assert_eq!(gtk_button_to_vnc_mask(4), 0x00); // Unknown
    }
}
