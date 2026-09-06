//! Keyboard shortcuts help dialog
//!
//! Displays all available keyboard shortcuts grouped by category, reflecting
//! any user-customized bindings from Settings → Keybindings.
//!
//! When built with the `adw-1-8` feature (libadwaita ≥ 1.8), uses the native
//! `AdwShortcutsDialog` with `AdwShortcutsSection` / `AdwShortcutsItem` widgets.
//! Otherwise falls back to a custom `adw::Dialog` with a searchable `ListBox`.
//!
//! The source of truth for all shortcuts is [`rustconn_core::default_keybindings()`]
//! combined with the user's keybinding overrides from settings.

use rustconn_core::config::keybindings::{
    KeybindingCategory, KeybindingDef, KeybindingSettings, default_keybindings,
};

use crate::i18n::i18n;

/// One line in the help dialog: a GTK accelerator and a translated label.
struct HelpEntry {
    /// GTK accelerator string, pipe-separated when the action has several.
    accel: String,
    /// Already-translated description.
    label: String,
}

/// Returns the effective accelerator for an action from the user's keybinding
/// settings, falling back to the definition's default when it is not overridden.
///
/// The settings are the source of truth rather than `accels_for_action` on the
/// live application, which is what this originally read. The application's
/// registration is not stable: [`crate::app::suspend_terminal_accels`] strips
/// every single-modifier accelerator while a terminal has focus (issue #216), and
/// the terminal usually *does* have focus when this dialog is opened. Reading the
/// live value there returned an empty list, which fell back to the default and
/// reintroduced the very bug this dialog was fixed for — a user override that the
/// help window does not show (issue #295).
fn effective_accel(overrides: Option<&KeybindingSettings>, def: &KeybindingDef) -> String {
    overrides.map_or_else(
        || def.default_accels.clone(),
        |settings| settings.get_accel(def).to_owned(),
    )
}

/// Shortcuts the application implements but does not offer for rebinding.
///
/// The keybinding registry exists to drive the *Settings ▸ Keybindings* page, so
/// it only holds actions the user can remap. These eleven are handled by widget
/// key controllers — sidebar row activation, VTE font zoom, the primary menu —
/// and never reach `set_accels_for_action`, so a dialog built purely from the
/// registry silently stopped documenting them. `F10` is one the project's own
/// GNOME HIG notes list as mandatory.
///
/// The labels are wrapped here rather than stored as bare `&'static str` on
/// purpose: `xgettext` only sees a string when it is an argument to `i18n()` at
/// the call site, and this file is in `POTFILES.in` while
/// `rustconn-core/src/config/keybindings.rs` is not.
fn fixed_shortcuts(category: KeybindingCategory) -> Vec<HelpEntry> {
    let entry = |accel: &str, label: String| HelpEntry {
        accel: accel.to_owned(),
        label,
    };
    match category {
        KeybindingCategory::Connections => vec![
            entry("<Control>e", i18n("Edit selected connection (sidebar)")),
            entry(
                "Delete",
                i18n("Delete selected connection or group (sidebar)"),
            ),
            entry("F2", i18n("Rename selected item")),
            entry("<Control>d", i18n("Duplicate connection (sidebar)")),
            entry("<Control>c", i18n("Copy connection")),
            entry("<Control>v", i18n("Paste connection")),
            entry("Return", i18n("Connect to selected")),
        ],
        KeybindingCategory::Terminal => vec![
            entry("<Control>plus", i18n("Zoom In (font size)")),
            entry("<Control>minus", i18n("Zoom Out (font size)")),
            entry("<Control>0", i18n("Reset Zoom")),
        ],
        KeybindingCategory::Application => {
            vec![entry("F10", i18n("Open Primary Menu"))]
        }
        KeybindingCategory::Navigation
        | KeybindingCategory::SplitView
        | KeybindingCategory::View => Vec::new(),
    }
}

/// Every fixed shortcut, in category order, as `(accelerator, translated label)`.
///
/// The conflict checker on the *Settings ▸ Keybindings* page needs exactly the
/// list this dialog shows: a combination a widget key controller already owns is
/// a conflict even though it is absent from the rebindable registry (issue
/// #295). It reads the list from here rather than keeping its own, because the
/// copy it started with held seven of the eleven — `Delete`, `F2`, `Return` and
/// `F10` were missing — with nothing but a comment to keep the two in step.
pub(crate) fn fixed_shortcut_accels() -> Vec<(String, String)> {
    KeybindingCategory::all()
        .iter()
        .copied()
        .flat_map(fixed_shortcuts)
        .map(|entry| (entry.accel, entry.label))
        .collect()
}

/// Every line the dialog shows for one category: registry entries first, then
/// the fixed ones, so a remappable shortcut is never buried under a static list.
fn entries_for(
    overrides: Option<&KeybindingSettings>,
    category: KeybindingCategory,
    defs: &[KeybindingDef],
) -> Vec<HelpEntry> {
    let mut entries: Vec<HelpEntry> = defs
        .iter()
        .filter(|def| def.category == category)
        .map(|def| HelpEntry {
            accel: effective_accel(overrides, def),
            label: i18n(&def.label),
        })
        .collect();
    entries.extend(fixed_shortcuts(category));
    entries
}

/// Makes the registry's own labels visible to `xgettext`.
///
/// [`default_keybindings`] and [`KeybindingCategory::label`] live in
/// `rustconn-core`, which is not in `POTFILES.in` — `xgettext` is pointed at the
/// GUI crate, and pointing it at core would drag in a great deal that is not
/// user-facing. The dialog therefore renders `i18n(&def.label)` against strings
/// that have no msgid unless they also appear literally in a file the extractor
/// reads. This function is that file.
///
/// Never called. Keep it in step with [`default_keybindings`]; the test at the
/// bottom of this file fails when the two drift.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "exists so xgettext can see labels that rustconn-core owns"
    )
)]
fn i18n_markers() -> Vec<String> {
    vec![
        // Categories
        i18n("Application"),
        i18n("Connections"),
        i18n("Navigation"),
        i18n("Terminal"),
        i18n("Split View"),
        i18n("View"),
        // Application
        i18n("Quit"),
        i18n("Keyboard Shortcuts"),
        i18n("Connection History"),
        i18n("Statistics"),
        i18n("Password Generator"),
        i18n("Wake On LAN"),
        i18n("SSH Tunnel Manager"),
        // Connections
        i18n("New Connection"),
        i18n("New Connection (Advanced)"),
        i18n("New Group"),
        i18n("Import"),
        i18n("Export"),
        i18n("Quick Connect"),
        i18n("Local Shell"),
        i18n("Move to Group"),
        // Navigation
        i18n("Search"),
        i18n("Focus Sidebar"),
        i18n("Focus Terminal"),
        i18n("Command Palette"),
        i18n("Command Palette (Commands)"),
        i18n("Settings"),
        // Terminal
        i18n("Copy"),
        i18n("Paste"),
        i18n("Find in Terminal"),
        i18n("Close Tab"),
        i18n("Next Tab"),
        i18n("Previous Tab"),
        i18n("Tab Overview"),
        i18n("Switch Tab"),
        i18n("Move Session to New Window"),
        // Split View
        i18n("Split Horizontal"),
        i18n("Split Vertical"),
        i18n("Close Pane"),
        i18n("Remove from Split"),
        i18n("Remove Split"),
        i18n("Focus Next Pane"),
        // View
        i18n("Toggle Fullscreen"),
        i18n("Toggle Sidebar"),
        i18n("Toggle Compact Interface"),
        i18n("Toggle Keyboard Passthrough"),
        i18n("Toggle Split Broadcast"),
    ]
}

// ============================================================
// Native AdwShortcutsDialog (libadwaita >= 1.8)
// ============================================================

#[cfg(feature = "adw-1-8")]
mod native {
    use gtk4::prelude::*;
    use libadwaita as adw;
    use rustconn_core::config::keybindings::{KeybindingCategory, KeybindingSettings};

    use super::{default_keybindings, entries_for};
    use crate::i18n::i18n;

    /// Keyboard shortcuts help dialog using native `AdwShortcutsDialog`
    pub struct ShortcutsDialog {
        dialog: adw::ShortcutsDialog,
    }

    impl ShortcutsDialog {
        /// Creates a new shortcuts dialog reflecting current user overrides.
        #[must_use]
        pub fn new(
            _parent: Option<&impl IsA<gtk4::Window>>,
            overrides: Option<&KeybindingSettings>,
        ) -> Self {
            let dialog = adw::ShortcutsDialog::new();

            let defaults = default_keybindings();

            // Driven by `KeybindingCategory::all()` rather than by walking the
            // registry and breaking on each category change: the fixed
            // shortcuts have to land in their own category's section, and the
            // registry is not guaranteed to be grouped (its "Application
            // (additional)" block already is not).
            for category in KeybindingCategory::all() {
                let entries = entries_for(overrides, *category, &defaults);
                if entries.is_empty() {
                    continue;
                }
                let section = adw::ShortcutsSection::new(Some(&i18n(category.label())));
                for entry in entries {
                    // `AdwShortcutsItem` takes the accelerator in the format
                    // `AdwShortcutLabel` accepts, which is GTK's: alternatives are
                    // separated by a *space*. Our own registry joins them with `|`
                    // (that is what `set_accels_for_action` is fed after a split),
                    // and handing that string over unchanged made
                    // `gtk_accelerator_parse` reject the whole thing — six items
                    // rendered with no keys at all, in exactly the builds that use
                    // this path, since `adw-1-8` is on for every package.
                    section.add(adw::ShortcutsItem::new(
                        &entry.label,
                        &entry.accel.replace('|', " "),
                    ));
                }
                dialog.add(section);
            }

            Self { dialog }
        }

        /// Shows the dialog
        pub fn show(&self, parent: Option<&impl IsA<gtk4::Widget>>) {
            use adw::prelude::AdwDialogExt;
            self.dialog.present(parent);
        }
    }
}

// ============================================================
// Legacy fallback (custom adw::Dialog with searchable ListBox)
// ============================================================

#[cfg(not(feature = "adw-1-8"))]
mod legacy {
    use adw::prelude::*;
    use gtk4::prelude::*;
    use gtk4::{
        Box as GtkBox, Label, ListBox, ListBoxRow, Orientation, ScrolledWindow, SearchEntry,
    };
    use libadwaita as adw;
    use rustconn_core::config::keybindings::{KeybindingCategory, KeybindingSettings};

    use super::{default_keybindings, entries_for};
    use crate::i18n::i18n;

    /// Keyboard shortcuts help dialog (legacy fallback)
    pub struct ShortcutsDialog {
        dialog: adw::Dialog,
    }

    impl ShortcutsDialog {
        /// Creates a new shortcuts dialog reflecting current user overrides.
        #[must_use]
        pub fn new(
            _parent: Option<&impl IsA<gtk4::Window>>,
            overrides: Option<&KeybindingSettings>,
        ) -> Self {
            let dialog = adw::Dialog::builder()
                .title(i18n("Keyboard Shortcuts"))
                .content_width(600)
                .content_height(500)
                .build();

            let header = adw::HeaderBar::new();

            let clamp = adw::Clamp::builder()
                .maximum_size(600)
                .tightening_threshold(400)
                .build();

            let content = GtkBox::new(Orientation::Vertical, 12);
            content.set_margin_top(12);
            content.set_margin_bottom(12);
            content.set_margin_start(12);
            content.set_margin_end(12);

            clamp.set_child(Some(&content));

            let toolbar_view = adw::ToolbarView::new();
            toolbar_view.add_top_bar(&header);
            toolbar_view.set_content(Some(&clamp));
            dialog.set_child(Some(&toolbar_view));

            let search_entry = SearchEntry::builder()
                .placeholder_text(i18n("Search shortcuts…"))
                .build();
            content.append(&search_entry);

            let scrolled = ScrolledWindow::builder()
                .hscrollbar_policy(gtk4::PolicyType::Never)
                .vscrollbar_policy(gtk4::PolicyType::Automatic)
                .vexpand(true)
                .build();

            let list_box = ListBox::builder()
                .selection_mode(gtk4::SelectionMode::None)
                .css_classes(["boxed-list"])
                .build();

            let defaults = default_keybindings();

            // Same category-driven order as the native path — see the comment
            // there for why it is not a walk over the registry.
            for category in KeybindingCategory::all() {
                let entries = entries_for(overrides, *category, &defaults);
                if entries.is_empty() {
                    continue;
                }
                let header_row = Self::create_category_header(&i18n(category.label()));
                list_box.append(&header_row);
                for entry in entries {
                    let keys = accel_key_parts(&entry.accel);
                    let row = Self::create_shortcut_row(&keys, &entry.label);
                    list_box.append(&row);
                }
            }

            scrolled.set_child(Some(&list_box));
            content.append(&scrolled);

            let list_box_clone = list_box.clone();
            search_entry.connect_search_changed(move |entry| {
                let search_text = entry.text().to_lowercase();
                Self::filter_shortcuts(&list_box_clone, &search_text);
            });

            Self { dialog }
        }

        fn create_category_header(category: &str) -> ListBoxRow {
            let row = ListBoxRow::new();
            row.set_activatable(false);
            row.set_selectable(false);

            let label = Label::builder()
                .label(category)
                .halign(gtk4::Align::Start)
                .margin_top(12)
                .margin_bottom(6)
                .margin_start(6)
                .css_classes(["heading"])
                .build();

            row.set_child(Some(&label));
            row.set_widget_name(&format!("category:{category}"));
            row
        }

        /// Builds one row from the already-split key parts.
        ///
        /// Takes the parts rather than the joined string on purpose: splitting
        /// `"Ctrl++"` back on `'+'` yields `["Ctrl", "", ""]` and rendered two
        /// empty keycaps for `<Control>plus`, which is a real binding (terminal
        /// zoom in). The separator cannot be recovered from the display string,
        /// so it is never collapsed into one.
        fn create_shortcut_row(keys: &[String], description: &str) -> ListBoxRow {
            let row = ListBoxRow::new();
            row.set_activatable(false);

            let hbox = GtkBox::new(Orientation::Horizontal, 12);
            hbox.set_margin_top(12);
            hbox.set_margin_bottom(12);
            hbox.set_margin_start(12);
            hbox.set_margin_end(12);

            let desc_label = Label::builder()
                .label(description)
                .halign(gtk4::Align::Start)
                .hexpand(true)
                .build();
            hbox.append(&desc_label);

            let keys_box = GtkBox::new(Orientation::Horizontal, 4);
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    let plus = Label::new(Some("+"));
                    plus.add_css_class("dim-label");
                    keys_box.append(&plus);
                }
                let key_label = Label::builder().label(key).css_classes(["keycap"]).build();
                keys_box.append(&key_label);
            }
            hbox.append(&keys_box);

            row.set_child(Some(&hbox));
            row.set_widget_name(&format!(
                "shortcut:{}:{}",
                keys.join("+").to_lowercase(),
                description.to_lowercase()
            ));
            row
        }

        fn filter_shortcuts(list_box: &ListBox, search_text: &str) {
            let mut row_index = 0;
            while let Some(row) = list_box.row_at_index(row_index) {
                let name = row.widget_name();
                let name_str = name.as_str();

                if name_str.starts_with("category:") {
                    row.set_visible(
                        search_text.is_empty()
                            || Self::category_has_matches(list_box, row_index, search_text),
                    );
                } else if name_str.starts_with("shortcut:") {
                    let visible = search_text.is_empty() || name_str.contains(search_text);
                    row.set_visible(visible);
                }

                row_index += 1;
            }
        }

        fn category_has_matches(
            list_box: &ListBox,
            category_index: i32,
            search_text: &str,
        ) -> bool {
            let mut row_index = category_index + 1;
            while let Some(row) = list_box.row_at_index(row_index) {
                let name = row.widget_name();
                let name_str = name.as_str();

                if name_str.starts_with("category:") {
                    break;
                }

                if name_str.starts_with("shortcut:") && name_str.contains(search_text) {
                    return true;
                }

                row_index += 1;
            }
            false
        }

        /// Shows the dialog
        pub fn show(&self, parent: Option<&impl IsA<gtk4::Widget>>) {
            self.dialog.present(parent);
        }
    }

    /// Joined view of [`accel_key_parts`], for the tests to assert against.
    ///
    /// The widget builds from the parts instead: once joined, a `+` key cannot be
    /// told from the separator.
    #[cfg(test)]
    pub(super) fn accel_to_human_readable(accel: &str) -> String {
        accel_key_parts(accel).join("+")
    }

    /// Splits a GTK accelerator into its display parts, in order.
    ///
    /// Pipe-separated multi-accels show only the first one for brevity.
    ///
    /// An unrecognised modifier token is passed through as written rather than
    /// relabelled. The previous fallback printed every unknown token as `Ctrl`,
    /// so a binding the parser did not know about was displayed as a different,
    /// plausible-looking shortcut — worse than showing the raw token, because the
    /// user has no way to tell it apart from a correct line.
    pub(super) fn accel_key_parts(accel: &str) -> Vec<String> {
        // Take only the first accelerator when multiple are pipe-separated.
        let first = accel.split('|').next().unwrap_or(accel);

        let mut parts: Vec<String> = Vec::new();
        let mut rest = first.trim();

        while let Some(after_open) = rest.strip_prefix('<') {
            let Some(close) = after_open.find('>') else {
                break;
            };
            let token = &after_open[..close];
            rest = &after_open[close + 1..];

            let rendered = match token.to_ascii_lowercase().as_str() {
                "control" | "ctrl" | "primary" => "Ctrl".to_owned(),
                "shift" => "Shift".to_owned(),
                "alt" | "mod1" => "Alt".to_owned(),
                "super" | "meta" => "Super".to_owned(),
                // Unknown token: show it, do not guess.
                _ => token.to_owned(),
            };
            parts.push(rendered);
        }

        // The remainder is the key name.
        let key_display = match rest.to_ascii_lowercase().as_str() {
            "return" | "kp_enter" => "Enter",
            "escape" => "Esc",
            "backspace" => "Backspace",
            "tab" | "iso_left_tab" => "Tab",
            "space" => "Space",
            "delete" | "kp_delete" => "Delete",
            "home" | "kp_home" => "Home",
            "end" | "kp_end" => "End",
            "page_up" | "kp_page_up" => "Page Up",
            "page_down" | "kp_page_down" => "Page Down",
            "up" | "kp_up" => "↑",
            "down" | "kp_down" => "↓",
            "left" | "kp_left" => "←",
            "right" | "kp_right" => "→",
            "plus" | "kp_add" => "+",
            "minus" | "kp_subtract" => "−",
            "comma" => ",",
            "period" => ".",
            "question" => "?",
            "exclam" => "!",
            "at" => "@",
            "grave" => "`",
            "percent" => "%",
            _ => rest,
        };

        // GTK writes letter keys lowercase (`<Control><Shift>c`), but a shortcut
        // is conventionally shown as `Ctrl+Shift+C`, which is what the previous
        // hand-written table did. Only single letters are touched: `F10` and
        // `Page Up` are already correct, and uppercasing them would not be.
        if key_display.len() == 1 && key_display.chars().all(char::is_alphabetic) {
            parts.push(key_display.to_uppercase());
        } else {
            parts.push(key_display.to_owned());
        }
        parts
    }
}

// ============================================================
// Public re-export — unified API regardless of feature
// ============================================================

#[cfg(not(feature = "adw-1-8"))]
pub use legacy::ShortcutsDialog;
#[cfg(feature = "adw-1-8")]
pub use native::ShortcutsDialog;

#[cfg(test)]
mod tests {
    use super::*;

    /// Every accelerator the application implements, whether or not the user can
    /// rebind it.
    ///
    /// This is the list the help dialog is supposed to document, and it exists
    /// because the dialog cannot be inspected from a test without a display. It
    /// replaces `registry_accelerators_are_documented_in_dialog`, which asserted
    /// that the registry was a subset of a hand-written array — the direction
    /// that became trivially true when the dialog started being *built* from the
    /// registry, at which point the array and the test were deleted together and
    /// eleven shortcuts stopped being documented with nothing to notice.
    ///
    /// A new shortcut belongs in `default_keybindings()` when the user should be
    /// able to remap it, and in `fixed_shortcuts()` when it is owned by a widget
    /// key controller. Either way it belongs here.
    const EXPECTED_ACCELS: &[&str] = &[
        // Application
        "<Control>q",
        "<Control>question|F1",
        "<Control>h",
        "<Control><Shift>i",
        "<Control>g",
        "<Control><Shift>l",
        "<Control>t",
        "F10",
        // Connections
        "<Control>n",
        "<Control><Shift>n",
        "<Control><Shift>g",
        "<Control>i",
        "<Control><Shift>e",
        "<Control><Shift>q",
        "<Control><Shift>t",
        "<Control>m",
        "<Control>e",
        "Delete",
        "F2",
        "<Control>d",
        "<Control>c",
        "<Control>v",
        "Return",
        // Navigation
        "<Control>f",
        "<Control>1|<Alt>1",
        "<Control>2|<Alt>2",
        "<Control>p",
        "<Control><Shift>p",
        "<Control>comma",
        // Terminal
        "<Control><Shift>c",
        "<Control><Shift>v",
        "<Control><Shift>f",
        "<Control>w|<Control><Shift>w",
        "<Control>Tab|<Control>Page_Down",
        "<Control><Shift>Tab|<Control>Page_Up",
        "<Control><Shift>o",
        "<Control>percent",
        "<Control><Shift>m",
        "<Control>plus",
        "<Control>minus",
        "<Control>0",
        // Split View
        "<Control><Shift>h",
        "<Control><Shift>s",
        "<Control><Shift>x",
        "<Control><Shift>r",
        "<Control><Shift>j",
        "<Control>grave",
        // View
        "F11",
        "F9",
        "<Control><Shift>d",
        "<Control><Shift>BackSpace",
        "<Control><Shift>b",
    ];

    /// Collects what the dialog would render, without needing a display.
    ///
    /// `None` for the application stands in for "not yet realised", which makes
    /// every registry entry fall back to its default accelerator — the same
    /// values `EXPECTED_ACCELS` records.
    fn documented_accels() -> Vec<String> {
        let defaults = default_keybindings();
        KeybindingCategory::all()
            .iter()
            .flat_map(|category| entries_for(None, *category, &defaults))
            .map(|entry| entry.accel)
            .collect()
    }

    #[test]
    fn every_implemented_shortcut_is_documented() {
        let documented = documented_accels();
        for expected in EXPECTED_ACCELS {
            assert!(
                documented.iter().any(|accel| accel == expected),
                "accelerator '{expected}' is implemented but the shortcuts dialog does not \
                 list it — add it to default_keybindings() or fixed_shortcuts()"
            );
        }
    }

    #[test]
    fn the_dialog_lists_nothing_undocumented() {
        // The other direction: a registry entry nobody recorded in
        // EXPECTED_ACCELS means the two lists have drifted, and the next person
        // to lose a shortcut will not be able to tell which side is right.
        for accel in documented_accels() {
            assert!(
                EXPECTED_ACCELS.contains(&accel.as_str()),
                "the dialog lists '{accel}', which EXPECTED_ACCELS does not — update the \
                 list so it stays the record of what the application implements"
            );
        }
    }

    #[test]
    fn no_shortcut_is_listed_twice() {
        let documented = documented_accels();
        let unique: std::collections::BTreeSet<&String> = documented.iter().collect();
        assert_eq!(
            unique.len(),
            documented.len(),
            "the dialog lists the same accelerator more than once: {documented:?}"
        );
    }

    #[test]
    fn fixed_shortcuts_are_not_in_the_rebindable_registry() {
        // A fixed shortcut that also reaches the registry would appear twice and,
        // worse, would be presented as remappable when it is not.
        let registry: Vec<String> = default_keybindings()
            .into_iter()
            .map(|def| def.default_accels)
            .collect();
        for category in KeybindingCategory::all() {
            for entry in fixed_shortcuts(*category) {
                assert!(
                    !registry.contains(&entry.accel),
                    "'{}' is in both fixed_shortcuts() and default_keybindings()",
                    entry.accel
                );
            }
        }
    }

    #[test]
    fn every_category_with_fixed_shortcuts_also_has_registry_entries() {
        // Not a hard requirement, but a category that exists only for fixed
        // shortcuts would render a section the Settings page cannot show, and
        // that asymmetry is worth failing on rather than discovering visually.
        let defaults = default_keybindings();
        for category in KeybindingCategory::all() {
            if fixed_shortcuts(*category).is_empty() {
                continue;
            }
            assert!(
                defaults.iter().any(|def| def.category == *category),
                "category {category:?} has fixed shortcuts but no registry entries"
            );
        }
    }

    #[test]
    fn i18n_markers_cover_every_registry_label() {
        // The markers function is the only reason the registry's labels have
        // msgids at all. Nothing calls it, so nothing would notice a label added
        // to `rustconn-core` and not mirrored here — it would simply render in
        // English in all 16 locales, which is exactly what happened when the
        // previous marker block was deleted.
        let markers = i18n_markers();
        for def in default_keybindings() {
            let translated = i18n(&def.label);
            assert!(
                markers.contains(&translated),
                "registry label '{}' has no i18n_markers() entry, so it has no msgid",
                def.label
            );
        }
        for category in KeybindingCategory::all() {
            let translated = i18n(category.label());
            assert!(
                markers.contains(&translated),
                "category label '{}' has no i18n_markers() entry",
                category.label()
            );
        }
    }

    #[cfg(not(feature = "adw-1-8"))]
    mod human_readable {
        use super::super::legacy::accel_to_human_readable;

        #[test]
        fn renders_modifiers_and_named_keys() {
            assert_eq!(accel_to_human_readable("<Control><Shift>c"), "Ctrl+Shift+C");
            assert_eq!(accel_to_human_readable("<Control>question"), "Ctrl+?");
            assert_eq!(accel_to_human_readable("<Control>grave"), "Ctrl+`");
            assert_eq!(accel_to_human_readable("<Control>plus"), "Ctrl++");
            assert_eq!(accel_to_human_readable("<Control>minus"), "Ctrl+−");
            assert_eq!(accel_to_human_readable("Return"), "Enter");
            assert_eq!(accel_to_human_readable("Delete"), "Delete");
            assert_eq!(accel_to_human_readable("F10"), "F10");
        }

        #[test]
        fn normalises_control_aliases() {
            assert_eq!(accel_to_human_readable("<Primary>c"), "Ctrl+C");
            assert_eq!(accel_to_human_readable("<Ctrl>c"), "Ctrl+C");
            assert_eq!(accel_to_human_readable("<Mod1>x"), "Alt+X");
        }

        #[test]
        fn shows_only_the_first_of_several_accelerators() {
            assert_eq!(
                accel_to_human_readable("<Control>w|<Control><Shift>w"),
                "Ctrl+W"
            );
        }

        #[test]
        fn passes_an_unknown_modifier_through_instead_of_calling_it_ctrl() {
            // The old fallback rendered this as "Ctrl+X", i.e. a real shortcut
            // the user has, attached to the wrong action.
            assert_eq!(accel_to_human_readable("<Hyper>x"), "Hyper+X");
        }
    }
}
