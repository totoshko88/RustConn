//! SPICE protocol options for the connection dialog
//!
//! This module provides the SPICE-specific UI components including:
//! - TLS encryption settings
//! - CA certificate configuration
//! - USB redirection
//! - Clipboard sharing
//! - Image compression settings
//! - Shared folders management

// These functions are prepared for future refactoring when dialog.rs is further modularized
#![allow(dead_code)]

use super::protocol_layout::ProtocolLayoutBuilder;
use super::shared_folders;
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, CheckButton, DropDown, Entry, Orientation, StringList};
use libadwaita as adw;
use rustconn_core::models::SharedFolder;
use std::cell::RefCell;
use std::rc::Rc;

/// Return type for SPICE options creation
#[allow(clippy::type_complexity)]
pub type SpiceOptionsWidgets = (
    GtkBox,
    CheckButton,                    // tls_check
    Entry,                          // ca_cert_entry
    Button,                         // ca_cert_button
    CheckButton,                    // skip_verify_check
    CheckButton,                    // usb_check
    CheckButton,                    // clipboard_check
    DropDown,                       // compression_dropdown
    Rc<RefCell<Vec<SharedFolder>>>, // shared_folders
    gtk4::ListBox,                  // folders_list
);

/// Creates the SPICE options panel using libadwaita components following GNOME HIG.
#[must_use]
pub fn create_spice_options() -> SpiceOptionsWidgets {
    let (container, content) = ProtocolLayoutBuilder::new().build();

    // === Security Group ===
    let (security_group, tls_check, ca_cert_entry, ca_cert_button, skip_verify_check) =
        create_security_group();
    content.append(&security_group);

    // === Features Group ===
    let (features_group, usb_check, clipboard_check, compression_dropdown) =
        create_features_group();
    content.append(&features_group);

    // === Shared Folders Group ===
    let (folders_group, shared_folders, folders_list) =
        shared_folders::create_shared_folders_group();
    content.append(&folders_group);

    (
        container,
        tls_check,
        ca_cert_entry,
        ca_cert_button,
        skip_verify_check,
        usb_check,
        clipboard_check,
        compression_dropdown,
        shared_folders,
        folders_list,
    )
}

/// Creates the Security preferences group
fn create_security_group() -> (
    adw::PreferencesGroup,
    CheckButton,
    Entry,
    Button,
    CheckButton,
) {
    let security_group = adw::PreferencesGroup::builder().title("Security").build();

    // TLS enabled
    let tls_check = CheckButton::new();
    let tls_row = adw::ActionRow::builder()
        .title("TLS Encryption")
        .subtitle("Encrypt connection with TLS")
        .activatable_widget(&tls_check)
        .build();
    tls_row.add_suffix(&tls_check);
    security_group.add(&tls_row);

    // CA certificate path
    let ca_cert_box = GtkBox::new(Orientation::Horizontal, 4);
    ca_cert_box.set_valign(gtk4::Align::Center);
    let ca_cert_entry = Entry::builder()
        .hexpand(true)
        .placeholder_text("Path to CA certificate")
        .build();
    let ca_cert_button = Button::builder()
        .icon_name("folder-open-symbolic")
        .tooltip_text("Browse for certificate")
        .build();
    ca_cert_box.append(&ca_cert_entry);
    ca_cert_box.append(&ca_cert_button);

    let ca_cert_row = adw::ActionRow::builder()
        .title("CA Certificate")
        .subtitle("Certificate authority for TLS verification")
        .build();
    ca_cert_row.add_suffix(&ca_cert_box);
    security_group.add(&ca_cert_row);

    // Skip certificate verification
    let skip_verify_check = CheckButton::new();
    let skip_verify_row = adw::ActionRow::builder()
        .title("Skip Verification")
        .subtitle("Disable certificate verification (insecure)")
        .activatable_widget(&skip_verify_check)
        .build();
    skip_verify_row.add_suffix(&skip_verify_check);
    security_group.add(&skip_verify_row);

    (
        security_group,
        tls_check,
        ca_cert_entry,
        ca_cert_button,
        skip_verify_check,
    )
}

/// Creates the Features preferences group
fn create_features_group() -> (adw::PreferencesGroup, CheckButton, CheckButton, DropDown) {
    let features_group = adw::PreferencesGroup::builder().title("Features").build();

    // USB redirection
    let usb_check = CheckButton::new();
    let usb_row = adw::ActionRow::builder()
        .title("USB Redirection")
        .subtitle("Forward USB devices to remote")
        .activatable_widget(&usb_check)
        .build();
    usb_row.add_suffix(&usb_check);
    features_group.add(&usb_row);

    // Clipboard sharing
    let clipboard_check = CheckButton::new();
    clipboard_check.set_active(true);
    let clipboard_row = adw::ActionRow::builder()
        .title("Clipboard Sharing")
        .subtitle("Synchronize clipboard with remote")
        .activatable_widget(&clipboard_check)
        .build();
    clipboard_row.add_suffix(&clipboard_check);
    features_group.add(&clipboard_row);

    // Image compression
    let compression_list = StringList::new(&["Auto", "Off", "GLZ", "LZ", "QUIC"]);
    let compression_dropdown = DropDown::new(Some(compression_list), gtk4::Expression::NONE);
    compression_dropdown.set_selected(0);
    compression_dropdown.set_valign(gtk4::Align::Center);

    let compression_row = adw::ActionRow::builder()
        .title("Image Compression")
        .subtitle("Algorithm for image data")
        .build();
    compression_row.add_suffix(&compression_dropdown);
    features_group.add(&compression_row);

    (
        features_group,
        usb_check,
        clipboard_check,
        compression_dropdown,
    )
}
