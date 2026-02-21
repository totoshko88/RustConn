//! FreeRDP thread isolation and clipboard file transfer
//!
//! This module provides thread-safe FreeRDP wrapper and clipboard file transfer
//! state management for RDP sessions.
//!
//! # Safety Notes
//!
//! Mutex locks in this module protect simple state flags and process handles.
//! They are held briefly. If a mutex is poisoned (indicating a thread panic while
//! holding the lock), we recover gracefully by extracting the inner value and
//! setting an error state rather than propagating the panic.

use super::buffer::PixelBuffer;
use super::types::{EmbeddedRdpError, FreeRdpThreadState, RdpCommand, RdpConfig, RdpEvent};
use secrecy::ExposeSecret;
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
        crate::utils::progress_fraction(self.bytes_received, self.total_size)
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
    pub fn overall_progress(&self) -> f64 {
        if self.total_files == 0 {
            return 0.0;
        }
        crate::utils::progress_fraction(self.completed_count as u64, self.total_files as u64)
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
// Mutex Poisoning Recovery Helpers
// ============================================================================

/// Safely locks a mutex, recovering from poisoning by extracting the inner value.
///
/// If the mutex is poisoned (a thread panicked while holding the lock),
/// we recover by extracting the inner value. This is safe because our
/// mutex-protected values are simple state flags that can be reset.
fn lock_or_recover<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!("Mutex was poisoned, recovering inner value");
            poisoned.into_inner()
        }
    }
}

// ============================================================================
// FreeRDP Thread Isolation (Requirement 6.3)
// ============================================================================

/// Consolidated shared state for the FreeRDP thread.
///
/// Groups process handle, thread state, and fallback flag into a single
/// mutex-protected struct to reduce lock contention and simplify reasoning
/// about concurrent access.
struct FreeRdpSharedState {
    /// Handle to the FreeRDP child process
    process: Option<Child>,
    /// Current thread state
    state: FreeRdpThreadState,
    /// Whether fallback to external client was triggered
    fallback_triggered: bool,
}

impl FreeRdpSharedState {
    /// Creates a new shared state with default values
    fn new() -> Self {
        Self {
            process: None,
            state: FreeRdpThreadState::NotStarted,
            fallback_triggered: false,
        }
    }

    /// Kills and waits for the child process if running
    fn cleanup_process(&mut self) {
        if let Some(mut child) = self.process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Thread-safe FreeRDP wrapper that isolates Qt from GTK main thread
///
/// This struct runs FreeRDP operations in a dedicated thread to avoid
/// Qt/GTK threading conflicts that cause QSocketNotifier and Wayland
/// requestActivate errors.
#[allow(dead_code)]
pub struct FreeRdpThread {
    /// Consolidated process, state, and fallback flag (single lock)
    shared: Arc<Mutex<FreeRdpSharedState>>,
    /// Shared memory buffer for frame data (separate lock for rendering)
    frame_buffer: Arc<Mutex<PixelBuffer>>,
    /// Channel for sending commands to FreeRDP thread
    command_tx: mpsc::Sender<RdpCommand>,
    /// Channel for receiving events from FreeRDP thread
    event_rx: mpsc::Receiver<RdpEvent>,
    /// Thread handle
    thread_handle: Option<JoinHandle<()>>,
}

impl FreeRdpThread {
    /// Spawns FreeRDP in a dedicated thread to avoid Qt/GTK conflicts
    pub fn spawn(config: &RdpConfig) -> Result<Self, EmbeddedRdpError> {
        let (cmd_tx, cmd_rx) = mpsc::channel::<RdpCommand>();
        let (evt_tx, evt_rx) = mpsc::channel::<RdpEvent>();

        let frame_buffer = Arc::new(Mutex::new(PixelBuffer::new(config.width, config.height)));
        let shared = Arc::new(Mutex::new(FreeRdpSharedState::new()));

        let frame_buffer_clone = Arc::clone(&frame_buffer);
        let shared_clone = Arc::clone(&shared);
        let config_clone = config.clone();

        let thread_handle = thread::spawn(move || {
            Self::run_freerdp_loop(
                cmd_rx,
                evt_tx,
                frame_buffer_clone,
                shared_clone,
                config_clone,
            );
        });

        // Initialize state - safe because thread just started and mutex is not poisoned
        lock_or_recover(&shared).state = FreeRdpThreadState::Idle;

        Ok(Self {
            shared,
            frame_buffer,
            command_tx: cmd_tx,
            event_rx: evt_rx,
            thread_handle: Some(thread_handle),
        })
    }

    /// Main loop for FreeRDP operations running in dedicated thread
    ///
    /// Uses mutex poisoning recovery to gracefully handle thread panics.
    fn run_freerdp_loop(
        cmd_rx: mpsc::Receiver<RdpCommand>,
        evt_tx: mpsc::Sender<RdpEvent>,
        _frame_buffer: Arc<Mutex<PixelBuffer>>,
        shared: Arc<Mutex<FreeRdpSharedState>>,
        initial_config: RdpConfig,
    ) {
        // Note: Qt/Wayland env vars are set per-process via Command::env()
        // in launch_freerdp() to avoid data races from std::env::set_var
        // in multi-threaded context (unsafe since Rust 1.66+).

        let mut current_config = Some(initial_config);

        loop {
            match cmd_rx.recv() {
                Ok(RdpCommand::Connect(config)) => {
                    lock_or_recover(&shared).state = FreeRdpThreadState::Connecting;
                    current_config = Some(config.clone());

                    match Self::launch_freerdp(&config, &shared) {
                        Ok(()) => {
                            lock_or_recover(&shared).state = FreeRdpThreadState::Connected;
                            let _ = evt_tx.send(RdpEvent::Connected);
                        }
                        Err(e) => {
                            let mut s = lock_or_recover(&shared);
                            s.fallback_triggered = true;
                            s.state = FreeRdpThreadState::Error;
                            drop(s);
                            let _ = evt_tx.send(RdpEvent::FallbackTriggered(e.to_string()));
                        }
                    }
                }
                Ok(RdpCommand::Disconnect) => {
                    let mut s = lock_or_recover(&shared);
                    s.cleanup_process();
                    s.state = FreeRdpThreadState::Idle;
                    drop(s);
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
                    tracing::debug!("[FreeRDP] Ctrl+Alt+Del requested");
                }
                Ok(RdpCommand::Shutdown) => {
                    let mut s = lock_or_recover(&shared);
                    s.state = FreeRdpThreadState::ShuttingDown;
                    s.cleanup_process();
                    break;
                }
                Err(_) => {
                    lock_or_recover(&shared).cleanup_process();
                    break;
                }
            }
        }
    }

    /// Launches FreeRDP with Qt error suppression
    ///
    /// Uses mutex poisoning recovery for safe process handle storage.
    fn launch_freerdp(
        config: &RdpConfig,
        shared: &Arc<Mutex<FreeRdpSharedState>>,
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
        // Do NOT set QT_QPA_PLATFORM — allow wlfreerdp to use native Wayland backend

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
            if !password.expose_secret().is_empty() {
                // Use /from-stdin to avoid exposing password in /proc/PID/cmdline
                cmd.arg("/from-stdin");
                cmd.stdin(Stdio::piped());
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
            Ok(mut child) => {
                // Write password via stdin when /from-stdin is used
                if let Some(ref password) = config.password {
                    if !password.expose_secret().is_empty() {
                        if let Some(mut stdin) = child.stdin.take() {
                            use std::io::Write;
                            let _ = writeln!(stdin, "{}", password.expose_secret());
                        }
                    }
                }
                lock_or_recover(shared).process = Some(child);
                Ok(())
            }
            Err(e) => Err(EmbeddedRdpError::FreeRdpInit(e.to_string())),
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
    ///
    /// Uses mutex poisoning recovery for safe state access.
    pub fn state(&self) -> FreeRdpThreadState {
        lock_or_recover(&self.shared).state
    }

    /// Returns whether fallback was triggered
    ///
    /// Uses mutex poisoning recovery for safe flag access.
    pub fn fallback_triggered(&self) -> bool {
        lock_or_recover(&self.shared).fallback_triggered
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
