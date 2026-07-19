//! Import dialog for importing connections from external sources
//!
//! Provides a GTK4 dialog with source selection, progress display,
//! and result summary for importing connections from Asbru-CM, SSH config,
//! Remmina, and Ansible inventory files.
//!
//! Updated for GTK 4.10+ compatibility using Window instead of Dialog.

mod sources;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, Label, ListBox, Orientation, ProgressBar, ScrolledWindow, Separator,
    Stack,
};
use libadwaita as adw;
use rustconn_core::import::{
    AnsibleInventoryImporter, AsbruImporter, ImportResult, ImportSource, LibvirtDaemonImporter,
    LibvirtXmlImporter, RemminaImporter, SshConfigImporter,
};
use rustconn_core::progress::LocalProgressReporter;

use crate::i18n::{i18n, i18n_f};

/// Import dialog for importing connections from external sources
pub struct ImportDialog {
    dialog: adw::Dialog,
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
    parent: Option<gtk4::Window>,
}

impl ImportDialog {
    /// Creates a new import dialog
    #[must_use]
    pub fn new(parent: Option<&gtk4::Window>) -> Self {
        let dialog = adw::Dialog::builder()
            .title(i18n("Import Connections"))
            .content_width(600)
            .content_height(500)
            .build();

        // Header bar with Import icon button and standard window buttons (GNOME HIG)
        let header = adw::HeaderBar::new();
        let import_button = Button::from_icon_name("document-open-symbolic");
        import_button.set_tooltip_text(Some(&i18n("Import")));
        import_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Import"))]);
        import_button.add_css_class("suggested-action");
        header.pack_start(&import_button);

        // Create main layout with header at top using ToolbarView
        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);

        // Create main content area with clamp
        let clamp = adw::Clamp::builder()
            .maximum_size(600)
            .tightening_threshold(400)
            .build();

        let content = GtkBox::new(Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // Create stack for different views
        let stack = Stack::new();
        stack.set_vexpand(true);
        content.append(&stack);

        clamp.set_child(Some(&content));
        toolbar_view.set_content(Some(&clamp));
        dialog.set_child(Some(&toolbar_view));

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

        let dialog_inst = Self {
            dialog,
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
            parent: parent.cloned(),
        };

        // Wire up source selection to import button state
        dialog_inst.connect_source_selection_to_import_button();

        dialog_inst
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

    fn create_progress_page() -> (GtkBox, ProgressBar, Label) {
        let vbox = GtkBox::new(Orientation::Vertical, 12);
        vbox.set_valign(gtk4::Align::Center);

        let header = Label::builder()
            .label(i18n("Importing..."))
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
            .label(i18n("Scanning for connections..."))
            .css_classes(["dim-label"])
            .build();
        vbox.append(&progress_label);

        (vbox, progress_bar, progress_label)
    }

    fn create_result_page() -> (GtkBox, Label, Label) {
        let vbox = GtkBox::new(Orientation::Vertical, 12);

        let header = Label::builder()
            .label(i18n("Import Complete"))
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
            .label(i18n("Details"))
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

    /// Converts an import result or error into an `ImportResult`.
    ///
    /// On success, returns the result as-is. On error, logs the technical
    /// details via `tracing` and returns an `ImportResult` with the error
    /// preserved in the `errors` vec so the UI can display it.
    fn import_or_error(
        result: Result<ImportResult, rustconn_core::error::ImportError>,
        source_name: &str,
    ) -> ImportResult {
        match result {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(?e, "Import failed for {}", source_name);
                let mut failed = ImportResult::default();
                failed.add_error(e);
                failed
            }
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
                Self::import_or_error(importer.import(), "SSH config")
            }
            "asbru" => {
                let importer = AsbruImporter::new();
                Self::import_or_error(importer.import(), "Asbru-CM")
            }
            "remmina" => {
                let importer = RemminaImporter::new();
                Self::import_or_error(importer.import(), "Remmina")
            }
            "ansible" => {
                let importer = AnsibleInventoryImporter::new();
                Self::import_or_error(importer.import(), "Ansible inventory")
            }
            "libvirt_daemon" => {
                let importer = LibvirtDaemonImporter::new();
                Self::import_or_error(importer.import(), "Libvirt Daemon")
            }
            _ => ImportResult::default(),
        }
    }

    /// Updates the result page with import results
    ///
    /// Displays a summary of successful imports and detailed information about:
    /// - Successfully imported connections and groups
    /// - Skipped entries with reasons
    /// - Errors encountered during import
    pub fn show_results(&self, result: &ImportResult) {
        self.show_results_with_source(result, None);
    }

    /// Updates the result page with import results and optional source name
    ///
    /// Displays a summary including the source name if provided.
    pub fn show_results_with_source(&self, result: &ImportResult, source_name: Option<&str>) {
        let conn_count = result.connections.len().to_string();
        let group_count = result.groups.len().to_string();
        let summary = source_name.map_or_else(
            || {
                i18n_f(
                    "Successfully imported {} connection(s) and {} group(s).",
                    &[&conn_count, &group_count],
                )
            },
            |name| {
                i18n_f(
                    "Successfully imported {} connection(s) and {} group(s).\nConnections will be added to '{} Import' group.",
                    &[&conn_count, &group_count, name],
                )
            },
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
            details.push_str(&i18n("Imported connections:"));
            details.push('\n');
            for conn in &result.connections {
                let _ = writeln!(details, "  • {} ({}:{})", conn.name, conn.host, conn.port);
            }
            details.push('\n');
        }

        // List skipped entries
        if !result.skipped.is_empty() {
            let _ = writeln!(
                details,
                "{}",
                i18n_f("Skipped {} entries:", &[&result.skipped.len().to_string()])
            );
            for skipped in &result.skipped {
                let _ = writeln!(details, "  • {}: {}", skipped.identifier, skipped.reason);
            }
            details.push('\n');
        }

        // List errors
        if !result.errors.is_empty() {
            let _ = writeln!(
                details,
                "{}",
                i18n_f("Errors ({}):", &[&result.errors.len().to_string()])
            );
            for error in &result.errors {
                let _ = writeln!(details, "  • {error}");
            }
        }

        if details.is_empty() {
            details = i18n("No connections found in the selected source.");
        }

        details
    }

    /// Runs the dialog and calls the callback with the result
    ///
    /// The import button is wired to:
    /// 1. Get the selected source via `get_selected_source()`
    /// 2. Perform import via `do_import()`
    /// 3. Display results via `show_results()`
    pub fn run<F: Fn(Option<ImportResult>) + 'static>(&self, cb: F) {
        // Store callback
        *self.on_complete.borrow_mut() = Some(Box::new(cb));

        let dialog = self.dialog.clone();
        let stack = self.stack.clone();
        let source_list = self.source_list.clone();
        let progress_bar = self.progress_bar.clone();
        let progress_label = self.progress_label.clone();
        let result_label = self.result_label.clone();
        let result_details = self.result_details.clone();
        let import_button = self.import_button.clone();
        let result_cell = self.result.clone();
        let on_complete = self.on_complete.clone();

        // Wire import button click to do_import()
        import_button.connect_clicked(move |btn| {
            let current_page = stack.visible_child_name();

            if current_page.as_deref() == Some("result") {
                // Done - close dialog
                if let Some(ref cb) = *on_complete.borrow() {
                    cb(result_cell.borrow_mut().take());
                }
                dialog.close();
                return;
            }

            // Get selected source using get_selected_source() pattern
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
                progress_label.set_text(&i18n_f("Importing from {}...", &[&display_name]));

                // Perform import with progress reporting
                let result =
                    Self::do_import_with_progress(&source_id, &progress_bar, &progress_label);

                progress_bar.set_fraction(1.0);
                progress_label.set_text(&i18n("Import complete"));

                // Show results using show_results() pattern
                let summary = i18n_f(
                    "Successfully imported {} connection(s) and {} group(s).",
                    &[
                        &result.connections.len().to_string(),
                        &result.groups.len().to_string(),
                    ],
                );
                result_label.set_text(&summary);

                let details = Self::format_import_details(&result);
                result_details.set_text(&details);

                *result_cell.borrow_mut() = Some(result);
                stack.set_visible_child_name("result");
                btn.set_label(&i18n("Done"));
                btn.set_sensitive(true);
            }
        });

        self.dialog.present(self.parent.as_ref());
    }

    /// Runs the dialog and calls the callback with the result and source name
    ///
    /// Similar to `run()` but also provides the source name to the callback.
    /// The import button is wired to:
    /// 1. Get the selected source via `get_selected_source()`
    /// 2. Perform import via `do_import()`
    /// 3. Display results via `show_results_with_source()`
    #[expect(
        clippy::too_many_lines,
        reason = "long match/dispatch over many enum variants; splitting per variant only relocates the boilerplate"
    )]
    pub fn run_with_source<F: Fn(Option<ImportResult>, String) + 'static>(&self, cb: F) {
        // Store callback
        *self.on_complete_with_source.borrow_mut() = Some(Box::new(cb));

        let dialog = self.dialog.clone();
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
        let parent_window = self.parent.clone();

        // Wire import button click to do_import()
        import_button.connect_clicked(move |btn| {
            let current_page = stack.visible_child_name();

            if current_page.as_deref() == Some("result") {
                // Done - close dialog
                if let Some(ref cb) = *on_complete_with_source.borrow() {
                    let source = source_name_cell.borrow().clone();
                    cb(result_cell.borrow_mut().take(), source);
                }
                dialog.close();
                return;
            }

            // Get selected source using get_selected_source() pattern
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
                progress_label.set_text(&i18n_f("Importing from {}...", &[&display_name]));

                // Handle special case for file-based import
                if source_id == "ssh_config_file" {
                    Self::handle_ssh_config_file_import(
                        parent_window.as_ref(),
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
                        parent_window.as_ref(),
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
                        parent_window.as_ref(),
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
                        parent_window.as_ref(),
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
                        parent_window.as_ref(),
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

                if source_id == "rdm_file" {
                    Self::handle_rdm_file_import(
                        parent_window.as_ref(),
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

                if source_id == "mobaxterm_file" {
                    Self::handle_mobaxterm_file_import(
                        parent_window.as_ref(),
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

                if source_id == "libvirt_file" {
                    Self::handle_libvirt_file_import(
                        parent_window.as_ref(),
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

                if source_id == "vv_file" {
                    Self::handle_vv_file_import(
                        parent_window.as_ref(),
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

                if source_id == "rdp_file" {
                    Self::handle_rdp_file_import(
                        parent_window.as_ref(),
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

                if source_id == "csv_file" {
                    Self::handle_csv_file_import(
                        parent_window.as_ref(),
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

                if source_id == "multi_file" {
                    Self::handle_multi_file_import(
                        parent_window.as_ref(),
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

                // Perform import with progress reporting
                let result = Self::do_import_with_progress(
                    &source_id,
                    &progress_bar,
                    &progress_label,
                );

                // Store source name
                *source_name_cell.borrow_mut() = display_name.clone();

                progress_bar.set_fraction(1.0);
                progress_label.set_text(&i18n("Import complete"));

                // Show results using show_results_with_source() pattern
                let conn_count = result.connections.len();
                let group_count = result.groups.len();
                let summary = i18n_f(
                    "Successfully imported {} connection(s) and {} group(s).\nConnections will be added to '{} Import' group.",
                    &[&conn_count.to_string(), &group_count.to_string(), &display_name],
                );
                result_label.set_text(&summary);

                let details = Self::format_import_details(&result);
                result_details.set_text(&details);

                *result_cell.borrow_mut() = Some(result);
                stack.set_visible_child_name("result");
                btn.set_label(&i18n("Done"));
                btn.set_sensitive(true);
            }
        });

        // Double-click on source row triggers import
        let gesture = gtk4::GestureClick::new();
        gesture.set_button(1); // Left mouse button
        let import_button_dblclick = self.import_button.clone();
        let source_list_dblclick = self.source_list.clone();
        gesture.connect_pressed(move |gesture, n_press, _x, y| {
            if n_press == 2 {
                // Double-click
                if let Some(row) = source_list_dblclick.row_at_y(y as i32) {
                    // Only trigger if row is sensitive (available)
                    if row.is_sensitive() {
                        import_button_dblclick.emit_clicked();
                    }
                }
                gesture.set_state(gtk4::EventSequenceState::Claimed);
            }
        });
        self.source_list.add_controller(gesture);

        self.dialog.present(self.parent.as_ref());
    }

    /// Returns a reference to the underlying dialog
    #[must_use]
    pub const fn dialog(&self) -> &adw::Dialog {
        &self.dialog
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
        reporter.report(0, 1, &i18n_f("Starting import from {}...", &[source_id]));

        let result = match source_id {
            "ssh_config" => {
                let importer = SshConfigImporter::new();
                let paths = importer.default_paths();
                let total = paths.len().max(1);

                for (i, path) in paths.iter().enumerate() {
                    reporter.report(
                        i,
                        total,
                        &i18n_f("Importing from {}...", &[&path.display().to_string()]),
                    );
                    if reporter.is_cancelled() {
                        return ImportResult::default();
                    }
                }

                Self::import_or_error(importer.import(), "SSH config")
            }
            "asbru" => {
                let importer = AsbruImporter::new();
                let paths = importer.default_paths();
                let total = paths.len().max(1);

                for (i, path) in paths.iter().enumerate() {
                    reporter.report(
                        i,
                        total,
                        &i18n_f("Importing from {}...", &[&path.display().to_string()]),
                    );
                    if reporter.is_cancelled() {
                        return ImportResult::default();
                    }
                }

                Self::import_or_error(importer.import(), "Asbru-CM")
            }
            "remmina" => {
                let importer = RemminaImporter::new();
                let paths = importer.default_paths();
                let total = paths.len().max(1);

                for (i, path) in paths.iter().enumerate() {
                    reporter.report(
                        i,
                        total,
                        &i18n_f("Importing from {}...", &[&path.display().to_string()]),
                    );
                    if reporter.is_cancelled() {
                        return ImportResult::default();
                    }
                }

                Self::import_or_error(importer.import(), "Remmina")
            }
            "ansible" => {
                let importer = AnsibleInventoryImporter::new();
                let paths = importer.default_paths();
                let total = paths.len().max(1);

                for (i, path) in paths.iter().enumerate() {
                    reporter.report(
                        i,
                        total,
                        &i18n_f("Importing from {}...", &[&path.display().to_string()]),
                    );
                    if reporter.is_cancelled() {
                        return ImportResult::default();
                    }
                }

                Self::import_or_error(importer.import(), "Ansible inventory")
            }
            "libvirt" => {
                let importer = LibvirtXmlImporter::new();
                let paths = importer.default_paths();
                let total = paths.len().max(1);

                for (i, path) in paths.iter().enumerate() {
                    reporter.report(
                        i,
                        total,
                        &i18n_f("Importing from {}...", &[&path.display().to_string()]),
                    );
                    if reporter.is_cancelled() {
                        return ImportResult::default();
                    }
                }

                Self::import_or_error(importer.import(), "Libvirt")
            }
            "libvirt_daemon" => {
                reporter.report(0, 1, &i18n("Querying libvirt daemon..."));
                if reporter.is_cancelled() {
                    return ImportResult::default();
                }

                let importer = LibvirtDaemonImporter::new();
                Self::import_or_error(importer.import(), "Libvirt Daemon")
            }
            _ => ImportResult::default(),
        };

        // Report completion
        reporter.report(1, 1, &i18n("Import complete"));
        result
    }
}
