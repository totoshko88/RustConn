//! Document management dialogs for `RustConn`
//!
//! Provides dialogs for creating, opening, saving, and managing documents.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, CheckButton, Entry, FileDialog, FileFilter, Label, Orientation, PasswordEntry,
};
use libadwaita as adw;
use secrecy::SecretString;
use uuid::Uuid;

use crate::alert::{self, SaveChangesResponse};
use crate::i18n::{i18n, i18n_f};

/// Callback type for document dialog results
pub type DocumentCallback = Rc<RefCell<Option<Box<dyn Fn(Option<DocumentDialogResult>)>>>>;

/// Result from document dialog
#[derive(Debug, Clone)]
pub enum DocumentDialogResult {
    /// Create a new document
    Create {
        name: String,
        password: Option<SecretString>,
    },
    /// Open an existing document
    Open {
        path: PathBuf,
        password: Option<SecretString>,
    },
    /// Save document
    Save {
        id: Uuid,
        path: PathBuf,
        password: Option<SecretString>,
    },
    /// Close document (with save prompt result)
    Close { id: Uuid, save: bool },
}

/// Dialog for creating a new document
pub struct NewDocumentDialog {
    dialog: adw::Dialog,
    name_entry: Entry,
    password_check: CheckButton,
    password_entry: PasswordEntry,
    confirm_entry: PasswordEntry,
    on_complete: DocumentCallback,
    parent: Option<gtk4::Widget>,
}

impl NewDocumentDialog {
    /// Creates a new document creation dialog
    #[must_use]
    pub fn new(parent: Option<&gtk4::Window>) -> Self {
        let dialog = adw::Dialog::builder()
            .title(i18n("New Document"))
            .content_width(400)
            .build();

        // Header bar (GNOME HIG)
        let (header, create_btn) = crate::dialogs::widgets::dialog_header("Create");
        create_btn.set_sensitive(false);

        // Content
        let content = GtkBox::new(Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // Use ToolbarView for adw::Dialog
        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        let clamp = adw::Clamp::builder()
            .maximum_size(600)
            .child(&content)
            .build();
        toolbar_view.set_content(Some(&clamp));
        dialog.set_child(Some(&toolbar_view));

        // Name field
        let name_label = Label::builder()
            .label(i18n("Document Name"))
            .halign(gtk4::Align::Start)
            .build();
        content.append(&name_label);

        let name_entry = Entry::builder()
            .placeholder_text(i18n("My Connections"))
            .build();
        name_entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
            name_label.upcast_ref()
        ])]);
        content.append(&name_entry);

        // Password protection
        let password_check = CheckButton::builder()
            .label(i18n("Protect with password"))
            .build();
        content.append(&password_check);

        // Password fields (initially hidden)
        let password_box = GtkBox::new(Orientation::Vertical, 8);
        password_box.set_margin_start(24);
        password_box.set_visible(false);

        let password_label = Label::builder()
            .label(i18n("Password"))
            .halign(gtk4::Align::Start)
            .build();
        password_box.append(&password_label);

        let password_entry = PasswordEntry::builder().show_peek_icon(true).build();
        password_entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
            password_label.upcast_ref(),
        ])]);
        password_box.append(&password_entry);

        let confirm_label = Label::builder()
            .label(i18n("Confirm Password"))
            .halign(gtk4::Align::Start)
            .build();
        password_box.append(&confirm_label);

        let confirm_entry = PasswordEntry::builder().show_peek_icon(true).build();
        confirm_entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
            confirm_label.upcast_ref()
        ])]);
        password_box.append(&confirm_entry);

        content.append(&password_box);

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

        // Guards against a double callback: the action button delivers its
        // result and then closes the dialog, which fires `connect_closed`.
        // Without this flag the caller would receive `Some(..)` immediately
        // followed by a spurious `None`.
        let completed = Rc::new(Cell::new(false));

        // On dialog closed (Escape or programmatic close) → notify callback
        let on_complete_clone = on_complete.clone();
        let completed_clone = completed.clone();
        dialog.connect_closed(move |_| {
            if completed_clone.get() {
                return;
            }
            if let Some(ref cb) = *on_complete_clone.borrow() {
                cb(None);
            }
        });

        // Create button
        let dialog_clone = dialog.clone();
        let on_complete_clone = on_complete.clone();
        let completed_clone = completed.clone();
        let name_entry_clone = name_entry.clone();
        let password_check_clone = password_check.clone();
        let password_entry_clone = password_entry.clone();
        create_btn.connect_clicked(move |_| {
            let name = name_entry_clone.text().to_string();
            let password = if password_check_clone.is_active() {
                Some(SecretString::from(password_entry_clone.text().to_string()))
            } else {
                None
            };

            completed_clone.set(true);
            if let Some(ref cb) = *on_complete_clone.borrow() {
                cb(Some(DocumentDialogResult::Create { name, password }));
            }
            dialog_clone.close();
        });

        Self {
            dialog,
            name_entry,
            password_check,
            password_entry,
            confirm_entry,
            on_complete,
            parent: parent.map(|p| p.clone().upcast::<gtk4::Widget>()),
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
        self.dialog.present(self.parent.as_ref());
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
        filter.set_name(Some(&i18n("RustConn Documents")));

        let filters = gtk4::gio::ListStore::new::<FileFilter>();
        filters.append(&filter);

        let dialog = FileDialog::builder()
            .title(i18n("Open Document"))
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
        let dialog = adw::Dialog::builder()
            .title(i18n("Enter Password"))
            .content_width(350)
            .build();

        let (header, open_btn) = crate::dialogs::widgets::dialog_header("Open");

        let content = GtkBox::new(Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        let label = Label::builder()
            .label(i18n(
                "This document is password protected.\n\
                 Enter the password to open it.",
            ))
            .halign(gtk4::Align::Start)
            .build();
        content.append(&label);

        let password_entry = PasswordEntry::builder().show_peek_icon(true).build();
        content.append(&password_entry);

        // Use ToolbarView for adw::Dialog
        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&content));
        dialog.set_child(Some(&toolbar_view));

        // Guards against a double callback (see NewDocumentDialog): the Open
        // button delivers its result and then closes the dialog, which fires
        // `connect_closed`.
        let completed = Rc::new(Cell::new(false));

        // On dialog closed (Escape) → notify callback with None
        let on_complete_clone = on_complete.clone();
        let completed_clone = completed.clone();
        dialog.connect_closed(move |_| {
            if completed_clone.get() {
                return;
            }
            if let Some(ref cb) = *on_complete_clone.borrow() {
                cb(None);
            }
        });

        // Open
        let dialog_clone = dialog.clone();
        let path_clone = path;
        open_btn.connect_clicked(move |_| {
            let password = SecretString::from(password_entry.text().to_string());
            completed.set(true);
            if let Some(ref cb) = *on_complete.borrow() {
                cb(Some(DocumentDialogResult::Open {
                    path: path_clone.clone(),
                    password: Some(password),
                }));
            }
            dialog_clone.close();
        });

        let parent_widget = parent.map(|p| p.clone().upcast::<gtk4::Widget>());
        dialog.present(parent_widget.as_ref());
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
        filter.set_name(Some(&i18n("RustConn Documents")));

        let filters = gtk4::gio::ListStore::new::<FileFilter>();
        filters.append(&filter);

        let dialog = FileDialog::builder()
            .title(i18n("Save Document"))
            .filters(&filters)
            .initial_name(format!("{suggested_name}.rcdb"))
            .modal(true)
            .build();

        let on_complete = self.on_complete.clone();

        dialog.save(parent, gtk4::gio::Cancellable::NONE, move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path()
                    && let Some(ref cb) = *on_complete.borrow()
                {
                    cb(Some(DocumentDialogResult::Save {
                        id: doc_id,
                        path,
                        password: None, // Password set separately if needed
                    }));
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
            &i18n("Save changes?"),
            &i18n_f(
                "Document \"{}\" has unsaved changes. Do you want to save before closing?",
                &[doc_name],
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
    dialog: adw::Dialog,
    enable_check: CheckButton,
    password_entry: PasswordEntry,
    confirm_entry: PasswordEntry,
    on_complete: DocumentCallback,
    doc_id: Rc<RefCell<Option<Uuid>>>,
    parent: Option<gtk4::Widget>,
}

impl DocumentProtectionDialog {
    /// Creates a new document protection dialog
    #[must_use]
    pub fn new(parent: Option<&gtk4::Window>) -> Self {
        let dialog = adw::Dialog::builder()
            .title(i18n("Document Protection"))
            .content_width(400)
            .build();

        // Header bar (GNOME HIG)
        let (header, apply_btn) = super::widgets::dialog_header("Apply");

        // Content
        let content = GtkBox::new(Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // Use ToolbarView for adw::Dialog
        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        let clamp = adw::Clamp::builder()
            .maximum_size(600)
            .child(&content)
            .build();
        toolbar_view.set_content(Some(&clamp));
        dialog.set_child(Some(&toolbar_view));

        // Info label
        let info_label = Label::builder()
            .label(i18n(
                "Password protection encrypts the document contents.\n\
                 You will need to enter the password each time you open it.",
            ))
            .halign(gtk4::Align::Start)
            .wrap(true)
            .build();
        content.append(&info_label);

        // Enable checkbox
        let enable_check = CheckButton::builder()
            .label(i18n("Enable password protection"))
            .build();
        content.append(&enable_check);

        // Password fields (initially hidden)
        let password_box = GtkBox::new(Orientation::Vertical, 8);
        password_box.set_margin_start(24);
        password_box.set_visible(false);

        let password_label = Label::builder()
            .label(i18n("New Password"))
            .halign(gtk4::Align::Start)
            .build();
        password_box.append(&password_label);

        let password_entry = PasswordEntry::builder().show_peek_icon(true).build();
        password_entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
            password_label.upcast_ref(),
        ])]);
        password_box.append(&password_entry);

        let confirm_label = Label::builder()
            .label(i18n("Confirm Password"))
            .halign(gtk4::Align::Start)
            .build();
        password_box.append(&confirm_label);

        let confirm_entry = PasswordEntry::builder().show_peek_icon(true).build();
        confirm_entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
            confirm_label.upcast_ref()
        ])]);
        password_box.append(&confirm_entry);

        // Password strength hint
        let hint_label = Label::builder()
            .label(i18n("Use a strong password that you can remember."))
            .halign(gtk4::Align::Start)
            .css_classes(["dim-label"])
            .build();
        password_box.append(&hint_label);

        content.append(&password_box);

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

        // Guards against a double callback (see NewDocumentDialog): the Apply
        // button delivers its result and then closes the dialog, which fires
        // `connect_closed`.
        let completed = Rc::new(Cell::new(false));

        // On dialog closed (Escape) → notify callback with None
        let on_complete_clone = on_complete.clone();
        let completed_clone = completed.clone();
        dialog.connect_closed(move |_| {
            if completed_clone.get() {
                return;
            }
            if let Some(ref cb) = *on_complete_clone.borrow() {
                cb(None);
            }
        });

        // Apply button
        let dialog_clone = dialog.clone();
        let on_complete_clone = on_complete.clone();
        let completed_clone = completed.clone();
        let enable_check_clone = enable_check.clone();
        let password_entry_clone = password_entry.clone();
        let doc_id_clone = doc_id.clone();
        apply_btn.connect_clicked(move |_| {
            let password = if enable_check_clone.is_active() {
                Some(SecretString::from(password_entry_clone.text().to_string()))
            } else {
                None
            };

            if let Some(id) = *doc_id_clone.borrow()
                && let Some(ref cb) = *on_complete_clone.borrow()
            {
                completed_clone.set(true);
                cb(Some(DocumentDialogResult::Save {
                    id,
                    path: PathBuf::new(), // Path will be determined by caller
                    password,
                }));
            }
            dialog_clone.close();
        });

        Self {
            dialog,
            enable_check,
            password_entry,
            confirm_entry,
            on_complete,
            doc_id,
            parent: parent.map(|p| p.clone().upcast::<gtk4::Widget>()),
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
        self.dialog.present(self.parent.as_ref());
    }
}
