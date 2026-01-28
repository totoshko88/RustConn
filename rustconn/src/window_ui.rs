//! Window UI components
//!
//! This module contains UI creation functions for the main window,
//! including header bar and application menu construction.

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
    // Order: Quick Connect, Add, Remove, Add Group

    // Quick connect button
    let quick_connect_button = Button::from_icon_name("go-jump-symbolic");
    quick_connect_button.set_tooltip_text(Some("Quick Connect (Ctrl+Shift+Q)"));
    quick_connect_button.set_action_name(Some("win.quick-connect"));
    header_bar.pack_start(&quick_connect_button);

    // Add connection button
    let add_button = Button::from_icon_name("list-add-symbolic");
    add_button.set_tooltip_text(Some("New Connection (Ctrl+N)"));
    add_button.set_action_name(Some("win.new-connection"));
    header_bar.pack_start(&add_button);

    // Remove button (sensitive only when item selected)
    let remove_button = Button::from_icon_name("list-remove-symbolic");
    remove_button.set_tooltip_text(Some("Delete Selected (Delete)"));
    remove_button.set_action_name(Some("win.delete-connection"));
    header_bar.pack_start(&remove_button);

    // Add group button
    let add_group_button = Button::from_icon_name("folder-new-symbolic");
    add_group_button.set_tooltip_text(Some("New Group (Ctrl+Shift+N)"));
    add_group_button.set_action_name(Some("win.new-group"));
    header_bar.pack_start(&add_group_button);

    // === Right side (pack_end) - Secondary actions ===

    // Add menu button (rightmost)
    let menu_button = MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text("Menu")
        .build();

    let menu = create_app_menu();
    menu_button.set_menu_model(Some(&menu));
    header_bar.pack_end(&menu_button);

    // Add settings button
    let settings_button = Button::from_icon_name("emblem-system-symbolic");
    settings_button.set_tooltip_text(Some("Settings (Ctrl+,)"));
    settings_button.set_action_name(Some("win.settings"));
    header_bar.pack_end(&settings_button);

    // Add split view buttons
    let split_vertical_button = Button::from_icon_name("object-flip-horizontal-symbolic");
    split_vertical_button.set_tooltip_text(Some("Split Vertical (Ctrl+Shift+S)"));
    split_vertical_button.set_action_name(Some("win.split-vertical"));
    header_bar.pack_end(&split_vertical_button);

    let split_horizontal_button = Button::from_icon_name("object-flip-vertical-symbolic");
    split_horizontal_button.set_tooltip_text(Some("Split Horizontal (Ctrl+Shift+H)"));
    split_horizontal_button.set_action_name(Some("win.split-horizontal"));
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
/// 5. App: Settings, About, Quit
#[must_use]
pub fn create_app_menu() -> gio::Menu {
    let menu = gio::Menu::new();

    // Connections section
    let conn_section = gio::Menu::new();
    conn_section.append(Some("New Connection"), Some("win.new-connection"));
    conn_section.append(Some("New Group"), Some("win.new-group"));
    conn_section.append(Some("Quick Connect"), Some("win.quick-connect"));
    conn_section.append(Some("Local Shell"), Some("win.local-shell"));
    menu.append_section(None, &conn_section);

    // Tools section (managers)
    let tools_section = gio::Menu::new();
    tools_section.append(Some("Snippets..."), Some("win.manage-snippets"));
    tools_section.append(Some("Clusters..."), Some("win.manage-clusters"));
    tools_section.append(Some("Templates..."), Some("win.manage-templates"));
    tools_section.append(Some("Active Sessions"), Some("win.show-sessions"));
    tools_section.append(Some("Connection History..."), Some("win.show-history"));
    tools_section.append(Some("Statistics..."), Some("win.show-statistics"));
    tools_section.append(
        Some("Password Generator..."),
        Some("win.password-generator"),
    );
    menu.append_section(None, &tools_section);

    // File section (import/export connections)
    let file_section = gio::Menu::new();
    file_section.append(Some("Import Connections..."), Some("win.import"));
    file_section.append(Some("Export Connections..."), Some("win.export"));
    menu.append_section(None, &file_section);

    // Edit section
    let edit_section = gio::Menu::new();
    edit_section.append(Some("Copy Connection"), Some("win.copy-connection"));
    edit_section.append(Some("Paste Connection"), Some("win.paste-connection"));
    menu.append_section(None, &edit_section);

    // App section
    let app_section = gio::Menu::new();
    app_section.append(Some("Settings"), Some("win.settings"));
    app_section.append(Some("About"), Some("app.about"));
    app_section.append(Some("Quit"), Some("app.quit"));
    menu.append_section(None, &app_section);

    menu
}
