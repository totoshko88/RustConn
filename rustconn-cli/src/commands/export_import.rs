//! Export and import connection commands.

use std::path::Path;

use rustconn_core::models::{Connection, ConnectionGroup};

use crate::cli::{ExportFormatArg, ImportFormatArg};
use crate::error::CliError;
use crate::util::create_config_manager;

/// Export connections command handler
pub fn cmd_export(
    config_path: Option<&Path>,
    format: ExportFormatArg,
    output: &Path,
) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Config(format!("Failed to load groups: {e}")))?;

    let export_format = match format {
        ExportFormatArg::Ansible => rustconn_core::export::ExportFormat::Ansible,
        ExportFormatArg::SshConfig => rustconn_core::export::ExportFormat::SshConfig,
        ExportFormatArg::Remmina => rustconn_core::export::ExportFormat::Remmina,
        ExportFormatArg::Asbru => rustconn_core::export::ExportFormat::Asbru,
        ExportFormatArg::Native => rustconn_core::export::ExportFormat::Native,
        ExportFormatArg::RoyalTs => rustconn_core::export::ExportFormat::RoyalTs,
        ExportFormatArg::MobaXterm => rustconn_core::export::ExportFormat::MobaXterm,
    };

    let options = rustconn_core::export::ExportOptions::new(export_format, output.to_path_buf());

    let result = export_connections(&connections, &groups, &options)?;

    println!(
        "Export complete: {} connections exported, {} skipped",
        result.exported_count, result.skipped_count
    );

    if !result.warnings.is_empty() {
        for warning in &result.warnings {
            tracing::warn!("Export: {warning}");
        }
    }

    if !result.output_files.is_empty() {
        println!("\nOutput files:");
        for file in &result.output_files {
            println!("  - {}", file.display());
        }
    }

    Ok(())
}

/// Exports connections using the appropriate exporter based on format
fn export_connections(
    connections: &[Connection],
    groups: &[ConnectionGroup],
    options: &rustconn_core::export::ExportOptions,
) -> Result<rustconn_core::export::ExportResult, CliError> {
    use rustconn_core::export::{
        AnsibleExporter, AsbruExporter, ExportFormat, ExportTarget, MobaXtermExporter,
        NativeExport, RemminaExporter, RoyalTsExporter, SshConfigExporter,
    };

    let result = match options.format {
        ExportFormat::Ansible => {
            let exporter = AnsibleExporter::new();
            exporter
                .export(connections, groups, options)
                .map_err(|e| CliError::Export(e.to_string()))?
        }
        ExportFormat::SshConfig => {
            let exporter = SshConfigExporter::new();
            exporter
                .export(connections, groups, options)
                .map_err(|e| CliError::Export(e.to_string()))?
        }
        ExportFormat::Remmina => {
            let exporter = RemminaExporter::new();
            exporter
                .export(connections, groups, options)
                .map_err(|e| CliError::Export(e.to_string()))?
        }
        ExportFormat::Asbru => {
            let exporter = AsbruExporter::new();
            exporter
                .export(connections, groups, options)
                .map_err(|e| CliError::Export(e.to_string()))?
        }
        ExportFormat::Native => {
            let native_export = NativeExport::with_data(
                connections.to_vec(),
                groups.to_vec(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            );

            native_export
                .to_file(&options.output_path)
                .map_err(|e| CliError::Export(e.to_string()))?;
            rustconn_core::export::ExportResult {
                exported_count: connections.len(),
                skipped_count: 0,
                warnings: Vec::new(),
                output_files: vec![options.output_path.clone()],
            }
        }
        ExportFormat::RoyalTs => {
            let exporter = RoyalTsExporter::new();
            exporter
                .export(connections, groups, options)
                .map_err(|e| CliError::Export(e.to_string()))?
        }
        ExportFormat::MobaXterm => {
            let exporter = MobaXtermExporter::new();
            exporter
                .export(connections, groups, options)
                .map_err(|e| CliError::Export(e.to_string()))?
        }
    };

    Ok(result)
}

/// Import connections command handler
pub fn cmd_import(
    config_path: Option<&Path>,
    format: ImportFormatArg,
    file: &Path,
) -> Result<(), CliError> {
    if !file.exists() {
        return Err(CliError::Import(format!(
            "File not found: {}",
            file.display()
        )));
    }

    let config_manager = create_config_manager(config_path)?;

    let mut existing_connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load existing connections: {e}")))?;

    let mut existing_groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Config(format!("Failed to load existing groups: {e}")))?;

    let import_result = import_connections(format, file)?;

    println!("Import Summary:");
    println!(
        "  Connections imported: {}",
        import_result.connections.len()
    );
    println!("  Groups imported: {}", import_result.groups.len());
    println!("  Snippets imported: {}", import_result.snippets.len());
    println!("  Entries skipped: {}", import_result.skipped.len());
    println!("  Errors: {}", import_result.errors.len());

    if !import_result.skipped.is_empty() {
        for skipped in &import_result.skipped {
            if let Some(ref location) = skipped.location {
                tracing::warn!(
                    "Import skipped: {} ({}): {}",
                    skipped.identifier,
                    location,
                    skipped.reason
                );
            } else {
                tracing::warn!("Import skipped: {}: {}", skipped.identifier, skipped.reason);
            }
        }
    }

    if !import_result.errors.is_empty() {
        for error in &import_result.errors {
            tracing::error!("Import error: {error}");
        }
    }

    let initial_count = existing_connections.len();
    let initial_group_count = existing_groups.len();

    for group in import_result.groups {
        if !existing_groups.iter().any(|g| g.name == group.name) {
            existing_groups.push(group);
        }
    }

    for conn in import_result.connections {
        let is_duplicate = existing_connections
            .iter()
            .any(|c| c.name == conn.name && c.host == conn.host);

        if !is_duplicate {
            existing_connections.push(conn);
        }
    }

    let new_connections = existing_connections.len() - initial_count;
    let new_groups = existing_groups.len() - initial_group_count;

    // Merge snippets (native format only)
    let mut new_snippets = 0;
    if !import_result.snippets.is_empty() {
        let mut existing_snippets = config_manager
            .load_snippets()
            .map_err(|e| CliError::Config(format!("Failed to load existing snippets: {e}")))?;
        let initial_snippet_count = existing_snippets.len();

        for snippet in import_result.snippets {
            let is_duplicate = existing_snippets.iter().any(|s| s.name == snippet.name);
            if !is_duplicate {
                existing_snippets.push(snippet);
            }
        }

        new_snippets = existing_snippets.len() - initial_snippet_count;
        config_manager
            .save_snippets(&existing_snippets)
            .map_err(|e| CliError::Config(format!("Failed to save snippets: {e}")))?;
    }

    config_manager
        .save_connections(&existing_connections)
        .map_err(|e| CliError::Config(format!("Failed to save connections: {e}")))?;

    config_manager
        .save_groups(&existing_groups)
        .map_err(|e| CliError::Config(format!("Failed to save groups: {e}")))?;

    println!("\nMerge results:");
    println!("  New connections added: {new_connections}");
    println!("  New groups added: {new_groups}");
    println!("  New snippets added: {new_snippets}");
    println!("  Total connections: {}", existing_connections.len());
    println!("  Total groups: {}", existing_groups.len());

    Ok(())
}

/// Imports connections using the appropriate importer based on format
fn import_connections(
    format: ImportFormatArg,
    file: &Path,
) -> Result<rustconn_core::import::ImportResult, CliError> {
    use rustconn_core::import::{
        AnsibleInventoryImporter, AsbruImporter, ImportSource, MobaXtermImporter, RemminaImporter,
        RoyalTsImporter, SshConfigImporter,
    };

    let result = match format {
        ImportFormatArg::Ansible => {
            let importer = AnsibleInventoryImporter::new();
            importer
                .import_from_path(file)
                .map_err(|e| CliError::Import(e.to_string()))?
        }
        ImportFormatArg::SshConfig => {
            let importer = SshConfigImporter::new();
            importer
                .import_from_path(file)
                .map_err(|e| CliError::Import(e.to_string()))?
        }
        ImportFormatArg::Remmina => {
            let importer = RemminaImporter::new();
            importer
                .import_from_path(file)
                .map_err(|e| CliError::Import(e.to_string()))?
        }
        ImportFormatArg::Asbru => {
            let importer = AsbruImporter::new();
            importer
                .import_from_path(file)
                .map_err(|e| CliError::Import(e.to_string()))?
        }
        ImportFormatArg::Native => {
            let native = rustconn_core::export::NativeExport::from_file(file)
                .map_err(|e| CliError::Import(e.to_string()))?;

            rustconn_core::import::ImportResult {
                connections: native.connections,
                groups: native.groups,
                skipped: Vec::new(),
                errors: Vec::new(),
                credentials: std::collections::HashMap::new(),
                snippets: native.snippets,
            }
        }
        ImportFormatArg::RoyalTs => {
            let importer = RoyalTsImporter::new();
            importer
                .import_from_path(file)
                .map_err(|e| CliError::Import(e.to_string()))?
        }
        ImportFormatArg::MobaXterm => {
            let importer = MobaXtermImporter::with_path(file.to_path_buf());
            importer
                .import_from_path(file)
                .map_err(|e| CliError::Import(e.to_string()))?
        }
    };

    Ok(result)
}
