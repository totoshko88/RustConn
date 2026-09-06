//! SSH Agent settings tab using libadwaita components
//!
//! SSH agent status and key loading is performed asynchronously to avoid blocking the UI.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, Label, ListBox, Orientation, glib};
use libadwaita as adw;
use rustconn_core::sftp::{SocketPathValidation, validate_socket_path};
use rustconn_core::ssh_agent::SshAgentManager;

use crate::i18n::{i18n, i18n_f};

/// Creates the SSH Agent settings page using AdwPreferencesPage
pub fn create_ssh_agent_page() -> (
    adw::PreferencesPage,
    Label,         // status
    Label,         // socket path
    Button,        // start agent
    ListBox,       // loaded keys
    Button,        // add key
    Button,        // refresh
    ListBox,       // available_keys_list
    adw::EntryRow, // custom_socket_entry
) {
    let page = adw::PreferencesPage::builder()
        .title(i18n("SSH Agent"))
        .icon_name("network-server-symbolic")
        .build();

    // === Agent Status Group ===
    let status_group = adw::PreferencesGroup::builder()
        .title(i18n("Agent Status"))
        .build();

    let ssh_agent_status_label = Label::builder()
        .label(i18n("Checking…"))
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        .build();
    let status_row = adw::ActionRow::builder().title(i18n("Status")).build();
    status_row.add_suffix(&ssh_agent_status_label);
    status_group.add(&status_row);

    let ssh_agent_socket_label = Label::builder()
        .label("")
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        .css_classes(["dim-label"])
        .selectable(true)
        .ellipsize(gtk4::pango::EllipsizeMode::Middle)
        .max_width_chars(40)
        .build();
    let socket_row = adw::ActionRow::builder().title(i18n("Socket")).build();
    socket_row.add_suffix(&ssh_agent_socket_label);
    status_group.add(&socket_row);

    // Control buttons row
    let ssh_agent_start_button = Button::builder()
        .label(i18n("Start Agent"))
        .valign(gtk4::Align::Center)
        .build();
    let ssh_agent_refresh_button = Button::builder()
        .icon_name("view-refresh-symbolic")
        .valign(gtk4::Align::Center)
        .tooltip_text(i18n("Refresh status"))
        .build();
    ssh_agent_refresh_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Refresh SSH agent status",
    ))]);

    let buttons_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .valign(gtk4::Align::Center)
        .build();
    buttons_box.append(&ssh_agent_start_button);
    buttons_box.append(&ssh_agent_refresh_button);

    let control_row = adw::ActionRow::builder().title(i18n("Controls")).build();
    control_row.add_suffix(&buttons_box);
    status_group.add(&control_row);

    // Custom SSH agent socket path entry
    let custom_socket_entry = adw::EntryRow::builder()
        .title(i18n("Custom SSH Agent Socket Path"))
        .build();
    custom_socket_entry.set_show_apply_button(false);
    // Subtitle via tooltip since EntryRow doesn't have .set_subtitle()
    custom_socket_entry.set_tooltip_text(Some(&i18n(
        "Overrides auto-detected SSH_AUTH_SOCK for all connections",
    )));
    // Three lines were removed here: `set_text("")` on a row that is already
    // empty, then a clone of the same row and `set_text("")` again. Both were
    // commented as setting a placeholder, which neither does — and the row is
    // filled from settings by `load_settings` anyway.

    // Real-time validation feedback (Task 4.2)
    custom_socket_entry.connect_changed(|entry| {
        let text = entry.text();
        let path = text.as_str();
        // Remove previous validation classes
        entry.remove_css_class("success");
        entry.remove_css_class("warning");
        entry.remove_css_class("error");
        match validate_socket_path(path) {
            SocketPathValidation::Empty => {}
            SocketPathValidation::Valid => {
                entry.add_css_class("success");
            }
            SocketPathValidation::NotFound => {
                entry.add_css_class("warning");
            }
            SocketPathValidation::NotAbsolute => {
                entry.add_css_class("error");
            }
        }
    });

    status_group.add(&custom_socket_entry);

    page.add(&status_group);

    // === Loaded Keys Group ===
    let keys_group = adw::PreferencesGroup::builder()
        .title(i18n("Loaded Keys"))
        .description(i18n("Keys currently loaded in the SSH agent"))
        .build();

    let ssh_agent_keys_list = ListBox::builder()
        .selection_mode(gtk4::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    keys_group.add(&ssh_agent_keys_list);

    // Add Key button
    let ssh_agent_add_key_button = Button::builder()
        .label(i18n("Add Key"))
        .valign(gtk4::Align::Center)
        .build();
    let add_key_row = adw::ActionRow::builder()
        .title(i18n("Add SSH Key"))
        .subtitle(i18n("Load a key from file"))
        .activatable(true)
        .build();
    add_key_row.add_suffix(&ssh_agent_add_key_button);
    add_key_row.set_activatable_widget(Some(&ssh_agent_add_key_button));
    keys_group.add(&add_key_row);

    page.add(&keys_group);

    // === Available Key Files Group ===
    let available_group = adw::PreferencesGroup::builder()
        .title(i18n("Available Key Files"))
        .description(i18n("Key files found in ~/.ssh/"))
        .build();

    let available_keys_list = ListBox::builder()
        .selection_mode(gtk4::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();

    available_group.add(&available_keys_list);
    page.add(&available_group);

    (
        page,
        ssh_agent_status_label,
        ssh_agent_socket_label,
        ssh_agent_start_button,
        ssh_agent_keys_list,
        ssh_agent_add_key_button,
        ssh_agent_refresh_button,
        available_keys_list,
        custom_socket_entry,
    )
}

/// Loads SSH agent settings into UI controls asynchronously
pub fn load_ssh_agent_settings(
    ssh_agent_status_label: &Label,
    ssh_agent_socket_label: &Label,
    ssh_agent_keys_list: &ListBox,
    ssh_agent_manager: &Rc<RefCell<SshAgentManager>>,
) {
    // Show loading state immediately
    ssh_agent_status_label.set_text(&i18n("Checking…"));
    ssh_agent_status_label.remove_css_class("error");
    ssh_agent_status_label.remove_css_class("success");
    ssh_agent_status_label.add_css_class("dim-label");
    ssh_agent_socket_label.set_text("...");

    // Clear existing keys and show loading
    while let Some(child) = ssh_agent_keys_list.first_child() {
        ssh_agent_keys_list.remove(&child);
    }
    let loading_row = adw::ActionRow::builder()
        .title(i18n("Loading keys…"))
        .build();
    let spinner = crate::spinner::new();
    loading_row.add_prefix(&spinner);
    ssh_agent_keys_list.append(&loading_row);

    // Clone for async closure
    let status_label = ssh_agent_status_label.clone();
    let socket_label = ssh_agent_socket_label.clone();
    let keys_list = ssh_agent_keys_list.clone();
    let manager = ssh_agent_manager.clone();

    // Clone socket path before spawning thread (Rc<RefCell<_>> is not Send)
    let socket_path_clone = {
        let mgr = manager.borrow();
        mgr.socket_path().map(String::from)
    };

    // Run status check on a real OS thread so the GTK main loop stays idle
    // and can render frames while the check runs in the background.
    // GTK widgets are not Send, so we use a channel to pass results back.
    let socket_for_bg = socket_path_clone.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let bg_mgr = SshAgentManager::new(socket_for_bg);
        let status = bg_mgr.get_status();
        let _ = tx.send(status);
    });

    // Poll the channel from the main thread; GTK widgets stay here.
    glib::idle_add_local(move || match rx.try_recv() {
        Ok(status) => {
            if let Some(ref socket_path) = socket_path_clone {
                socket_label.set_text(socket_path);
            } else {
                socket_label.set_text(&i18n("Not available"));
            }

            // Clear loading row
            while let Some(child) = keys_list.first_child() {
                keys_list.remove(&child);
            }

            if let Ok(agent_status) = status {
                let status_text = if agent_status.running {
                    i18n("Running")
                } else {
                    i18n("Not running")
                };
                status_label.set_text(&status_text);
                status_label.remove_css_class("error");
                status_label.remove_css_class("dim-label");

                if agent_status.running {
                    status_label.add_css_class("success");

                    if agent_status.keys.is_empty() {
                        let empty_row = adw::ActionRow::builder()
                            .title(i18n("No keys loaded"))
                            .subtitle(i18n("Add keys using ssh-add or the button above"))
                            .build();
                        keys_list.append(&empty_row);
                    } else {
                        for key in &agent_status.keys {
                            let key_row = create_loaded_key_row(
                                key,
                                &manager,
                                &keys_list,
                                &status_label,
                                &socket_label,
                            );
                            keys_list.append(&key_row);
                        }
                    }
                } else {
                    status_label.add_css_class("dim-label");
                    let empty_row = adw::ActionRow::builder()
                        .title(i18n("Agent not running"))
                        .subtitle(i18n("Start the agent to manage keys"))
                        .build();
                    keys_list.append(&empty_row);
                }
            } else {
                status_label.set_text(&i18n("Error"));
                status_label.remove_css_class("dim-label");
                status_label.add_css_class("error");

                let empty_row = adw::ActionRow::builder()
                    .title(i18n("Agent not running"))
                    .subtitle(i18n("Start the agent to manage keys"))
                    .build();
                keys_list.append(&empty_row);
            }
            glib::ControlFlow::Break
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
    });
}

/// Populates the available keys list with load buttons asynchronously
pub fn populate_available_keys_list(
    available_keys_list: &ListBox,
    ssh_agent_manager: &Rc<RefCell<SshAgentManager>>,
    ssh_agent_keys_list: &ListBox,
    ssh_agent_status_label: &Label,
    ssh_agent_socket_label: &Label,
) {
    // Clear existing items and show loading
    while let Some(child) = available_keys_list.first_child() {
        available_keys_list.remove(&child);
    }
    let loading_row = adw::ActionRow::builder()
        .title(i18n("Scanning ~/.ssh/…"))
        .build();
    let spinner = crate::spinner::new();
    loading_row.add_prefix(&spinner);
    available_keys_list.append(&loading_row);

    // Clone for async closure
    let keys_list = available_keys_list.clone();
    let manager = ssh_agent_manager.clone();
    let agent_keys_list = ssh_agent_keys_list.clone();
    let status_label = ssh_agent_status_label.clone();
    let socket_label = ssh_agent_socket_label.clone();

    glib::spawn_future_local(async move {
        // List key files (this is a quick filesystem operation but still async for consistency)
        let key_files_result = SshAgentManager::list_key_files();

        // Clear loading row
        while let Some(child) = keys_list.first_child() {
            keys_list.remove(&child);
        }

        match key_files_result {
            Ok(key_files) if key_files.is_empty() => {
                let empty_row = adw::ActionRow::builder()
                    .title(i18n("No SSH key files found"))
                    .subtitle(i18n("Generate keys with ssh-keygen"))
                    .build();
                keys_list.append(&empty_row);
            }
            Ok(key_files) => {
                for key_file in key_files {
                    let key_name = key_file
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("Unknown")
                        .to_string();
                    let key_path_str = key_file.display().to_string();

                    let key_row = adw::ActionRow::builder()
                        .title(&key_name)
                        .subtitle(&key_path_str)
                        .build();

                    let load_button = Button::builder()
                        .icon_name("list-add-symbolic")
                        .valign(gtk4::Align::Center)
                        .tooltip_text(i18n("Load this key"))
                        .build();
                    load_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
                        "Load SSH key into agent",
                    ))]);

                    // Connect load button handler
                    let manager_clone = manager.clone();
                    let keys_list_clone = agent_keys_list.clone();
                    let status_label_clone = status_label.clone();
                    let socket_label_clone = socket_label.clone();
                    let key_path = key_file.clone();

                    load_button.connect_clicked(move |button| {
                        add_key_with_passphrase_dialog(
                            button,
                            &key_path,
                            &manager_clone,
                            &keys_list_clone,
                            &status_label_clone,
                            &socket_label_clone,
                        );
                    });

                    key_row.add_suffix(&load_button);
                    // Row activation loads the key (the only action on the
                    // row); removal stays click-only on its own button.
                    key_row.set_activatable_widget(Some(&load_button));
                    keys_list.append(&key_row);
                }
            }
            Err(_) => {
                let error_row = adw::ActionRow::builder()
                    .title(i18n("Failed to scan ~/.ssh/ directory"))
                    .build();
                error_row.add_css_class("error");
                keys_list.append(&error_row);
            }
        }
    });
}

/// Shows a passphrase dialog and adds the key to the agent
fn add_key_with_passphrase_dialog(
    button: &Button,
    key_path: &Path,
    ssh_agent_manager: &Rc<RefCell<SshAgentManager>>,
    ssh_agent_keys_list: &ListBox,
    ssh_agent_status_label: &Label,
    ssh_agent_socket_label: &Label,
) {
    // Get the window for the dialog
    let Some(root) = button.root() else {
        tracing::error!("Cannot get root window for passphrase dialog");
        return;
    };
    let Some(parent_window) = root.downcast_ref::<gtk4::Window>() else {
        tracing::error!("Root is not a Window");
        return;
    };

    let key_name = key_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("key")
        .to_string();

    // Create passphrase dialog using adw::Dialog.
    //
    // Width only, no content_height: the body is a label and one entry, so the
    // natural height is the right one and pinning it just adds dead space. The
    // 180 that used to be here was sized for a body that also held the action
    // button, which now lives in the header. `password.rs` does the same for the
    // same reason.
    let dialog = adw::Dialog::builder()
        .title(i18n_f("Add Key: {}", &[&key_name]))
        .content_width(400)
        .build();

    let toolbar_view = adw::ToolbarView::new();

    // Cancel at the start, the action at the end, both in the header — the same
    // shape as `portable_passphrase_change` and `credential_transfer`, the other
    // two dialogs that take input and hide the title buttons. This one used to
    // hide them and put its only button in the body, which left no visible way
    // out: Escape and click-outside did close it, but nothing said so.
    let cancel_button = Button::with_label(&i18n("Cancel"));

    let header = adw::HeaderBar::builder()
        .show_end_title_buttons(false)
        .show_start_title_buttons(false)
        .build();

    let content = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let body_label = Label::builder()
        .label(i18n(
            "Enter passphrase (leave empty if key has no passphrase)",
        ))
        .wrap(true)
        .halign(gtk4::Align::Start)
        .build();

    let passphrase_entry = gtk4::PasswordEntry::builder()
        .placeholder_text(i18n("Passphrase (optional)"))
        .show_peek_icon(true)
        .hexpand(true)
        .build();

    let add_button = Button::builder()
        .label(i18n("Add Key"))
        .css_classes(["suggested-action"])
        .build();

    header.pack_start(&cancel_button);
    header.pack_end(&add_button);

    content.append(&body_label);
    content.append(&passphrase_entry);

    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&content));
    dialog.set_child(Some(&toolbar_view));

    // Cancel closes without touching the agent. `can-close` is left on, so
    // Escape and click-outside already did this; the button is what makes it
    // discoverable.
    let dialog_for_cancel = dialog.clone();
    cancel_button.connect_clicked(move |_| {
        dialog_for_cancel.close();
    });

    // Enter in the passphrase field submits, as in the connection password
    // dialog. Registered before the entry is moved into the handler below.
    let add_for_activate = add_button.clone();
    passphrase_entry.connect_activate(move |_| {
        add_for_activate.emit_clicked();
    });

    // Connect add button
    let manager_clone = ssh_agent_manager.clone();
    let keys_list_clone = ssh_agent_keys_list.clone();
    let status_label_clone = ssh_agent_status_label.clone();
    let socket_label_clone = ssh_agent_socket_label.clone();
    let key_path_clone = key_path.to_path_buf();
    let dialog_clone2 = dialog.clone();

    add_button.connect_clicked(move |_| {
        let passphrase_text = passphrase_entry.text();
        let passphrase = if passphrase_text.is_empty() {
            None
        } else {
            Some(secrecy::SecretString::from(passphrase_text.to_string()))
        };

        let manager = manager_clone.borrow();
        match manager.add_key(&key_path_clone, passphrase.as_ref()) {
            Ok(()) => {
                tracing::info!("Key added successfully: {}", key_path_clone.display());
                dialog_clone2.close();
                // Refresh the keys list
                drop(manager);
                load_ssh_agent_settings(
                    &status_label_clone,
                    &socket_label_clone,
                    &keys_list_clone,
                    &manager_clone,
                );
            }
            Err(e) => {
                tracing::error!("Failed to add key: {e}");
                dialog_clone2.close();
                // Show error feedback so the user knows the key wasn't added.
                if let Some(root) = status_label_clone.root()
                    && let Some(window) = root.downcast_ref::<gtk4::Window>()
                {
                    crate::toast::show_toast_on_window(
                        window,
                        &i18n_f("Failed to add key: {}", &[&e.to_string()]),
                        crate::toast::ToastType::Error,
                    );
                }
            }
        }
    });

    dialog.present(Some(parent_window));
}

thread_local! {
    /// The SSH key file chooser request currently in flight, if any.
    ///
    /// Held so a second **Add Key** click can cancel the first request instead of
    /// stacking a second chooser or requiring the button to be disabled. A
    /// `thread_local` because everything here is GTK — main thread only by
    /// construction, so there is nothing to synchronise — and because the chooser
    /// is opened by a free function that keeps no state of its own.
    static IN_FLIGHT_KEY_CHOOSER: RefCell<Option<gtk4::gio::Cancellable>> =
        const { RefCell::new(None) };
}

/// Shows a file chooser dialog to add a key from any location
pub fn show_add_key_file_chooser(
    button: &Button,
    ssh_agent_manager: &Rc<RefCell<SshAgentManager>>,
    ssh_agent_keys_list: &ListBox,
    ssh_agent_status_label: &Label,
    ssh_agent_socket_label: &Label,
) {
    // Both of these are programming-error paths rather than anything a user can
    // cause, but they are also two more ways this button can appear to do
    // nothing, so they say which one happened.
    //
    // Neither returns early past a disabled button, which used to matter: the
    // button was made insensitive before these checks in an earlier draft, so a
    // failure here left it dead. It stays live throughout now — see the
    // superseding comment below.
    let Some(root) = button.root() else {
        tracing::error!("Add Key: button has no root, cannot parent the file chooser");
        return;
    };
    let Some(window) = root.downcast_ref::<gtk4::Window>() else {
        tracing::error!(
            root = %root.type_(),
            "Add Key: root is not a GtkWindow, cannot parent the file chooser"
        );
        return;
    };

    let file_dialog = gtk4::FileDialog::builder()
        .title(i18n("Select SSH Key File"))
        .modal(true)
        .build();

    // Filters, matching the KeePass choosers on this same page. Private keys have
    // no reliable extension, so the pattern list is a convenience and "All Files"
    // has to stay reachable.
    let key_filter = gtk4::FileFilter::new();
    key_filter.set_name(Some(&i18n("SSH Keys")));
    for pattern in ["*.pem", "*.key", "id_*", "*_rsa", "*_ed25519", "*_ecdsa"] {
        key_filter.add_pattern(pattern);
    }
    let all_filter = gtk4::FileFilter::new();
    all_filter.set_name(Some(&i18n("All Files")));
    all_filter.add_pattern("*");
    let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
    filters.append(&key_filter);
    filters.append(&all_filter);
    file_dialog.set_filters(Some(&filters));
    file_dialog.set_default_filter(Some(&key_filter));

    // No `set_initial_folder`: keys inside ~/.ssh are already listed under
    // Available Key Files and can be added from there, so the chooser is for keys
    // kept somewhere else — which is where defaulting into ~/.ssh gets in the way.
    // GTK reopens wherever it was last anyway.

    let manager_clone = ssh_agent_manager.clone();
    let keys_list_clone = ssh_agent_keys_list.clone();
    let status_label_clone = ssh_agent_status_label.clone();
    let socket_label_clone = ssh_agent_socket_label.clone();
    let button_clone = button.clone();
    let window_clone = window.clone();

    // One chooser at a time, by superseding rather than by disabling the button.
    //
    // Nothing used to stop a second click stacking a second chooser, and clicking
    // again is the natural response to a chooser that is slow to appear. The first
    // attempt at that was `button.set_sensitive(false)` with the button re-enabled
    // in the callback — which is wrong twice over. The callback fires when the user
    // picks a file or closes the chooser, so it can legitimately be minutes away
    // while they browse, and there is no signal for "the chooser appeared" to time
    // out against instead. Worse, `shell-environment.md` documents an environment
    // where the callback never fires at all: there the button would have stayed
    // dead for the rest of the session, with no message — a worse version of the
    // "Add Key does nothing" report this whole path exists to fix.
    //
    // So the button stays live and a new click cancels the request in flight. A
    // cancelled request arrives at the callback below and is recognised there.
    let cancellable = gtk4::gio::Cancellable::new();
    IN_FLIGHT_KEY_CHOOSER.with(|slot| {
        if let Some(previous) = slot.borrow_mut().replace(cancellable.clone()) {
            tracing::debug!("Superseding an SSH key file chooser that was still open");
            previous.cancel();
        }
    });

    tracing::debug!("Opening the SSH key file chooser");

    file_dialog.open(Some(window), Some(&cancellable), move |result| {
        // Cancellation means a later click took over, so the slot now holds that
        // request and must not be cleared. Every other outcome is terminal for
        // this one.
        let superseded = result
            .as_ref()
            .err()
            .is_some_and(|e| e.matches(gtk4::gio::IOErrorEnum::Cancelled));
        if superseded {
            tracing::debug!("Key file chooser was superseded by a later Add Key click");
            return;
        }
        IN_FLIGHT_KEY_CHOOSER.with(|slot| {
            slot.borrow_mut().take();
        });

        match result {
            Ok(file) => {
                let Some(path) = file.path() else {
                    // A chooser result with no local path — a virtual location
                    // such as a remote mount. ssh-add needs a real file.
                    tracing::warn!("Chosen key file has no local path");
                    crate::alert::show_error(
                        &window_clone,
                        &i18n("Cannot Use That File"),
                        &i18n(
                            "That location has no local file path. Copy the key to a local folder first.",
                        ),
                    );
                    return;
                };
                tracing::debug!(path = %path.display(), "Key file chosen");
                add_key_with_passphrase_dialog(
                    &button_clone,
                    &path,
                    &manager_clone,
                    &keys_list_clone,
                    &status_label_clone,
                    &socket_label_clone,
                );
            }
            // Closing the chooser is an Err too, and the only one that means
            // nothing went wrong.
            Err(e) if e.matches(gtk4::DialogError::Dismissed) => {
                tracing::debug!("Key file chooser dismissed");
            }
            Err(e) => {
                // This arm did not exist while the result was matched with
                // `if let Ok(...)`, which collapsed a dismissal, a real failure
                // and a callback that never runs into one silent outcome. Telling
                // them apart is what identified the "Add Key does nothing" report
                // as the environment rather than this code — see the portal note
                // in steering `shell-environment.md`.
                tracing::warn!(error = %e, "SSH key file chooser failed to open");
                crate::alert::show_error(
                    &window_clone,
                    &i18n("Could Not Open the File Chooser"),
                    &i18n_f(
                        "{}\n\nKeys already in ~/.ssh are listed under Available Key Files, and can be added from there without the file chooser.",
                        &[&e.to_string()],
                    ),
                );
            }
        }
    });
}

fn create_loaded_key_row(
    key: &rustconn_core::ssh_agent::AgentKey,
    ssh_agent_manager: &Rc<RefCell<SshAgentManager>>,
    ssh_agent_keys_list: &ListBox,
    ssh_agent_status_label: &Label,
    ssh_agent_socket_label: &Label,
) -> adw::ActionRow {
    let title = format!("{} ({} bits)", key.key_type, key.bits);
    let subtitle = if key.comment.is_empty() {
        format!("SHA256:{}", key.fingerprint)
    } else {
        format!("{} • SHA256:{}", key.comment, key.fingerprint)
    };

    let row = adw::ActionRow::builder()
        .title(&title)
        .subtitle(&subtitle)
        .build();

    let remove_button = Button::builder()
        .icon_name("user-trash-symbolic")
        .valign(gtk4::Align::Center)
        .tooltip_text(i18n("Remove from agent"))
        .css_classes(["destructive-action", "flat"])
        .build();
    remove_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Remove key from SSH agent",
    ))]);

    // Connect remove button handler
    let manager_clone = ssh_agent_manager.clone();
    let keys_list_clone = ssh_agent_keys_list.clone();
    let status_label_clone = ssh_agent_status_label.clone();
    let socket_label_clone = ssh_agent_socket_label.clone();
    let comment = key.comment.clone();

    remove_button.connect_clicked(move |button| {
        // Try to find the key file path from comment (usually contains the path)
        // If comment is empty or doesn't look like a path, we need to use fingerprint
        let key_path = if !comment.is_empty() && comment.contains('/') {
            std::path::PathBuf::from(&comment)
        } else {
            // Try common SSH key locations
            if let Some(home) = dirs::home_dir() {
                let ssh_dir = home.join(".ssh");
                // Try to find key by fingerprint in available keys
                if let Ok(key_files) = SshAgentManager::list_key_files() {
                    // For now, we'll use ssh-add -d with the fingerprint directly
                    // This requires a different approach - use ssh-add -D to remove all
                    // or find the key file that matches
                    for key_file in key_files {
                        // We can't easily match fingerprint to file without loading each key
                        // So we'll try the comment as a hint
                        if key_file.to_string_lossy().contains(&comment) {
                            return remove_key_and_refresh(
                                button,
                                &key_file,
                                &manager_clone,
                                &keys_list_clone,
                                &status_label_clone,
                                &socket_label_clone,
                            );
                        }
                    }
                }
                ssh_dir.join("id_rsa") // fallback
            } else {
                std::path::PathBuf::from(&comment)
            }
        };

        remove_key_and_refresh(
            button,
            &key_path,
            &manager_clone,
            &keys_list_clone,
            &status_label_clone,
            &socket_label_clone,
        );
    });

    row.add_suffix(&remove_button);
    row
}

/// Helper function to remove a key and refresh the UI
fn remove_key_and_refresh(
    _button: &Button,
    key_path: &std::path::Path,
    ssh_agent_manager: &Rc<RefCell<SshAgentManager>>,
    ssh_agent_keys_list: &ListBox,
    ssh_agent_status_label: &Label,
    ssh_agent_socket_label: &Label,
) {
    let manager = ssh_agent_manager.borrow();
    match manager.remove_key(key_path) {
        Ok(()) => {
            tracing::info!("Key removed successfully: {}", key_path.display());
            drop(manager);
            // Refresh the keys list
            load_ssh_agent_settings(
                ssh_agent_status_label,
                ssh_agent_socket_label,
                ssh_agent_keys_list,
                ssh_agent_manager,
            );
        }
        Err(e) => {
            tracing::error!("Failed to remove key: {e}");
            // Show error toast on parent dialog if available
            // Note: PreferencesDialog doesn't inherit from Root, so we just log the error
            tracing::error!("Failed to remove key: {e}");
        }
    }
}
