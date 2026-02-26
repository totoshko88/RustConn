//! Property tests for export format functionality

use proptest::prelude::*;
use rustconn_core::export::{ExportError, ExportFormat, ExportOptions, ExportResult};
use std::path::PathBuf;

proptest! {
    /// Property: ExportFormat::all returns all formats
    #[test]
    fn export_format_all_returns_all(_dummy in 0..1) {
        let all = ExportFormat::all();
        prop_assert_eq!(all.len(), 7);
        prop_assert!(all.contains(&ExportFormat::Ansible));
        prop_assert!(all.contains(&ExportFormat::SshConfig));
        prop_assert!(all.contains(&ExportFormat::Remmina));
        prop_assert!(all.contains(&ExportFormat::Asbru));
        prop_assert!(all.contains(&ExportFormat::Native));
        prop_assert!(all.contains(&ExportFormat::RoyalTs));
        prop_assert!(all.contains(&ExportFormat::MobaXterm));
    }

    /// Property: Each ExportFormat has a non-empty display name
    #[test]
    fn export_format_has_display_name(_dummy in 0..1) {
        for format in ExportFormat::all() {
            let name = format.display_name();
            prop_assert!(!name.is_empty());
            prop_assert!(name.len() >= 3); // At least 3 chars
        }
    }

    /// Property: Each ExportFormat has a non-empty file extension
    #[test]
    fn export_format_has_file_extension(_dummy in 0..1) {
        for format in ExportFormat::all() {
            let ext = format.file_extension();
            prop_assert!(!ext.is_empty());
            prop_assert!(!ext.contains('.')); // Extension without dot
        }
    }

    /// Property: Only Remmina exports to directory
    #[test]
    fn only_remmina_exports_to_directory(_dummy in 0..1) {
        for format in ExportFormat::all() {
            let exports_to_dir = format.exports_to_directory();
            if *format == ExportFormat::Remmina {
                prop_assert!(exports_to_dir);
            } else {
                prop_assert!(!exports_to_dir);
            }
        }
    }

    /// Property: ExportFormat Display matches display_name
    #[test]
    fn export_format_display_matches_name(_dummy in 0..1) {
        for format in ExportFormat::all() {
            let display = format.to_string();
            let name = format.display_name();
            prop_assert_eq!(display, name);
        }
    }

    /// Property: ExportOptions preserves format and path
    #[test]
    fn export_options_preserves_fields(
        path in "[a-z/]{1,50}\\.[a-z]{2,4}",
    ) {
        for format in ExportFormat::all() {
            let options = ExportOptions::new(*format, PathBuf::from(&path));
            prop_assert_eq!(options.format, *format);
            prop_assert_eq!(options.output_path, PathBuf::from(&path));
            prop_assert!(options.include_groups); // Default
        }
    }

    /// Property: ExportOptions builder methods work correctly
    #[test]
    fn export_options_builder_works(
        include_groups in proptest::bool::ANY,
    ) {
        let options = ExportOptions::new(ExportFormat::Ansible, PathBuf::from("/tmp/test.ini"))
            .with_groups(include_groups);

        prop_assert_eq!(options.include_groups, include_groups);
    }

    /// Property: ExportResult starts empty
    #[test]
    fn export_result_starts_empty(_dummy in 0..1) {
        let result = ExportResult::new();
        prop_assert_eq!(result.exported_count, 0);
        prop_assert_eq!(result.skipped_count, 0);
        prop_assert!(result.warnings.is_empty());
        prop_assert!(result.output_files.is_empty());
        prop_assert_eq!(result.total_processed(), 0);
        prop_assert!(!result.has_skipped());
        prop_assert!(!result.has_warnings());
    }

    /// Property: ExportResult total_processed is sum of exported and skipped
    #[test]
    fn export_result_total_is_sum(
        exported in 0usize..1000,
        skipped in 0usize..1000,
    ) {
        let mut result = ExportResult::new();
        result.exported_count = exported;
        result.skipped_count = skipped;

        prop_assert_eq!(result.total_processed(), exported + skipped);
    }

    /// Property: ExportResult has_skipped is true iff skipped_count > 0
    #[test]
    fn export_result_has_skipped_consistency(skipped in 0usize..100) {
        let mut result = ExportResult::new();
        result.skipped_count = skipped;

        prop_assert_eq!(result.has_skipped(), skipped > 0);
    }

    /// Property: ExportResult has_warnings is true iff warnings not empty
    #[test]
    fn export_result_has_warnings_consistency(warning_count in 0usize..10) {
        let mut result = ExportResult::new();
        for i in 0..warning_count {
            result.add_warning(format!("Warning {i}"));
        }

        prop_assert_eq!(result.has_warnings(), warning_count > 0);
        prop_assert_eq!(result.warnings.len(), warning_count);
    }

    /// Property: ExportResult increment methods work correctly
    #[test]
    fn export_result_increment_works(
        export_increments in 0usize..50,
        skip_increments in 0usize..50,
    ) {
        let mut result = ExportResult::new();

        for _ in 0..export_increments {
            result.increment_exported();
        }
        for _ in 0..skip_increments {
            result.increment_skipped();
        }

        prop_assert_eq!(result.exported_count, export_increments);
        prop_assert_eq!(result.skipped_count, skip_increments);
    }

    /// Property: ExportResult summary contains all counts
    #[test]
    fn export_result_summary_contains_counts(
        exported in 0usize..100,
        skipped in 0usize..100,
        warnings in 0usize..10,
    ) {
        let mut result = ExportResult::new();
        result.exported_count = exported;
        result.skipped_count = skipped;
        for i in 0..warnings {
            result.add_warning(format!("Warning {i}"));
        }

        let summary = result.summary();
        let exported_str = format!("Exported: {exported}");
        let skipped_str = format!("Skipped: {skipped}");
        let warnings_str = format!("Warnings: {warnings}");

        prop_assert!(summary.contains(&exported_str));
        prop_assert!(summary.contains(&skipped_str));
        prop_assert!(summary.contains(&warnings_str));
    }

    /// Property: ExportResult add_output_file accumulates files
    #[test]
    fn export_result_add_output_file_accumulates(file_count in 1usize..10) {
        let mut result = ExportResult::new();

        for i in 0..file_count {
            result.add_output_file(PathBuf::from(format!("/tmp/file{i}.txt")));
        }

        prop_assert_eq!(result.output_files.len(), file_count);
    }

    /// Property: ExportError variants have meaningful messages
    #[test]
    fn export_error_has_message(msg in "[a-zA-Z0-9 ]{1,50}") {
        let errors = vec![
            ExportError::UnsupportedProtocol(msg.clone()),
            ExportError::WriteError(msg.clone()),
            ExportError::InvalidData(msg.clone()),
            ExportError::InvalidPath(msg.clone()),
            ExportError::Serialization(msg.clone()),
        ];

        for error in errors {
            let error_str = error.to_string();
            prop_assert!(!error_str.is_empty());
            prop_assert!(error_str.contains(&msg));
        }
    }

    /// Property: ExportError::Cancelled has fixed message
    #[test]
    fn export_error_cancelled_message(_dummy in 0..1) {
        let error = ExportError::Cancelled;
        prop_assert_eq!(error.to_string(), "Export cancelled");
    }
}

#[test]
fn test_export_format_equality() {
    assert_eq!(ExportFormat::Ansible, ExportFormat::Ansible);
    assert_ne!(ExportFormat::Ansible, ExportFormat::SshConfig);
}

#[test]
fn test_export_format_clone() {
    let format = ExportFormat::Native;
    let cloned = format;
    assert_eq!(format, cloned);
}

#[test]
fn test_export_format_debug() {
    let format = ExportFormat::RoyalTs;
    let debug = format!("{format:?}");
    assert!(debug.contains("RoyalTs"));
}

#[test]
fn test_export_options_default_values() {
    let options = ExportOptions::new(ExportFormat::Asbru, PathBuf::from("/tmp/test.yml"));

    // Default: include groups
    assert!(options.include_groups);
}

#[test]
fn test_export_result_default() {
    let result = ExportResult::default();
    assert_eq!(result.exported_count, 0);
    assert_eq!(result.skipped_count, 0);
}

#[test]
fn test_export_error_io() {
    let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
    let export_error: ExportError = io_error.into();

    let error_str = export_error.to_string();
    assert!(error_str.contains("IO error"));
}

#[test]
fn test_export_format_serialization() {
    // Test JSON serialization
    let format = ExportFormat::Native;
    let json = serde_json::to_string(&format).expect("serialize");
    assert_eq!(json, "\"native\"");

    let deserialized: ExportFormat = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(deserialized, format);
}

#[test]
fn test_all_export_formats_serialize() {
    for format in ExportFormat::all() {
        let json = serde_json::to_string(format).expect("serialize");
        let deserialized: ExportFormat = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(&deserialized, format);
    }
}

#[test]
fn test_export_format_file_extensions_unique() {
    let extensions: Vec<&str> = ExportFormat::all()
        .iter()
        .map(|f| f.file_extension())
        .collect();

    // Check that most extensions are unique (some may share)
    let unique_count = {
        let mut sorted = extensions.clone();
        sorted.sort();
        sorted.dedup();
        sorted.len()
    };

    // At least 5 unique extensions
    assert!(unique_count >= 5);
}

#[test]
fn test_export_result_warnings_preserved() {
    let mut result = ExportResult::new();
    result.add_warning("First warning");
    result.add_warning("Second warning");
    result.add_warning("Third warning");

    assert_eq!(result.warnings.len(), 3);
    assert_eq!(result.warnings[0], "First warning");
    assert_eq!(result.warnings[1], "Second warning");
    assert_eq!(result.warnings[2], "Third warning");
}
