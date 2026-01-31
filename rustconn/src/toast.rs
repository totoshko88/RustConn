//! Toast notification system for non-blocking user feedback
//!
//! Provides a simple toast notification system that displays transient messages
//! without blocking user interaction. Uses GTK4 Revealer for smooth animations.
//!
//! # Usage
//!
//! ```rust,ignore
//! use crate::toast::ToastOverlay;
//!
//! let overlay = ToastOverlay::new();
//! overlay.show_toast("Connection copied to clipboard");
//! overlay.show_error("Failed to save settings");
//! ```

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Label, Orientation, Overlay, Revealer, RevealerTransitionType};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

/// Default duration for toast messages in milliseconds
const DEFAULT_TOAST_DURATION_MS: u32 = 3000;

/// Maximum number of queued toasts
const MAX_QUEUED_TOASTS: usize = 5;

/// Toast message types for styling
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastType {
    /// Informational message (default)
    Info,
    /// Success message (green tint)
    Success,
    /// Warning message (yellow tint)
    Warning,
    /// Error message (red tint)
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
}

/// A queued toast message
#[derive(Debug, Clone)]
struct QueuedToast {
    message: String,
    toast_type: ToastType,
    duration_ms: u32,
}

/// Toast overlay widget that can display non-blocking notifications
///
/// This widget wraps content and provides an overlay area for toast messages.
/// Toasts are displayed at the bottom of the overlay and auto-dismiss after
/// a configurable duration.
pub struct ToastOverlay {
    /// The overlay container
    overlay: Overlay,
    /// The revealer for animation
    revealer: Revealer,
    /// The toast label
    toast_label: Label,
    /// The toast container box
    toast_box: GtkBox,
    /// Queue of pending toasts
    queue: Rc<RefCell<VecDeque<QueuedToast>>>,
    /// Whether a toast is currently showing
    is_showing: Rc<RefCell<bool>>,
}

impl ToastOverlay {
    /// Creates a new toast overlay
    #[must_use]
    pub fn new() -> Self {
        let overlay = Overlay::new();

        // Create toast container
        let toast_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .halign(Align::Center)
            .valign(Align::End)
            .margin_bottom(24)
            .margin_start(24)
            .margin_end(24)
            .css_classes(["toast-container"])
            .build();

        // Create toast label
        let toast_label = Label::builder()
            .wrap(true)
            .max_width_chars(60)
            .css_classes(["toast-label"])
            .build();

        toast_box.append(&toast_label);

        // Create revealer for animation
        let revealer = Revealer::builder()
            .transition_type(RevealerTransitionType::SlideUp)
            .transition_duration(200)
            .child(&toast_box)
            .halign(Align::Center)
            .valign(Align::End)
            .build();

        overlay.add_overlay(&revealer);

        Self {
            overlay,
            revealer,
            toast_label,
            toast_box,
            queue: Rc::new(RefCell::new(VecDeque::new())),
            is_showing: Rc::new(RefCell::new(false)),
        }
    }

    /// Returns the overlay widget to add to the UI
    #[must_use]
    pub fn widget(&self) -> &Overlay {
        &self.overlay
    }

    /// Sets the main content of the overlay
    pub fn set_child(&self, child: Option<&impl IsA<gtk4::Widget>>) {
        self.overlay.set_child(child);
    }

    /// Shows a toast message with default duration
    pub fn show_toast(&self, message: &str) {
        self.show_toast_with_type(message, ToastType::Info);
    }

    /// Shows a success toast message
    pub fn show_success(&self, message: &str) {
        self.show_toast_with_type(message, ToastType::Success);
    }

    /// Shows a warning toast message
    pub fn show_warning(&self, message: &str) {
        self.show_toast_with_type(message, ToastType::Warning);
    }

    /// Shows an error toast message
    pub fn show_error(&self, message: &str) {
        self.show_toast_with_type(message, ToastType::Error);
    }

    /// Shows a toast message with a specific type
    pub fn show_toast_with_type(&self, message: &str, toast_type: ToastType) {
        self.show_toast_with_duration(message, toast_type, DEFAULT_TOAST_DURATION_MS);
    }

    /// Shows a toast message with custom duration
    pub fn show_toast_with_duration(&self, message: &str, toast_type: ToastType, duration_ms: u32) {
        let toast = QueuedToast {
            message: message.to_string(),
            toast_type,
            duration_ms,
        };

        // Add to queue (limit queue size)
        {
            let mut queue = self.queue.borrow_mut();
            if queue.len() >= MAX_QUEUED_TOASTS {
                queue.pop_front();
            }
            queue.push_back(toast);
        }

        // Try to show next toast
        self.try_show_next();
    }

    /// Attempts to show the next toast in the queue
    fn try_show_next(&self) {
        // Check if already showing
        if *self.is_showing.borrow() {
            return;
        }

        // Get next toast from queue
        let toast = {
            let mut queue = self.queue.borrow_mut();
            queue.pop_front()
        };

        let Some(toast) = toast else {
            return;
        };

        // Mark as showing
        *self.is_showing.borrow_mut() = true;

        // Update toast appearance
        self.toast_label.set_text(&toast.message);

        // Update CSS classes for toast type
        self.toast_box.remove_css_class("toast-info");
        self.toast_box.remove_css_class("toast-success");
        self.toast_box.remove_css_class("toast-warning");
        self.toast_box.remove_css_class("toast-error");
        self.toast_box.add_css_class(toast.toast_type.css_class());

        // Show the toast
        self.revealer.set_reveal_child(true);

        // Schedule hide - clone all needed components for the closure
        let revealer = self.revealer.clone();
        let is_showing = self.is_showing.clone();
        let queue = self.queue.clone();
        let toast_label = self.toast_label.clone();
        let toast_box = self.toast_box.clone();

        glib::timeout_add_local_once(
            std::time::Duration::from_millis(u64::from(toast.duration_ms)),
            move || {
                revealer.set_reveal_child(false);

                // Wait for animation to complete before showing next
                let is_showing_inner = is_showing.clone();
                let queue_inner = queue.clone();
                let revealer_inner = revealer.clone();
                let toast_label_inner = toast_label.clone();
                let toast_box_inner = toast_box.clone();

                glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || {
                    *is_showing_inner.borrow_mut() = false;

                    // Try to show next toast if any
                    let next_toast = queue_inner.borrow_mut().pop_front();
                    if let Some(next) = next_toast {
                        // Mark as showing
                        *is_showing_inner.borrow_mut() = true;

                        // Update toast appearance
                        toast_label_inner.set_text(&next.message);

                        // Update CSS classes for toast type
                        toast_box_inner.remove_css_class("toast-info");
                        toast_box_inner.remove_css_class("toast-success");
                        toast_box_inner.remove_css_class("toast-warning");
                        toast_box_inner.remove_css_class("toast-error");
                        toast_box_inner.add_css_class(next.toast_type.css_class());

                        // Show the toast
                        revealer_inner.set_reveal_child(true);

                        // Schedule hide for this toast (recursive via another timeout)
                        Self::schedule_toast_hide(
                            revealer_inner,
                            is_showing_inner,
                            queue_inner,
                            toast_label_inner,
                            toast_box_inner,
                            next.duration_ms,
                        );
                    }
                });
            },
        );
    }

    /// Schedules hiding a toast and showing the next one from queue
    ///
    /// This is a helper to avoid code duplication in the recursive toast display logic.
    fn schedule_toast_hide(
        revealer: Revealer,
        is_showing: Rc<RefCell<bool>>,
        queue: Rc<RefCell<VecDeque<QueuedToast>>>,
        toast_label: Label,
        toast_box: GtkBox,
        duration_ms: u32,
    ) {
        glib::timeout_add_local_once(
            std::time::Duration::from_millis(u64::from(duration_ms)),
            move || {
                revealer.set_reveal_child(false);

                // Wait for animation to complete before showing next
                let is_showing_inner = is_showing.clone();
                let queue_inner = queue.clone();
                let revealer_inner = revealer.clone();
                let toast_label_inner = toast_label.clone();
                let toast_box_inner = toast_box.clone();

                glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || {
                    *is_showing_inner.borrow_mut() = false;

                    // Try to show next toast if any
                    let next_toast = queue_inner.borrow_mut().pop_front();
                    if let Some(next) = next_toast {
                        // Mark as showing
                        *is_showing_inner.borrow_mut() = true;

                        // Update toast appearance
                        toast_label_inner.set_text(&next.message);

                        // Update CSS classes for toast type
                        toast_box_inner.remove_css_class("toast-info");
                        toast_box_inner.remove_css_class("toast-success");
                        toast_box_inner.remove_css_class("toast-warning");
                        toast_box_inner.remove_css_class("toast-error");
                        toast_box_inner.add_css_class(next.toast_type.css_class());

                        // Show the toast
                        revealer_inner.set_reveal_child(true);

                        // Schedule hide for this toast (recursive)
                        Self::schedule_toast_hide(
                            revealer_inner,
                            is_showing_inner,
                            queue_inner,
                            toast_label_inner,
                            toast_box_inner,
                            next.duration_ms,
                        );
                    }
                });
            },
        );
    }

    /// Immediately hides any showing toast
    pub fn hide(&self) {
        self.revealer.set_reveal_child(false);
        *self.is_showing.borrow_mut() = false;
    }

    /// Clears all queued toasts
    pub fn clear_queue(&self) {
        self.queue.borrow_mut().clear();
    }
}

impl Default for ToastOverlay {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper function to show a toast on a window
///
/// This creates a temporary toast that appears at the bottom of the window.
/// For persistent toast support, use `ToastOverlay` directly.
pub fn show_toast_on_window(window: &impl IsA<gtk4::Window>, message: &str, toast_type: ToastType) {
    // Create a floating toast label
    let toast_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::Center)
        .css_classes(["toast-container", toast_type.css_class()])
        .build();

    let toast_label = Label::builder()
        .label(message)
        .wrap(true)
        .max_width_chars(60)
        .css_classes(["toast-label"])
        .build();

    toast_box.append(&toast_label);

    // Create revealer
    let revealer = Revealer::builder()
        .transition_type(RevealerTransitionType::SlideUp)
        .transition_duration(200)
        .child(&toast_box)
        .halign(Align::Center)
        .valign(Align::End)
        .margin_bottom(24)
        .build();

    // Try to add to window's overlay if it has one
    if let Some(child) = window.child() {
        if let Some(overlay) = child.downcast_ref::<Overlay>() {
            overlay.add_overlay(&revealer);
            revealer.set_reveal_child(true);

            // Schedule removal
            let overlay_clone = overlay.clone();
            let revealer_clone = revealer.clone();
            glib::timeout_add_local_once(
                std::time::Duration::from_millis(DEFAULT_TOAST_DURATION_MS.into()),
                move || {
                    revealer_clone.set_reveal_child(false);

                    let overlay_inner = overlay_clone.clone();
                    let revealer_inner = revealer_clone.clone();
                    glib::timeout_add_local_once(
                        std::time::Duration::from_millis(250),
                        move || {
                            overlay_inner.remove_overlay(&revealer_inner);
                        },
                    );
                },
            );
        }
    }
}

/// CSS styles for toast notifications
///
/// Include this in your application's CSS to style toasts.
pub const TOAST_CSS: &str = r"
.toast-container {
    background-color: alpha(@theme_bg_color, 0.95);
    border-radius: 8px;
    padding: 12px 16px;
    box-shadow: 0 2px 8px alpha(black, 0.3);
    border: 1px solid alpha(@borders, 0.5);
}

.toast-label {
    font-weight: 500;
}

.toast-info {
    border-left: 4px solid @accent_bg_color;
}

.toast-success {
    border-left: 4px solid @success_color;
    background-color: alpha(@success_bg_color, 0.95);
}

.toast-warning {
    border-left: 4px solid @warning_color;
    background-color: alpha(@warning_bg_color, 0.95);
}

.toast-error {
    border-left: 4px solid @error_color;
    background-color: alpha(@error_bg_color, 0.95);
}
";
