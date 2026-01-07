//! Password prompt dialog for connection authentication
//!
//! Provides a simple dialog for entering credentials when connecting
//! to RDP/VNC sessions that require authentication.

use adw::prelude::*;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, CheckButton, Entry, Grid, Label, Orientation, Spinner};
use libadwaita as adw;
use rustconn_core::CancellationToken;
use std::cell::RefCell;
use std::rc::Rc;

/// Result from password dialog
#[derive(Debug, Clone)]
pub struct PasswordDialogResult {
    /// Username (may be updated by user)
    pub username: String,
    /// Password entered by user
    pub password: String,
    /// Domain for Windows authentication
    pub domain: String,
    /// Whether to save credentials
    pub save_credentials: bool,
    /// Whether the user requested migration to KeePass
    pub migrate_to_keepass: bool,
}

/// Password prompt dialog
#[allow(dead_code)] // Fields kept for GTK widget lifecycle
pub struct PasswordDialog {
    window: adw::Window,
    username_entry: Entry,
    password_entry: Entry,
    domain_entry: Entry,
    save_check: CheckButton,
    migrate_button: Button,
    connect_button: Button,
    spinner: Spinner,
    spinner_label: Label,
    spinner_box: GtkBox,
    result: Rc<RefCell<Option<PasswordDialogResult>>>,
    migrate_requested: Rc<RefCell<bool>>,
    /// Cancellation token for pending async operations
    cancel_token: Rc<RefCell<Option<CancellationToken>>>,
}

impl PasswordDialog {
    /// Creates a new password dialog
    #[must_use]
    pub fn new(parent: Option<&impl IsA<gtk4::Window>>) -> Self {
        let window = adw::Window::builder()
            .title("Authentication Required")
            .modal(true)
            .default_width(400)
            .resizable(false)
            .build();

        if let Some(p) = parent {
            window.set_transient_for(Some(p));
        }

        // Header bar with Cancel/Connect buttons (GNOME HIG)
        let header = adw::HeaderBar::new();
        header.set_show_end_title_buttons(false);
        header.set_show_start_title_buttons(false);
        let cancel_btn = Button::builder().label("Cancel").build();
        let connect_btn = Button::builder()
            .label("Connect")
            .css_classes(["suggested-action"])
            .build();
        header.pack_start(&cancel_btn);
        header.pack_end(&connect_btn);

        // Content
        let content = GtkBox::new(Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // Use ToolbarView for adw::Window
        let main_box = GtkBox::new(Orientation::Vertical, 0);
        main_box.append(&header);
        main_box.append(&content);
        window.set_content(Some(&main_box));

        // Info label
        let info_label = Label::builder()
            .label("Enter credentials for this connection:")
            .halign(gtk4::Align::Start)
            .css_classes(["dim-label"])
            .build();
        content.append(&info_label);

        // Loading indicator box (hidden by default)
        let spinner_box = GtkBox::new(Orientation::Horizontal, 8);
        spinner_box.set_halign(gtk4::Align::Center);
        spinner_box.set_visible(false);

        let spinner = Spinner::builder().spinning(false).build();
        let spinner_label = Label::builder()
            .label("Resolving credentials...")
            .css_classes(["dim-label"])
            .build();
        spinner_box.append(&spinner);
        spinner_box.append(&spinner_label);
        content.append(&spinner_box);

        // Grid for fields
        let grid = Grid::builder().row_spacing(8).column_spacing(12).build();
        content.append(&grid);

        let (domain_entry, username_entry, password_entry, save_check, migrate_button) =
            Self::build_form_fields(&grid);

        let result: Rc<RefCell<Option<PasswordDialogResult>>> = Rc::new(RefCell::new(None));
        let migrate_requested: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));

        Self::connect_signals(
            &window,
            &cancel_btn,
            &connect_btn,
            &migrate_button,
            &username_entry,
            &password_entry,
            &domain_entry,
            &save_check,
            &result,
            &migrate_requested,
        );

        Self {
            window,
            username_entry,
            password_entry,
            domain_entry,
            save_check,
            migrate_button,
            connect_button: connect_btn,
            spinner,
            spinner_label,
            spinner_box,
            result,
            migrate_requested,
            cancel_token: Rc::new(RefCell::new(None)),
        }
    }

    fn build_form_fields(grid: &Grid) -> (Entry, Entry, Entry, CheckButton, Button) {
        let mut row = 0;

        // Domain
        let domain_label = Label::builder()
            .label("Domain:")
            .halign(gtk4::Align::End)
            .build();
        let domain_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("(optional)")
            .build();
        grid.attach(&domain_label, 0, row, 1, 1);
        grid.attach(&domain_entry, 1, row, 1, 1);
        row += 1;

        // Username
        let username_label = Label::builder()
            .label("Username:")
            .halign(gtk4::Align::End)
            .build();
        let username_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("username")
            .build();
        grid.attach(&username_label, 0, row, 1, 1);
        grid.attach(&username_entry, 1, row, 1, 1);
        row += 1;

        // Password
        let password_label = Label::builder()
            .label("Password:")
            .halign(gtk4::Align::End)
            .build();
        let password_entry = Entry::builder()
            .hexpand(true)
            .visibility(false)
            .input_purpose(gtk4::InputPurpose::Password)
            .placeholder_text("password")
            .build();
        grid.attach(&password_label, 0, row, 1, 1);
        grid.attach(&password_entry, 1, row, 1, 1);
        row += 1;

        // Save credentials checkbox
        let save_check = CheckButton::builder().label("Save Credentials").build();
        grid.attach(&save_check, 1, row, 1, 1);
        row += 1;

        // Save to KeePass button (hidden by default)
        let migrate_button = Button::builder()
            .label("Save to KeePass")
            .tooltip_text("Migrate credentials from system keyring to KeePass")
            .visible(false)
            .build();
        grid.attach(&migrate_button, 1, row, 1, 1);

        (
            domain_entry,
            username_entry,
            password_entry,
            save_check,
            migrate_button,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn connect_signals(
        window: &adw::Window,
        cancel_btn: &Button,
        connect_btn: &Button,
        migrate_button: &Button,
        username_entry: &Entry,
        password_entry: &Entry,
        domain_entry: &Entry,
        save_check: &CheckButton,
        result: &Rc<RefCell<Option<PasswordDialogResult>>>,
        migrate_requested: &Rc<RefCell<bool>>,
    ) {
        // Connect cancel
        let window_clone = window.clone();
        cancel_btn.connect_clicked(move |_| {
            window_clone.close();
        });

        // Connect migrate button
        let migrate_requested_clone = migrate_requested.clone();
        migrate_button.connect_clicked(move |_| {
            *migrate_requested_clone.borrow_mut() = true;
        });

        // Connect connect button
        let window_clone = window.clone();
        let username_clone = username_entry.clone();
        let password_clone = password_entry.clone();
        let domain_clone = domain_entry.clone();
        let save_clone = save_check.clone();
        let result_clone = result.clone();
        let migrate_requested_clone = migrate_requested.clone();
        let connect_btn_clone = connect_btn.clone();
        connect_btn_clone.connect_clicked(move |_| {
            *result_clone.borrow_mut() = Some(PasswordDialogResult {
                username: username_clone.text().to_string(),
                password: password_clone.text().to_string(),
                domain: domain_clone.text().to_string(),
                save_credentials: save_clone.is_active(),
                migrate_to_keepass: *migrate_requested_clone.borrow(),
            });
            window_clone.close();
        });

        // Connect Enter key in password field
        let connect_btn_for_enter = connect_btn.clone();
        password_entry.connect_activate(move |_| {
            connect_btn_for_enter.emit_clicked();
        });
    }

    /// Sets the initial username
    pub fn set_username(&self, username: &str) {
        self.username_entry.set_text(username);
    }

    /// Sets the initial domain
    pub fn set_domain(&self, domain: &str) {
        self.domain_entry.set_text(domain);
    }

    /// Sets the initial password
    pub fn set_password(&self, password: &str) {
        self.password_entry.set_text(password);
    }

    /// Gets a reference to the password entry widget
    ///
    /// This is useful for async operations that need to update the password field.
    #[must_use]
    pub fn password_entry(&self) -> &Entry {
        &self.password_entry
    }

    /// Sets the connection name in the title
    pub fn set_connection_name(&self, name: &str) {
        self.window.set_title(Some(&format!("Connect to {name}")));
    }

    /// Shows or hides the "Save to KeePass" migration button
    ///
    /// This button should be shown when:
    /// - KeePass integration is enabled
    /// - Credentials exist in Keyring but not in KeePass
    ///
    /// # Requirements Coverage
    /// - Requirement 3.3: Display "Save to KeePass" button when migration is needed
    pub fn set_show_migrate_button(&self, show: bool) {
        self.migrate_button.set_visible(show);
    }

    /// Pre-fills the dialog fields from connection settings
    ///
    /// This method populates the username and domain fields from the
    /// connection's saved settings, allowing users to only enter the password.
    ///
    /// # Requirements Coverage
    /// - Requirement 2.4: Pre-fill username and domain from saved connection settings
    ///
    /// # Arguments
    /// * `username` - Optional username from connection settings
    /// * `domain` - Optional domain from connection settings
    pub fn prefill_from_connection(&self, username: Option<&str>, domain: Option<&str>) {
        if let Some(user) = username {
            self.username_entry.set_text(user);
        }
        if let Some(dom) = domain {
            self.domain_entry.set_text(dom);
        }
    }

    /// Shows the dialog and calls callback with result
    pub fn show<F: Fn(Option<PasswordDialogResult>) + 'static>(&self, callback: F) {
        let result = self.result.clone();
        let callback = Rc::new(callback);

        self.window.connect_close_request(move |_| {
            let res = result.borrow().clone();
            callback(res);
            glib::Propagation::Proceed
        });

        self.window.present();

        // Focus password field if username is set
        if self.username_entry.text().is_empty() {
            self.username_entry.grab_focus();
        } else {
            self.password_entry.grab_focus();
        }
    }

    /// Returns the window widget
    #[must_use]
    pub const fn window(&self) -> &adw::Window {
        &self.window
    }

    /// Shows the loading indicator during async credential resolution
    ///
    /// This method displays a spinner and message while credentials are being
    /// resolved asynchronously, preventing UI freezing.
    ///
    /// # Requirements Coverage
    /// - Requirement 9.3: Show loading indicator during async resolution
    pub fn show_loading(&self, message: Option<&str>) {
        let msg = message.unwrap_or("Resolving credentials...");
        self.spinner_label.set_text(msg);
        self.spinner.set_spinning(true);
        self.spinner_box.set_visible(true);
        self.connect_button.set_sensitive(false);
    }

    /// Hides the loading indicator
    ///
    /// This method hides the spinner and re-enables the connect button
    /// after async credential resolution completes.
    ///
    /// # Requirements Coverage
    /// - Requirement 9.3: Hide loading indicator when resolution completes
    pub fn hide_loading(&self) {
        self.spinner.set_spinning(false);
        self.spinner_box.set_visible(false);
        self.connect_button.set_sensitive(true);
    }

    /// Shows an error message without freezing the UI
    ///
    /// This method displays an error message in the loading area
    /// when credential resolution fails.
    ///
    /// # Requirements Coverage
    /// - Requirement 9.4: Display error message without freezing UI
    pub fn show_error(&self, message: &str) {
        self.spinner.set_spinning(false);
        self.spinner_label.set_text(message);
        self.spinner_label.add_css_class("error");
        self.spinner_box.set_visible(true);
        self.connect_button.set_sensitive(true);
    }

    /// Clears any error message
    pub fn clear_error(&self) {
        self.spinner_label.remove_css_class("error");
        self.spinner_box.set_visible(false);
    }

    /// Returns a reference to the connect button for external control
    #[must_use]
    pub const fn connect_button(&self) -> &Button {
        &self.connect_button
    }

    /// Sets the cancellation token for pending async operations
    ///
    /// When the dialog is closed, this token will be cancelled to stop
    /// any pending credential resolution operations.
    ///
    /// # Requirements Coverage
    /// - Requirement 9.5: Support cancellation of pending requests
    pub fn set_cancel_token(&self, token: CancellationToken) {
        *self.cancel_token.borrow_mut() = Some(token);
    }

    /// Cancels any pending async operations
    ///
    /// This method should be called when the dialog is closed to cancel
    /// any pending credential resolution operations.
    ///
    /// # Requirements Coverage
    /// - Requirement 9.5: Cancel on dialog close
    pub fn cancel_pending_operations(&self) {
        if let Some(token) = self.cancel_token.borrow().as_ref() {
            token.cancel();
        }
    }

    /// Clears the cancellation token
    pub fn clear_cancel_token(&self) {
        *self.cancel_token.borrow_mut() = None;
    }

    /// Shows the dialog with cancellation support
    ///
    /// This method shows the dialog and automatically cancels any pending
    /// operations when the dialog is closed.
    ///
    /// # Requirements Coverage
    /// - Requirement 9.5: Cancel on dialog close
    pub fn show_with_cancellation<F: Fn(Option<PasswordDialogResult>) + 'static>(
        &self,
        callback: F,
    ) {
        let result = self.result.clone();
        let cancel_token = self.cancel_token.clone();
        let callback = Rc::new(callback);

        self.window.connect_close_request(move |_| {
            // Cancel any pending operations when dialog closes
            if let Some(token) = cancel_token.borrow().as_ref() {
                token.cancel();
            }

            let res = result.borrow().clone();
            callback(res);
            glib::Propagation::Proceed
        });

        self.window.present();

        // Focus password field if username is set
        if self.username_entry.text().is_empty() {
            self.username_entry.grab_focus();
        } else {
            self.password_entry.grab_focus();
        }
    }
}
