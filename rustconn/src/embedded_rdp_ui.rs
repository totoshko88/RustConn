//! UI helpers for embedded RDP widget
//!
//! This module contains utility functions for:
//! - Coordinate transformation (widget ↔ RDP)
//! - Status overlay drawing

use crate::embedded_rdp_types::RdpConfig;
use crate::embedded_rdp_types::RdpConnectionState;
use std::cell::RefCell;
use std::rc::Rc;

/// Transforms widget coordinates to RDP framebuffer coordinates
///
/// This function handles the coordinate transformation needed when the widget
/// size differs from the RDP framebuffer size. It maintains aspect ratio and
/// centers the content.
///
/// # Arguments
/// * `x`, `y` - Widget coordinates (from mouse event)
/// * `widget_w`, `widget_h` - Current widget dimensions
/// * `rdp_w`, `rdp_h` - RDP framebuffer dimensions
///
/// # Returns
/// Tuple of (rdp_x, rdp_y) clamped to valid framebuffer coordinates
#[must_use]
pub fn transform_widget_to_rdp(
    x: f64,
    y: f64,
    widget_w: f64,
    widget_h: f64,
    rdp_w: f64,
    rdp_h: f64,
) -> (f64, f64) {
    // Calculate scale factor maintaining aspect ratio
    let scale = (widget_w / rdp_w).min(widget_h / rdp_h);

    // Calculate centering offsets
    let offset_x = rdp_w.mul_add(-scale, widget_w) / 2.0;
    let offset_y = rdp_h.mul_add(-scale, widget_h) / 2.0;

    // Transform and clamp to valid range
    let rdp_x = ((x - offset_x) / scale).clamp(0.0, rdp_w - 1.0);
    let rdp_y = ((y - offset_y) / scale).clamp(0.0, rdp_h - 1.0);

    (rdp_x, rdp_y)
}

/// Converts GTK button number to RDP button mask bit
///
/// GTK buttons: 1=left, 2=middle, 3=right
/// RDP mask bits: 0x01=left, 0x02=right, 0x04=middle
#[must_use]
pub const fn gtk_button_to_rdp_mask(gtk_button: u32) -> u8 {
    match gtk_button {
        1 => 0x01, // Left
        2 => 0x04, // Middle
        3 => 0x02, // Right
        _ => 0x00,
    }
}

/// Converts GTK button number to RDP button number
///
/// GTK buttons: 1=left, 2=middle, 3=right
/// RDP buttons: 1=left, 2=right, 3=middle
#[must_use]
pub const fn gtk_button_to_rdp_button(gtk_button: u32) -> u8 {
    match gtk_button {
        1 => 1, // Left
        2 => 3, // Middle (GTK 2 → RDP 3)
        3 => 2, // Right (GTK 3 → RDP 2)
        _ => 1,
    }
}

/// Draws the status overlay on the RDP widget
///
/// This shows connection status, host information, and hints to the user.
/// Used when not rendering framebuffer (external mode, connecting, etc.)
#[allow(clippy::too_many_arguments)]
pub fn draw_status_overlay(
    cr: &gtk4::cairo::Context,
    width: i32,
    height: i32,
    current_state: RdpConnectionState,
    embedded: bool,
    config: &Rc<RefCell<Option<RdpConfig>>>,
    _rdp_width: &Rc<RefCell<u32>>,
    _rdp_height: &Rc<RefCell<u32>>,
) {
    cr.select_font_face(
        "Sans",
        gtk4::cairo::FontSlant::Normal,
        gtk4::cairo::FontWeight::Normal,
    );

    let center_y = f64::from(height) / 2.0 - 40.0;

    // Protocol icon (circle with "R" for RDP)
    let icon_color = match current_state {
        RdpConnectionState::Connected => (0.3, 0.6, 0.4), // Green
        RdpConnectionState::Connecting => (0.5, 0.5, 0.3), // Yellow
        RdpConnectionState::Error => (0.6, 0.3, 0.3),     // Red
        RdpConnectionState::Disconnected => (0.3, 0.5, 0.7), // Blue
    };
    cr.set_source_rgb(icon_color.0, icon_color.1, icon_color.2);
    cr.arc(
        f64::from(width) / 2.0,
        center_y,
        40.0,
        0.0,
        2.0 * std::f64::consts::PI,
    );
    let _ = cr.fill();

    // "R" letter in circle
    cr.set_source_rgb(1.0, 1.0, 1.0);
    cr.set_font_size(32.0);
    let extents = cr.text_extents("R").unwrap();
    cr.move_to(
        f64::from(width) / 2.0 - extents.width() / 2.0,
        center_y + extents.height() / 2.0,
    );
    let _ = cr.show_text("R");

    // Host name
    let config_ref = config.borrow();
    let host = config_ref
        .as_ref()
        .map(|c| c.host.as_str())
        .unwrap_or("No connection");

    cr.set_source_rgb(0.9, 0.9, 0.9);
    cr.set_font_size(18.0);
    let extents = cr.text_extents(host).unwrap();
    cr.move_to((f64::from(width) - extents.width()) / 2.0, center_y + 70.0);
    let _ = cr.show_text(host);

    // Status message
    cr.set_font_size(13.0);
    let (status_text, status_color) = match current_state {
        RdpConnectionState::Disconnected => {
            if config_ref.is_some() {
                ("Session ended", (0.8, 0.4, 0.4))
            } else {
                ("No connection configured", (0.5, 0.5, 0.5))
            }
        }
        RdpConnectionState::Connecting => {
            if embedded {
                ("Connecting via IronRDP...", (0.8, 0.8, 0.6))
            } else {
                ("Starting FreeRDP...", (0.8, 0.8, 0.6))
            }
        }
        RdpConnectionState::Connected => {
            if embedded {
                ("✓ Connected - waiting for display", (0.6, 0.8, 0.6))
            } else {
                ("✓ RDP session running in FreeRDP window", (0.6, 0.8, 0.6))
            }
        }
        RdpConnectionState::Error => ("Connection failed", (0.8, 0.4, 0.4)),
    };

    cr.set_source_rgb(status_color.0, status_color.1, status_color.2);
    let extents = cr.text_extents(status_text).unwrap();
    cr.move_to((f64::from(width) - extents.width()) / 2.0, center_y + 100.0);
    let _ = cr.show_text(status_text);

    // Hint for external mode
    if current_state == RdpConnectionState::Connected && !embedded {
        cr.set_source_rgb(0.6, 0.6, 0.6);
        cr.set_font_size(11.0);
        let hint = "Switch to the FreeRDP window to interact with the session";
        let extents = cr.text_extents(hint).unwrap();
        cr.move_to((f64::from(width) - extents.width()) / 2.0, center_y + 125.0);
        let _ = cr.show_text(hint);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transform_widget_to_rdp_centered() {
        // Widget and RDP same size - no transformation needed
        let (x, y) = transform_widget_to_rdp(100.0, 100.0, 1920.0, 1080.0, 1920.0, 1080.0);
        assert!((x - 100.0).abs() < 0.001);
        assert!((y - 100.0).abs() < 0.001);
    }

    #[test]
    fn test_transform_widget_to_rdp_scaled() {
        // Widget is 2x larger than RDP
        let (x, y) = transform_widget_to_rdp(200.0, 200.0, 3840.0, 2160.0, 1920.0, 1080.0);
        assert!((x - 100.0).abs() < 0.001);
        assert!((y - 100.0).abs() < 0.001);
    }

    #[test]
    fn test_transform_widget_to_rdp_clamped() {
        // Coordinates outside RDP area should be clamped
        let (x, y) = transform_widget_to_rdp(-100.0, -100.0, 1920.0, 1080.0, 1920.0, 1080.0);
        assert!(x >= 0.0);
        assert!(y >= 0.0);

        let (x, y) = transform_widget_to_rdp(10000.0, 10000.0, 1920.0, 1080.0, 1920.0, 1080.0);
        assert!(x <= 1919.0);
        assert!(y <= 1079.0);
    }

    #[test]
    fn test_gtk_button_to_rdp_mask() {
        assert_eq!(gtk_button_to_rdp_mask(1), 0x01); // Left
        assert_eq!(gtk_button_to_rdp_mask(2), 0x04); // Middle
        assert_eq!(gtk_button_to_rdp_mask(3), 0x02); // Right
        assert_eq!(gtk_button_to_rdp_mask(4), 0x00); // Unknown
    }

    #[test]
    fn test_gtk_button_to_rdp_button() {
        assert_eq!(gtk_button_to_rdp_button(1), 1); // Left → Left
        assert_eq!(gtk_button_to_rdp_button(2), 3); // Middle → Middle (RDP 3)
        assert_eq!(gtk_button_to_rdp_button(3), 2); // Right → Right (RDP 2)
    }
}
