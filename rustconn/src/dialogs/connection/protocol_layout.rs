//! Common layout builder for protocol options panels
//!
//! This module provides a reusable builder for the standard protocol options
//! layout pattern: ScrolledWindow → Clamp → Box with consistent margins.

// Builder methods are available for future customization needs
#![allow(dead_code)]

use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Orientation, ScrolledWindow};
use libadwaita as adw;

/// Builder for protocol options panel layout.
///
/// Creates the standard layout structure used by all protocol options:
/// - ScrolledWindow (vertical scrolling only)
/// - Clamp (max 600px, tightening at 400px)
/// - Vertical Box with 12px spacing and margins
///
/// # Example
/// ```ignore
/// let (container, content) = ProtocolLayoutBuilder::new()
///     .build();
///
/// // Add preference groups to content
/// content.append(&my_group);
/// ```
#[derive(Debug, Clone)]
pub struct ProtocolLayoutBuilder {
    max_size: i32,
    tightening_threshold: i32,
    spacing: i32,
    margin: i32,
}

impl Default for ProtocolLayoutBuilder {
    fn default() -> Self {
        Self {
            max_size: 600,
            tightening_threshold: 400,
            spacing: 12,
            margin: 12,
        }
    }
}

impl ProtocolLayoutBuilder {
    /// Creates a new builder with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the maximum width for the clamp.
    #[must_use]
    pub fn max_size(mut self, size: i32) -> Self {
        self.max_size = size;
        self
    }

    /// Sets the tightening threshold for the clamp.
    #[must_use]
    pub fn tightening_threshold(mut self, threshold: i32) -> Self {
        self.tightening_threshold = threshold;
        self
    }

    /// Sets the spacing between child widgets.
    #[must_use]
    pub fn spacing(mut self, spacing: i32) -> Self {
        self.spacing = spacing;
        self
    }

    /// Sets the margin around the content.
    #[must_use]
    pub fn margin(mut self, margin: i32) -> Self {
        self.margin = margin;
        self
    }

    /// Builds the layout and returns the container and content box.
    ///
    /// Returns a tuple of:
    /// - The outer container (GtkBox) to be used as the tab content
    /// - The inner content box where preference groups should be added
    #[must_use]
    pub fn build(self) -> (GtkBox, GtkBox) {
        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .build();

        let clamp = adw::Clamp::builder()
            .maximum_size(self.max_size)
            .tightening_threshold(self.tightening_threshold)
            .build();

        let content = GtkBox::new(Orientation::Vertical, self.spacing);
        content.set_margin_top(self.margin);
        content.set_margin_bottom(self.margin);
        content.set_margin_start(self.margin);
        content.set_margin_end(self.margin);

        clamp.set_child(Some(&content));
        scrolled.set_child(Some(&clamp));

        let container = GtkBox::new(Orientation::Vertical, 0);
        container.append(&scrolled);

        (container, content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_values() {
        let builder = ProtocolLayoutBuilder::default();
        assert_eq!(builder.max_size, 600);
        assert_eq!(builder.tightening_threshold, 400);
        assert_eq!(builder.spacing, 12);
        assert_eq!(builder.margin, 12);
    }

    #[test]
    fn test_builder_chaining() {
        let builder = ProtocolLayoutBuilder::new()
            .max_size(800)
            .tightening_threshold(500)
            .spacing(16)
            .margin(8);

        assert_eq!(builder.max_size, 800);
        assert_eq!(builder.tightening_threshold, 500);
        assert_eq!(builder.spacing, 16);
        assert_eq!(builder.margin, 8);
    }
}
