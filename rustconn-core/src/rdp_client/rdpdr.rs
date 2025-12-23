//! RDPDR (Device Redirection) backend for shared folders
//!
//! This module implements the `RdpdrBackend` trait from `ironrdp-rdpdr` to provide
//! shared folder functionality for RDP sessions.

use ironrdp::core::impl_as_any;
use ironrdp::pdu::PduResult;
use ironrdp::rdpdr::pdu::efs::{
    ClientDriveQueryDirectoryResponse, ClientDriveQueryInformationResponse,
    ClientDriveQueryVolumeInformationResponse, ClientDriveSetInformationResponse,
    DeviceCloseResponse, DeviceControlResponse, DeviceCreateResponse,
    DeviceIoResponse, DeviceReadResponse, DeviceWriteResponse, FileAttributes,
    FileBasicInformation, FileBothDirectoryInformation, FileInformationClass,
    FileInformationClassLevel, FileFsAttributeInformation, FileFsFullSizeInformation,
    FileFsSizeInformation, FileFsVolumeInformation, FileStandardInformation,
    FileSystemAttributes, FileSystemInformationClass, FileSystemInformationClassLevel,
    Information, NtStatus, ServerDeviceAnnounceResponse, ServerDriveIoRequest,
    Boolean, CreateDisposition, CreateOptions, DeviceCloseRequest, DeviceControlRequest,
    DeviceCreateRequest, DeviceReadRequest, DeviceWriteRequest,
    ServerDriveQueryDirectoryRequest, ServerDriveQueryInformationRequest,
    ServerDriveQueryVolumeInformationRequest, ServerDriveSetInformationRequest,
};
use ironrdp::rdpdr::pdu::esc::{ScardCall, ScardIoCtlCode};
use ironrdp::rdpdr::pdu::RdpdrPdu;
use ironrdp::rdpdr::RdpdrBackend;
use ironrdp::svc::SvcMessage;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use tracing::{debug, warn};

/// RDPDR backend for Linux/Unix shared folders
#[derive(Debug)]
pub struct RustConnRdpdrBackend {
    /// Base path for the shared folder
    base_path: String,
    /// Next file ID to assign
    next_file_id: u32,
    /// Map of file IDs to open file handles
    file_handles: HashMap<u32, File>,
    /// Map of file IDs to their paths
    file_paths: HashMap<u32, String>,
    /// Map of file IDs to directory iterators
    dir_entries: HashMap<u32, Vec<String>>,
}

impl_as_any!(RustConnRdpdrBackend);

impl RustConnRdpdrBackend {
    /// Creates a new RDPDR backend with the given base path
    #[must_use]
    pub fn new(base_path: String) -> Self {
        // Ensure path ends with /
        let base_path = if base_path.ends_with('/') {
            base_path
        } else {
            format!("{base_path}/")
        };
        Self {
            base_path,
            next_file_id: 1,
            file_handles: HashMap::new(),
            file_paths: HashMap::new(),
            dir_entries: HashMap::new(),
        }
    }

    /// Allocates a new file ID
    fn alloc_file_id(&mut self) -> u32 {
        let id = self.next_file_id;
        self.next_file_id = self.next_file_id.wrapping_add(1);
        id
    }

    /// Converts a Windows-style path to Unix path
    fn to_unix_path(&self, windows_path: &str) -> String {
        let unix_path = windows_path.replace('\\', "/");
        format!("{}{}", self.base_path, unix_path.trim_start_matches('/'))
    }
}

impl RdpdrBackend for RustConnRdpdrBackend {
    fn handle_server_device_announce_response(
        &mut self,
        pdu: ServerDeviceAnnounceResponse,
    ) -> PduResult<()> {
        debug!("Device announce response: {:?}", pdu);
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

    fn handle_drive_io_request(&mut self, req: ServerDriveIoRequest) -> PduResult<Vec<SvcMessage>> {
        debug!("Drive IO request: {:?}", req);
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
                self.handle_set_info(set_req)
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
            ServerDriveIoRequest::ServerDriveNotifyChangeDirectoryRequest(_) => {
                // Directory change notifications not implemented
                Ok(Vec::new())
            }
            ServerDriveIoRequest::ServerDriveLockControlRequest(_) => {
                // File locking not implemented
                Ok(Vec::new())
            }
        }
    }
}

impl RustConnRdpdrBackend {
    fn handle_create(&mut self, req: DeviceCreateRequest) -> PduResult<Vec<SvcMessage>> {
        let file_id = self.alloc_file_id();
        let path = self.to_unix_path(&req.path);
        debug!("Create request for path: {} (file_id: {})", path, file_id);

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
                    if req.create_disposition == CreateDisposition::FILE_CREATE
                        || req.create_disposition == CreateDisposition::FILE_OPEN_IF
                    {
                        if std::fs::create_dir_all(&path).is_ok() {
                            if let Ok(file) = OpenOptions::new().read(true).open(&path) {
                                self.file_handles.insert(file_id, file);
                                self.file_paths.insert(file_id, path);
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
            }
        }

        // Handle file creation/opening
        let mut opts = OpenOptions::new();
        match req.create_disposition {
            CreateDisposition::FILE_OPEN => {
                opts.read(true);
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

        match opts.open(&path) {
            Ok(file) => {
                self.file_handles.insert(file_id, file);
                self.file_paths.insert(file_id, path);
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
                warn!("Failed to open file {}: {}", path, e);
                Ok(vec![SvcMessage::from(RdpdrPdu::DeviceCreateResponse(
                    DeviceCreateResponse {
                        device_io_reply: DeviceIoResponse::new(
                            req.device_io_request,
                            NtStatus::NO_SUCH_FILE,
                        ),
                        file_id,
                        information: Information::empty(),
                    },
                ))])
            }
        }
    }

    fn handle_close(&mut self, req: DeviceCloseRequest) -> PduResult<Vec<SvcMessage>> {
        let file_id = req.device_io_request.file_id;
        self.file_handles.remove(&file_id);
        self.file_paths.remove(&file_id);
        self.dir_entries.remove(&file_id);
        Ok(vec![SvcMessage::from(RdpdrPdu::DeviceCloseResponse(
            DeviceCloseResponse {
                device_io_response: DeviceIoResponse::new(
                    req.device_io_request,
                    NtStatus::SUCCESS,
                ),
            },
        ))])
    }

    fn handle_read(&mut self, req: DeviceReadRequest) -> PduResult<Vec<SvcMessage>> {
        let file_id = req.device_io_request.file_id;
        if let Some(file) = self.file_handles.get_mut(&file_id) {
            if file.seek(SeekFrom::Start(req.offset)).is_ok() {
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

    fn handle_write(&mut self, req: DeviceWriteRequest) -> PduResult<Vec<SvcMessage>> {
        let file_id = req.device_io_request.file_id;
        if let Some(file) = self.file_handles.get_mut(&file_id) {
            if file.seek(SeekFrom::Start(req.offset)).is_ok() {
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


    #[allow(clippy::needless_pass_by_ref_mut)]
    fn handle_query_info(
        &mut self,
        req: ServerDriveQueryInformationRequest,
    ) -> PduResult<Vec<SvcMessage>> {
        let file_id = req.device_io_request.file_id;
        let file = match self.file_handles.get(&file_id) {
            Some(f) => f,
            None => {
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
            }
        };

        let meta = match file.metadata() {
            Ok(m) => m,
            Err(_) => {
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
            }
        };

        let path = self.file_paths.get(&file_id).cloned().unwrap_or_default();
        let file_name = PathBuf::from(&path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let file_attrs = get_file_attributes(&meta, &file_name);

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
            _ => None,
        };

        Ok(vec![SvcMessage::from(
            RdpdrPdu::ClientDriveQueryInformationResponse(ClientDriveQueryInformationResponse {
                device_io_response: DeviceIoResponse::new(
                    req.device_io_request,
                    NtStatus::SUCCESS,
                ),
                buffer,
            }),
        )])
    }

    #[allow(clippy::needless_pass_by_ref_mut)]
    fn handle_query_volume(
        &mut self,
        req: ServerDriveQueryVolumeInformationRequest,
    ) -> PduResult<Vec<SvcMessage>> {
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
            FileSystemInformationClassLevel::FILE_FS_VOLUME_INFORMATION => {
                Some(FileSystemInformationClass::FileFsVolumeInformation(
                    FileFsVolumeInformation {
                        volume_creation_time: unix_to_filetime(0),
                        volume_serial_number: 0x1234_5678,
                        supports_objects: Boolean::False,
                        volume_label: "RustConn".to_owned(),
                    },
                ))
            }
            FileSystemInformationClassLevel::FILE_FS_SIZE_INFORMATION => {
                // Return some reasonable defaults
                Some(FileSystemInformationClass::FileFsSizeInformation(
                    FileFsSizeInformation {
                        total_alloc_units: 1_000_000,
                        available_alloc_units: 500_000,
                        sectors_per_alloc_unit: 8,
                        bytes_per_sector: 512,
                    },
                ))
            }
            FileSystemInformationClassLevel::FILE_FS_FULL_SIZE_INFORMATION => {
                Some(FileSystemInformationClass::FileFsFullSizeInformation(
                    FileFsFullSizeInformation {
                        total_alloc_units: 1_000_000,
                        caller_available_alloc_units: 500_000,
                        actual_available_alloc_units: 500_000,
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

    fn handle_query_directory(
        &mut self,
        req: ServerDriveQueryDirectoryRequest,
    ) -> PduResult<Vec<SvcMessage>> {
        let file_id = req.device_io_request.file_id;

        if req.initial_query > 0 {
            // Initial query - read directory contents
            let path = match self.file_paths.get(&file_id) {
                Some(p) => p.clone(),
                None => {
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
                }
            };

            // Read directory entries
            let entries: Vec<String> = match std::fs::read_dir(&path) {
                Ok(dir) => dir
                    .filter_map(|e| e.ok())
                    .map(|e| e.path().to_string_lossy().into_owned())
                    .collect(),
                Err(_) => Vec::new(),
            };

            self.dir_entries.insert(file_id, entries);
        }

        // Get next entry
        let entries = self.dir_entries.get_mut(&file_id);
        let entry_path = entries.and_then(|e| if e.is_empty() { None } else { Some(e.remove(0)) });

        match entry_path {
            Some(full_path) => {
                let file_name = PathBuf::from(&full_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();

                if let Ok(meta) = std::fs::metadata(&full_path) {
                    let file_attrs = get_file_attributes(&meta, &file_name);
                    let info = FileBothDirectoryInformation::new(
                        unix_to_filetime(meta.ctime()),
                        unix_to_filetime(meta.ctime()),
                        unix_to_filetime(meta.atime()),
                        unix_to_filetime(meta.mtime()),
                        meta.size() as i64,
                        file_attrs,
                        file_name,
                    );
                    return Ok(vec![SvcMessage::from(
                        RdpdrPdu::ClientDriveQueryDirectoryResponse(
                            ClientDriveQueryDirectoryResponse {
                                device_io_reply: DeviceIoResponse::new(
                                    req.device_io_request,
                                    NtStatus::SUCCESS,
                                ),
                                buffer: Some(FileInformationClass::BothDirectory(info)),
                            },
                        ),
                    )]);
                }
            }
            None => {}
        }

        // No more entries
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

    #[allow(clippy::needless_pass_by_ref_mut)]
    fn handle_set_info(
        &mut self,
        req: ServerDriveSetInformationRequest,
    ) -> PduResult<Vec<SvcMessage>> {
        // Basic implementation - just acknowledge
        Ok(vec![SvcMessage::from(
            RdpdrPdu::ClientDriveSetInformationResponse(
                ClientDriveSetInformationResponse::new(&req, NtStatus::SUCCESS)
                    .unwrap_or_else(|_| ClientDriveSetInformationResponse::new(&req, NtStatus::UNSUCCESSFUL).expect("infallible")),
            ),
        )])
    }
}

/// Converts Unix timestamp (seconds) to Windows FILETIME (100-nanosecond intervals since 1601)
const fn unix_to_filetime(unix_secs: i64) -> i64 {
    // Windows FILETIME epoch is January 1, 1601
    // Unix epoch is January 1, 1970
    // Difference is 11644473600 seconds
    const EPOCH_DIFF: i64 = 116_444_736_000_000_000;
    unix_secs.saturating_mul(10_000_000).saturating_add(EPOCH_DIFF)
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
