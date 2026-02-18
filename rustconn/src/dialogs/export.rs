//! Export dialog for exporting connections to external formats
//!
//! Provides a GTK4 dialog with format selection, output path selection,
//! and options for exporting connections to Ansible, SSH Config, Remmina,
//! and Asbru-CM formats.
//!
//! Requirements: 3.1, 4.1, 5.1, 6.1

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, DropDown, Entry, FileDialog, Label, Orientation, ProgressBar,
    ScrolledWindow, Separator, Spinner, Stack, StringList,
};
use libadwaita as adw;
use rustconn_core::export::{
    AnsibleExporter, AsbruExporter, ExportFormat, ExportOptions, ExportResult, ExportTarget,
    MobaXtermExporter, NativeExport, RemminaExporter, RoyalTsExporter, SshConfigExporter,
};
use rustconn_core::models::{Connection, ConnectionGroup, Snippet};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use crate::i18n::i18n;

/// Callback type for export dialog completion
pub type ExportCallback = Rc<RefCell<Option<Box<dyn Fn(Option<ExportResult>)>>>>;

/// Export dialog for exporting connections to external formats
#[allow(dead_code)] // Fields kept for GTK widget lifecycle
pub struct ExportDialog {
    window: adw::Window,
    stack: Stack,
    // Format selection
    format_dropdown: DropDown,
    // Output path
    output_path_entry: Entry,
    browse_button: Button,
    // Options
    include_passwords_row: adw::SwitchRow,
    include_groups_row: adw::SwitchRow,
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
    snippets: Rc<RefCell<Vec<Snippet>>>,
    result: Rc<RefCell<Option<ExportResult>>>,
    on_complete: ExportCallback,
}

impl ExportDialog {
    /// Creates a new export dialog
    #[must_use]
    pub fn new(parent: Option<&gtk4::Window>) -> Self {
        // Create window
        let window = adw::Window::builder()
            .title(i18n("Export Connections"))
            .modal(true)
            .default_width(600)
            .default_height(500)
            .build();
        window.set_size_request(350, 300);

        if let Some(p) = parent {
            window.set_transient_for(Some(p));
        }

        // Create header bar with Close/Export buttons (GNOME HIG)
        let header = adw::HeaderBar::new();
        header.set_show_end_title_buttons(false);
        header.set_show_start_title_buttons(false);
        let close_btn = Button::builder().label(i18n("Close")).build();
        let export_button = Button::builder()
            .label(i18n("Export"))
            .css_classes(["suggested-action"])
            .build();
        header.pack_start(&close_btn);
        header.pack_end(&export_button);

        // Close button handler
        let window_clone = window.clone();
        close_btn.connect_clicked(move |_| {
            window_clone.close();
        });

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

        // Use ToolbarView for adw::Window
        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&content));
        window.set_content(Some(&toolbar_view));

        // === Options Page ===
        let (
            options_page,
            format_dropdown,
            output_path_entry,
            browse_button,
            include_passwords_row,
            include_groups_row,
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
            include_passwords_row,
            include_groups_row,
            progress_bar,
            progress_label,
            progress_spinner,
            result_label,
            result_details,
            export_button,
            connections: Rc::new(RefCell::new(Vec::new())),
            groups: Rc::new(RefCell::new(Vec::new())),
            snippets: Rc::new(RefCell::new(Vec::new())),
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
        adw::SwitchRow,
        adw::SwitchRow,
    ) {
        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .build();

        let clamp = adw::Clamp::builder()
            .maximum_size(600)
            .tightening_threshold(400)
            .build();

        let main_vbox = GtkBox::new(Orientation::Vertical, 12);
        main_vbox.set_margin_top(12);
        main_vbox.set_margin_bottom(12);
        main_vbox.set_margin_start(12);
        main_vbox.set_margin_end(12);
        main_vbox.set_valign(gtk4::Align::Start);

        // Format selection section using PreferencesGroup
        let format_group = adw::PreferencesGroup::builder()
            .title(i18n("Export Format"))
            .description(i18n("Select the format to export your connections to"))
            .build();

        // Create format dropdown with all available formats
        let format_list = StringList::new(&[
            "Ansible Inventory",
            "SSH Config",
            "Remmina",
            "Asbru-CM",
            "RustConn Native (.rcn)",
            "Royal TS (.rtsz)",
            "MobaXterm (.mxtsessions)",
        ]);
        let format_dropdown = DropDown::new(Some(format_list), gtk4::Expression::NONE);
        format_dropdown.set_selected(0);
        format_dropdown.set_valign(gtk4::Align::Center);

        let format_row = adw::ActionRow::builder()
            .title(i18n("Format"))
            .subtitle(i18n("Target export format"))
            .build();
        format_row.add_suffix(&format_dropdown);
        format_group.add(&format_row);

        main_vbox.append(&format_group);

        // Output path section using PreferencesGroup
        let output_group = adw::PreferencesGroup::builder()
            .title(i18n("Output Location"))
            .description(i18n(
                "Remmina exports to a directory (one file per connection).\n\
                 Other formats export to a single file.",
            ))
            .build();

        let output_path_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text(i18n("Select output file or directory..."))
            .editable(false)
            .valign(gtk4::Align::Center)
            .build();

        let browse_button = Button::builder()
            .label(i18n("Browse..."))
            .valign(gtk4::Align::Center)
            .build();

        let output_row = adw::ActionRow::builder()
            .title(i18n("Output"))
            .subtitle(i18n("Destination path"))
            .build();
        output_row.add_suffix(&output_path_entry);
        output_row.add_suffix(&browse_button);
        output_group.add(&output_row);

        main_vbox.append(&output_group);

        // Options section using PreferencesGroup
        let options_group = adw::PreferencesGroup::builder()
            .title(i18n("Options"))
            .build();

        // Include passwords switch row
        let include_passwords_row = adw::SwitchRow::builder()
            .title(i18n("Include passwords"))
            .subtitle(i18n("If supported by format"))
            .active(false)
            .build();
        options_group.add(&include_passwords_row);

        // Include groups switch row
        let include_groups_row = adw::SwitchRow::builder()
            .title(i18n("Include group hierarchy"))
            .subtitle(i18n("Preserve folder structure"))
            .active(true)
            .build();
        options_group.add(&include_groups_row);

        // Security warning row
        let warning_row = adw::ActionRow::builder()
            .title(i18n("⚠ Security Warning"))
            .subtitle(i18n("Including passwords may expose sensitive data. Only enable if you trust the destination."))
            .build();
        let warning_icon = gtk4::Image::from_icon_name("dialog-warning-symbolic");
        warning_icon.set_valign(gtk4::Align::Center);
        warning_icon.add_css_class("warning");
        warning_row.add_prefix(&warning_icon);
        options_group.add(&warning_row);

        // Credentials info row
        let creds_info_row = adw::ActionRow::builder()
            .title(i18n("ℹ Credentials Storage"))
            .subtitle(i18n("Passwords are stored in your password manager and not included in exports by default. Export your credential structure separately if needed."))
            .build();
        let info_icon = gtk4::Image::from_icon_name("dialog-information-symbolic");
        info_icon.set_valign(gtk4::Align::Center);
        creds_info_row.add_prefix(&info_icon);
        options_group.add(&creds_info_row);

        main_vbox.append(&options_group);

        clamp.set_child(Some(&main_vbox));
        scrolled.set_child(Some(&clamp));

        (
            scrolled,
            format_dropdown,
            output_path_entry,
            browse_button,
            include_passwords_row,
            include_groups_row,
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
            .label(i18n("Exporting..."))
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
            .label(i18n("Preparing export..."))
            .css_classes(["dim-label"])
            .build();
        vbox.append(&progress_label);

        (vbox, progress_bar, progress_label, spinner)
    }

    /// Creates the result page
    fn create_result_page() -> (GtkBox, Label, Label) {
        let vbox = GtkBox::new(Orientation::Vertical, 12);

        let header = Label::builder()
            .label(i18n("Export Complete"))
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
            6 => ExportFormat::MobaXterm,
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
                .with_passwords(self.include_passwords_row.is_active())
                .with_groups(self.include_groups_row.is_active())
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

    /// Sets the snippets for export (used in native format)
    pub fn set_snippets(&self, snippets: Vec<Snippet>) {
        *self.snippets.borrow_mut() = snippets;
    }

    /// Performs the export operation
    fn do_export(
        connections: &[Connection],
        groups: &[ConnectionGroup],
        snippets: &[Snippet],
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
                    snippets.to_vec(),
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
            ExportFormat::MobaXterm => {
                let exporter = MobaXtermExporter;
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
                6 => ExportFormat::MobaXterm,
                _ => ExportFormat::Ansible,
            };

            let output_entry = output_path_entry.clone();

            if format.exports_to_directory() {
                // Use folder dialog for Remmina
                let dialog = FileDialog::builder()
                    .title(i18n("Select Export Directory"))
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
                    .title(i18n("Select Export File"))
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
                    ExportFormat::MobaXterm => {
                        filter.add_pattern("*.mxtsessions");
                        filter.set_name(Some("MobaXterm Sessions (*.mxtsessions)"));
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
                    6 => ExportFormat::MobaXterm,
                    _ => ExportFormat::Ansible,
                };

                // Update placeholder text based on format
                if format.exports_to_directory() {
                    output_path_entry
                        .set_placeholder_text(Some(&i18n("Select output directory...")));
                } else {
                    output_path_entry.set_placeholder_text(Some(&i18n("Select output file...")));
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
        let include_passwords = self.include_passwords_row.clone();
        let include_groups = self.include_groups_row.clone();
        let progress_bar = self.progress_bar.clone();
        let progress_label = self.progress_label.clone();
        let progress_spinner = self.progress_spinner.clone();
        let result_label = self.result_label.clone();
        let result_details = self.result_details.clone();
        let export_button = self.export_button.clone();
        let connections = self.connections.clone();
        let groups = self.groups.clone();
        let snippets = self.snippets.clone();
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
                // Show error using toast instead of AlertDialog
                crate::toast::show_toast_on_window(
                    &window,
                    &i18n("Please select an output file or directory"),
                    crate::toast::ToastType::Warning,
                );
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
                6 => ExportFormat::MobaXterm,
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
            let snips = snippets.borrow();

            progress_bar.set_fraction(0.5);
            progress_label.set_text("Writing output files...");

            let export_result = Self::do_export(&conns, &grps, &snips, &options);

            progress_bar.set_fraction(1.0);
            progress_spinner.set_spinning(false);

            match export_result {
                Ok(result) => {
                    progress_label.set_text(&i18n("Export complete"));

                    // Show results using helper method
                    let summary = Self::format_result_summary(&result, format);
                    result_label.set_text(&summary);

                    let details = Self::format_export_details(&result);
                    result_details.set_text(&details);

                    *result_cell.borrow_mut() = Some(result);
                    stack.set_visible_child_name("result");
                    btn.set_label(&i18n("Done"));
                    btn.set_sensitive(true);
                }
                Err(e) => {
                    // Show error
                    progress_label.set_text(&i18n("Export failed"));
                    result_label.set_text(&i18n("Export Failed"));
                    result_details.set_text(&format!("Error: {e}"));

                    stack.set_visible_child_name("result");
                    btn.set_label(&i18n("Close"));
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
