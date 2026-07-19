//! Drag-and-drop file transfer for embedded RDP sessions
//!
//! When files are dragged onto the RDP widget, they are announced to the
//! remote server via the CLIPRDR file clipboard channel (`CF_HDROP` /
//! `FileGroupDescriptorW`). The server can then "paste" the files.
//!
//! Includes a circuit breaker that auto-disables the feature after repeated
//! failures, showing a toast notification to the user.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gtk4::gdk;
use gtk4::prelude::*;

use crate::i18n::{i18n, i18n_f};

/// Metadata for a local file to be sent via RDP clipboard
#[derive(Debug, Clone)]
pub struct LocalFileInfo {
    /// Full local path
    pub path: PathBuf,
    /// File name (without directory)
    pub name: String,
    /// File size in bytes
    pub size: u64,
    /// File attributes (Windows-style, 0x20 = normal file, 0x10 = directory)
    pub attributes: u32,
    /// Last modified time as Windows FILETIME (100ns intervals since 1601-01-01)
    pub last_modified: i64,
}

impl LocalFileInfo {
    /// Windows file attribute: Normal file
    pub const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
    /// Windows file attribute: Archive (set on most files)
    pub const FILE_ATTRIBUTE_ARCHIVE: u32 = 0x20;
    /// Windows file attribute: Directory
    pub const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;

    /// Creates a `LocalFileInfo` from a filesystem path.
    ///
    /// Returns `None` if the file cannot be stat'd.
    pub fn from_path(path: &std::path::Path) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        let name = path.file_name()?.to_str()?.to_string();
        let size = metadata.len();

        let attributes = if metadata.is_dir() {
            Self::FILE_ATTRIBUTE_DIRECTORY
        } else {
            Self::FILE_ATTRIBUTE_ARCHIVE
        };

        // Convert SystemTime to Windows FILETIME
        // FILETIME epoch: 1601-01-01, Unix epoch: 1970-01-01
        // Difference: 11644473600 seconds
        let last_modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| {
                let windows_ticks =
                    (d.as_secs() + 11_644_473_600) * 10_000_000 + u64::from(d.subsec_nanos()) / 100;
                windows_ticks as i64
            })
            .unwrap_or(0);

        Some(Self {
            path: path.to_path_buf(),
            name,
            size,
            attributes,
            last_modified,
        })
    }
}

/// Circuit breaker for RDP file drag-and-drop.
///
/// Automatically disables the feature after `max_failures` consecutive
/// transfer failures to avoid frustrating the user with repeated errors.
#[derive(Debug)]
pub struct FileDndCircuitBreaker {
    /// Number of consecutive failures
    consecutive_failures: u32,
    /// Threshold for auto-disable
    max_failures: u32,
    /// Whether the feature is currently disabled
    disabled: bool,
    /// Reason for the last failure (for toast message)
    last_error: Option<String>,
}

impl FileDndCircuitBreaker {
    /// Creates a new circuit breaker with default threshold (3 failures).
    #[must_use]
    pub fn new() -> Self {
        Self {
            consecutive_failures: 0,
            max_failures: 3,
            disabled: false,
            last_error: None,
        }
    }

    /// Returns `true` if file DnD is currently available.
    #[must_use]
    pub fn is_available(&self) -> bool {
        !self.disabled
    }

    /// Records a successful transfer, resetting the failure counter.
    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
    }

    /// Records a failure. Returns `true` if the circuit breaker tripped
    /// (feature was just disabled).
    pub fn record_failure(&mut self, reason: &str) -> bool {
        self.consecutive_failures += 1;
        self.last_error = Some(reason.to_owned());

        tracing::warn!(
            consecutive_failures = self.consecutive_failures,
            max = self.max_failures,
            reason,
            "RDP file DnD failure recorded"
        );

        if self.consecutive_failures >= self.max_failures {
            self.disabled = true;
            true
        } else {
            false
        }
    }

    /// Disables the feature immediately (e.g., server doesn't support it).
    pub fn disable(&mut self, reason: &str) {
        self.disabled = true;
        self.last_error = Some(reason.to_owned());
        tracing::info!(reason, "RDP file DnD disabled");
    }

    /// Resets the circuit breaker, re-enabling the feature.
    pub fn reset(&mut self) {
        self.disabled = false;
        self.consecutive_failures = 0;
        self.last_error = None;
        tracing::info!("RDP file DnD circuit breaker reset");
    }

    /// Returns the last error message, if any.
    #[must_use]
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
}

impl Default for FileDndCircuitBreaker {
    fn default() -> Self {
        Self::new()
    }
}

/// Sets up a file drop target on the RDP drawing area widget.
///
/// When files are dropped, they are announced to the RDP server via the
/// CLIPRDR channel. The `on_files_dropped` callback receives the list of
/// local file metadata for the caller to send via `RdpClientCommand`.
///
/// The circuit breaker is checked before accepting drops. If tripped,
/// a toast is shown via `toast_overlay` (resolved lazily so it can be set
/// after widget construction).
pub fn setup_rdp_file_drop_target<F>(
    widget: &gtk4::Widget,
    circuit_breaker: Rc<RefCell<FileDndCircuitBreaker>>,
    toast_overlay: Rc<RefCell<Option<libadwaita::ToastOverlay>>>,
    on_files_dropped: F,
) where
    F: Fn(Vec<LocalFileInfo>) + 'static,
{
    let drop_target = gtk4::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);

    let cb_for_enter = circuit_breaker.clone();
    let widget_for_enter = widget.clone();
    drop_target.connect_enter(move |_target, _x, _y| {
        if !cb_for_enter.borrow().is_available() {
            return gdk::DragAction::empty();
        }
        widget_for_enter.add_css_class("terminal-drop-highlight");
        gdk::DragAction::COPY
    });

    let widget_for_leave = widget.clone();
    drop_target.connect_leave(move |_target| {
        widget_for_leave.remove_css_class("terminal-drop-highlight");
    });

    let cb_for_drop = circuit_breaker.clone();
    let widget_for_drop = widget.clone();
    let toast_for_drop = toast_overlay;
    let on_drop = Rc::new(on_files_dropped);

    drop_target.connect_drop(move |_target, value, _x, _y| {
        widget_for_drop.remove_css_class("terminal-drop-highlight");

        // Check circuit breaker
        if !cb_for_drop.borrow().is_available() {
            if let Some(ref overlay) = *toast_for_drop.borrow() {
                let error_msg = cb_for_drop
                    .borrow()
                    .last_error()
                    .unwrap_or("Unknown error")
                    .to_string();

                let toast =
                    libadwaita::Toast::new(&i18n_f("File drag disabled: {}", &[&error_msg]));
                toast.set_timeout(5);

                // "Try Again" button resets the circuit breaker
                toast.set_button_label(Some(&i18n("Try Again")));
                let cb_reset = cb_for_drop.clone();
                toast.connect_button_clicked(move |_| {
                    cb_reset.borrow_mut().reset();
                });

                overlay.add_toast(toast);
            }
            return false;
        }

        // Extract file list
        let file_list = match value.get::<gdk::FileList>() {
            Ok(list) => list,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to get FileList from RDP drop");
                return false;
            }
        };

        let files = file_list.files();
        if files.is_empty() {
            return false;
        }

        // Collect file metadata
        let mut local_files = Vec::with_capacity(files.len());
        for file in &files {
            if let Some(path) = file.path() {
                if let Some(info) = LocalFileInfo::from_path(&path) {
                    local_files.push(info);
                } else {
                    tracing::debug!(
                        path = %path.display(),
                        "Skipping file that cannot be stat'd"
                    );
                }
            }
        }

        if local_files.is_empty() {
            tracing::debug!("No valid files in RDP drop");
            return false;
        }

        tracing::info!(
            file_count = local_files.len(),
            "Files dropped onto RDP session"
        );

        on_drop(local_files);
        true
    });

    widget.add_controller(drop_target);
}

/// Builds a `FileGroupDescriptorW` binary blob for announcing files
/// to the RDP server via CLIPRDR.
///
/// Format: MS-RDPECLIP 2.2.5.2.3.1
/// - `cItems` (4 bytes LE): number of file descriptors
/// - `FILEDESCRIPTORW[cItems]`: 592 bytes each
#[must_use]
pub fn build_file_group_descriptor(files: &[LocalFileInfo]) -> Vec<u8> {
    /// Size of each FILEDESCRIPTORW structure
    const DESCRIPTOR_SIZE: usize = 592;
    /// Offset of cFileName within FILEDESCRIPTORW
    const FILENAME_OFFSET: usize = 76;
    /// Max filename length in UTF-16 chars (including null)
    const MAX_FILENAME_CHARS: usize = 260;

    /// FD_ATTRIBUTES flag
    const FD_ATTRIBUTES: u32 = 0x0004;
    /// FD_WRITESTIME flag
    const FD_WRITESTIME: u32 = 0x0020;
    /// FD_FILESIZE flag
    const FD_FILESIZE: u32 = 0x0040;

    let mut data = Vec::with_capacity(4 + files.len() * DESCRIPTOR_SIZE);

    // cItems
    data.extend_from_slice(&(files.len() as u32).to_le_bytes());

    for file in files {
        let mut descriptor = vec![0u8; DESCRIPTOR_SIZE];

        // dwFlags at offset 0: indicate which fields are valid
        let flags = FD_ATTRIBUTES | FD_WRITESTIME | FD_FILESIZE;
        descriptor[0..4].copy_from_slice(&flags.to_le_bytes());

        // dwFileAttributes at offset 36
        descriptor[36..40].copy_from_slice(&file.attributes.to_le_bytes());

        // ftLastWriteTime at offset 60 (FILETIME, 8 bytes)
        descriptor[60..68].copy_from_slice(&file.last_modified.to_le_bytes());

        // nFileSizeHigh at offset 68, nFileSizeLow at offset 72
        let size_high = (file.size >> 32) as u32;
        let size_low = file.size as u32;
        descriptor[68..72].copy_from_slice(&size_high.to_le_bytes());
        descriptor[72..76].copy_from_slice(&size_low.to_le_bytes());

        // cFileName at offset 76: UTF-16LE, null-terminated, max 260 chars
        let utf16: Vec<u16> = file.name.encode_utf16().collect();
        let chars_to_write = utf16.len().min(MAX_FILENAME_CHARS - 1);
        for (i, &ch) in utf16[..chars_to_write].iter().enumerate() {
            let offset = FILENAME_OFFSET + i * 2;
            descriptor[offset..offset + 2].copy_from_slice(&ch.to_le_bytes());
        }
        // Null terminator is already zero from vec initialization

        data.extend_from_slice(&descriptor);
    }

    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_circuit_breaker_initial_state() {
        let cb = FileDndCircuitBreaker::new();
        assert!(cb.is_available());
        assert_eq!(cb.last_error(), None);
    }

    #[test]
    fn test_circuit_breaker_trips_after_max_failures() {
        let mut cb = FileDndCircuitBreaker::new();
        assert!(!cb.record_failure("error 1"));
        assert!(!cb.record_failure("error 2"));
        assert!(cb.record_failure("error 3")); // trips
        assert!(!cb.is_available());
        assert_eq!(cb.last_error(), Some("error 3"));
    }

    #[test]
    fn test_circuit_breaker_reset() {
        let mut cb = FileDndCircuitBreaker::new();
        cb.record_failure("e1");
        cb.record_failure("e2");
        cb.record_failure("e3");
        assert!(!cb.is_available());

        cb.reset();
        assert!(cb.is_available());
        assert_eq!(cb.last_error(), None);
    }

    #[test]
    fn test_circuit_breaker_success_resets_counter() {
        let mut cb = FileDndCircuitBreaker::new();
        cb.record_failure("e1");
        cb.record_failure("e2");
        cb.record_success();
        // Counter reset, so 3 more failures needed
        assert!(!cb.record_failure("e3"));
        assert!(!cb.record_failure("e4"));
        assert!(cb.record_failure("e5"));
        assert!(!cb.is_available());
    }

    #[test]
    fn test_circuit_breaker_disable() {
        let mut cb = FileDndCircuitBreaker::new();
        cb.disable("Server doesn't support file clipboard");
        assert!(!cb.is_available());
        assert_eq!(
            cb.last_error(),
            Some("Server doesn't support file clipboard")
        );
    }

    #[test]
    fn test_file_group_descriptor_single_file() {
        let file = LocalFileInfo {
            path: PathBuf::from("/tmp/test.txt"),
            name: "test.txt".to_string(),
            size: 1024,
            attributes: LocalFileInfo::FILE_ATTRIBUTE_ARCHIVE,
            last_modified: 132_500_000_000_000_000, // some FILETIME
        };

        let data = build_file_group_descriptor(&[file]);

        // Check cItems = 1
        assert_eq!(&data[0..4], &1u32.to_le_bytes());
        // Total size: 4 + 592 = 596
        assert_eq!(data.len(), 596);

        // Check file size at offset 4+72 = 76 (nFileSizeLow)
        assert_eq!(&data[76..80], &1024u32.to_le_bytes());
    }

    #[test]
    fn test_file_group_descriptor_filename_encoding() {
        let file = LocalFileInfo {
            path: PathBuf::from("/tmp/файл.txt"),
            name: "файл.txt".to_string(),
            size: 0,
            attributes: LocalFileInfo::FILE_ATTRIBUTE_ARCHIVE,
            last_modified: 0,
        };

        let data = build_file_group_descriptor(&[file]);

        // Verify filename is UTF-16LE at offset 4 + 76 = 80
        let filename_start = 4 + 76;
        let expected_utf16: Vec<u16> = "файл.txt".encode_utf16().collect();
        for (i, &ch) in expected_utf16.iter().enumerate() {
            let offset = filename_start + i * 2;
            let actual = u16::from_le_bytes([data[offset], data[offset + 1]]);
            assert_eq!(actual, ch, "Mismatch at UTF-16 char index {i}");
        }
    }
}
