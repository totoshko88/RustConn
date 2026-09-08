//! RDP protocol options for the connection dialog
//!
//! This module provides the RDP-specific UI components including:
//! - Client mode selection (Embedded/External)
//! - Performance mode (Quality/Balanced/Speed)
//! - Resolution and color depth settings
//! - Audio routing (this computer / remote computer / off)
//! - RDP Gateway configuration
//! - Shared folders management
//! - Security layer and TLS settings
//! - Mouse jiggler and autotype settings
//! - Keyboard layout selection

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, DropDown, Entry, Label, Orientation, ScrolledWindow, SpinButton,
    StringList,
};
use libadwaita as adw;
use rustconn_core::models::{
    RdpAudioMode, RdpClientMode, RdpDisplayMode, RdpPerformanceMode, ScaleOverride, SharedFolder,
};

use crate::i18n::i18n;

/// Creates the RDP options panel with all protocol-specific widgets.
///
/// Returns a 35-element tuple matching the fields expected by `ConnectionDialog`.
pub(super) fn create_rdp_options() -> (
    GtkBox,
    DropDown,
    DropDown,
    SpinButton,
    SpinButton,
    DropDown,
    DropDown,
    DropDown,
    adw::SwitchRow,
    Entry,
    SpinButton,
    Entry,
    adw::SwitchRow,
    DropDown,
    SpinButton,
    adw::SwitchRow,
    adw::SwitchRow,
    adw::SwitchRow,
    adw::SwitchRow,
    adw::SwitchRow,
    SpinButton,
    SpinButton,
    SpinButton,
    adw::SwitchRow,
    adw::SwitchRow,
    DropDown,
    Rc<RefCell<Vec<SharedFolder>>>,
    gtk4::ListBox,
    Entry,
    DropDown,
    Entry,
    Entry,
    Entry,
    DropDown,
    DropDown,
) {
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
    let display_group = adw::PreferencesGroup::builder()
        .title(i18n("Display"))
        .build();

    // Client mode dropdown
    let client_mode_items: Vec<String> = vec![
        i18n(RdpClientMode::Embedded.display_name()),
        i18n(RdpClientMode::External.display_name()),
    ];
    let client_mode_strs: Vec<&str> = client_mode_items.iter().map(String::as_str).collect();
    let client_mode_list = StringList::new(&client_mode_strs);
    let client_mode_dropdown = DropDown::builder()
        .model(&client_mode_list)
        .valign(gtk4::Align::Center)
        .build();

    let client_mode_row = adw::ActionRow::builder()
        .title(i18n("Client Mode"))
        .subtitle(i18n(
            "Embedded renders in tab, External opens separate window",
        ))
        .build();
    client_mode_row.add_suffix(&client_mode_dropdown);
    display_group.add(&client_mode_row);

    // Performance mode dropdown
    let perf_items: Vec<String> = vec![
        i18n(RdpPerformanceMode::Quality.display_name()),
        i18n(RdpPerformanceMode::Balanced.display_name()),
        i18n(RdpPerformanceMode::Speed.display_name()),
    ];
    let perf_strs: Vec<&str> = perf_items.iter().map(String::as_str).collect();
    let performance_mode_list = StringList::new(&perf_strs);
    let performance_mode_dropdown = DropDown::builder()
        .model(&performance_mode_list)
        .valign(gtk4::Align::Center)
        .build();
    performance_mode_dropdown.set_selected(1); // Default to Balanced

    let performance_mode_row = adw::ActionRow::builder()
        .title(i18n("Performance Mode"))
        .subtitle(i18n("Quality/speed tradeoff for image rendering"))
        .build();
    performance_mode_row.add_suffix(&performance_mode_dropdown);
    display_group.add(&performance_mode_row);

    // Graphics mode dropdown (only relevant for embedded IronRDP client)
    let gfx_mode_items: Vec<String> = vec![
        i18n("Automatic"),
        i18n("Legacy (Compatible)"),
        i18n("RemoteFX"),
    ];
    let gfx_mode_strs: Vec<&str> = gfx_mode_items.iter().map(String::as_str).collect();
    let gfx_mode_list = StringList::new(&gfx_mode_strs);
    let graphics_mode_dropdown = DropDown::builder()
        .model(&gfx_mode_list)
        .valign(gtk4::Align::Center)
        .build();

    let graphics_mode_row = adw::ActionRow::builder()
        .title(i18n("Graphics Pipeline"))
        .subtitle(i18n(
            "Automatic uses GFX/H.264 when available. Legacy for older servers.",
        ))
        .build();
    graphics_mode_row.add_suffix(&graphics_mode_dropdown);
    display_group.add(&graphics_mode_row);

    // How an external FreeRDP window is sized. Shown for both client modes on
    // purpose: an embedded session hands over to the external client whenever
    // IronRDP cannot serve the server, and this is the only setting that decides
    // what happens then.
    let display_mode_items: Vec<String> = RdpDisplayMode::all()
        .iter()
        .map(|mode| i18n(mode.display_name()))
        .collect();
    let display_mode_strs: Vec<&str> = display_mode_items.iter().map(String::as_str).collect();
    let display_mode_list = StringList::new(&display_mode_strs);
    let display_mode_dropdown = DropDown::builder()
        .model(&display_mode_list)
        .valign(gtk4::Align::Center)
        .build();
    let display_mode_row = adw::ActionRow::builder()
        .title(i18n("External Window"))
        .subtitle(i18n(
            "Size of the separate window, also used when embedded mode falls back",
        ))
        .build();
    display_mode_row.add_suffix(&display_mode_dropdown);
    display_group.add(&display_mode_row);

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
        .title(i18n("Resolution"))
        .subtitle(i18n("Width × Height in pixels"))
        .build();
    resolution_row.add_suffix(&res_box);
    display_group.add(&resolution_row);

    // The resolution row follows the display mode rather than the client mode.
    // It used to be hidden whenever the client mode was Embedded while the save
    // path stored its value regardless, so every connection carried this spin
    // button's 1920x1080 default whether the user had ever seen the row or not —
    // and that value then sized every external window, 4K displays included.
    let resolution_row_for_mode = resolution_row.clone();
    display_mode_dropdown.connect_selected_notify(move |dropdown| {
        resolution_row_for_mode
            .set_visible(RdpDisplayMode::from_index(dropdown.selected()).uses_stored_resolution());
    });
    resolution_row.set_visible(RdpDisplayMode::default().uses_stored_resolution());

    // Color depth
    let color_items: Vec<String> = vec![
        i18n("32-bit (True Color)"),
        i18n("24-bit"),
        i18n("16-bit (High Color)"),
        i18n("15-bit"),
        i18n("8-bit"),
    ];
    let color_strs: Vec<&str> = color_items.iter().map(String::as_str).collect();
    let color_list = StringList::new(&color_strs);
    let color_dropdown = DropDown::new(Some(color_list), gtk4::Expression::NONE);
    color_dropdown.set_selected(0);
    color_dropdown.set_valign(gtk4::Align::Center);

    let color_row = adw::ActionRow::builder()
        .title(i18n("Color Depth"))
        .subtitle(i18n("Higher values provide better quality"))
        .build();
    color_row.add_suffix(&color_dropdown);
    display_group.add(&color_row);

    // Display scale. Honoured by both clients now: the embedded viewer converts
    // logical pixels to device pixels with it, and the external client sends it
    // as the session's DPI. It used to be hidden in External mode and dropped on
    // the way to FreeRDP, so a HiDPI session had no way to ask for readable text.
    let scale_items: Vec<String> = ScaleOverride::all()
        .iter()
        .map(|s| i18n(s.display_name()))
        .collect();
    let scale_strs: Vec<&str> = scale_items.iter().map(String::as_str).collect();
    let scale_list = StringList::new(&scale_strs);
    let scale_override_dropdown = DropDown::builder()
        .model(&scale_list)
        .valign(gtk4::Align::Center)
        .build();
    let scale_row = adw::ActionRow::builder()
        .title(i18n("Display Scale"))
        .subtitle(i18n("HiDPI scaling for the session"))
        .build();
    scale_row.add_suffix(&scale_override_dropdown);
    display_group.add(&scale_row);

    // RDP-1: Info row about embedded dynamic resolution
    let embedded_info_row = adw::ActionRow::builder()
        .title(i18n("Dynamic Resolution"))
        .subtitle(i18n("Embedded mode automatically matches window size"))
        .activatable(false)
        .build();
    embedded_info_row.add_prefix(&gtk4::Image::from_icon_name("dialog-information-symbolic"));
    display_group.add(&embedded_info_row);

    // Only the two rows that genuinely have no meaning outside the embedded
    // viewer follow the client mode. The resolution row follows the display mode
    // instead (see above), and the colour depth and display scale now apply to
    // both clients, so hiding either of them would hide a setting that is in use.
    let embedded_info_clone = embedded_info_row.clone();
    let graphics_mode_row_clone = graphics_mode_row.clone();
    client_mode_dropdown.connect_selected_notify(move |dropdown| {
        let is_embedded = RdpClientMode::from_index(dropdown.selected()) == RdpClientMode::Embedded;
        embedded_info_clone.set_visible(is_embedded);
        graphics_mode_row_clone.set_visible(is_embedded);
    });

    embedded_info_row.set_visible(true);

    content.append(&display_group);

    // === Features Group ===
    let features_group = adw::PreferencesGroup::builder()
        .title(i18n("Features"))
        .build();

    // Audio routing. Three states rather than a switch: "not redirected" and
    // "played on the remote machine" are different things on the wire, and a
    // bool could only ever express one of them (issue #245).
    let audio_items: Vec<String> = RdpAudioMode::all()
        .iter()
        .map(|m| crate::dialogs::rdp_audio_mode_label(*m))
        .collect();
    let audio_strs: Vec<&str> = audio_items.iter().map(String::as_str).collect();
    let audio_list = StringList::new(&audio_strs);
    let audio_mode_dropdown = DropDown::builder()
        .model(&audio_list)
        .valign(gtk4::Align::Center)
        .build();
    audio_mode_dropdown.set_selected(RdpAudioMode::default().index());
    let audio_row = adw::ActionRow::builder()
        .title(i18n("Audio"))
        .subtitle(i18n("Where the remote sound is played"))
        .build();
    audio_row.add_suffix(&audio_mode_dropdown);
    features_group.add(&audio_row);

    // Printer redirection
    let printer_check = adw::SwitchRow::builder()
        .title(i18n("Printer Redirection"))
        .subtitle(i18n("Share your local printer with the remote session"))
        .active(false)
        .build();
    features_group.add(&printer_check);

    // Clipboard sharing
    let clipboard_check = adw::SwitchRow::builder()
        .title(i18n("Clipboard Sharing"))
        .subtitle(i18n("Synchronize clipboard with remote"))
        .active(true)
        .build();
    features_group.add(&clipboard_check);

    // Show local cursor
    let rdp_show_local_cursor_check = adw::SwitchRow::builder()
        .title(i18n("Show Local Cursor"))
        .subtitle(i18n("Hide to avoid double cursor in embedded mode"))
        .active(true)
        .build();
    features_group.add(&rdp_show_local_cursor_check);

    // Floating session toolbar (issue #260). Stated positively here while the
    // stored field reads `hide_floating_toolbar`: the switch is inverted once,
    // in the builder and the populate path, so that a connection written by an
    // older RustConn — or by any importer, which all start from
    // `RdpConfig::default()` — keeps its toolbar.
    let rdp_floating_toolbar_check = adw::SwitchRow::builder()
        .title(i18n("Session Toolbar"))
        .subtitle(i18n(
            "Turn off to remove the floating toolbar and its reveal arrow. Ctrl+Alt+Del and the toolbar buttons go with it.",
        ))
        .active(true)
        .build();
    features_group.add(&rdp_floating_toolbar_check);

    // Mouse Jiggler — prevent idle disconnect
    let rdp_jiggler_check = adw::SwitchRow::builder()
        .title(i18n("Mouse Jiggler"))
        .subtitle(i18n("Prevent idle disconnect by simulating mouse movement"))
        .active(false)
        .build();
    features_group.add(&rdp_jiggler_check);

    let jiggler_adjustment = gtk4::Adjustment::new(60.0, 10.0, 600.0, 10.0, 60.0, 0.0);
    let rdp_jiggler_interval_spin = gtk4::SpinButton::builder()
        .adjustment(&jiggler_adjustment)
        .digits(0)
        .valign(gtk4::Align::Center)
        .sensitive(false)
        .build();
    let jiggler_interval_row = adw::ActionRow::builder()
        .title(i18n("Jiggler Interval"))
        .subtitle(i18n("Seconds between mouse movements"))
        .build();
    jiggler_interval_row.add_suffix(&rdp_jiggler_interval_spin);
    features_group.add(&jiggler_interval_row);

    // Toggle interval sensitivity based on jiggler switch
    let spin_ref = rdp_jiggler_interval_spin.clone();
    rdp_jiggler_check.connect_active_notify(move |switch| {
        spin_ref.set_sensitive(switch.is_active());
    });

    // Autotype settings — inter-character delay and initial delay
    let autotype_adjustment = gtk4::Adjustment::new(20.0, 5.0, 200.0, 5.0, 10.0, 0.0);
    let rdp_autotype_delay_spin = gtk4::SpinButton::builder()
        .adjustment(&autotype_adjustment)
        .digits(0)
        .valign(gtk4::Align::Center)
        .build();
    let autotype_delay_row = adw::ActionRow::builder()
        .title(i18n("Autotype Delay"))
        .subtitle(i18n(
            "Milliseconds between keystrokes (increase for Citrix)",
        ))
        .build();
    autotype_delay_row.add_suffix(&rdp_autotype_delay_spin);
    features_group.add(&autotype_delay_row);

    let autotype_initial_adjustment = gtk4::Adjustment::new(0.0, 0.0, 5000.0, 100.0, 500.0, 0.0);
    let rdp_autotype_initial_delay_spin = gtk4::SpinButton::builder()
        .adjustment(&autotype_initial_adjustment)
        .digits(0)
        .valign(gtk4::Align::Center)
        .build();
    let autotype_initial_row = adw::ActionRow::builder()
        .title(i18n("Autotype Initial Delay"))
        .subtitle(i18n("Milliseconds to wait before typing starts"))
        .build();
    autotype_initial_row.add_suffix(&rdp_autotype_initial_delay_spin);
    features_group.add(&autotype_initial_row);

    // Reconnect on Resize — force full reconnect instead of Display Control
    let rdp_reconnect_on_resize_check = adw::SwitchRow::builder()
        .title(i18n("Reconnect on Resize"))
        .subtitle(i18n(
            "Full reconnect instead of dynamic resize (for legacy servers or fixed resolution)",
        ))
        .active(false)
        .build();
    features_group.add(&rdp_reconnect_on_resize_check);

    // Multipath TCP — use multiple network paths for mobility and aggregation
    let rdp_mptcp_subtitle = if rustconn_core::connection::mptcp::is_mptcp_available() {
        i18n("Use multiple network paths for seamless mobility (Linux 5.6+, embedded mode only)")
    } else {
        i18n("Multiple network paths (embedded mode only). Not available: kernel MPTCP is disabled")
    };
    let rdp_mptcp_check = adw::SwitchRow::builder()
        .title(i18n("Multipath TCP"))
        .subtitle(rdp_mptcp_subtitle)
        .active(false)
        .build();
    features_group.add(&rdp_mptcp_check);

    // Disable NLA
    let disable_nla_check = adw::SwitchRow::builder()
        .title(i18n("Disable NLA"))
        .subtitle(i18n("Skip Network Level Authentication (less secure)"))
        .active(false)
        .build();
    features_group.add(&disable_nla_check);

    // Security Layer dropdown
    let security_layer_items: Vec<String> = rustconn_core::models::RdpSecurityLayer::all()
        .iter()
        .map(|s| i18n(s.display_name()))
        .collect();
    let security_layer_strs: Vec<&str> = security_layer_items.iter().map(String::as_str).collect();
    let security_layer_list = gtk4::StringList::new(&security_layer_strs);
    let security_layer_dropdown = DropDown::new(Some(security_layer_list), gtk4::Expression::NONE);
    security_layer_dropdown.set_selected(0);
    security_layer_dropdown.set_valign(gtk4::Align::Center);
    security_layer_dropdown
        .update_property(&[gtk4::accessible::Property::Label(&i18n("Security Layer"))]);
    let security_layer_row = adw::ActionRow::builder()
        .title(i18n("Security Layer"))
        .subtitle(i18n("RDP/TLS for legacy servers (forces external FreeRDP)"))
        .build();
    security_layer_row.add_suffix(&security_layer_dropdown);
    features_group.add(&security_layer_row);

    // TLS Security Level spin (0–5, default hidden)
    let tls_level_adj = gtk4::Adjustment::new(2.0, 0.0, 5.0, 1.0, 1.0, 0.0);
    let tls_security_level_spin = SpinButton::builder()
        .adjustment(&tls_level_adj)
        .climb_rate(1.0)
        .digits(0)
        .valign(gtk4::Align::Center)
        .build();
    let tls_level_row = adw::ActionRow::builder()
        .title(i18n("TLS Security Level"))
        .subtitle(i18n("0 = legacy (Win7/2012), 2 = default, 5 = strict"))
        .build();
    tls_level_row.add_suffix(&tls_security_level_spin);
    tls_level_row.set_visible(false); // Hidden by default
    features_group.add(&tls_level_row);

    // Show TLS level row only when security layer is not Negotiate
    let tls_level_row_clone = tls_level_row.clone();
    security_layer_dropdown.connect_selected_notify(move |dropdown| {
        // Show TLS level for RDP(1), TLS(2) — legacy modes that benefit from it
        let show = dropdown.selected() == 1 || dropdown.selected() == 2;
        tls_level_row_clone.set_visible(show);
    });

    // Ignore certificate
    let ignore_certificate_check = adw::SwitchRow::builder()
        .title(i18n("Accept Certificate"))
        .subtitle(i18n(
            "Accept changed/self-signed certificates (removes stored fingerprint)",
        ))
        .active(false)
        .build();
    features_group.add(&ignore_certificate_check);

    // Gateway
    let gateway_entry = Entry::builder()
        .hexpand(true)
        .placeholder_text(i18n("gateway.example.com"))
        .valign(gtk4::Align::Center)
        .build();

    let gateway_row = adw::ActionRow::builder()
        .title(i18n("RDP Gateway"))
        .subtitle(i18n("Remote Desktop Gateway server"))
        .build();
    gateway_row.add_suffix(&gateway_entry);
    features_group.add(&gateway_row);

    // Gateway port
    let gw_port_adj = gtk4::Adjustment::new(443.0, 1.0, 65535.0, 1.0, 10.0, 0.0);
    let gateway_port_spin = SpinButton::builder()
        .adjustment(&gw_port_adj)
        .climb_rate(1.0)
        .digits(0)
        .valign(gtk4::Align::Center)
        .build();
    let gw_port_row = adw::ActionRow::builder()
        .title(i18n("Gateway Port"))
        .subtitle(i18n("Default: 443"))
        .build();
    gw_port_row.add_suffix(&gateway_port_spin);
    features_group.add(&gw_port_row);

    // Gateway username
    let gateway_username_entry = Entry::builder()
        .hexpand(true)
        .placeholder_text(i18n("Same as connection username"))
        .valign(gtk4::Align::Center)
        .build();
    let gw_user_row = adw::ActionRow::builder()
        .title(i18n("Gateway Username"))
        .subtitle(i18n("If different from connection username"))
        .build();
    gw_user_row.add_suffix(&gateway_username_entry);
    features_group.add(&gw_user_row);

    // Show/hide gateway details based on gateway hostname
    let gw_port_row_clone = gw_port_row.clone();
    let gw_user_row_clone = gw_user_row.clone();
    gw_port_row.set_visible(false);
    gw_user_row.set_visible(false);
    gateway_entry.connect_changed(move |entry| {
        let visible = !entry.text().is_empty();
        gw_port_row_clone.set_visible(visible);
        gw_user_row_clone.set_visible(visible);
    });

    content.append(&features_group);

    // === Connection Group (Jump Host) ===
    let rdp_connection_group = adw::PreferencesGroup::builder()
        .title(i18n("Connection"))
        .build();

    let none_items: Vec<String> = vec![i18n("(None)")];
    let none_refs: Vec<&str> = none_items.iter().map(String::as_str).collect();
    let rdp_jump_host_list = StringList::new(&none_refs);
    let rdp_jump_host_dropdown = DropDown::new(Some(rdp_jump_host_list), gtk4::Expression::NONE);
    rdp_jump_host_dropdown.set_selected(0);
    crate::dialogs::widgets::enable_string_search(&rdp_jump_host_dropdown);
    rdp_jump_host_dropdown.set_size_request(200, -1);
    rdp_jump_host_dropdown.set_hexpand(false);

    let rdp_jump_host_row = adw::ActionRow::builder()
        .title(i18n("Jump Host"))
        .subtitle(i18n("Tunnel RDP through an SSH connection"))
        .build();
    rdp_jump_host_row.add_suffix(&rdp_jump_host_dropdown);
    rdp_connection_group.add(&rdp_jump_host_row);

    content.append(&rdp_connection_group);

    // === Shared Folders Group ===
    let folders_group = adw::PreferencesGroup::builder()
        .title(i18n("Shared Folders"))
        .description(i18n("Local folders accessible from remote session"))
        .build();

    let folders_list = gtk4::ListBox::builder()
        .selection_mode(gtk4::SelectionMode::Single)
        .css_classes(["boxed-list"])
        .build();
    folders_list.set_placeholder(Some(&Label::new(Some(&i18n("No shared folders")))));

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
    folders_buttons.set_margin_top(12);
    let add_folder_btn = Button::builder()
        .label(i18n("Add"))
        .css_classes(["suggested-action"])
        .build();
    let remove_folder_btn = Button::builder()
        .label(i18n("Remove"))
        .sensitive(false)
        .build();
    folders_buttons.append(&add_folder_btn);
    folders_buttons.append(&remove_folder_btn);
    folders_group.add(&folders_buttons);

    content.append(&folders_group);

    let shared_folders: Rc<RefCell<Vec<SharedFolder>>> = Rc::new(RefCell::new(Vec::new()));

    // Connect add folder button
    super::shared_folders::connect_add_folder_button(
        &add_folder_btn,
        &folders_list,
        &shared_folders,
    );

    // Connect remove folder button
    super::shared_folders::connect_remove_folder_button(
        &remove_folder_btn,
        &folders_list,
        &shared_folders,
    );

    // Enable/disable remove button based on selection
    let remove_btn_for_selection = remove_folder_btn;
    folders_list.connect_row_selected(move |_, row| {
        remove_btn_for_selection.set_sensitive(row.is_some());
    });

    // === RemoteApp Group ===
    let remoteapp_group = adw::PreferencesGroup::builder()
        .title(i18n("RemoteApp"))
        .description(i18n(
            "Launch a single application instead of full desktop (requires FreeRDP)",
        ))
        .build();

    let remote_app_program_entry = Entry::builder()
        .hexpand(true)
        .placeholder_text(i18n("||notepad or C:\\Program Files\\app.exe"))
        .valign(gtk4::Align::Center)
        .build();
    let program_row = adw::ActionRow::builder()
        .title(i18n("Program"))
        .subtitle(i18n("Application alias (||name) or full path"))
        .build();
    program_row.add_suffix(&remote_app_program_entry);
    remoteapp_group.add(&program_row);

    let remote_app_args_entry = Entry::builder()
        .hexpand(true)
        .placeholder_text(i18n("Command-line arguments"))
        .valign(gtk4::Align::Center)
        .build();
    let app_args_row = adw::ActionRow::builder()
        .title(i18n("Arguments"))
        .subtitle(i18n("Arguments passed to the remote application"))
        .build();
    app_args_row.add_suffix(&remote_app_args_entry);
    remoteapp_group.add(&app_args_row);

    let remote_app_name_entry = Entry::builder()
        .hexpand(true)
        .placeholder_text(i18n("Display name (optional)"))
        .valign(gtk4::Align::Center)
        .build();
    let app_name_row = adw::ActionRow::builder()
        .title(i18n("Display Name"))
        .subtitle(i18n("Shown in taskbar and window title"))
        .build();
    app_name_row.add_suffix(&remote_app_name_entry);
    remoteapp_group.add(&app_name_row);

    // Show/hide args and name rows based on program entry
    let app_args_row_clone = app_args_row.clone();
    let app_name_row_clone = app_name_row.clone();
    app_args_row.set_visible(false);
    app_name_row.set_visible(false);

    // Show a warning when RemoteApp is configured but FreeRDP is not available
    let freerdp_warning_row = adw::ActionRow::builder()
        .title(i18n("⚠ FreeRDP not found"))
        .subtitle(i18n(
            "RemoteApp requires FreeRDP. Install sdl-freerdp3, xfreerdp, or wlfreerdp.",
        ))
        .css_classes(["warning"])
        .build();
    freerdp_warning_row.set_visible(false);
    remoteapp_group.add(&freerdp_warning_row);

    let has_freerdp =
        crate::embedded_rdp::launcher::SafeFreeRdpLauncher::detect_freerdp().is_some();
    let freerdp_warning_clone = freerdp_warning_row;
    remote_app_program_entry.connect_changed(move |entry| {
        let has_program = !entry.text().is_empty();
        app_args_row_clone.set_visible(has_program);
        app_name_row_clone.set_visible(has_program);
        freerdp_warning_clone.set_visible(has_program && !has_freerdp);
    });

    content.append(&remoteapp_group);

    // === Advanced Group ===
    let advanced_group = adw::PreferencesGroup::builder()
        .title(i18n("Advanced"))
        .build();

    // Keyboard layout dropdown
    let kb_items: Vec<String> = vec![
        i18n("Auto (detect)"),
        i18n("US English"),
        i18n("German (de)"),
        i18n("French (fr)"),
        i18n("Spanish (es)"),
        i18n("Italian (it)"),
        i18n("Portuguese (pt)"),
        i18n("Portuguese - Brazil (br)"),
        i18n("English - UK (gb)"),
        i18n("German - Switzerland (ch)"),
        i18n("German - Austria (at)"),
        i18n("French - Belgium (be)"),
        i18n("Dutch (nl)"),
        i18n("Swedish (se)"),
        i18n("Norwegian (no)"),
        i18n("Danish (dk)"),
        i18n("Finnish (fi)"),
        i18n("Polish (pl)"),
        i18n("Czech (cz)"),
        i18n("Slovak (sk)"),
        i18n("Hungarian (hu)"),
        i18n("Romanian (ro)"),
        i18n("Croatian (hr)"),
        i18n("Slovenian (si)"),
        i18n("Serbian (rs)"),
        i18n("Bulgarian (bg)"),
        i18n("Russian (ru)"),
        i18n("Ukrainian (ua)"),
        i18n("Turkish (tr)"),
        i18n("Greek (gr)"),
        i18n("Japanese (jp)"),
        i18n("Korean (kr)"),
        i18n("United States - Dvorak"),
    ];
    let kb_strs: Vec<&str> = kb_items.iter().map(String::as_str).collect();
    let kb_layout_list = StringList::new(&kb_strs);
    let kb_layout_dropdown = DropDown::builder()
        .model(&kb_layout_list)
        .valign(gtk4::Align::Center)
        .build();
    let kb_layout_row = adw::ActionRow::builder()
        .title(i18n("Keyboard Layout"))
        .subtitle(i18n("Layout sent to RDP server (Auto uses system setting)"))
        .build();
    kb_layout_row.add_suffix(&kb_layout_dropdown);
    advanced_group.add(&kb_layout_row);

    let args_entry = Entry::builder()
        .hexpand(true)
        .placeholder_text(i18n("Additional command-line arguments"))
        .valign(gtk4::Align::Center)
        .build();

    let args_row = adw::ActionRow::builder()
        .title(i18n("Custom Arguments"))
        .subtitle(i18n("Extra FreeRDP command-line options"))
        .build();
    args_row.add_suffix(&args_entry);
    advanced_group.add(&args_row);

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
        scale_override_dropdown,
        audio_mode_dropdown,
        printer_check,
        gateway_entry,
        gateway_port_spin,
        gateway_username_entry,
        disable_nla_check,
        security_layer_dropdown,
        tls_security_level_spin,
        ignore_certificate_check,
        clipboard_check,
        rdp_show_local_cursor_check,
        rdp_floating_toolbar_check,
        rdp_jiggler_check,
        rdp_jiggler_interval_spin,
        rdp_autotype_delay_spin,
        rdp_autotype_initial_delay_spin,
        rdp_reconnect_on_resize_check,
        rdp_mptcp_check,
        rdp_jump_host_dropdown,
        shared_folders,
        folders_list,
        args_entry,
        kb_layout_dropdown,
        remote_app_program_entry,
        remote_app_args_entry,
        remote_app_name_entry,
        graphics_mode_dropdown,
        display_mode_dropdown,
    )
}
