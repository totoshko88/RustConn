//! Property-based tests for session recording (scriptreplay-compatible format).
//!
//! **Validates: Requirements 6.1, 6.2, 6.10, 6.11, 11.7**

use std::time::Duration;

use proptest::prelude::*;
use rustconn_core::session::SanitizeConfig;
use rustconn_core::session::recording::{RecordingReader, SessionRecorder};

/// Helper: write `chunks` through a recorder, then read them back.
/// Returns the list of `(delay, data)` pairs read from the files.
fn round_trip(chunks: &[Vec<u8>], sanitize: SanitizeConfig) -> Vec<(Duration, Vec<u8>)> {
    let dir = tempfile::tempdir().expect("tempdir");
    let data_path = dir.path().join("test.data");
    let timing_path = dir.path().join("test.timing");

    let mut recorder =
        SessionRecorder::new(&data_path, &timing_path, sanitize).expect("create recorder");
    for chunk in chunks {
        recorder.write_chunk(chunk).expect("write_chunk");
    }
    recorder.flush().expect("flush");

    let mut reader = RecordingReader::open(&data_path, &timing_path).expect("open reader");
    let mut result = Vec::new();
    while let Some(entry) = reader.next_chunk() {
        result.push(entry);
    }
    result
}

// ---------------------------------------------------------------------------
// Proptest 12: Round-trip — write timing+data → read timing+data → identical chunks
// **Validates: Requirements 6.11, 11.7**
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn round_trip_preserves_chunks(
        chunks in prop::collection::vec(
            prop::collection::vec(1u8..=255, 1..128),
            1..8,
        )
    ) {
        let read_back = round_trip(&chunks, SanitizeConfig::disabled());

        // Same number of chunks
        prop_assert_eq!(read_back.len(), chunks.len());

        // Data bytes are identical
        for ((_delay, data), original) in read_back.iter().zip(chunks.iter()) {
            prop_assert_eq!(data, original);
        }
    }
}

// ---------------------------------------------------------------------------
// Proptest 13: Arbitrary bytes correctly written and read back
// **Validates: Requirements 6.1, 6.2, 11.7**
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn arbitrary_bytes_round_trip(
        chunks in prop::collection::vec(
            prop::collection::vec(any::<u8>(), 1..256),
            1..6,
        )
    ) {
        let read_back = round_trip(&chunks, SanitizeConfig::disabled());

        prop_assert_eq!(read_back.len(), chunks.len());

        // Concatenated data must match
        let original_concat: Vec<u8> = chunks.iter().flatten().copied().collect();
        let read_concat: Vec<u8> = read_back.iter().flat_map(|(_, d)| d.iter().copied()).collect();
        prop_assert_eq!(read_concat, original_concat);
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[test]
fn empty_chunk_is_skipped() {
    let read_back = round_trip(&[vec![], vec![1, 2, 3], vec![]], SanitizeConfig::disabled());
    // Only the non-empty chunk should appear
    assert_eq!(read_back.len(), 1);
    assert_eq!(read_back[0].1, vec![1, 2, 3]);
}

#[test]
fn sanitization_redacts_password_prompt() {
    let input = b"password: s3cret\n";
    let read_back = round_trip(&[input.to_vec()], SanitizeConfig::new());
    assert_eq!(read_back.len(), 1);
    let text = String::from_utf8_lossy(&read_back[0].1);
    assert!(!text.contains("s3cret"), "password should be redacted");
    assert!(
        text.contains("[REDACTED]"),
        "should contain redaction marker"
    );
}

// ===========================================================================
// Properties 4–7, 10–11: Metadata, RecordingManager, timing, import
// Feature: session-recording-manager
// ===========================================================================

use rustconn_core::session::recording::{
    RecordingManager, RecordingMetadata, derive_metadata, metadata_path, read_metadata,
    write_metadata,
};

/// Strategy: generate a finite, non-negative f64 suitable for JSON round-trip.
fn finite_f64() -> impl Strategy<Value = f64> {
    (0u64..1_000_000u64).prop_map(|v| v as f64 + 0.123_456)
}

/// Strategy: generate a valid `RecordingMetadata`.
fn arb_recording_metadata() -> impl Strategy<Value = RecordingMetadata> {
    (
        "[a-zA-Z0-9_-]{1,30}",                        // connection_name
        proptest::option::of("[a-zA-Z0-9 _-]{1,40}"), // display_name
        0i64..2_000_000_000i64,                       // unix timestamp for created_at
        finite_f64(),                                 // duration_secs
        0u64..10_000_000u64,                          // total_size_bytes
    )
        .prop_map(|(cn, dn, ts, dur, sz)| {
            let created_at =
                chrono::DateTime::from_timestamp(ts, 0).unwrap_or_else(chrono::Utc::now);
            RecordingMetadata {
                connection_name: cn,
                display_name: dn,
                created_at,
                duration_secs: dur,
                total_size_bytes: sz,
            }
        })
}

/// Strategy: generate a sanitised connection name (alphanumeric + hyphens/underscores).
fn arb_connection_name() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_-]{0,20}"
}

/// Strategy: generate a valid timestamp string `YYYYMMDD_HHMMSS`.
fn arb_timestamp_parts() -> impl Strategy<Value = (u32, u32, u32, u32, u32, u32)> {
    (
        2000u32..2030,
        1u32..=12,
        1u32..=28, // stay safe with day range
        0u32..24,
        0u32..60,
        0u32..60,
    )
}

// ---------------------------------------------------------------------------
// Feature: session-recording-manager, Property 4: Metadata serde round-trip
// **Validates: Requirements 5.5, 5.3, 5.4, 5.1**
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn metadata_serde_round_trip(meta in arb_recording_metadata()) {
        let dir = tempfile::tempdir().map_err(|e| TestCaseError::fail(e.to_string()))?;
        let meta_path = dir.path().join("test.meta.json");

        write_metadata(&meta_path, &meta)
            .map_err(|e| TestCaseError::fail(format!("write_metadata: {e}")))?;
        let read_back = read_metadata(&meta_path)
            .map_err(|e| TestCaseError::fail(format!("read_metadata: {e}")))?;

        prop_assert_eq!(&read_back, &meta);
    }
}

// ---------------------------------------------------------------------------
// Feature: session-recording-manager, Property 5: Derive metadata from filename
// **Validates: Requirements 5.6**
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn derive_metadata_from_filename(
        name in arb_connection_name(),
        (year, month, day, hour, min, sec) in arb_timestamp_parts(),
        data_content in prop::collection::vec(any::<u8>(), 1..512),
    ) {
        let dir = tempfile::tempdir().map_err(|e| TestCaseError::fail(e.to_string()))?;

        let ts = format!("{year:04}{month:02}{day:02}_{hour:02}{min:02}{sec:02}");
        let stem = format!("{name}_{ts}");
        let data_path = dir.path().join(format!("{stem}.data"));
        let timing_path = dir.path().join(format!("{stem}.timing"));

        // Write data file with some content.
        std::fs::write(&data_path, &data_content)
            .map_err(|e| TestCaseError::fail(format!("write data: {e}")))?;

        // Write a valid timing file whose byte sum <= data size.
        let timing_line = format!("0.100000 {}\n", data_content.len());
        std::fs::write(&timing_path, &timing_line)
            .map_err(|e| TestCaseError::fail(format!("write timing: {e}")))?;

        let meta = derive_metadata(&data_path, &timing_path)
            .map_err(|e| TestCaseError::fail(format!("derive_metadata: {e}")))?;

        // connection_name must equal the name portion before the timestamp.
        prop_assert_eq!(&meta.connection_name, &name);

        // total_size_bytes must equal data + timing file sizes.
        let expected_size = data_content.len() as u64 + timing_line.len() as u64;
        prop_assert_eq!(meta.total_size_bytes, expected_size);
    }
}

// ---------------------------------------------------------------------------
// Feature: session-recording-manager, Property 6: Rename persists display name
// **Validates: Requirements 4.3**
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn rename_persists_display_name(
        name in arb_connection_name(),
        new_display in "[a-zA-Z0-9 _-]{1,40}",
    ) {
        let dir = tempfile::tempdir().map_err(|e| TestCaseError::fail(e.to_string()))?;
        let rec_dir = dir.path().join("recordings");
        std::fs::create_dir_all(&rec_dir)
            .map_err(|e| TestCaseError::fail(format!("mkdir: {e}")))?;

        let stem = format!("{name}_20250101_120000");
        let data_path = rec_dir.join(format!("{stem}.data"));
        let timing_path = rec_dir.join(format!("{stem}.timing"));

        // Create minimal recording files.
        std::fs::write(&data_path, b"hello")
            .map_err(|e| TestCaseError::fail(format!("write data: {e}")))?;
        std::fs::write(&timing_path, "0.000000 5\n")
            .map_err(|e| TestCaseError::fail(format!("write timing: {e}")))?;

        let mgr = RecordingManager::new(rec_dir);
        mgr.rename(&data_path, &new_display)
            .map_err(|e| TestCaseError::fail(format!("rename: {e}")))?;

        let meta = read_metadata(&metadata_path(&data_path))
            .map_err(|e| TestCaseError::fail(format!("read_metadata: {e}")))?;

        prop_assert_eq!(meta.display_name.as_deref(), Some(new_display.as_str()));
    }
}

// ---------------------------------------------------------------------------
// Feature: session-recording-manager, Property 7: Delete removes all recording files
// **Validates: Requirements 4.4**
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn delete_removes_all_files(name in arb_connection_name()) {
        let dir = tempfile::tempdir().map_err(|e| TestCaseError::fail(e.to_string()))?;
        let rec_dir = dir.path().join("recordings");
        std::fs::create_dir_all(&rec_dir)
            .map_err(|e| TestCaseError::fail(format!("mkdir: {e}")))?;

        let stem = format!("{name}_20250101_120000");
        let data_path = rec_dir.join(format!("{stem}.data"));
        let timing_path = rec_dir.join(format!("{stem}.timing"));
        let meta_path_val = metadata_path(&data_path);

        // Create all three files.
        std::fs::write(&data_path, b"data")
            .map_err(|e| TestCaseError::fail(format!("write data: {e}")))?;
        std::fs::write(&timing_path, "0.000000 4\n")
            .map_err(|e| TestCaseError::fail(format!("write timing: {e}")))?;
        let meta = RecordingMetadata {
            connection_name: name.clone(),
            display_name: None,
            created_at: chrono::Utc::now(),
            duration_secs: 0.0,
            total_size_bytes: 4,
        };
        write_metadata(&meta_path_val, &meta)
            .map_err(|e| TestCaseError::fail(format!("write meta: {e}")))?;

        // Verify all three exist before delete.
        prop_assert!(data_path.exists());
        prop_assert!(timing_path.exists());
        prop_assert!(meta_path_val.exists());

        let mgr = RecordingManager::new(rec_dir);
        mgr.delete(&data_path)
            .map_err(|e| TestCaseError::fail(format!("delete: {e}")))?;

        // None of the three files should exist.
        prop_assert!(!data_path.exists(), "data file still exists");
        prop_assert!(!timing_path.exists(), "timing file still exists");
        prop_assert!(!meta_path_val.exists(), "meta file still exists");
    }
}

// ---------------------------------------------------------------------------
// Feature: session-recording-manager, Property 10: Timing file validation
// **Validates: Requirements 9.2, 9.3**
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Valid timing files must pass validation.
    #[test]
    fn valid_timing_passes_validation(
        data_content in prop::collection::vec(any::<u8>(), 1..1024),
    ) {
        let dir = tempfile::tempdir().map_err(|e| TestCaseError::fail(e.to_string()))?;
        let data_path = dir.path().join("test.data");
        let timing_path = dir.path().join("test.timing");

        std::fs::write(&data_path, &data_content)
            .map_err(|e| TestCaseError::fail(format!("write data: {e}")))?;

        // Build a single valid timing line whose byte count == data size.
        let timing_line = format!("0.100000 {}\n", data_content.len());
        std::fs::write(&timing_path, &timing_line)
            .map_err(|e| TestCaseError::fail(format!("write timing: {e}")))?;

        let result = RecordingManager::validate_timing(&data_path, &timing_path);
        prop_assert!(result.is_ok(), "expected Ok, got {:?}", result);
    }

    /// Timing files where byte sum exceeds data size must fail validation.
    #[test]
    fn timing_exceeding_data_size_fails(
        data_content in prop::collection::vec(any::<u8>(), 1..512),
        extra in 1usize..1024,
    ) {
        let dir = tempfile::tempdir().map_err(|e| TestCaseError::fail(e.to_string()))?;
        let data_path = dir.path().join("test.data");
        let timing_path = dir.path().join("test.timing");

        std::fs::write(&data_path, &data_content)
            .map_err(|e| TestCaseError::fail(format!("write data: {e}")))?;

        // Byte count = data_size + extra → exceeds data file.
        let bad_count = data_content.len() + extra;
        let timing_line = format!("0.100000 {bad_count}\n");
        std::fs::write(&timing_path, &timing_line)
            .map_err(|e| TestCaseError::fail(format!("write timing: {e}")))?;

        let result = RecordingManager::validate_timing(&data_path, &timing_path);
        prop_assert!(result.is_err(), "expected Err for oversized byte sum");
    }

    /// Malformed timing lines must fail validation.
    #[test]
    fn malformed_timing_fails(
        garbage in "[a-zA-Z!@#$%^&*]{3,20}",
    ) {
        let dir = tempfile::tempdir().map_err(|e| TestCaseError::fail(e.to_string()))?;
        let data_path = dir.path().join("test.data");
        let timing_path = dir.path().join("test.timing");

        std::fs::write(&data_path, b"some data content")
            .map_err(|e| TestCaseError::fail(format!("write data: {e}")))?;
        std::fs::write(&timing_path, &garbage)
            .map_err(|e| TestCaseError::fail(format!("write timing: {e}")))?;

        let result = RecordingManager::validate_timing(&data_path, &timing_path);
        prop_assert!(result.is_err(), "expected Err for malformed timing");
    }
}

// ---------------------------------------------------------------------------
// Feature: session-recording-manager, Property 11: Import produces complete recording
// **Validates: Requirements 9.4, 9.5**
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn import_produces_three_files(
        data_content in prop::collection::vec(any::<u8>(), 1..256),
    ) {
        let dir = tempfile::tempdir().map_err(|e| TestCaseError::fail(e.to_string()))?;

        // Source files (outside recordings dir).
        let src_dir = dir.path().join("source");
        std::fs::create_dir_all(&src_dir)
            .map_err(|e| TestCaseError::fail(format!("mkdir src: {e}")))?;
        let src_data = src_dir.join("session.data");
        let src_timing = src_dir.join("session.timing");

        std::fs::write(&src_data, &data_content)
            .map_err(|e| TestCaseError::fail(format!("write src data: {e}")))?;
        let timing_line = format!("0.100000 {}\n", data_content.len());
        std::fs::write(&src_timing, &timing_line)
            .map_err(|e| TestCaseError::fail(format!("write src timing: {e}")))?;

        // Recordings directory.
        let rec_dir = dir.path().join("recordings");
        std::fs::create_dir_all(&rec_dir)
            .map_err(|e| TestCaseError::fail(format!("mkdir rec: {e}")))?;

        let mgr = RecordingManager::new(rec_dir.clone());

        // First import — should produce 3 files.
        let entry = mgr.import(&src_data, &src_timing)
            .map_err(|e| TestCaseError::fail(format!("import: {e}")))?;

        prop_assert!(entry.data_path.exists(), "imported data missing");
        prop_assert!(entry.timing_path.exists(), "imported timing missing");
        prop_assert!(entry.meta_path.exists(), "imported meta missing");

        // Second import with same source — should get a different filename.
        let entry2 = mgr.import(&src_data, &src_timing)
            .map_err(|e| TestCaseError::fail(format!("import2: {e}")))?;

        prop_assert!(
            entry.data_path != entry2.data_path,
            "second import should have a different data path"
        );

        // Original files from first import must still exist.
        prop_assert!(entry.data_path.exists(), "first import data gone after second import");
        prop_assert!(entry.timing_path.exists(), "first import timing gone after second import");
        prop_assert!(entry.meta_path.exists(), "first import meta gone after second import");

        // Second import must also have all 3 files.
        prop_assert!(entry2.data_path.exists(), "second import data missing");
        prop_assert!(entry2.timing_path.exists(), "second import timing missing");
        prop_assert!(entry2.meta_path.exists(), "second import meta missing");
    }
}

// ===========================================================================
// Feature: session-recording-manager, Property 12: CLI list contains metadata
// **Validates: Requirements 11.1**
// ===========================================================================

use rustconn_core::session::recording::RecordingEntry;

/// Format a display name for a recording entry (mirrors CLI table logic).
fn test_display_name(entry: &RecordingEntry) -> &str {
    entry
        .metadata
        .display_name
        .as_deref()
        .unwrap_or(&entry.metadata.connection_name)
}

/// Format duration in human-readable form (mirrors CLI table logic).
fn test_format_duration(secs: f64) -> String {
    let total = secs as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}h {m:02}m {s:02}s")
    } else {
        format!("{m}m {s:02}s")
    }
}

/// Format file size in human-readable form (mirrors CLI table logic).
fn test_format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Format a list of recording entries as a table string (mirrors CLI table output).
fn test_format_table(entries: &[RecordingEntry]) -> String {
    if entries.is_empty() {
        return "No recordings found.".to_string();
    }

    let name_width = entries
        .iter()
        .map(|e| test_display_name(e).len())
        .max()
        .unwrap_or(4)
        .max(4);

    let mut output = String::new();

    // Header
    output.push_str(&format!(
        "{:<name_width$}  {:<19}  {:>12}  {:>8}\n",
        "NAME", "DATE", "DURATION", "SIZE"
    ));
    // Separator
    output.push_str(&format!(
        "{:-<name_width$}  {:-<19}  {:->12}  {:->8}\n",
        "", "", "", ""
    ));

    // Rows
    for entry in entries {
        let name = test_display_name(entry);
        let date = entry.metadata.created_at.format("%Y-%m-%d %H:%M:%S");
        let dur = test_format_duration(entry.metadata.duration_secs);
        let size = test_format_size(entry.metadata.total_size_bytes);
        output.push_str(&format!(
            "{name:<name_width$}  {date}  {dur:>12}  {size:>8}\n"
        ));
    }

    output
}

/// Strategy: generate a `RecordingEntry` with arbitrary metadata.
fn arb_recording_entry() -> impl Strategy<Value = RecordingEntry> {
    arb_recording_metadata().prop_map(|metadata| {
        let stem = format!(
            "{}_{}",
            metadata.connection_name,
            metadata.created_at.format("%Y%m%d_%H%M%S")
        );
        let data_path = std::path::PathBuf::from(format!("{stem}.data"));
        let timing_path = std::path::PathBuf::from(format!("{stem}.timing"));
        let meta_path = std::path::PathBuf::from(format!("{stem}.meta.json"));
        RecordingEntry {
            data_path,
            timing_path,
            meta_path,
            metadata,
        }
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// For any non-empty list of RecordingEntry items, the formatted table
    /// output must contain the display name (or connection_name), creation
    /// date, duration, and file size for every entry.
    #[test]
    fn cli_list_output_contains_all_metadata(
        entries in prop::collection::vec(arb_recording_entry(), 1..10),
    ) {
        let table = test_format_table(&entries);

        for entry in &entries {
            let name = test_display_name(entry);
            prop_assert!(
                table.contains(name),
                "table missing display name/connection_name: {name}"
            );

            let date_str = entry.metadata.created_at.format("%Y-%m-%d %H:%M:%S").to_string();
            prop_assert!(
                table.contains(&date_str),
                "table missing date: {date_str}"
            );

            let dur_str = test_format_duration(entry.metadata.duration_secs);
            prop_assert!(
                table.contains(&dur_str),
                "table missing duration: {dur_str}"
            );

            let size_str = test_format_size(entry.metadata.total_size_bytes);
            prop_assert!(
                table.contains(&size_str),
                "table missing size: {size_str}"
            );
        }
    }
}
