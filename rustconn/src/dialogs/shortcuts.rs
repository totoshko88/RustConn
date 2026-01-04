//! Keyboard shortcuts help dialog
//!
//! Displays all available keyboard shortcuts in a searchable dialog.

use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, HeaderBar, Label, ListBox, ListBoxRow, Orientation, ScrolledWindow, SearchEntry,
    Window,
};

/// Keyboard shortcut entry
struct ShortcutEntry {
    /// Shortcut key combination (e.g., "Ctrl+N")
    keys: &'static str,
    /// Description of what the shortcut does
    description: &'static str,
    /// Category for grouping
    category: &'static str,
}

/// All keyboard shortcuts in the application
const SHORTCUTS: &[ShortcutEntry] = &[
    // Connection shortcuts
    ShortcutEntry {
        keys: "Ctrl+N",
        description: "New connection",
        category: "Connections",
    },
    ShortcutEntry {
        keys: "Ctrl+G",
        description: "New group",
        category: "Connections",
    },
    ShortcutEntry {
        keys: "Ctrl+I",
        description: "Import connections",
        category: "Connections",
    },
    ShortcutEntry {
        keys: "Ctrl+E",
        description: "Export connections",
        category: "Connections",
    },
    ShortcutEntry {
        keys: "Delete",
        description: "Delete selected connection/group",
        category: "Connections",
    },
    ShortcutEntry {
        keys: "F2",
        description: "Rename selected item",
        category: "Connections",
    },
    ShortcutEntry {
        keys: "Ctrl+D",
        description: "Duplicate connection",
        category: "Connections",
    },
    ShortcutEntry {
        keys: "Ctrl+C",
        description: "Copy connection",
        category: "Connections",
    },
    ShortcutEntry {
        keys: "Ctrl+V",
        description: "Paste connection",
        category: "Connections",
    },
    ShortcutEntry {
        keys: "Enter",
        description: "Connect to selected",
        category: "Connections",
    },
    // Terminal shortcuts
    ShortcutEntry {
        keys: "Ctrl+Shift+C",
        description: "Copy from terminal",
        category: "Terminal",
    },
    ShortcutEntry {
        keys: "Ctrl+Shift+V",
        description: "Paste to terminal",
        category: "Terminal",
    },
    ShortcutEntry {
        keys: "Ctrl+Shift+F",
        description: "Search in terminal",
        category: "Terminal",
    },
    ShortcutEntry {
        keys: "Ctrl+W",
        description: "Close current tab",
        category: "Terminal",
    },
    ShortcutEntry {
        keys: "Ctrl+Tab",
        description: "Next tab",
        category: "Terminal",
    },
    ShortcutEntry {
        keys: "Ctrl+Shift+Tab",
        description: "Previous tab",
        category: "Terminal",
    },
    ShortcutEntry {
        keys: "Ctrl+T",
        description: "Open local shell",
        category: "Terminal",
    },
    // Split view shortcuts
    ShortcutEntry {
        keys: "Ctrl+Shift+S",
        description: "Split vertical",
        category: "Split View",
    },
    ShortcutEntry {
        keys: "Ctrl+Shift+H",
        description: "Split horizontal",
        category: "Split View",
    },
    // Navigation shortcuts
    ShortcutEntry {
        keys: "Ctrl+F",
        description: "Focus search",
        category: "Navigation",
    },
    ShortcutEntry {
        keys: "Ctrl+L",
        description: "Focus sidebar",
        category: "Navigation",
    },
    ShortcutEntry {
        keys: "Ctrl+`",
        description: "Focus terminal",
        category: "Navigation",
    },
    // Application shortcuts
    ShortcutEntry {
        keys: "Ctrl+,",
        description: "Open settings",
        category: "Application",
    },
    ShortcutEntry {
        keys: "Ctrl+Q",
        description: "Quit application",
        category: "Application",
    },
    ShortcutEntry {
        keys: "Ctrl+?",
        description: "Show keyboard shortcuts",
        category: "Application",
    },
    ShortcutEntry {
        keys: "F1",
        description: "Show about dialog",
        category: "Application",
    },
];

/// Keyboard shortcuts help dialog
pub struct ShortcutsDialog {
    window: Window,
}

impl ShortcutsDialog {
    /// Creates a new shortcuts dialog
    #[must_use]
    pub fn new(parent: Option<&impl IsA<Window>>) -> Self {
        let window = Window::builder()
            .title("Keyboard Shortcuts")
            .modal(true)
            .default_width(500)
            .default_height(600)
            .build();

        if let Some(p) = parent {
            window.set_transient_for(Some(p));
        }

        // Header bar
        let header = HeaderBar::new();
        header.set_show_title_buttons(true);
        window.set_titlebar(Some(&header));

        // Main content
        let content = GtkBox::new(Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // Search entry
        let search_entry = SearchEntry::builder()
            .placeholder_text("Search shortcuts...")
            .build();
        content.append(&search_entry);

        // Scrolled list
        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .build();

        let list_box = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();

        // Group shortcuts by category
        let mut current_category = "";
        for shortcut in SHORTCUTS {
            // Add category header if changed
            if shortcut.category != current_category {
                current_category = shortcut.category;
                let header_row = Self::create_category_header(current_category);
                list_box.append(&header_row);
            }

            let row = Self::create_shortcut_row(shortcut.keys, shortcut.description);
            list_box.append(&row);
        }

        scrolled.set_child(Some(&list_box));
        content.append(&scrolled);

        // Connect search filtering
        let list_box_clone = list_box.clone();
        search_entry.connect_search_changed(move |entry| {
            let search_text = entry.text().to_lowercase();
            Self::filter_shortcuts(&list_box_clone, &search_text);
        });

        window.set_child(Some(&content));

        Self { window }
    }

    /// Creates a category header row
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

    /// Creates a shortcut row with keys and description
    fn create_shortcut_row(keys: &str, description: &str) -> ListBoxRow {
        let row = ListBoxRow::new();
        row.set_activatable(false);

        let hbox = GtkBox::new(Orientation::Horizontal, 12);
        hbox.set_margin_top(8);
        hbox.set_margin_bottom(8);
        hbox.set_margin_start(12);
        hbox.set_margin_end(12);

        // Description on the left
        let desc_label = Label::builder()
            .label(description)
            .halign(gtk4::Align::Start)
            .hexpand(true)
            .build();
        hbox.append(&desc_label);

        // Keys on the right with keyboard styling
        let keys_box = GtkBox::new(Orientation::Horizontal, 4);
        for key in keys.split('+') {
            let key_label = Label::builder().label(key).css_classes(["keycap"]).build();
            keys_box.append(&key_label);

            // Add "+" separator between keys (except for last)
            if key != keys.split('+').next_back().unwrap_or("") {
                let plus = Label::new(Some("+"));
                plus.add_css_class("dim-label");
                keys_box.append(&plus);
            }
        }
        hbox.append(&keys_box);

        row.set_child(Some(&hbox));
        // Store searchable text in widget name for filtering
        row.set_widget_name(&format!(
            "shortcut:{}:{}",
            keys.to_lowercase(),
            description.to_lowercase()
        ));
        row
    }

    /// Filters shortcuts based on search text
    fn filter_shortcuts(list_box: &ListBox, search_text: &str) {
        let mut row_index = 0;
        while let Some(row) = list_box.row_at_index(row_index) {
            let name = row.widget_name();
            let name_str = name.as_str();

            if name_str.starts_with("category:") {
                // Category headers - show if any child matches
                // For simplicity, always show categories when searching
                row.set_visible(
                    search_text.is_empty()
                        || Self::category_has_matches(list_box, row_index, search_text),
                );
            } else if name_str.starts_with("shortcut:") {
                // Shortcut rows - filter by search text
                let visible = search_text.is_empty() || name_str.contains(search_text);
                row.set_visible(visible);
            }

            row_index += 1;
        }
    }

    /// Checks if a category has any matching shortcuts
    fn category_has_matches(list_box: &ListBox, category_index: i32, search_text: &str) -> bool {
        let mut row_index = category_index + 1;
        while let Some(row) = list_box.row_at_index(row_index) {
            let name = row.widget_name();
            let name_str = name.as_str();

            if name_str.starts_with("category:") {
                // Hit next category, stop
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
    pub fn show(&self) {
        self.window.present();
    }
}

/// CSS styles for keyboard shortcuts dialog
pub const SHORTCUTS_CSS: &str = r"
.keycap {
    background-color: alpha(@theme_fg_color, 0.1);
    border: 1px solid alpha(@borders, 0.5);
    border-radius: 4px;
    padding: 2px 8px;
    font-family: monospace;
    font-size: 0.9em;
    min-width: 24px;
    text-align: center;
}
";
