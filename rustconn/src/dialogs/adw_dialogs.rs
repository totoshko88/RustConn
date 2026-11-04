//! Adwaita dialog and toast utilities
//!
//! This module provides helper functions for creating native-looking
//! toasts using libadwaita components.
//!
//! Note: For alert dialogs, use `adw::AlertDialog` via the `crate::alert` module.

use gtk4::prelude::*;
use libadwaita as adw;

use crate::toast::plain_toast;

/// Shows a toast notification in an AdwToastOverlay
pub fn show_toast(overlay: &adw::ToastOverlay, message: &str) {
    show_toast_with_timeout(overlay, message, 3);
}

/// Shows a toast with custom timeout
pub fn show_toast_with_timeout(overlay: &adw::ToastOverlay, message: &str, timeout_secs: u32) {
    let toast = plain_toast(message);
    toast.set_timeout(timeout_secs);
    overlay.add_toast(toast);
}

/// Shows a toast with an action button
pub fn show_toast_with_action(
    overlay: &adw::ToastOverlay,
    message: &str,
    action_label: &str,
    action_name: &str,
) {
    let toast = plain_toast(message);
    toast.set_timeout(5);
    toast.set_button_label(Some(action_label));
    toast.set_action_name(Some(action_name));
    overlay.add_toast(toast);
}

/// Creates a new ToastOverlay wrapping the given child widget
pub fn create_toast_overlay(child: &impl IsA<gtk4::Widget>) -> adw::ToastOverlay {
    let overlay = adw::ToastOverlay::new();
    overlay.set_child(Some(child));
    overlay
}
