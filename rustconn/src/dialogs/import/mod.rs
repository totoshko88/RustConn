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
    AnsibleInventoryImporter, AsbruImporter, ImportResult, ImportSource, ImportWarning,
    LibvirtDaemonImporter, LibvirtXmlImporter, RemminaImporter, SshConfigImporter,
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
            .label(i18n("Importing…"))
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
            .label(i18n("Scanning for connections…"))
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
        Self::do_import_blocking(source_id)
    }

    /// Performs the import operation on a background thread.
    ///
    /// This is the `Send`-safe variant used by `spawn_blocking_with_callback`.
    /// Does not reference any GTK widgets.
    #[must_use]
    pub fn do_import_blocking(source_id: &str) -> ImportResult {
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
            "libvirt" => {
                let importer = LibvirtXmlImporter::new();
                Self::import_or_error(importer.import(), "Libvirt")
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
        self.result_label
            .set_text(&Self::format_summary(result, source_name));

        let details = Self::format_import_details(result);
        self.result_details.set_text(&details);
    }

    /// Formats the one-line verdict shown above the import details.
    ///
    /// `source_name` names the group the connections land in, when known.
    /// Split out of `show_results_with_source` so the wording is decided by a
    /// pure function that the unit tests can reach without a display.
    #[must_use]
    pub fn format_summary(result: &ImportResult, source_name: Option<&str>) -> String {
        if result.connections.is_empty() && !result.errors.is_empty() {
            return i18n_f(
                "Import failed with {} error(s). No connections were imported.",
                &[&result.errors.len().to_string()],
            );
        }

        // Nothing arrived and nothing failed, so neither the success nor the
        // failure wording is true. Saying "Successfully imported 0
        // connection(s)" above a list of warnings is what made the result page
        // contradict itself.
        if result.connections.is_empty() && result.groups.is_empty() && !result.warnings.is_empty()
        {
            return i18n_f(
                "Nothing was imported. See the {} warning(s) below.",
                &[&result.warnings.len().to_string()],
            );
        }

        let conn_count = result.connections.len().to_string();
        let group_count = result.groups.len().to_string();
        source_name.map_or_else(
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
        )
    }

    /// Translates one warning produced by `rustconn-core` into display text.
    ///
    /// `rustconn-core` is GUI-free and carries no i18n, so it hands the dialog
    /// a typed reason and the msgid literals live here, where `xgettext` can
    /// extract them. Every literal must stay byte-identical to
    /// `ImportWarning::message()` in `rustconn-core`.
    #[must_use]
    pub fn format_warning(warning: &ImportWarning) -> String {
        match warning {
            ImportWarning::PasswordsEncrypted { source_name } => i18n_f(
                "Passwords were not imported: {} keeps them encrypted inside the document. The affected connections ask for a password on connect.",
                &[source_name],
            ),
            ImportWarning::InlineCaCertificateSaved { path } => i18n_f(
                "Inline CA certificate saved to {}",
                &[&path.display().to_string()],
            ),
            ImportWarning::ConnectionAutomation {
                connection_name,
                task_count,
                expect_rule_count,
            } => i18n_f(
                "Connection '{}' has {} automation task(s) and {} expect rule(s) — review before running",
                &[
                    connection_name,
                    &task_count.to_string(),
                    &expect_rule_count.to_string(),
                ],
            ),
            ImportWarning::PortIgnored {
                connection_name,
                rejected_port,
                port,
            } => i18n_f(
                "Connection '{}': ignored unusable Port '{}', using port {}",
                &[connection_name, rejected_port, &port.to_string()],
            ),
            ImportWarning::CloudSyncImportMode => {
                i18n("Imported as Cloud Sync group (Import mode). Use Sync Now to keep it updated.")
            }
            ImportWarning::TemplatesNotImported { count } => i18n_f(
                "{} template(s) skipped (not supported in batch import)",
                &[&count.to_string()],
            ),
            ImportWarning::ClustersNotImported { count } => i18n_f(
                "{} cluster(s) skipped (not supported in batch import)",
                &[&count.to_string()],
            ),
            ImportWarning::VariablesNotImported { count } => i18n_f(
                "{} variable(s) skipped (not supported in batch import)",
                &[&count.to_string()],
            ),
        }
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

        // List warnings about limitations of the source format
        if !result.warnings.is_empty() {
            let _ = writeln!(
                details,
                "{}",
                i18n_f("Warnings ({}):", &[&result.warnings.len().to_string()])
            );
            for warning in &result.warnings {
                let _ = writeln!(details, "  • {}", Self::format_warning(warning));
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
                progress_label.set_text(&i18n_f("Importing from {}…", &[&display_name]));

                // Run import on a background thread to avoid blocking the GTK
                // main loop (file I/O, virsh subprocess, argon2 can take seconds).
                progress_bar.pulse();
                let progress_bar_c = progress_bar.clone();
                let progress_label_c = progress_label.clone();
                let result_label_c = result_label.clone();
                let result_details_c = result_details.clone();
                let result_cell_c = result_cell.clone();
                let stack_c = stack.clone();
                let btn_c = btn.clone();
                let source_id_owned = source_id.clone();
                crate::utils::spawn_blocking_with_callback(
                    move || Self::do_import_blocking(&source_id_owned),
                    move |result| {
                        progress_bar_c.set_fraction(1.0);
                        progress_label_c.set_text(&i18n("Import complete"));

                        // Show results — distinguish success from failure
                        result_label_c.set_text(&Self::format_summary(&result, None));

                        let details = Self::format_import_details(&result);
                        result_details_c.set_text(&details);

                        *result_cell_c.borrow_mut() = Some(result);
                        stack_c.set_visible_child_name("result");
                        btn_c.set_label(&i18n("Done"));
                        btn_c.set_sensitive(true);
                    },
                );
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
                progress_label.set_text(&i18n_f("Importing from {}…", &[&display_name]));

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

                // Run import on a background thread to avoid blocking
                // the GTK main loop.
                progress_bar.pulse();
                let progress_bar_c = progress_bar.clone();
                let progress_label_c = progress_label.clone();
                let result_label_c = result_label.clone();
                let result_details_c = result_details.clone();
                let result_cell_c = result_cell.clone();
                let source_name_cell_c = source_name_cell.clone();
                let stack_c = stack.clone();
                let btn_c = btn.clone();
                let display_name_c = display_name.clone();
                let source_id_owned = source_id.clone();
                crate::utils::spawn_blocking_with_callback(
                    move || Self::do_import_blocking(&source_id_owned),
                    move |result| {
                        // Store source name
                        *source_name_cell_c.borrow_mut() = display_name_c.clone();

                        progress_bar_c.set_fraction(1.0);
                        progress_label_c.set_text(&i18n("Import complete"));

                        // Show results — distinguish success from failure
                        result_label_c
                            .set_text(&Self::format_summary(&result, Some(&display_name_c)));

                        let details = Self::format_import_details(&result);
                        result_details_c.set_text(&details);

                        *result_cell_c.borrow_mut() = Some(result);
                        stack_c.set_visible_child_name("result");
                        btn_c.set_label(&i18n("Done"));
                        btn_c.set_sensitive(true);
                    },
                );
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
        reporter.report(0, 1, &i18n_f("Starting import from {}…", &[source_id]));

        let result = match source_id {
            "ssh_config" => {
                let importer = SshConfigImporter::new();
                let paths = importer.default_paths();
                let total = paths.len().max(1);

                for (i, path) in paths.iter().enumerate() {
                    reporter.report(
                        i,
                        total,
                        &i18n_f("Importing from {}…", &[&path.display().to_string()]),
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
                        &i18n_f("Importing from {}…", &[&path.display().to_string()]),
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
                        &i18n_f("Importing from {}…", &[&path.display().to_string()]),
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
                        &i18n_f("Importing from {}…", &[&path.display().to_string()]),
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
                        &i18n_f("Importing from {}…", &[&path.display().to_string()]),
                    );
                    if reporter.is_cancelled() {
                        return ImportResult::default();
                    }
                }

                Self::import_or_error(importer.import(), "Libvirt")
            }
            "libvirt_daemon" => {
                reporter.report(0, 1, &i18n("Querying libvirt daemon…"));
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rustconn_core::error::ImportError;
    use rustconn_core::import::{ImportResult, ImportWarning, SkippedEntry};
    use rustconn_core::models::{Connection, ProtocolConfig, SshConfig};

    use super::ImportDialog;

    // `format_import_details` and `format_summary` are pure `&ImportResult ->
    // String` functions, so none of these tests initialise GTK. `i18n()` with
    // no bound text domain returns the msgid, which is what the assertions
    // below compare against.

    fn ssh_connection(name: &str, host: &str, port: u16) -> Connection {
        Connection::new(
            name.to_string(),
            host.to_string(),
            port,
            ProtocolConfig::Ssh(SshConfig::default()),
        )
    }

    fn parse_error(reason: &str) -> ImportError {
        ImportError::ParseError {
            source_name: "Royal TS".to_string(),
            reason: reason.to_string(),
        }
    }

    #[test]
    fn details_list_imported_connections() {
        let mut result = ImportResult::new();
        result.add_connection(ssh_connection("web01", "web01.example.com", 22));
        result.add_connection(ssh_connection("db01", "db01.example.com", 2222));

        let details = ImportDialog::format_import_details(&result);
        assert!(details.contains("Imported connections:"), "{details}");
        assert!(
            details.contains("• web01 (web01.example.com:22)"),
            "{details}"
        );
        assert!(
            details.contains("• db01 (db01.example.com:2222)"),
            "{details}"
        );
        assert!(!details.contains("Warnings ("), "{details}");
        assert!(!details.contains("Errors ("), "{details}");
        assert!(!details.contains("No connections found"), "{details}");

        assert_eq!(
            ImportDialog::format_summary(&result, None),
            "Successfully imported 2 connection(s) and 0 group(s)."
        );
    }

    /// A result with nothing but warnings used to be summarised as
    /// "Successfully imported 0 connection(s) and 0 group(s)." above the very
    /// warnings explaining that nothing arrived.
    #[test]
    fn warnings_only_summary_agrees_with_the_details() {
        let mut result = ImportResult::new();
        result.add_warning(ImportWarning::PasswordsEncrypted {
            source_name: "Royal TS",
        });

        let summary = ImportDialog::format_summary(&result, Some("Royal TS"));
        assert!(!summary.contains("Successfully imported"), "{summary}");
        assert!(!summary.contains("Import failed"), "{summary}");
        assert_eq!(summary, "Nothing was imported. See the 1 warning(s) below.");

        let details = ImportDialog::format_import_details(&result);
        assert!(details.contains("Warnings (1):"), "{details}");
        assert!(
            details.contains("Passwords were not imported: Royal TS keeps them encrypted"),
            "{details}"
        );
        // The details are not empty, so the "nothing found" fallback must not
        // fire and contradict the warning list.
        assert!(!details.contains("No connections found"), "{details}");
    }

    #[test]
    fn details_list_warnings_and_errors_together() {
        let mut result = ImportResult::new();
        result.add_warning(ImportWarning::InlineCaCertificateSaved {
            path: PathBuf::from("/tmp/ca-0123.pem"),
        });
        result.add_error(parse_error("unexpected end of document"));

        let details = ImportDialog::format_import_details(&result);
        assert!(details.contains("Warnings (1):"), "{details}");
        assert!(
            details.contains("• Inline CA certificate saved to /tmp/ca-0123.pem"),
            "{details}"
        );
        assert!(details.contains("Errors (1):"), "{details}");
        assert!(details.contains("unexpected end of document"), "{details}");

        // An error outranks a warning: the import still failed.
        assert_eq!(
            ImportDialog::format_summary(&result, None),
            "Import failed with 1 error(s). No connections were imported."
        );
    }

    #[test]
    fn details_sections_keep_a_fixed_order() {
        let mut result = ImportResult::new();
        result.add_connection(ssh_connection("web01", "web01.example.com", 22));
        result.add_skipped(SkippedEntry::new("no-host", "Missing host"));
        result.add_warning(ImportWarning::TemplatesNotImported { count: 3 });
        result.add_error(parse_error("truncated"));

        let details = ImportDialog::format_import_details(&result);
        let offset = |needle: &str| {
            details
                .find(needle)
                .unwrap_or_else(|| panic!("missing section {needle} in:\n{details}"))
        };

        let connections = offset("Imported connections:");
        let skipped = offset("Skipped 1 entries:");
        let warnings = offset("Warnings (1):");
        let errors = offset("Errors (1):");

        assert!(
            connections < skipped && skipped < warnings && warnings < errors,
            "sections must read connections, skipped, warnings, errors:\n{details}"
        );
        assert!(
            details.contains("• 3 template(s) skipped (not supported in batch import)"),
            "{details}"
        );
    }

    #[test]
    fn empty_result_falls_back_to_the_nothing_found_message() {
        let result = ImportResult::new();

        assert_eq!(
            ImportDialog::format_import_details(&result),
            "No connections found in the selected source."
        );
        // Nothing at all still reads as a zero-count success. Left as-is:
        // the details already say nothing was found, and only the
        // warnings-only case was reported as self-contradictory.
        assert_eq!(
            ImportDialog::format_summary(&result, None),
            "Successfully imported 0 connection(s) and 0 group(s)."
        );
    }

    /// Every literal in `format_warning` must match `ImportWarning::message()`
    /// byte for byte, or `xgettext` extracts a msgid the dialog never looks up
    /// and the warning stays untranslated forever. Untranslated `i18n_f`
    /// substitutes placeholders exactly like the core `Display` impl, so
    /// equality here is the check.
    #[test]
    fn warning_literals_match_the_core_msgids() {
        let warnings = [
            ImportWarning::PasswordsEncrypted {
                source_name: "Royal TS",
            },
            ImportWarning::InlineCaCertificateSaved {
                path: PathBuf::from("/tmp/ca.pem"),
            },
            ImportWarning::ConnectionAutomation {
                connection_name: "web01".to_string(),
                task_count: 2,
                expect_rule_count: 1,
            },
            ImportWarning::PortIgnored {
                connection_name: "typo".to_string(),
                rejected_port: "70000".to_string(),
                port: 22,
            },
            ImportWarning::CloudSyncImportMode,
            ImportWarning::TemplatesNotImported { count: 1 },
            ImportWarning::ClustersNotImported { count: 2 },
            ImportWarning::VariablesNotImported { count: 3 },
        ];

        for warning in &warnings {
            assert_eq!(
                ImportDialog::format_warning(warning),
                warning.to_string(),
                "dialog literal drifted from the core msgid for {warning:?}"
            );
        }
    }
}
