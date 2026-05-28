//! Per-tab container wrapper for `TabPage` content.
//!
//! `TabPageContainer` guarantees that every `TabPage.child()` always has a
//! non-zero allocation, which is required for `AdwTabOverview` to render
//! thumbnails without triggering Pango `size >= 0` assertions.
//!
//! Each container can be in one of three states:
//! - **Single** — a single VTE terminal (normal tab)
//! - **Split** — a `SplitViewBridge` widget with multiple panes
//! - **Welcome** — the welcome/status page shown when no sessions exist

use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Orientation};

/// State of the content inside a `TabPageContainer`.
#[allow(
    dead_code,
    reason = "Split/Welcome variants used in Phase 2 of TabOverview refactoring"
)]
enum ContainerState {
    /// A single terminal (the default for new SSH tabs).
    Single,
    /// A split layout with multiple panes.
    Split,
    /// The welcome/status page.
    Welcome,
}

/// Wrapper around the `GtkBox` that serves as `TabPage.child()`.
///
/// The outer box is always `hexpand + vexpand`, so GTK never assigns it
/// a 0×0 allocation — even when the tab is not selected.
pub struct TabPageContainer {
    /// The widget that becomes `TabPage.child()`.
    outer: GtkBox,
    /// Current content state.
    #[allow(dead_code, reason = "Read in Phase 2 of TabOverview refactoring")]
    state: ContainerState,
}

impl TabPageContainer {
    /// Creates a container in **Single** state.
    ///
    /// `content` is typically the vertical box holding the terminal overlay
    /// and (optionally) the monitoring bar.
    #[must_use]
    pub fn single(content: &GtkBox) -> Self {
        let outer = GtkBox::new(Orientation::Vertical, 0);
        outer.set_hexpand(true);
        outer.set_vexpand(true);
        outer.append(content);
        Self {
            outer,
            state: ContainerState::Single,
        }
    }

    /// Creates a container in **Welcome** state.
    #[must_use]
    pub fn welcome(status_page: &gtk4::Widget) -> Self {
        let outer = GtkBox::new(Orientation::Vertical, 0);
        outer.set_hexpand(true);
        outer.set_vexpand(true);
        outer.append(status_page);
        Self {
            outer,
            state: ContainerState::Welcome,
        }
    }

    /// Returns the outer widget (used as `TabPage.child()`).
    #[must_use]
    pub fn widget(&self) -> &GtkBox {
        &self.outer
    }

    /// Transitions to **Split** state.
    ///
    /// Removes the current single-terminal content and replaces it with
    /// the split view bridge widget. The caller is responsible for
    /// reparenting the terminal into the bridge *before* calling this.
    #[allow(dead_code, reason = "Used in Phase 2 of TabOverview refactoring")]
    pub fn switch_to_split(&mut self, split_widget: &GtkBox) {
        self.clear_children();
        self.outer.append(split_widget);
        self.state = ContainerState::Split;
    }

    /// Transitions back to **Single** state.
    ///
    /// Removes the split widget and inserts the single-terminal content.
    #[allow(dead_code, reason = "Used in Phase 2 of TabOverview refactoring")]
    pub fn switch_to_single(&mut self, content: &GtkBox) {
        self.clear_children();
        self.outer.append(content);
        self.state = ContainerState::Single;
    }

    /// Returns `true` if the container is currently in split mode.
    #[must_use]
    #[allow(dead_code, reason = "Used in Phase 2 of TabOverview refactoring")]
    pub fn is_split(&self) -> bool {
        matches!(self.state, ContainerState::Split)
    }

    /// Returns `true` if the container is currently showing the welcome page.
    #[must_use]
    #[allow(dead_code, reason = "Used in Phase 2 of TabOverview refactoring")]
    pub fn is_welcome(&self) -> bool {
        matches!(self.state, ContainerState::Welcome)
    }

    /// Removes all children from the outer box.
    fn clear_children(&self) {
        while let Some(child) = self.outer.first_child() {
            self.outer.remove(&child);
        }
    }
}
