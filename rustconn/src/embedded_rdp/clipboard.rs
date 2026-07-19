//! Clipboard button handlers for the embedded RDP widget
//!
//! Contains setup for Copy, Paste, and Ctrl+Alt+Del toolbar buttons.

use gtk4::prelude::*;
use gtk4::{Button, Label, glib};
#[cfg(feature = "rdp-embedded")]
use rustconn_core::rdp_client::RdpClientCommand;

use super::types::{RdpCommand, RdpConnectionState};
use crate::i18n::{i18n, i18n_f};

/// Shows a brief status message that auto-hides after the given duration.
fn show_status_briefly(label: &Label, text: &str, duration_secs: u64) {
    label.set_text(text);
    label.set_visible(true);
    let hide = label.clone();
    glib::timeout_add_local_once(std::time::Duration::from_secs(duration_secs), move || {
        hide.set_visible(false);
    });
}

impl super::EmbeddedRdpWidget {
    /// Sets up the clipboard Copy/Paste button handlers
    pub(super) fn setup_clipboard_buttons(&self, copy_btn: &Button, paste_btn: &Button) {
        // Copy button - copy remote clipboard text to local clipboard
        {
            let state = self.state.clone();
            let is_embedded = self.is_embedded.clone();
            let remote_clipboard_text = self.remote_clipboard_text.clone();
            let container = self.container.clone();
            let status_label = self.status_label.clone();
            let suppressed = self.clipboard_sync_suppressed.clone();

            copy_btn.connect_clicked(move |_| {
                let current_state = *state.borrow();
                let embedded = *is_embedded.borrow();

                if current_state != RdpConnectionState::Connected || !embedded {
                    tracing::debug!(
                        protocol = "rdp",
                        ?current_state,
                        embedded,
                        "Copy button: not connected or not embedded"
                    );
                    return;
                }

                // Check if we have remote clipboard text
                if let Some(ref text) = *remote_clipboard_text.borrow() {
                    let char_count = text.len();

                    // Suppress the clipboard-changed handler so we don't
                    // send this text back to the server in a feedback loop.
                    *suppressed.borrow_mut() = true;

                    // Use the root widget's display for clipboard access —
                    // on Wayland the clipboard is tied to the focused surface,
                    // and the top-level window surface is the most reliable owner.
                    let clipboard = if let Some(root) = container.root()
                        && let Some(window) = root.downcast_ref::<gtk4::Window>()
                    {
                        gtk4::prelude::WidgetExt::display(window).clipboard()
                    } else {
                        container.display().clipboard()
                    };
                    clipboard.set_text(text);

                    // Re-enable sync after a short delay (GTK emits changed asynchronously)
                    let suppressed_restore = suppressed.clone();
                    glib::timeout_add_local_once(
                        std::time::Duration::from_millis(100),
                        move || {
                            *suppressed_restore.borrow_mut() = false;
                        },
                    );

                    tracing::debug!(
                        protocol = "rdp",
                        chars = char_count,
                        "Copy button: set local clipboard from remote"
                    );

                    // Show feedback
                    show_status_briefly(
                        &status_label,
                        &i18n_f("Copied {} chars", &[&char_count.to_string()]),
                        2,
                    );
                } else {
                    tracing::debug!(
                        protocol = "rdp",
                        "Copy button: no remote clipboard data available"
                    );
                    show_status_briefly(&status_label, &i18n("No remote clipboard data"), 2);
                }
            });
        }

        // Paste button - send local clipboard text to remote and simulate Ctrl+V
        {
            #[cfg(feature = "rdp-embedded")]
            let ironrdp_tx = self.ironrdp_command_tx.clone();
            let container = self.container.clone();
            let state = self.state.clone();
            let is_embedded = self.is_embedded.clone();
            #[cfg(feature = "rdp-embedded")]
            let is_ironrdp = self.is_ironrdp.clone();
            let status_label = self.status_label.clone();

            paste_btn.connect_clicked(move |_| {
                let current_state = *state.borrow();
                let embedded = *is_embedded.borrow();

                if current_state != RdpConnectionState::Connected || !embedded {
                    tracing::debug!(
                        protocol = "rdp",
                        ?current_state,
                        embedded,
                        "Paste button: not connected or not embedded"
                    );
                    return;
                }

                // Use the root widget's display for clipboard access —
                // on Wayland the clipboard is tied to the focused surface.
                let clipboard = if let Some(root) = container.root()
                    && let Some(window) = root.downcast_ref::<gtk4::Window>()
                {
                    gtk4::prelude::WidgetExt::display(window).clipboard()
                } else {
                    container.display().clipboard()
                };

                #[cfg(feature = "rdp-embedded")]
                let using_ironrdp = *is_ironrdp.borrow();
                #[cfg(feature = "rdp-embedded")]
                let tx = ironrdp_tx.clone();
                let status = status_label.clone();

                clipboard.read_text_async(
                    None::<&gtk4::gio::Cancellable>,
                    move |result: Result<Option<glib::GString>, glib::Error>| {
                        match result {
                            Ok(Some(text)) => {
                                let char_count = text.len();

                                #[cfg(feature = "rdp-embedded")]
                                if using_ironrdp {
                                    if let Some(ref sender) = *tx.borrow() {
                                        // Step 1: Update the server's clipboard via CLIPRDR
                                        let _ = sender.send(RdpClientCommand::ClipboardText(
                                            text.to_string(),
                                        ));

                                        // Step 2: After a short delay (let the server process
                                        // the format list + data request), send Ctrl+V to
                                        // actually paste into the active window.
                                        let tx_paste = tx.clone();
                                        glib::timeout_add_local_once(
                                            std::time::Duration::from_millis(150),
                                            move || {
                                                if let Some(ref sender) = *tx_paste.borrow() {
                                                    // Ctrl+V: Ctrl down (0x1D), V down (0x2F),
                                                    // V up, Ctrl up
                                                    let keys = vec![
                                                        (0x1D, true, false),  // Ctrl down
                                                        (0x2F, true, false),  // V down
                                                        (0x2F, false, false), // V up
                                                        (0x1D, false, false), // Ctrl up
                                                    ];
                                                    let _ = sender.send(
                                                        RdpClientCommand::SendKeySequence { keys },
                                                    );
                                                    tracing::debug!(
                                                        protocol = "rdp",
                                                        "Paste button: sent Ctrl+V to server"
                                                    );
                                                }
                                            },
                                        );

                                        tracing::debug!(
                                            protocol = "rdp",
                                            chars = char_count,
                                            "Paste button: sent local clipboard to server"
                                        );
                                        show_status_briefly(
                                            &status,
                                            &i18n_f("Pasted {} chars", &[&char_count.to_string()]),
                                            2,
                                        );
                                    } else {
                                        tracing::warn!(
                                            protocol = "rdp",
                                            "Paste button: IronRDP command channel not available"
                                        );
                                        show_status_briefly(
                                            &status,
                                            &i18n("Clipboard channel not ready"),
                                            2,
                                        );
                                    }
                                }
                                // For FreeRDP, clipboard is handled by the external process
                            }
                            Ok(None) => {
                                tracing::debug!(
                                    protocol = "rdp",
                                    "Paste button: local clipboard is empty"
                                );
                                show_status_briefly(&status, &i18n("Local clipboard is empty"), 2);
                            }
                            Err(e) => {
                                tracing::warn!(
                                    protocol = "rdp",
                                    error = %e,
                                    "Paste button: failed to read local clipboard"
                                );
                                show_status_briefly(&status, &i18n("Cannot read clipboard"), 2);
                            }
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
