//! RDP client implementation using `IronRDP`
//!
//! This module provides the async RDP client that connects to RDP servers
//! and produces framebuffer events for the GUI to render.
//!
//! # Architecture
//!
//! The RDP client follows the same pattern as the `VncClient`:
//! - Runs in a background thread with its own Tokio runtime
//! - Communicates via `std::sync::mpsc` channels for cross-runtime compatibility
//! - Produces events for framebuffer updates, resolution changes, etc.
//! - Accepts commands for keyboard/mouse input, disconnect, etc.

// Allow clippy warnings for this file - RDP protocol uses various integer sizes
// #![allow(clippy::cast_possible_truncation)]
// #![allow(clippy::cast_sign_loss)]
// #![allow(clippy::missing_panics_doc)]
// #![allow(clippy::default_trait_access)]

use super::{RdpClientCommand, RdpClientConfig, RdpClientError, RdpClientEvent};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

mod commands;
mod connection;
mod session;

/// Sender for commands to the RDP client (thread-safe, non-async)
pub type RdpCommandSender = std::sync::mpsc::Sender<RdpClientCommand>;

/// Receiver for events from the RDP client (thread-safe, non-async)
pub type RdpEventReceiver = std::sync::mpsc::Receiver<RdpClientEvent>;

/// RDP client state for tracking connection lifecycle
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RdpClientState {
    /// Client is disconnected from the server
    #[default]
    Disconnected,
    /// Client is attempting to connect
    Connecting,
    /// Client is connected and active
    Connected,
    /// Client is in the process of disconnecting
    Disconnecting,
    /// Client encountered an error
    Error,
}

/// RDP client handle for managing connections
pub struct RdpClient {
    command_tx: Option<std::sync::mpsc::Sender<RdpClientCommand>>,
    event_rx: Option<std::sync::mpsc::Receiver<RdpClientEvent>>,
    connected: Arc<AtomicBool>,
    config: RdpClientConfig,
    thread_handle: Option<JoinHandle<()>>,
    shutdown_signal: Arc<AtomicBool>,
}

impl RdpClient {
    /// Creates a new RDP client with the given configuration
    #[must_use]
    pub fn new(config: RdpClientConfig) -> Self {
        Self {
            command_tx: None,
            event_rx: None,
            connected: Arc::new(AtomicBool::new(false)),
            config,
            thread_handle: None,
            shutdown_signal: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Connects to the RDP server
    ///
    /// # Errors
    ///
    /// Returns `RdpClientError::AlreadyConnected` if already connected.
    pub fn connect(&mut self) -> Result<(), RdpClientError> {
        if self.connected.load(Ordering::SeqCst) {
            return Err(RdpClientError::AlreadyConnected);
        }

        self.shutdown_signal.store(false, Ordering::SeqCst);

        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let (command_tx, command_rx) = std::sync::mpsc::channel();

        self.event_rx = Some(event_rx);
        self.command_tx = Some(command_tx);

        let config = self.config.clone();
        let connected = self.connected.clone();
        let shutdown_signal = self.shutdown_signal.clone();

        self.connected.store(true, Ordering::SeqCst);

        let handle = std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = event_tx.send(RdpClientEvent::Error(format!(
                        "Failed to create Tokio runtime: {e}"
                    )));
                    connected.store(false, Ordering::SeqCst);
                    return;
                }
            };

            rt.block_on(async move {
                let result =
                    run_rdp_client(config, event_tx.clone(), command_rx, shutdown_signal).await;
                connected.store(false, Ordering::SeqCst);

                if let Err(e) = result {
                    let _ = event_tx.send(RdpClientEvent::Error(e.to_string()));
                }
                let _ = event_tx.send(RdpClientEvent::Disconnected);
            });
        });

        self.thread_handle = Some(handle);
        Ok(())
    }

    /// Tries to receive an event from the RDP client without blocking
    #[must_use]
    pub fn try_recv_event(&self) -> Option<RdpClientEvent> {
        self.event_rx.as_ref()?.try_recv().ok()
    }

    /// Sends a command to the RDP client.
    ///
    /// # Errors
    ///
    /// Returns `RdpClientError::NotConnected` if not connected,
    /// or `RdpClientError::ChannelError` if the channel is closed.
    pub fn send_command(&self, command: RdpClientCommand) -> Result<(), RdpClientError> {
        let tx = self
            .command_tx
            .as_ref()
            .ok_or(RdpClientError::NotConnected)?;
        tx.send(command)
            .map_err(|e| RdpClientError::ChannelError(e.to_string()))
    }

    /// Sends a key event to the RDP server.
    ///
    /// # Errors
    ///
    /// Returns error if not connected or channel is closed.
    pub fn send_key(&self, scancode: u16, pressed: bool) -> Result<(), RdpClientError> {
        self.send_command(RdpClientCommand::KeyEvent {
            scancode,
            pressed,
            extended: false,
        })
    }

    /// Sends a pointer/mouse event to the RDP server.
    ///
    /// # Errors
    ///
    /// Returns error if not connected or channel is closed.
    pub fn send_pointer(&self, x: u16, y: u16, buttons: u8) -> Result<(), RdpClientError> {
        self.send_command(RdpClientCommand::PointerEvent { x, y, buttons })
    }

    /// Sends Ctrl+Alt+Del key sequence to the RDP server.
    ///
    /// # Errors
    ///
    /// Returns error if not connected or channel is closed.
    pub fn send_ctrl_alt_del(&self) -> Result<(), RdpClientError> {
        self.send_command(RdpClientCommand::SendCtrlAltDel)
    }

    /// Requests a desktop size change.
    ///
    /// # Errors
    ///
    /// Returns error if not connected or channel is closed.
    pub fn set_desktop_size(&self, width: u16, height: u16) -> Result<(), RdpClientError> {
        self.send_command(RdpClientCommand::SetDesktopSize { width, height })
    }

    /// Disconnects from the RDP server and cleans up resources
    pub fn disconnect(&mut self) {
        self.shutdown_signal.store(true, Ordering::SeqCst);
        if let Some(tx) = &self.command_tx {
            let _ = tx.send(RdpClientCommand::Disconnect);
        }
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
        self.command_tx = None;
        self.event_rx = None;
        self.connected.store(false, Ordering::SeqCst);
    }

    /// Returns whether all resources have been cleaned up
    #[must_use]
    pub fn is_cleaned_up(&self) -> bool {
        self.command_tx.is_none()
            && self.event_rx.is_none()
            && self.thread_handle.is_none()
            && !self.connected.load(Ordering::SeqCst)
    }

    /// Returns whether the client is currently connected
    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    /// Returns a reference to the client configuration
    #[must_use]
    pub const fn config(&self) -> &RdpClientConfig {
        &self.config
    }

    /// Returns a reference to the event receiver channel
    #[must_use]
    pub const fn event_receiver(&self) -> Option<&std::sync::mpsc::Receiver<RdpClientEvent>> {
        self.event_rx.as_ref()
    }

    /// Returns a clone of the command sender channel
    #[must_use]
    pub fn command_sender(&self) -> Option<std::sync::mpsc::Sender<RdpClientCommand>> {
        self.command_tx.clone()
    }
}

impl Drop for RdpClient {
    fn drop(&mut self) {
        self.disconnect();
    }
}

/// Runs the RDP client protocol loop using `IronRDP`
#[allow(clippy::future_not_send)]
async fn run_rdp_client(
    config: RdpClientConfig,
    event_tx: std::sync::mpsc::Sender<RdpClientEvent>,
    command_rx: std::sync::mpsc::Receiver<RdpClientCommand>,
    shutdown_signal: Arc<AtomicBool>,
) -> Result<(), RdpClientError> {
    // Phase 1-3: Establish connection
    let (framed, connection_result) =
        connection::establish_connection(&config, event_tx.clone()).await?;

    // Send connected event
    let _ = event_tx.send(RdpClientEvent::Connected {
        width: connection_result.desktop_size.width,
        height: connection_result.desktop_size.height,
    });

    // Phase 4: Active session loop
    session::run_active_session(
        framed,
        connection_result,
        event_tx,
        command_rx,
        shutdown_signal,
    )
    .await
}
