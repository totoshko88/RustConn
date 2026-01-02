//! Import dialog for importing connections from external sources
//!
//! Provides a GTK4 dialog with source selection, progress display,
//! and result summary for importing connections from Asbru-CM, SSH config,
//! Remmina, and Ansible inventory files.
//!
//! Updated for GTK 4.10+ compatibility using Window instead of Dialog.

use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, Frame, HeaderBar, Label, ListBox, ListBoxRow, Orientation, ProgressBar,
    ScrolledWindow, Separator, Stack, Window,
};
use rustconn_core::export::NativeExport;
use rustconn_core::import::{
    AnsibleInventoryImporter, AsbruImporter, ImportResult, ImportSource, RemminaImporter,
    RoyalTsImporter, SshConfigImporter,
};
use rustconn_core::progress::LocalProgressReporter;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Import dialog for importing connections from external sources
pub struct ImportDialog {
    window: Window,
    stack: Stack,
    source_list: ListBox,
    progress_bar: ProgressBar,
    progress_label: Label,
    result_label: Label,
    result_details: Label,
    import_button: Button,
    // Note: close_button is not stored as a field since its click handler
    // is connected inline in the constructor and it's not accessed elsewhere
    result: Rc<RefCell<Option<ImportResult>>>,
    source_name: Rc<RefCell<String>>,
    on_complete: super::ImportCallback,
    on_complete_with_source: super::ImportWithSourceCallback,
}

impl ImportDialog {
    /// Creates a new import dialog
    #[must_use]
    pub fn new(parent: Option<&Window>) -> Self {
        // Create window instead of deprecated Dialog
        let window = Window::builder()
            .title("Import Connections")
            .modal(true)
            .default_width(750)
            .default_height(500)
            .build();

        if let Some(p) = parent {
            window.set_transient_for(Some(p));
        }

        // Create header bar (no Close button - window X is sufficient)
        let header = HeaderBar::new();
        let import_button = Button::builder()
            .label("Import")
            .css_classes(["suggested-action"])
            .build();
        header.pack_start(&import_button);
        window.set_titlebar(Some(&header));

        // Create main content area
        let content = GtkBox::new(Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // Create stack for different views
        let stack = Stack::new();
        stack.set_vexpand(true);
        content.append(&stack);
        window.set_child(Some(&content));

        // === Source Selection Page ===
        let source_page = Self::create_source_page();
        stack.add_named(&source_page.0, Some("source"));

        // === Progress Page ===
        let (progress_page, progress_bar, progress_label) = Self::create_progress_page();
        stack.add_named(&progress_page, Some("progress"));

        // === Result Page ===
        let (result_page, result_label, result_details) = Self::create_result_page();
        stack.add_named(&result_page, Some("result"));

        // Set initial page
        stack.set_visible_child_name("source");

        let on_complete: super::ImportCallback = Rc::new(RefCell::new(None));
        let on_complete_with_source: super::ImportWithSourceCallback = Rc::new(RefCell::new(None));
        let source_name: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));

        let dialog = Self {
            window,
            stack,
            source_list: source_page.1,
            progress_bar,
            progress_label,
            result_label,
            result_details,
            import_button,
            result: Rc::new(RefCell::new(None)),
            source_name,
            on_complete,
            on_complete_with_source,
        };

        // Wire up source selection to import button state (Requirement 5.1)
        dialog.connect_source_selection_to_import_button();

        dialog
    }

    /// Connects source list selection changes to import button enabled state
    ///
    /// When a source is selected, the import button is enabled.
    /// When no source is selected or the selected source is unavailable, the button is disabled.
    fn connect_source_selection_to_import_button(&self) {
        let import_button = self.import_button.clone();

        // Update button state based on initial selection
        self.update_import_button_state();

        // Connect to selection changes
        self.source_list.connect_row_selected(move |_, row| {
            let should_enable = row.is_some_and(vte4::WidgetExt::is_sensitive);
            import_button.set_sensitive(should_enable);
        });
    }

    /// Updates the import button state based on current selection
    fn update_import_button_state(&self) {
        let should_enable = self
            .source_list
            .selected_row()
            .is_some_and(|row| row.is_sensitive());
        self.import_button.set_sensitive(should_enable);
    }

    fn create_source_page() -> (GtkBox, ListBox) {
        let vbox = GtkBox::new(Orientation::Vertical, 12);

        let header = Label::builder()
            .label("Select Import Source")
            .css_classes(["title-3"])
            .halign(gtk4::Align::Start)
            .build();
        vbox.append(&header);

        let description = Label::builder()
            .label("Choose the source from which to import connections:")
            .halign(gtk4::Align::Start)
            .wrap(true)
            .build();
        vbox.append(&description);

        // Create list box for sources
        let list_box = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .css_classes(["boxed-list"])
            .build();

        // Add import sources
        let sources: Vec<(&str, &str, &str, bool)> = vec![
            (
                "ssh_config",
                "SSH Config",
                "Import from ~/.ssh/config",
                SshConfigImporter::new().is_available(),
            ),
            (
                "ssh_config_file",
                "SSH Config File",
                "Import from a specific SSH config file",
                true,
            ),
            (
                "asbru",
                "Asbru-CM",
                "Import from Asbru-CM/PAC Manager config",
                AsbruImporter::new().is_available(),
            ),
            (
                "asbru_file",
                "Asbru-CM YAML File",
                "Import from a specific Asbru-CM YAML file",
                true,
            ),
            (
                "remmina",
                "Remmina",
                "Import from Remmina connection files",
                RemminaImporter::new().is_available(),
            ),
            (
                "ansible",
                "Ansible Inventory",
                "Import from Ansible inventory files",
                AnsibleInventoryImporter::new().is_available(),
            ),
            (
                "ansible_file",
                "Ansible Inventory File",
                "Import from a specific Ansible inventory file",
                true,
            ),
            (
                "native_file",
                "RustConn Native (.rcn)",
                "Import from a RustConn native export file",
                true,
            ),
            (
                "royalts_file",
                "Royal TS (.rtsz)",
                "Import from a Royal TS export file",
                true,
            ),
        ];

        for (id, name, desc, available) in sources {
            let row = Self::create_source_row(id, name, desc, available);
            list_box.append(&row);
        }

        // Select first available row
        if let Some(row) = list_box.row_at_index(0) {
            list_box.select_row(Some(&row));
        }

        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .child(&list_box)
            .build();

        let frame = Frame::builder().child(&scrolled).build();
        vbox.append(&frame);

        (vbox, list_box)
    }

    fn create_source_row(id: &str, name: &str, description: &str, available: bool) -> ListBoxRow {
        let hbox = GtkBox::new(Orientation::Horizontal, 12);
        hbox.set_margin_top(8);
        hbox.set_margin_bottom(8);
        hbox.set_margin_start(12);
        hbox.set_margin_end(12);

        let vbox = GtkBox::new(Orientation::Vertical, 4);
        vbox.set_hexpand(true);

        let name_label = Label::builder()
            .label(name)
            .halign(gtk4::Align::Start)
            .css_classes(["heading"])
            .build();
        vbox.append(&name_label);

        let desc_label = Label::builder()
            .label(description)
            .halign(gtk4::Align::Start)
            .css_classes(["dim-label"])
            .build();
        vbox.append(&desc_label);

        hbox.append(&vbox);

        // Status indicator
        let status = if available {
            Label::builder()
                .label("Available")
                .css_classes(["success"])
                .build()
        } else {
            Label::builder()
                .label("Not Found")
                .css_classes(["dim-label"])
                .build()
        };
        hbox.append(&status);

        ListBoxRow::builder()
            .child(&hbox)
            .sensitive(available)
            .name(id)
            .build()
    }

    fn create_progress_page() -> (GtkBox, ProgressBar, Label) {
        let vbox = GtkBox::new(Orientation::Vertical, 12);
        vbox.set_valign(gtk4::Align::Center);

        let header = Label::builder()
            .label("Importing...")
            .css_classes(["title-3"])
            .build();
        vbox.append(&header);

        let progress_bar = ProgressBar::builder()
            .show_text(true)
            .margin_top(12)
            .margin_bottom(12)
            .build();
        vbox.append(&progress_bar);

        let progress_label = Label::builder()
            .label("Scanning for connections...")
            .css_classes(["dim-label"])
            .build();
        vbox.append(&progress_label);

        (vbox, progress_bar, progress_label)
    }

    fn create_result_page() -> (GtkBox, Label, Label) {
        let vbox = GtkBox::new(Orientation::Vertical, 12);

        let header = Label::builder()
            .label("Import Complete")
            .css_classes(["title-3"])
            .halign(gtk4::Align::Start)
            .build();
        vbox.append(&header);

        let result_label = Label::builder()
            .halign(gtk4::Align::Start)
            .wrap(true)
            .build();
        vbox.append(&result_label);

        vbox.append(&Separator::new(Orientation::Horizontal));

        let details_header = Label::builder()
            .label("Details")
            .css_classes(["heading"])
            .halign(gtk4::Align::Start)
            .margin_top(8)
            .build();
        vbox.append(&details_header);

        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .build();

        let result_details = Label::builder()
            .halign(gtk4::Align::Start)
            .valign(gtk4::Align::Start)
            .wrap(true)
            .selectable(true)
            .build();
        scrolled.set_child(Some(&result_details));

        vbox.append(&scrolled);

        (vbox, result_label, result_details)
    }

    /// Gets the selected import source ID
    ///
    /// Returns the source ID string (e.g., "`ssh_config`", "asbru") if a source is selected,
    /// or None if no source is selected.
    #[must_use]
    pub fn get_selected_source(&self) -> Option<String> {
        self.source_list.selected_row().and_then(|row| {
            let name = row.widget_name();
            if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            }
        })
    }

    /// Gets the display name for a source ID
    #[must_use]
    pub fn get_source_display_name(source_id: &str) -> &'static str {
        match source_id {
            "ssh_config" => "SSH Config",
            "ssh_config_file" => "SSH Config File",
            "asbru" => "Asbru-CM",
            "asbru_file" => "Asbru-CM File",
            "remmina" => "Remmina",
            "ansible" => "Ansible",
            "ansible_file" => "Ansible File",
            "native_file" => "RustConn Native",
            "royalts_file" => "Royal TS",
            _ => "Unknown",
        }
    }

    /// Performs the import operation for the given source ID
    ///
    /// This method executes the appropriate importer based on the source ID
    /// and returns the import result containing connections, groups, skipped entries, and errors.
    #[must_use]
    pub fn do_import(&self, source_id: &str) -> ImportResult {
        match source_id {
            "ssh_config" => {
                let importer = SshConfigImporter::new();
                importer.import().unwrap_or_default()
            }
            "asbru" => {
                let importer = AsbruImporter::new();
                importer.import().unwrap_or_default()
            }
            "remmina" => {
                let importer = RemminaImporter::new();
                importer.import().unwrap_or_default()
            }
            "ansible" => {
                let importer = AnsibleInventoryImporter::new();
                importer.import().unwrap_or_default()
            }
            _ => ImportResult::default(),
        }
    }

    /// Updates the result page with import results
    ///
    /// Displays a summary of successful imports and detailed information about:
    /// - Successfully imported connections and groups
    /// - Skipped entries with reasons (Requirement 5.2)
    /// - Errors encountered during import (Requirement 5.3)
    pub fn show_results(&self, result: &ImportResult) {
        self.show_results_with_source(result, None);
    }

    /// Updates the result page with import results and optional source name
    ///
    /// Displays a summary including the source name if provided.
    pub fn show_results_with_source(&self, result: &ImportResult, source_name: Option<&str>) {
        let conn_count = result.connections.len();
        let group_count = result.groups.len();
        let summary = source_name.map_or_else(
            || format!("Successfully imported {conn_count} connection(s) and {group_count} group(s)."),
            |name| format!(
                "Successfully imported {conn_count} connection(s) and {group_count} group(s).\nConnections will be added to '{name} Import' group."
            ),
        );
        self.result_label.set_text(&summary);

        let details = Self::format_import_details(result);
        self.result_details.set_text(&details);
    }

    /// Formats import result details into a displayable string
    #[must_use]
    pub fn format_import_details(result: &ImportResult) -> String {
        use std::fmt::Write;
        let mut details = String::new();

        // List imported connections
        if !result.connections.is_empty() {
            details.push_str("Imported connections:\n");
            for conn in &result.connections {
                let _ = writeln!(details, "  • {} ({}:{})", conn.name, conn.host, conn.port);
            }
            details.push('\n');
        }

        // List skipped entries (Requirement 5.2)
        if !result.skipped.is_empty() {
            let _ = writeln!(details, "Skipped {} entries:", result.skipped.len());
            for skipped in &result.skipped {
                let _ = writeln!(details, "  • {}: {}", skipped.identifier, skipped.reason);
            }
            details.push('\n');
        }

        // List errors (Requirement 5.3)
        if !result.errors.is_empty() {
            let _ = writeln!(details, "Errors ({}):", result.errors.len());
            for error in &result.errors {
                let _ = writeln!(details, "  • {error}");
            }
        }

        if details.is_empty() {
            details = "No connections found in the selected source.".to_string();
        }

        details
    }

    /// Runs the dialog and calls the callback with the result
    ///
    /// The import button is wired to:
    /// 1. Get the selected source via `get_selected_source()` (Requirement 5.1)
    /// 2. Perform import via `do_import()` (Requirement 5.1)
    /// 3. Display results via `show_results()` (Requirements 5.2, 5.3)
    pub fn run<F: Fn(Option<ImportResult>) + 'static>(&self, cb: F) {
        // Store callback
        *self.on_complete.borrow_mut() = Some(Box::new(cb));

        let window = self.window.clone();
        let stack = self.stack.clone();
        let source_list = self.source_list.clone();
        let progress_bar = self.progress_bar.clone();
        let progress_label = self.progress_label.clone();
        let result_label = self.result_label.clone();
        let result_details = self.result_details.clone();
        let import_button = self.import_button.clone();
        let result_cell = self.result.clone();
        let on_complete = self.on_complete.clone();

        // Wire import button click to do_import() (Requirement 5.1)
        import_button.connect_clicked(move |btn| {
            let current_page = stack.visible_child_name();

            if current_page.as_deref() == Some("result") {
                // Done - close dialog
                if let Some(ref cb) = *on_complete.borrow() {
                    cb(result_cell.borrow_mut().take());
                }
                window.close();
                return;
            }

            // Get selected source using get_selected_source() pattern (Requirement 5.1)
            let source_id = source_list.selected_row().and_then(|row| {
                let name = row.widget_name();
                if name.is_empty() {
                    None
                } else {
                    Some(name.to_string())
                }
            });

            if let Some(source_id) = source_id {
                // Show progress page
                stack.set_visible_child_name("progress");
                btn.set_sensitive(false);
                progress_bar.set_fraction(0.0);

                let display_name = Self::get_source_display_name(&source_id);
                progress_label.set_text(&format!("Importing from {display_name}..."));

                // Perform import with progress reporting (Requirements 3.1, 3.6)
                let result =
                    Self::do_import_with_progress(&source_id, &progress_bar, &progress_label);

                progress_bar.set_fraction(1.0);
                progress_label.set_text("Import complete");

                // Show results using show_results() pattern (Requirements 5.2, 5.3)
                let summary = format!(
                    "Successfully imported {} connection(s) and {} group(s).",
                    result.connections.len(),
                    result.groups.len()
                );
                result_label.set_text(&summary);

                let details = Self::format_import_details(&result);
                result_details.set_text(&details);

                *result_cell.borrow_mut() = Some(result);
                stack.set_visible_child_name("result");
                btn.set_label("Done");
                btn.set_sensitive(true);
            }
        });

        self.window.present();
    }

    /// Runs the dialog and calls the callback with the result and source name
    ///
    /// Similar to `run()` but also provides the source name to the callback.
    /// The import button is wired to:
    /// 1. Get the selected source via `get_selected_source()` (Requirement 5.1)
    /// 2. Perform import via `do_import()` (Requirement 5.1)
    /// 3. Display results via `show_results_with_source()` (Requirements 5.2, 5.3)
    #[allow(clippy::too_many_lines)]
    pub fn run_with_source<F: Fn(Option<ImportResult>, String) + 'static>(&self, cb: F) {
        // Store callback
        *self.on_complete_with_source.borrow_mut() = Some(Box::new(cb));

        let window = self.window.clone();
        let stack = self.stack.clone();
        let source_list = self.source_list.clone();
        let progress_bar = self.progress_bar.clone();
        let progress_label = self.progress_label.clone();
        let result_label = self.result_label.clone();
        let result_details = self.result_details.clone();
        let import_button = self.import_button.clone();
        let result_cell = self.result.clone();
        let source_name_cell = self.source_name.clone();
        let on_complete_with_source = self.on_complete_with_source.clone();

        // Wire import button click to do_import() (Requirement 5.1)
        import_button.connect_clicked(move |btn| {
            let current_page = stack.visible_child_name();

            if current_page.as_deref() == Some("result") {
                // Done - close dialog
                if let Some(ref cb) = *on_complete_with_source.borrow() {
                    let source = source_name_cell.borrow().clone();
                    cb(result_cell.borrow_mut().take(), source);
                }
                window.close();
                return;
            }

            // Get selected source using get_selected_source() pattern (Requirement 5.1)
            let source_id = source_list
                .selected_row()
                .and_then(|row| {
                    let name = row.widget_name();
                    if name.is_empty() {
                        None
                    } else {
                        Some(name.to_string())
                    }
                });

            if let Some(source_id) = source_id {
                // Show progress page
                stack.set_visible_child_name("progress");
                btn.set_sensitive(false);
                progress_bar.set_fraction(0.0);

                let display_name = Self::get_source_display_name(&source_id);
                progress_label.set_text(&format!("Importing from {display_name}..."));

                // Handle special case for file-based import
                if source_id == "ssh_config_file" {
                    Self::handle_ssh_config_file_import(
                        &window,
                        &stack,
                        &progress_bar,
                        &progress_label,
                        &result_label,
                        &result_details,
                        &result_cell,
                        &source_name_cell,
                        btn,
                    );
                    return;
                }

                if source_id == "asbru_file" {
                    Self::handle_asbru_file_import(
                        &window,
                        &stack,
                        &progress_bar,
                        &progress_label,
                        &result_label,
                        &result_details,
                        &result_cell,
                        &source_name_cell,
                        btn,
                    );
                    return;
                }

                if source_id == "ansible_file" {
                    Self::handle_ansible_file_import(
                        &window,
                        &stack,
                        &progress_bar,
                        &progress_label,
                        &result_label,
                        &result_details,
                        &result_cell,
                        &source_name_cell,
                        btn,
                    );
                    return;
                }

                if source_id == "native_file" {
                    Self::handle_native_file_import(
                        &window,
                        &stack,
                        &progress_bar,
                        &progress_label,
                        &result_label,
                        &result_details,
                        &result_cell,
                        &source_name_cell,
                        btn,
                    );
                    return;
                }

                if source_id == "royalts_file" {
                    Self::handle_royalts_file_import(
                        &window,
                        &stack,
                        &progress_bar,
                        &progress_label,
                        &result_label,
                        &result_details,
                        &result_cell,
                        &source_name_cell,
                        btn,
                    );
                    return;
                }

                // Perform import with progress reporting (Requirements 3.1, 3.6)
                let result = Self::do_import_with_progress(
                    &source_id,
                    &progress_bar,
                    &progress_label,
                );

                // Store source name
                *source_name_cell.borrow_mut() = display_name.to_string();

                progress_bar.set_fraction(1.0);
                progress_label.set_text("Import complete");

                // Show results using show_results_with_source() pattern (Requirements 5.2, 5.3)
                let conn_count = result.connections.len();
                let group_count = result.groups.len();
                let summary = format!(
                    "Successfully imported {conn_count} connection(s) and {group_count} group(s).\nConnections will be added to '{display_name} Import' group."
                );
                result_label.set_text(&summary);

                let details = Self::format_import_details(&result);
                result_details.set_text(&details);

                *result_cell.borrow_mut() = Some(result);
                stack.set_visible_child_name("result");
                btn.set_label("Done");
                btn.set_sensitive(true);
            }
        });

        self.window.present();
    }

    /// Handles the special case of importing from an SSH config file
    ///
    /// Opens a file chooser dialog for selecting any SSH config file,
    /// parses it using `SshConfigImporter::import_from_path()`, and displays
    /// a preview with connection count before import.
    ///
    /// Requirements: 1.1, 1.5
    #[allow(clippy::too_many_arguments)]
    fn handle_ssh_config_file_import(
        window: &Window,
        stack: &Stack,
        progress_bar: &ProgressBar,
        progress_label: &Label,
        result_label: &Label,
        result_details: &Label,
        result_cell: &Rc<RefCell<Option<ImportResult>>>,
        source_name_cell: &Rc<RefCell<String>>,
        btn: &Button,
    ) {
        // Use file dialog for selecting SSH config file (Requirement 1.1)
        let file_dialog = gtk4::FileDialog::builder()
            .title("Select SSH Config File")
            .modal(true)
            .build();

        // Set filter for SSH config files (typically no extension or "config")
        let filter = gtk4::FileFilter::new();
        filter.add_pattern("config");
        filter.add_pattern("config.*");
        filter.add_pattern("*");
        filter.set_name(Some("SSH config files"));
        let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
        filters.append(&filter);
        file_dialog.set_filters(Some(&filters));

        let stack_clone = stack.clone();
        let progress_bar_clone = progress_bar.clone();
        let progress_label_clone = progress_label.clone();
        let result_label_clone = result_label.clone();
        let result_details_clone = result_details.clone();
        let result_cell_clone = result_cell.clone();
        let source_name_cell_clone = source_name_cell.clone();
        let btn_clone = btn.clone();

        file_dialog.open(
            Some(window),
            gtk4::gio::Cancellable::NONE,
            move |file_result| {
                if let Ok(file) = file_result {
                    if let Some(path) = file.path() {
                        stack_clone.set_visible_child_name("progress");
                        btn_clone.set_sensitive(false);
                        progress_bar_clone.set_fraction(0.5);
                        progress_label_clone
                            .set_text(&format!("Importing from {}...", path.display()));

                        // Parse SSH config file using import_from_path (Requirement 1.2, 1.3)
                        let importer = SshConfigImporter::new();
                        let result = importer.import_from_path(&path).unwrap_or_default();

                        // Extract filename for display
                        let filename = path
                            .file_name().map_or_else(|| "SSH Config File".to_string(), |n| n.to_string_lossy().to_string());

                        source_name_cell_clone.borrow_mut().clone_from(&filename);

                        progress_bar_clone.set_fraction(1.0);

                        // Show results with preview including connection count (Requirement 1.5)
                        let conn_count = result.connections.len();
                        let group_count = result.groups.len();
                        let summary = format!(
                            "Successfully imported {conn_count} connection(s) and {group_count} group(s).\nConnections will be added to '{filename} Import' group."
                        );
                        result_label_clone.set_text(&summary);

                        let details = Self::format_import_details(&result);
                        result_details_clone.set_text(&details);

                        *result_cell_clone.borrow_mut() = Some(result);
                        stack_clone.set_visible_child_name("result");
                        btn_clone.set_label("Done");
                        btn_clone.set_sensitive(true);
                    }
                }
            },
        );
    }

    /// Handles the special case of importing from an Asbru-CM YAML file
    #[allow(clippy::too_many_arguments)]
    fn handle_asbru_file_import(
        window: &Window,
        stack: &Stack,
        progress_bar: &ProgressBar,
        progress_label: &Label,
        result_label: &Label,
        result_details: &Label,
        result_cell: &Rc<RefCell<Option<ImportResult>>>,
        source_name_cell: &Rc<RefCell<String>>,
        btn: &Button,
    ) {
        // Use file dialog
        let file_dialog = gtk4::FileDialog::builder()
            .title("Select Asbru-CM YAML File")
            .modal(true)
            .build();

        // Set filter for YAML files
        let filter = gtk4::FileFilter::new();
        filter.add_pattern("*.yml");
        filter.add_pattern("*.yaml");
        filter.set_name(Some("YAML files"));
        let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
        filters.append(&filter);
        file_dialog.set_filters(Some(&filters));

        let stack_clone = stack.clone();
        let progress_bar_clone = progress_bar.clone();
        let progress_label_clone = progress_label.clone();
        let result_label_clone = result_label.clone();
        let result_details_clone = result_details.clone();
        let result_cell_clone = result_cell.clone();
        let source_name_cell_clone = source_name_cell.clone();
        let btn_clone = btn.clone();

        file_dialog.open(
            Some(window),
            gtk4::gio::Cancellable::NONE,
            move |file_result| {
                if let Ok(file) = file_result {
                    if let Some(path) = file.path() {
                        stack_clone.set_visible_child_name("progress");
                        btn_clone.set_sensitive(false);
                        progress_bar_clone.set_fraction(0.5);
                        progress_label_clone
                            .set_text(&format!("Importing from {}...", path.display()));

                        let importer = AsbruImporter::new();
                        let result = importer.import_from_path(&path).unwrap_or_default();

                        // Extract filename for display
                        let filename = path
                            .file_name().map_or_else(|| "Asbru-CM File".to_string(), |n| n.to_string_lossy().to_string());

                        source_name_cell_clone.borrow_mut().clone_from(&filename);

                        progress_bar_clone.set_fraction(1.0);

                        // Show results using format_import_details() (Requirements 5.2, 5.3)
                        let conn_count = result.connections.len();
                        let group_count = result.groups.len();
                        let summary = format!(
                            "Successfully imported {conn_count} connection(s) and {group_count} group(s).\nConnections will be added to '{filename} Import' group."
                        );
                        result_label_clone.set_text(&summary);

                        let details = Self::format_import_details(&result);
                        result_details_clone.set_text(&details);

                        *result_cell_clone.borrow_mut() = Some(result);
                        stack_clone.set_visible_child_name("result");
                        btn_clone.set_label("Done");
                        btn_clone.set_sensitive(true);
                    }
                }
            },
        );
    }

    /// Handles the special case of importing from an Ansible inventory file
    #[allow(clippy::too_many_arguments)]
    fn handle_ansible_file_import(
        window: &Window,
        stack: &Stack,
        progress_bar: &ProgressBar,
        progress_label: &Label,
        result_label: &Label,
        result_details: &Label,
        result_cell: &Rc<RefCell<Option<ImportResult>>>,
        source_name_cell: &Rc<RefCell<String>>,
        btn: &Button,
    ) {
        // Use file dialog
        let file_dialog = gtk4::FileDialog::builder()
            .title("Select Ansible Inventory File")
            .modal(true)
            .build();

        // Set filter for inventory files (INI, YAML, or no extension)
        let filter = gtk4::FileFilter::new();
        filter.add_pattern("*.yml");
        filter.add_pattern("*.yaml");
        filter.add_pattern("*.ini");
        filter.add_pattern("hosts");
        filter.add_pattern("inventory");
        filter.add_pattern("*");
        filter.set_name(Some("Ansible inventory files"));
        let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
        filters.append(&filter);
        file_dialog.set_filters(Some(&filters));

        let stack_clone = stack.clone();
        let progress_bar_clone = progress_bar.clone();
        let progress_label_clone = progress_label.clone();
        let result_label_clone = result_label.clone();
        let result_details_clone = result_details.clone();
        let result_cell_clone = result_cell.clone();
        let source_name_cell_clone = source_name_cell.clone();
        let btn_clone = btn.clone();

        file_dialog.open(
            Some(window),
            gtk4::gio::Cancellable::NONE,
            move |file_result| {
                if let Ok(file) = file_result {
                    if let Some(path) = file.path() {
                        stack_clone.set_visible_child_name("progress");
                        btn_clone.set_sensitive(false);
                        progress_bar_clone.set_fraction(0.5);
                        progress_label_clone
                            .set_text(&format!("Importing from {}...", path.display()));

                        let importer = AnsibleInventoryImporter::new();
                        let result = importer.import_from_path(&path).unwrap_or_default();

                        // Extract filename for display
                        let filename = path
                            .file_name().map_or_else(|| "Ansible Inventory".to_string(), |n| n.to_string_lossy().to_string());

                        source_name_cell_clone.borrow_mut().clone_from(&filename);

                        progress_bar_clone.set_fraction(1.0);

                        // Show results using format_import_details() (Requirements 5.2, 5.3)
                        let conn_count = result.connections.len();
                        let group_count = result.groups.len();
                        let summary = format!(
                            "Successfully imported {conn_count} connection(s) and {group_count} group(s).\nConnections will be added to '{filename} Import' group."
                        );
                        result_label_clone.set_text(&summary);

                        let details = Self::format_import_details(&result);
                        result_details_clone.set_text(&details);

                        *result_cell_clone.borrow_mut() = Some(result);
                        stack_clone.set_visible_child_name("result");
                        btn_clone.set_label("Done");
                        btn_clone.set_sensitive(true);
                    }
                }
            },
        );
    }

    /// Returns a reference to the underlying window
    #[must_use]
    pub const fn window(&self) -> &Window {
        &self.window
    }

    /// Creates a progress reporter that updates the dialog's progress bar
    ///
    /// This method creates a `LocalProgressReporter` that updates the
    /// progress bar and label in the import dialog during import operations.
    ///
    /// # Arguments
    ///
    /// * `progress_bar` - The progress bar to update
    /// * `progress_label` - The label to update with status messages
    /// * `cancelled` - Shared cancellation flag
    ///
    /// # Returns
    ///
    /// A `LocalProgressReporter` that can be used for progress updates.
    #[must_use]
    pub fn create_progress_reporter(
        progress_bar: &ProgressBar,
        progress_label: &Label,
        cancelled: Rc<Cell<bool>>,
    ) -> LocalProgressReporter<impl Fn(usize, usize, &str)> {
        let bar = progress_bar.clone();
        let label = progress_label.clone();

        LocalProgressReporter::with_cancel_flag(
            move |current, total, message| {
                // Cast is safe: progress counts are small enough that f64 precision is sufficient
                #[allow(clippy::cast_precision_loss)]
                let fraction = if total > 0 {
                    current as f64 / total as f64
                } else {
                    0.0
                };
                bar.set_fraction(fraction);
                bar.set_text(Some(&format!("{current}/{total}")));
                label.set_text(message);

                // Process pending GTK events to keep UI responsive
                while gtk4::glib::MainContext::default().iteration(false) {}
            },
            cancelled,
        )
    }

    /// Performs import with progress reporting
    ///
    /// This method performs the import operation, updating the progress bar
    /// during the operation. Since GTK widgets are not thread-safe, we use
    /// a local progress reporter that updates the UI directly.
    ///
    /// # Arguments
    ///
    /// * `source_id` - The ID of the import source
    /// * `progress_bar` - The progress bar to update
    /// * `progress_label` - The label to update with status messages
    ///
    /// # Returns
    ///
    /// The import result containing connections, groups, skipped entries, and errors.
    #[must_use]
    pub fn do_import_with_progress(
        source_id: &str,
        progress_bar: &ProgressBar,
        progress_label: &Label,
    ) -> ImportResult {
        let cancelled = Rc::new(Cell::new(false));
        let reporter = Self::create_progress_reporter(progress_bar, progress_label, cancelled);

        // Report start of import
        reporter.report(0, 1, &format!("Starting import from {source_id}..."));

        let result = match source_id {
            "ssh_config" => {
                let importer = SshConfigImporter::new();
                let paths = importer.default_paths();
                let total = paths.len().max(1);

                for (i, path) in paths.iter().enumerate() {
                    reporter.report(i, total, &format!("Importing from {}...", path.display()));
                    if reporter.is_cancelled() {
                        return ImportResult::default();
                    }
                }

                importer.import().unwrap_or_default()
            }
            "asbru" => {
                let importer = AsbruImporter::new();
                let paths = importer.default_paths();
                let total = paths.len().max(1);

                for (i, path) in paths.iter().enumerate() {
                    reporter.report(i, total, &format!("Importing from {}...", path.display()));
                    if reporter.is_cancelled() {
                        return ImportResult::default();
                    }
                }

                importer.import().unwrap_or_default()
            }
            "remmina" => {
                let importer = RemminaImporter::new();
                let paths = importer.default_paths();
                let total = paths.len().max(1);

                for (i, path) in paths.iter().enumerate() {
                    reporter.report(i, total, &format!("Importing from {}...", path.display()));
                    if reporter.is_cancelled() {
                        return ImportResult::default();
                    }
                }

                importer.import().unwrap_or_default()
            }
            "ansible" => {
                let importer = AnsibleInventoryImporter::new();
                let paths = importer.default_paths();
                let total = paths.len().max(1);

                for (i, path) in paths.iter().enumerate() {
                    reporter.report(i, total, &format!("Importing from {}...", path.display()));
                    if reporter.is_cancelled() {
                        return ImportResult::default();
                    }
                }

                importer.import().unwrap_or_default()
            }
            _ => ImportResult::default(),
        };

        // Report completion
        reporter.report(1, 1, "Import complete");
        result
    }

    /// Handles the special case of importing from a RustConn native file (.rcn)
    ///
    /// Opens a file chooser dialog for selecting a .rcn file,
    /// parses it using `NativeExport::from_file()`, and displays
    /// a preview with connection count before import.
    ///
    /// Requirements: 13.1, 13.3
    #[allow(clippy::too_many_arguments)]
    fn handle_native_file_import(
        window: &Window,
        stack: &Stack,
        progress_bar: &ProgressBar,
        progress_label: &Label,
        result_label: &Label,
        result_details: &Label,
        result_cell: &Rc<RefCell<Option<ImportResult>>>,
        source_name_cell: &Rc<RefCell<String>>,
        btn: &Button,
    ) {
        // Use file dialog for selecting RustConn native file
        let file_dialog = gtk4::FileDialog::builder()
            .title("Select RustConn Native File")
            .modal(true)
            .build();

        // Set filter for .rcn files
        let filter = gtk4::FileFilter::new();
        filter.add_pattern("*.rcn");
        filter.set_name(Some("RustConn Native (*.rcn)"));
        let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
        filters.append(&filter);
        file_dialog.set_filters(Some(&filters));

        let stack_clone = stack.clone();
        let progress_bar_clone = progress_bar.clone();
        let progress_label_clone = progress_label.clone();
        let result_label_clone = result_label.clone();
        let result_details_clone = result_details.clone();
        let result_cell_clone = result_cell.clone();
        let source_name_cell_clone = source_name_cell.clone();
        let btn_clone = btn.clone();

        file_dialog.open(
            Some(window),
            gtk4::gio::Cancellable::NONE,
            move |file_result| {
                if let Ok(file) = file_result {
                    if let Some(path) = file.path() {
                        stack_clone.set_visible_child_name("progress");
                        btn_clone.set_sensitive(false);
                        progress_bar_clone.set_fraction(0.5);
                        progress_label_clone
                            .set_text(&format!("Importing from {}...", path.display()));

                        // Parse native file
                        match NativeExport::from_file(&path) {
                            Ok(native_export) => {
                                // Convert NativeExport to ImportResult
                                let result = ImportResult {
                                    connections: native_export.connections,
                                    groups: native_export.groups,
                                    skipped: Vec::new(),
                                    errors: Vec::new(),
                                };

                                // Extract filename for display
                                let filename = path.file_name().map_or_else(
                                    || "RustConn Native".to_string(),
                                    |n| n.to_string_lossy().to_string(),
                                );

                                source_name_cell_clone.borrow_mut().clone_from(&filename);

                                progress_bar_clone.set_fraction(1.0);

                                // Show results
                                let conn_count = result.connections.len();
                                let group_count = result.groups.len();
                                let summary = format!(
                                    "Successfully imported {conn_count} connection(s) and {group_count} group(s).\nConnections will be added to '{filename} Import' group."
                                );
                                result_label_clone.set_text(&summary);

                                let details = Self::format_import_details(&result);
                                result_details_clone.set_text(&details);

                                *result_cell_clone.borrow_mut() = Some(result);
                                stack_clone.set_visible_child_name("result");
                                btn_clone.set_label("Done");
                                btn_clone.set_sensitive(true);
                            }
                            Err(e) => {
                                // Show error
                                progress_bar_clone.set_fraction(1.0);
                                result_label_clone.set_text("Import Failed");
                                result_details_clone.set_text(&format!("Error: {e}"));

                                stack_clone.set_visible_child_name("result");
                                btn_clone.set_label("Close");
                                btn_clone.set_sensitive(true);
                            }
                        }
                    }
                }
            },
        );
    }

    /// Handles the special case of importing from a Royal TS file (.rtsz)
    #[allow(clippy::too_many_arguments)]
    fn handle_royalts_file_import(
        window: &Window,
        stack: &Stack,
        progress_bar: &ProgressBar,
        progress_label: &Label,
        result_label: &Label,
        result_details: &Label,
        result_cell: &Rc<RefCell<Option<ImportResult>>>,
        source_name_cell: &Rc<RefCell<String>>,
        btn: &Button,
    ) {
        let file_dialog = gtk4::FileDialog::builder()
            .title("Select Royal TS File")
            .modal(true)
            .build();

        let filter = gtk4::FileFilter::new();
        filter.add_pattern("*.rtsz");
        filter.add_pattern("*.json");
        filter.set_name(Some("Royal TS files (*.rtsz, *.json)"));
        let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
        filters.append(&filter);
        file_dialog.set_filters(Some(&filters));

        let stack_clone = stack.clone();
        let progress_bar_clone = progress_bar.clone();
        let progress_label_clone = progress_label.clone();
        let result_label_clone = result_label.clone();
        let result_details_clone = result_details.clone();
        let result_cell_clone = result_cell.clone();
        let source_name_cell_clone = source_name_cell.clone();
        let btn_clone = btn.clone();

        file_dialog.open(
            Some(window),
            gtk4::gio::Cancellable::NONE,
            move |file_result| {
                if let Ok(file) = file_result {
                    if let Some(path) = file.path() {
                        stack_clone.set_visible_child_name("progress");
                        btn_clone.set_sensitive(false);
                        progress_bar_clone.set_fraction(0.5);
                        progress_label_clone
                            .set_text(&format!("Importing from {}...", path.display()));

                        let importer = RoyalTsImporter::new();
                        let result = importer.import_from_path(&path).unwrap_or_default();

                        let filename = path.file_name().map_or_else(
                            || "Royal TS".to_string(),
                            |n| n.to_string_lossy().to_string(),
                        );

                        source_name_cell_clone.borrow_mut().clone_from(&filename);

                        progress_bar_clone.set_fraction(1.0);

                        let conn_count = result.connections.len();
                        let group_count = result.groups.len();
                        let summary = format!(
                            "Successfully imported {conn_count} connection(s) and {group_count} group(s).\nConnections will be added to '{filename} Import' group."
                        );
                        result_label_clone.set_text(&summary);

                        let details = Self::format_import_details(&result);
                        result_details_clone.set_text(&details);

                        *result_cell_clone.borrow_mut() = Some(result);
                        stack_clone.set_visible_child_name("result");
                        btn_clone.set_label("Done");
                        btn_clone.set_sensitive(true);
                    }
                }
            },
        );
    }
}
