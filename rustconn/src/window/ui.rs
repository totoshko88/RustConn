//! Window UI components
//!
//! This module contains UI creation functions for the main window,
//! including header bar and application menu construction.

use crate::i18n::i18n;
use gtk4::gio;
use gtk4::prelude::*;
use gtk4::{Button, Label, MenuButton};
use libadwaita as adw;

/// Creates the header bar with title and controls
///
/// Layout:
/// - Left side (pack_start): Quick Connect, Add, Remove, Add Group
/// - Center: Title
/// - Right side (pack_end): Menu, Settings, Split Vertical, Split Horizontal
#[must_use]
pub fn create_header_bar() -> adw::HeaderBar {
    let header_bar = adw::HeaderBar::new();

    // Add title
    let title = Label::new(Some("RustConn"));
    title.add_css_class("title");
    header_bar.set_title_widget(Some(&title));

    // === Left side (pack_start) - Primary connection actions ===
    // Order: Sidebar Toggle, Quick Connect, Add, Remove, Add Group

    // Sidebar toggle button
    let sidebar_toggle = Button::from_icon_name("sidebar-show-symbolic");
    sidebar_toggle.set_tooltip_text(Some(&i18n("Toggle Sidebar (F9)")));
    sidebar_toggle.set_action_name(Some("win.toggle-sidebar"));
    sidebar_toggle.update_property(&[gtk4::accessible::Property::Label("Toggle Sidebar")]);
    header_bar.pack_start(&sidebar_toggle);

    // Quick connect button
    let quick_connect_button = Button::from_icon_name("go-jump-symbolic");
    quick_connect_button.set_tooltip_text(Some(&i18n("Quick Connect (Ctrl+Shift+Q)")));
    quick_connect_button.set_action_name(Some("win.quick-connect"));
    quick_connect_button.update_property(&[gtk4::accessible::Property::Label("Quick Connect")]);
    header_bar.pack_start(&quick_connect_button);

    // Add connection button
    let add_button = Button::from_icon_name("list-add-symbolic");
    add_button.set_tooltip_text(Some(&i18n("New Connection (Ctrl+N)")));
    add_button.set_action_name(Some("win.new-connection"));
    add_button.update_property(&[gtk4::accessible::Property::Label("New Connection")]);
    header_bar.pack_start(&add_button);

    // Remove button (sensitive only when item selected)
    let remove_button = Button::from_icon_name("list-remove-symbolic");
    remove_button.set_tooltip_text(Some(&i18n("Delete Selected (Delete)")));
    remove_button.set_action_name(Some("win.delete-connection"));
    remove_button.update_property(&[gtk4::accessible::Property::Label("Delete Selected")]);
    header_bar.pack_start(&remove_button);

    // Add group button
    let add_group_button = Button::from_icon_name("folder-new-symbolic");
    add_group_button.set_tooltip_text(Some(&i18n("New Group (Ctrl+Shift+N)")));
    add_group_button.set_action_name(Some("win.new-group"));
    add_group_button.update_property(&[gtk4::accessible::Property::Label("New Group")]);
    header_bar.pack_start(&add_group_button);

    // === Right side (pack_end) - Secondary actions ===

    // Add menu button (rightmost)
    let menu_button = MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text(i18n("Menu"))
        .build();
    menu_button.update_property(&[gtk4::accessible::Property::Label("Menu")]);

    let menu = create_app_menu();
    menu_button.set_menu_model(Some(&menu));
    header_bar.pack_end(&menu_button);

    // Add settings button
    let settings_button = Button::from_icon_name("emblem-system-symbolic");
    settings_button.set_tooltip_text(Some(&i18n("Settings (Ctrl+,)")));
    settings_button.set_action_name(Some("win.settings"));
    settings_button.update_property(&[gtk4::accessible::Property::Label("Settings")]);
    header_bar.pack_end(&settings_button);

    // Add split view buttons
    let split_vertical_button = Button::from_icon_name("object-flip-horizontal-symbolic");
    split_vertical_button.set_tooltip_text(Some(&i18n("Split Vertical (Ctrl+Shift+S)")));
    split_vertical_button.set_action_name(Some("win.split-vertical"));
    split_vertical_button.update_property(&[gtk4::accessible::Property::Label("Split Vertical")]);
    header_bar.pack_end(&split_vertical_button);

    let split_horizontal_button = Button::from_icon_name("object-flip-vertical-symbolic");
    split_horizontal_button.set_tooltip_text(Some(&i18n("Split Horizontal (Ctrl+Shift+H)")));
    split_horizontal_button.set_action_name(Some("win.split-horizontal"));
    split_horizontal_button
        .update_property(&[gtk4::accessible::Property::Label("Split Horizontal")]);
    header_bar.pack_end(&split_horizontal_button);

    header_bar
}

/// Creates the application menu
///
/// Menu sections:
/// 1. Connections: New Connection, New Group, Quick Connect, Local Shell
/// 2. Tools: Snippets, Clusters, Templates, Sessions, History, Statistics, Password Generator
/// 3. File: Import, Export
/// 4. Edit: Copy Connection, Paste Connection
/// 5. App: Settings, Flatpak Components (if Flatpak), About, Quit
#[must_use]
pub fn create_app_menu() -> gio::Menu {
    let menu = gio::Menu::new();

    // Connections section
    let conn_section = gio::Menu::new();
    conn_section.append(Some(&i18n("New Connection")), Some("win.new-connection"));
    conn_section.append(Some(&i18n("New Group")), Some("win.new-group"));
    conn_section.append(Some(&i18n("Quick Connect")), Some("win.quick-connect"));
    conn_section.append(Some(&i18n("Local Shell")), Some("win.local-shell"));
    menu.append_section(None, &conn_section);

    // Tools section (managers)
    let tools_section = gio::Menu::new();
    tools_section.append(Some(&i18n("Snippets...")), Some("win.manage-snippets"));
    tools_section.append(Some(&i18n("Clusters...")), Some("win.manage-clusters"));
    tools_section.append(Some(&i18n("Templates...")), Some("win.manage-templates"));
    tools_section.append(Some(&i18n("Variables...")), Some("win.manage-variables"));
    tools_section.append(Some(&i18n("Active Sessions")), Some("win.show-sessions"));
    tools_section.append(
        Some(&i18n("Connection History...")),
        Some("win.show-history"),
    );
    tools_section.append(Some(&i18n("Statistics...")), Some("win.show-statistics"));
    tools_section.append(
        Some(&i18n("Password Generator...")),
        Some("win.password-generator"),
    );
    tools_section.append(
        Some(&i18n("Wake On LAN...")),
        Some("win.wake-on-lan-dialog"),
    );
    menu.append_section(None, &tools_section);

    // File section (import/export connections)
    let file_section = gio::Menu::new();
    file_section.append(Some(&i18n("Import Connections...")), Some("win.import"));
    file_section.append(Some(&i18n("Export Connections...")), Some("win.export"));
    menu.append_section(None, &file_section);

    // Edit section
    let edit_section = gio::Menu::new();
    edit_section.append(Some(&i18n("Copy Connection")), Some("win.copy-connection"));
    edit_section.append(
        Some(&i18n("Paste Connection")),
        Some("win.paste-connection"),
    );
    menu.append_section(None, &edit_section);

    // App section
    let app_section = gio::Menu::new();
    app_section.append(Some(&i18n("Settings")), Some("win.settings"));
    // Flatpak Components menu item - only visible in Flatpak environment
    // The action is always registered but does nothing outside Flatpak
    if rustconn_core::flatpak::is_flatpak() {
        app_section.append(
            Some(&i18n("Flatpak Components...")),
            Some("win.flatpak-components"),
        );
    }
    app_section.append(Some(&i18n("About")), Some("app.about"));
    app_section.append(Some(&i18n("Quit")), Some("app.quit"));
    menu.append_section(None, &app_section);

    menu
}
