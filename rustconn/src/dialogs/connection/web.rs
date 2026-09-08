//! Web bookmark protocol options for the connection dialog
//!
//! UI panel for Web bookmark connections with browser mode selection,
//! JavaScript toggle, user agent configuration, and private/incognito mode.

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Entry, StringList};
use libadwaita as adw;
use rustconn_core::models::WebBrowserMode;

use super::protocol_layout::ProtocolLayoutBuilder;
use super::widgets::{EntryRowBuilder, SwitchRowBuilder};
use crate::i18n::i18n;

/// Return type for Web options creation (extended for embedded browser).
///
/// Contains all widget references needed by the connection dialog
/// for reading/writing Web protocol settings.
pub struct WebOptionsWidgets {
    /// Outer container box (added to protocol stack)
    pub container: GtkBox,
    /// Custom browser command entry
    pub browser_entry: Entry,
    /// Private / incognito mode toggle
    pub private_mode_switch: adw::SwitchRow,
    /// Browser Mode dropdown (Embedded/System/Custom)
    pub browser_mode_combo: adw::ComboRow,
    /// JavaScript enabled/disabled toggle
    pub javascript_switch: adw::SwitchRow,
    /// User agent string entry (optional)
    pub user_agent_row: adw::EntryRow,
    /// Floating navigation toolbar toggle (issue #260)
    pub floating_toolbar_switch: adw::SwitchRow,
    /// SSH connection to browse through as a SOCKS tunnel ("(None)" = direct).
    /// Populated with SSH connections after construction, like the jump-host
    /// dropdowns; only meaningful in the embedded browser.
    pub tunnel_dropdown: gtk4::DropDown,
}

/// Creates the Web bookmark options panel using libadwaita components.
///
/// The panel has groups for browser mode selection, embedded browser settings,
/// and general browser preferences. URL is configured in the host field on
/// the Basic tab (relabeled to "URL" for Web protocol).
#[must_use]
pub fn create_web_options() -> WebOptionsWidgets {
    let (container, content) = ProtocolLayoutBuilder::new().build();

    // === Browser Mode Group ===
    let mode_group = adw::PreferencesGroup::builder()
        .title(i18n("Browser Mode"))
        .description(i18n("Choose how the URL is opened"))
        .build();

    let browser_mode_combo = build_browser_mode_combo();
    mode_group.add(&browser_mode_combo);

    content.append(&mode_group);

    // === Browser Command Group ===
    let browser_group = adw::PreferencesGroup::builder()
        .title(i18n("Custom Browser"))
        .description(i18n(
            "Specify a browser command when Custom mode is selected.",
        ))
        .build();

    let (browser_row, browser_entry) = EntryRowBuilder::new(i18n("Browser Command"))
        .subtitle(i18n(
            "Command or path to browser binary (e.g. firefox, chromium)",
        ))
        .placeholder(i18n("Required for Custom mode"))
        .build();
    browser_group.add(&browser_row);

    content.append(&browser_group);

    // === Embedded Browser Settings Group ===
    let embedded_group = adw::PreferencesGroup::builder()
        .title(i18n("Embedded Browser Settings"))
        .description(i18n("Settings applied when using the embedded browser"))
        .build();

    let javascript_switch = SwitchRowBuilder::new("JavaScript")
        .subtitle("Enable or disable JavaScript execution in the embedded browser")
        .active(true)
        .build();
    embedded_group.add(&javascript_switch);

    let user_agent_row = adw::EntryRow::builder().title(i18n("User Agent")).build();
    user_agent_row.set_show_apply_button(false);
    embedded_group.add(&user_agent_row);

    // Browse through an SSH host (dynamic SOCKS tunnel). Model is filled with
    // SSH connections after construction (see populate.rs); "(None)" is a direct
    // connection. Wrapped in an ActionRow so it reads like the other rows.
    let tunnel_dropdown = gtk4::DropDown::builder().build();
    tunnel_dropdown.set_valign(gtk4::Align::Center);
    tunnel_dropdown.set_size_request(200, -1);
    tunnel_dropdown.set_hexpand(false);
    // Type-to-search: a long connection list is impractical to scroll (user feedback).
    crate::dialogs::widgets::enable_string_search(&tunnel_dropdown);
    let tunnel_row = adw::ActionRow::builder()
        .title(i18n("Tunnel Through SSH"))
        .subtitle(i18n(
            "Browse via a SOCKS proxy over the chosen SSH connection (embedded or Chromium browser)",
        ))
        .build();
    tunnel_row.add_suffix(&tunnel_dropdown);
    tunnel_row.set_activatable_widget(Some(&tunnel_dropdown));
    embedded_group.add(&tunnel_row);

    // Floating navigation toolbar (issue #260). Positive here, stored as
    // `hide_floating_toolbar`; see the RDP panel for why. Unlike the remote
    // desktops, most of what this toolbar offers has a keyboard route, which is
    // what the subtitle says.
    let floating_toolbar_switch = SwitchRowBuilder::new("Navigation Toolbar")
        .subtitle(
            "Turn off to remove the floating toolbar and its reveal arrow. Keyboard shortcuts for back, forward, reload and zoom still work.",
        )
        .active(true)
        .build();
    embedded_group.add(&floating_toolbar_switch);

    content.append(&embedded_group);

    // === General Settings Group ===
    let general_group = adw::PreferencesGroup::builder()
        .title(i18n("General"))
        .build();

    let private_mode_switch = SwitchRowBuilder::new("Private / Incognito Mode")
        .subtitle("Open URL in a private browsing window (Firefox, Chrome, Brave)")
        .active(false)
        .build();
    general_group.add(&private_mode_switch);

    content.append(&general_group);

    // Connect browser mode changes to show/hide browser command group
    // and validate browser entry
    {
        let browser_entry_clone = browser_entry.clone();
        let browser_group_clone = browser_group.clone();
        browser_mode_combo.connect_selected_notify(move |combo| {
            let mode = browser_mode_from_combo_index(combo.selected());
            // Show browser command group only for Custom mode
            browser_group_clone.set_visible(mode == WebBrowserMode::Custom);
            // Clear error styling when switching away from Custom
            if mode != WebBrowserMode::Custom {
                browser_entry_clone.remove_css_class("error");
            }
        });

        // Set initial visibility based on default mode
        let initial_mode = browser_mode_from_combo_index(browser_mode_combo.selected());
        browser_group.set_visible(initial_mode == WebBrowserMode::Custom);
    }

    WebOptionsWidgets {
        container,
        browser_entry,
        private_mode_switch,
        browser_mode_combo,
        javascript_switch,
        user_agent_row,
        floating_toolbar_switch,
        tunnel_dropdown,
    }
}

/// Builds the Browser Mode `adw::ComboRow` with appropriate options.
///
/// When `web-embedded` is enabled: "Embedded", "System", "Custom"
/// When `web-embedded` is disabled: "System", "Custom"
fn build_browser_mode_combo() -> adw::ComboRow {
    let items: &[&str] = {
        #[cfg(feature = "web-embedded")]
        {
            &["Embedded", "System", "Custom"]
        }
        #[cfg(not(feature = "web-embedded"))]
        {
            &["System", "Custom"]
        }
    };

    let translated_items: Vec<String> = items.iter().map(|s| i18n(s)).collect();
    let item_strs: Vec<&str> = translated_items.iter().map(String::as_str).collect();
    let string_list = StringList::new(&item_strs);

    adw::ComboRow::builder()
        .title(i18n("Mode"))
        .subtitle(i18n("How the URL is opened"))
        .model(&string_list)
        .selected(0)
        .build()
}

/// Maps a `ComboRow` selection index to `WebBrowserMode`.
///
/// The mapping depends on whether `web-embedded` is enabled:
/// - With feature: 0=Embedded, 1=System, 2=Custom
/// - Without feature: 0=System, 1=Custom
#[must_use]
pub fn browser_mode_from_combo_index(index: u32) -> WebBrowserMode {
    #[cfg(feature = "web-embedded")]
    {
        match index {
            0 => WebBrowserMode::Embedded,
            1 => WebBrowserMode::System,
            _ => WebBrowserMode::Custom,
        }
    }
    #[cfg(not(feature = "web-embedded"))]
    {
        match index {
            0 => WebBrowserMode::System,
            _ => WebBrowserMode::Custom,
        }
    }
}

/// Maps a `WebBrowserMode` to the correct `ComboRow` selection index.
///
/// Handles the fallback case: a stored `Embedded` mode on a build without the
/// feature has no row of its own, so the combo shows System. Note what that
/// costs — saving the connection from this dialog then writes System, because
/// the combo is the only thing the save path reads. That is deliberate: the user
/// is looking at the value that will be stored. Every *other* save path leaves
/// the stored `Embedded` untouched, which is the difference from the silent
/// rewrite this used to cause on every connect.
#[must_use]
pub fn combo_index_from_browser_mode(mode: WebBrowserMode) -> u32 {
    #[cfg(feature = "web-embedded")]
    {
        match mode {
            WebBrowserMode::Embedded => 0,
            WebBrowserMode::System => 1,
            WebBrowserMode::Custom => 2,
        }
    }
    #[cfg(not(feature = "web-embedded"))]
    {
        match mode {
            // Embedded has no row in this build — show System, its runtime fallback
            WebBrowserMode::Embedded | WebBrowserMode::System => 0,
            WebBrowserMode::Custom => 1,
        }
    }
}

// `validate_web_options` used to live here, carrying
// `#[expect(dead_code, reason = "public API for upcoming connection-editor
// validation wiring")]`. That wiring had in fact already happened, in
// `ConnectionDialogData::validate`, and it was done better: a refusal there
// returns a message that reaches the user through `alert::show_error`, whereas
// this returned a bare `bool` and communicated the reason only as a CSS class.
// Keeping a second, weaker copy of the same rule invited the two to disagree
// about when a Custom browser command is acceptable.
//
// The live `error` styling applied while the mode combo changes is unaffected —
// see the `connect_selected_notify` handler above, which is feedback rather than
// a save gate.
