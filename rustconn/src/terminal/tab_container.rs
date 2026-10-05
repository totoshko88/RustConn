//! Per-tab container wrapper for `TabPage` content.
//!
//! `TabPageContainer` guarantees that every `TabPage.child()` always has a
//! non-zero allocation, which is required for `AdwTabOverview` to render
//! thumbnails without triggering Pango `size >= 0` assertions.
//!
//! A container holds a single VTE terminal (normal tab), a split layout
//! (`SplitViewBridge` widget), or the welcome/status page shown when no
//! sessions exist.

use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Orientation};

/// Widget name on the Welcome placeholder's outer box — an identity marker,
/// never shown and never translated.
const WELCOME_WIDGET_NAME: &str = "rustconn-welcome-page";

/// Wrapper around the `GtkBox` that serves as `TabPage.child()`.
///
/// The outer box is always `hexpand + vexpand`, so GTK never assigns it
/// a 0×0 allocation — even when the tab is not selected.
pub struct TabPageContainer {
    /// The widget that becomes `TabPage.child()`.
    outer: GtkBox,
}

impl TabPageContainer {
    /// Creates a container holding a single terminal.
    ///
    /// `content` is typically the vertical box holding the terminal overlay
    /// and (optionally) the monitoring bar.
    #[must_use]
    pub fn single(content: &GtkBox) -> Self {
        Self::wrap(content.upcast_ref())
    }

    /// Creates a container holding the welcome/status page.
    ///
    /// The outer box is tagged so [`Self::is_welcome`] can tell the
    /// placeholder apart from a session tab without looking at its title — a
    /// session may be called "Welcome" too, in any language.
    #[must_use]
    pub fn welcome(status_page: &gtk4::Widget) -> Self {
        let container = Self::wrap(status_page);
        container.outer.set_widget_name(WELCOME_WIDGET_NAME);
        container
    }

    /// Whether `child` (a `TabPage.child()`) is the Welcome placeholder.
    #[must_use]
    pub fn is_welcome(child: &gtk4::Widget) -> bool {
        child.widget_name() == WELCOME_WIDGET_NAME
    }

    fn wrap(content: &gtk4::Widget) -> Self {
        let outer = GtkBox::new(Orientation::Vertical, 0);
        outer.set_hexpand(true);
        outer.set_vexpand(true);
        outer.append(content);
        Self { outer }
    }

    /// Returns the outer widget (used as `TabPage.child()`).
    #[must_use]
    pub fn widget(&self) -> &GtkBox {
        &self.outer
    }

    /// Replaces the current content with the split view bridge widget.
    ///
    /// The caller is responsible for reparenting the terminal into the
    /// bridge *before* calling this.
    pub fn switch_to_split(&self, split_widget: &GtkBox) {
        self.clear_children();
        self.outer.append(split_widget);
    }

    /// Replaces the split widget with single-terminal content.
    pub fn switch_to_single(&self, content: &GtkBox) {
        self.clear_children();
        self.outer.append(content);
    }

    /// Removes all children from the outer box.
    fn clear_children(&self) {
        while let Some(child) = self.outer.first_child() {
            self.outer.remove(&child);
        }
    }
}
