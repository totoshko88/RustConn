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
/// Handles the fallback case: if stored mode is Embedded but the feature
/// is disabled, returns the index for System.
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
            // Fallback: Embedded not available, select System
            WebBrowserMode::System => 0,
            WebBrowserMode::Custom => 1,
        }
    }
}

/// Validates the browser command entry for Custom mode.
///
/// Returns `true` if the configuration is valid (either not Custom mode,
/// or Custom mode with a non-empty browser command).
#[must_use]
#[expect(
    dead_code,
    reason = "public API for upcoming connection-editor validation wiring"
)]
pub fn validate_web_options(combo: &adw::ComboRow, browser_entry: &Entry) -> bool {
    let mode = browser_mode_from_combo_index(combo.selected());
    if mode == WebBrowserMode::Custom {
        let text = browser_entry.text();
        let is_valid = !text.trim().is_empty();
        if is_valid {
            browser_entry.remove_css_class("error");
        } else {
            browser_entry.add_css_class("error");
        }
        is_valid
    } else {
        browser_entry.remove_css_class("error");
        true
    }
}
