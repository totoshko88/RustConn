//! Multi-monitor support for RDP sessions
//!
//! This module provides structures and utilities for multi-monitor RDP sessions.
//! While IronRDP doesn't fully support multi-monitor yet, this prepares the
//! infrastructure for when it becomes available.
//!
//! # RDP Multi-Monitor Protocol
//!
//! Per MS-RDPBCGR 2.2.1.3.6, the client can advertise multiple monitors during
//! connection negotiation. Each monitor has:
//! - Position (left, top, right, bottom)
//! - Flags (primary monitor indicator)
//! - Physical dimensions (optional)

#![allow(clippy::cast_lossless)]

use serde::{Deserialize, Serialize};

/// Monitor definition for RDP multi-monitor support
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorDefinition {
    /// Monitor index (0-based)
    pub index: u32,
    /// Left edge position in virtual desktop coordinates
    pub left: i32,
    /// Top edge position in virtual desktop coordinates
    pub top: i32,
    /// Right edge position in virtual desktop coordinates
    pub right: i32,
    /// Bottom edge position in virtual desktop coordinates
    pub bottom: i32,
    /// Whether this is the primary monitor
    pub is_primary: bool,
    /// Physical width in millimeters (optional)
    pub physical_width_mm: Option<u32>,
    /// Physical height in millimeters (optional)
    pub physical_height_mm: Option<u32>,
}

impl MonitorDefinition {
    /// Creates a new monitor definition
    #[must_use]
    pub const fn new(index: u32, left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            index,
            left,
            top,
            right,
            bottom,
            is_primary: false,
            physical_width_mm: None,
            physical_height_mm: None,
        }
    }

    /// Creates a primary monitor definition
    #[must_use]
    pub const fn primary(width: u32, height: u32) -> Self {
        Self {
            index: 0,
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
            is_primary: true,
            physical_width_mm: None,
            physical_height_mm: None,
        }
    }

    /// Sets this monitor as primary
    #[must_use]
    pub const fn with_primary(mut self, is_primary: bool) -> Self {
        self.is_primary = is_primary;
        self
    }

    /// Sets physical dimensions
    #[must_use]
    pub const fn with_physical_size(mut self, width_mm: u32, height_mm: u32) -> Self {
        self.physical_width_mm = Some(width_mm);
        self.physical_height_mm = Some(height_mm);
        self
    }

    /// Returns the width of this monitor
    #[must_use]
    pub const fn width(&self) -> u32 {
        (self.right - self.left) as u32
    }

    /// Returns the height of this monitor
    #[must_use]
    pub const fn height(&self) -> u32 {
        (self.bottom - self.top) as u32
    }

    /// Returns the area of this monitor in pixels
    #[must_use]
    pub const fn area(&self) -> u64 {
        self.width() as u64 * self.height() as u64
    }

    /// Calculates DPI if physical dimensions are available
    #[must_use]
    pub fn dpi(&self) -> Option<u32> {
        let width_mm = self.physical_width_mm?;
        let height_mm = self.physical_height_mm?;

        if width_mm == 0 || height_mm == 0 {
            return None;
        }

        // Calculate diagonal in pixels and mm
        let diag_px = f64::from(self.width().pow(2) + self.height().pow(2)).sqrt();
        let diag_mm = f64::from(width_mm.pow(2) + height_mm.pow(2)).sqrt();

        // DPI = pixels / inches, 1 inch = 25.4 mm
        Some((diag_px / (diag_mm / 25.4)) as u32)
    }
}

/// Multi-monitor layout configuration
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorLayout {
    /// List of monitors
    pub monitors: Vec<MonitorDefinition>,
    /// Whether to use all monitors
    pub use_all_monitors: bool,
    /// Selected monitor indices (if not using all)
    pub selected_monitors: Vec<u32>,
    /// Whether to span across monitors (single desktop)
    pub span_monitors: bool,
}

impl MonitorLayout {
    /// Creates a new empty monitor layout
    #[must_use]
    pub const fn new() -> Self {
        Self {
            monitors: Vec::new(),
            use_all_monitors: false,
            selected_monitors: Vec::new(),
            span_monitors: false,
        }
    }

    /// Creates a single-monitor layout
    #[must_use]
    pub fn single(width: u32, height: u32) -> Self {
        Self {
            monitors: vec![MonitorDefinition::primary(width, height)],
            use_all_monitors: true,
            selected_monitors: vec![0],
            span_monitors: false,
        }
    }

    /// Adds a monitor to the layout
    pub fn add_monitor(&mut self, monitor: MonitorDefinition) {
        self.monitors.push(monitor);
    }

    /// Returns the primary monitor
    #[must_use]
    pub fn primary_monitor(&self) -> Option<&MonitorDefinition> {
        self.monitors.iter().find(|m| m.is_primary)
    }

    /// Returns the total virtual desktop bounds
    #[must_use]
    pub fn virtual_desktop_bounds(&self) -> (i32, i32, i32, i32) {
        if self.monitors.is_empty() {
            return (0, 0, 0, 0);
        }

        let left = self.monitors.iter().map(|m| m.left).min().unwrap_or(0);
        let top = self.monitors.iter().map(|m| m.top).min().unwrap_or(0);
        let right = self.monitors.iter().map(|m| m.right).max().unwrap_or(0);
        let bottom = self.monitors.iter().map(|m| m.bottom).max().unwrap_or(0);

        (left, top, right, bottom)
    }

    /// Returns the total virtual desktop size
    #[must_use]
    pub fn virtual_desktop_size(&self) -> (u32, u32) {
        let (left, top, right, bottom) = self.virtual_desktop_bounds();
        ((right - left) as u32, (bottom - top) as u32)
    }

    /// Returns the number of monitors
    #[must_use]
    pub fn monitor_count(&self) -> usize {
        self.monitors.len()
    }

    /// Returns whether multi-monitor is enabled
    #[must_use]
    pub fn is_multimonitor(&self) -> bool {
        self.monitors.len() > 1
    }

    /// Gets monitors that should be used for the session
    #[must_use]
    pub fn active_monitors(&self) -> Vec<&MonitorDefinition> {
        if self.use_all_monitors {
            self.monitors.iter().collect()
        } else {
            self.monitors
                .iter()
                .filter(|m| self.selected_monitors.contains(&m.index))
                .collect()
        }
    }
}

/// Detects monitors from the system
///
/// This function attempts to detect the current monitor configuration.
/// On Linux, it tries to parse output from xrandr or use Wayland protocols.
///
/// # Returns
///
/// A `MonitorLayout` with detected monitors, or a default single-monitor
/// layout if detection fails.
#[must_use]
pub fn detect_monitors() -> MonitorLayout {
    // Try to detect monitors from environment
    // This is a placeholder - actual implementation would use:
    // - Wayland: wl_output protocol
    // - X11: xrandr
    // - GTK: Gdk.Display.get_monitors() (but that's in GUI crate)

    // For now, return a sensible default
    MonitorLayout::single(1920, 1080)
}

/// Monitor arrangement for multi-monitor sessions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MonitorArrangement {
    /// Extend desktop across monitors
    #[default]
    Extend,
    /// Duplicate/mirror primary monitor
    Duplicate,
    /// Use only primary monitor
    PrimaryOnly,
    /// Use only secondary monitor(s)
    SecondaryOnly,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_monitor_definition_new() {
        let monitor = MonitorDefinition::new(0, 0, 0, 1920, 1080);
        assert_eq!(monitor.width(), 1920);
        assert_eq!(monitor.height(), 1080);
        assert!(!monitor.is_primary);
    }

    #[test]
    fn test_monitor_definition_primary() {
        let monitor = MonitorDefinition::primary(1920, 1080);
        assert!(monitor.is_primary);
        assert_eq!(monitor.index, 0);
        assert_eq!(monitor.left, 0);
        assert_eq!(monitor.top, 0);
    }

    #[test]
    fn test_monitor_area() {
        let monitor = MonitorDefinition::primary(1920, 1080);
        assert_eq!(monitor.area(), 1920 * 1080);
    }

    #[test]
    fn test_monitor_dpi() {
        // 24" 1920x1080 monitor is approximately 92 DPI
        let monitor = MonitorDefinition::primary(1920, 1080).with_physical_size(527, 296); // ~24" diagonal

        let dpi = monitor.dpi().unwrap();
        assert!(dpi > 85 && dpi < 100, "DPI was {}", dpi);
    }

    #[test]
    fn test_monitor_layout_single() {
        let layout = MonitorLayout::single(1920, 1080);
        assert_eq!(layout.monitor_count(), 1);
        assert!(!layout.is_multimonitor());
    }

    #[test]
    fn test_monitor_layout_virtual_desktop() {
        let mut layout = MonitorLayout::new();
        layout.add_monitor(MonitorDefinition::new(0, 0, 0, 1920, 1080).with_primary(true));
        layout.add_monitor(MonitorDefinition::new(1, 1920, 0, 3840, 1080));

        let (width, height) = layout.virtual_desktop_size();
        assert_eq!(width, 3840);
        assert_eq!(height, 1080);
    }

    #[test]
    fn test_monitor_layout_bounds() {
        let mut layout = MonitorLayout::new();
        layout.add_monitor(MonitorDefinition::new(0, -1920, 0, 0, 1080));
        layout.add_monitor(MonitorDefinition::new(1, 0, 0, 1920, 1080).with_primary(true));

        let (left, top, right, bottom) = layout.virtual_desktop_bounds();
        assert_eq!(left, -1920);
        assert_eq!(top, 0);
        assert_eq!(right, 1920);
        assert_eq!(bottom, 1080);
    }

    #[test]
    fn test_active_monitors() {
        let mut layout = MonitorLayout::new();
        layout.add_monitor(MonitorDefinition::new(0, 0, 0, 1920, 1080).with_primary(true));
        layout.add_monitor(MonitorDefinition::new(1, 1920, 0, 3840, 1080));
        layout.use_all_monitors = false;
        layout.selected_monitors = vec![0];

        let active = layout.active_monitors();
        assert_eq!(active.len(), 1);
        assert!(active[0].is_primary);
    }
}
