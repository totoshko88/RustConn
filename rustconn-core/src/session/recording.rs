//! Session recording in `scriptreplay`-compatible format.
//!
//! Produces two files per session:
//! - **data file** — raw (optionally sanitised) terminal output bytes
//! - **timing file** — one line per chunk: `{delay_seconds} {byte_count}\n`
//!
//! The pair can be replayed with `scriptreplay timing data`.

use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::logger::{SanitizeConfig, sanitize_output};

// ---------------------------------------------------------------------------
// SessionRecorder
// ---------------------------------------------------------------------------

/// Records terminal output into scriptreplay-compatible data + timing files.
pub struct SessionRecorder {
    data_file: BufWriter<File>,
    timing_file: BufWriter<File>,
    last_timestamp: Instant,
    sanitize: SanitizeConfig,
}

impl SessionRecorder {
    /// Creates a new recorder, opening (or creating) the data and timing files.
    ///
    /// # Errors
    ///
    /// Returns `io::Error` if the files cannot be created.
    pub fn new(
        data_path: impl AsRef<Path>,
        timing_path: impl AsRef<Path>,
        sanitize: SanitizeConfig,
    ) -> io::Result<Self> {
        let data_file = BufWriter::new(File::create(data_path)?);
        let timing_file = BufWriter::new(File::create(timing_path)?);
        Ok(Self {
            data_file,
            timing_file,
            last_timestamp: Instant::now(),
            sanitize,
        })
    }

    /// Writes a chunk of terminal output, applying sanitisation and recording
    /// the timing entry.
    ///
    /// # Errors
    ///
    /// Returns `io::Error` on write failure.
    pub fn write_chunk(&mut self, data: &[u8]) -> io::Result<()> {
        if data.is_empty() {
            return Ok(());
        }

        let now = Instant::now();
        let delay = now.duration_since(self.last_timestamp);
        self.last_timestamp = now;

        // Sanitise if enabled — operates on the UTF-8 interpretation of the
        // bytes; non-UTF-8 chunks are written as-is.
        let sanitised: Vec<u8> = if self.sanitize.enabled {
            match std::str::from_utf8(data) {
                Ok(text) => sanitize_output(text, &self.sanitize).into_bytes(),
                Err(_) => data.to_vec(),
            }
        } else {
            data.to_vec()
        };

        // Write data
        self.data_file.write_all(&sanitised)?;

        // Write timing line: "{delay_secs} {byte_count}\n"
        let secs = delay.as_secs_f64();
        writeln!(self.timing_file, "{secs:.6} {}", sanitised.len())?;

        Ok(())
    }

    /// Flushes both underlying files.
    ///
    /// # Errors
    ///
    /// Returns `io::Error` on flush failure.
    pub fn flush(&mut self) -> io::Result<()> {
        self.data_file.flush()?;
        self.timing_file.flush()?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// RecordingReader
// ---------------------------------------------------------------------------

/// Reads a previously recorded session (data + timing files).
pub struct RecordingReader {
    data: Vec<u8>,
    timing_entries: Vec<(Duration, usize)>,
    position: usize,
    entry_index: usize,
}

impl RecordingReader {
    /// Opens a recorded session from the given data and timing file paths.
    ///
    /// # Errors
    ///
    /// Returns `io::Error` if the files cannot be read or the timing file
    /// contains malformed lines.
    pub fn open(data_path: impl AsRef<Path>, timing_path: impl AsRef<Path>) -> io::Result<Self> {
        let data = fs::read(data_path)?;
        let timing_file = BufReader::new(File::open(timing_path)?);

        let mut timing_entries = Vec::new();
        for line in timing_file.lines() {
            let line = line?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let mut parts = trimmed.split_whitespace();
            let secs: f64 = parts
                .next()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad timing delay"))?;
            let count: usize = parts
                .next()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad timing count"))?;
            timing_entries.push((Duration::from_secs_f64(secs), count));
        }

        // Strip the `script` header line ("Script started on ...") from the
        // beginning of the data file.  The header is always a single line
        // terminated by `\n`.  We also reduce the first timing entry's byte
        // count so that data ↔ timing alignment is preserved.
        let (data, timing_entries) = strip_script_header(data, timing_entries);
        // Also strip the echoed `script -q -f --log-out …` command that
        // appears in the data file (the VTE erase sequence is sent via
        // `feed()` and is NOT captured by `script`).
        let (data, timing_entries) = strip_script_command_echo(data, timing_entries);

        Ok(Self {
            data,
            timing_entries,
            position: 0,
            entry_index: 0,
        })
    }

    /// Returns the next chunk together with its delay, or `None` when all
    /// chunks have been consumed.
    pub fn next_chunk(&mut self) -> Option<(Duration, Vec<u8>)> {
        if self.entry_index >= self.timing_entries.len() {
            return None;
        }
        let (delay, count) = self.timing_entries[self.entry_index];
        let end = (self.position + count).min(self.data.len());
        let chunk = self.data[self.position..end].to_vec();
        self.position = end;
        self.entry_index += 1;
        Some((delay, chunk))
    }
}

/// Strip the `Script started on …` header that `script --log-out` writes to
/// the data file.  Returns the trimmed data and adjusted timing entries so
/// that byte offsets stay consistent.
fn strip_script_header(
    mut data: Vec<u8>,
    mut timing: Vec<(Duration, usize)>,
) -> (Vec<u8>, Vec<(Duration, usize)>) {
    const PREFIX: &[u8] = b"Script started on ";
    if !data.starts_with(PREFIX) {
        return (data, timing);
    }
    // Find the end of the header line (first '\n')
    let header_len = data.iter().position(|&b| b == b'\n').map_or(0, |i| i + 1);
    if header_len == 0 {
        return (data, timing);
    }
    data.drain(..header_len);

    // Adjust timing entries: subtract the stripped bytes from the first
    // entries until the full header_len is accounted for.
    let mut remaining = header_len;
    while remaining > 0 && !timing.is_empty() {
        let (delay, count) = timing[0];
        if count <= remaining {
            remaining -= count;
            timing.remove(0);
            // Shift the delay of the removed entry to the next one
            if !timing.is_empty() {
                timing[0].0 += delay;
            }
        } else {
            timing[0] = (delay, count - remaining);
            remaining = 0;
        }
    }
    (data, timing)
}

/// Strip the echoed `script -q -f --log-out …` command from the beginning of
/// the data file.  RustConn prefixes the command with a space (for
/// `HISTCONTROL=ignorespace`) so we look for ` script -q -f --log-out` in the
/// first 512 bytes.  The entire line up to and including the trailing `\n` is
/// removed, and timing entries are adjusted identically to
/// [`strip_script_header`].
fn strip_script_command_echo(
    mut data: Vec<u8>,
    mut timing: Vec<(Duration, usize)>,
) -> (Vec<u8>, Vec<(Duration, usize)>) {
    const MARKER: &[u8] = b"script -q -f --log-out";
    let search_len = data.len().min(512);
    let search_window = &data[..search_len];

    // Find the marker within the first 512 bytes
    let Some(marker_pos) = search_window
        .windows(MARKER.len())
        .position(|w| w == MARKER)
    else {
        return (data, timing);
    };

    // Walk backwards to the start of the line (or start of data)
    let line_start = data[..marker_pos]
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |i| i + 1);

    // Walk forwards to the end of the line
    let line_end = data[marker_pos..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(data.len(), |i| marker_pos + i + 1);

    let strip_len = line_end - line_start;
    if strip_len == 0 {
        return (data, timing);
    }

    data.drain(line_start..line_end);

    // Adjust timing entries: skip `line_start` bytes first, then subtract
    // `strip_len` bytes from the entries that covered the removed range.
    let mut skip = line_start;
    let mut remaining = strip_len;
    let mut i = 0;
    // Skip entries that are entirely before the removed range
    while skip > 0 && i < timing.len() {
        let count = timing[i].1;
        if count <= skip {
            skip -= count;
            i += 1;
        } else {
            // Entry partially before the range — split: keep the prefix part
            // and start removing from the rest.
            let before = skip;
            skip = 0;
            let after = count - before;
            if after <= remaining {
                remaining -= after;
                timing[i].1 = before;
                i += 1;
            } else {
                timing[i].1 = before + (after - remaining);
                remaining = 0;
            }
        }
    }
    // Remove entries that fall entirely within the stripped range
    while remaining > 0 && i < timing.len() {
        let (delay, count) = timing[i];
        if count <= remaining {
            remaining -= count;
            timing.remove(i);
            if i < timing.len() {
                timing[i].0 += delay;
            }
        } else {
            timing[i] = (delay, count - remaining);
            remaining = 0;
        }
    }

    (data, timing)
}

// ---------------------------------------------------------------------------
// Recording path helpers
// ---------------------------------------------------------------------------

/// Returns the default recordings directory (`$XDG_DATA_HOME/rustconn/recordings/`).
///
/// Returns `None` when the data directory cannot be determined.
#[must_use]
pub fn default_recordings_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("rustconn").join("recordings"))
}

/// Builds the data and timing file paths for a new recording.
///
/// The connection name is sanitised for use in filenames and the current UTC
/// timestamp is appended.
///
/// Returns `(data_path, timing_path)`.
#[must_use]
pub fn recording_paths(dir: &Path, connection_name: &str) -> (PathBuf, PathBuf) {
    let sanitised = sanitize_recording_name(connection_name);
    let ts = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let base = format!("{sanitised}_{ts}");
    (
        dir.join(format!("{base}.data")),
        dir.join(format!("{base}.timing")),
    )
}

/// Ensures the recordings directory exists and is writable.
///
/// Returns `Ok(path)` on success, or an `io::Error` if the directory cannot be
/// created or is not writable.
///
/// # Errors
///
/// Returns `io::Error` if the directory cannot be created.
pub fn ensure_recordings_dir(dir: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    // Quick writability check — try creating a temp file.
    let probe = dir.join(".rustconn_probe");
    File::create(&probe)?;
    let _ = fs::remove_file(&probe);
    Ok(dir.to_path_buf())
}

/// Sanitises a connection name for use in a filename.
fn sanitize_recording_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect()
}

// ---------------------------------------------------------------------------
// RecordingMetadata & sidecar helpers
// ---------------------------------------------------------------------------

use serde::{Deserialize, Serialize};

/// Metadata for a recorded session, stored as a JSON sidecar file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordingMetadata {
    /// Original connection name at the time of recording.
    pub connection_name: String,
    /// User-defined display name (editable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// UTC timestamp when recording started.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Recording duration in seconds.
    pub duration_secs: f64,
    /// Combined size of data + timing files in bytes.
    pub total_size_bytes: u64,
}

/// Derives the `.meta.json` path from a data file path.
///
/// Replaces the extension of the data file with `.meta.json`.
#[must_use]
pub fn metadata_path(data_path: &Path) -> PathBuf {
    let stem = data_path.file_stem().unwrap_or_default().to_string_lossy();
    data_path.with_file_name(format!("{stem}.meta.json"))
}

/// Reads metadata from a JSON sidecar file.
///
/// # Errors
///
/// Returns `io::Error` if the file cannot be read or contains invalid JSON.
pub fn read_metadata(meta_path: &Path) -> io::Result<RecordingMetadata> {
    let content = fs::read_to_string(meta_path)?;
    serde_json::from_str(&content).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Writes metadata to a JSON sidecar file (pretty-printed).
///
/// # Errors
///
/// Returns `io::Error` if the file cannot be written.
pub fn write_metadata(meta_path: &Path, meta: &RecordingMetadata) -> io::Result<()> {
    let json = serde_json::to_string_pretty(meta).map_err(io::Error::other)?;
    fs::write(meta_path, json)
}

/// Derives metadata from filename pattern and filesystem attributes.
///
/// Expects the data file to follow the naming pattern
/// `{connection_name}_{YYYYMMDD_HHMMSS}.data`. Falls back to the full stem as
/// `connection_name` when the pattern does not match.
///
/// Duration is computed by summing all delay values in the timing file.
///
/// # Errors
///
/// Returns `io::Error` if the files cannot be read or the timing file is
/// malformed.
pub fn derive_metadata(data_path: &Path, timing_path: &Path) -> io::Result<RecordingMetadata> {
    let stem = data_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    // Try to parse "{name}_{YYYYMMDD_HHMMSS}" from the stem.
    let (connection_name, created_at) = parse_recording_stem(&stem);

    // File sizes.
    let data_size = fs::metadata(data_path).map(|m| m.len()).unwrap_or(0);
    let timing_size = fs::metadata(timing_path).map(|m| m.len()).unwrap_or(0);

    // Duration: sum of all delay values in the timing file.
    let duration_secs = sum_timing_delays(timing_path)?;

    Ok(RecordingMetadata {
        connection_name,
        display_name: None,
        created_at,
        duration_secs,
        total_size_bytes: data_size + timing_size,
    })
}

/// Parses a recording stem of the form `{name}_{YYYYMMDD_HHMMSS}`.
///
/// Returns `(connection_name, created_at)`. When the timestamp suffix cannot be
/// parsed the full stem is used as the connection name and `Utc::now()` is
/// returned as a fallback.
fn parse_recording_stem(stem: &str) -> (String, chrono::DateTime<chrono::Utc>) {
    // The timestamp suffix is exactly 15 chars: YYYYMMDD_HHMMSS
    // preceded by an underscore separator, so we need at least 16 chars
    // after the connection name.
    if let Some(sep_pos) = stem.len().checked_sub(16)
        && stem.as_bytes().get(sep_pos) == Some(&b'_')
    {
        let ts_str = &stem[sep_pos + 1..];
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(ts_str, "%Y%m%d_%H%M%S") {
            let name = stem[..sep_pos].to_string();
            let dt = naive.and_utc();
            return (name, dt);
        }
    }
    // Fallback: use full stem as name, current time as timestamp.
    (stem.to_string(), chrono::Utc::now())
}

/// Sums all delay values in a timing file.
fn sum_timing_delays(timing_path: &Path) -> io::Result<f64> {
    let file = BufReader::new(File::open(timing_path)?);
    let mut total = 0.0_f64;
    for line in file.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let secs: f64 = trimmed
            .split_whitespace()
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad timing delay"))?;
        total += secs;
    }
    Ok(total)
}

// ---------------------------------------------------------------------------
// RecordingEntry
// ---------------------------------------------------------------------------

/// A recording entry with resolved file paths and metadata.
#[derive(Debug, Clone)]
pub struct RecordingEntry {
    /// Path to the `.data` file.
    pub data_path: PathBuf,
    /// Path to the `.timing` file.
    pub timing_path: PathBuf,
    /// Path to the `.meta.json` sidecar file.
    pub meta_path: PathBuf,
    /// Parsed or derived metadata.
    pub metadata: RecordingMetadata,
}

// ---------------------------------------------------------------------------
// RecordingManager
// ---------------------------------------------------------------------------

/// Manages recording files on disk (list, delete, import, rename).
pub struct RecordingManager {
    recordings_dir: PathBuf,
}

impl RecordingManager {
    /// Creates a new manager for the given recordings directory.
    #[must_use]
    pub fn new(recordings_dir: PathBuf) -> Self {
        Self { recordings_dir }
    }

    /// Lists all recordings, sorted by creation date (newest first).
    ///
    /// Scans the recordings directory for `*.data` files, resolves the
    /// corresponding `.timing` and `.meta.json` sidecars, and returns a
    /// sorted vector of [`RecordingEntry`] items.
    ///
    /// # Errors
    ///
    /// Returns `io::Error` if the directory cannot be read.
    pub fn list(&self) -> io::Result<Vec<RecordingEntry>> {
        let read_dir = match fs::read_dir(&self.recordings_dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };

        let mut entries = Vec::new();

        for dir_entry in read_dir {
            let dir_entry = dir_entry?;
            let path = dir_entry.path();

            // Only consider *.data files.
            if path.extension().and_then(|e| e.to_str()) != Some("data") {
                continue;
            }

            let stem = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            let timing_path = path.with_file_name(format!("{stem}.timing"));
            if !timing_path.exists() {
                continue; // Skip data files without a matching timing file.
            }

            let meta_path = metadata_path(&path);

            let metadata = if meta_path.exists() {
                read_metadata(&meta_path).unwrap_or_else(|_| {
                    // Fallback: derive when the sidecar is corrupt.
                    derive_metadata(&path, &timing_path).unwrap_or_else(|_| RecordingMetadata {
                        connection_name: stem.clone(),
                        display_name: None,
                        created_at: chrono::Utc::now(),
                        duration_secs: 0.0,
                        total_size_bytes: 0,
                    })
                })
            } else {
                derive_metadata(&path, &timing_path).unwrap_or_else(|_| RecordingMetadata {
                    connection_name: stem.clone(),
                    display_name: None,
                    created_at: chrono::Utc::now(),
                    duration_secs: 0.0,
                    total_size_bytes: 0,
                })
            };

            entries.push(RecordingEntry {
                data_path: path,
                timing_path,
                meta_path,
                metadata,
            });
        }

        // Sort by created_at descending (newest first).
        entries.sort_by(|a, b| b.metadata.created_at.cmp(&a.metadata.created_at));

        Ok(entries)
    }

    /// Deletes a recording (data + timing + meta files).
    ///
    /// Removes all three files associated with the recording. Missing files
    /// are silently ignored.
    ///
    /// # Errors
    ///
    /// Returns `io::Error` if a file exists but cannot be removed.
    pub fn delete(&self, data_path: &Path) -> io::Result<()> {
        let stem = data_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let timing_path = data_path.with_file_name(format!("{stem}.timing"));
        let meta_path = metadata_path(data_path);

        for path in [data_path, timing_path.as_path(), meta_path.as_path()] {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Renames a recording by updating the `display_name` in its metadata
    /// sidecar.
    ///
    /// If no sidecar exists yet, one is derived from the filename and
    /// filesystem attributes before updating.
    ///
    /// # Errors
    ///
    /// Returns `io::Error` if the metadata cannot be read or written.
    pub fn rename(&self, data_path: &Path, new_name: &str) -> io::Result<()> {
        let meta_path = metadata_path(data_path);

        let stem = data_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let timing_path = data_path.with_file_name(format!("{stem}.timing"));

        let mut meta = if meta_path.exists() {
            read_metadata(&meta_path)?
        } else {
            derive_metadata(data_path, &timing_path)?
        };

        meta.display_name = Some(new_name.to_string());
        write_metadata(&meta_path, &meta)
    }

    /// Validates a timing file against its data file.
    ///
    /// Checks that every line in the timing file is well-formed
    /// (`{f64} {usize}`) and that the sum of all byte counts does not exceed
    /// the data file size.
    ///
    /// # Errors
    ///
    /// Returns `io::Error` if the timing file is malformed or the byte count
    /// sum exceeds the data file size.
    pub fn validate_timing(data_path: &Path, timing_path: &Path) -> io::Result<()> {
        let data_size = fs::metadata(data_path)?.len();

        let file = BufReader::new(File::open(timing_path)?);
        let mut byte_sum: u64 = 0;

        for (line_num, line) in file.lines().enumerate() {
            let line = line?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let mut parts = trimmed.split_whitespace();

            let _delay: f64 = parts.next().and_then(|s| s.parse().ok()).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("bad timing delay on line {}", line_num + 1),
                )
            })?;

            let count: usize = parts.next().and_then(|s| s.parse().ok()).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("bad timing byte count on line {}", line_num + 1),
                )
            })?;

            byte_sum += count as u64;
        }

        if byte_sum > data_size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("byte count sum ({byte_sum}) exceeds data file size ({data_size})"),
            ));
        }

        Ok(())
    }

    /// Imports an external scriptreplay file pair into the recordings
    /// directory.
    ///
    /// Validates the timing file, copies both files with an `imported_` prefix,
    /// appends a numeric suffix when names conflict, and generates a
    /// `.meta.json` sidecar.
    ///
    /// # Errors
    ///
    /// Returns `io::Error` if validation fails, files cannot be copied, or
    /// metadata cannot be written.
    pub fn import(&self, source_data: &Path, source_timing: &Path) -> io::Result<RecordingEntry> {
        // 1. Validate the timing file against the data file.
        Self::validate_timing(source_data, source_timing)?;

        // 2. Determine the base name: imported_{original_stem}
        let original_stem = source_data
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let base = format!("imported_{original_stem}");

        // 3. Resolve conflicts with numeric suffix.
        let (dest_data, dest_timing) = self.resolve_import_paths(&base);

        // 4. Ensure the recordings directory exists.
        fs::create_dir_all(&self.recordings_dir)?;

        // 5. Copy files.
        fs::copy(source_data, &dest_data)?;
        fs::copy(source_timing, &dest_timing)?;

        // 6. Generate metadata sidecar.
        let meta = derive_metadata(&dest_data, &dest_timing)?;
        let meta_path = metadata_path(&dest_data);
        write_metadata(&meta_path, &meta)?;

        Ok(RecordingEntry {
            data_path: dest_data,
            timing_path: dest_timing,
            meta_path,
            metadata: meta,
        })
    }

    /// Resolves destination paths for an import, appending a numeric suffix
    /// (`_1`, `_2`, …) when a file with the same name already exists.
    fn resolve_import_paths(&self, base: &str) -> (PathBuf, PathBuf) {
        let mut candidate_data = self.recordings_dir.join(format!("{base}.data"));
        let mut candidate_timing = self.recordings_dir.join(format!("{base}.timing"));

        if !candidate_data.exists() && !candidate_timing.exists() {
            return (candidate_data, candidate_timing);
        }

        let mut suffix = 1u32;
        loop {
            candidate_data = self.recordings_dir.join(format!("{base}_{suffix}.data"));
            candidate_timing = self.recordings_dir.join(format!("{base}_{suffix}.timing"));

            if !candidate_data.exists() && !candidate_timing.exists() {
                return (candidate_data, candidate_timing);
            }
            suffix += 1;
        }
    }

    /// Exports a recording's data and timing files to a destination directory.
    ///
    /// Copies the `.data` and `.timing` files to `dest_dir`, preserving
    /// original filenames.
    ///
    /// # Errors
    ///
    /// Returns `io::Error` if files cannot be copied.
    pub fn export(&self, data_path: &Path, dest_dir: &Path) -> io::Result<(PathBuf, PathBuf)> {
        let stem = data_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let timing_path = data_path.with_file_name(format!("{stem}.timing"));

        fs::create_dir_all(dest_dir)?;

        let dest_data = dest_dir.join(format!("{stem}.data"));
        let dest_timing = dest_dir.join(format!("{stem}.timing"));

        fs::copy(data_path, &dest_data)?;
        fs::copy(&timing_path, &dest_timing)?;

        Ok((dest_data, dest_timing))
    }
}
