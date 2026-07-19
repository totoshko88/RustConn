//! Window UI components
//!
//! This module contains UI creation functions for the main window,
//! including header bar and application menu construction.

use gtk4::prelude::*;
use gtk4::{Button, Label, MenuButton, gio};
use libadwaita as adw;

use crate::i18n::i18n;

/// Creates the header bar with title and controls
///
/// Layout:
/// - Left side (pack_start): Quick Connect, Add, Remove, Add Group
/// - Center: Title + Spinner
/// - Right side (pack_end): Menu, Settings, Split Vertical, Split Horizontal
///
/// Returns the header bar, the busy spinner widget (initially hidden),
/// the passthrough indicator button (initially hidden), the
/// broadcast toggle button (initially hidden, only visible when the
/// active terminal belongs to a cluster), and the primary menu button
/// (needed to suspend the GTK-internal F10 binding in passthrough mode).
#[must_use]
pub fn create_header_bar() -> (
    adw::HeaderBar,
    gtk4::Spinner,
    gtk4::Button,
    gtk4::ToggleButton,
    MenuButton,
    Label,
) {
    let header_bar = adw::HeaderBar::new();

    // Title area: label + spinner in a horizontal box
    let title_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    title_box.set_halign(gtk4::Align::Center);
    title_box.set_valign(gtk4::Align::Center);

    let title = Label::new(Some("RustConn"));
    title.add_css_class("title");
    title_box.append(&title);

    let busy_spinner = gtk4::Spinner::new();
    busy_spinner.set_visible(false);
    busy_spinner.set_tooltip_text(Some(&i18n("Operation in progress")));
    busy_spinner.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Operation in progress",
    ))]);
    title_box.append(&busy_spinner);

    header_bar.set_title_widget(Some(&title_box));

    // === Left side (pack_start) - Primary connection actions ===
    // Order: Sidebar Toggle, Quick Connect, Add, Remove, Add Group

    // Sidebar toggle button
    let sidebar_toggle = Button::from_icon_name("sidebar-show-symbolic");
    sidebar_toggle.set_tooltip_text(Some(&i18n("Toggle Sidebar (F9)")));
    sidebar_toggle.set_action_name(Some("win.toggle-sidebar"));
    sidebar_toggle.update_property(&[gtk4::accessible::Property::Label(&i18n("Toggle Sidebar"))]);
    header_bar.pack_start(&sidebar_toggle);

    // Quick connect button
    let quick_connect_button = Button::from_icon_name("go-jump-symbolic");
    quick_connect_button.set_tooltip_text(Some(&i18n("Quick Connect (Ctrl+Shift+Q)")));
    quick_connect_button.set_action_name(Some("win.quick-connect"));
    quick_connect_button
        .update_property(&[gtk4::accessible::Property::Label(&i18n("Quick Connect"))]);
    header_bar.pack_start(&quick_connect_button);

    // Add connection button
    let add_button = Button::from_icon_name("list-add-symbolic");
    add_button.set_tooltip_text(Some(&i18n("New Connection (Ctrl+N)")));
    add_button.set_action_name(Some("win.new-connection"));
    add_button.update_property(&[gtk4::accessible::Property::Label(&i18n("New Connection"))]);
    header_bar.pack_start(&add_button);

    // Remove button (sensitive only when item selected)
    let remove_button = Button::from_icon_name("list-remove-symbolic");
    remove_button.set_tooltip_text(Some(&i18n("Delete Selected (Delete)")));
    remove_button.set_action_name(Some("win.delete-connection"));
    remove_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Delete Selected"))]);
    header_bar.pack_start(&remove_button);

    // Add group button
    let add_group_button = Button::from_icon_name("folder-new-symbolic");
    add_group_button.set_tooltip_text(Some(&i18n("New Group (Ctrl+Shift+G)")));
    add_group_button.set_action_name(Some("win.new-group"));
    add_group_button.update_property(&[gtk4::accessible::Property::Label(&i18n("New Group"))]);
    header_bar.pack_start(&add_group_button);

    // === Right side (pack_end) - Secondary actions ===

    // Add menu button (rightmost)
    let menu_button = MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text(i18n("Menu (F10)"))
        .build();
    menu_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Menu"))]);
    // Mark as primary menu so GTK auto-binds F10 (GNOME HIG: every app has F10
    // for the primary menu).
    menu_button.set_primary(true);

    let menu = create_app_menu();
    menu_button.set_menu_model(Some(&menu));
    header_bar.pack_end(&menu_button);

    // Add settings button
    let settings_button = Button::from_icon_name("preferences-system-symbolic");
    settings_button.set_tooltip_text(Some(&i18n("Settings (Ctrl+,)")));
    settings_button.set_action_name(Some("win.settings"));
    settings_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Settings"))]);
    header_bar.pack_end(&settings_button);

    // Add split view buttons
    let split_vertical_button = Button::from_icon_name("object-flip-horizontal-symbolic");
    split_vertical_button.set_tooltip_text(Some(&i18n("Split Vertical (Ctrl+Shift+S)")));
    split_vertical_button.set_action_name(Some("win.split-vertical"));
    split_vertical_button
        .update_property(&[gtk4::accessible::Property::Label(&i18n("Split Vertical"))]);
    header_bar.pack_end(&split_vertical_button);

    let split_horizontal_button = Button::from_icon_name("object-flip-vertical-symbolic");
    split_horizontal_button.set_tooltip_text(Some(&i18n("Split Horizontal (Ctrl+Shift+H)")));
    split_horizontal_button.set_action_name(Some("win.split-horizontal"));
    split_horizontal_button
        .update_property(&[gtk4::accessible::Property::Label(&i18n("Split Horizontal"))]);
    header_bar.pack_end(&split_horizontal_button);

    // Shell button — prominent, icon + label, accent color, leftmost in right group
    let shell_button = Button::new();
    let shell_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let shell_icon = gtk4::Image::from_icon_name("utilities-terminal-symbolic");
    shell_icon.set_pixel_size(16);
    let shell_label = Label::new(Some(&i18n("Shell")));
    shell_box.append(&shell_icon);
    shell_box.append(&shell_label);
    shell_button.set_child(Some(&shell_box));
    shell_button.set_tooltip_text(Some(&i18n("Local Shell (Ctrl+Shift+T)")));
    shell_button.set_action_name(Some("win.local-shell"));
    shell_button.add_css_class("suggested-action");
    shell_button.add_css_class("pill");
    shell_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Open Local Shell"))]);
    header_bar.pack_end(&shell_button);

    // Broadcast toggle — visible only when the active tab has a split layout.
    // Mirrors keystrokes from the focused panel to all other panels in the split.
    let broadcast_toggle = gtk4::ToggleButton::new();
    let bc_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    let bc_icon = gtk4::Image::from_icon_name("network-transmit-receive-symbolic");
    bc_icon.set_pixel_size(16);
    let bc_label = Label::new(Some(&i18n("Broadcast")));
    bc_label.add_css_class("caption");
    bc_box.append(&bc_icon);
    bc_box.append(&bc_label);
    broadcast_toggle.set_child(Some(&bc_box));
    broadcast_toggle.set_tooltip_text(Some(&i18n(
        "Mirror keystrokes to all split panels (Ctrl+Shift+B)",
    )));
    broadcast_toggle.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Toggle split broadcast",
    ))]);
    broadcast_toggle.set_action_name(Some("win.toggle-broadcast"));
    broadcast_toggle.add_css_class("flat");
    broadcast_toggle.add_css_class("pill");
    broadcast_toggle.set_visible(false);
    header_bar.pack_end(&broadcast_toggle);

    // Keyboard passthrough indicator — visible only when passthrough mode is active
    let passthrough_indicator = Button::new();
    let pt_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    let pt_icon = gtk4::Image::from_icon_name("input-keyboard-symbolic");
    pt_icon.set_pixel_size(16);
    let pt_label = Label::new(Some(&i18n("Passthrough")));
    pt_label.add_css_class("caption");
    pt_box.append(&pt_icon);
    pt_box.append(&pt_label);
    passthrough_indicator.set_child(Some(&pt_box));
    passthrough_indicator.set_tooltip_text(Some(&i18n(
        "Keyboard passthrough active — click to disable",
    )));
    passthrough_indicator.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Keyboard passthrough active — click to disable",
    ))]);
    passthrough_indicator.set_action_name(Some("win.toggle-passthrough"));
    passthrough_indicator.add_css_class("warning");
    passthrough_indicator.add_css_class("flat");
    passthrough_indicator.add_css_class("pill");
    passthrough_indicator.set_visible(false);
    header_bar.pack_end(&passthrough_indicator);

    // GNOME HIG (Pointer & Touch): icon-only buttons must meet the 44×44px
    // minimum tap target. Buttons with a text label (Shell, Broadcast,
    // Passthrough) already exceed it via their content.
    for icon_button in [
        &sidebar_toggle,
        &quick_connect_button,
        &add_button,
        &remove_button,
        &add_group_button,
        &settings_button,
        &split_vertical_button,
        &split_horizontal_button,
    ] {
        icon_button.set_size_request(44, 44);
    }
    menu_button.set_size_request(44, 44);

    (
        header_bar,
        busy_spinner,
        passthrough_indicator,
        broadcast_toggle,
        menu_button,
        title,
    )
}

/// Creates the application menu
///
/// Menu sections:
/// 1. Connections: New Connection, New Connection (Advanced), New Group, Quick Connect, Local Shell
/// 2. Tools (submenu): Snippets, Clusters, Workspaces, Templates, Variables, and a
///    section with Password Generator, Wake On LAN, SSH Tunnels
/// 3. Sessions (submenu): Active Sessions, History, Statistics, Recordings
/// 4. File: Import, Export, Copy, Paste
/// 5. App: Settings, Fullscreen, Passthrough, Keyboard Shortcuts, About, Quit
#[must_use]
pub fn create_app_menu() -> gio::Menu {
    let menu = gio::Menu::new();

    // Connections section — primary actions (always top-level for quick access)
    let conn_section = gio::Menu::new();
    conn_section.append(Some(&i18n("New Connection")), Some("win.new-connection"));
    conn_section.append(
        Some(&i18n("New Connection (Advanced)…")),
        Some("win.new-connection-advanced"),
    );
    conn_section.append(Some(&i18n("New Group")), Some("win.new-group"));
    conn_section.append(Some(&i18n("Quick Connect")), Some("win.quick-connect"));
    conn_section.append(Some(&i18n("Local Shell")), Some("win.local-shell"));
    menu.append_section(None, &conn_section);

    // Tools submenu — managers grouped together to reduce top-level height
    let tools_submenu = gio::Menu::new();
    tools_submenu.append(Some(&i18n("Snippets...")), Some("win.manage-snippets"));
    tools_submenu.append(Some(&i18n("Clusters...")), Some("win.manage-clusters"));
    tools_submenu.append(Some(&i18n("Workspaces...")), Some("win.manage-workspaces"));
    tools_submenu.append(Some(&i18n("Templates...")), Some("win.manage-templates"));
    tools_submenu.append(Some(&i18n("Variables...")), Some("win.manage-variables"));

    let tools_section_sep = gio::Menu::new();
    tools_section_sep.append(
        Some(&i18n("Password Generator...")),
        Some("win.password-generator"),
    );
    tools_section_sep.append(
        Some(&i18n("Wake On LAN...")),
        Some("win.wake-on-lan-dialog"),
    );
    tools_section_sep.append(Some(&i18n("SSH Tunnels...")), Some("win.ssh-tunnels"));
    tools_submenu.append_section(None, &tools_section_sep);

    let tools_section = gio::Menu::new();
    tools_section.append_submenu(Some(&i18n("Tools")), &tools_submenu);
    menu.append_section(None, &tools_section);

    // Sessions submenu — monitoring and history
    let sessions_submenu = gio::Menu::new();
    sessions_submenu.append(Some(&i18n("Active Sessions...")), Some("win.show-sessions"));
    sessions_submenu.append(
        Some(&i18n("Connection History...")),
        Some("win.show-history"),
    );
    sessions_submenu.append(Some(&i18n("Statistics...")), Some("win.show-statistics"));
    sessions_submenu.append(Some(&i18n("Recordings...")), Some("win.manage-recordings"));

    let sessions_section = gio::Menu::new();
    sessions_section.append_submenu(Some(&i18n("Sessions")), &sessions_submenu);
    menu.append_section(None, &sessions_section);

    // File section (import/export + clipboard)
    let file_section = gio::Menu::new();
    file_section.append(Some(&i18n("Import Connections...")), Some("win.import"));
    file_section.append(Some(&i18n("Export Connections...")), Some("win.export"));
    file_section.append(Some(&i18n("Copy Connection")), Some("win.copy-connection"));
    file_section.append(
        Some(&i18n("Paste Connection")),
        Some("win.paste-connection"),
    );
    menu.append_section(None, &file_section);

    // Settings section (separated from app meta per GNOME HIG)
    let settings_section = gio::Menu::new();
    settings_section.append(Some(&i18n("Settings...")), Some("win.settings"));
    // External CLI components menu — visible in any confined sandbox (snap or
    // Flatpak), where host binaries are unavailable and tools are downloaded
    // into the app's writable data dir. The action is always registered but
    // does nothing outside a sandbox.
    if rustconn_core::is_sandboxed() {
        settings_section.append(Some(&i18n("Components...")), Some("win.flatpak-components"));
    }
    menu.append_section(None, &settings_section);

    // App meta section (GNOME HIG: Fullscreen, Passthrough, Shortcuts, About, Quit)
    let app_section = gio::Menu::new();
    app_section.append(Some(&i18n("Fullscreen")), Some("win.toggle-fullscreen"));
    app_section.append(Some(&i18n("Compact Interface")), Some("win.toggle-compact"));
    app_section.append(
        Some(&i18n("Keyboard Passthrough")),
        Some("win.toggle-passthrough"),
    );
    app_section.append(Some(&i18n("Keyboard Shortcuts...")), Some("app.shortcuts"));
    app_section.append(Some(&i18n("About RustConn")), Some("app.about"));
    app_section.append(Some(&i18n("Quit")), Some("app.quit"));
    menu.append_section(None, &app_section);

    menu
}
