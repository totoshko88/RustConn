//! Keyboard shortcuts help dialog
//!
//! Displays all available keyboard shortcuts grouped by category.
//!
//! When built with the `adw-1-8` feature (libadwaita ≥ 1.8), uses the native
//! `AdwShortcutsDialog` with `AdwShortcutsSection` / `AdwShortcutsItem` widgets.
//! Otherwise falls back to a custom `adw::Window` with a searchable `ListBox`.

/// Keyboard shortcut entry definition
#[allow(dead_code)] // `accel` used only with `adw-1-8`; `keys` used only without it
struct ShortcutEntry {
    /// GTK accelerator string (e.g., "`<Control>`n")
    accel: &'static str,
    /// Human-readable key combo for the legacy fallback (e.g., "Ctrl+N")
    keys: &'static str,
    /// Translatable description of what the shortcut does
    description: &'static str,
    /// Category for grouping
    category: &'static str,
}

/// All keyboard shortcuts in the application
const SHORTCUTS: &[ShortcutEntry] = &[
    // Connection shortcuts
    ShortcutEntry {
        accel: "<Control>n",
        keys: "Ctrl+N",
        description: "New connection",
        category: "Connections",
    },
    ShortcutEntry {
        accel: "<Control>g",
        keys: "Ctrl+G",
        description: "New group",
        category: "Connections",
    },
    ShortcutEntry {
        accel: "<Control>i",
        keys: "Ctrl+I",
        description: "Import connections",
        category: "Connections",
    },
    ShortcutEntry {
        accel: "<Control>e",
        keys: "Ctrl+E",
        description: "Edit selected connection (sidebar)",
        category: "Connections",
    },
    ShortcutEntry {
        accel: "Delete",
        keys: "Delete",
        description: "Delete selected connection/group (sidebar)",
        category: "Connections",
    },
    ShortcutEntry {
        accel: "F2",
        keys: "F2",
        description: "Rename selected item",
        category: "Connections",
    },
    ShortcutEntry {
        accel: "<Control>d",
        keys: "Ctrl+D",
        description: "Duplicate connection (sidebar)",
        category: "Connections",
    },
    ShortcutEntry {
        accel: "<Control>c",
        keys: "Ctrl+C",
        description: "Copy connection",
        category: "Connections",
    },
    ShortcutEntry {
        accel: "<Control>v",
        keys: "Ctrl+V",
        description: "Paste connection",
        category: "Connections",
    },
    ShortcutEntry {
        accel: "<Control>m",
        keys: "Ctrl+M",
        description: "Move to group (sidebar)",
        category: "Connections",
    },
    ShortcutEntry {
        accel: "Return",
        keys: "Enter",
        description: "Connect to selected",
        category: "Connections",
    },
    // Terminal shortcuts
    ShortcutEntry {
        accel: "<Control><Shift>c",
        keys: "Ctrl+Shift+C",
        description: "Copy from terminal",
        category: "Terminal",
    },
    ShortcutEntry {
        accel: "<Control><Shift>v",
        keys: "Ctrl+Shift+V",
        description: "Paste to terminal",
        category: "Terminal",
    },
    ShortcutEntry {
        accel: "<Control><Shift>f",
        keys: "Ctrl+Shift+F",
        description: "Search in terminal",
        category: "Terminal",
    },
    ShortcutEntry {
        accel: "<Control><Shift>w",
        keys: "Ctrl+Shift+W",
        description: "Close current tab",
        category: "Terminal",
    },
    ShortcutEntry {
        accel: "<Control>Tab",
        keys: "Ctrl+Tab",
        description: "Next tab",
        category: "Terminal",
    },
    ShortcutEntry {
        accel: "<Control><Shift>Tab",
        keys: "Ctrl+Shift+Tab",
        description: "Previous tab",
        category: "Terminal",
    },
    ShortcutEntry {
        accel: "<Control>t",
        keys: "Ctrl+T",
        description: "Open local shell",
        category: "Terminal",
    },
    // Split view shortcuts
    ShortcutEntry {
        accel: "<Control><Shift>s",
        keys: "Ctrl+Shift+S",
        description: "Split vertical",
        category: "Split View",
    },
    ShortcutEntry {
        accel: "<Control><Shift>h",
        keys: "Ctrl+Shift+H",
        description: "Split horizontal",
        category: "Split View",
    },
    // Navigation shortcuts
    ShortcutEntry {
        accel: "<Control>f",
        keys: "Ctrl+F",
        description: "Focus search",
        category: "Navigation",
    },
    ShortcutEntry {
        accel: "<Control>l",
        keys: "Ctrl+L",
        description: "Focus sidebar",
        category: "Navigation",
    },
    ShortcutEntry {
        accel: "<Control>grave",
        keys: "Ctrl+`",
        description: "Focus terminal",
        category: "Navigation",
    },
    // Application shortcuts
    ShortcutEntry {
        accel: "<Control>comma",
        keys: "Ctrl+,",
        description: "Open settings",
        category: "Application",
    },
    ShortcutEntry {
        accel: "<Control>q",
        keys: "Ctrl+Q",
        description: "Quit application",
        category: "Application",
    },
    ShortcutEntry {
        accel: "<Control>question",
        keys: "Ctrl+?",
        description: "Show keyboard shortcuts",
        category: "Application",
    },
    ShortcutEntry {
        accel: "F1",
        keys: "F1",
        description: "Show about dialog",
        category: "Application",
    },
];

// ============================================================
// Native AdwShortcutsDialog (libadwaita >= 1.8)
// ============================================================

#[cfg(feature = "adw-1-8")]
mod native {
    use super::SHORTCUTS;
    use crate::i18n::i18n;
    use adw::prelude::*;
    use libadwaita as adw;

    /// Keyboard shortcuts help dialog using native `AdwShortcutsDialog`
    pub struct ShortcutsDialog {
        dialog: adw::ShortcutsDialog,
    }

    impl ShortcutsDialog {
        /// Creates a new shortcuts dialog
        #[must_use]
        pub fn new(_parent: Option<&impl IsA<gtk4::Window>>) -> Self {
            let dialog = adw::ShortcutsDialog::new();

            let mut current_category = "";
            let mut section: Option<adw::ShortcutsSection> = None;

            for shortcut in SHORTCUTS {
                if shortcut.category != current_category {
                    if let Some(s) = section.take() {
                        dialog.add(s);
                    }
                    current_category = shortcut.category;
                    section = Some(adw::ShortcutsSection::new(Some(&i18n(current_category))));
                }

                if let Some(ref s) = section {
                    let item = adw::ShortcutsItem::new(&i18n(shortcut.description), shortcut.accel);
                    s.add(item);
                }
            }

            if let Some(s) = section {
                dialog.add(s);
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
// Legacy fallback (custom adw::Window with searchable ListBox)
// ============================================================

#[cfg(not(feature = "adw-1-8"))]
mod legacy {
    use super::SHORTCUTS;
    use crate::i18n::i18n;
    use adw::prelude::*;
    use gtk4::prelude::*;
    use gtk4::{
        Box as GtkBox, Label, ListBox, ListBoxRow, Orientation, ScrolledWindow, SearchEntry,
    };
    use libadwaita as adw;

    /// Keyboard shortcuts help dialog (legacy fallback)
    pub struct ShortcutsDialog {
        window: adw::Window,
    }

    impl ShortcutsDialog {
        /// Creates a new shortcuts dialog
        #[must_use]
        pub fn new(parent: Option<&impl IsA<gtk4::Window>>) -> Self {
            let window = adw::Window::builder()
                .title(i18n("Keyboard Shortcuts"))
                .modal(true)
                .default_width(500)
                .default_height(400)
                .build();

            if let Some(p) = parent {
                window.set_transient_for(Some(p));
            }

            window.set_size_request(320, 280);

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
            window.set_content(Some(&toolbar_view));

            let search_entry = SearchEntry::builder()
                .placeholder_text(i18n("Search shortcuts..."))
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

            let mut current_category = "";
            for shortcut in SHORTCUTS {
                if shortcut.category != current_category {
                    current_category = shortcut.category;
                    let header_row = Self::create_category_header(&i18n(current_category));
                    list_box.append(&header_row);
                }
                let row = Self::create_shortcut_row(shortcut.keys, &i18n(shortcut.description));
                list_box.append(&row);
            }

            scrolled.set_child(Some(&list_box));
            content.append(&scrolled);

            let list_box_clone = list_box.clone();
            search_entry.connect_search_changed(move |entry| {
                let search_text = entry.text().to_lowercase();
                Self::filter_shortcuts(&list_box_clone, &search_text);
            });

            Self { window }
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

        fn create_shortcut_row(keys: &str, description: &str) -> ListBoxRow {
            let row = ListBoxRow::new();
            row.set_activatable(false);

            let hbox = GtkBox::new(Orientation::Horizontal, 12);
            hbox.set_margin_top(8);
            hbox.set_margin_bottom(8);
            hbox.set_margin_start(12);
            hbox.set_margin_end(12);

            let desc_label = Label::builder()
                .label(description)
                .halign(gtk4::Align::Start)
                .hexpand(true)
                .build();
            hbox.append(&desc_label);

            let keys_box = GtkBox::new(Orientation::Horizontal, 4);
            for key in keys.split('+') {
                let key_label = Label::builder().label(key).css_classes(["keycap"]).build();
                keys_box.append(&key_label);

                if key != keys.split('+').next_back().unwrap_or("") {
                    let plus = Label::new(Some("+"));
                    plus.add_css_class("dim-label");
                    keys_box.append(&plus);
                }
            }
            hbox.append(&keys_box);

            row.set_child(Some(&hbox));
            row.set_widget_name(&format!(
                "shortcut:{}:{}",
                keys.to_lowercase(),
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

        /// Shows the dialog (ignores parent — uses transient_for set in constructor)
        pub fn show(&self, _parent: Option<&impl IsA<gtk4::Widget>>) {
            self.window.present();
        }
    }
}

// ============================================================
// Public re-export — unified API regardless of feature
// ============================================================

#[cfg(feature = "adw-1-8")]
pub use native::ShortcutsDialog;

#[cfg(not(feature = "adw-1-8"))]
pub use legacy::ShortcutsDialog;
