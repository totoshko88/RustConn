//! Document management dialogs for `RustConn`
//!
//! Provides dialogs for creating, opening, saving, and managing documents.

use crate::alert::{self, SaveChangesResponse};
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, CheckButton, Entry, FileDialog, FileFilter, Label, Orientation,
    PasswordEntry,
};
use libadwaita as adw;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use uuid::Uuid;

/// Callback type for document dialog results
pub type DocumentCallback = Rc<RefCell<Option<Box<dyn Fn(Option<DocumentDialogResult>)>>>>;

/// Result from document dialog
#[derive(Debug, Clone)]
pub enum DocumentDialogResult {
    /// Create a new document
    Create {
        name: String,
        password: Option<String>,
    },
    /// Open an existing document
    Open {
        path: PathBuf,
        password: Option<String>,
    },
    /// Save document
    Save {
        id: Uuid,
        path: PathBuf,
        password: Option<String>,
    },
    /// Close document (with save prompt result)
    Close { id: Uuid, save: bool },
}

/// Dialog for creating a new document
pub struct NewDocumentDialog {
    window: adw::Window,
    name_entry: Entry,
    password_check: CheckButton,
    password_entry: PasswordEntry,
    confirm_entry: PasswordEntry,
    on_complete: DocumentCallback,
}

impl NewDocumentDialog {
    /// Creates a new document creation dialog
    #[must_use]
    pub fn new(parent: Option<&gtk4::Window>) -> Self {
        let window = adw::Window::builder()
            .title("New Document")
            .modal(true)
            .default_width(400)
            .build();

        if let Some(p) = parent {
            window.set_transient_for(Some(p));
        }

        window.set_size_request(280, -1);

        // Header bar with Cancel/Create buttons (GNOME HIG)
        let header = adw::HeaderBar::new();
        header.set_show_end_title_buttons(false);
        header.set_show_start_title_buttons(false);
        let cancel_btn = Button::builder().label("Cancel").build();
        let create_btn = Button::builder()
            .label("Create")
            .css_classes(["suggested-action"])
            .sensitive(false)
            .build();
        header.pack_start(&cancel_btn);
        header.pack_end(&create_btn);

        // Content
        let content = GtkBox::new(Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // Use ToolbarView for adw::Window
        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&content));
        window.set_content(Some(&toolbar_view));

        // Name field
        let name_label = Label::builder()
            .label("Document Name")
            .halign(gtk4::Align::Start)
            .build();
        content.append(&name_label);

        let name_entry = Entry::builder().placeholder_text("My Connections").build();
        content.append(&name_entry);

        // Password protection
        let password_check = CheckButton::builder()
            .label("Protect with password")
            .build();
        content.append(&password_check);

        // Password fields (initially hidden)
        let password_box = GtkBox::new(Orientation::Vertical, 8);
        password_box.set_margin_start(24);
        password_box.set_visible(false);

        let password_label = Label::builder()
            .label("Password")
            .halign(gtk4::Align::Start)
            .build();
        password_box.append(&password_label);

        let password_entry = PasswordEntry::builder().show_peek_icon(true).build();
        password_box.append(&password_entry);

        let confirm_label = Label::builder()
            .label("Confirm Password")
            .halign(gtk4::Align::Start)
            .build();
        password_box.append(&confirm_label);

        let confirm_entry = PasswordEntry::builder().show_peek_icon(true).build();
        password_box.append(&confirm_entry);

        content.append(&password_box);
        window.set_child(Some(&content));

        let on_complete: DocumentCallback = Rc::new(RefCell::new(None));

        // Toggle password fields visibility
        let password_box_clone = password_box.clone();
        password_check.connect_toggled(move |check| {
            password_box_clone.set_visible(check.is_active());
        });

        // Validate input and enable/disable create button
        let create_btn_clone = create_btn.clone();
        let name_entry_clone = name_entry.clone();
        let password_check_clone = password_check.clone();
        let password_entry_clone = password_entry.clone();
        let confirm_entry_clone = confirm_entry.clone();

        let validate = move || {
            let name_valid = !name_entry_clone.text().is_empty();
            let password_valid = if password_check_clone.is_active() {
                let pwd = password_entry_clone.text();
                let confirm = confirm_entry_clone.text();
                !pwd.is_empty() && pwd == confirm
            } else {
                true
            };
            create_btn_clone.set_sensitive(name_valid && password_valid);
        };

        let validate_clone = validate.clone();
        name_entry.connect_changed(move |_| validate_clone());

        let validate_clone = validate.clone();
        password_entry.connect_changed(move |_| validate_clone());

        let validate_clone = validate.clone();
        confirm_entry.connect_changed(move |_| validate_clone());

        let validate_clone = validate;
        password_check.connect_toggled(move |_| validate_clone());

        // Cancel button
        let window_clone = window.clone();
        let on_complete_clone = on_complete.clone();
        cancel_btn.connect_clicked(move |_| {
            if let Some(ref cb) = *on_complete_clone.borrow() {
                cb(None);
            }
            window_clone.close();
        });

        // Create button
        let window_clone = window.clone();
        let on_complete_clone = on_complete.clone();
        let name_entry_clone = name_entry.clone();
        let password_check_clone = password_check.clone();
        let password_entry_clone = password_entry.clone();
        create_btn.connect_clicked(move |_| {
            let name = name_entry_clone.text().to_string();
            let password = if password_check_clone.is_active() {
                Some(password_entry_clone.text().to_string())
            } else {
                None
            };

            if let Some(ref cb) = *on_complete_clone.borrow() {
                cb(Some(DocumentDialogResult::Create { name, password }));
            }
            window_clone.close();
        });

        Self {
            window,
            name_entry,
            password_check,
            password_entry,
            confirm_entry,
            on_complete,
        }
    }

    /// Sets the callback for when the dialog completes
    pub fn set_callback<F>(&self, callback: F)
    where
        F: Fn(Option<DocumentDialogResult>) + 'static,
    {
        *self.on_complete.borrow_mut() = Some(Box::new(callback));
    }

    /// Shows the dialog
    pub fn present(&self) {
        self.name_entry.set_text("");
        self.password_check.set_active(false);
        self.password_entry.set_text("");
        self.confirm_entry.set_text("");
        self.window.present();
    }
}

/// Dialog for opening a document with optional password
pub struct OpenDocumentDialog {
    on_complete: DocumentCallback,
}

impl OpenDocumentDialog {
    /// Creates a new open document dialog
    #[must_use]
    pub fn new() -> Self {
        Self {
            on_complete: Rc::new(RefCell::new(None)),
        }
    }

    /// Sets the callback for when the dialog completes
    pub fn set_callback<F>(&self, callback: F)
    where
        F: Fn(Option<DocumentDialogResult>) + 'static,
    {
        *self.on_complete.borrow_mut() = Some(Box::new(callback));
    }

    /// Shows the file chooser dialog
    pub fn present(&self, parent: Option<&gtk4::Window>) {
        let filter = FileFilter::new();
        filter.add_pattern("*.rcdb");
        filter.add_pattern("*.json");
        filter.add_pattern("*.yaml");
        filter.add_pattern("*.yml");
        filter.set_name(Some("RustConn Documents"));

        let filters = gtk4::gio::ListStore::new::<FileFilter>();
        filters.append(&filter);

        let dialog = FileDialog::builder()
            .title("Open Document")
            .filters(&filters)
            .modal(true)
            .build();

        let on_complete = self.on_complete.clone();
        let parent_clone = parent.cloned();

        dialog.open(parent, gtk4::gio::Cancellable::NONE, move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    // Check if file might be encrypted (by extension or content)
                    let needs_password = path.extension().is_some_and(|ext| ext == "rcdb");

                    if needs_password {
                        // Show password dialog
                        Self::show_password_dialog(
                            parent_clone.as_ref(),
                            path,
                            on_complete.clone(),
                        );
                    } else if let Some(ref cb) = *on_complete.borrow() {
                        cb(Some(DocumentDialogResult::Open {
                            path,
                            password: None,
                        }));
                    }
                }
            } else if let Some(ref cb) = *on_complete.borrow() {
                cb(None);
            }
        });
    }

    /// Shows a password dialog for encrypted documents
    fn show_password_dialog(
        parent: Option<&gtk4::Window>,
        path: PathBuf,
        on_complete: DocumentCallback,
    ) {
        let window = adw::Window::builder()
            .title("Enter Password")
            .modal(true)
            .default_width(350)
            .build();

        if let Some(p) = parent {
            window.set_transient_for(Some(p));
        }

        window.set_size_request(280, -1);

        let header = adw::HeaderBar::new();
        header.set_show_end_title_buttons(false);
        header.set_show_start_title_buttons(false);
        let cancel_btn = Button::builder().label("Cancel").build();
        let open_btn = Button::builder()
            .label("Open")
            .css_classes(["suggested-action"])
            .build();
        header.pack_start(&cancel_btn);
        header.pack_end(&open_btn);

        let content = GtkBox::new(Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        let label = Label::builder()
            .label("This document is password protected.\nEnter the password to open it.")
            .halign(gtk4::Align::Start)
            .build();
        content.append(&label);

        let password_entry = PasswordEntry::builder().show_peek_icon(true).build();
        content.append(&password_entry);

        // Use ToolbarView for adw::Window
        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&content));
        window.set_content(Some(&toolbar_view));

        // Cancel
        let window_clone = window.clone();
        let on_complete_clone = on_complete.clone();
        cancel_btn.connect_clicked(move |_| {
            if let Some(ref cb) = *on_complete_clone.borrow() {
                cb(None);
            }
            window_clone.close();
        });

        // Open
        let window_clone = window.clone();
        let path_clone = path;
        open_btn.connect_clicked(move |_| {
            let password = password_entry.text().to_string();
            if let Some(ref cb) = *on_complete.borrow() {
                cb(Some(DocumentDialogResult::Open {
                    path: path_clone.clone(),
                    password: Some(password),
                }));
            }
            window_clone.close();
        });

        window.present();
    }
}

impl Default for OpenDocumentDialog {
    fn default() -> Self {
        Self::new()
    }
}

/// Dialog for saving a document
pub struct SaveDocumentDialog {
    on_complete: DocumentCallback,
}

impl SaveDocumentDialog {
    /// Creates a new save document dialog
    #[must_use]
    pub fn new() -> Self {
        Self {
            on_complete: Rc::new(RefCell::new(None)),
        }
    }

    /// Sets the callback for when the dialog completes
    pub fn set_callback<F>(&self, callback: F)
    where
        F: Fn(Option<DocumentDialogResult>) + 'static,
    {
        *self.on_complete.borrow_mut() = Some(Box::new(callback));
    }

    /// Shows the file chooser dialog for saving
    pub fn present(&self, parent: Option<&gtk4::Window>, doc_id: Uuid, suggested_name: &str) {
        let filter = FileFilter::new();
        filter.add_pattern("*.rcdb");
        filter.set_name(Some("RustConn Documents"));

        let filters = gtk4::gio::ListStore::new::<FileFilter>();
        filters.append(&filter);

        let dialog = FileDialog::builder()
            .title("Save Document")
            .filters(&filters)
            .initial_name(format!("{suggested_name}.rcdb"))
            .modal(true)
            .build();

        let on_complete = self.on_complete.clone();

        dialog.save(parent, gtk4::gio::Cancellable::NONE, move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    if let Some(ref cb) = *on_complete.borrow() {
                        cb(Some(DocumentDialogResult::Save {
                            id: doc_id,
                            path,
                            password: None, // Password set separately if needed
                        }));
                    }
                }
            } else if let Some(ref cb) = *on_complete.borrow() {
                cb(None);
            }
        });
    }
}

impl Default for SaveDocumentDialog {
    fn default() -> Self {
        Self::new()
    }
}

/// Dialog for confirming document close with unsaved changes
pub struct CloseDocumentDialog {
    on_complete: DocumentCallback,
}

impl CloseDocumentDialog {
    /// Creates a new close document dialog
    #[must_use]
    pub fn new() -> Self {
        Self {
            on_complete: Rc::new(RefCell::new(None)),
        }
    }

    /// Sets the callback for when the dialog completes
    pub fn set_callback<F>(&self, callback: F)
    where
        F: Fn(Option<DocumentDialogResult>) + 'static,
    {
        *self.on_complete.borrow_mut() = Some(Box::new(callback));
    }

    /// Shows the confirmation dialog
    ///
    /// # Panics
    ///
    /// Panics if `parent` is `None`. The parent window is required for modal dialogs.
    pub fn present(&self, parent: Option<&gtk4::Window>, doc_id: Uuid, doc_name: &str) {
        let Some(parent_window) = parent else {
            tracing::error!("CloseDocumentDialog::present called without parent window");
            // Call callback with None to signal cancellation
            if let Some(ref cb) = *self.on_complete.borrow() {
                cb(None);
            }
            return;
        };

        let on_complete = self.on_complete.clone();

        alert::show_save_changes(
            parent_window,
            "Save changes?",
            &format!(
                "Document \"{doc_name}\" has unsaved changes. Do you want to save before closing?"
            ),
            move |response| match response {
                SaveChangesResponse::DontSave => {
                    if let Some(ref cb) = *on_complete.borrow() {
                        cb(Some(DocumentDialogResult::Close {
                            id: doc_id,
                            save: false,
                        }));
                    }
                }
                SaveChangesResponse::Save => {
                    if let Some(ref cb) = *on_complete.borrow() {
                        cb(Some(DocumentDialogResult::Close {
                            id: doc_id,
                            save: true,
                        }));
                    }
                }
                SaveChangesResponse::Cancel => {
                    if let Some(ref cb) = *on_complete.borrow() {
                        cb(None);
                    }
                }
            },
        );
    }
}

impl Default for CloseDocumentDialog {
    fn default() -> Self {
        Self::new()
    }
}

/// Dialog for setting/changing document password protection
pub struct DocumentProtectionDialog {
    window: adw::Window,
    enable_check: CheckButton,
    password_entry: PasswordEntry,
    confirm_entry: PasswordEntry,
    on_complete: DocumentCallback,
    doc_id: Rc<RefCell<Option<Uuid>>>,
}

impl DocumentProtectionDialog {
    /// Creates a new document protection dialog
    #[must_use]
    pub fn new(parent: Option<&gtk4::Window>) -> Self {
        let window = adw::Window::builder()
            .title("Document Protection")
            .modal(true)
            .default_width(400)
            .build();

        if let Some(p) = parent {
            window.set_transient_for(Some(p));
        }

        window.set_size_request(280, -1);

        // Header bar with Cancel/Apply buttons (GNOME HIG)
        let header = adw::HeaderBar::new();
        header.set_show_end_title_buttons(false);
        header.set_show_start_title_buttons(false);
        let cancel_btn = Button::builder().label("Cancel").build();
        let apply_btn = Button::builder()
            .label("Apply")
            .css_classes(["suggested-action"])
            .build();
        header.pack_start(&cancel_btn);
        header.pack_end(&apply_btn);

        // Content
        let content = GtkBox::new(Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // Use ToolbarView for adw::Window
        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&content));
        window.set_content(Some(&toolbar_view));

        // Info label
        let info_label = Label::builder()
            .label("Password protection encrypts the document contents.\nYou will need to enter the password each time you open it.")
            .halign(gtk4::Align::Start)
            .wrap(true)
            .build();
        content.append(&info_label);

        // Enable checkbox
        let enable_check = CheckButton::builder()
            .label("Enable password protection")
            .build();
        content.append(&enable_check);

        // Password fields (initially hidden)
        let password_box = GtkBox::new(Orientation::Vertical, 8);
        password_box.set_margin_start(24);
        password_box.set_visible(false);

        let password_label = Label::builder()
            .label("New Password")
            .halign(gtk4::Align::Start)
            .build();
        password_box.append(&password_label);

        let password_entry = PasswordEntry::builder().show_peek_icon(true).build();
        password_box.append(&password_entry);

        let confirm_label = Label::builder()
            .label("Confirm Password")
            .halign(gtk4::Align::Start)
            .build();
        password_box.append(&confirm_label);

        let confirm_entry = PasswordEntry::builder().show_peek_icon(true).build();
        password_box.append(&confirm_entry);

        // Password strength hint
        let hint_label = Label::builder()
            .label("Use a strong password that you can remember.")
            .halign(gtk4::Align::Start)
            .css_classes(["dim-label"])
            .build();
        password_box.append(&hint_label);

        content.append(&password_box);
        window.set_child(Some(&content));

        let on_complete: DocumentCallback = Rc::new(RefCell::new(None));
        let doc_id: Rc<RefCell<Option<Uuid>>> = Rc::new(RefCell::new(None));

        // Toggle password fields visibility
        let password_box_clone = password_box.clone();
        enable_check.connect_toggled(move |check| {
            password_box_clone.set_visible(check.is_active());
        });

        // Validate input
        let apply_btn_clone = apply_btn.clone();
        let enable_check_clone = enable_check.clone();
        let password_entry_clone = password_entry.clone();
        let confirm_entry_clone = confirm_entry.clone();

        let validate = move || {
            let valid = if enable_check_clone.is_active() {
                let pwd = password_entry_clone.text();
                let confirm = confirm_entry_clone.text();
                !pwd.is_empty() && pwd == confirm
            } else {
                true // Disabling protection is always valid
            };
            apply_btn_clone.set_sensitive(valid);
        };

        let validate_clone = validate.clone();
        password_entry.connect_changed(move |_| validate_clone());

        let validate_clone = validate.clone();
        confirm_entry.connect_changed(move |_| validate_clone());

        let validate_clone = validate;
        enable_check.connect_toggled(move |_| validate_clone());

        // Cancel button
        let window_clone = window.clone();
        let on_complete_clone = on_complete.clone();
        cancel_btn.connect_clicked(move |_| {
            if let Some(ref cb) = *on_complete_clone.borrow() {
                cb(None);
            }
            window_clone.close();
        });

        // Apply button
        let window_clone = window.clone();
        let on_complete_clone = on_complete.clone();
        let enable_check_clone = enable_check.clone();
        let password_entry_clone = password_entry.clone();
        let doc_id_clone = doc_id.clone();
        apply_btn.connect_clicked(move |_| {
            let password = if enable_check_clone.is_active() {
                Some(password_entry_clone.text().to_string())
            } else {
                None
            };

            if let Some(id) = *doc_id_clone.borrow() {
                if let Some(ref cb) = *on_complete_clone.borrow() {
                    cb(Some(DocumentDialogResult::Save {
                        id,
                        path: PathBuf::new(), // Path will be determined by caller
                        password,
                    }));
                }
            }
            window_clone.close();
        });

        Self {
            window,
            enable_check,
            password_entry,
            confirm_entry,
            on_complete,
            doc_id,
        }
    }

    /// Sets the callback for when the dialog completes
    pub fn set_callback<F>(&self, callback: F)
    where
        F: Fn(Option<DocumentDialogResult>) + 'static,
    {
        *self.on_complete.borrow_mut() = Some(Box::new(callback));
    }

    /// Shows the dialog for a specific document
    pub fn present(&self, doc_id: Uuid, is_currently_protected: bool) {
        *self.doc_id.borrow_mut() = Some(doc_id);
        self.enable_check.set_active(is_currently_protected);
        self.password_entry.set_text("");
        self.confirm_entry.set_text("");
        self.window.present();
    }
}
