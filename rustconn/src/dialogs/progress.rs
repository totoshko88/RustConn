//! Progress dialog for long-running operations
//!
//! Provides a GTK4 dialog for displaying progress during operations like
//! imports, exports, and bulk operations.

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, Label, Orientation, ProgressBar};
use libadwaita as adw;
use std::cell::Cell;
use std::rc::Rc;

use crate::i18n::i18n;

/// Progress dialog for displaying operation progress
pub struct ProgressDialog {
    dialog: adw::Dialog,
    progress_bar: ProgressBar,
    status_label: Label,
    cancel_button: Button,
    cancelled: Rc<Cell<bool>>,
    parent: Option<gtk4::Widget>,
}

impl ProgressDialog {
    /// Creates a new progress dialog
    ///
    /// # Arguments
    ///
    /// * `parent` - Optional parent window for modal behavior
    /// * `title` - Title of the progress dialog
    /// * `cancellable` - Whether to show a cancel button
    #[must_use]
    pub fn new(parent: Option<&gtk4::Window>, title: &str, cancellable: bool) -> Self {
        let dialog = adw::Dialog::builder()
            .title(title)
            .content_width(400)
            .can_close(false)
            .build();

        // Create main content area
        let content = GtkBox::new(Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // Status label
        let status_label = Label::builder()
            .label(i18n("Starting..."))
            .halign(gtk4::Align::Start)
            .wrap(true)
            .max_width_chars(50)
            .build();
        content.append(&status_label);

        // Progress bar
        let progress_bar = ProgressBar::builder().show_text(true).hexpand(true).build();
        content.append(&progress_bar);

        // Cancel button (optional)
        let cancelled = Rc::new(Cell::new(false));
        let cancel_button = Button::builder()
            .label(i18n("Cancel"))
            .halign(gtk4::Align::Center)
            .margin_top(12)
            .build();

        if cancellable {
            content.append(&cancel_button);

            // Connect cancel button
            let cancelled_clone = Rc::clone(&cancelled);
            let cancel_btn_clone = cancel_button.clone();
            cancel_button.connect_clicked(move |_| {
                cancelled_clone.set(true);
                cancel_btn_clone.set_sensitive(false);
                cancel_btn_clone.set_label(&i18n("Cancelling..."));
            });
        }

        let header = adw::HeaderBar::new();
        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        let clamp = adw::Clamp::builder()
            .maximum_size(600)
            .child(&content)
            .build();
        toolbar_view.set_content(Some(&clamp));
        dialog.set_child(Some(&toolbar_view));

        let stored_parent: Option<gtk4::Widget> =
            parent.map(|p| p.clone().upcast::<gtk4::Widget>());

        Self {
            dialog,
            progress_bar,
            status_label,
            cancel_button,
            cancelled,
            parent: stored_parent,
        }
    }

    /// Updates the progress display
    ///
    /// # Arguments
    ///
    /// * `fraction` - Progress fraction (0.0 to 1.0)
    /// * `message` - Status message to display
    pub fn update(&self, fraction: f64, message: &str) {
        self.progress_bar.set_fraction(fraction.clamp(0.0, 1.0));
        self.progress_bar
            .set_text(Some(&format!("{:.0}%", fraction * 100.0)));
        self.status_label.set_text(message);
    }

    /// Updates the progress display with item counts
    ///
    /// # Arguments
    ///
    /// * `current` - Current item number
    /// * `total` - Total number of items
    /// * `message` - Status message to display
    pub fn update_with_count(&self, current: usize, total: usize, message: &str) {
        let fraction = if total > 0 {
            current as f64 / total as f64
        } else {
            0.0
        };
        self.progress_bar.set_fraction(fraction);
        self.progress_bar
            .set_text(Some(&format!("{current}/{total}")));
        self.status_label.set_text(message);
    }

    /// Returns true if the operation was cancelled
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.get()
    }

    /// Shows the progress dialog
    pub fn show(&self) {
        self.dialog
            .present(self.parent.as_ref().map(|w| w as &gtk4::Widget));
    }

    /// Closes the progress dialog
    pub fn close(&self) {
        self.dialog.set_can_close(true);
        self.dialog.close();
    }

    /// Returns a reference to the underlying dialog
    #[must_use]
    pub const fn dialog(&self) -> &adw::Dialog {
        &self.dialog
    }

    /// Sets the progress to indeterminate mode (pulsing)
    pub fn set_indeterminate(&self, indeterminate: bool) {
        if indeterminate {
            self.progress_bar.pulse();
        } else {
            self.progress_bar.set_fraction(0.0);
        }
    }

    /// Pulses the progress bar (for indeterminate progress)
    pub fn pulse(&self) {
        self.progress_bar.pulse();
    }

    /// Sets the cancel button sensitivity
    pub fn set_cancellable(&self, cancellable: bool) {
        self.cancel_button.set_sensitive(cancellable);
    }

    /// Resets the cancelled state
    pub fn reset_cancelled(&self) {
        self.cancelled.set(false);
        self.cancel_button.set_sensitive(true);
        self.cancel_button.set_label(&i18n("Cancel"));
    }
}
