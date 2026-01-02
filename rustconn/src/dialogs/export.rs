//! Export dialog for exporting connections to external formats
//!
//! Provides a GTK4 dialog with format selection, output path selection,
//! and options for exporting connections to Ansible, SSH Config, Remmina,
//! and Asbru-CM formats.
//!
//! Requirements: 3.1, 4.1, 5.1, 6.1

use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, CheckButton, DropDown, Entry, FileDialog, Frame, Grid, HeaderBar, Label,
    Orientation, ProgressBar, ScrolledWindow, Separator, Spinner, Stack, StringList, Window,
};
use rustconn_core::export::{
    AnsibleExporter, AsbruExporter, ExportFormat, ExportOptions, ExportResult, ExportTarget,
    NativeExport, RemminaExporter, RoyalTsExporter, SshConfigExporter,
};
use rustconn_core::models::{Connection, ConnectionGroup};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

/// Callback type for export dialog completion
pub type ExportCallback = Rc<RefCell<Option<Box<dyn Fn(Option<ExportResult>)>>>>;

/// Export dialog for exporting connections to external formats
#[allow(dead_code)] // Fields kept for GTK widget lifecycle
pub struct ExportDialog {
    window: Window,
    stack: Stack,
    // Format selection
    format_dropdown: DropDown,
    // Output path
    output_path_entry: Entry,
    browse_button: Button,
    // Options
    include_passwords_check: CheckButton,
    include_groups_check: CheckButton,
    // Progress
    progress_bar: ProgressBar,
    progress_label: Label,
    progress_spinner: Spinner,
    // Result
    result_label: Label,
    result_details: Label,
    // Buttons
    export_button: Button,
    // State
    connections: Rc<RefCell<Vec<Connection>>>,
    groups: Rc<RefCell<Vec<ConnectionGroup>>>,
    result: Rc<RefCell<Option<ExportResult>>>,
    on_complete: ExportCallback,
}

impl ExportDialog {
    /// Creates a new export dialog
    #[must_use]
    pub fn new(parent: Option<&Window>) -> Self {
        // Create window
        let window = Window::builder()
            .title("Export Connections")
            .modal(true)
            .default_width(750)
            .default_height(500)
            .build();

        if let Some(p) = parent {
            window.set_transient_for(Some(p));
        }

        // Create header bar (no Cancel button - window X is sufficient)
        let header = HeaderBar::new();
        let export_button = Button::builder()
            .label("Export")
            .css_classes(["suggested-action"])
            .build();
        header.pack_start(&export_button);
        window.set_titlebar(Some(&header));

        // Create main content area
        let content = GtkBox::new(Orientation::Vertical, 0);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // Create stack for different views
        let stack = Stack::new();
        stack.set_vexpand(true);
        content.append(&stack);
        window.set_child(Some(&content));

        // === Options Page ===
        let (
            options_page,
            format_dropdown,
            output_path_entry,
            browse_button,
            include_passwords_check,
            include_groups_check,
        ) = Self::create_options_page();
        stack.add_named(&options_page, Some("options"));

        // === Progress Page ===
        let (progress_page, progress_bar, progress_label, progress_spinner) =
            Self::create_progress_page();
        stack.add_named(&progress_page, Some("progress"));

        // === Result Page ===
        let (result_page, result_label, result_details) = Self::create_result_page();
        stack.add_named(&result_page, Some("result"));

        // Set initial page
        stack.set_visible_child_name("options");

        let on_complete: ExportCallback = Rc::new(RefCell::new(None));

        Self {
            window,
            stack,
            format_dropdown,
            output_path_entry,
            browse_button,
            include_passwords_check,
            include_groups_check,
            progress_bar,
            progress_label,
            progress_spinner,
            result_label,
            result_details,
            export_button,
            connections: Rc::new(RefCell::new(Vec::new())),
            groups: Rc::new(RefCell::new(Vec::new())),
            result: Rc::new(RefCell::new(None)),
            on_complete,
        }
    }

    /// Creates the options page with format selection and output path
    #[allow(clippy::type_complexity)]
    fn create_options_page() -> (
        ScrolledWindow,
        DropDown,
        Entry,
        Button,
        CheckButton,
        CheckButton,
    ) {
        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .build();

        let main_vbox = GtkBox::new(Orientation::Vertical, 12);
        main_vbox.set_margin_top(12);
        main_vbox.set_margin_bottom(12);
        main_vbox.set_margin_start(12);
        main_vbox.set_margin_end(12);
        main_vbox.set_valign(gtk4::Align::Start);

        // Format selection section
        let format_frame = Frame::builder().label("Export Format").build();
        let format_vbox = GtkBox::new(Orientation::Vertical, 8);
        format_vbox.set_margin_top(8);
        format_vbox.set_margin_bottom(8);
        format_vbox.set_margin_start(8);
        format_vbox.set_margin_end(8);

        let format_grid = Grid::builder().row_spacing(8).column_spacing(12).build();

        let format_label = Label::builder()
            .label("Format:")
            .halign(gtk4::Align::End)
            .build();

        // Create format dropdown with all available formats
        let format_list = StringList::new(&[
            "Ansible Inventory",
            "SSH Config",
            "Remmina",
            "Asbru-CM",
            "RustConn Native (.rcn)",
            "Royal TS (.rtsz)",
        ]);
        let format_dropdown = DropDown::new(Some(format_list), gtk4::Expression::NONE);
        format_dropdown.set_selected(0);
        format_dropdown.set_hexpand(true);

        format_grid.attach(&format_label, 0, 0, 1, 1);
        format_grid.attach(&format_dropdown, 1, 0, 1, 1);

        // Format description
        let format_desc = Label::builder()
            .label("Select the format to export your connections to.")
            .halign(gtk4::Align::Start)
            .wrap(true)
            .css_classes(["dim-label"])
            .margin_top(4)
            .build();

        format_vbox.append(&format_grid);
        format_vbox.append(&format_desc);
        format_frame.set_child(Some(&format_vbox));
        main_vbox.append(&format_frame);

        // Output path section
        let output_frame = Frame::builder().label("Output Location").build();
        let output_vbox = GtkBox::new(Orientation::Vertical, 8);
        output_vbox.set_margin_top(8);
        output_vbox.set_margin_bottom(8);
        output_vbox.set_margin_start(8);
        output_vbox.set_margin_end(8);

        let output_grid = Grid::builder().row_spacing(8).column_spacing(8).build();

        let output_label = Label::builder()
            .label("Output:")
            .halign(gtk4::Align::End)
            .build();

        let output_path_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("Select output file or directory...")
            .editable(false)
            .build();

        let browse_button = Button::builder().label("Browse...").build();

        output_grid.attach(&output_label, 0, 0, 1, 1);
        output_grid.attach(&output_path_entry, 1, 0, 1, 1);
        output_grid.attach(&browse_button, 2, 0, 1, 1);

        let output_hint = Label::builder()
            .label("Remmina exports to a directory (one file per connection).\nOther formats export to a single file.")
            .halign(gtk4::Align::Start)
            .wrap(true)
            .css_classes(["dim-label"])
            .margin_top(4)
            .build();

        output_vbox.append(&output_grid);
        output_vbox.append(&output_hint);
        output_frame.set_child(Some(&output_vbox));
        main_vbox.append(&output_frame);

        // Options section
        let options_frame = Frame::builder().label("Options").build();
        let options_vbox = GtkBox::new(Orientation::Vertical, 8);
        options_vbox.set_margin_top(8);
        options_vbox.set_margin_bottom(8);
        options_vbox.set_margin_start(8);
        options_vbox.set_margin_end(8);

        let include_passwords_check = CheckButton::builder()
            .label("Include passwords (if supported by format)")
            .active(false)
            .build();

        let include_groups_check = CheckButton::builder()
            .label("Include group hierarchy")
            .active(true)
            .build();

        let security_warning = Label::builder()
            .label("⚠ Warning: Including passwords may expose sensitive data.\nOnly enable if you trust the destination.")
            .halign(gtk4::Align::Start)
            .wrap(true)
            .css_classes(["dim-label"])
            .margin_top(8)
            .build();

        options_vbox.append(&include_passwords_check);
        options_vbox.append(&include_groups_check);
        options_vbox.append(&security_warning);
        options_frame.set_child(Some(&options_vbox));
        main_vbox.append(&options_frame);

        scrolled.set_child(Some(&main_vbox));

        (
            scrolled,
            format_dropdown,
            output_path_entry,
            browse_button,
            include_passwords_check,
            include_groups_check,
        )
    }

    /// Creates the progress page
    fn create_progress_page() -> (GtkBox, ProgressBar, Label, Spinner) {
        let vbox = GtkBox::new(Orientation::Vertical, 12);
        vbox.set_valign(gtk4::Align::Center);
        vbox.set_halign(gtk4::Align::Center);

        let spinner = Spinner::builder()
            .spinning(true)
            .width_request(48)
            .height_request(48)
            .build();
        vbox.append(&spinner);

        let header = Label::builder()
            .label("Exporting...")
            .css_classes(["title-3"])
            .build();
        vbox.append(&header);

        let progress_bar = ProgressBar::builder()
            .show_text(true)
            .margin_top(12)
            .margin_bottom(12)
            .width_request(300)
            .build();
        vbox.append(&progress_bar);

        let progress_label = Label::builder()
            .label("Preparing export...")
            .css_classes(["dim-label"])
            .build();
        vbox.append(&progress_label);

        (vbox, progress_bar, progress_label, spinner)
    }

    /// Creates the result page
    fn create_result_page() -> (GtkBox, Label, Label) {
        let vbox = GtkBox::new(Orientation::Vertical, 12);

        let header = Label::builder()
            .label("Export Complete")
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

    /// Gets the selected export format
    #[must_use]
    pub fn get_selected_format(&self) -> ExportFormat {
        match self.format_dropdown.selected() {
            0 => ExportFormat::Ansible,
            1 => ExportFormat::SshConfig,
            2 => ExportFormat::Remmina,
            3 => ExportFormat::Asbru,
            4 => ExportFormat::Native,
            5 => ExportFormat::RoyalTs,
            _ => ExportFormat::Ansible,
        }
    }

    /// Gets the output path
    #[must_use]
    pub fn get_output_path(&self) -> Option<PathBuf> {
        let text = self.output_path_entry.text();
        if text.is_empty() {
            None
        } else {
            Some(PathBuf::from(text.as_str()))
        }
    }

    /// Gets the export options
    #[must_use]
    pub fn get_export_options(&self) -> Option<ExportOptions> {
        self.get_output_path().map(|output_path| {
            ExportOptions::new(self.get_selected_format(), output_path)
                .with_passwords(self.include_passwords_check.is_active())
                .with_groups(self.include_groups_check.is_active())
        })
    }

    /// Sets the connections to export
    pub fn set_connections(&self, connections: Vec<Connection>) {
        *self.connections.borrow_mut() = connections;
    }

    /// Sets the groups for export
    pub fn set_groups(&self, groups: Vec<ConnectionGroup>) {
        *self.groups.borrow_mut() = groups;
    }

    /// Performs the export operation
    fn do_export(
        connections: &[Connection],
        groups: &[ConnectionGroup],
        options: &ExportOptions,
    ) -> Result<ExportResult, String> {
        match options.format {
            ExportFormat::Ansible => {
                let exporter = AnsibleExporter;
                exporter
                    .export(connections, groups, options)
                    .map_err(|e| e.to_string())
            }
            ExportFormat::SshConfig => {
                let exporter = SshConfigExporter;
                exporter
                    .export(connections, groups, options)
                    .map_err(|e| e.to_string())
            }
            ExportFormat::Remmina => {
                let exporter = RemminaExporter;
                exporter
                    .export(connections, groups, options)
                    .map_err(|e| e.to_string())
            }
            ExportFormat::Asbru => {
                let exporter = AsbruExporter;
                exporter
                    .export(connections, groups, options)
                    .map_err(|e| e.to_string())
            }
            ExportFormat::Native => {
                // Native export includes all data types
                let export = NativeExport::with_data(
                    connections.to_vec(),
                    groups.to_vec(),
                    Vec::new(), // Templates would need to be passed in
                    Vec::new(), // Clusters would need to be passed in
                    Vec::new(), // Variables would need to be passed in
                );
                export
                    .to_file(&options.output_path)
                    .map_err(|e| e.to_string())?;

                let mut result = ExportResult::new();
                result.exported_count = connections.len();
                result.add_output_file(options.output_path.clone());
                Ok(result)
            }
            ExportFormat::RoyalTs => {
                let exporter = RoyalTsExporter;
                exporter
                    .export(connections, groups, options)
                    .map_err(|e| e.to_string())
            }
        }
    }

    /// Formats the result summary message
    fn format_result_summary(result: &ExportResult, format: ExportFormat) -> String {
        let summary = format!(
            "Successfully exported {} connection(s) to {} format.",
            result.exported_count,
            format.display_name()
        );

        if result.skipped_count > 0 {
            format!(
                "{}\n\n{} connection(s) were skipped (unsupported protocol).",
                summary, result.skipped_count
            )
        } else {
            summary
        }
    }

    /// Formats export result details into a displayable string
    #[must_use]
    pub fn format_export_details(result: &ExportResult) -> String {
        use std::fmt::Write;
        let mut details = String::new();

        // List output files
        if !result.output_files.is_empty() {
            details.push_str("Output files:\n");
            for file in &result.output_files {
                let _ = writeln!(details, "  • {}", file.display());
            }
            details.push('\n');
        }

        // Summary
        let _ = writeln!(details, "Summary:");
        let _ = writeln!(details, "  • Exported: {}", result.exported_count);
        let _ = writeln!(details, "  • Skipped: {}", result.skipped_count);

        // List warnings
        if !result.warnings.is_empty() {
            details.push('\n');
            let _ = writeln!(details, "Warnings ({}):", result.warnings.len());
            for warning in &result.warnings {
                let _ = writeln!(details, "  • {warning}");
            }
        }

        if details.is_empty() {
            details = "No connections were exported.".to_string();
        }

        details
    }

    /// Runs the dialog and calls the callback with the result
    pub fn run<F: Fn(Option<ExportResult>) + 'static>(&self, cb: F) {
        // Store callback
        *self.on_complete.borrow_mut() = Some(Box::new(cb));

        // Connect browse button
        self.connect_browse_button();

        // Connect format dropdown to update output path hint
        self.connect_format_change();

        // Connect export button
        self.connect_export_button();

        self.window.present();
    }

    /// Connects the browse button to show file/folder dialog
    fn connect_browse_button(&self) {
        let format_dropdown = self.format_dropdown.clone();
        let output_path_entry = self.output_path_entry.clone();
        let window = self.window.clone();

        self.browse_button.connect_clicked(move |_| {
            let format = match format_dropdown.selected() {
                0 => ExportFormat::Ansible,
                1 => ExportFormat::SshConfig,
                2 => ExportFormat::Remmina,
                3 => ExportFormat::Asbru,
                4 => ExportFormat::Native,
                5 => ExportFormat::RoyalTs,
                _ => ExportFormat::Ansible,
            };

            let output_entry = output_path_entry.clone();

            if format.exports_to_directory() {
                // Use folder dialog for Remmina
                let dialog = FileDialog::builder()
                    .title("Select Export Directory")
                    .modal(true)
                    .build();

                dialog.select_folder(Some(&window), gtk4::gio::Cancellable::NONE, move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            output_entry.set_text(&path.to_string_lossy());
                        }
                    }
                });
            } else {
                // Use file dialog for other formats
                let dialog = FileDialog::builder()
                    .title("Select Export File")
                    .modal(true)
                    .build();

                // Set default filename based on format
                let default_name = format!("rustconn-export.{}", format.file_extension());
                dialog.set_initial_name(Some(&default_name));

                // Set filter based on format
                let filter = gtk4::FileFilter::new();
                match format {
                    ExportFormat::Ansible => {
                        filter.add_pattern("*.ini");
                        filter.add_pattern("*.yml");
                        filter.add_pattern("*.yaml");
                        filter.set_name(Some("Ansible Inventory (*.ini, *.yml)"));
                    }
                    ExportFormat::SshConfig => {
                        filter.add_pattern("*");
                        filter.set_name(Some("SSH Config"));
                    }
                    ExportFormat::Asbru => {
                        filter.add_pattern("*.yml");
                        filter.add_pattern("*.yaml");
                        filter.set_name(Some("Asbru-CM YAML (*.yml)"));
                    }
                    ExportFormat::Remmina => {
                        // Should not reach here
                        filter.add_pattern("*.remmina");
                        filter.set_name(Some("Remmina (*.remmina)"));
                    }
                    ExportFormat::Native => {
                        filter.add_pattern("*.rcn");
                        filter.set_name(Some("RustConn Native (*.rcn)"));
                    }
                    ExportFormat::RoyalTs => {
                        filter.add_pattern("*.rtsz");
                        filter.set_name(Some("Royal TS (*.rtsz)"));
                    }
                }

                let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
                filters.append(&filter);
                dialog.set_filters(Some(&filters));

                dialog.save(Some(&window), gtk4::gio::Cancellable::NONE, move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            output_entry.set_text(&path.to_string_lossy());
                        }
                    }
                });
            }
        });
    }

    /// Connects format dropdown change to update UI hints
    fn connect_format_change(&self) {
        let output_path_entry = self.output_path_entry.clone();

        self.format_dropdown
            .connect_selected_notify(move |dropdown| {
                let format = match dropdown.selected() {
                    0 => ExportFormat::Ansible,
                    1 => ExportFormat::SshConfig,
                    2 => ExportFormat::Remmina,
                    3 => ExportFormat::Asbru,
                    4 => ExportFormat::Native,
                    5 => ExportFormat::RoyalTs,
                    _ => ExportFormat::Ansible,
                };

                // Update placeholder text based on format
                if format.exports_to_directory() {
                    output_path_entry.set_placeholder_text(Some("Select output directory..."));
                } else {
                    output_path_entry.set_placeholder_text(Some("Select output file..."));
                }

                // Clear current path when format changes
                output_path_entry.set_text("");
            });
    }

    /// Connects the export button to perform export
    fn connect_export_button(&self) {
        let window = self.window.clone();
        let stack = self.stack.clone();
        let format_dropdown = self.format_dropdown.clone();
        let output_path_entry = self.output_path_entry.clone();
        let include_passwords = self.include_passwords_check.clone();
        let include_groups = self.include_groups_check.clone();
        let progress_bar = self.progress_bar.clone();
        let progress_label = self.progress_label.clone();
        let progress_spinner = self.progress_spinner.clone();
        let result_label = self.result_label.clone();
        let result_details = self.result_details.clone();
        let export_button = self.export_button.clone();
        let connections = self.connections.clone();
        let groups = self.groups.clone();
        let result_cell = self.result.clone();
        let on_complete = self.on_complete.clone();

        export_button.connect_clicked(move |btn| {
            let current_page = stack.visible_child_name();

            if current_page.as_deref() == Some("result") {
                // Done - close dialog and optionally open output location
                if let Some(ref cb) = *on_complete.borrow() {
                    cb(result_cell.borrow_mut().take());
                }
                window.close();
                return;
            }

            // Validate output path
            let output_text = output_path_entry.text();
            if output_text.is_empty() {
                // Show error
                let alert = gtk4::AlertDialog::builder()
                    .message("No Output Path")
                    .detail("Please select an output file or directory.")
                    .modal(true)
                    .build();
                alert.show(Some(&window));
                return;
            }

            let output_path = PathBuf::from(output_text.as_str());
            let format = match format_dropdown.selected() {
                0 => ExportFormat::Ansible,
                1 => ExportFormat::SshConfig,
                2 => ExportFormat::Remmina,
                3 => ExportFormat::Asbru,
                4 => ExportFormat::Native,
                5 => ExportFormat::RoyalTs,
                _ => ExportFormat::Ansible,
            };

            let options = ExportOptions::new(format, output_path.clone())
                .with_passwords(include_passwords.is_active())
                .with_groups(include_groups.is_active());

            // Show progress page
            stack.set_visible_child_name("progress");
            btn.set_sensitive(false);
            progress_bar.set_fraction(0.0);
            progress_spinner.set_spinning(true);
            progress_label.set_text(&format!("Exporting to {}...", format.display_name()));

            // Perform export
            let conns = connections.borrow();
            let grps = groups.borrow();

            progress_bar.set_fraction(0.5);
            progress_label.set_text("Writing output files...");

            let export_result = Self::do_export(&conns, &grps, &options);

            progress_bar.set_fraction(1.0);
            progress_spinner.set_spinning(false);

            match export_result {
                Ok(result) => {
                    progress_label.set_text("Export complete");

                    // Show results using helper method
                    let summary = Self::format_result_summary(&result, format);
                    result_label.set_text(&summary);

                    let details = Self::format_export_details(&result);
                    result_details.set_text(&details);

                    *result_cell.borrow_mut() = Some(result);
                    stack.set_visible_child_name("result");
                    btn.set_label("Done");
                    btn.set_sensitive(true);
                }
                Err(e) => {
                    // Show error
                    progress_label.set_text("Export failed");
                    result_label.set_text("Export Failed");
                    result_details.set_text(&format!("Error: {e}"));

                    stack.set_visible_child_name("result");
                    btn.set_label("Close");
                    btn.set_sensitive(true);
                }
            }
        });
    }

    /// Opens the output location in the file manager
    pub fn open_output_location(path: &std::path::Path) {
        // For directories, open the directory
        // For files, open the parent directory
        let dir_to_open = if path.is_dir() {
            path.to_path_buf()
        } else {
            path.parent()
                .map(std::path::Path::to_path_buf)
                .unwrap_or_else(|| path.to_path_buf())
        };

        if let Err(e) = open::that(&dir_to_open) {
            eprintln!("Failed to open output location: {e}");
        }
    }
}
