//! Clipboard button handlers for the embedded RDP widget
//!
//! Contains setup for Copy, Paste, and Ctrl+Alt+Del toolbar buttons.

use gtk4::Button;
use gtk4::glib;
use gtk4::prelude::*;

use super::types::{RdpCommand, RdpConnectionState};

#[cfg(feature = "rdp-embedded")]
use rustconn_core::rdp_client::RdpClientCommand;

impl super::EmbeddedRdpWidget {
    /// Sets up the clipboard Copy/Paste button handlers
    pub(super) fn setup_clipboard_buttons(&self, copy_btn: &Button, paste_btn: &Button) {
        // Copy button - copy remote clipboard text to local clipboard
        {
            let state = self.state.clone();
            let is_embedded = self.is_embedded.clone();
            let remote_clipboard_text = self.remote_clipboard_text.clone();
            let drawing_area = self.drawing_area.clone();
            let status_label = self.status_label.clone();

            copy_btn.connect_clicked(move |_| {
                let current_state = *state.borrow();
                let embedded = *is_embedded.borrow();

                if current_state != RdpConnectionState::Connected || !embedded {
                    return;
                }

                // Check if we have remote clipboard text
                if let Some(ref text) = *remote_clipboard_text.borrow() {
                    let char_count = text.len();

                    // Copy to local clipboard
                    let display = drawing_area.display();
                    let clipboard = display.clipboard();
                    clipboard.set_text(text);

                    // Show feedback
                    status_label.set_text(&format!("Copied {char_count} chars"));
                    status_label.set_visible(true);
                    let status_hide = status_label.clone();
                    glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || {
                        status_hide.set_visible(false);
                    });
                } else {
                    status_label.set_text("No remote clipboard data");
                    status_label.set_visible(true);
                    let status_hide = status_label.clone();
                    glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || {
                        status_hide.set_visible(false);
                    });
                }
            });
        }

        // Paste button - send local clipboard text to remote
        {
            #[cfg(feature = "rdp-embedded")]
            let ironrdp_tx = self.ironrdp_command_tx.clone();
            let drawing_area = self.drawing_area.clone();
            let state = self.state.clone();
            let is_embedded = self.is_embedded.clone();
            #[cfg(feature = "rdp-embedded")]
            let is_ironrdp = self.is_ironrdp.clone();
            let status_label = self.status_label.clone();

            paste_btn.connect_clicked(move |_| {
                let current_state = *state.borrow();
                let embedded = *is_embedded.borrow();

                if current_state != RdpConnectionState::Connected || !embedded {
                    return;
                }

                // Get text from local clipboard and send to remote
                let display = drawing_area.display();
                let clipboard = display.clipboard();

                #[cfg(feature = "rdp-embedded")]
                let using_ironrdp = *is_ironrdp.borrow();
                #[cfg(feature = "rdp-embedded")]
                let tx = ironrdp_tx.clone();
                let status = status_label.clone();

                clipboard.read_text_async(
                    None::<&gtk4::gio::Cancellable>,
                    move |result: Result<Option<glib::GString>, glib::Error>| {
                        if let Ok(Some(text)) = result {
                            let char_count = text.len();

                            #[cfg(feature = "rdp-embedded")]
                            if using_ironrdp {
                                // Send clipboard text via IronRDP
                                if let Some(ref sender) = *tx.borrow() {
                                    let _ = sender
                                        .send(RdpClientCommand::ClipboardText(text.to_string()));
                                    // Show brief feedback
                                    status.set_text(&format!("Pasted {char_count} chars"));
                                    status.set_visible(true);
                                    // Hide after 2 seconds
                                    let status_hide = status.clone();
                                    glib::timeout_add_local_once(
                                        std::time::Duration::from_secs(2),
                                        move || {
                                            status_hide.set_visible(false);
                                        },
                                    );
                                }
                            }
                            // For FreeRDP, clipboard is handled by the external process
                        }
                    },
                );
            });
        }
    }

    /// Sets up the Ctrl+Alt+Del button handler
    pub(super) fn setup_ctrl_alt_del_button(&self, button: &Button) {
        #[cfg(feature = "rdp-embedded")]
        {
            let ironrdp_tx = self.ironrdp_command_tx.clone();
            let freerdp_thread = self.freerdp_thread.clone();
            let state = self.state.clone();
            let is_embedded = self.is_embedded.clone();
            let is_ironrdp = self.is_ironrdp.clone();

            button.connect_clicked(move |_| {
                let current_state = *state.borrow();
                let embedded = *is_embedded.borrow();
                let using_ironrdp = *is_ironrdp.borrow();

                if current_state != RdpConnectionState::Connected || !embedded {
                    return;
                }

                if using_ironrdp {
                    // Send via IronRDP
                    if let Some(ref tx) = *ironrdp_tx.borrow() {
                        let _ = tx.send(RdpClientCommand::SendCtrlAltDel);
                    }
                } else {
                    // Send via FreeRDP thread
                    if let Some(ref thread) = *freerdp_thread.borrow() {
                        let _ = thread.send_command(RdpCommand::SendCtrlAltDel);
                        tracing::debug!("Sent Ctrl+Alt+Del via FreeRDP");
                    }
                }
            });
        }

        #[cfg(not(feature = "rdp-embedded"))]
        {
            let freerdp_thread = self.freerdp_thread.clone();
            let state = self.state.clone();
            let is_embedded = self.is_embedded.clone();

            button.connect_clicked(move |_| {
                let current_state = *state.borrow();
                let embedded = *is_embedded.borrow();

                if current_state != RdpConnectionState::Connected || !embedded {
                    return;
                }

                if let Some(ref thread) = *freerdp_thread.borrow() {
                    let _ = thread.send_command(RdpCommand::SendCtrlAltDel);
                    tracing::debug!("Sent Ctrl+Alt+Del via FreeRDP");
                }
            });
        }
    }
}
