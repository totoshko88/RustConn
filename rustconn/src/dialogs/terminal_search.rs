//! Terminal search dialog for finding text in VTE terminals
//!
//! Provides a search interface for VTE terminals with regex support,
//! highlight all matches, and navigation between matches.

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, CheckButton, Label, Orientation, SearchEntry};
use libadwaita as adw;
use std::cell::RefCell;
use std::rc::Rc;
use vte4::prelude::*;
use vte4::Terminal;

/// Terminal search dialog for VTE terminals
pub struct TerminalSearchDialog {
    window: adw::Window,
    search_entry: SearchEntry,
    case_sensitive: CheckButton,
    regex_toggle: CheckButton,
    highlight_all: CheckButton,
    match_label: Label,
    terminal: Terminal,
    current_search: Rc<RefCell<String>>,
    close_btn: Button,
    prev_btn: Button,
    next_btn: Button,
}

impl TerminalSearchDialog {
    /// Creates a new terminal search dialog
    #[must_use]
    pub fn new(parent: Option<&gtk4::Window>, terminal: Terminal) -> Self {
        let window = adw::Window::builder()
            .title("Search in Terminal")
            .modal(true)
            .default_width(400)
            .default_height(180)
            .build();

        if let Some(p) = parent {
            window.set_transient_for(Some(p));
        }

        window.set_size_request(280, -1);

        // Create header bar
        let header = adw::HeaderBar::new();
        header.set_show_end_title_buttons(false);
        header.set_show_start_title_buttons(false);

        let close_btn = Button::builder().label("Close").build();
        header.pack_start(&close_btn);

        // Create main content
        let content = GtkBox::new(Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // Use ToolbarView for adw::Window
        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&content));
        window.set_content(Some(&toolbar_view));

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

        let regex_toggle = CheckButton::builder()
            .label("Regex")
            .tooltip_text("Use regular expression pattern")
            .build();
        options_box.append(&regex_toggle);

        let highlight_all = CheckButton::builder()
            .label("Highlight All")
            .tooltip_text("Highlight all matches in terminal")
            .active(true)
            .build();
        options_box.append(&highlight_all);

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

        let current_search = Rc::new(RefCell::new(String::new()));

        let dialog = Self {
            window,
            search_entry,
            case_sensitive,
            regex_toggle,
            highlight_all,
            match_label,
            terminal,
            current_search,
            close_btn,
            prev_btn,
            next_btn,
        };

        dialog.setup_signals();
        dialog
    }

    /// Sets up signal handlers for the dialog
    fn setup_signals(&self) {
        // Close button handler — clear highlights on close
        let window = self.window.clone();
        let terminal_close = self.terminal.clone();
        self.close_btn.connect_clicked(move |_| {
            terminal_close.match_remove_all();
            terminal_close.search_set_regex(None, 0);
            window.close();
        });

        // Search on text change
        let terminal = self.terminal.clone();
        let case_sensitive = self.case_sensitive.clone();
        let regex_toggle = self.regex_toggle.clone();
        let highlight_all = self.highlight_all.clone();
        let match_label = self.match_label.clone();
        let current_search = self.current_search.clone();

        self.search_entry.connect_search_changed(move |entry| {
            let text = entry.text();
            if text.is_empty() {
                match_label.set_text("Enter text to search");
                *current_search.borrow_mut() = String::new();
                terminal.match_remove_all();
                terminal.search_set_regex(None, 0);
                return;
            }

            *current_search.borrow_mut() = text.to_string();
            Self::perform_search(
                &terminal,
                &text,
                case_sensitive.is_active(),
                regex_toggle.is_active(),
                highlight_all.is_active(),
                &match_label,
            );
        });

        // Re-search when any toggle changes
        let make_toggle_handler = |term: Terminal,
                                   entry: SearchEntry,
                                   cs: CheckButton,
                                   rx: CheckButton,
                                   hl: CheckButton,
                                   lbl: Label| {
            move |_: &CheckButton| {
                let text = entry.text();
                if !text.is_empty() {
                    Self::perform_search(
                        &term,
                        &text,
                        cs.is_active(),
                        rx.is_active(),
                        hl.is_active(),
                        &lbl,
                    );
                }
            }
        };

        self.case_sensitive.connect_toggled(make_toggle_handler(
            self.terminal.clone(),
            self.search_entry.clone(),
            self.case_sensitive.clone(),
            self.regex_toggle.clone(),
            self.highlight_all.clone(),
            self.match_label.clone(),
        ));

        self.regex_toggle.connect_toggled(make_toggle_handler(
            self.terminal.clone(),
            self.search_entry.clone(),
            self.case_sensitive.clone(),
            self.regex_toggle.clone(),
            self.highlight_all.clone(),
            self.match_label.clone(),
        ));

        self.highlight_all.connect_toggled({
            let terminal = self.terminal.clone();
            let search_entry = self.search_entry.clone();
            let case_sensitive = self.case_sensitive.clone();
            let regex_toggle = self.regex_toggle.clone();
            move |btn| {
                let text = search_entry.text();
                if text.is_empty() {
                    return;
                }

                // Only toggle hover-highlight without navigating
                terminal.match_remove_all();
                if btn.is_active() {
                    let pattern = if regex_toggle.is_active() {
                        if case_sensitive.is_active() {
                            text.to_string()
                        } else {
                            format!("(?i){text}")
                        }
                    } else {
                        let escaped = regex::escape(&text);
                        if case_sensitive.is_active() {
                            escaped
                        } else {
                            format!("(?i){escaped}")
                        }
                    };
                    if let Ok(hl_regex) = vte4::Regex::for_search(&pattern, 0) {
                        terminal.match_add_regex(&hl_regex, 0);
                    }
                }
            }
        });

        // Navigation buttons
        let terminal_prev = self.terminal.clone();
        self.prev_btn.connect_clicked(move |_| {
            terminal_prev.search_find_previous();
        });

        let terminal_next = self.terminal.clone();
        self.next_btn.connect_clicked(move |_| {
            terminal_next.search_find_next();
        });

        // Handle Enter key to find next
        let terminal_enter = self.terminal.clone();
        self.search_entry.connect_activate(move |_| {
            terminal_enter.search_find_next();
        });

        // Handle Escape key to close — clear highlights
        let window_escape = self.window.clone();
        let terminal_escape = self.terminal.clone();
        let key_controller = gtk4::EventControllerKey::new();
        key_controller.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                terminal_escape.match_remove_all();
                terminal_escape.search_set_regex(None, 0);
                window_escape.close();
                return gtk4::glib::Propagation::Stop;
            }
            gtk4::glib::Propagation::Proceed
        });
        self.window.add_controller(key_controller);
    }

    /// Performs a search in the terminal
    fn perform_search(
        terminal: &Terminal,
        text: &str,
        case_sensitive: bool,
        is_regex: bool,
        highlight_all: bool,
        match_label: &Label,
    ) {
        // Build the pattern
        let pattern = if is_regex {
            if case_sensitive {
                text.to_string()
            } else {
                format!("(?i){text}")
            }
        } else {
            let escaped = regex::escape(text);
            if case_sensitive {
                escaped
            } else {
                format!("(?i){escaped}")
            }
        };

        let regex_result = vte4::Regex::for_search(&pattern, 0);

        // Always clear previous match highlights first
        terminal.match_remove_all();

        if let Ok(regex) = regex_result {
            terminal.search_set_regex(Some(&regex), 0);
            terminal.search_set_wrap_around(true);

            // Add hover-highlight for all matches when enabled
            // VTE4 match_add_regex highlights text on mouse hover
            if highlight_all {
                if let Ok(hl_regex) = vte4::Regex::for_search(&pattern, 0) {
                    terminal.match_add_regex(&hl_regex, 0);
                }
            }

            if terminal.search_find_next() {
                match_label.set_text("Found matches");
            } else {
                match_label.set_text("No matches found");
            }
        } else if is_regex {
            match_label.set_text("Invalid regex pattern");
            terminal.search_set_regex(None, 0);
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
    pub const fn window(&self) -> &adw::Window {
        &self.window
    }
}
