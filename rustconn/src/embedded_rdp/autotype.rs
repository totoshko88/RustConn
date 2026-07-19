//! Autotype handlers for the embedded RDP widget
//!
//! Provides "Type Clipboard" and "Type Text" functionality that sends text
//! as individual Unicode keystroke events, bypassing clipboard restrictions
//! (GPO, Citrix policy, UAC dialogs, password fields that reject paste).
//!
//! Uses `TS_UNICODE_KEYBOARD_EVENT` PDU via IronRDP which is keyboard-layout
//! independent — works regardless of DE/US/other layout mismatches.

use gtk4::prelude::*;
use gtk4::{Button, Label, glib};
use libadwaita as adw;
use libadwaita::prelude::*;
#[cfg(feature = "rdp-embedded")]
use rustconn_core::rdp_client::RdpClientCommand;

use super::types::RdpConnectionState;
use crate::i18n::{i18n, i18n_f};

impl super::EmbeddedRdpWidget {
    /// Creates the "Type Clipboard" button for the toolbar.
    ///
    /// Reads the local clipboard and sends its contents as individual
    /// Unicode keystrokes to the remote RDP session.
    #[cfg(feature = "rdp-embedded")]
    pub(super) fn setup_autotype_clipboard_button(&self, button: &Button) {
        let ironrdp_tx = self.ironrdp_command_tx.clone();
        let container = self.container.clone();
        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();
        let is_ironrdp = self.is_ironrdp.clone();
        let status_label = self.status_label.clone();
        let config = self.config.clone();

        button.connect_clicked(move |_| {
            let current_state = *state.borrow();
            let embedded = *is_embedded.borrow();
            let using_ironrdp = *is_ironrdp.borrow();

            if current_state != RdpConnectionState::Connected || !embedded || !using_ironrdp {
                tracing::debug!(
                    protocol = "rdp",
                    ?current_state,
                    embedded,
                    using_ironrdp,
                    "Autotype clipboard: not available"
                );
                return;
            }

            // Read autotype timing from connection config
            let (delay_ms, initial_ms) = if let Some(ref cfg) = *config.borrow() {
                (cfg.autotype_delay_ms, cfg.autotype_initial_delay_ms)
            } else {
                (20, 0)
            };

            let clipboard = if let Some(root) = container.root()
                && let Some(window) = root.downcast_ref::<gtk4::Window>()
            {
                gtk4::prelude::WidgetExt::display(window).clipboard()
            } else {
                container.display().clipboard()
            };

            let tx = ironrdp_tx.clone();
            let status = status_label.clone();

            clipboard.read_text_async(
                None::<&gtk4::gio::Cancellable>,
                move |result: Result<Option<glib::GString>, glib::Error>| match result {
                    Ok(Some(text)) => {
                        let char_count = text.len();
                        if let Some(ref sender) = *tx.borrow() {
                            let _ = sender.send(RdpClientCommand::AutotypeText {
                                text: text.to_string(),
                                inter_char_delay_ms: delay_ms,
                                initial_delay_ms: initial_ms,
                            });
                            tracing::debug!(
                                protocol = "rdp",
                                chars = char_count,
                                delay_ms,
                                "Autotype clipboard: sending"
                            );
                            show_autotype_status(
                                &status,
                                &i18n_f("Typing {} chars...", &[&char_count.to_string()]),
                                3,
                            );
                        } else {
                            show_autotype_status(&status, &i18n("Autotype channel not ready"), 2);
                        }
                    }
                    Ok(None) => {
                        show_autotype_status(&status, &i18n("Local clipboard is empty"), 2);
                    }
                    Err(e) => {
                        tracing::warn!(
                            protocol = "rdp",
                            error = %e,
                            "Autotype clipboard: failed to read"
                        );
                        show_autotype_status(&status, &i18n("Cannot read clipboard"), 2);
                    }
                },
            );
        });
    }

    /// Creates the "Type Text" button for the toolbar.
    ///
    /// Opens a dialog where the user can type or paste text, then sends it
    /// as keystrokes. The text never touches the system clipboard — useful
    /// for passwords and sensitive strings.
    #[cfg(feature = "rdp-embedded")]
    pub(super) fn setup_autotype_dialog_button(&self, button: &Button) {
        let ironrdp_tx = self.ironrdp_command_tx.clone();
        let container = self.container.clone();
        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();
        let is_ironrdp = self.is_ironrdp.clone();
        let status_label = self.status_label.clone();
        let config = self.config.clone();

        button.connect_clicked(move |_| {
            let current_state = *state.borrow();
            let embedded = *is_embedded.borrow();
            let using_ironrdp = *is_ironrdp.borrow();

            if current_state != RdpConnectionState::Connected || !embedded || !using_ironrdp {
                return;
            }

            let (delay_ms, initial_ms) = if let Some(ref cfg) = *config.borrow() {
                (cfg.autotype_delay_ms, cfg.autotype_initial_delay_ms)
            } else {
                (20, 0)
            };

            let tx = ironrdp_tx.clone();
            let status = status_label.clone();

            // Build the autotype dialog as an adw::Dialog (libadwaita pattern:
            // ToolbarView + HeaderBar). On Wayland a raw modal gtk4::Window
            // renders as a separate window — adw::Dialog stays attached.
            let dialog = adw::Dialog::new();
            dialog.set_title(&i18n("Type Text into Remote Session"));
            dialog.set_content_width(420);
            dialog.set_content_height(240);

            let toolbar_view = adw::ToolbarView::new();
            let header = adw::HeaderBar::new();

            // Action buttons live in the header bar (HIG): cancel at the
            // start, the suggested primary action at the end.
            let cancel_btn = Button::with_label(&i18n("Cancel"));
            let type_btn = Button::with_label(&i18n("Type Now"));
            type_btn.add_css_class("suggested-action");
            header.pack_start(&cancel_btn);
            header.pack_end(&type_btn);
            toolbar_view.add_top_bar(&header);

            let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
            content.set_margin_start(12);
            content.set_margin_end(12);
            content.set_margin_top(12);
            content.set_margin_bottom(12);

            // Info label
            let info_label = Label::new(Some(&i18n(
                "Text entered here will be sent as keystrokes to the remote session. It will not be placed in the system clipboard.",
            )));
            info_label.set_wrap(true);
            info_label.set_xalign(0.0);
            info_label.add_css_class("dim-label");
            content.append(&info_label);

            // Text view in a scrolled window
            let scrolled = gtk4::ScrolledWindow::new();
            scrolled.set_vexpand(true);
            scrolled.set_min_content_height(80);

            let text_view = gtk4::TextView::new();
            text_view.set_wrap_mode(gtk4::WrapMode::WordChar);
            text_view.set_monospace(true);
            text_view.set_hexpand(true);
            text_view.set_vexpand(true);
            scrolled.set_child(Some(&text_view));
            content.append(&scrolled);

            // Password visibility toggle
            let password_check = gtk4::CheckButton::with_label(&i18n("Hide text (password mode)"));
            password_check.connect_toggled({
                let tv = text_view.clone();
                move |check| {
                    if check.is_active() {
                        tv.add_css_class("password-entry");
                        tv.set_input_purpose(gtk4::InputPurpose::Password);
                    } else {
                        tv.remove_css_class("password-entry");
                        tv.set_input_purpose(gtk4::InputPurpose::FreeForm);
                    }
                }
            });
            content.append(&password_check);

            toolbar_view.set_content(Some(&content));
            dialog.set_child(Some(&toolbar_view));

            // Cancel closes dialog
            let dialog_cancel = dialog.clone();
            cancel_btn.connect_clicked(move |_| {
                dialog_cancel.close();
            });

            // Type Now sends text and closes
            let dialog_type = dialog.clone();
            type_btn.connect_clicked(move |_| {
                let buffer = text_view.buffer();
                let text = buffer.text(
                    &buffer.start_iter(),
                    &buffer.end_iter(),
                    false,
                );
                let text_str = text.to_string();

                if text_str.is_empty() {
                    show_autotype_status(&status, &i18n("No text to type"), 2);
                    dialog_type.close();
                    return;
                }

                let char_count = text_str.len();

                if let Some(ref sender) = *tx.borrow() {
                    let _ = sender.send(RdpClientCommand::AutotypeText {
                        text: text_str,
                        inter_char_delay_ms: delay_ms,
                        initial_delay_ms: initial_ms,
                    });
                    tracing::debug!(
                        protocol = "rdp",
                        chars = char_count,
                        delay_ms,
                        "Autotype dialog: sending"
                    );
                    show_autotype_status(
                        &status,
                        &i18n_f("Typing {} chars...", &[&char_count.to_string()]),
                        3,
                    );
                }

                // Clear the buffer before closing (zeroize sensitive text)
                buffer.set_text("");
                dialog_type.close();
            });

            dialog.present(Some(&container));
        });
    }
}

/// Shows a brief status message that auto-hides after the given duration.
fn show_autotype_status(label: &Label, text: &str, duration_secs: u64) {
    label.set_text(text);
    label.set_visible(true);
    let hide = label.clone();
    glib::timeout_add_local_once(std::time::Duration::from_secs(duration_secs), move || {
        hide.set_visible(false);
    });
}
