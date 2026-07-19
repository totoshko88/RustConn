//! Source detection, source list UI, and per-source file import handlers.
//!
//! Extracted from `import.rs` as part of ARCH-5 decomposition.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, Frame, Label, ListBox, ListBoxRow, Orientation, ProgressBar,
    ScrolledWindow, Stack,
};
use rustconn_core::export::NativeExport;
use rustconn_core::import::{
    AnsibleInventoryImporter, AsbruImporter, CsvImporter, CsvParseOptions, ImportResult,
    ImportSource, LibvirtDaemonImporter, LibvirtXmlImporter, MobaXtermImporter, RdmImporter,
    RdpFileImporter, RemminaImporter, RoyalTsImporter, SecureCrtImporter, SshConfigImporter,
    VirtViewerImporter,
};

use super::ImportDialog;
use crate::i18n::{i18n, i18n_f};

impl ImportDialog {
    pub(super) fn create_source_page() -> (GtkBox, ListBox) {
        let vbox = GtkBox::new(Orientation::Vertical, 12);

        let header = Label::builder()
            .label(i18n("Select Import Source"))
            .css_classes(["title-3"])
            .halign(gtk4::Align::Start)
            .build();
        vbox.append(&header);

        let description = Label::builder()
            .label(i18n("Choose the source from which to import connections:"))
            .halign(gtk4::Align::Start)
            .wrap(true)
            .build();
        vbox.append(&description);

        // Create list box for sources
        let list_box = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .css_classes(["boxed-list"])
            .build();

        // Add import sources — Native (.rcn) first, then alphabetically
        let sources: Vec<(&str, String, String, bool)> = vec![
            (
                "native_file",
                i18n("RustConn Native (.rcn)"),
                i18n("Import from a RustConn native export file"),
                true,
            ),
            (
                "ansible",
                i18n("Ansible Inventory"),
                i18n("Import from Ansible inventory files"),
                AnsibleInventoryImporter::new().is_available(),
            ),
            (
                "ansible_file",
                i18n("Ansible Inventory File"),
                i18n("Import from a specific Ansible inventory file"),
                true,
            ),
            (
                "asbru",
                i18n("Asbru-CM"),
                i18n("Import from Asbru-CM/PAC Manager config"),
                AsbruImporter::new().is_available(),
            ),
            (
                "asbru_file",
                i18n("Asbru-CM YAML File"),
                i18n("Import from a specific Asbru-CM YAML file"),
                true,
            ),
            (
                "csv_file",
                i18n("CSV"),
                i18n("Import connections from a CSV file"),
                true,
            ),
            (
                "libvirt",
                i18n("Libvirt / GNOME Boxes"),
                i18n("Import VMs from libvirt domain XML files"),
                LibvirtXmlImporter::new().is_available(),
            ),
            (
                "libvirt_daemon",
                i18n("Libvirt Daemon (virsh)"),
                i18n("Query running libvirtd for VMs (requires virsh)"),
                LibvirtDaemonImporter::is_virsh_available(),
            ),
            (
                "libvirt_file",
                i18n("Libvirt XML File"),
                i18n("Import from a libvirt domain XML or virsh dumpxml output"),
                true,
            ),
            (
                "mobaxterm_file",
                i18n("MobaXterm (.mxtsessions)"),
                i18n("Import from a MobaXterm session export file"),
                true,
            ),
            (
                "rdp_file",
                i18n("RDP File (.rdp)"),
                i18n("Import RDP connection from a Microsoft .rdp file"),
                true,
            ),
            (
                "rdm_file",
                i18n("Remote Desktop Manager (JSON)"),
                i18n("Import from a Remote Desktop Manager JSON export file"),
                true,
            ),
            (
                "remmina",
                i18n("Remmina"),
                i18n("Import from Remmina connection files"),
                RemminaImporter::new().is_available(),
            ),
            (
                "royalts_file",
                i18n("Royal TS (.rtsz)"),
                i18n("Import from a Royal TS export file"),
                true,
            ),
            (
                "ssh_config",
                i18n("SSH Config"),
                i18n("Import from ~/.ssh/config"),
                SshConfigImporter::new().is_available(),
            ),
            (
                "ssh_config_file",
                i18n("SSH Config File"),
                i18n("Import from a specific SSH config file"),
                true,
            ),
            (
                "vv_file",
                i18n("Virt-Viewer (.vv)"),
                i18n("Import SPICE/VNC connection from a virt-viewer file"),
                true,
            ),
            (
                "multi_file",
                i18n("Multiple Files (batch)"),
                i18n("Import connections from multiple files at once"),
                true,
            ),
        ];

        for (id, name, desc, available) in &sources {
            let row = Self::create_source_row(id, name, desc, *available);
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

    pub(super) fn create_source_row(
        id: &str,
        name: &str,
        description: &str,
        available: bool,
    ) -> ListBoxRow {
        let hbox = GtkBox::new(Orientation::Horizontal, 12);
        hbox.set_margin_top(12);
        hbox.set_margin_bottom(12);
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
                .label(i18n("Available"))
                .css_classes(["success"])
                .build()
        } else {
            Label::builder()
                .label(i18n("Not Found"))
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

    pub fn get_source_display_name(source_id: &str) -> String {
        match source_id {
            "ssh_config" => i18n("SSH Config"),
            "ssh_config_file" => i18n("SSH Config File"),
            "asbru" => i18n("Asbru-CM"),
            "asbru_file" => i18n("Asbru-CM File"),
            "remmina" => i18n("Remmina"),
            "ansible" => i18n("Ansible"),
            "ansible_file" => i18n("Ansible File"),
            "native_file" => i18n("RustConn Native"),
            "royalts_file" => i18n("Royal TS"),
            "rdm_file" => i18n("Remote Desktop Manager"),
            "mobaxterm_file" => i18n("MobaXterm"),
            "vv_file" => i18n("Virt-Viewer"),
            "multi_file" => i18n("Multiple Files"),
            "libvirt" => i18n("Libvirt / GNOME Boxes"),
            "libvirt_file" => i18n("Libvirt XML"),
            "libvirt_daemon" => i18n("Libvirt Daemon"),
            "rdp_file" => i18n("RDP File"),
            "csv_file" => i18n("CSV"),
            _ => i18n("Unknown"),
        }
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
    pub(super) fn handle_ssh_config_file_import(
        parent_window: Option<&gtk4::Window>,
        stack: &Stack,
        progress_bar: &ProgressBar,
        progress_label: &Label,
        result_label: &Label,
        result_details: &Label,
        result_cell: &Rc<RefCell<Option<ImportResult>>>,
        source_name_cell: &Rc<RefCell<String>>,
        btn: &Button,
    ) {
        // Use file dialog for selecting SSH config file
        let file_dialog = gtk4::FileDialog::builder()
            .title(i18n("Select SSH Config File"))
            .modal(true)
            .build();

        // Set filter for SSH config files (typically no extension or "config")
        let filter = gtk4::FileFilter::new();
        filter.add_pattern("config");
        filter.add_pattern("config.*");
        filter.add_pattern("*");
        filter.set_name(Some(&i18n("SSH config files")));
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
            parent_window,
            gtk4::gio::Cancellable::NONE,
            move |file_result| {
                if let Ok(file) = file_result {
                    if let Some(path) = file.path() {
                        stack_clone.set_visible_child_name("progress");
                        btn_clone.set_sensitive(false);
                        progress_bar_clone.set_fraction(0.5);
                        progress_label_clone
                            .set_text(&i18n_f("Importing from {}...", &[&path.display().to_string()]));

                        // Parse SSH config file using import_from_path
                        let importer = SshConfigImporter::new();
                        let result = Self::import_or_error(
                            importer.import_from_path(&path),
                            "SSH config",
                        );

                        // Extract filename for display
                        let filename = path
                            .file_name().map_or_else(|| i18n("SSH Config File"), |n| n.to_string_lossy().to_string());

                        source_name_cell_clone.borrow_mut().clone_from(&filename);

                        progress_bar_clone.set_fraction(1.0);

                        // Show results with preview including connection count
                        let conn_count = result.connections.len();
                        let group_count = result.groups.len();
                        let summary = i18n_f(
                            "Successfully imported {} connection(s) and {} group(s).\nConnections will be added to '{} Import' group.",
                            &[&conn_count.to_string(), &group_count.to_string(), &filename],
                        );
                        result_label_clone.set_text(&summary);

                        let details = Self::format_import_details(&result);
                        result_details_clone.set_text(&details);

                        *result_cell_clone.borrow_mut() = Some(result);
                        stack_clone.set_visible_child_name("result");
                        btn_clone.set_label(&i18n("Done"));
                        btn_clone.set_sensitive(true);
                    }
                } else {
                    // User cancelled file selection - return to source page
                    stack_clone.set_visible_child_name("source");
                    btn_clone.set_sensitive(true);
                }
            },
        );
    }

    /// Handles the special case of importing from an Asbru-CM YAML file
    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
    pub(super) fn handle_asbru_file_import(
        parent_window: Option<&gtk4::Window>,
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
            .title(i18n("Select Asbru-CM YAML File"))
            .modal(true)
            .build();

        // Set filter for YAML files
        let filter = gtk4::FileFilter::new();
        filter.add_pattern("*.yml");
        filter.add_pattern("*.yaml");
        filter.set_name(Some(&i18n("YAML files")));
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
            parent_window,
            gtk4::gio::Cancellable::NONE,
            move |file_result| {
                if let Ok(file) = file_result {
                    if let Some(path) = file.path() {
                        stack_clone.set_visible_child_name("progress");
                        btn_clone.set_sensitive(false);
                        progress_bar_clone.set_fraction(0.5);
                        progress_label_clone
                            .set_text(&i18n_f("Importing from {}...", &[&path.display().to_string()]));

                        let importer = AsbruImporter::new();
                        let result = Self::import_or_error(
                            importer.import_from_path(&path),
                            "Asbru-CM",
                        );

                        // Extract filename for display
                        let filename = path
                            .file_name().map_or_else(|| i18n("Asbru-CM File"), |n| n.to_string_lossy().to_string());

                        source_name_cell_clone.borrow_mut().clone_from(&filename);

                        progress_bar_clone.set_fraction(1.0);

                        // Show results using format_import_details()
                        let conn_count = result.connections.len();
                        let group_count = result.groups.len();
                        let summary = i18n_f(
                            "Successfully imported {} connection(s) and {} group(s).\nConnections will be added to '{} Import' group.",
                            &[&conn_count.to_string(), &group_count.to_string(), &filename],
                        );
                        result_label_clone.set_text(&summary);

                        let details = Self::format_import_details(&result);
                        result_details_clone.set_text(&details);

                        *result_cell_clone.borrow_mut() = Some(result);
                        stack_clone.set_visible_child_name("result");
                        btn_clone.set_label(&i18n("Done"));
                        btn_clone.set_sensitive(true);
                    }
                } else {
                    // User cancelled file selection - return to source page
                    stack_clone.set_visible_child_name("source");
                    btn_clone.set_sensitive(true);
                }
            },
        );
    }

    /// Handles the special case of importing from an Ansible inventory file
    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
    pub(super) fn handle_ansible_file_import(
        parent_window: Option<&gtk4::Window>,
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
            .title(i18n("Select Ansible Inventory File"))
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
        filter.set_name(Some(&i18n("Ansible inventory files")));
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
            parent_window,
            gtk4::gio::Cancellable::NONE,
            move |file_result| {
                if let Ok(file) = file_result {
                    if let Some(path) = file.path() {
                        stack_clone.set_visible_child_name("progress");
                        btn_clone.set_sensitive(false);
                        progress_bar_clone.set_fraction(0.5);
                        progress_label_clone
                            .set_text(&i18n_f("Importing from {}...", &[&path.display().to_string()]));

                        let importer = AnsibleInventoryImporter::new();
                        let result = Self::import_or_error(
                            importer.import_from_path(&path),
                            "Ansible inventory",
                        );

                        // Extract filename for display
                        let filename = path
                            .file_name().map_or_else(|| i18n("Ansible Inventory"), |n| n.to_string_lossy().to_string());

                        source_name_cell_clone.borrow_mut().clone_from(&filename);

                        progress_bar_clone.set_fraction(1.0);

                        // Show results using format_import_details()
                        let conn_count = result.connections.len();
                        let group_count = result.groups.len();
                        let summary = i18n_f(
                            "Successfully imported {} connection(s) and {} group(s).\nConnections will be added to '{} Import' group.",
                            &[&conn_count.to_string(), &group_count.to_string(), &filename],
                        );
                        result_label_clone.set_text(&summary);

                        let details = Self::format_import_details(&result);
                        result_details_clone.set_text(&details);

                        *result_cell_clone.borrow_mut() = Some(result);
                        stack_clone.set_visible_child_name("result");
                        btn_clone.set_label(&i18n("Done"));
                        btn_clone.set_sensitive(true);
                    }
                } else {
                    // User cancelled file selection - return to source page
                    stack_clone.set_visible_child_name("source");
                    btn_clone.set_sensitive(true);
                }
            },
        );
    }

    /// Handles the special case of importing from a RustConn native file (.rcn)
    ///
    /// Opens a file chooser dialog for selecting a .rcn file,
    /// parses it using `NativeExport::from_file()`, and displays
    /// a preview with connection count before import.
    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
    pub(super) fn handle_native_file_import(
        parent_window: Option<&gtk4::Window>,
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
            .title(i18n("Select RustConn Native File"))
            .modal(true)
            .build();

        // Set filter for .rcn files
        let filter = gtk4::FileFilter::new();
        filter.add_pattern("*.rcn");
        filter.set_name(Some(&i18n("RustConn Native (*.rcn)")));
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
            parent_window,
            gtk4::gio::Cancellable::NONE,
            move |file_result| {
                if let Ok(file) = file_result {
                    if let Some(path) = file.path() {
                        stack_clone.set_visible_child_name("progress");
                        btn_clone.set_sensitive(false);
                        progress_bar_clone.set_fraction(0.5);
                        progress_label_clone
                            .set_text(&i18n_f("Importing from {}...", &[&path.display().to_string()]));
                        // Parse native file — try NativeExport first, then GroupSyncExport
                        match NativeExport::from_file(&path) {
                            Ok(native_export) => {
                                // Convert NativeExport to ImportResult
                                let result = ImportResult {
                                    connections: native_export.connections,
                                    groups: native_export.groups,
                                    skipped: Vec::new(),
                                    errors: Vec::new(),
                                    credentials: std::collections::HashMap::new(),
                                    snippets: native_export.snippets,
                                    smart_folders: native_export.smart_folders,
                                    warnings: Vec::new(),
                                };

                                // Extract filename for display
                                let filename = path.file_name().map_or_else(
                                    || i18n("RustConn Native"),
                                    |n| n.to_string_lossy().to_string(),
                                );

                                source_name_cell_clone.borrow_mut().clone_from(&filename);

                                progress_bar_clone.set_fraction(1.0);

                                // Show results
                                let conn_count = result.connections.len();
                                let group_count = result.groups.len();
                                let summary = i18n_f(
                                    "Successfully imported {} connection(s) and {} group(s).\nConnections will be added to '{} Import' group.",
                                    &[&conn_count.to_string(), &group_count.to_string(), &filename],
                                );
                                result_label_clone.set_text(&summary);

                                let details = Self::format_import_details(&result);
                                result_details_clone.set_text(&details);

                                *result_cell_clone.borrow_mut() = Some(result);
                                stack_clone.set_visible_child_name("result");
                                btn_clone.set_label(&i18n("Done"));
                                btn_clone.set_sensitive(true);
                            }
                            Err(_native_err) => {
                                // Try parsing as GroupSyncExport (Cloud Sync file)
                                match rustconn_core::sync::GroupSyncExport::from_file(&path) {
                                    Ok(sync_export) => {
                                        // Convert GroupSyncExport connections to regular Connections
                                        let root_group_id = uuid::Uuid::new_v4();
                                        let connections: Vec<_> = sync_export
                                            .connections
                                            .iter()
                                            .map(|sc| {
                                                rustconn_core::sync::group_export::sync_connection_to_connection(
                                                    sc,
                                                    root_group_id,
                                                )
                                            })
                                            .collect();

                                        let result = ImportResult {
                                            connections,
                                            groups: Vec::new(),
                                            skipped: Vec::new(),
                                            errors: Vec::new(),
                                            credentials: std::collections::HashMap::new(),
                                            snippets: Vec::new(),
                                            smart_folders: Vec::new(),
                                            warnings: vec![i18n(
                                                "Imported as Cloud Sync group (Import mode). Use Sync Now to keep it updated.",
                                            )],
                                        };

                                        let filename = path.file_name().map_or_else(
                                            || i18n("Cloud Sync"),
                                            |n| n.to_string_lossy().to_string(),
                                        );

                                        // Store the sync filename for later use
                                        source_name_cell_clone.borrow_mut().clone_from(&sync_export.root_group.name);

                                        progress_bar_clone.set_fraction(1.0);

                                        let conn_count = result.connections.len();
                                        let summary = i18n_f(
                                            "Imported {} connection(s) from Cloud Sync file '{}'\nGroup '{}' will be created in Import mode.",
                                            &[&conn_count.to_string(), &filename, &sync_export.root_group.name],
                                        );
                                        result_label_clone.set_text(&summary);

                                        let details = Self::format_import_details(&result);
                                        result_details_clone.set_text(&details);

                                        *result_cell_clone.borrow_mut() = Some(result);
                                        stack_clone.set_visible_child_name("result");
                                        btn_clone.set_label(&i18n("Done"));
                                        btn_clone.set_sensitive(true);
                                    }
                                    Err(sync_err) => {
                                        progress_bar_clone.set_fraction(1.0);
                                        result_label_clone.set_text(&i18n("Import Failed"));
                                        result_details_clone.set_text(&i18n_f("Error: {}", &[&sync_err.to_string()]));

                                        stack_clone.set_visible_child_name("result");
                                        btn_clone.set_label(&i18n("Close"));
                                        btn_clone.set_sensitive(true);
                                    }
                                }
                            }
                        }
                    }
                } else {
                    // User cancelled file selection - return to source page
                    stack_clone.set_visible_child_name("source");
                    btn_clone.set_sensitive(true);
                }
            },
        );
    }

    /// Handles the special case of importing from a Royal TS file (.rtsz)
    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
    pub(super) fn handle_royalts_file_import(
        parent_window: Option<&gtk4::Window>,
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
            .title(i18n("Select Royal TS File"))
            .modal(true)
            .build();

        let filter = gtk4::FileFilter::new();
        filter.add_pattern("*.rtsz");
        filter.add_pattern("*.json");
        filter.set_name(Some(&i18n("Royal TS files (*.rtsz, *.json)")));
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
            parent_window,
            gtk4::gio::Cancellable::NONE,
            move |file_result| {
                if let Ok(file) = file_result {
                    if let Some(path) = file.path() {
                        stack_clone.set_visible_child_name("progress");
                        btn_clone.set_sensitive(false);
                        progress_bar_clone.set_fraction(0.5);
                        progress_label_clone
                            .set_text(&i18n_f("Importing from {}...", &[&path.display().to_string()]));

                        let importer = RoyalTsImporter::new();
                        let result = Self::import_or_error(
                            importer.import_from_path(&path),
                            "Royal TS",
                        );

                        let filename = path.file_name().map_or_else(
                            || i18n("Royal TS"),
                            |n| n.to_string_lossy().to_string(),
                        );

                        source_name_cell_clone.borrow_mut().clone_from(&filename);

                        progress_bar_clone.set_fraction(1.0);

                        let conn_count = result.connections.len();
                        let group_count = result.groups.len();
                        let summary = i18n_f(
                            "Successfully imported {} connection(s) and {} group(s).\nConnections will be added to '{} Import' group.",
                            &[&conn_count.to_string(), &group_count.to_string(), &filename],
                        );
                        result_label_clone.set_text(&summary);

                        let details = Self::format_import_details(&result);
                        result_details_clone.set_text(&details);

                        *result_cell_clone.borrow_mut() = Some(result);
                        stack_clone.set_visible_child_name("result");
                        btn_clone.set_label(&i18n("Done"));
                        btn_clone.set_sensitive(true);
                    }
                } else {
                    // User cancelled file selection - return to source page
                    stack_clone.set_visible_child_name("source");
                    btn_clone.set_sensitive(true);
                }
            },
        );
    }

    /// Handles the special case of importing from a Remote Desktop Manager JSON file
    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
    pub(super) fn handle_rdm_file_import(
        parent_window: Option<&gtk4::Window>,
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
            .title(i18n("Select RDM JSON File"))
            .modal(true)
            .build();

        let filter = gtk4::FileFilter::new();
        filter.add_pattern("*.json");
        filter.set_name(Some(&i18n("JSON files")));
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
            parent_window,
            gtk4::gio::Cancellable::NONE,
            move |file_result| {
                if let Ok(file) = file_result {
                    if let Some(path) = file.path() {
                        stack_clone.set_visible_child_name("progress");
                        btn_clone.set_sensitive(false);
                        progress_bar_clone.set_fraction(0.5);
                        progress_label_clone
                            .set_text(&i18n_f("Importing from {}...", &[&path.display().to_string()]));

                        let importer = RdmImporter::new();
                        let result = Self::import_or_error(
                            importer.import_from_path(&path),
                            "RDM",
                        );

                        // Extract filename for display
                        let filename = path.file_name().map_or_else(
                            || i18n("RDM JSON"),
                            |n| n.to_string_lossy().to_string(),
                        );

                        source_name_cell_clone.borrow_mut().clone_from(&filename);

                        progress_bar_clone.set_fraction(1.0);

                        // Show results
                        let conn_count = result.connections.len();
                        let group_count = result.groups.len();
                        let summary = i18n_f(
                            "Successfully imported {} connection(s) and {} group(s).\nConnections will be added to '{} Import' group.",
                            &[&conn_count.to_string(), &group_count.to_string(), &filename],
                        );
                        result_label_clone.set_text(&summary);

                        let details = Self::format_import_details(&result);
                        result_details_clone.set_text(&details);

                        *result_cell_clone.borrow_mut() = Some(result);
                        stack_clone.set_visible_child_name("result");
                        btn_clone.set_label(&i18n("Done"));
                        btn_clone.set_sensitive(true);
                    }
                } else {
                    // User cancelled file selection - return to source page
                    stack_clone.set_visible_child_name("source");
                    btn_clone.set_sensitive(true);
                }
            },
        );
    }

    /// Handles the special case of importing from a MobaXterm session file
    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
    pub(super) fn handle_mobaxterm_file_import(
        parent_window: Option<&gtk4::Window>,
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
            .title(i18n("Select MobaXterm Session File"))
            .modal(true)
            .build();

        let filter = gtk4::FileFilter::new();
        filter.add_pattern("*.mxtsessions");
        filter.set_name(Some(&i18n("MobaXterm Sessions (*.mxtsessions)")));
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
            parent_window,
            gtk4::gio::Cancellable::NONE,
            move |file_result| {
                if let Ok(file) = file_result {
                    if let Some(path) = file.path() {
                        stack_clone.set_visible_child_name("progress");
                        btn_clone.set_sensitive(false);
                        progress_bar_clone.set_fraction(0.5);
                        progress_label_clone
                            .set_text(&i18n_f("Importing from {}...", &[&path.display().to_string()]));

                        let importer = MobaXtermImporter::with_path(path.clone());
                        let result = Self::import_or_error(
                            importer.import_from_path(&path),
                            "MobaXterm",
                        );

                        // Extract filename for display
                        let filename = path.file_name().map_or_else(
                            || i18n("MobaXterm"),
                            |n| n.to_string_lossy().to_string(),
                        );

                        source_name_cell_clone.borrow_mut().clone_from(&filename);

                        progress_bar_clone.set_fraction(1.0);

                        // Show results
                        let conn_count = result.connections.len();
                        let group_count = result.groups.len();
                        let summary = i18n_f(
                            "Successfully imported {} connection(s) and {} group(s).\nConnections will be added to '{} Import' group.",
                            &[&conn_count.to_string(), &group_count.to_string(), &filename],
                        );
                        result_label_clone.set_text(&summary);

                        let details = Self::format_import_details(&result);
                        result_details_clone.set_text(&details);

                        *result_cell_clone.borrow_mut() = Some(result);
                        stack_clone.set_visible_child_name("result");
                        btn_clone.set_label(&i18n("Done"));
                        btn_clone.set_sensitive(true);
                    }
                } else {
                    // User cancelled file selection - return to source page
                    stack_clone.set_visible_child_name("source");
                    btn_clone.set_sensitive(true);
                }
            },
        );
    }

    /// Handles importing from a libvirt domain XML file
    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
    pub(super) fn handle_libvirt_file_import(
        parent_window: Option<&gtk4::Window>,
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
            .title(i18n("Select Libvirt Domain XML File"))
            .modal(true)
            .build();

        let filter = gtk4::FileFilter::new();
        filter.add_pattern("*.xml");
        filter.set_name(Some(&i18n("XML files (*.xml)")));
        let all_filter = gtk4::FileFilter::new();
        all_filter.add_pattern("*");
        all_filter.set_name(Some(&i18n("All files")));
        let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
        filters.append(&filter);
        filters.append(&all_filter);
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
            parent_window,
            gtk4::gio::Cancellable::NONE,
            move |file_result| {
                if let Ok(file) = file_result {
                    if let Some(path) = file.path() {
                        stack_clone.set_visible_child_name("progress");
                        btn_clone.set_sensitive(false);
                        progress_bar_clone.set_fraction(0.5);
                        progress_label_clone
                            .set_text(&i18n_f("Importing from {}...", &[&path.display().to_string()]));

                        let importer = LibvirtXmlImporter::new();
                        let result =
                            Self::import_or_error(importer.import_from_path(&path), "Libvirt XML");

                        let filename = path.file_name().map_or_else(
                            || i18n("Libvirt XML"),
                            |n| n.to_string_lossy().to_string(),
                        );

                        source_name_cell_clone.borrow_mut().clone_from(&filename);

                        progress_bar_clone.set_fraction(1.0);

                        let conn_count = result.connections.len();
                        let group_count = result.groups.len();
                        let summary = i18n_f(
                            "Successfully imported {} connection(s) and {} group(s).\nConnections will be added to '{} Import' group.",
                            &[&conn_count.to_string(), &group_count.to_string(), &filename],
                        );
                        result_label_clone.set_text(&summary);

                        let details = Self::format_import_details(&result);
                        result_details_clone.set_text(&details);

                        *result_cell_clone.borrow_mut() = Some(result);
                        stack_clone.set_visible_child_name("result");
                        btn_clone.set_label(&i18n("Done"));
                        btn_clone.set_sensitive(true);
                    }
                } else {
                    stack_clone.set_visible_child_name("source");
                    btn_clone.set_sensitive(true);
                }
            },
        );
    }

    /// Handles importing from a virt-viewer (.vv) file
    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
    pub(super) fn handle_vv_file_import(
        parent_window: Option<&gtk4::Window>,
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
            .title(i18n("Select Virt-Viewer File"))
            .modal(true)
            .build();

        let filter = gtk4::FileFilter::new();
        filter.add_pattern("*.vv");
        filter.set_name(Some(&i18n("Virt-Viewer files (*.vv)")));
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
            parent_window,
            gtk4::gio::Cancellable::NONE,
            move |file_result| {
                if let Ok(file) = file_result {
                    if let Some(path) = file.path() {
                        stack_clone.set_visible_child_name("progress");
                        btn_clone.set_sensitive(false);
                        progress_bar_clone.set_fraction(0.5);
                        progress_label_clone.set_text(&i18n_f(
                            "Importing from {}...",
                            &[&path.display().to_string()],
                        ));

                        let importer = VirtViewerImporter::new();
                        let result =
                            Self::import_or_error(importer.import_from_path(&path), "Virt-Viewer");

                        let filename = path.file_name().map_or_else(
                            || i18n("Virt-Viewer"),
                            |n| n.to_string_lossy().to_string(),
                        );

                        source_name_cell_clone.borrow_mut().clone_from(&filename);

                        progress_bar_clone.set_fraction(1.0);

                        let conn_count = result.connections.len();
                        let group_count = result.groups.len();
                        let summary = i18n_f(
                            "Successfully imported {} connection(s) and {} group(s).\nConnections will be added to '{} Import' group.",
                            &[&conn_count.to_string(), &group_count.to_string(), &filename],
                        );
                        result_label_clone.set_text(&summary);

                        let details = Self::format_import_details(&result);
                        result_details_clone.set_text(&details);

                        *result_cell_clone.borrow_mut() = Some(result);
                        stack_clone.set_visible_child_name("result");
                        btn_clone.set_label(&i18n("Done"));
                        btn_clone.set_sensitive(true);
                    }
                } else {
                    stack_clone.set_visible_child_name("source");
                    btn_clone.set_sensitive(true);
                }
            },
        );
    }

    /// Handles import from a Microsoft .rdp file via file chooser dialog.
    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
    pub(super) fn handle_rdp_file_import(
        parent_window: Option<&gtk4::Window>,
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
            .title(i18n("Select RDP File"))
            .modal(true)
            .build();

        let filter = gtk4::FileFilter::new();
        filter.add_pattern("*.rdp");
        filter.set_name(Some(&i18n("RDP Files (*.rdp)")));
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
            parent_window,
            gtk4::gio::Cancellable::NONE,
            move |file_result| {
                if let Ok(file) = file_result {
                    if let Some(path) = file.path() {
                        stack_clone.set_visible_child_name("progress");
                        btn_clone.set_sensitive(false);
                        progress_bar_clone.set_fraction(0.5);
                        progress_label_clone
                            .set_text(&i18n_f("Importing from {}...", &[&path.display().to_string()]));

                        let importer = RdpFileImporter::new();
                        let result =
                            Self::import_or_error(importer.import_from_path(&path), "RDP File");

                        let filename = path.file_name().map_or_else(
                            || i18n("RDP File"),
                            |n| n.to_string_lossy().to_string(),
                        );

                        source_name_cell_clone.borrow_mut().clone_from(&filename);

                        progress_bar_clone.set_fraction(1.0);

                        let conn_count = result.connections.len();
                        let group_count = result.groups.len();
                        let summary = i18n_f(
                            "Successfully imported {} connection(s) and {} group(s).\nConnections will be added to '{} Import' group.",
                            &[&conn_count.to_string(), &group_count.to_string(), &filename],
                        );
                        result_label_clone.set_text(&summary);

                        let details = Self::format_import_details(&result);
                        result_details_clone.set_text(&details);

                        *result_cell_clone.borrow_mut() = Some(result);
                        stack_clone.set_visible_child_name("result");
                        btn_clone.set_label(&i18n("Done"));
                        btn_clone.set_sensitive(true);
                    }
                } else {
                    stack_clone.set_visible_child_name("source");
                    btn_clone.set_sensitive(true);
                }
            },
        );
    }

    /// Handles importing from a CSV file via file chooser dialog.
    ///
    /// Opens a file chooser for selecting a .csv file, parses it using
    /// `CsvImporter` with default options (comma delimiter, auto-detect headers),
    /// and displays a preview with connection count before import.
    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
    pub(super) fn handle_csv_file_import(
        parent_window: Option<&gtk4::Window>,
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
            .title(i18n("Select CSV File"))
            .modal(true)
            .build();

        let filter = gtk4::FileFilter::new();
        filter.add_pattern("*.csv");
        filter.add_pattern("*.tsv");
        filter.set_name(Some(&i18n("CSV files (*.csv, *.tsv)")));
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
            parent_window,
            gtk4::gio::Cancellable::NONE,
            move |file_result| {
                if let Ok(file) = file_result {
                    if let Some(path) = file.path() {
                        stack_clone.set_visible_child_name("progress");
                        btn_clone.set_sensitive(false);
                        progress_bar_clone.set_fraction(0.5);
                        progress_label_clone
                            .set_text(&i18n_f("Importing from {}...", &[&path.display().to_string()]));

                        // Auto-detect delimiter from file extension and content
                        let delimiter = if path
                            .extension()
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("tsv"))
                        {
                            b'\t'
                        } else if let Ok(first_line) = std::fs::read_to_string(&path)
                            .map(|s| s.lines().next().unwrap_or_default().to_string())
                        {
                            // Heuristic: if semicolons outnumber commas, use semicolon
                            let commas = first_line.matches(',').count();
                            let semicolons = first_line.matches(';').count();
                            let tabs = first_line.matches('\t').count();
                            if tabs > commas && tabs > semicolons {
                                b'\t'
                            } else if semicolons > commas {
                                b';'
                            } else {
                                b','
                            }
                        } else {
                            b','
                        };
                        let options = CsvParseOptions {
                            delimiter,
                            ..CsvParseOptions::default()
                        };
                        let importer = CsvImporter::with_options(options);
                        let result = Self::import_or_error(importer.import_from_path(&path), "CSV");

                        let filename = path
                            .file_name()
                            .map_or_else(|| i18n("CSV"), |n| n.to_string_lossy().to_string());

                        source_name_cell_clone.borrow_mut().clone_from(&filename);

                        progress_bar_clone.set_fraction(1.0);

                        let conn_count = result.connections.len();
                        let group_count = result.groups.len();
                        let summary = i18n_f(
                            "Successfully imported {} connection(s) and {} group(s).\nConnections will be added to '{} Import' group.",
                            &[&conn_count.to_string(), &group_count.to_string(), &filename],
                        );
                        result_label_clone.set_text(&summary);

                        let details = Self::format_import_details(&result);
                        result_details_clone.set_text(&details);

                        *result_cell_clone.borrow_mut() = Some(result);
                        stack_clone.set_visible_child_name("result");
                        btn_clone.set_label(&i18n("Done"));
                        btn_clone.set_sensitive(true);
                    }
                } else {
                    stack_clone.set_visible_child_name("source");
                    btn_clone.set_sensitive(true);
                }
            },
        );
    }

    /// Handles multi-file batch import using `BatchImporter`.
    ///
    /// Opens a file chooser dialog that allows selecting multiple files
    /// (CSV, SSH config, RDP, etc.) and imports them all in a single batch
    /// with progress reporting.
    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
    pub(super) fn handle_multi_file_import(
        parent_window: Option<&gtk4::Window>,
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
            .title(i18n("Select Files to Import"))
            .modal(true)
            .build();

        // Accept all supported formats
        let filter = gtk4::FileFilter::new();
        filter.add_pattern("*.csv");
        filter.add_pattern("*.tsv");
        filter.add_pattern("*.rdp");
        filter.add_pattern("*.vv");
        filter.add_pattern("*.rcn");
        filter.add_pattern("*.json");
        filter.add_pattern("*.rtsz");
        filter.add_pattern("*.mxtsessions");
        filter.add_pattern("*.xml");
        filter.add_pattern("*.yaml");
        filter.add_pattern("*.yml");
        filter.add_pattern("*.ini");
        filter.set_name(Some(&i18n("All supported formats")));
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

        file_dialog.open_multiple(
            parent_window,
            gtk4::gio::Cancellable::NONE,
            move |files_result| {
                if let Ok(files) = files_result {
                    let paths: Vec<std::path::PathBuf> = (0..files.n_items())
                        .filter_map(|i| {
                            files
                                .item(i)
                                .and_then(|obj| obj.downcast::<gtk4::gio::File>().ok())
                                .and_then(|f| f.path())
                        })
                        .collect();

                    if paths.is_empty() {
                        stack_clone.set_visible_child_name("source");
                        btn_clone.set_sensitive(true);
                        return;
                    }

                    stack_clone.set_visible_child_name("progress");
                    btn_clone.set_sensitive(false);

                    let file_count = paths.len();
                    progress_label_clone.set_text(&i18n_f(
                        "Importing from {} files...",
                        &[&file_count.to_string()],
                    ));

                    // Import each file based on extension, merging results
                    let mut combined = ImportResult::default();
                    for (idx, path) in paths.iter().enumerate() {
                        #[expect(
    clippy::cast_precision_loss,
    reason = "f64 conversion is intentional for display/UI arithmetic where sub-integer precision is irrelevant"
)]
                        let fraction = (idx as f64) / (file_count as f64);
                        progress_bar_clone.set_fraction(fraction);

                        let filename = path
                            .file_name()
                            .map_or_else(|| String::from("?"), |n| n.to_string_lossy().to_string());
                        progress_label_clone.set_text(&i18n_f(
                            "Importing {}...",
                            &[&filename],
                        ));

                        let file_result = Self::import_file_by_extension(path);
                        // Merge into combined result
                        combined.connections.extend(file_result.connections);
                        combined.groups.extend(file_result.groups);
                        combined.skipped.extend(file_result.skipped);
                        combined.errors.extend(file_result.errors);
                    }

                    *source_name_cell_clone.borrow_mut() = i18n("Multiple Files");

                    progress_bar_clone.set_fraction(1.0);

                    let conn_count = combined.connections.len();
                    let group_count = combined.groups.len();
                    let error_count = combined.errors.len();
                    let summary = if error_count > 0 {
                        i18n_f(
                            "Imported {} connection(s) and {} group(s) from {} files ({} errors).\nConnections will be added to 'Multiple Files Import' group.",
                            &[
                                &conn_count.to_string(),
                                &group_count.to_string(),
                                &file_count.to_string(),
                                &error_count.to_string(),
                            ],
                        )
                    } else {
                        i18n_f(
                            "Imported {} connection(s) and {} group(s) from {} files.\nConnections will be added to 'Multiple Files Import' group.",
                            &[
                                &conn_count.to_string(),
                                &group_count.to_string(),
                                &file_count.to_string(),
                            ],
                        )
                    };
                    result_label_clone.set_text(&summary);

                    let details = Self::format_import_details(&combined);
                    result_details_clone.set_text(&details);

                    *result_cell_clone.borrow_mut() = Some(combined);
                    stack_clone.set_visible_child_name("result");
                    btn_clone.set_label(&i18n("Done"));
                    btn_clone.set_sensitive(true);
                } else {
                    stack_clone.set_visible_child_name("source");
                    btn_clone.set_sensitive(true);
                }
            },
        );
    }

    /// Imports a single file based on its extension.
    ///
    /// Detects the format from the file extension and uses the appropriate importer.
    pub(super) fn import_file_by_extension(path: &std::path::Path) -> ImportResult {
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        match ext.as_str() {
            "csv" | "tsv" => {
                let delimiter = if ext == "tsv" { b'\t' } else { b',' };
                let options = CsvParseOptions {
                    delimiter,
                    ..CsvParseOptions::default()
                };
                let importer = CsvImporter::with_options(options);
                Self::import_or_error(importer.import_from_path(path), "CSV")
            }
            "rdp" => {
                let importer = RdpFileImporter::new();
                Self::import_or_error(importer.import_from_path(path), "RDP")
            }
            "vv" => {
                let importer = VirtViewerImporter::new();
                Self::import_or_error(importer.import_from_path(path), "Virt-Viewer")
            }
            "rcn" => {
                // Native RustConn format
                match NativeExport::from_file(path) {
                    Ok(native_export) => {
                        let mut warnings = Vec::new();
                        if !native_export.templates.is_empty() {
                            warnings.push(format!(
                                "{} template(s) skipped (not supported in batch import)",
                                native_export.templates.len()
                            ));
                        }
                        if !native_export.clusters.is_empty() {
                            warnings.push(format!(
                                "{} cluster(s) skipped (not supported in batch import)",
                                native_export.clusters.len()
                            ));
                        }
                        if !native_export.variables.is_empty() {
                            warnings.push(format!(
                                "{} variable(s) skipped (not supported in batch import)",
                                native_export.variables.len()
                            ));
                        }
                        ImportResult {
                            connections: native_export.connections,
                            groups: native_export.groups,
                            skipped: Vec::new(),
                            errors: Vec::new(),
                            credentials: std::collections::HashMap::new(),
                            snippets: native_export.snippets,
                            smart_folders: native_export.smart_folders,
                            warnings,
                        }
                    }
                    Err(e) => {
                        let mut r = ImportResult::default();
                        r.add_error(rustconn_core::error::ImportError::InvalidEntry {
                            source_name: "RustConn Native".to_string(),
                            reason: e.to_string(),
                        });
                        r
                    }
                }
            }
            "json" => {
                // Try RDM JSON format
                let importer = RdmImporter::new();
                Self::import_or_error(importer.import_from_path(path), "RDM JSON")
            }
            "rtsz" => {
                let importer = RoyalTsImporter::new();
                Self::import_or_error(importer.import_from_path(path), "Royal TS")
            }
            "mxtsessions" => {
                let importer = MobaXtermImporter::new();
                Self::import_or_error(importer.import_from_path(path), "MobaXterm")
            }
            "xml" => {
                let importer = LibvirtXmlImporter::new();
                Self::import_or_error(importer.import_from_path(path), "Libvirt XML")
            }
            "yaml" | "yml" => {
                // Try Asbru format first, then Ansible
                let importer = AsbruImporter::new();
                let result = importer.import_from_path(path);
                if result.is_ok() {
                    Self::import_or_error(result, "Asbru-CM")
                } else {
                    let importer = AnsibleInventoryImporter::new();
                    Self::import_or_error(importer.import_from_path(path), "Ansible")
                }
            }
            "ini" => {
                let importer = SecureCrtImporter::new();
                Self::import_or_error(importer.import_from_path(path), "SecureCRT")
            }
            _ => {
                // Try SSH config as fallback
                let importer = SshConfigImporter::new();
                Self::import_or_error(importer.import_from_path(path), "SSH Config")
            }
        }
    }
}
