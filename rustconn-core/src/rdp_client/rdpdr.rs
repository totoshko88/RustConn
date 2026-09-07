//! RDPDR (Device Redirection) backend for shared folders
//!
//! This module implements the `RdpdrBackend` trait from `ironrdp-rdpdr` to provide
//! shared folder functionality for RDP sessions.
//!
//! # Directory Change Notifications
//!
//! The module supports real-time directory change notifications using the `notify` crate
//! (inotify on Linux). When Windows Explorer or other applications request to be notified
//! of directory changes, this module sets up file system watches and sends notifications
//! when files are created, modified, deleted, or renamed.

use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use ironrdp::core::impl_as_any;
use ironrdp::pdu::PduResult;
use ironrdp::rdpdr::RdpdrBackend;
use ironrdp::rdpdr::pdu::RdpdrPdu;
use ironrdp::rdpdr::pdu::efs::{
    Boolean, ClientDriveQueryDirectoryResponse, ClientDriveQueryInformationResponse,
    ClientDriveQueryVolumeInformationResponse, ClientDriveSetInformationResponse,
    CreateDisposition, CreateOptions, DesiredAccess, DeviceCloseRequest, DeviceCloseResponse,
    DeviceControlRequest, DeviceControlResponse, DeviceCreateRequest, DeviceCreateResponse,
    DeviceIoResponse, DeviceReadRequest, DeviceReadResponse, DeviceWriteRequest,
    DeviceWriteResponse, FileAttributeTagInformation, FileAttributes, FileBasicInformation,
    FileBothDirectoryInformation, FileDirectoryInformation, FileFsAttributeInformation,
    FileFsFullSizeInformation, FileFsSizeInformation, FileFsVolumeInformation,
    FileFullDirectoryInformation, FileInformationClass, FileInformationClassLevel,
    FileNamesInformation, FileRenameInformation, FileStandardInformation, FileSystemAttributes,
    FileSystemInformationClass, FileSystemInformationClassLevel, Information, NtStatus,
    PrinterIoRequest, ServerDeviceAnnounceResponse, ServerDriveIoRequest,
    ServerDriveLockControlRequest, ServerDriveNotifyChangeDirectoryRequest,
    ServerDriveQueryDirectoryRequest, ServerDriveQueryInformationRequest,
    ServerDriveQueryVolumeInformationRequest, ServerDriveSetInformationRequest,
};
use ironrdp::rdpdr::pdu::esc::{ScardCall, ScardIoCtlCode};
use ironrdp::svc::SvcMessage;
use tracing::{debug, trace, warn};

use super::dir_watcher::{DirectoryChange, DirectoryWatcher, WatchRequest};

/// A cached directory entry returned one PDU at a time.
#[derive(Debug, Clone)]
struct CachedDirectoryEntry {
    path: PathBuf,
    file_name: String,
}

/// RDPDR backend for Linux/Unix shared folders
#[derive(Debug)]
pub struct RustConnRdpdrBackend {
    /// Map of device IDs to their base paths (supports multiple shared folders)
    drive_paths: HashMap<u32, String>,
    /// Fallback base path (first shared folder, used when device_id is unknown)
    default_base_path: String,
    /// Next file ID to assign
    next_file_id: u32,
    /// Map of file IDs to open file handles
    file_handles: HashMap<u32, File>,
    /// Map of file IDs to their paths
    file_paths: HashMap<u32, String>,
    /// Map of file IDs to the device_id they belong to
    file_device_map: HashMap<u32, u32>,
    /// Map of file IDs to pending directory entries
    dir_entries: HashMap<u32, Vec<CachedDirectoryEntry>>,
    /// Map of file IDs to pending directory change notifications
    pending_notifications: HashMap<u32, PendingNotification>,
    /// File IDs the server marked for delete-on-close via
    /// `FileDispositionInformation`; removed from disk in `handle_close`.
    delete_pending: HashSet<u32>,
    /// Accumulated print-job bytes keyed by printer file ID (MS-RDPEPC).
    /// The server streams a PostScript document via Write IRPs; we buffer it
    /// and hand it to CUPS on Close.
    print_jobs: HashMap<u32, Vec<u8>>,
    /// Map of printer device IDs to their local CUPS queue names.
    /// Used to route each print job back to the correct local queue on Close.
    printer_queues: HashMap<u32, String>,
    /// Directory watcher for change notifications
    dir_watcher: Option<DirectoryWatcher>,
}

/// Pending directory change notification
#[derive(Debug, Clone)]
#[expect(dead_code, reason = "Fields read via Debug in trace! logging")]
struct PendingNotification {
    /// Device IO request header
    device_io_request: ironrdp::rdpdr::pdu::efs::DeviceIoRequest,
    /// Watch tree (recursive)
    watch_tree: bool,
    /// Completion filter
    completion_filter: u32,
}

impl_as_any!(RustConnRdpdrBackend);

impl RustConnRdpdrBackend {
    /// Creates a new RDPDR backend with drive paths and printer queues mapped
    /// by device ID.
    #[must_use]
    pub fn new(drive_paths: HashMap<u32, String>, printer_queues: HashMap<u32, String>) -> Self {
        // Ensure all paths end with /
        let drive_paths: HashMap<u32, String> = drive_paths
            .into_iter()
            .map(|(id, p)| {
                let p = if p.ends_with('/') { p } else { format!("{p}/") };
                (id, p)
            })
            .collect();

        let default_base_path = drive_paths
            .values()
            .next()
            .cloned()
            .unwrap_or_else(|| "/tmp/".to_string());

        // Try to create directory watcher
        let dir_watcher = match DirectoryWatcher::new() {
            Ok(watcher) => {
                debug!("Directory watcher initialized for RDPDR");
                Some(watcher)
            }
            Err(e) => {
                warn!(
                    "Failed to initialize directory watcher: {}. Directory change notifications will be disabled.",
                    e
                );
                None
            }
        };

        Self {
            drive_paths,
            default_base_path,
            next_file_id: 1,
            file_handles: HashMap::new(),
            file_paths: HashMap::new(),
            file_device_map: HashMap::new(),
            dir_entries: HashMap::new(),
            pending_notifications: HashMap::new(),
            delete_pending: HashSet::new(),
            print_jobs: HashMap::new(),
            printer_queues,
            dir_watcher,
        }
    }

    /// Allocates a new file ID
    const fn alloc_file_id(&mut self) -> u32 {
        let id = self.next_file_id;
        self.next_file_id = self.next_file_id.wrapping_add(1);
        id
    }

    /// Returns the base path for a given device ID, falling back to the default
    fn base_path_for_device(&self, device_id: u32) -> &str {
        self.drive_paths
            .get(&device_id)
            .map_or(self.default_base_path.as_str(), String::as_str)
    }

    /// Resolves a server-supplied Windows path beneath its redirected drive.
    ///
    /// Parent traversal and symlink components are rejected before any
    /// filesystem operation. RDP servers are remote peers, so their paths must
    /// never be trusted to stay inside the user-selected share on their own.
    fn resolve_share_path(&self, device_id: u32, windows_path: &str) -> Result<PathBuf, NtStatus> {
        let base = std::fs::canonicalize(self.base_path_for_device(device_id)).map_err(|e| {
            warn!(device_id, error = %e, "RDPDR share root cannot be resolved");
            io_error_to_status(&e)
        })?;

        // A leading slash denotes the redirected-drive root in RDPDR, not the
        // host filesystem root. Everything after it still has to be relative.
        let unix_path = windows_path.replace('\\', "/");
        let mut relative = PathBuf::new();
        for component in Path::new(unix_path.trim_start_matches('/')).components() {
            match component {
                Component::Normal(name) => relative.push(name),
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    warn!(device_id, path = %windows_path, "RDPDR path escaped the share root");
                    return Err(NtStatus::ACCESS_DENIED);
                }
            }
        }

        // Reject every existing symlink component. Besides blocking direct
        // escapes, this prevents a symlinked parent from redirecting a create,
        // rename or delete outside the capability represented by `base`.
        let mut current = base.clone();
        for component in relative.components() {
            let Component::Normal(name) = component else {
                continue;
            };
            current.push(name);
            match std::fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    warn!(device_id, path = %windows_path, "RDPDR path contains a symlink");
                    return Err(NtStatus::ACCESS_DENIED);
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                Err(e) => return Err(io_error_to_status(&e)),
            }
        }

        Ok(base.join(relative))
    }

    /// Polls the directory watcher for pending change notifications
    ///
    /// This should be called periodically to check for file system changes
    /// and generate the appropriate RDP responses.
    ///
    /// # Current Limitations
    ///
    /// `ironrdp-rdpdr` 0.7 (the version paired with ironrdp 0.17) still does not
    /// expose a `ClientDriveNotifyChangeDirectoryResponse` type — it has the
    /// server-side `ServerDriveNotifyChangeDirectoryRequest` and
    /// `ClientDriveQueryDirectoryResponse`, but not the notify *response*. So the
    /// inotify integration detects changes correctly, but the responses cannot be
    /// sent until upstream adds this PDU. This is an external blocker, not
    /// unfinished local work.
    ///
    /// Per MS-RDPEFS 2.2.3.4.11, the response should contain:
    /// - `DeviceIoResponse` header with the original request's `DeviceIoRequest`
    /// - Buffer containing `FILE_NOTIFY_INFORMATION` structures (MS-FSCC 2.4.42)
    ///
    /// When `ironrdp-rdpdr` adds `ClientDriveNotifyChangeDirectoryResponse`,
    /// update this method to construct and return the proper response PDUs.
    pub fn poll_directory_changes(&mut self) -> Vec<SvcMessage> {
        let Some(watcher) = &self.dir_watcher else {
            return Vec::new();
        };

        let changes = watcher.recv_all();
        let mut responded_file_ids = Vec::new();

        for change in changes {
            if let Some(notification) = self.pending_notifications.get(&change.file_id) {
                debug!(
                    "Directory change detected: file_id={}, action={:?}, file={}",
                    change.file_id, change.action, change.file_name
                );

                // TODO: Send actual response when ironrdp adds ClientDriveNotifyChangeDirectoryResponse.
                // The response format per MS-RDPEFS 2.2.3.4.11:
                // - DeviceIoResponse with original DeviceIoRequest and NtStatus::SUCCESS
                // - Buffer containing FILE_NOTIFY_INFORMATION structures, built by
                //   `build_file_notify_info` (kept ready below).
                //
                // Example (when available):
                // responses.push(SvcMessage::from(RdpdrPdu::ClientDriveNotifyChangeDirectoryResponse(
                //     ClientDriveNotifyChangeDirectoryResponse {
                //         device_io_response: DeviceIoResponse::new(
                //             notification.device_io_request.clone(),
                //             NtStatus::SUCCESS,
                //         ),
                //         buffer: Some(build_file_notify_info(&change)),
                //     },
                // )));

                trace!(
                    "Directory change notification ready (awaiting ironrdp support): \
                     file_id={}, action={:?}, device_id={}, completion_id={}",
                    change.file_id,
                    change.action,
                    notification.device_io_request.device_id,
                    notification.device_io_request.completion_id,
                );

                // Mark for removal (one-shot notification per MS-RDPEFS)
                responded_file_ids.push(change.file_id);
            }
        }

        // Remove processed notifications
        for file_id in responded_file_ids {
            self.pending_notifications.remove(&file_id);
            // Also remove the watch since it's one-shot
            if let Some(watcher) = &mut self.dir_watcher {
                watcher.remove_watch(file_id);
            }
        }

        // Return empty until ironrdp adds ClientDriveNotifyChangeDirectoryResponse
        Vec::new()
    }
}

impl RdpdrBackend for RustConnRdpdrBackend {
    fn handle_server_device_announce_response(
        &mut self,
        pdu: ServerDeviceAnnounceResponse,
    ) -> PduResult<()> {
        tracing::debug!("RDPDR device announce response: {:?}", pdu);
        Ok(())
    }

    fn handle_scard_call(
        &mut self,
        _req: DeviceControlRequest<ScardIoCtlCode>,
        _call: ScardCall,
    ) -> PduResult<()> {
        // Smart card not supported
        Ok(())
    }

    fn handle_printer_io_request(&mut self, req: PrinterIoRequest) -> PduResult<Vec<SvcMessage>> {
        match req {
            PrinterIoRequest::Create(create_req) => {
                // Open a fresh spool buffer for this print job.
                let file_id = self.alloc_file_id();
                self.print_jobs.insert(file_id, Vec::new());
                tracing::debug!("RDPDR printer: job opened (file_id={})", file_id);
                Ok(vec![SvcMessage::from(RdpdrPdu::DeviceCreateResponse(
                    DeviceCreateResponse {
                        device_io_reply: DeviceIoResponse::new(
                            create_req.device_io_request,
                            NtStatus::SUCCESS,
                        ),
                        file_id,
                        information: Information::FILE_OPENED,
                    },
                ))])
            }
            PrinterIoRequest::Write(write_req) => {
                // Append streamed PostScript bytes; echo the length back.
                let file_id = write_req.device_io_request.file_id;
                let length = write_req.write_data.len() as u32;
                if let Some(buf) = self.print_jobs.get_mut(&file_id) {
                    buf.extend_from_slice(&write_req.write_data);
                }
                Ok(vec![SvcMessage::from(RdpdrPdu::DeviceWriteResponse(
                    DeviceWriteResponse {
                        device_io_reply: DeviceIoResponse::new(
                            write_req.device_io_request,
                            NtStatus::SUCCESS,
                        ),
                        length,
                    },
                ))])
            }
            PrinterIoRequest::Close(close_req) => {
                // Job finished — hand the accumulated document to the matching
                // local CUPS queue (falling back to the default when unknown).
                let file_id = close_req.device_io_request.file_id;
                let device_id = close_req.device_io_request.device_id;
                let queue = self.printer_queues.get(&device_id).cloned();
                if let Some(job) = self.print_jobs.remove(&file_id)
                    && !job.is_empty()
                {
                    // Spool off the session thread: `lp` copies the document
                    // into the CUPS queue and can block on large jobs, and the
                    // active session runs on a single-threaded runtime — doing
                    // it inline would stall framebuffer updates and input until
                    // `lp` returns. Best-effort, so the handle is detached.
                    std::thread::spawn(move || spool_to_cups(&job, queue.as_deref()));
                }
                Ok(vec![SvcMessage::from(RdpdrPdu::DeviceCloseResponse(
                    DeviceCloseResponse {
                        device_io_response: DeviceIoResponse::new(
                            close_req.device_io_request,
                            NtStatus::SUCCESS,
                        ),
                    },
                ))])
            }
        }
    }

    fn handle_drive_io_request(&mut self, req: ServerDriveIoRequest) -> PduResult<Vec<SvcMessage>> {
        tracing::trace!("RDPDR drive IO request: {:?}", req);
        match req {
            ServerDriveIoRequest::ServerCreateDriveRequest(create_req) => {
                self.handle_create(create_req)
            }
            ServerDriveIoRequest::DeviceCloseRequest(close_req) => self.handle_close(close_req),
            ServerDriveIoRequest::DeviceReadRequest(read_req) => self.handle_read(read_req),
            ServerDriveIoRequest::DeviceWriteRequest(write_req) => self.handle_write(write_req),
            ServerDriveIoRequest::ServerDriveQueryInformationRequest(query_req) => {
                self.handle_query_info(query_req)
            }
            ServerDriveIoRequest::ServerDriveQueryVolumeInformationRequest(vol_req) => {
                self.handle_query_volume(vol_req)
            }
            ServerDriveIoRequest::ServerDriveQueryDirectoryRequest(dir_req) => {
                self.handle_query_directory(dir_req)
            }
            ServerDriveIoRequest::ServerDriveSetInformationRequest(set_req) => {
                self.handle_set_info(&set_req)
            }
            ServerDriveIoRequest::DeviceControlRequest(ctrl_req) => {
                // Return success for device control requests
                Ok(vec![SvcMessage::from(RdpdrPdu::DeviceControlResponse(
                    DeviceControlResponse {
                        device_io_reply: DeviceIoResponse::new(ctrl_req.header, NtStatus::SUCCESS),
                        output_buffer: None,
                    },
                ))])
            }
            ServerDriveIoRequest::ServerDriveNotifyChangeDirectoryRequest(notify_req) => {
                self.handle_notify_change_directory(notify_req)
            }
            ServerDriveIoRequest::ServerDriveLockControlRequest(lock_req) => {
                self.handle_lock_control(lock_req)
            }
        }
    }
}

impl RustConnRdpdrBackend {
    #[expect(
        clippy::too_many_lines,
        reason = "long match/dispatch over many enum variants; splitting per variant only relocates the boilerplate"
    )]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "function returns Result for trait/API uniformity even though this branch never fails"
    )]
    fn handle_create(&mut self, req: DeviceCreateRequest) -> PduResult<Vec<SvcMessage>> {
        let file_id = self.alloc_file_id();
        let device_id = req.device_io_request.device_id;
        let path = match self.resolve_share_path(device_id, &req.path) {
            Ok(path) => path.to_string_lossy().into_owned(),
            Err(status) => {
                return Ok(vec![SvcMessage::from(RdpdrPdu::DeviceCreateResponse(
                    DeviceCreateResponse {
                        device_io_reply: DeviceIoResponse::new(req.device_io_request, status),
                        file_id,
                        information: Information::empty(),
                    },
                ))]);
            }
        };
        tracing::trace!(
            "RDPDR create: file_id={}, device_id={}, path='{}', disposition={:?}",
            file_id,
            device_id,
            path,
            req.create_disposition
        );

        // Check if it's a directory request
        let is_dir_request =
            req.create_options.bits() & CreateOptions::FILE_DIRECTORY_FILE.bits() != 0;

        // Check existing file/directory
        let metadata = std::fs::metadata(&path);

        if is_dir_request {
            match &metadata {
                Ok(m) if m.is_dir() => {
                    // Directory exists, open it
                    if let Ok(file) = OpenOptions::new().read(true).open(&path) {
                        self.file_handles.insert(file_id, file);
                        self.file_paths.insert(file_id, path);
                        self.file_device_map.insert(file_id, device_id);
                        return Ok(vec![SvcMessage::from(RdpdrPdu::DeviceCreateResponse(
                            DeviceCreateResponse {
                                device_io_reply: DeviceIoResponse::new(
                                    req.device_io_request,
                                    NtStatus::SUCCESS,
                                ),
                                file_id,
                                information: Information::FILE_OPENED,
                            },
                        ))]);
                    }
                }
                Ok(_) => {
                    // Path exists but is not a directory
                    return Ok(vec![SvcMessage::from(RdpdrPdu::DeviceCreateResponse(
                        DeviceCreateResponse {
                            device_io_reply: DeviceIoResponse::new(
                                req.device_io_request,
                                NtStatus::NOT_A_DIRECTORY,
                            ),
                            file_id,
                            information: Information::empty(),
                        },
                    ))]);
                }
                Err(_) => {
                    // Directory doesn't exist, try to create if requested
                    if (req.create_disposition == CreateDisposition::FILE_CREATE
                        || req.create_disposition == CreateDisposition::FILE_OPEN_IF)
                        && std::fs::create_dir_all(&path).is_ok()
                        && let Ok(file) = OpenOptions::new().read(true).open(&path)
                    {
                        self.file_handles.insert(file_id, file);
                        self.file_paths.insert(file_id, path);
                        self.file_device_map.insert(file_id, device_id);
                        return Ok(vec![SvcMessage::from(RdpdrPdu::DeviceCreateResponse(
                            DeviceCreateResponse {
                                device_io_reply: DeviceIoResponse::new(
                                    req.device_io_request,
                                    NtStatus::SUCCESS,
                                ),
                                file_id,
                                information: Information::FILE_SUPERSEDED,
                            },
                        ))]);
                    }
                }
            }
        }

        // Windows opens an existing file with FILE_OPEN even when it intends to
        // write to it — `desired_access` is the only signal. Opening read-only
        // there made every later Write or truncate on that handle fail
        // (issue #256), so honour the requested access.
        let wants_write = req.desired_access.intersects(
            DesiredAccess::FILE_WRITE_DATA_OR_FILE_ADD_FILE
                | DesiredAccess::FILE_APPEND_DATA_OR_FILE_ADD_SUBDIRECTORY
                | DesiredAccess::FILE_WRITE_EA
                | DesiredAccess::FILE_WRITE_ATTRIBUTES,
        );

        // Handle file creation/opening
        let mut opts = OpenOptions::new();
        match req.create_disposition {
            CreateDisposition::FILE_OPEN => {
                opts.read(true).write(wants_write);
            }
            CreateDisposition::FILE_CREATE => {
                opts.read(true).write(true).create_new(true);
            }
            CreateDisposition::FILE_OPEN_IF => {
                opts.read(true).write(true).create(true);
            }
            CreateDisposition::FILE_OVERWRITE => {
                opts.read(true).write(true).truncate(true);
            }
            CreateDisposition::FILE_OVERWRITE_IF => {
                opts.read(true).write(true).truncate(true).create(true);
            }
            CreateDisposition::FILE_SUPERSEDE => {
                opts.read(true).write(true).create(true).append(true);
            }
            _ => {
                opts.read(true);
            }
        }

        // A read-only file on disk, or a directory, still has to be openable for
        // the metadata queries Explorer runs before it touches anything.
        let opened = opts.open(&path).or_else(|e| {
            if wants_write {
                trace!(
                    "RDPDR open: write-mode open failed for '{}', retrying read-only \
                     (file may be read-only on disk): {}",
                    path, e
                );
                OpenOptions::new().read(true).open(&path)
            } else {
                Err(e)
            }
        });

        match opened {
            Ok(file) => {
                // Copies into a share are otherwise invisible below TRACE: the
                // write and timestamp paths log nothing on success, which made
                // issue #256 far harder to narrow down than it needed to be.
                // One line per content-changing open keeps DEBUG readable.
                if wants_write || req.create_disposition != CreateDisposition::FILE_OPEN {
                    debug!(
                        "RDPDR open for write: '{path}' (disposition={:?})",
                        req.create_disposition
                    );
                }
                self.file_handles.insert(file_id, file);
                self.file_paths.insert(file_id, path);
                self.file_device_map.insert(file_id, device_id);
                let info = match req.create_disposition {
                    CreateDisposition::FILE_CREATE => Information::FILE_SUPERSEDED,
                    CreateDisposition::FILE_OVERWRITE | CreateDisposition::FILE_OVERWRITE_IF => {
                        Information::FILE_OVERWRITTEN
                    }
                    _ => Information::FILE_OPENED,
                };
                Ok(vec![SvcMessage::from(RdpdrPdu::DeviceCreateResponse(
                    DeviceCreateResponse {
                        device_io_reply: DeviceIoResponse::new(
                            req.device_io_request,
                            NtStatus::SUCCESS,
                        ),
                        file_id,
                        information: info,
                    },
                ))])
            }
            Err(e) => {
                // A missing path is ordinary control flow, not a fault: Explorer
                // probes candidate names to pick "New folder (2)", and a stale
                // listing keeps resolving names that were already renamed away.
                // Only a real failure (permissions, not-a-directory, …) is worth
                // a warning.
                if e.kind() == std::io::ErrorKind::NotFound {
                    trace!("RDPDR create: '{path}' does not exist");
                } else {
                    warn!("Failed to open file {}: {}", path, e);
                }
                Ok(vec![SvcMessage::from(RdpdrPdu::DeviceCreateResponse(
                    DeviceCreateResponse {
                        device_io_reply: DeviceIoResponse::new(
                            req.device_io_request,
                            io_error_to_status(&e),
                        ),
                        file_id,
                        information: Information::empty(),
                    },
                ))])
            }
        }
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "function returns Result for trait/API uniformity even though this branch never fails"
    )]
    fn handle_close(&mut self, req: DeviceCloseRequest) -> PduResult<Vec<SvcMessage>> {
        let file_id = req.device_io_request.file_id;
        self.file_handles.remove(&file_id);
        let closed_path = self.file_paths.remove(&file_id);
        self.file_device_map.remove(&file_id);

        // Delete-on-close, requested earlier via FileDispositionInformation.
        // The handle is dropped above so the unlink sees no open descriptor of
        // ours; on Unix an unlink would succeed regardless, but ordering keeps
        // the directory watcher from reporting a phantom modification.
        if self.delete_pending.remove(&file_id)
            && let Some(path) = closed_path.as_deref()
        {
            let removed = if std::fs::metadata(path).is_ok_and(|m| m.is_dir()) {
                std::fs::remove_dir(path)
            } else {
                std::fs::remove_file(path)
            };
            match removed {
                Ok(()) => debug!("RDPDR delete-on-close removed '{path}'"),
                Err(e) => warn!("RDPDR delete-on-close failed for '{path}': {e}"),
            }
        }

        self.dir_entries.remove(&file_id);
        self.pending_notifications.remove(&file_id);

        // Remove directory watch if exists
        if let Some(watcher) = &mut self.dir_watcher {
            watcher.remove_watch(file_id);
        }

        Ok(vec![SvcMessage::from(RdpdrPdu::DeviceCloseResponse(
            DeviceCloseResponse {
                device_io_response: DeviceIoResponse::new(req.device_io_request, NtStatus::SUCCESS),
            },
        ))])
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "function returns Result for trait/API uniformity even though this branch never fails"
    )]
    fn handle_read(&mut self, req: DeviceReadRequest) -> PduResult<Vec<SvcMessage>> {
        let file_id = req.device_io_request.file_id;
        if let Some(file) = self.file_handles.get_mut(&file_id)
            && file.seek(SeekFrom::Start(req.offset)).is_ok()
        {
            let mut buf = vec![0u8; req.length as usize];
            match file.read(&mut buf) {
                Ok(n) => {
                    buf.truncate(n);
                    return Ok(vec![SvcMessage::from(RdpdrPdu::DeviceReadResponse(
                        DeviceReadResponse {
                            device_io_reply: DeviceIoResponse::new(
                                req.device_io_request,
                                NtStatus::SUCCESS,
                            ),
                            read_data: buf,
                        },
                    ))]);
                }
                Err(e) => {
                    warn!("Read error: {}", e);
                }
            }
        }
        Ok(vec![SvcMessage::from(RdpdrPdu::DeviceReadResponse(
            DeviceReadResponse {
                device_io_reply: DeviceIoResponse::new(
                    req.device_io_request,
                    NtStatus::NO_SUCH_FILE,
                ),
                read_data: Vec::new(),
            },
        ))])
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "function returns Result for trait/API uniformity even though this branch never fails"
    )]
    fn handle_write(&mut self, req: DeviceWriteRequest) -> PduResult<Vec<SvcMessage>> {
        let file_id = req.device_io_request.file_id;
        if let Some(file) = self.file_handles.get_mut(&file_id)
            && file.seek(SeekFrom::Start(req.offset)).is_ok()
        {
            match file.write(&req.write_data) {
                Ok(n) => {
                    let _ = file.flush();
                    return Ok(vec![SvcMessage::from(RdpdrPdu::DeviceWriteResponse(
                        DeviceWriteResponse {
                            device_io_reply: DeviceIoResponse::new(
                                req.device_io_request,
                                NtStatus::SUCCESS,
                            ),
                            length: n as u32,
                        },
                    ))]);
                }
                Err(e) => {
                    warn!("Write error: {}", e);
                }
            }
        }
        Ok(vec![SvcMessage::from(RdpdrPdu::DeviceWriteResponse(
            DeviceWriteResponse {
                device_io_reply: DeviceIoResponse::new(
                    req.device_io_request,
                    NtStatus::UNSUCCESSFUL,
                ),
                length: 0,
            },
        ))])
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "function returns Result for trait/API uniformity even though this branch never fails"
    )]
    #[expect(
        clippy::needless_pass_by_ref_mut,
        reason = "&mut required by the trait/upstream contract even when this branch does not mutate"
    )]
    fn handle_query_info(
        &mut self,
        req: ServerDriveQueryInformationRequest,
    ) -> PduResult<Vec<SvcMessage>> {
        let file_id = req.device_io_request.file_id;
        let Some(file) = self.file_handles.get(&file_id) else {
            return Ok(vec![SvcMessage::from(
                RdpdrPdu::ClientDriveQueryInformationResponse(
                    ClientDriveQueryInformationResponse {
                        device_io_response: DeviceIoResponse::new(
                            req.device_io_request,
                            NtStatus::NO_SUCH_FILE,
                        ),
                        buffer: None,
                    },
                ),
            )]);
        };

        let Ok(meta) = file.metadata() else {
            return Ok(vec![SvcMessage::from(
                RdpdrPdu::ClientDriveQueryInformationResponse(
                    ClientDriveQueryInformationResponse {
                        device_io_response: DeviceIoResponse::new(
                            req.device_io_request,
                            NtStatus::UNSUCCESSFUL,
                        ),
                        buffer: None,
                    },
                ),
            )]);
        };

        let path = self.file_paths.get(&file_id).cloned().unwrap_or_default();
        let file_name = PathBuf::from(&path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let file_attrs = get_file_attributes(&meta, &file_name);

        #[expect(
            clippy::cast_possible_wrap,
            reason = "value range fits the target signed type by construction in this code path"
        )]
        let buffer = match req.file_info_class_lvl {
            FileInformationClassLevel::FILE_BASIC_INFORMATION => {
                Some(FileInformationClass::Basic(FileBasicInformation {
                    creation_time: unix_to_filetime(meta.ctime()),
                    last_access_time: unix_to_filetime(meta.atime()),
                    last_write_time: unix_to_filetime(meta.mtime()),
                    change_time: unix_to_filetime(meta.ctime()),
                    file_attributes: file_attrs,
                }))
            }
            FileInformationClassLevel::FILE_STANDARD_INFORMATION => {
                Some(FileInformationClass::Standard(FileStandardInformation {
                    allocation_size: meta.size() as i64,
                    end_of_file: meta.size() as i64,
                    number_of_links: meta.nlink() as u32,
                    delete_pending: Boolean::False,
                    directory: if meta.is_dir() {
                        Boolean::True
                    } else {
                        Boolean::False
                    },
                }))
            }
            // Windows Explorer queries this on the source handle of every
            // rename/delete/copy to detect reparse points before touching the
            // file (issue #256). Answering SUCCESS with an empty buffer — as
            // the `_` arm below used to — makes the redirector fail the IRP
            // with STATUS_IO_DEVICE_ERROR, which Explorer reports as
            // 0x8007045D and the whole operation aborts. We never expose
            // reparse points, so the tag is always 0.
            FileInformationClassLevel::FILE_ATTRIBUTE_TAG_INFORMATION => Some(
                FileInformationClass::AttributeTag(FileAttributeTagInformation {
                    file_attributes: file_attrs,
                    reparse_tag: 0,
                }),
            ),
            _ => None,
        };

        // A zero-length buffer alongside STATUS_SUCCESS is malformed: the
        // server sized the IRP for the requested class and gets nothing back.
        // Report the class as unsupported instead so Windows can fall back.
        let Some(buffer) = buffer else {
            warn!(
                "Unsupported RDPDR query information class: {}",
                req.file_info_class_lvl
            );
            return Ok(vec![SvcMessage::from(
                RdpdrPdu::ClientDriveQueryInformationResponse(
                    ClientDriveQueryInformationResponse {
                        device_io_response: DeviceIoResponse::new(
                            req.device_io_request,
                            NtStatus::NOT_SUPPORTED,
                        ),
                        buffer: None,
                    },
                ),
            )]);
        };

        Ok(vec![SvcMessage::from(
            RdpdrPdu::ClientDriveQueryInformationResponse(ClientDriveQueryInformationResponse {
                device_io_response: DeviceIoResponse::new(req.device_io_request, NtStatus::SUCCESS),
                buffer: Some(buffer),
            }),
        )])
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "function returns Result for trait/API uniformity even though this branch never fails"
    )]
    #[expect(
        clippy::needless_pass_by_ref_mut,
        reason = "&mut required by the trait/upstream contract even when this branch does not mutate"
    )]
    fn handle_query_volume(
        &mut self,
        req: ServerDriveQueryVolumeInformationRequest,
    ) -> PduResult<Vec<SvcMessage>> {
        let device_id = req.device_io_request.device_id;
        let base = self.base_path_for_device(device_id).to_owned();
        let buffer = match req.fs_info_class_lvl {
            FileSystemInformationClassLevel::FILE_FS_ATTRIBUTE_INFORMATION => {
                Some(FileSystemInformationClass::FileFsAttributeInformation(
                    FileFsAttributeInformation {
                        file_system_attributes: FileSystemAttributes::FILE_CASE_SENSITIVE_SEARCH
                            | FileSystemAttributes::FILE_CASE_PRESERVED_NAMES
                            | FileSystemAttributes::FILE_UNICODE_ON_DISK,
                        max_component_name_len: 255,
                        file_system_name: "RustConn".to_owned(),
                    },
                ))
            }
            FileSystemInformationClassLevel::FILE_FS_VOLUME_INFORMATION => Some(
                FileSystemInformationClass::FileFsVolumeInformation(FileFsVolumeInformation {
                    volume_creation_time: unix_to_filetime(0),
                    volume_serial_number: 0x1234_5678,
                    supports_objects: Boolean::False,
                    volume_label: "RustConn".to_owned(),
                }),
            ),
            FileSystemInformationClassLevel::FILE_FS_SIZE_INFORMATION => {
                let (total_units, avail_units) = get_disk_stats(&base);
                Some(FileSystemInformationClass::FileFsSizeInformation(
                    FileFsSizeInformation {
                        total_alloc_units: total_units,
                        available_alloc_units: avail_units,
                        sectors_per_alloc_unit: 8,
                        bytes_per_sector: 512,
                    },
                ))
            }
            FileSystemInformationClassLevel::FILE_FS_FULL_SIZE_INFORMATION => {
                let (total_units, avail_units) = get_disk_stats(&base);
                Some(FileSystemInformationClass::FileFsFullSizeInformation(
                    FileFsFullSizeInformation {
                        total_alloc_units: total_units,
                        caller_available_alloc_units: avail_units,
                        actual_available_alloc_units: avail_units,
                        sectors_per_alloc_unit: 8,
                        bytes_per_sector: 512,
                    },
                ))
            }
            _ => None,
        };

        Ok(vec![SvcMessage::from(
            RdpdrPdu::ClientDriveQueryVolumeInformationResponse(
                ClientDriveQueryVolumeInformationResponse {
                    device_io_reply: DeviceIoResponse::new(
                        req.device_io_request,
                        NtStatus::SUCCESS,
                    ),
                    buffer,
                },
            ),
        )])
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "function returns Result for trait/API uniformity even though this branch never fails"
    )]
    fn handle_query_directory(
        &mut self,
        req: ServerDriveQueryDirectoryRequest,
    ) -> PduResult<Vec<SvcMessage>> {
        let file_id = req.device_io_request.file_id;

        if req.initial_query > 0 {
            let Some(path) = self.file_paths.get(&file_id).map(PathBuf::from) else {
                return Ok(vec![SvcMessage::from(
                    RdpdrPdu::ClientDriveQueryDirectoryResponse(
                        ClientDriveQueryDirectoryResponse {
                            device_io_reply: DeviceIoResponse::new(
                                req.device_io_request,
                                NtStatus::NO_SUCH_FILE,
                            ),
                            buffer: None,
                        },
                    ),
                )]);
            };

            let pattern = directory_search_pattern(&req.path);
            let mut entries = Vec::new();

            // Windows directory enumeration includes these pseudo entries.
            // At the share root, `..` deliberately points back to the root so
            // no metadata outside the redirected capability is exposed.
            for pseudo in [".", ".."] {
                if wildcard_match(pattern, pseudo) {
                    let pseudo_path = if pseudo == "." {
                        path.clone()
                    } else {
                        let base = std::fs::canonicalize(
                            self.base_path_for_device(req.device_io_request.device_id),
                        )
                        .unwrap_or_else(|_| path.clone());
                        path.parent()
                            .filter(|parent| parent.starts_with(&base))
                            .map_or_else(|| path.clone(), Path::to_path_buf)
                    };
                    entries.push(CachedDirectoryEntry {
                        path: pseudo_path,
                        file_name: pseudo.to_owned(),
                    });
                }
            }

            if let Ok(directory) = std::fs::read_dir(&path) {
                let mut real_entries = directory
                    .filter_map(std::result::Result::ok)
                    // Opening symlinks is forbidden by `resolve_share_path`, so
                    // do not advertise entries that the server cannot safely use.
                    .filter(|entry| entry.file_type().is_ok_and(|kind| !kind.is_symlink()))
                    .filter_map(|entry| {
                        let file_name = entry.file_name().to_string_lossy().into_owned();
                        wildcard_match(pattern, &file_name).then(|| CachedDirectoryEntry {
                            path: entry.path(),
                            file_name,
                        })
                    })
                    .collect::<Vec<_>>();
                real_entries.sort_by_key(|entry| entry.file_name.to_lowercase());
                entries.extend(real_entries);
            }

            self.dir_entries.insert(file_id, entries);
        }

        let entry = self
            .dir_entries
            .get_mut(&file_id)
            .and_then(|entries| (!entries.is_empty()).then(|| entries.remove(0)));

        if let Some(entry) = entry
            && let Ok(metadata) = std::fs::metadata(&entry.path)
            && let Some(buffer) =
                directory_information(&req.file_info_class_lvl, &metadata, entry.file_name)
        {
            return Ok(vec![SvcMessage::from(
                RdpdrPdu::ClientDriveQueryDirectoryResponse(ClientDriveQueryDirectoryResponse {
                    device_io_reply: DeviceIoResponse::new(
                        req.device_io_request,
                        NtStatus::SUCCESS,
                    ),
                    buffer: Some(buffer),
                }),
            )]);
        }

        let status = if req.initial_query > 0 {
            NtStatus::NO_SUCH_FILE
        } else {
            NtStatus::NO_MORE_FILES
        };

        Ok(vec![SvcMessage::from(
            RdpdrPdu::ClientDriveQueryDirectoryResponse(ClientDriveQueryDirectoryResponse {
                device_io_reply: DeviceIoResponse::new(req.device_io_request, status),
                buffer: None,
            }),
        )])
    }

    /// Applies a Set Information request to the backing filesystem.
    ///
    /// Until 0.19.12 this only acknowledged the request with `STATUS_SUCCESS`
    /// and discarded `set_buffer`, so renaming, truncating or deleting anything
    /// inside a shared folder silently did nothing — Explorer then failed the
    /// operation with "The file or folder does not exist" (issue #256).
    #[expect(
        clippy::unnecessary_wraps,
        reason = "function returns Result for trait/API uniformity even though this branch never fails"
    )]
    fn handle_set_info(
        &mut self,
        req: &ServerDriveSetInformationRequest,
    ) -> PduResult<Vec<SvcMessage>> {
        let file_id = req.device_io_request.file_id;
        let device_id = req.device_io_request.device_id;

        let status = match &req.set_buffer {
            FileInformationClass::Rename(rename) => self.apply_rename(file_id, device_id, rename),
            FileInformationClass::EndOfFile(eof) => self.apply_truncate(file_id, eof.end_of_file),
            FileInformationClass::Allocation(alloc) => {
                self.apply_truncate(file_id, alloc.allocation_size)
            }
            FileInformationClass::Disposition(disposition) => {
                // Delete-on-close: the removal happens in `handle_close`, which
                // is where the server releases its last handle.
                if disposition.delete_pending == 0 {
                    self.delete_pending.remove(&file_id);
                } else {
                    self.delete_pending.insert(file_id);
                }
                NtStatus::SUCCESS
            }
            FileInformationClass::Basic(basic) => self.apply_times(file_id, basic),
            // `ServerDriveSetInformationRequest::decode` rejects every other
            // class, so this is unreachable in practice.
            other => {
                warn!("Unsupported RDPDR set information class: {other}");
                NtStatus::NOT_SUPPORTED
            }
        };

        Ok(vec![SvcMessage::from(
            RdpdrPdu::ClientDriveSetInformationResponse(
                ClientDriveSetInformationResponse::new(req, status).unwrap_or_else(|_| {
                    ClientDriveSetInformationResponse::new(req, NtStatus::UNSUCCESSFUL)
                        .expect("infallible")
                }),
            ),
        )])
    }

    /// Renames the file behind `file_id` to the share-relative path in `rename`.
    ///
    /// `std::fs::rename` always clobbers the destination on Unix, so an
    /// existing target is rejected up front unless the server allowed it.
    fn apply_rename(
        &mut self,
        file_id: u32,
        device_id: u32,
        rename: &FileRenameInformation,
    ) -> NtStatus {
        let Some(old_path) = self.file_paths.get(&file_id).cloned() else {
            warn!("Rename requested for unknown file_id {file_id}");
            return NtStatus::NO_SUCH_FILE;
        };
        let new_path = match self.resolve_share_path(device_id, &rename.file_name) {
            Ok(path) => path.to_string_lossy().into_owned(),
            Err(status) => return status,
        };

        if rename.replace_if_exists == Boolean::False
            && new_path != old_path
            && std::fs::exists(&new_path).unwrap_or(false)
        {
            return NtStatus::OBJECT_NAME_COLLISION;
        }

        match std::fs::rename(&old_path, &new_path) {
            Ok(()) => {
                debug!("RDPDR rename: '{old_path}' -> '{new_path}'");
                // Keep the handle's bookkeeping in sync: the server reuses this
                // file_id for follow-up queries and the eventual Close.
                self.file_paths.insert(file_id, new_path);
                NtStatus::SUCCESS
            }
            Err(e) => {
                warn!("Rename '{old_path}' -> '{new_path}' failed: {e}");
                io_error_to_status(&e)
            }
        }
    }

    /// Resizes the file behind `file_id` (`FileEndOfFileInformation` and
    /// `FileAllocationInformation` both map to `ftruncate`, as in FreeRDP).
    fn apply_truncate(&mut self, file_id: u32, size: i64) -> NtStatus {
        let Ok(size) = u64::try_from(size) else {
            warn!("Refusing negative truncate size {size} for file_id {file_id}");
            return NtStatus::UNSUCCESSFUL;
        };
        let Some(file) = self.file_handles.get_mut(&file_id) else {
            warn!("Truncate requested for unknown file_id {file_id}");
            return NtStatus::NO_SUCH_FILE;
        };

        match file.set_len(size) {
            Ok(()) => NtStatus::SUCCESS,
            Err(e) => {
                warn!("Truncate to {size} failed for file_id {file_id}: {e}");
                io_error_to_status(&e)
            }
        }
    }

    /// Applies the access/write timestamps from `FileBasicInformation`.
    ///
    /// A zero timestamp means "leave unchanged" (MS-FSCC 2.4.7), and the
    /// attribute bits are deliberately ignored — mapping them onto Unix
    /// permissions would surprise the user with mode changes on their files.
    /// Reporting failure here would break copying *into* a share, since
    /// Windows stamps the destination once the data is written.
    #[cfg_attr(
        target_pointer_width = "64",
        expect(
            clippy::useless_conversion,
            reason = "libc::UTIME_OMIT is c_long: i64 on 64-bit targets, i32 on 32-bit ones"
        )
    )]
    fn apply_times(&self, file_id: u32, basic: &FileBasicInformation) -> NtStatus {
        use nix::sys::stat::futimens;
        use nix::sys::time::TimeSpec;

        let Some(file) = self.file_handles.get(&file_id) else {
            warn!("Set basic information requested for unknown file_id {file_id}");
            return NtStatus::NO_SUCH_FILE;
        };

        // `UTIME_OMIT` leaves the corresponding timestamp untouched.
        let omit = TimeSpec::new(0, i64::from(nix::libc::UTIME_OMIT));
        let to_spec =
            |filetime: i64| filetime_to_unix(filetime).map_or(omit, |secs| TimeSpec::new(secs, 0));

        if let Err(e) = futimens(
            file,
            &to_spec(basic.last_access_time),
            &to_spec(basic.last_write_time),
        ) {
            // Non-fatal: the bytes are already correct, only the metadata is
            // stale, and failing the IRP would abort the whole copy.
            warn!("Failed to set timestamps for file_id {file_id}: {e}");
        }

        NtStatus::SUCCESS
    }

    /// Handles directory change notification requests
    ///
    /// The server sends this request to be notified when a directory changes.
    /// We set up an inotify watch on the directory and will respond when changes occur.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "function returns Result for trait/API uniformity even though this branch never fails"
    )]
    #[expect(
        clippy::needless_pass_by_value,
        reason = "value is consumed by trait/API contract; borrowing would force callers to clone before passing"
    )]
    fn handle_notify_change_directory(
        &mut self,
        req: ServerDriveNotifyChangeDirectoryRequest,
    ) -> PduResult<Vec<SvcMessage>> {
        let file_id = req.device_io_request.file_id;

        debug!(
            "Directory change notification request: file_id={}, watch_tree={}, filter={:#x}",
            file_id, req.watch_tree, req.completion_filter
        );

        // Get the path for this file_id
        let Some(p) = self.file_paths.get(&file_id) else {
            warn!(
                "Directory change notification for unknown file_id: {}",
                file_id
            );
            return Ok(Vec::new());
        };
        let path = p.clone();

        // Store the pending notification
        self.pending_notifications.insert(
            file_id,
            PendingNotification {
                device_io_request: req.device_io_request.clone(),
                watch_tree: req.watch_tree != 0,
                completion_filter: req.completion_filter,
            },
        );

        // Set up the directory watch if watcher is available
        if let Some(watcher) = &mut self.dir_watcher {
            let watch_request = WatchRequest {
                file_id,
                path: PathBuf::from(&path),
                watch_tree: req.watch_tree != 0,
                completion_filter: req.completion_filter,
            };

            if let Err(e) = watcher.add_watch(watch_request) {
                warn!("Failed to add directory watch for {}: {}", path, e);
                // Continue anyway - we've stored the pending notification
            } else {
                debug!("Directory watch added for: {}", path);
            }
        }

        // Return empty vec - we don't respond immediately.
        // The response will be sent when a change is detected via poll_directory_changes()
        Ok(Vec::new())
    }

    /// Handles file lock control requests
    ///
    /// Implements byte-range locking for shared folder files.
    /// This is important for applications that use file locking for synchronization.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "function returns Result for trait/API uniformity even though this branch never fails"
    )]
    fn handle_lock_control(
        &self,
        req: ServerDriveLockControlRequest,
    ) -> PduResult<Vec<SvcMessage>> {
        let file_id = req.device_io_request.file_id;

        debug!("Lock control request: file_id={}", file_id);

        // Check if file exists
        if !self.file_handles.contains_key(&file_id) {
            return Ok(vec![SvcMessage::from(RdpdrPdu::DeviceCloseResponse(
                DeviceCloseResponse {
                    device_io_response: DeviceIoResponse::new(
                        req.device_io_request,
                        NtStatus::UNSUCCESSFUL,
                    ),
                },
            ))]);
        }

        // `ServerDriveLockControlRequest` in ironrdp-rdpdr 0.7 still exposes only
        // limited fields, so we acknowledge the lock request with success.
        // A full implementation would parse the lock information from the PDU
        // and maintain lock state, but the current ironrdp API does not expose
        // the lock details directly.
        //
        // For basic compatibility, we just acknowledge success.
        // This allows applications that use advisory locking to work,
        // though actual lock enforcement isn't implemented.

        Ok(vec![SvcMessage::from(RdpdrPdu::DeviceCloseResponse(
            DeviceCloseResponse {
                device_io_response: DeviceIoResponse::new(req.device_io_request, NtStatus::SUCCESS),
            },
        ))])
    }
}

/// Returns the final filename pattern from an RDPDR directory query.
fn directory_search_pattern(request_path: &str) -> &str {
    let pattern = request_path
        .trim_end_matches(['\\', '/'])
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("*");
    if pattern.is_empty() || pattern == "*.*" {
        "*"
    } else {
        pattern
    }
}

/// Matches the `*` and `?` wildcards used by Windows directory enumeration.
fn wildcard_match(pattern: &str, value: &str) -> bool {
    let pattern = pattern.to_lowercase().chars().collect::<Vec<_>>();
    let value = value.to_lowercase().chars().collect::<Vec<_>>();
    let mut previous = vec![false; value.len() + 1];
    previous[0] = true;

    for token in pattern {
        let mut current = vec![false; value.len() + 1];
        if token == '*' {
            current[0] = previous[0];
            for index in 0..value.len() {
                current[index + 1] = previous[index + 1] || current[index];
            }
        } else {
            for (index, character) in value.iter().enumerate() {
                current[index + 1] = previous[index] && (token == '?' || token == *character);
            }
        }
        previous = current;
    }

    previous[value.len()]
}

/// Builds the exact directory information class requested by the server.
fn directory_information(
    level: &FileInformationClassLevel,
    metadata: &std::fs::Metadata,
    file_name: String,
) -> Option<FileInformationClass> {
    let creation_time = unix_to_filetime(metadata.ctime());
    let last_access_time = unix_to_filetime(metadata.atime());
    let last_write_time = unix_to_filetime(metadata.mtime());
    let change_time = unix_to_filetime(metadata.ctime());
    let file_size = i64::try_from(metadata.size()).unwrap_or(i64::MAX);
    let attributes = get_file_attributes(metadata, &file_name);

    if level == &FileInformationClassLevel::FILE_DIRECTORY_INFORMATION {
        Some(FileInformationClass::Directory(
            FileDirectoryInformation::new(
                creation_time,
                last_access_time,
                last_write_time,
                change_time,
                file_size,
                attributes,
                file_name,
            ),
        ))
    } else if level == &FileInformationClassLevel::FILE_FULL_DIRECTORY_INFORMATION {
        Some(FileInformationClass::FullDirectory(
            FileFullDirectoryInformation::new(
                creation_time,
                last_access_time,
                last_write_time,
                change_time,
                file_size,
                attributes,
                file_name,
            ),
        ))
    } else if level == &FileInformationClassLevel::FILE_BOTH_DIRECTORY_INFORMATION {
        Some(FileInformationClass::BothDirectory(
            FileBothDirectoryInformation::new(
                creation_time,
                last_access_time,
                last_write_time,
                change_time,
                file_size,
                attributes,
                file_name,
            ),
        ))
    } else if level == &FileInformationClassLevel::FILE_NAMES_INFORMATION {
        Some(FileInformationClass::Names(FileNamesInformation::new(
            file_name,
        )))
    } else {
        None
    }
}

/// Returns (total_alloc_units, available_alloc_units) for the filesystem containing `path`.
///
/// Uses `nix::sys::statvfs` for safe filesystem queries without `unsafe` code.
/// Values are normalized to 4096-byte allocation units (8 sectors × 512 bytes)
/// to match the `sectors_per_alloc_unit` and `bytes_per_sector` reported to Windows.
/// Falls back to hardcoded defaults if the statvfs call fails.
#[cfg_attr(
    target_pointer_width = "64",
    expect(
        clippy::useless_conversion,
        reason = "statvfs values are u64 on 64-bit targets; conversions preserve 32-bit portability"
    )
)]
fn get_disk_stats(path: &str) -> (i64, i64) {
    const ALLOC_UNIT_BYTES: u64 = 4096; // 8 sectors × 512 bytes

    match nix::sys::statvfs::statvfs(path) {
        Ok(stat) => {
            // nix::sys::statvfs returns platform-dependent integer types:
            // u64 on 64-bit (x86_64, aarch64), u32 on 32-bit targets.
            // u64::from() is needed for 32-bit compatibility but is identity on 64-bit.
            let frag_size = u64::from(stat.fragment_size());
            // Convert from filesystem blocks to 4096-byte allocation units
            let total_bytes = u64::from(stat.blocks()).saturating_mul(frag_size);
            let avail_bytes = u64::from(stat.blocks_available()).saturating_mul(frag_size);

            let total = i64::try_from(total_bytes / ALLOC_UNIT_BYTES).unwrap_or(1_000_000);
            let available = i64::try_from(avail_bytes / ALLOC_UNIT_BYTES).unwrap_or(500_000);
            (total, available)
        }
        Err(e) => {
            warn!("statvfs failed for {path:?}: {e}, using defaults");
            (1_000_000, 500_000)
        }
    }
}

/// Maps a filesystem error onto the closest NTSTATUS.
///
/// Returning a truthful status matters: Explorer surfaces it to the user, and a
/// blanket `STATUS_UNSUCCESSFUL` turns "permission denied" into an unhelpful
/// generic failure.
fn io_error_to_status(e: &std::io::Error) -> NtStatus {
    match e.kind() {
        std::io::ErrorKind::NotFound => NtStatus::NO_SUCH_FILE,
        std::io::ErrorKind::PermissionDenied => NtStatus::ACCESS_DENIED,
        std::io::ErrorKind::AlreadyExists => NtStatus::OBJECT_NAME_COLLISION,
        std::io::ErrorKind::DirectoryNotEmpty => NtStatus::DIRECTORY_NOT_EMPTY,
        std::io::ErrorKind::NotADirectory => NtStatus::NOT_A_DIRECTORY,
        _ => NtStatus::UNSUCCESSFUL,
    }
}

/// Converts a Windows FILETIME back to Unix seconds.
///
/// Returns `None` for the sentinel values MS-FSCC 2.4.7 uses to mean "leave
/// this timestamp alone" (0) and "no change" (-1).
const fn filetime_to_unix(filetime: i64) -> Option<i64> {
    const EPOCH_DIFF: i64 = 116_444_736_000_000_000;
    if filetime <= 0 {
        return None;
    }
    Some((filetime - EPOCH_DIFF) / 10_000_000)
}

/// Converts Unix timestamp (seconds) to Windows FILETIME (100-nanosecond intervals since 1601)
const fn unix_to_filetime(unix_secs: i64) -> i64 {
    // Windows FILETIME epoch is January 1, 1601
    // Unix epoch is January 1, 1970
    // Difference is 11644473600 seconds
    const EPOCH_DIFF: i64 = 116_444_736_000_000_000;
    unix_secs
        .saturating_mul(10_000_000)
        .saturating_add(EPOCH_DIFF)
}

/// Builds `FILE_NOTIFY_INFORMATION` structure for a directory change
///
/// Format (MS-FSCC 2.4.42):
/// - `NextEntryOffset`: u32 (0 for last entry)
/// - Action: u32 (`FILE_ACTION_*`)
/// - `FileNameLength`: u32 (in bytes)
/// - `FileName`: \[u16\] (UTF-16LE, not null-terminated)
#[expect(
    dead_code,
    reason = "FILE_NOTIFY_INFORMATION builder kept ready for when ironrdp adds \
              ClientDriveNotifyChangeDirectoryResponse (MS-RDPEFS 2.2.3.4.11)"
)]
fn build_file_notify_info(change: &DirectoryChange) -> Vec<u8> {
    let file_name_utf16: Vec<u16> = change.file_name.encode_utf16().collect();
    let file_name_bytes = file_name_utf16.len() * 2;

    let mut buffer = Vec::with_capacity(12 + file_name_bytes);

    // NextEntryOffset (0 = last entry)
    buffer.extend_from_slice(&0u32.to_le_bytes());

    // Action
    buffer.extend_from_slice(&(change.action as u32).to_le_bytes());

    // FileNameLength (in bytes)
    buffer.extend_from_slice(&(file_name_bytes as u32).to_le_bytes());

    // FileName (UTF-16LE)
    for ch in file_name_utf16 {
        buffer.extend_from_slice(&ch.to_le_bytes());
    }

    buffer
}

/// Gets Windows file attributes from Unix metadata
fn get_file_attributes(meta: &std::fs::Metadata, file_name: &str) -> FileAttributes {
    let mut attrs = FileAttributes::empty();

    if meta.is_dir() {
        attrs |= FileAttributes::FILE_ATTRIBUTE_DIRECTORY;
    } else {
        attrs |= FileAttributes::FILE_ATTRIBUTE_ARCHIVE;
    }

    // Hidden files (starting with .)
    if file_name.starts_with('.') && file_name.len() > 1 {
        attrs |= FileAttributes::FILE_ATTRIBUTE_HIDDEN;
    }

    // Read-only
    if meta.permissions().readonly() {
        attrs |= FileAttributes::FILE_ATTRIBUTE_READONLY;
    }

    attrs
}

/// Sends an accumulated print job to a local CUPS queue (or the default queue
/// when `queue` is `None`).
///
/// The virtual printer is announced with a PostScript driver, so the buffer
/// holds a PostScript document that `lp` can consume directly. Best-effort:
/// failures are logged but never surfaced to the RDP session.
///
// ponytail: shells out to CUPS `lp`; fine for the common Linux desktop. A
// native IPP client would drop the runtime dependency on cups-client but pulls
// in another crate — not worth it until users ask.
/// How long any single CUPS helper is given before it is killed.
///
/// All three of these run on the RDP connection's own thread — `lpstat` twice
/// while the channel is announced, `lp` whenever the guest prints — and all three
/// waited indefinitely. A CUPS daemon that is stuck, or a queue on an
/// unreachable print server, therefore stalled the session rather than costing it
/// a printer.
///
/// Two seconds because these are local IPC calls that normally answer in tens of
/// milliseconds. Losing the answer means "no printers to forward", which the
/// callers already treat as an ordinary outcome rather than an error.
const CUPS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

fn spool_to_cups(document: &[u8], queue: Option<&str>) {
    let mut cmd = Command::new("lp");
    cmd.args(["-t", "RustConn RDP"]);
    if let Some(q) = queue {
        // Route to the matching local queue. Passed as a single argument so
        // queue names with spaces or odd characters are handled safely.
        cmd.args(["-d", q]);
    }
    let mut child = match cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            warn!("Failed to spawn `lp` for RDP printer redirection: {e}");
            return;
        }
    };

    // Write the document, then drop stdin to close the pipe so `lp` proceeds.
    //
    // This write is the one part of the path [`CUPS_TIMEOUT`] cannot bound, and it
    // is the more likely place to block of the two: a document larger than the
    // pipe buffer — 64 KiB on Linux, and a PostScript page easily exceeds it —
    // blocks in `write_all` until `lp` drains it, so a stuck `lp` stalls here and
    // the budget below never starts counting. Handing the write to a thread and
    // bounding it would mean holding the document across a thread boundary for a
    // best-effort print job; instead the write is what it always was, and the
    // limitation is written down rather than implied by the budget on the next
    // line.
    //
    // ponytail: unbounded write to `lp`'s stdin ahead of a bounded wait. Fine
    // while the local CUPS scheduler drains its pipe, which is its normal
    // behaviour; move the write onto a thread with its own deadline if a stuck
    // spooler is ever reported to hang a session.
    if let Some(mut stdin) = child.stdin.take()
        && let Err(e) = stdin.write_all(document)
    {
        warn!("Failed to write print job to `lp`: {e}");
    }

    match crate::proc::wait_bounded(child, CUPS_TIMEOUT, "lp (RDP print job)") {
        Ok(waited) if waited.succeeded() => {
            debug!(
                "RDP print job sent to CUPS queue {:?} ({} bytes)",
                queue.unwrap_or("<default>"),
                document.len()
            );
        }
        Ok(crate::proc::Waited::TimedOut) => {
            warn!("`lp` did not finish the RDP print job in time and was stopped");
        }
        Ok(crate::proc::Waited::Exited(output)) => {
            warn!("`lp` exited with {} for RDP print job", output.status);
        }
        Err(e) => warn!("Failed to wait for `lp`: {e}"),
    }
}

/// Lists local CUPS print queues, one destination per line via `lpstat -e`.
///
/// Returns an empty vector if `lpstat` is unavailable or fails; callers should
/// treat that as "no printers to forward" rather than an error.
// Called from an async context on a single-threaded runtime, so this still parks
// the runtime — but for at most [`CUPS_TIMEOUT`] rather than for as long as CUPS
// feels like taking. The note here used to say "fine while lpstat responds in
// <100 ms; move to spawn_blocking if that becomes a real issue", which named the
// right hazard and the wrong remedy: `spawn_blocking` moves an unbounded wait off
// the runtime thread, it does not bound it, and the connection would still stall
// waiting for the answer.
pub(crate) fn list_cups_printers() -> Vec<String> {
    let child = Command::new("lpstat")
        .arg("-e")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let output = match child {
        Ok(child) => match crate::proc::wait_bounded(child, CUPS_TIMEOUT, "lpstat -e") {
            Ok(crate::proc::Waited::Exited(output)) => output,
            Ok(crate::proc::Waited::TimedOut) => {
                warn!("`lpstat -e` did not answer in time; no printers will be forwarded");
                return Vec::new();
            }
            Err(e) => {
                warn!("Failed to wait for `lpstat -e` ({e}); no printers will be forwarded");
                return Vec::new();
            }
        },
        Err(e) => {
            warn!("`lpstat -e` failed ({e}); no printers will be forwarded");
            return Vec::new();
        }
    };
    parse_cups_printers(&String::from_utf8_lossy(&output.stdout))
}

/// Parses the destination list emitted by `lpstat -e` (one queue per line).
fn parse_cups_printers(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// Returns the CUPS default destination name from `lpstat -d`, if any.
///
/// Used only to decide announce ordering (default announced last so it wins the
/// IronRDP `DEFAULTPRINTER` flag race).
// Same bounded-blocking caveat as `list_cups_printers`.
pub(crate) fn cups_default_printer() -> Option<String> {
    let child = Command::new("lpstat")
        .arg("-d")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let output = crate::proc::wait_bounded(child, CUPS_TIMEOUT, "lpstat -d")
        .ok()?
        .output()?;
    parse_cups_default(&String::from_utf8_lossy(&output.stdout))
}

/// Parses the default destination from `lpstat -d` output.
///
/// Format: `system default destination: <name>` or
/// `no system default destination`.
fn parse_cups_default(stdout: &str) -> Option<String> {
    stdout
        .split(':')
        .nth(1)
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod printer_tests {
    use super::{parse_cups_default, parse_cups_printers};

    #[test]
    fn parses_multiline_printer_list() {
        let out = "Office_LaserJet\n  PDF \n\nKitchen_Inkjet\n";
        assert_eq!(
            parse_cups_printers(out),
            vec!["Office_LaserJet", "PDF", "Kitchen_Inkjet"]
        );
    }

    #[test]
    fn empty_printer_list() {
        assert!(parse_cups_printers("").is_empty());
        assert!(parse_cups_printers("\n  \n").is_empty());
    }

    #[test]
    fn parses_default_printer() {
        assert_eq!(
            parse_cups_default("system default destination: Office_LaserJet\n").as_deref(),
            Some("Office_LaserJet")
        );
    }

    #[test]
    fn no_default_printer() {
        assert_eq!(parse_cups_default("no system default destination\n"), None);
    }
}

/// Wire-level regression tests for the drive IO paths behind issue #256.
///
/// Windows Explorer aborts every rename/delete/copy on the redirected drive
/// with `0x8007045D` (ERROR_IO_DEVICE) if a Query Information response claims
/// `STATUS_SUCCESS` but carries a zero-length buffer, so these assert on the
/// encoded `Length` field rather than on internal state. The Set Information
/// tests then check that the request actually reaches the filesystem.
#[cfg(test)]
mod drive_io_tests {
    use std::collections::HashMap;

    use ironrdp::rdpdr::pdu::efs::{
        CreateOptions, DesiredAccess, DeviceIoRequest, FileDispositionInformation,
        FileEndOfFileInformation, MajorFunction, MinorFunction, SharedAccess,
    };

    use super::*;

    /// Byte offset of the `Length` field: 4-byte `RDPDR_HEADER` followed by the
    /// 12-byte `DR_DEVICE_IOCOMPLETION` (DeviceId, CompletionId, IoStatus).
    const LENGTH_OFFSET: usize = 16;
    /// Byte offset of `IoStatus` inside `DR_DEVICE_IOCOMPLETION`.
    const IO_STATUS_OFFSET: usize = 12;

    fn io_request(file_id: u32, major_function: MajorFunction) -> DeviceIoRequest {
        DeviceIoRequest {
            device_id: 1,
            file_id,
            completion_id: 0,
            major_function,
            minor_function: MinorFunction::from(0),
        }
    }

    fn read_u32_at(bytes: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("4-byte slice"))
    }

    /// Opens `<share>/probe.txt` and returns the backend plus the assigned file
    /// id, so each test starts from a live handle in `file_handles`.
    fn backend_with_open_file(dir: &tempfile::TempDir) -> (RustConnRdpdrBackend, u32) {
        std::fs::write(dir.path().join("probe.txt"), b"payload").expect("write probe file");

        let drives = HashMap::from([(1u32, dir.path().to_string_lossy().into_owned())]);
        let mut backend = RustConnRdpdrBackend::new(drives, HashMap::new());

        let create = ServerDriveIoRequest::ServerCreateDriveRequest(DeviceCreateRequest {
            device_io_request: io_request(0, MajorFunction::Create),
            // Mirror what Windows sends when it opens an existing file it means
            // to modify: FILE_OPEN plus write access in `desired_access`.
            desired_access: DesiredAccess::FILE_READ_DATA_OR_FILE_LIST_DIRECTORY
                | DesiredAccess::FILE_WRITE_DATA_OR_FILE_ADD_FILE,
            allocation_size: 0,
            file_attributes: FileAttributes::empty(),
            shared_access: SharedAccess::empty(),
            create_disposition: CreateDisposition::FILE_OPEN,
            create_options: CreateOptions::empty(),
            path: "\\probe.txt".to_owned(),
        });
        backend
            .handle_drive_io_request(create)
            .expect("create succeeds");

        (backend, 1)
    }

    fn query(
        backend: &mut RustConnRdpdrBackend,
        file_id: u32,
        lvl: FileInformationClassLevel,
    ) -> Vec<u8> {
        let responses = backend
            .handle_drive_io_request(ServerDriveIoRequest::ServerDriveQueryInformationRequest(
                ServerDriveQueryInformationRequest {
                    device_io_request: io_request(file_id, MajorFunction::QueryInformation),
                    file_info_class_lvl: lvl,
                },
            ))
            .expect("query information succeeds");

        assert_eq!(responses.len(), 1, "expected exactly one response PDU");
        responses[0]
            .encode_unframed_pdu()
            .expect("response PDU encodes")
    }

    #[test]
    fn attribute_tag_information_carries_an_eight_byte_buffer() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (mut backend, file_id) = backend_with_open_file(&dir);

        let bytes = query(
            &mut backend,
            file_id,
            FileInformationClassLevel::FILE_ATTRIBUTE_TAG_INFORMATION,
        );

        assert_eq!(
            read_u32_at(&bytes, IO_STATUS_OFFSET),
            0,
            "FileAttributeTagInformation must complete with STATUS_SUCCESS"
        );
        // FileAttributes (4) + ReparseTag (4). A zero here is the issue #256 bug.
        assert_eq!(
            read_u32_at(&bytes, LENGTH_OFFSET),
            8,
            "buffer must hold FileAttributes + ReparseTag"
        );
        assert_eq!(
            read_u32_at(&bytes, LENGTH_OFFSET + 8),
            0,
            "we never expose reparse points, so ReparseTag stays 0"
        );
    }

    #[test]
    fn unsupported_class_reports_not_supported_rather_than_empty_success() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (mut backend, file_id) = backend_with_open_file(&dir);

        let bytes = query(
            &mut backend,
            file_id,
            FileInformationClassLevel::FILE_NAMES_INFORMATION,
        );

        assert_eq!(
            read_u32_at(&bytes, IO_STATUS_OFFSET),
            0xC000_00BB,
            "unhandled classes must report STATUS_NOT_SUPPORTED"
        );
    }

    #[test]
    fn basic_and_standard_information_still_answer_with_a_buffer() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (mut backend, file_id) = backend_with_open_file(&dir);

        for lvl in [
            FileInformationClassLevel::FILE_BASIC_INFORMATION,
            FileInformationClassLevel::FILE_STANDARD_INFORMATION,
        ] {
            let bytes = query(&mut backend, file_id, lvl.clone());
            assert_eq!(read_u32_at(&bytes, IO_STATUS_OFFSET), 0, "{lvl} status");
            assert!(
                read_u32_at(&bytes, LENGTH_OFFSET) > 0,
                "{lvl} buffer length"
            );
        }
    }

    /// Issues a Set Information request and returns the encoded response bytes.
    fn set_info(
        backend: &mut RustConnRdpdrBackend,
        file_id: u32,
        set_buffer: FileInformationClass,
    ) -> Vec<u8> {
        let responses = backend
            .handle_drive_io_request(ServerDriveIoRequest::ServerDriveSetInformationRequest(
                ServerDriveSetInformationRequest {
                    device_io_request: io_request(file_id, MajorFunction::SetInformation),
                    set_buffer,
                },
            ))
            .expect("set information succeeds");

        assert_eq!(responses.len(), 1, "expected exactly one response PDU");
        responses[0]
            .encode_unframed_pdu()
            .expect("response PDU encodes")
    }

    fn close(backend: &mut RustConnRdpdrBackend, file_id: u32) {
        backend
            .handle_drive_io_request(ServerDriveIoRequest::DeviceCloseRequest(
                DeviceCloseRequest {
                    device_io_request: io_request(file_id, MajorFunction::Close),
                },
            ))
            .expect("close succeeds");
    }

    #[test]
    fn rename_moves_the_file_on_disk() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (mut backend, file_id) = backend_with_open_file(&dir);

        let bytes = set_info(
            &mut backend,
            file_id,
            FileInformationClass::Rename(FileRenameInformation {
                replace_if_exists: Boolean::False,
                file_name: "\\renamed.txt".to_owned(),
            }),
        );

        assert_eq!(read_u32_at(&bytes, IO_STATUS_OFFSET), 0, "rename status");
        assert!(
            dir.path().join("renamed.txt").exists(),
            "new name must exist on disk"
        );
        assert!(
            !dir.path().join("probe.txt").exists(),
            "old name must be gone"
        );
        // Follow-up IRPs reuse the same file_id, so the tracked path must move too.
        let expected = dir
            .path()
            .join("renamed.txt")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            backend.file_paths.get(&file_id),
            Some(&expected),
            "tracked path must follow the rename"
        );
    }

    #[test]
    fn rename_onto_an_existing_name_is_refused_without_replace() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("taken.txt"), b"other").expect("write blocker");
        let (mut backend, file_id) = backend_with_open_file(&dir);

        let bytes = set_info(
            &mut backend,
            file_id,
            FileInformationClass::Rename(FileRenameInformation {
                replace_if_exists: Boolean::False,
                file_name: "\\taken.txt".to_owned(),
            }),
        );

        assert_eq!(
            read_u32_at(&bytes, IO_STATUS_OFFSET),
            0xC000_0035,
            "must report STATUS_OBJECT_NAME_COLLISION"
        );
        assert_eq!(
            std::fs::read(dir.path().join("taken.txt")).expect("blocker still readable"),
            b"other",
            "the existing file must not be clobbered"
        );
    }

    #[test]
    fn end_of_file_information_truncates() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (mut backend, file_id) = backend_with_open_file(&dir);

        let bytes = set_info(
            &mut backend,
            file_id,
            FileInformationClass::EndOfFile(FileEndOfFileInformation { end_of_file: 3 }),
        );

        assert_eq!(read_u32_at(&bytes, IO_STATUS_OFFSET), 0, "truncate status");
        assert_eq!(
            std::fs::read(dir.path().join("probe.txt")).expect("read back"),
            b"pay",
            "file must be truncated to 3 bytes"
        );
    }

    #[test]
    fn disposition_information_deletes_on_close() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (mut backend, file_id) = backend_with_open_file(&dir);

        let bytes = set_info(
            &mut backend,
            file_id,
            FileInformationClass::Disposition(FileDispositionInformation { delete_pending: 1 }),
        );
        assert_eq!(
            read_u32_at(&bytes, IO_STATUS_OFFSET),
            0,
            "disposition status"
        );
        assert!(
            dir.path().join("probe.txt").exists(),
            "removal must wait for Close"
        );

        close(&mut backend, file_id);
        assert!(
            !dir.path().join("probe.txt").exists(),
            "Close must remove the file"
        );
    }

    #[test]
    fn share_paths_reject_parent_traversal_and_symlinks() {
        let share = tempfile::tempdir().expect("share");
        let outside = tempfile::tempdir().expect("outside");
        std::os::unix::fs::symlink(outside.path(), share.path().join("escape"))
            .expect("create symlink");

        let drives = HashMap::from([(1u32, share.path().to_string_lossy().into_owned())]);
        let backend = RustConnRdpdrBackend::new(drives, HashMap::new());

        assert!(matches!(
            backend.resolve_share_path(1, "\\..\\outside.txt"),
            Err(NtStatus::ACCESS_DENIED)
        ));
        assert!(matches!(
            backend.resolve_share_path(1, "\\escape\\outside.txt"),
            Err(NtStatus::ACCESS_DENIED)
        ));
        assert_eq!(
            backend
                .resolve_share_path(1, "\\safe\\file.txt")
                .expect("safe path"),
            share.path().join("safe/file.txt")
        );
    }

    #[test]
    fn rename_cannot_escape_the_share() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (mut backend, file_id) = backend_with_open_file(&dir);

        let bytes = set_info(
            &mut backend,
            file_id,
            FileInformationClass::Rename(FileRenameInformation {
                replace_if_exists: Boolean::True,
                file_name: "\\..\\escaped.txt".to_owned(),
            }),
        );

        assert_eq!(
            read_u32_at(&bytes, IO_STATUS_OFFSET),
            0xC000_0022,
            "traversal must report STATUS_ACCESS_DENIED"
        );
        assert!(dir.path().join("probe.txt").exists());
    }

    #[test]
    fn directory_query_honours_pattern_and_information_class() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("target.txt"), b"target").expect("target");
        std::fs::write(dir.path().join("distractor.txt"), b"other").expect("distractor");
        let drives = HashMap::from([(1u32, dir.path().to_string_lossy().into_owned())]);
        let mut backend = RustConnRdpdrBackend::new(drives, HashMap::new());

        backend
            .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(
                DeviceCreateRequest {
                    device_io_request: io_request(0, MajorFunction::Create),
                    desired_access: DesiredAccess::FILE_READ_DATA_OR_FILE_LIST_DIRECTORY,
                    allocation_size: 0,
                    file_attributes: FileAttributes::empty(),
                    shared_access: SharedAccess::empty(),
                    create_disposition: CreateDisposition::FILE_OPEN,
                    create_options: CreateOptions::FILE_DIRECTORY_FILE,
                    path: "\\".to_owned(),
                },
            ))
            .expect("open directory");

        let responses = backend
            .handle_drive_io_request(ServerDriveIoRequest::ServerDriveQueryDirectoryRequest(
                ServerDriveQueryDirectoryRequest {
                    device_io_request: io_request(1, MajorFunction::DirectoryControl),
                    file_info_class_lvl: FileInformationClassLevel::FILE_NAMES_INFORMATION,
                    initial_query: 1,
                    path: "\\target.txt".to_owned(),
                },
            ))
            .expect("query directory");
        let bytes = responses[0].encode_unframed_pdu().expect("encode response");

        assert_eq!(read_u32_at(&bytes, IO_STATUS_OFFSET), 0);
        let target_utf16 = "target.txt"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        assert!(
            bytes
                .windows(target_utf16.len())
                .any(|window| window == target_utf16),
            "response must contain only the requested filename"
        );
        assert_eq!(
            read_u32_at(&bytes, LENGTH_OFFSET),
            12 + u32::try_from(target_utf16.len()).expect("filename length"),
            "FILE_NAMES_INFORMATION must use its compact wire layout"
        );
    }

    #[test]
    fn windows_wildcards_are_case_insensitive() {
        assert!(wildcard_match("*.TXT", "report.txt"));
        assert!(wildcard_match("file?.log", "File1.log"));
        assert!(!wildcard_match("file?.log", "file10.log"));
        assert_eq!(directory_search_pattern("\\folder\\*.*"), "*");
    }
}
