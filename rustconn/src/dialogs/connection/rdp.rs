//! RDP protocol options for the connection dialog
//!
//! This module provides the RDP-specific UI components including:
//! - Client mode selection (Embedded/External)
//! - Performance mode (Quality/Balanced/Speed)
//! - Resolution and color depth settings
//! - Audio redirection
//! - RDP Gateway configuration
//! - Shared folders management

// These functions are prepared for future refactoring when dialog.rs is further modularized
#![allow(dead_code)]

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, CheckButton, DropDown, Entry, FileDialog, Label, Orientation,
    ScrolledWindow, SpinButton, StringList,
};
use libadwaita as adw;
use rustconn_core::models::{RdpClientMode, RdpPerformanceMode, SharedFolder};
use std::cell::RefCell;
use std::rc::Rc;

/// Return type for RDP options creation
#[allow(clippy::type_complexity)]
pub type RdpOptionsWidgets = (
    GtkBox,
    DropDown,                       // client_mode_dropdown
    DropDown,                       // performance_mode_dropdown
    SpinButton,                     // width_spin
    SpinButton,                     // height_spin
    DropDown,                       // color_dropdown
    CheckButton,                    // audio_check
    Entry,                          // gateway_entry
    Rc<RefCell<Vec<SharedFolder>>>, // shared_folders
    gtk4::ListBox,                  // folders_list
    Entry,                          // custom_args_entry
);

/// Creates the RDP options panel using libadwaita components following GNOME HIG.
#[must_use]
pub fn create_rdp_options() -> RdpOptionsWidgets {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .vexpand(true)
        .build();

    let clamp = adw::Clamp::builder()
        .maximum_size(600)
        .tightening_threshold(400)
        .build();

    let content = GtkBox::new(Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    // === Display Group ===
    let (
        display_group,
        client_mode_dropdown,
        performance_mode_dropdown,
        width_spin,
        height_spin,
        color_dropdown,
    ) = create_display_group();
    content.append(&display_group);

    // === Features Group ===
    let (features_group, audio_check, gateway_entry) = create_features_group();
    content.append(&features_group);

    // === Shared Folders Group ===
    let (folders_group, shared_folders, folders_list) = create_shared_folders_group();
    content.append(&folders_group);

    // === Advanced Group ===
    let (advanced_group, args_entry) = create_advanced_group();
    content.append(&advanced_group);

    clamp.set_child(Some(&content));
    scrolled.set_child(Some(&clamp));

    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.append(&scrolled);

    (
        vbox,
        client_mode_dropdown,
        performance_mode_dropdown,
        width_spin,
        height_spin,
        color_dropdown,
        audio_check,
        gateway_entry,
        shared_folders,
        folders_list,
        args_entry,
    )
}

/// Creates the Display preferences group
#[allow(clippy::type_complexity)]
fn create_display_group() -> (
    adw::PreferencesGroup,
    DropDown,
    DropDown,
    SpinButton,
    SpinButton,
    DropDown,
) {
    let display_group = adw::PreferencesGroup::builder().title("Display").build();

    // Client mode dropdown
    let client_mode_list = StringList::new(&[
        RdpClientMode::Embedded.display_name(),
        RdpClientMode::External.display_name(),
    ]);
    let client_mode_dropdown = DropDown::builder()
        .model(&client_mode_list)
        .valign(gtk4::Align::Center)
        .build();

    let client_mode_row = adw::ActionRow::builder()
        .title("Client Mode")
        .subtitle("Embedded renders in tab, External opens separate window")
        .build();
    client_mode_row.add_suffix(&client_mode_dropdown);
    display_group.add(&client_mode_row);

    // Performance mode dropdown
    let performance_mode_list = StringList::new(&[
        RdpPerformanceMode::Quality.display_name(),
        RdpPerformanceMode::Balanced.display_name(),
        RdpPerformanceMode::Speed.display_name(),
    ]);
    let performance_mode_dropdown = DropDown::builder()
        .model(&performance_mode_list)
        .valign(gtk4::Align::Center)
        .build();
    performance_mode_dropdown.set_selected(1); // Default to Balanced

    let performance_mode_row = adw::ActionRow::builder()
        .title("Performance Mode")
        .subtitle("Quality/speed tradeoff for image rendering")
        .build();
    performance_mode_row.add_suffix(&performance_mode_dropdown);
    display_group.add(&performance_mode_row);

    // Resolution
    let res_box = GtkBox::new(Orientation::Horizontal, 4);
    res_box.set_valign(gtk4::Align::Center);
    let width_adj = gtk4::Adjustment::new(1920.0, 640.0, 7680.0, 1.0, 100.0, 0.0);
    let width_spin = SpinButton::builder()
        .adjustment(&width_adj)
        .climb_rate(1.0)
        .digits(0)
        .build();
    let x_label = Label::new(Some("×"));
    let height_adj = gtk4::Adjustment::new(1080.0, 480.0, 4320.0, 1.0, 100.0, 0.0);
    let height_spin = SpinButton::builder()
        .adjustment(&height_adj)
        .climb_rate(1.0)
        .digits(0)
        .build();
    res_box.append(&width_spin);
    res_box.append(&x_label);
    res_box.append(&height_spin);

    let resolution_row = adw::ActionRow::builder()
        .title("Resolution")
        .subtitle("Width × Height in pixels")
        .build();
    resolution_row.add_suffix(&res_box);
    display_group.add(&resolution_row);

    // Color depth
    let color_list = StringList::new(&[
        "32-bit (True Color)",
        "24-bit",
        "16-bit (High Color)",
        "15-bit",
        "8-bit",
    ]);
    let color_dropdown = DropDown::new(Some(color_list), gtk4::Expression::NONE);
    color_dropdown.set_selected(0);
    color_dropdown.set_valign(gtk4::Align::Center);

    let color_row = adw::ActionRow::builder()
        .title("Color Depth")
        .subtitle("Higher values provide better quality")
        .build();
    color_row.add_suffix(&color_dropdown);
    display_group.add(&color_row);

    // Connect client mode dropdown to show/hide resolution/color rows
    let resolution_row_clone = resolution_row.clone();
    let color_row_clone = color_row.clone();
    client_mode_dropdown.connect_selected_notify(move |dropdown| {
        let is_embedded = dropdown.selected() == 0;
        resolution_row_clone.set_visible(!is_embedded);
        color_row_clone.set_visible(!is_embedded);
    });

    // Set initial state (Embedded - hide resolution/color)
    resolution_row.set_visible(false);
    color_row.set_visible(false);

    (
        display_group,
        client_mode_dropdown,
        performance_mode_dropdown,
        width_spin,
        height_spin,
        color_dropdown,
    )
}

/// Creates the Features preferences group
fn create_features_group() -> (adw::PreferencesGroup, CheckButton, Entry) {
    let features_group = adw::PreferencesGroup::builder().title("Features").build();

    // Audio redirect
    let audio_check = CheckButton::new();
    let audio_row = adw::ActionRow::builder()
        .title("Audio Redirection")
        .subtitle("Play remote audio locally")
        .activatable_widget(&audio_check)
        .build();
    audio_row.add_suffix(&audio_check);
    features_group.add(&audio_row);

    // Gateway
    let gateway_entry = Entry::builder()
        .hexpand(true)
        .placeholder_text("gateway.example.com")
        .valign(gtk4::Align::Center)
        .build();

    let gateway_row = adw::ActionRow::builder()
        .title("RDP Gateway")
        .subtitle("Remote Desktop Gateway server")
        .build();
    gateway_row.add_suffix(&gateway_entry);
    features_group.add(&gateway_row);

    (features_group, audio_check, gateway_entry)
}

/// Creates the Shared Folders preferences group
fn create_shared_folders_group() -> (
    adw::PreferencesGroup,
    Rc<RefCell<Vec<SharedFolder>>>,
    gtk4::ListBox,
) {
    let folders_group = adw::PreferencesGroup::builder()
        .title("Shared Folders")
        .description("Local folders accessible from remote session")
        .build();

    let folders_list = gtk4::ListBox::builder()
        .selection_mode(gtk4::SelectionMode::Single)
        .css_classes(["boxed-list"])
        .build();
    folders_list.set_placeholder(Some(&Label::new(Some("No shared folders"))));

    let folders_scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .min_content_height(80)
        .max_content_height(120)
        .child(&folders_list)
        .build();
    folders_group.add(&folders_scrolled);

    let folders_buttons = GtkBox::new(Orientation::Horizontal, 8);
    folders_buttons.set_halign(gtk4::Align::End);
    folders_buttons.set_margin_top(8);
    let add_folder_btn = Button::builder()
        .label("Add")
        .css_classes(["suggested-action"])
        .build();
    let remove_folder_btn = Button::builder().label("Remove").sensitive(false).build();
    folders_buttons.append(&add_folder_btn);
    folders_buttons.append(&remove_folder_btn);
    folders_group.add(&folders_buttons);

    let shared_folders: Rc<RefCell<Vec<SharedFolder>>> = Rc::new(RefCell::new(Vec::new()));

    // Connect add folder button
    connect_add_folder_button(&add_folder_btn, &folders_list, &shared_folders);

    // Connect remove folder button
    connect_remove_folder_button(&remove_folder_btn, &folders_list, &shared_folders);

    // Enable/disable remove button based on selection
    let remove_btn_for_selection = remove_folder_btn;
    folders_list.connect_row_selected(move |_, row| {
        remove_btn_for_selection.set_sensitive(row.is_some());
    });

    (folders_group, shared_folders, folders_list)
}

/// Creates the Advanced preferences group
fn create_advanced_group() -> (adw::PreferencesGroup, Entry) {
    let advanced_group = adw::PreferencesGroup::builder().title("Advanced").build();

    let args_entry = Entry::builder()
        .hexpand(true)
        .placeholder_text("Additional command-line arguments")
        .valign(gtk4::Align::Center)
        .build();

    let args_row = adw::ActionRow::builder()
        .title("Custom Arguments")
        .subtitle("Extra FreeRDP command-line options")
        .build();
    args_row.add_suffix(&args_entry);
    advanced_group.add(&args_row);

    (advanced_group, args_entry)
}

/// Connects the add folder button to show file dialog and add folder
pub fn connect_add_folder_button(
    add_btn: &Button,
    folders_list: &gtk4::ListBox,
    shared_folders: &Rc<RefCell<Vec<SharedFolder>>>,
) {
    let folders_list_clone = folders_list.clone();
    let shared_folders_clone = shared_folders.clone();
    add_btn.connect_clicked(move |btn| {
        let file_dialog = FileDialog::builder()
            .title("Select Folder to Share")
            .modal(true)
            .build();

        let folders_list = folders_list_clone.clone();
        let shared_folders = shared_folders_clone.clone();
        let parent = btn.root().and_then(|r| r.downcast::<gtk4::Window>().ok());

        file_dialog.select_folder(
            parent.as_ref(),
            gtk4::gio::Cancellable::NONE,
            move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        let share_name = path.file_name().map_or_else(
                            || "Share".to_string(),
                            |n| n.to_string_lossy().to_string(),
                        );

                        let folder = SharedFolder {
                            local_path: path.clone(),
                            share_name: share_name.clone(),
                        };

                        shared_folders.borrow_mut().push(folder);
                        add_folder_row_to_list(&folders_list, &path, &share_name);
                    }
                }
            },
        );
    });
}

/// Adds a folder row to the list UI
pub fn add_folder_row_to_list(
    folders_list: &gtk4::ListBox,
    path: &std::path::Path,
    share_name: &str,
) {
    let row_box = GtkBox::new(Orientation::Horizontal, 8);
    row_box.set_margin_top(4);
    row_box.set_margin_bottom(4);
    row_box.set_margin_start(8);
    row_box.set_margin_end(8);

    let path_label = Label::builder()
        .label(path.to_string_lossy().as_ref())
        .hexpand(true)
        .halign(gtk4::Align::Start)
        .ellipsize(gtk4::pango::EllipsizeMode::Middle)
        .build();
    let name_label = Label::builder()
        .label(format!("→ {share_name}"))
        .halign(gtk4::Align::End)
        .build();

    row_box.append(&path_label);
    row_box.append(&name_label);
    folders_list.append(&row_box);
}

/// Connects the remove folder button
pub fn connect_remove_folder_button(
    remove_btn: &Button,
    folders_list: &gtk4::ListBox,
    shared_folders: &Rc<RefCell<Vec<SharedFolder>>>,
) {
    let folders_list_clone = folders_list.clone();
    let shared_folders_clone = shared_folders.clone();
    remove_btn.connect_clicked(move |_| {
        if let Some(selected_row) = folders_list_clone.selected_row() {
            if let Ok(index) = usize::try_from(selected_row.index()) {
                if index < shared_folders_clone.borrow().len() {
                    shared_folders_clone.borrow_mut().remove(index);
                    folders_list_clone.remove(&selected_row);
                }
            }
        }
    });
}
