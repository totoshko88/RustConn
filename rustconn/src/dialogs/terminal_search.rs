//! Terminal search dialog for finding text in VTE terminals
//!
//! Provides a search interface for VTE terminals with basic text search
//! and navigation between matches.

use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, CheckButton, HeaderBar, Label, Orientation, SearchEntry, Window,
};
use std::cell::RefCell;
use std::rc::Rc;
use vte4::prelude::*;
use vte4::Terminal;

/// Terminal search dialog for VTE terminals
pub struct TerminalSearchDialog {
    window: Window,
    search_entry: SearchEntry,
    case_sensitive: CheckButton,
    match_label: Label,
    terminal: Terminal,
    current_search: Rc<RefCell<String>>,
}

impl TerminalSearchDialog {
    /// Creates a new terminal search dialog
    #[must_use]
    pub fn new(parent: Option<&gtk4::Window>, terminal: Terminal) -> Self {
        let window = Window::builder()
            .title("Search in Terminal")
            .modal(true)
            .default_width(400)
            .default_height(150)
            .resizable(false)
            .build();

        if let Some(p) = parent {
            window.set_transient_for(Some(p));
        }

        // Create header bar
        let header = HeaderBar::new();
        header.set_show_title_buttons(false);

        let close_btn = Button::builder().label("Close").build();
        header.pack_start(&close_btn);

        window.set_titlebar(Some(&header));

        // Create main content
        let content = GtkBox::new(Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // Search entry
        let search_entry = SearchEntry::builder()
            .placeholder_text("Search text...")
            .hexpand(true)
            .build();
        content.append(&search_entry);

        // Options row
        let options_box = GtkBox::new(Orientation::Horizontal, 12);

        let case_sensitive = CheckButton::builder().label("Case sensitive").build();
        options_box.append(&case_sensitive);

        content.append(&options_box);

        // Navigation row
        let nav_box = GtkBox::new(Orientation::Horizontal, 6);

        let prev_btn = Button::builder()
            .icon_name("go-up-symbolic")
            .tooltip_text("Previous match")
            .build();
        nav_box.append(&prev_btn);

        let next_btn = Button::builder()
            .icon_name("go-down-symbolic")
            .tooltip_text("Next match")
            .build();
        nav_box.append(&next_btn);

        let match_label = Label::builder()
            .label("Enter text to search")
            .hexpand(true)
            .halign(gtk4::Align::Start)
            .build();
        nav_box.append(&match_label);

        content.append(&nav_box);
        window.set_child(Some(&content));

        let current_search = Rc::new(RefCell::new(String::new()));

        let dialog = Self {
            window,
            search_entry,
            case_sensitive,
            match_label,
            terminal,
            current_search,
        };

        dialog.setup_signals();
        dialog
    }

    /// Sets up signal handlers for the dialog
    fn setup_signals(&self) {
        let window = self.window.clone();
        let close_btn = self
            .window
            .titlebar()
            .and_then(|t| t.downcast::<HeaderBar>().ok())
            .and_then(|h| h.first_child())
            .and_then(|c| c.downcast::<Button>().ok());

        if let Some(btn) = close_btn {
            btn.connect_clicked(move |_| {
                window.close();
            });
        }

        // Search on text change
        let terminal = self.terminal.clone();
        let case_sensitive = self.case_sensitive.clone();
        let match_label = self.match_label.clone();
        let current_search = self.current_search.clone();

        self.search_entry.connect_search_changed(move |entry| {
            let text = entry.text();
            if text.is_empty() {
                match_label.set_text("Enter text to search");
                *current_search.borrow_mut() = String::new();
                return;
            }

            *current_search.borrow_mut() = text.to_string();
            Self::perform_search(&terminal, &text, case_sensitive.is_active(), &match_label);
        });

        // Update search when case sensitivity changes
        let terminal_clone = self.terminal.clone();
        let search_entry_clone = self.search_entry.clone();
        let case_sensitive_clone = self.case_sensitive.clone();
        let match_label_clone = self.match_label.clone();
        let _current_search_clone = self.current_search.clone();

        self.case_sensitive.connect_toggled(move |_| {
            let text = search_entry_clone.text();
            if !text.is_empty() {
                Self::perform_search(
                    &terminal_clone,
                    &text,
                    case_sensitive_clone.is_active(),
                    &match_label_clone,
                );
            }
        });

        // Navigation buttons
        let content = self
            .window
            .child()
            .and_then(|c| c.downcast::<GtkBox>().ok());
        if let Some(content_box) = content {
            if let Some(nav_box) = content_box
                .last_child()
                .and_then(|c| c.downcast::<GtkBox>().ok())
            {
                if let Some(prev_btn) = nav_box
                    .first_child()
                    .and_then(|c| c.downcast::<Button>().ok())
                {
                    let terminal_prev = self.terminal.clone();
                    prev_btn.connect_clicked(move |_| {
                        terminal_prev.search_find_previous();
                    });
                }

                if let Some(next_btn) = nav_box
                    .first_child()
                    .and_then(|c| c.next_sibling())
                    .and_then(|c| c.downcast::<Button>().ok())
                {
                    let terminal_next = self.terminal.clone();
                    next_btn.connect_clicked(move |_| {
                        terminal_next.search_find_next();
                    });
                }
            }
        }

        // Handle Enter key to find next
        let terminal_enter = self.terminal.clone();
        self.search_entry.connect_activate(move |_| {
            terminal_enter.search_find_next();
        });

        // Handle Escape key to close
        let window_escape = self.window.clone();
        let key_controller = gtk4::EventControllerKey::new();
        key_controller.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                window_escape.close();
                return gtk4::glib::Propagation::Stop;
            }
            gtk4::glib::Propagation::Proceed
        });
        self.window.add_controller(key_controller);
    }

    /// Performs a search in the terminal using basic text search
    fn perform_search(terminal: &Terminal, text: &str, case_sensitive: bool, match_label: &Label) {
        // Escape regex special characters for literal search
        let pattern = regex::escape(text);

        // Create regex with appropriate flags
        let regex_result = if case_sensitive {
            vte4::Regex::for_search(&pattern, 0)
        } else {
            // VTE4 doesn't expose regex flags directly, so we'll use a simple approach
            vte4::Regex::for_search(&format!("(?i){pattern}"), 0)
        };

        if let Ok(regex) = regex_result {
            terminal.search_set_regex(Some(&regex), 0);

            // Try to find first match
            if terminal.search_find_next() {
                match_label.set_text("Found matches");
            } else {
                match_label.set_text("No matches found");
            }
        } else {
            match_label.set_text("Search error");
            terminal.search_set_regex(None, 0);
        }
    }

    /// Shows the dialog
    pub fn show(&self) {
        self.window.present();
        self.search_entry.grab_focus();
    }

    /// Returns the underlying window
    #[must_use]
    pub const fn window(&self) -> &Window {
        &self.window
    }
}
