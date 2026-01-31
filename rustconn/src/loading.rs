//! Loading overlay component for long-running operations
//!
//! Provides a reusable loading overlay that can be shown during async operations
//! like imports, exports, connection tests, etc.

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Label, Orientation, Overlay, Spinner};
use libadwaita as adw;
use libadwaita::prelude::AdwWindowExt;
use std::cell::RefCell;
use std::rc::Rc;

/// Loading overlay that can be shown over any widget
///
/// Uses `adw::StatusPage` for consistent GNOME HIG styling.
pub struct LoadingOverlay {
    /// The overlay container
    overlay: Overlay,
    /// The loading status page
    status_page: adw::StatusPage,
    /// The spinner widget
    spinner: Spinner,
    /// Progress label (optional)
    progress_label: Label,
    /// Whether the overlay is currently visible
    is_visible: Rc<RefCell<bool>>,
    /// Cancellation callback
    on_cancel: Rc<RefCell<Option<Box<dyn Fn() + 'static>>>>,
}

impl LoadingOverlay {
    /// Creates a new loading overlay
    #[must_use]
    pub fn new() -> Self {
        let overlay = Overlay::new();

        // Create the loading container with semi-transparent background
        let loading_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .halign(Align::Fill)
            .valign(Align::Fill)
            .css_classes(["loading-overlay-background"])
            .build();

        // Create status page for the loading content
        let status_page = adw::StatusPage::builder()
            .title("Loading...")
            .halign(Align::Center)
            .valign(Align::Center)
            .build();

        // Create spinner
        let spinner = Spinner::builder()
            .spinning(true)
            .halign(Align::Center)
            .width_request(48)
            .height_request(48)
            .build();

        // Progress label (hidden by default)
        let progress_label = Label::builder()
            .halign(Align::Center)
            .css_classes(["dim-label"])
            .visible(false)
            .build();

        // Container for spinner and progress
        let content_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(12)
            .halign(Align::Center)
            .build();
        content_box.append(&spinner);
        content_box.append(&progress_label);

        status_page.set_child(Some(&content_box));
        loading_box.append(&status_page);

        // Initially hidden
        loading_box.set_visible(false);

        overlay.add_overlay(&loading_box);

        Self {
            overlay,
            status_page,
            spinner,
            progress_label,
            is_visible: Rc::new(RefCell::new(false)),
            on_cancel: Rc::new(RefCell::new(None)),
        }
    }

    /// Returns the overlay widget
    #[must_use]
    pub fn widget(&self) -> &Overlay {
        &self.overlay
    }

    /// Sets the main content of the overlay
    pub fn set_child(&self, child: Option<&impl IsA<gtk4::Widget>>) {
        self.overlay.set_child(child);
    }

    /// Shows the loading overlay with a message
    pub fn show(&self, message: &str) {
        self.status_page.set_title(message);
        self.progress_label.set_visible(false);
        self.spinner.set_spinning(true);

        // Get the overlay widget and show it
        if let Some(loading_box) = self.get_loading_box() {
            loading_box.set_visible(true);
        }
        *self.is_visible.borrow_mut() = true;
    }

    /// Shows the loading overlay with a message and description
    pub fn show_with_description(&self, title: &str, description: &str) {
        self.status_page.set_title(title);
        self.status_page.set_description(Some(description));
        self.progress_label.set_visible(false);
        self.spinner.set_spinning(true);

        if let Some(loading_box) = self.get_loading_box() {
            loading_box.set_visible(true);
        }
        *self.is_visible.borrow_mut() = true;
    }

    /// Updates the progress text
    pub fn set_progress(&self, progress_text: &str) {
        self.progress_label.set_text(progress_text);
        self.progress_label.set_visible(true);
    }

    /// Updates the title
    pub fn set_title(&self, title: &str) {
        self.status_page.set_title(title);
    }

    /// Hides the loading overlay
    pub fn hide(&self) {
        if let Some(loading_box) = self.get_loading_box() {
            loading_box.set_visible(false);
        }
        self.spinner.set_spinning(false);
        self.status_page.set_description(None);
        *self.is_visible.borrow_mut() = false;
    }

    /// Returns whether the overlay is currently visible
    #[must_use]
    pub fn is_visible(&self) -> bool {
        *self.is_visible.borrow()
    }

    /// Sets a cancellation callback
    pub fn set_on_cancel<F: Fn() + 'static>(&self, callback: F) {
        *self.on_cancel.borrow_mut() = Some(Box::new(callback));
    }

    /// Gets the loading box from the overlay
    fn get_loading_box(&self) -> Option<GtkBox> {
        // The loading box is the first overlay child
        let mut child = self.overlay.first_child();
        while let Some(widget) = child {
            if widget
                .css_classes()
                .iter()
                .any(|c| c == "loading-overlay-background")
            {
                return widget.downcast::<GtkBox>().ok();
            }
            child = widget.next_sibling();
        }
        None
    }
}

impl Default for LoadingOverlay {
    fn default() -> Self {
        Self::new()
    }
}

/// Shows a loading dialog for modal operations
///
/// Returns a handle that can be used to update progress or close the dialog.
pub struct LoadingDialog {
    window: adw::Window,
    status_page: adw::StatusPage,
    spinner: Spinner,
    progress_label: Label,
    cancel_button: gtk4::Button,
    on_cancel: Rc<RefCell<Option<Box<dyn Fn() + 'static>>>>,
}

impl LoadingDialog {
    /// Creates a new loading dialog
    #[must_use]
    pub fn new(parent: Option<&impl IsA<gtk4::Window>>, title: &str) -> Self {
        let window = adw::Window::builder()
            .title(title)
            .modal(true)
            .default_width(400)
            .default_height(200)
            .resizable(false)
            .deletable(false)
            .build();

        if let Some(parent) = parent {
            window.set_transient_for(Some(parent));
        }

        let content = GtkBox::new(Orientation::Vertical, 0);

        let status_page = adw::StatusPage::builder()
            .title(title)
            .vexpand(true)
            .build();

        let spinner = Spinner::builder()
            .spinning(true)
            .halign(Align::Center)
            .width_request(48)
            .height_request(48)
            .build();

        let progress_label = Label::builder()
            .halign(Align::Center)
            .css_classes(["dim-label"])
            .visible(false)
            .margin_top(8)
            .build();

        let spinner_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(8)
            .halign(Align::Center)
            .build();
        spinner_box.append(&spinner);
        spinner_box.append(&progress_label);

        status_page.set_child(Some(&spinner_box));
        content.append(&status_page);

        // Cancel button (hidden by default)
        let cancel_button = gtk4::Button::builder()
            .label("Cancel")
            .halign(Align::Center)
            .margin_bottom(16)
            .visible(false)
            .build();
        content.append(&cancel_button);

        window.set_content(Some(&content));

        let on_cancel: Rc<RefCell<Option<Box<dyn Fn() + 'static>>>> = Rc::new(RefCell::new(None));

        // Connect cancel button
        let on_cancel_clone = on_cancel.clone();
        cancel_button.connect_clicked(move |_| {
            if let Some(ref callback) = *on_cancel_clone.borrow() {
                callback();
            }
        });

        Self {
            window,
            status_page,
            spinner,
            progress_label,
            cancel_button,
            on_cancel,
        }
    }

    /// Shows the dialog
    pub fn show(&self) {
        self.spinner.set_spinning(true);
        self.window.present();
    }

    /// Updates the title
    pub fn set_title(&self, title: &str) {
        self.status_page.set_title(title);
    }

    /// Updates the description
    pub fn set_description(&self, description: &str) {
        self.status_page.set_description(Some(description));
    }

    /// Updates the progress text
    pub fn set_progress(&self, progress_text: &str) {
        self.progress_label.set_text(progress_text);
        self.progress_label.set_visible(true);
    }

    /// Enables the cancel button with a callback
    pub fn enable_cancel<F: Fn() + 'static>(&self, callback: F) {
        *self.on_cancel.borrow_mut() = Some(Box::new(callback));
        self.cancel_button.set_visible(true);
    }

    /// Closes the dialog
    pub fn close(&self) {
        self.spinner.set_spinning(false);
        self.window.close();
    }

    /// Returns the window for use as a parent
    #[must_use]
    pub fn window(&self) -> &adw::Window {
        &self.window
    }
}

/// Helper to run an async operation with a loading dialog
///
/// Shows a loading dialog while the operation runs, then closes it.
pub fn with_loading_dialog<F, T>(
    parent: Option<&impl IsA<gtk4::Window>>,
    title: &str,
    operation: F,
) -> LoadingDialog
where
    F: FnOnce() -> T + 'static,
    T: 'static,
{
    let dialog = LoadingDialog::new(parent, title);
    dialog.show();

    // Schedule the operation to run after the dialog is shown
    let dialog_window = dialog.window.clone();
    glib::idle_add_local_once(move || {
        let _result = operation();
        dialog_window.close();
    });

    dialog
}

/// CSS styles for loading overlays
pub const LOADING_CSS: &str = r"
.loading-overlay-background {
    background-color: alpha(@window_bg_color, 0.85);
}
";
