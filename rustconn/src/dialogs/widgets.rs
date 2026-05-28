//! Reusable widget builders for dialogs
//!
//! This module provides builder patterns for common libadwaita widgets used
//! across dialogs, reducing code duplication and ensuring consistent
//! styling following GNOME HIG.

use crate::i18n::i18n;
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Button, CheckButton, DropDown, Entry, SpinButton, StringList};
use libadwaita as adw;

/// Creates a standard dialog header bar following GNOME HIG.
///
/// Returns `(header_bar, end_button)`. The action button is placed at the end
/// with the `suggested-action` CSS class. Since `adw::Dialog` natively handles
/// Escape to close, no Cancel button is needed.
///
/// # Arguments
///
/// * `end_label` - Label for the end (action) button.
#[must_use]
pub fn dialog_header(end_label: &str) -> (adw::HeaderBar, Button) {
    let header = adw::HeaderBar::new();
    header.set_show_end_title_buttons(false);
    header.set_show_start_title_buttons(false);

    let end_btn = Button::builder()
        .label(i18n(end_label))
        .css_classes(["suggested-action"])
        .build();

    header.pack_end(&end_btn);

    (header, end_btn)
}

/// Common label strings used across dialogs
pub mod labels {
    use crate::i18n::i18n;

    /// Label for root group in group dropdown
    pub fn root_group() -> String {
        i18n("(Root)")
    }

    /// Label for no selection in dropdowns
    pub fn none_label() -> String {
        i18n("(None)")
    }

    /// Label when no SSH keys are loaded from agent
    pub fn no_keys_loaded() -> String {
        i18n("(No keys loaded)")
    }
}

/// Builder for creating `adw::ActionRow` with a `CheckButton` suffix.
///
/// # Example
/// ```ignore
/// let (row, checkbox) = CheckboxRowBuilder::new("Enable Feature")
///     .subtitle("Description of the feature")
///     .active(true)
///     .build();
/// group.add(&row);
/// ```
#[derive(Default)]
pub struct CheckboxRowBuilder {
    title: String,
    subtitle: Option<String>,
    active: bool,
}

impl CheckboxRowBuilder {
    /// Creates a new builder with the given title.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
            active: false,
        }
    }

    /// Sets the subtitle (description) for the row.
    #[must_use]
    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Sets the initial active state of the checkbox.
    #[must_use]
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    /// Builds the `ActionRow` and `CheckButton`.
    ///
    /// Returns a tuple of (row, checkbox) for adding to a preferences group
    /// and connecting signals.
    #[must_use]
    pub fn build(self) -> (adw::ActionRow, CheckButton) {
        let checkbox = CheckButton::new();
        checkbox.set_active(self.active);

        let mut row_builder = adw::ActionRow::builder()
            .title(i18n(&self.title))
            .activatable_widget(&checkbox);

        if let Some(subtitle) = &self.subtitle {
            row_builder = row_builder.subtitle(i18n(subtitle));
        }

        let row = row_builder.build();
        row.add_suffix(&checkbox);

        (row, checkbox)
    }
}

/// Builder for creating `adw::ActionRow` with an `Entry` suffix.
///
/// # i18n
///
/// `title`, `subtitle` and `placeholder` are passed through to the widget
/// **verbatim** — callers MUST wrap user-facing strings in `i18n()` /
/// `i18n_f()` at the call site so `xgettext` extracts them into the POT.
///
/// # Example
/// ```ignore
/// let (row, entry) = EntryRowBuilder::new(&i18n("Hostname"))
///     .subtitle(&i18n("Server address"))
///     .placeholder(&i18n("example.com"))
///     .build();
/// group.add(&row);
/// ```
#[derive(Default)]
pub struct EntryRowBuilder {
    title: String,
    subtitle: Option<String>,
    placeholder: Option<String>,
    text: Option<String>,
}

impl EntryRowBuilder {
    /// Creates a new builder with the given **already-translated** title.
    ///
    /// Pass `&i18n("...")` so `xgettext` picks up the literal.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
            placeholder: None,
            text: None,
        }
    }

    /// Sets the **already-translated** subtitle (description) for the row.
    #[must_use]
    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Sets the **already-translated** placeholder text for the entry.
    #[must_use]
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    /// Sets the initial text value (raw, not translated).
    #[must_use]
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    /// Builds the `ActionRow` and `Entry`.
    ///
    /// Returns a tuple of (row, entry) for adding to a preferences group
    /// and connecting signals.
    #[must_use]
    pub fn build(self) -> (adw::ActionRow, Entry) {
        let mut entry_builder = Entry::builder().hexpand(true).valign(gtk4::Align::Center);

        if let Some(placeholder) = &self.placeholder {
            entry_builder = entry_builder.placeholder_text(placeholder);
        }

        let entry = entry_builder.build();

        if let Some(text) = &self.text {
            entry.set_text(text);
        }

        // title / subtitle are expected to be already translated by the caller
        // (see struct doc comment).
        let mut row_builder = adw::ActionRow::builder().title(&self.title);

        if let Some(subtitle) = &self.subtitle {
            row_builder = row_builder.subtitle(subtitle);
        }

        let row = row_builder.build();
        row.add_suffix(&entry);

        (row, entry)
    }
}

/// Builder for creating `adw::ActionRow` with a `SpinButton` suffix.
///
/// # Example
/// ```ignore
/// let (row, spin) = SpinRowBuilder::new("Port")
///     .subtitle("Connection port")
///     .range(1.0, 65535.0)
///     .value(22.0)
///     .build();
/// group.add(&row);
/// ```
#[derive(Default)]
pub struct SpinRowBuilder {
    title: String,
    subtitle: Option<String>,
    min: f64,
    max: f64,
    step: f64,
    value: f64,
    digits: u32,
}

impl SpinRowBuilder {
    /// Creates a new builder with the given title.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
            min: 0.0,
            max: 100.0,
            step: 1.0,
            value: 0.0,
            digits: 0,
        }
    }

    /// Sets the subtitle (description) for the row.
    #[must_use]
    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Sets the range (min, max) for the spin button.
    #[must_use]
    pub fn range(mut self, min: f64, max: f64) -> Self {
        self.min = min;
        self.max = max;
        self
    }

    /// Sets the step increment.
    #[must_use]
    pub fn step(mut self, step: f64) -> Self {
        self.step = step;
        self
    }

    /// Sets the initial value.
    #[must_use]
    pub fn value(mut self, value: f64) -> Self {
        self.value = value;
        self
    }

    /// Sets the number of decimal digits to display.
    #[must_use]
    pub fn digits(mut self, digits: u32) -> Self {
        self.digits = digits;
        self
    }

    /// Builds the `ActionRow` and `SpinButton`.
    ///
    /// Returns a tuple of (row, spin_button) for adding to a preferences group
    /// and connecting signals.
    #[must_use]
    pub fn build(self) -> (adw::ActionRow, SpinButton) {
        let adjustment =
            gtk4::Adjustment::new(self.value, self.min, self.max, self.step, 10.0, 0.0);

        let spin = SpinButton::builder()
            .adjustment(&adjustment)
            .digits(self.digits)
            .valign(gtk4::Align::Center)
            .build();

        let mut row_builder = adw::ActionRow::builder().title(i18n(&self.title));

        if let Some(subtitle) = &self.subtitle {
            row_builder = row_builder.subtitle(i18n(subtitle));
        }

        let row = row_builder.build();
        row.add_suffix(&spin);

        (row, spin)
    }
}

/// Builder for creating `adw::ActionRow` with a `DropDown` suffix.
///
/// # Example
/// ```ignore
/// let (row, dropdown) = DropdownRowBuilder::new("Protocol")
///     .subtitle("Connection protocol")
///     .items(&["SSH", "RDP", "VNC"])
///     .selected(0)
///     .build();
/// group.add(&row);
/// ```
#[derive(Default)]
pub struct DropdownRowBuilder {
    title: String,
    subtitle: Option<String>,
    items: Vec<String>,
    selected: u32,
}

impl DropdownRowBuilder {
    /// Creates a new builder with the given title.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
            items: Vec::new(),
            selected: 0,
        }
    }

    /// Sets the subtitle (description) for the row.
    #[must_use]
    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Sets the dropdown items.
    #[must_use]
    pub fn items(mut self, items: &[&str]) -> Self {
        self.items = items.iter().map(|s| (*s).to_string()).collect();
        self
    }

    /// Sets the initially selected item index.
    #[must_use]
    pub fn selected(mut self, index: u32) -> Self {
        self.selected = index;
        self
    }

    /// Builds the `ActionRow` and `DropDown`.
    ///
    /// Returns a tuple of (row, dropdown) for adding to a preferences group
    /// and connecting signals.
    #[must_use]
    pub fn build(self) -> (adw::ActionRow, DropDown) {
        let items_refs: Vec<&str> = self.items.iter().map(String::as_str).collect();
        let string_list = StringList::new(&items_refs);
        let dropdown = DropDown::new(Some(string_list), gtk4::Expression::NONE);
        dropdown.set_selected(self.selected);

        let mut row_builder = adw::ActionRow::builder().title(i18n(&self.title));

        if let Some(subtitle) = &self.subtitle {
            row_builder = row_builder.subtitle(i18n(subtitle));
        }

        let row = row_builder.build();
        row.add_suffix(&dropdown);

        (row, dropdown)
    }
}

/// Builder for creating `adw::SwitchRow`.
///
/// # Example
/// ```ignore
/// let switch_row = SwitchRowBuilder::new("Enable Feature")
///     .subtitle("Description of the feature")
///     .active(true)
///     .build();
/// group.add(&switch_row);
/// ```
#[derive(Default)]
pub struct SwitchRowBuilder {
    title: String,
    subtitle: Option<String>,
    active: bool,
}

impl SwitchRowBuilder {
    /// Creates a new builder with the given title.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
            active: false,
        }
    }

    /// Sets the subtitle (description) for the row.
    #[must_use]
    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Sets the initial active state of the switch.
    #[must_use]
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    /// Builds the `SwitchRow`.
    ///
    /// Returns the switch row for adding to a preferences group.
    #[must_use]
    pub fn build(self) -> adw::SwitchRow {
        let mut builder = adw::SwitchRow::builder()
            .title(i18n(&self.title))
            .active(self.active);

        if let Some(subtitle) = &self.subtitle {
            builder = builder.subtitle(i18n(subtitle));
        }

        builder.build()
    }
}

/// Marker function for xgettext to discover dialog header button labels.
/// These strings are used indirectly via `dialog_header()` and would otherwise
/// be invisible to the POT extraction tool.
///
/// This function is never called at runtime.
#[allow(
    dead_code,
    reason = "kept alive for GTK widget lifecycle / future API exposure"
)]
fn _i18n_markers() {
    // Button labels used in dialog_header() across the codebase
    i18n("Close");
    i18n("Save");
    i18n("Create");
    i18n("Export");
    i18n("Import");
    i18n("Copy");
    i18n("Connect");
    i18n("Send");
    i18n("Use Template");
    i18n("Done");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_labels_functions() {
        // Labels return non-empty strings (actual content depends on locale)
        assert!(!labels::root_group().is_empty());
        assert!(!labels::none_label().is_empty());
        assert!(!labels::no_keys_loaded().is_empty());
    }
}
