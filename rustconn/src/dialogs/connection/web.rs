//! Web bookmark protocol options for the connection dialog
//!
//! UI panel for Web bookmark connections with browser selection
//! and private/incognito mode toggle.

use super::protocol_layout::ProtocolLayoutBuilder;
use super::widgets::{EntryRowBuilder, SwitchRowBuilder};
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Entry};
use libadwaita as adw;

use crate::i18n::i18n;

/// Return type for Web options creation
///
/// Contains:
/// - Container box
/// - Browser entry (custom browser command)
/// - Private mode switch row
pub type WebOptionsWidgets = (GtkBox, Entry, adw::SwitchRow);

/// Creates the Web bookmark options panel using libadwaita components.
///
/// The panel has groups for browser settings. URL is configured in the
/// host field on the Basic tab (relabeled to "URL" for Web protocol).
#[must_use]
pub fn create_web_options() -> WebOptionsWidgets {
    let (container, content) = ProtocolLayoutBuilder::new().build();

    // === Browser Group ===
    let browser_group = adw::PreferencesGroup::builder()
        .title(i18n("Browser"))
        .description(i18n(
            "Configure which browser opens the URL. Leave empty for system default.",
        ))
        .build();

    let (browser_row, browser_entry) = EntryRowBuilder::new(i18n("Custom Browser"))
        .subtitle(i18n(
            "Command or path to browser binary (e.g. firefox, chromium)",
        ))
        .placeholder(i18n("System default (xdg-open)"))
        .build();
    browser_group.add(&browser_row);

    let private_mode_switch = SwitchRowBuilder::new(i18n("Private / Incognito Mode"))
        .subtitle(i18n(
            "Open URL in a private browsing window (Firefox, Chrome, Brave)",
        ))
        .active(false)
        .build();
    browser_group.add(&private_mode_switch);

    content.append(&browser_group);

    // === Info Group ===
    let info_group = adw::PreferencesGroup::builder()
        .title(i18n("How It Works"))
        .build();

    let info_row = adw::ActionRow::builder()
        .title(i18n("URL opens in the system browser"))
        .subtitle(i18n(
            "No embedded browser — RustConn delegates to the OS via GTK4 UriLauncher (portal-aware, works in Flatpak). Credentials are stored for copy-to-clipboard via the sidebar context menu.",
        ))
        .build();
    let info_icon = gtk4::Image::from_icon_name("dialog-information-symbolic");
    info_row.add_prefix(&info_icon);
    info_group.add(&info_row);

    content.append(&info_group);

    (container, browser_entry, private_mode_switch)
}
