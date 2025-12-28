//! FreeRDP thread isolation and clipboard file transfer
//!
//! This module provides thread-safe FreeRDP wrapper and clipboard file transfer
//! state management for RDP sessions.

use crate::embedded_rdp_buffer::PixelBuffer;
use crate::embedded_rdp_types::{
    EmbeddedRdpError, FreeRdpThreadState, RdpCommand, RdpConfig, RdpEvent,
};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

#[cfg(feature = "rdp-embedded")]
use rustconn_core::rdp_client::ClipboardFileInfo;

// ============================================================================
// Clipboard File Transfer State (for rdp-embedded feature)
// ============================================================================

/// State of a single file download from RDP clipboard
#[cfg(feature = "rdp-embedded")]
#[derive(Debug, Clone)]
pub struct FileDownloadState {
    /// File information from server
    pub file_info: ClipboardFileInfo,
    /// Total file size (may be updated after size request)
    pub total_size: u64,
    /// Bytes received so far
    pub bytes_received: u64,
    /// Accumulated data chunks
    pub data: Vec<u8>,
    /// Whether download is complete
    pub complete: bool,
    /// Local path where file will be saved
    pub local_path: Option<PathBuf>,
}

#[cfg(feature = "rdp-embedded")]
impl FileDownloadState {
    /// Creates a new file download state
    pub fn new(file_info: ClipboardFileInfo) -> Self {
        let total_size = file_info.size;
        Self {
            file_info,
            total_size,
            bytes_received: 0,
            data: Vec::new(),
            complete: false,
            local_path: None,
        }
    }

    /// Returns download progress as fraction (0.0 to 1.0)
    #[allow(dead_code)]
    pub fn progress(&self) -> f64 {
        if self.total_size == 0 {
            return if self.complete { 1.0 } else { 0.0 };
        }
        #[allow(clippy::cast_precision_loss)]
        let progress = self.bytes_received as f64 / self.total_size as f64;
        progress.min(1.0)
    }
}

/// Manages clipboard file transfer state
#[cfg(feature = "rdp-embedded")]
#[derive(Debug, Default)]
pub struct ClipboardFileTransfer {
    /// Available files from server clipboard
    pub available_files: Vec<ClipboardFileInfo>,
    /// Active downloads keyed by stream_id
    pub downloads: HashMap<u32, FileDownloadState>,
    /// Next stream ID to use for requests
    pub next_stream_id: u32,
    /// Target directory for saving files
    pub target_directory: Option<PathBuf>,
    /// Total files to download
    pub total_files: usize,
    /// Completed downloads count
    pub completed_count: usize,
}

#[cfg(feature = "rdp-embedded")]
impl ClipboardFileTransfer {
    /// Creates a new file transfer manager
    pub fn new() -> Self {
        Self {
            available_files: Vec::new(),
            downloads: HashMap::new(),
            next_stream_id: 1,
            target_directory: None,
            total_files: 0,
            completed_count: 0,
        }
    }

    /// Sets available files from server clipboard
    pub fn set_available_files(&mut self, files: Vec<ClipboardFileInfo>) {
        self.available_files = files;
        self.downloads.clear();
        self.next_stream_id = 1;
        self.total_files = 0;
        self.completed_count = 0;
    }

    /// Starts download for a file, returns stream_id
    pub fn start_download(&mut self, file_index: u32) -> Option<u32> {
        let file_info = self.available_files.get(file_index as usize)?.clone();
        let stream_id = self.next_stream_id;
        self.next_stream_id += 1;
        self.downloads
            .insert(stream_id, FileDownloadState::new(file_info));
        Some(stream_id)
    }

    /// Updates file size for a download
    pub fn update_size(&mut self, stream_id: u32, size: u64) {
        if let Some(state) = self.downloads.get_mut(&stream_id) {
            state.total_size = size;
        }
    }

    /// Appends data to a download
    pub fn append_data(&mut self, stream_id: u32, data: &[u8], is_last: bool) {
        if let Some(state) = self.downloads.get_mut(&stream_id) {
            state.data.extend_from_slice(data);
            state.bytes_received += data.len() as u64;
            if is_last {
                state.complete = true;
                self.completed_count += 1;
            }
        }
    }

    /// Saves a completed download to disk
    pub fn save_download(&self, stream_id: u32) -> Result<PathBuf, std::io::Error> {
        let state = self.downloads.get(&stream_id).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "Download not found")
        })?;

        if !state.complete {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Download not complete",
            ));
        }

        let target_dir = self.target_directory.as_ref().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "Target directory not set")
        })?;

        let file_path = target_dir.join(&state.file_info.name);
        let mut file = std::fs::File::create(&file_path)?;
        file.write_all(&state.data)?;
        Ok(file_path)
    }

    /// Returns overall progress (0.0 to 1.0)
    #[allow(clippy::cast_precision_loss)]
    pub fn overall_progress(&self) -> f64 {
        if self.total_files == 0 {
            return 0.0;
        }
        self.completed_count as f64 / self.total_files as f64
    }

    /// Returns true if all downloads are complete
    pub fn all_complete(&self) -> bool {
        self.total_files > 0 && self.completed_count >= self.total_files
    }

    /// Clears all state
    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.available_files.clear();
        self.downloads.clear();
        self.next_stream_id = 1;
        self.target_directory = None;
        self.total_files = 0;
        self.completed_count = 0;
    }
}

// ============================================================================
// FreeRDP Thread Isolation (Requirement 6.3)
// ============================================================================

/// Thread-safe FreeRDP wrapper that isolates Qt from GTK main thread
///
/// This struct runs FreeRDP operations in a dedicated thread to avoid
/// Qt/GTK threading conflicts that cause QSocketNotifier and Wayland
/// requestActivate errors.
#[allow(dead_code)]
pub struct FreeRdpThread {
    /// Handle to the FreeRDP process
    process: Arc<Mutex<Option<Child>>>,
    /// Shared memory buffer for frame data
    frame_buffer: Arc<Mutex<PixelBuffer>>,
    /// Channel for sending commands to FreeRDP thread
    command_tx: mpsc::Sender<RdpCommand>,
    /// Channel for receiving events from FreeRDP thread
    event_rx: mpsc::Receiver<RdpEvent>,
    /// Thread handle
    thread_handle: Option<JoinHandle<()>>,
    /// Current thread state
    state: Arc<Mutex<FreeRdpThreadState>>,
    /// Whether fallback was triggered
    fallback_triggered: Arc<Mutex<bool>>,
}

impl FreeRdpThread {
    /// Spawns FreeRDP in a dedicated thread to avoid Qt/GTK conflicts
    pub fn spawn(config: &RdpConfig) -> Result<Self, EmbeddedRdpError> {
        let (cmd_tx, cmd_rx) = mpsc::channel::<RdpCommand>();
        let (evt_tx, evt_rx) = mpsc::channel::<RdpEvent>();

        let frame_buffer = Arc::new(Mutex::new(PixelBuffer::new(config.width, config.height)));
        let process = Arc::new(Mutex::new(None));
        let state = Arc::new(Mutex::new(FreeRdpThreadState::NotStarted));
        let fallback_triggered = Arc::new(Mutex::new(false));

        let frame_buffer_clone = Arc::clone(&frame_buffer);
        let process_clone = Arc::clone(&process);
        let state_clone = Arc::clone(&state);
        let fallback_clone = Arc::clone(&fallback_triggered);
        let config_clone = config.clone();

        let thread_handle = thread::spawn(move || {
            Self::run_freerdp_loop(
                cmd_rx,
                evt_tx,
                frame_buffer_clone,
                process_clone,
                state_clone,
                fallback_clone,
                config_clone,
            );
        });

        *state.lock().unwrap() = FreeRdpThreadState::Idle;

        Ok(Self {
            process,
            frame_buffer,
            command_tx: cmd_tx,
            event_rx: evt_rx,
            thread_handle: Some(thread_handle),
            state,
            fallback_triggered,
        })
    }

    /// Main loop for FreeRDP operations running in dedicated thread
    fn run_freerdp_loop(
        cmd_rx: mpsc::Receiver<RdpCommand>,
        evt_tx: mpsc::Sender<RdpEvent>,
        _frame_buffer: Arc<Mutex<PixelBuffer>>,
        process: Arc<Mutex<Option<Child>>>,
        state: Arc<Mutex<FreeRdpThreadState>>,
        fallback_triggered: Arc<Mutex<bool>>,
        initial_config: RdpConfig,
    ) {
        // Set environment variables to suppress Qt/Wayland warnings
        std::env::set_var("QT_LOGGING_RULES", "qt.qpa.wayland=false;qt.qpa.*=false");
        std::env::set_var("QT_QPA_PLATFORM", "xcb");

        let mut current_config = Some(initial_config);

        loop {
            match cmd_rx.recv() {
                Ok(RdpCommand::Connect(config)) => {
                    *state.lock().unwrap() = FreeRdpThreadState::Connecting;
                    current_config = Some(config.clone());

                    match Self::launch_freerdp(&config, &process) {
                        Ok(()) => {
                            *state.lock().unwrap() = FreeRdpThreadState::Connected;
                            let _ = evt_tx.send(RdpEvent::Connected);
                        }
                        Err(e) => {
                            *fallback_triggered.lock().unwrap() = true;
                            *state.lock().unwrap() = FreeRdpThreadState::Error;
                            let _ = evt_tx.send(RdpEvent::FallbackTriggered(e.to_string()));
                        }
                    }
                }
                Ok(RdpCommand::Disconnect) => {
                    Self::cleanup_process(&process);
                    *state.lock().unwrap() = FreeRdpThreadState::Idle;
                    let _ = evt_tx.send(RdpEvent::Disconnected);
                }
                Ok(RdpCommand::KeyEvent {
                    keyval: _,
                    pressed: _,
                }) => {
                    // Forward keyboard event to FreeRDP process
                }
                Ok(RdpCommand::MouseEvent {
                    x: _,
                    y: _,
                    button: _,
                    pressed: _,
                }) => {
                    // Forward mouse event to FreeRDP process
                }
                Ok(RdpCommand::Resize { width, height }) => {
                    if let Some(ref mut config) = current_config {
                        config.width = width;
                        config.height = height;
                    }
                }
                Ok(RdpCommand::SendCtrlAltDel) => {
                    eprintln!("[FreeRDP] Ctrl+Alt+Del requested");
                }
                Ok(RdpCommand::Shutdown) => {
                    *state.lock().unwrap() = FreeRdpThreadState::ShuttingDown;
                    Self::cleanup_process(&process);
                    break;
                }
                Err(_) => {
                    Self::cleanup_process(&process);
                    break;
                }
            }
        }
    }

    /// Launches FreeRDP with Qt error suppression
    fn launch_freerdp(
        config: &RdpConfig,
        process: &Arc<Mutex<Option<Child>>>,
    ) -> Result<(), EmbeddedRdpError> {
        // Try wlfreerdp first for embedded mode
        let binary = if Command::new("which")
            .arg("wlfreerdp")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
        {
            "wlfreerdp"
        } else {
            return Err(EmbeddedRdpError::WlFreeRdpNotAvailable);
        };

        let mut cmd = Command::new(binary);

        // Set environment to suppress Qt warnings
        cmd.env("QT_LOGGING_RULES", "qt.qpa.wayland=false;qt.qpa.*=false");
        cmd.env("QT_QPA_PLATFORM", "xcb");

        // Build connection arguments
        if let Some(ref domain) = config.domain {
            if !domain.is_empty() {
                cmd.arg(format!("/d:{domain}"));
            }
        }

        if let Some(ref username) = config.username {
            cmd.arg(format!("/u:{username}"));
        }

        if let Some(ref password) = config.password {
            if !password.is_empty() {
                cmd.arg(format!("/p:{password}"));
            }
        }

        cmd.arg(format!("/w:{}", config.width));
        cmd.arg(format!("/h:{}", config.height));
        cmd.arg("/cert:ignore");
        cmd.arg("/dynamic-resolution");

        if config.clipboard_enabled {
            cmd.arg("+clipboard");
        }

        for arg in &config.extra_args {
            cmd.arg(arg);
        }

        if config.port == 3389 {
            cmd.arg(format!("/v:{}", config.host));
        } else {
            cmd.arg(format!("/v:{}:{}", config.host, config.port));
        }

        // Redirect stderr to suppress Qt warnings
        cmd.stderr(Stdio::null());

        match cmd.spawn() {
            Ok(child) => {
                *process.lock().unwrap() = Some(child);
                Ok(())
            }
            Err(e) => Err(EmbeddedRdpError::FreeRdpInit(e.to_string())),
        }
    }

    /// Cleans up the FreeRDP process
    fn cleanup_process(process: &Arc<Mutex<Option<Child>>>) {
        let child = process.lock().unwrap().take();
        if let Some(mut child) = child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// Sends a command to the FreeRDP thread
    pub fn send_command(&self, cmd: RdpCommand) -> Result<(), EmbeddedRdpError> {
        self.command_tx
            .send(cmd)
            .map_err(|e| EmbeddedRdpError::ThreadError(e.to_string()))
    }

    /// Tries to receive an event from the FreeRDP thread (non-blocking)
    pub fn try_recv_event(&self) -> Option<RdpEvent> {
        self.event_rx.try_recv().ok()
    }

    /// Returns the current thread state
    pub fn state(&self) -> FreeRdpThreadState {
        *self.state.lock().unwrap()
    }

    /// Returns whether fallback was triggered
    pub fn fallback_triggered(&self) -> bool {
        *self.fallback_triggered.lock().unwrap()
    }

    /// Returns a reference to the frame buffer
    pub fn frame_buffer(&self) -> &Arc<Mutex<PixelBuffer>> {
        &self.frame_buffer
    }

    /// Shuts down the FreeRDP thread
    pub fn shutdown(&mut self) {
        let _ = self.command_tx.send(RdpCommand::Shutdown);
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for FreeRdpThread {
    fn drop(&mut self) {
        self.shutdown();
    }
}
