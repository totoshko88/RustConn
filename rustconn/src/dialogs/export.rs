//! Export dialog for exporting connections to external formats
//!
//! Provides a GTK4 dialog with format selection, output path selection,
//! and options for exporting connections to Ansible, SSH Config, Remmina,
//! and Asbru-CM formats.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, DropDown, Entry, FileDialog, Label, Orientation, ProgressBar,
    ScrolledWindow, Separator, Stack, StringList,
};
use libadwaita as adw;
use rustconn_core::cluster::Cluster;
use rustconn_core::export::{
    AnsibleExporter, AsbruExporter, CsvExportField, CsvExportOptions, CsvExporter, ExportFormat,
    ExportOptions, ExportResult, ExportTarget, MobaXtermExporter, NativeExport, RemminaExporter,
    RoyalTsExporter, SecureCrtExporter, SshConfigExporter,
};
use rustconn_core::models::{
    Connection, ConnectionGroup, ConnectionTemplate, SmartFolder, Snippet,
};
use rustconn_core::variables::Variable;

use crate::i18n::{i18n, i18n_f};

/// Callback type for export dialog completion
pub type ExportCallback = Rc<RefCell<Option<Box<dyn Fn(Option<ExportResult>)>>>>;

/// Export dialog for exporting connections to external formats
pub struct ExportDialog {
    dialog: adw::Dialog,
    stack: Stack,
    // Format selection
    format_dropdown: DropDown,
    // Output path
    output_path_entry: Entry,
    browse_button: Button,
    // Options
    include_groups_row: adw::SwitchRow,
    // Progress
    progress_bar: ProgressBar,
    progress_label: Label,
    #[cfg(feature = "adw-1-6")]
    #[expect(
        dead_code,
        reason = "adw::Spinner spins automatically; field keeps widget alive"
    )]
    progress_spinner: adw::Spinner,
    #[cfg(not(feature = "adw-1-6"))]
    progress_spinner: gtk4::Spinner,
    // Result
    result_label: Label,
    result_details: Label,
    // CSV-specific options
    csv_options_group: adw::PreferencesGroup,
    csv_delimiter_dropdown: DropDown,
    csv_field_name: adw::SwitchRow,
    csv_field_host: adw::SwitchRow,
    csv_field_port: adw::SwitchRow,
    csv_field_protocol: adw::SwitchRow,
    csv_field_username: adw::SwitchRow,
    csv_field_group: adw::SwitchRow,
    csv_field_tags: adw::SwitchRow,
    csv_field_description: adw::SwitchRow,
    // Group filter
    group_filter_dropdown: DropDown,
    /// Flat list of (group_id, display_name) for the filter dropdown.
    /// Index 0 is always `(None, "All connections")`.
    groups_filter_data: Rc<RefCell<Vec<(Option<uuid::Uuid>, String)>>>,
    // Buttons
    export_button: Button,
    // State
    connections: Rc<RefCell<Vec<Connection>>>,
    groups: Rc<RefCell<Vec<ConnectionGroup>>>,
    snippets: Rc<RefCell<Vec<Snippet>>>,
    smart_folders: Rc<RefCell<Vec<SmartFolder>>>,
    templates: Rc<RefCell<Vec<ConnectionTemplate>>>,
    clusters: Rc<RefCell<Vec<Cluster>>>,
    variables: Rc<RefCell<Vec<Variable>>>,
    result: Rc<RefCell<Option<ExportResult>>>,
    on_complete: ExportCallback,
    parent: Option<gtk4::Window>,
}

impl ExportDialog {
    /// Creates a new export dialog
    #[must_use]
    pub fn new(parent: Option<&gtk4::Window>) -> Self {
        // Create dialog
        let dialog = adw::Dialog::builder()
            .title(i18n("Export Connections"))
            .content_width(600)
            .content_height(650)
            .build();

        // Header bar with Export icon button and standard window buttons (GNOME HIG)
        let header = adw::HeaderBar::new();
        let export_button = Button::from_icon_name("media-floppy-symbolic");
        export_button.set_tooltip_text(Some(&i18n("Export")));
        export_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Export"))]);
        export_button.add_css_class("suggested-action");
        header.pack_start(&export_button);

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

        // Use ToolbarView for adw::Dialog
        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&content));
        dialog.set_child(Some(&toolbar_view));

        // === Options Page ===
        let (
            options_page,
            format_dropdown,
            output_path_entry,
            browse_button,
            include_groups_row,
            csv_options_group,
            csv_delimiter_dropdown,
            csv_field_name,
            csv_field_host,
            csv_field_port,
            csv_field_protocol,
            csv_field_username,
            csv_field_group,
            csv_field_tags,
            csv_field_description,
            group_filter_dropdown,
        ) = Self::create_options_page();
        stack.add_named(&options_page, Some("options"));

        // === Progress Page ===
        let (progress_page, progress_bar, progress_label) = Self::create_progress_page();
        // Create spinner with cfg guard — adw::Spinner always spins (visibility-controlled),
        // gtk4::Spinner uses set_spinning()
        #[cfg(feature = "adw-1-6")]
        let progress_spinner = adw::Spinner::builder()
            .width_request(48)
            .height_request(48)
            .build();
        #[cfg(not(feature = "adw-1-6"))]
        let progress_spinner = gtk4::Spinner::builder()
            .spinning(true)
            .width_request(48)
            .height_request(48)
            .build();
        // Prepend spinner before the "Exporting..." label
        progress_page.prepend(&progress_spinner);
        stack.add_named(&progress_page, Some("progress"));

        // === Result Page ===
        let (result_page, result_label, result_details) = Self::create_result_page();
        stack.add_named(&result_page, Some("result"));

        // Set initial page
        stack.set_visible_child_name("options");

        let on_complete: ExportCallback = Rc::new(RefCell::new(None));

        Self {
            dialog,
            stack,
            format_dropdown,
            output_path_entry,
            browse_button,
            include_groups_row,
            progress_bar,
            progress_label,
            progress_spinner,
            result_label,
            result_details,
            csv_options_group,
            csv_delimiter_dropdown,
            csv_field_name,
            csv_field_host,
            csv_field_port,
            csv_field_protocol,
            csv_field_username,
            csv_field_group,
            csv_field_tags,
            csv_field_description,
            group_filter_dropdown,
            groups_filter_data: Rc::new(RefCell::new(vec![(None, i18n("All connections"))])),
            export_button,
            connections: Rc::new(RefCell::new(Vec::new())),
            groups: Rc::new(RefCell::new(Vec::new())),
            snippets: Rc::new(RefCell::new(Vec::new())),
            smart_folders: Rc::new(RefCell::new(Vec::new())),
            templates: Rc::new(RefCell::new(Vec::new())),
            clusters: Rc::new(RefCell::new(Vec::new())),
            variables: Rc::new(RefCell::new(Vec::new())),
            result: Rc::new(RefCell::new(None)),
            on_complete,
            parent: parent.cloned(),
        }
    }

    /// Creates the options page with format selection and output path
    #[expect(
        clippy::type_complexity,
        reason = "internal helper signature documents the exact tuple layout used by the caller; aliasing would obscure the data flow"
    )]
    fn create_options_page() -> (
        ScrolledWindow,
        DropDown,
        Entry,
        Button,
        adw::SwitchRow,
        adw::PreferencesGroup,
        DropDown,
        adw::SwitchRow,
        adw::SwitchRow,
        adw::SwitchRow,
        adw::SwitchRow,
        adw::SwitchRow,
        adw::SwitchRow,
        adw::SwitchRow,
        adw::SwitchRow,
        DropDown,
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
        // Native (.rcn) is the default; remaining formats sorted alphabetically
        let format_list = StringList::new(&[
            &i18n("RustConn Native (.rcn)"),
            &i18n("Ansible Inventory"),
            &i18n("Asbru-CM"),
            &i18n("CSV (*.csv)"),
            &i18n("MobaXterm (.mxtsessions)"),
            &i18n("Remmina"),
            &i18n("Royal TS (.rtsz)"),
            &i18n("SecureCRT (.ini)"),
            &i18n("SSH Config"),
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

        // Group filter section — select which group subtree to export
        let scope_group = adw::PreferencesGroup::builder()
            .title(i18n("Export Scope"))
            .description(i18n("Choose which connections to export"))
            .build();

        let group_filter_list = StringList::new(&[&i18n("All connections")]);
        let group_filter_dropdown = DropDown::new(Some(group_filter_list), gtk4::Expression::NONE);
        group_filter_dropdown.set_selected(0);
        group_filter_dropdown.set_valign(gtk4::Align::Center);

        let scope_row = adw::ActionRow::builder()
            .title(i18n("Group"))
            .subtitle(i18n("Export all or a specific group"))
            .build();
        scope_row.add_suffix(&group_filter_dropdown);
        scope_group.add(&scope_row);

        main_vbox.append(&scope_group);

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

        // Include groups switch row
        let include_groups_row = adw::SwitchRow::builder()
            .title(i18n("Include group hierarchy"))
            .subtitle(i18n("Preserve folder structure"))
            .active(true)
            .build();
        options_group.add(&include_groups_row);

        // Credentials info row
        let creds_info_row = adw::ActionRow::builder()
            .title(i18n("ℹ Credentials Storage"))
            .subtitle(i18n("Passwords are stored in your password manager and not included in exports. Export your credential structure separately if needed."))
            .build();
        let info_icon = gtk4::Image::from_icon_name("dialog-information-symbolic");
        info_icon.set_valign(gtk4::Align::Center);
        creds_info_row.add_prefix(&info_icon);
        options_group.add(&creds_info_row);

        main_vbox.append(&options_group);

        // CSV-specific options section (hidden by default, shown when CSV is selected)
        let csv_options_group = adw::PreferencesGroup::builder()
            .title(i18n("CSV Options"))
            .description(i18n("Configure CSV export settings"))
            .visible(false)
            .build();

        // Delimiter selection
        let csv_delimiter_list =
            StringList::new(&[&i18n("Comma"), &i18n("Semicolon"), &i18n("Tab")]);
        let csv_delimiter_dropdown =
            DropDown::new(Some(csv_delimiter_list), gtk4::Expression::NONE);
        csv_delimiter_dropdown.set_selected(0);
        csv_delimiter_dropdown.set_valign(gtk4::Align::Center);

        let delimiter_row = adw::ActionRow::builder()
            .title(i18n("Delimiter"))
            .subtitle(i18n("Field separator character"))
            .build();
        delimiter_row.add_suffix(&csv_delimiter_dropdown);
        csv_options_group.add(&delimiter_row);

        // Field selection switches
        let csv_field_name = adw::SwitchRow::builder()
            .title(i18n("Name"))
            .active(true)
            .build();
        csv_options_group.add(&csv_field_name);

        let csv_field_host = adw::SwitchRow::builder()
            .title(i18n("Host"))
            .active(true)
            .build();
        csv_options_group.add(&csv_field_host);

        let csv_field_port = adw::SwitchRow::builder()
            .title(i18n("Port"))
            .active(true)
            .build();
        csv_options_group.add(&csv_field_port);

        let csv_field_protocol = adw::SwitchRow::builder()
            .title(i18n("Protocol"))
            .active(true)
            .build();
        csv_options_group.add(&csv_field_protocol);

        let csv_field_username = adw::SwitchRow::builder()
            .title(i18n("Username"))
            .active(true)
            .build();
        csv_options_group.add(&csv_field_username);

        let csv_field_group = adw::SwitchRow::builder()
            .title(i18n("Group"))
            .active(true)
            .build();
        csv_options_group.add(&csv_field_group);

        let csv_field_tags = adw::SwitchRow::builder()
            .title(i18n("Tags"))
            .active(true)
            .build();
        csv_options_group.add(&csv_field_tags);

        let csv_field_description = adw::SwitchRow::builder()
            .title(i18n("Description"))
            .active(true)
            .build();
        csv_options_group.add(&csv_field_description);

        main_vbox.append(&csv_options_group);

        clamp.set_child(Some(&main_vbox));
        scrolled.set_child(Some(&clamp));

        (
            scrolled,
            format_dropdown,
            output_path_entry,
            browse_button,
            include_groups_row,
            csv_options_group,
            csv_delimiter_dropdown,
            csv_field_name,
            csv_field_host,
            csv_field_port,
            csv_field_protocol,
            csv_field_username,
            csv_field_group,
            csv_field_tags,
            csv_field_description,
            group_filter_dropdown,
        )
    }

    /// Creates the progress page
    fn create_progress_page() -> (GtkBox, ProgressBar, Label) {
        let vbox = GtkBox::new(Orientation::Vertical, 12);
        vbox.set_valign(gtk4::Align::Center);
        vbox.set_halign(gtk4::Align::Center);

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

        (vbox, progress_bar, progress_label)
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
        Self::format_from_index(self.format_dropdown.selected())
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

    /// Sets the connections to export
    pub fn set_connections(&self, connections: Vec<Connection>) {
        *self.connections.borrow_mut() = connections;
    }

    /// Sets the groups for export and populates the group filter dropdown
    pub fn set_groups(&self, groups: Vec<ConnectionGroup>) {
        // Build hierarchical group list for the filter dropdown
        let mut filter_data: Vec<(Option<uuid::Uuid>, String)> =
            vec![(None, i18n("All connections"))];

        #[expect(
            clippy::items_after_statements,
            reason = "local helper introduced inline next to its only call site; hoisting would scatter related logic"
        )]
        fn add_group_recursive(
            group: &ConnectionGroup,
            all_groups: &[ConnectionGroup],
            filter_data: &mut Vec<(Option<uuid::Uuid>, String)>,
            depth: usize,
        ) {
            let indent = "  ".repeat(depth);
            filter_data.push((Some(group.id), format!("{}{}", indent, group.name)));
            let children: Vec<_> = all_groups
                .iter()
                .filter(|g| g.parent_id == Some(group.id))
                .collect();
            for child in children {
                add_group_recursive(child, all_groups, filter_data, depth + 1);
            }
        }

        let root_groups: Vec<_> = groups.iter().filter(|g| g.parent_id.is_none()).collect();
        for group in root_groups {
            add_group_recursive(group, &groups, &mut filter_data, 0);
        }

        // Update dropdown model
        let names: Vec<&str> = filter_data.iter().map(|(_, name)| name.as_str()).collect();
        let string_list = StringList::new(&names);
        self.group_filter_dropdown.set_model(Some(&string_list));
        self.group_filter_dropdown.set_selected(0);

        *self.groups_filter_data.borrow_mut() = filter_data;
        *self.groups.borrow_mut() = groups;
    }

    /// Sets the snippets for export (used in native format)
    pub fn set_snippets(&self, snippets: Vec<Snippet>) {
        *self.snippets.borrow_mut() = snippets;
    }

    /// Sets the smart folders to include in native export
    pub fn set_smart_folders(&self, smart_folders: Vec<SmartFolder>) {
        *self.smart_folders.borrow_mut() = smart_folders;
    }

    /// Sets the templates to include in native export
    pub fn set_templates(&self, templates: Vec<ConnectionTemplate>) {
        *self.templates.borrow_mut() = templates;
    }

    /// Sets the clusters to include in native export
    pub fn set_clusters(&self, clusters: Vec<Cluster>) {
        *self.clusters.borrow_mut() = clusters;
    }

    /// Sets the variables to include in native export
    pub fn set_variables(&self, variables: Vec<Variable>) {
        *self.variables.borrow_mut() = variables;
    }

    /// Maps a format dropdown index to an `ExportFormat`
    fn format_from_index(index: u32) -> ExportFormat {
        match index {
            0 => ExportFormat::Native,
            1 => ExportFormat::Ansible,
            2 => ExportFormat::Asbru,
            3 => ExportFormat::Csv,
            4 => ExportFormat::MobaXterm,
            5 => ExportFormat::Remmina,
            6 => ExportFormat::RoyalTs,
            7 => ExportFormat::SecureCrt,
            8 => ExportFormat::SshConfig,
            _ => ExportFormat::Native,
        }
    }

    /// Performs the export operation
    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
    fn do_export(
        connections: &[Connection],
        groups: &[ConnectionGroup],
        snippets: &[Snippet],
        smart_folders: &[SmartFolder],
        templates: &[ConnectionTemplate],
        clusters: &[Cluster],
        variables: &[Variable],
        options: &ExportOptions,
        csv_options: Option<&CsvExportOptions>,
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
                let mut export = NativeExport::with_data(
                    connections.to_vec(),
                    groups.to_vec(),
                    templates.to_vec(),
                    clusters.to_vec(),
                    variables.to_vec(),
                    snippets.to_vec(),
                );
                export.smart_folders = smart_folders.to_vec();
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
            ExportFormat::Csv => {
                let exporter = if let Some(csv_opts) = csv_options {
                    CsvExporter::with_options(csv_opts.clone())
                } else {
                    CsvExporter::new()
                };
                exporter
                    .export(connections, groups, options)
                    .map_err(|e| e.to_string())
            }
            ExportFormat::SecureCrt => {
                let exporter = SecureCrtExporter;
                exporter
                    .export(connections, groups, options)
                    .map_err(|e| e.to_string())
            }
        }
    }

    /// Formats the result summary message
    fn format_result_summary(result: &ExportResult, format: ExportFormat) -> String {
        let summary = i18n_f(
            "Successfully exported {} connection(s) to {} format.",
            &[
                &result.exported_count.to_string(),
                &i18n(format.display_name()),
            ],
        );

        if result.skipped_count > 0 {
            format!(
                "{}\n\n{}",
                summary,
                i18n_f(
                    "{} connection(s) were skipped (unsupported protocol).",
                    &[&result.skipped_count.to_string()]
                )
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
            details.push_str(&i18n("Output files:"));
            details.push('\n');
            for file in &result.output_files {
                let _ = writeln!(details, "  • {}", file.display());
            }
            details.push('\n');
        }

        // Summary
        let _ = writeln!(details, "{}:", i18n("Summary"));
        let _ = writeln!(
            details,
            "  • {}: {}",
            i18n("Exported"),
            result.exported_count
        );
        let _ = writeln!(details, "  • {}: {}", i18n("Skipped"), result.skipped_count);

        // List warnings
        if !result.warnings.is_empty() {
            details.push('\n');
            let _ = writeln!(details, "{} ({}):", i18n("Warnings"), result.warnings.len());
            for warning in &result.warnings {
                let _ = writeln!(details, "  • {warning}");
            }
        }

        if details.is_empty() {
            details = i18n("No connections were exported.");
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

        self.dialog.present(self.parent.as_ref());
    }

    /// Connects the browse button to show file/folder dialog
    fn connect_browse_button(&self) {
        let format_dropdown = self.format_dropdown.clone();
        let output_path_entry = self.output_path_entry.clone();
        let parent_window = self.parent.clone();

        self.browse_button.connect_clicked(move |_| {
            let format = Self::format_from_index(format_dropdown.selected());

            let output_entry = output_path_entry.clone();

            if format.exports_to_directory() {
                // Use folder dialog for Remmina
                let dialog = FileDialog::builder()
                    .title(i18n("Select Export Directory"))
                    .modal(true)
                    .build();

                dialog.select_folder(
                    parent_window.as_ref(),
                    gtk4::gio::Cancellable::NONE,
                    move |result| {
                        if let Ok(file) = result
                            && let Some(path) = file.path()
                        {
                            output_entry.set_text(&path.to_string_lossy());
                        }
                    },
                );
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
                        filter.set_name(Some(&i18n("Ansible Inventory (*.ini, *.yml)")));
                    }
                    ExportFormat::SshConfig => {
                        filter.add_pattern("*");
                        filter.set_name(Some(&i18n("SSH Config")));
                    }
                    ExportFormat::Asbru => {
                        filter.add_pattern("*.yml");
                        filter.add_pattern("*.yaml");
                        filter.set_name(Some(&i18n("Asbru-CM YAML (*.yml)")));
                    }
                    ExportFormat::Remmina => {
                        // Should not reach here
                        filter.add_pattern("*.remmina");
                        filter.set_name(Some(&i18n("Remmina (*.remmina)")));
                    }
                    ExportFormat::Native => {
                        filter.add_pattern("*.rcn");
                        filter.set_name(Some(&i18n("RustConn Native (*.rcn)")));
                    }
                    ExportFormat::RoyalTs => {
                        filter.add_pattern("*.rtsz");
                        filter.set_name(Some(&i18n("Royal TS (*.rtsz)")));
                    }
                    ExportFormat::MobaXterm => {
                        filter.add_pattern("*.mxtsessions");
                        filter.set_name(Some(&i18n("MobaXterm Sessions (*.mxtsessions)")));
                    }
                    ExportFormat::Csv => {
                        filter.add_pattern("*.csv");
                        filter.set_name(Some(&i18n("CSV (*.csv)")));
                    }
                    ExportFormat::SecureCrt => {
                        // Should not reach here (exports to directory)
                        filter.add_pattern("*.ini");
                        filter.set_name(Some(&i18n("SecureCRT Sessions (*.ini)")));
                    }
                }

                let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
                filters.append(&filter);
                dialog.set_filters(Some(&filters));

                dialog.save(
                    parent_window.as_ref(),
                    gtk4::gio::Cancellable::NONE,
                    move |result| {
                        if let Ok(file) = result
                            && let Some(path) = file.path()
                        {
                            output_entry.set_text(&path.to_string_lossy());
                        }
                    },
                );
            }
        });
    }

    /// Connects format dropdown change to update UI hints
    fn connect_format_change(&self) {
        let output_path_entry = self.output_path_entry.clone();
        let csv_options_group = self.csv_options_group.clone();

        self.format_dropdown
            .connect_selected_notify(move |dropdown| {
                let format = Self::format_from_index(dropdown.selected());

                // Show/hide CSV options based on format
                csv_options_group.set_visible(format == ExportFormat::Csv);

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
        let dialog = self.dialog.clone();
        let parent_window = self.parent.clone();
        let stack = self.stack.clone();
        let format_dropdown = self.format_dropdown.clone();
        let output_path_entry = self.output_path_entry.clone();
        let include_groups = self.include_groups_row.clone();
        let progress_bar = self.progress_bar.clone();
        let progress_label = self.progress_label.clone();
        #[cfg(not(feature = "adw-1-6"))]
        let progress_spinner = self.progress_spinner.clone();
        let result_label = self.result_label.clone();
        let result_details = self.result_details.clone();
        let export_button = self.export_button.clone();
        let connections = self.connections.clone();
        let groups = self.groups.clone();
        let snippets = self.snippets.clone();
        let smart_folders = self.smart_folders.clone();
        let templates = self.templates.clone();
        let clusters = self.clusters.clone();
        let variables = self.variables.clone();
        let result_cell = self.result.clone();
        let on_complete = self.on_complete.clone();
        let csv_delimiter_dropdown = self.csv_delimiter_dropdown.clone();
        let csv_field_name = self.csv_field_name.clone();
        let csv_field_host = self.csv_field_host.clone();
        let csv_field_port = self.csv_field_port.clone();
        let csv_field_protocol = self.csv_field_protocol.clone();
        let csv_field_username = self.csv_field_username.clone();
        let csv_field_group = self.csv_field_group.clone();
        let csv_field_tags = self.csv_field_tags.clone();
        let csv_field_description = self.csv_field_description.clone();
        let group_filter_dropdown = self.group_filter_dropdown.clone();
        let groups_filter_data = self.groups_filter_data.clone();

        export_button.connect_clicked(move |btn| {
            let current_page = stack.visible_child_name();

            if current_page.as_deref() == Some("result") {
                // Done - close dialog and optionally open output location
                if let Some(ref cb) = *on_complete.borrow() {
                    cb(result_cell.borrow_mut().take());
                }
                dialog.close();
                return;
            }

            // Validate output path
            let output_text = output_path_entry.text();
            if output_text.is_empty() {
                // Show error using toast
                if let Some(ref win) = parent_window {
                    crate::toast::show_toast_on_window(
                        win,
                        &i18n("Select an output file or directory"),
                        crate::toast::ToastType::Warning,
                    );
                }
                return;
            }

            let output_path = PathBuf::from(output_text.as_str());
            let format = Self::format_from_index(format_dropdown.selected());

            // Build CSV export options if CSV format is selected
            let csv_opts = if format == ExportFormat::Csv {
                let delimiter = match csv_delimiter_dropdown.selected() {
                    0 => b',',
                    1 => b';',
                    2 => b'\t',
                    _ => b',',
                };
                let mut fields = Vec::new();
                if csv_field_name.is_active() {
                    fields.push(CsvExportField::Name);
                }
                if csv_field_host.is_active() {
                    fields.push(CsvExportField::Host);
                }
                if csv_field_port.is_active() {
                    fields.push(CsvExportField::Port);
                }
                if csv_field_protocol.is_active() {
                    fields.push(CsvExportField::Protocol);
                }
                if csv_field_username.is_active() {
                    fields.push(CsvExportField::Username);
                }
                if csv_field_group.is_active() {
                    fields.push(CsvExportField::GroupName);
                }
                if csv_field_tags.is_active() {
                    fields.push(CsvExportField::Tags);
                }
                if csv_field_description.is_active() {
                    fields.push(CsvExportField::Description);
                }
                Some(CsvExportOptions { delimiter, fields })
            } else {
                None
            };

            let options = ExportOptions::new(format, output_path.clone())
                .with_groups(include_groups.is_active());

            // Show progress page
            stack.set_visible_child_name("progress");
            btn.set_sensitive(false);
            progress_bar.set_fraction(0.0);
            #[cfg(not(feature = "adw-1-6"))]
            progress_spinner.set_spinning(true);
            progress_label.set_text(&i18n_f(
                "Exporting to {}...",
                &[&i18n(format.display_name())],
            ));

            // Perform export on a background thread to avoid blocking the UI
            let all_conns = connections.borrow().clone();
            let all_grps = groups.borrow().clone();
            let snips = snippets.borrow().clone();
            let smart_fldrs = smart_folders.borrow().clone();
            let tmpls = templates.borrow().clone();
            let clstrs = clusters.borrow().clone();
            let vars = variables.borrow().clone();

            // Filter connections and groups by selected group (and its descendants)
            let (conns, grps) = {
                let filter_idx = group_filter_dropdown.selected() as usize;
                let filter_data = groups_filter_data.borrow();
                if filter_idx == 0 || filter_idx >= filter_data.len() {
                    // "All connections" — no filtering
                    (all_conns, all_grps)
                } else if let Some(root_group_id) = filter_data[filter_idx].0 {
                    // Collect the selected group and all its descendants
                    let mut matching_group_ids = std::collections::HashSet::new();
                    matching_group_ids.insert(root_group_id);
                    // Iteratively expand children
                    let mut changed = true;
                    while changed {
                        changed = false;
                        for g in &all_grps {
                            if let Some(pid) = g.parent_id
                                && matching_group_ids.contains(&pid)
                                && matching_group_ids.insert(g.id)
                            {
                                changed = true;
                            }
                        }
                    }
                    let filtered_conns = all_conns
                        .into_iter()
                        .filter(|c| {
                            c.group_id
                                .is_some_and(|gid| matching_group_ids.contains(&gid))
                        })
                        .collect();
                    let filtered_grps = all_grps
                        .into_iter()
                        .filter(|g| matching_group_ids.contains(&g.id))
                        .collect();
                    (filtered_conns, filtered_grps)
                } else {
                    (all_conns, all_grps)
                }
            };

            progress_bar.set_fraction(0.3);
            progress_label.set_text(&i18n("Writing output files..."));

            let options_clone = options.clone();
            let csv_opts_clone = csv_opts.clone();
            let btn = btn.clone();
            let progress_bar = progress_bar.clone();
            let progress_label = progress_label.clone();
            #[cfg(not(feature = "adw-1-6"))]
            let progress_spinner = progress_spinner.clone();
            let result_label = result_label.clone();
            let result_details = result_details.clone();
            let result_cell = result_cell.clone();
            let stack = stack.clone();
            crate::utils::spawn_blocking_with_callback(
                move || {
                    Self::do_export(
                        &conns,
                        &grps,
                        &snips,
                        &smart_fldrs,
                        &tmpls,
                        &clstrs,
                        &vars,
                        &options_clone,
                        csv_opts_clone.as_ref(),
                    )
                },
                move |export_result| {
                    progress_bar.set_fraction(1.0);
                    #[cfg(not(feature = "adw-1-6"))]
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
                            result_details.set_text(&crate::i18n::i18n_f(
                                "Could not write the export file: {}. Check destination permissions and free disk space.",
                                &[e.as_str()],
                            ));

                            stack.set_visible_child_name("result");
                            btn.set_label(&i18n("Close"));
                            btn.set_sensitive(true);
                        }
                    }
                },
            );
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
            tracing::warn!(%e, "Failed to open output location");
        }
    }
}
