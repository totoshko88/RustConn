//! Toast notification system using libadwaita
//!
//! Wraps `adw::ToastOverlay` to provide a simple interface for showing notifications.
//! Supports standard toast types (info, success, warning, error) and actions.
//!
//! # Accessibility
//!
//! Toast notifications are automatically announced by screen readers via
//! libadwaita's built-in accessibility support.

use adw::prelude::*;
use gtk4 as gui;
use gui::glib;
use libadwaita as adw;

/// Toast message types for styling and semantic meaning
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastType {
    /// Informational message (default)
    Info,
    /// Success message
    Success,
    /// Warning message
    Warning,
    /// Error message
    Error,
}

impl ToastType {
    /// Returns the CSS class for this toast type
    #[must_use]
    pub const fn css_class(&self) -> &'static str {
        match self {
            Self::Info => "toast-info",
            Self::Success => "toast-success",
            Self::Warning => "toast-warning",
            Self::Error => "toast-error",
        }
    }

    /// Returns the icon name for this toast type
    #[must_use]
    pub const fn icon_name(&self) -> &'static str {
        match self {
            Self::Info => "dialog-information-symbolic",
            Self::Success => "object-select-symbolic",
            Self::Warning => "dialog-warning-symbolic",
            Self::Error => "dialog-error-symbolic",
        }
    }

    /// Returns the priority for this toast type
    /// Higher priority toasts are shown first
    #[must_use]
    pub const fn priority(&self) -> adw::ToastPriority {
        match self {
            Self::Info => adw::ToastPriority::Normal,
            Self::Success => adw::ToastPriority::Normal,
            Self::Warning => adw::ToastPriority::High,
            Self::Error => adw::ToastPriority::High,
        }
    }
}

/// Toast overlay widget that wraps `adw::ToastOverlay`
pub struct ToastOverlay {
    /// The underlying libadwaita toast overlay
    overlay: adw::ToastOverlay,
}

impl ToastOverlay {
    /// Creates a new toast overlay
    #[must_use]
    pub fn new() -> Self {
        Self {
            overlay: adw::ToastOverlay::new(),
        }
    }

    /// Returns the overlay widget to add to the UI
    #[must_use]
    pub fn widget(&self) -> &adw::ToastOverlay {
        &self.overlay
    }

    /// Sets the main content of the overlay
    pub fn set_child(&self, child: Option<&impl IsA<gui::Widget>>) {
        self.overlay.set_child(child);
    }

    /// Shows a toast message with default options
    pub fn show_toast(&self, message: &str) {
        let toast = adw::Toast::new(message);
        self.overlay.add_toast(toast);
    }

    /// Shows a toast type with appropriate priority
    ///
    /// Uses `adw::ToastPriority` to ensure important messages (warnings, errors)
    /// are shown before less important ones.
    pub fn show_toast_with_type(&self, message: &str, toast_type: ToastType) {
        let toast = adw::Toast::new(message);
        toast.set_priority(toast_type.priority());
        self.overlay.add_toast(toast);
    }

    /// Shows a success toast message
    pub fn show_success(&self, message: &str) {
        self.show_toast_with_type(message, ToastType::Success);
    }

    /// Shows a warning toast message (high priority)
    pub fn show_warning(&self, message: &str) {
        self.show_toast_with_type(message, ToastType::Warning);
    }

    /// Shows an error toast message (high priority)
    pub fn show_error(&self, message: &str) {
        self.show_toast_with_type(message, ToastType::Error);
    }

    /// Shows a toast with an action (e.g. "Undo")
    pub fn show_toast_with_action(
        &self,
        message: &str,
        action_label: &str,
        action_name: &str,
        action_target: Option<&glib::Variant>,
    ) {
        let toast = adw::Toast::new(message);
        toast.set_button_label(Some(action_label));
        toast.set_action_name(Some(action_name));
        if let Some(target) = action_target {
            toast.set_action_target_value(Some(target));
        }
        self.overlay.add_toast(toast);
    }
}

impl Default for ToastOverlay {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper function to show a toast on a window (legacy support)
///
/// Tries to find an `adw::ToastOverlay` in the window structure or falls back to
/// standard overlay injection if possible (though less ideal with adw).
pub fn show_toast_on_window(window: &impl IsA<gui::Window>, message: &str, _toast_type: ToastType) {
    // This is a "best effort" helper. Ideally we should pass the overlay explicitly.
    // If the window is an adw::ApplicationWindow or similar that exposes an overlay...
    // But currently we don't have a direct way to find the main overlay unless we walk the hierarchy.

    // For now, if we can't find the overlay, we might just log it or (better)
    // upgrading call sites to use the proper overlay.
    // However, to keep existing code working without massive refactoring:
    // We can try to create a transient overlay? No, that won't work.

    // Instead of complex hierarchy walking, let's rely on the fact that
    // most call sites using this function are likely in dialogs or windows
    // where we might want to attach a toast overlay.

    // Since we are refactoring, let's fix the call sites to use the overlay if possible.
    // But for this helper, let's leave it as a no-op or simple print for safety during migration
    // if we can't easily hook into adw::ToastOverlay.

    // Actually, `adw::ToastOverlay` is usually the root content.
    // If we can get it, we use it.
    // But `window.child()` might be `adw::ToolbarView`.

    // Let's implement a hierarchy check
    if let Some(child) = window.child()
        && let Some(overlay) = find_toast_overlay(&child)
    {
        let toast = adw::Toast::new(message);
        overlay.add_toast(toast);
        return;
    }

    // Fallback: log so we don't silently lose messages during dev
    tracing::warn!(toast_message = %message, "Could not find ToastOverlay in window hierarchy");
}

/// Helper to recursively find a `ToastOverlay` in the widget tree
///
/// Walks the tree using `first_child()` / `next_sibling()` which works
/// regardless of internal `adw::ApplicationWindow` wrapper widgets.
fn find_toast_overlay(widget: &gui::Widget) -> Option<adw::ToastOverlay> {
    if let Some(overlay) = widget.downcast_ref::<adw::ToastOverlay>() {
        return Some(overlay.clone());
    }

    // Walk children: GTK4 uses first_child / next_sibling linked list
    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(found) = find_toast_overlay(&c) {
            return Some(found);
        }
        child = c.next_sibling();
    }

    None
}

/// Helper to show an Undo toast on a window
pub fn show_undo_toast_on_window(
    window: &impl IsA<gui::Window>,
    message: &str,
    action_target: &str,
) {
    if let Some(child) = window.child()
        && let Some(overlay) = find_toast_overlay(&child)
    {
        let toast = adw::Toast::new(message);
        toast.set_button_label(Some("Undo"));
        toast.set_action_name(Some("win.undo-delete"));
        toast.set_action_target_value(Some(&glib::Variant::from(action_target)));
        overlay.add_toast(toast);
    }
}
