//! Terminal search dialog for finding text in VTE terminals
//!
//! Provides a search interface for VTE terminals with regex support,
//! highlight all matches, and navigation between matches.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, CheckButton, Label, Orientation, SearchEntry};
use libadwaita as adw;
use rustconn_core::{CompiledHighlightRules, HighlightRule};
use uuid::Uuid;
use vte4::Terminal;
use vte4::prelude::*;

use crate::i18n::i18n;
use crate::terminal::highlight_overlay::HighlightOverlay;

/// PCRE2 multiline compile flag. VTE's `search_set_regex()` rejects a regex
/// compiled without it, so a search built with flags 0 never ran.
const PCRE2_MULTILINE: u32 = 0x0000_0400;

/// Extra pixels kept free below the terminal for the sheet's edge and shadow.
const SHEET_SLACK: i32 = 8;

/// Builds the regex text for a search: the user's text as a regex, or escaped
/// for a literal search, with `(?i)` in front unless the search is case sensitive.
fn search_pattern(text: &str, case_sensitive: bool, is_regex: bool) -> String {
    let body = if is_regex {
        text.to_string()
    } else {
        regex::escape(text)
    };
    if case_sensitive {
        body
    } else {
        format!("(?i){body}")
    }
}

/// Paints every visible match of the current search on the terminal.
///
/// VTE's own `match_add_regex()` only underlines the text under the mouse
/// pointer, so "Highlight All" draws its matches on a layer above the terminal,
/// the same one the highlight rules use (issue #441).
struct SearchHighlight {
    rules: Rc<RefCell<HashMap<Uuid, CompiledHighlightRules>>>,
    id: Uuid,
    overlay: RefCell<Option<HighlightOverlay>>,
}

impl SearchHighlight {
    fn new(terminal: &Terminal) -> Self {
        let rules = Rc::new(RefCell::new(HashMap::new()));
        let id = Uuid::new_v4();
        let overlay = HighlightOverlay::attach(terminal, Rc::clone(&rules), id, Vec::new());
        Self {
            rules,
            id,
            overlay: RefCell::new(Some(overlay)),
        }
    }

    /// Shows the matches of `pattern`, or none when it is `None`.
    // shortcut: the layer uses the Rust regex engine, VTE the PCRE2 one, so a
    // regex search that only PCRE2 accepts is found but not highlighted.
    fn set(&self, pattern: Option<String>) {
        match pattern {
            Some(p) => {
                self.rules.borrow_mut().insert(self.id, match_rules(p));
            }
            None => {
                self.rules.borrow_mut().remove(&self.id);
            }
        }
        if let Some(overlay) = self.overlay.borrow().as_ref() {
            overlay.queue_draw();
        }
    }

    /// Takes the layer off the terminal.
    fn remove(&self) {
        self.rules.borrow_mut().clear();
        let overlay = self.overlay.borrow_mut().take();
        drop(overlay);
    }
}

/// One rule that tints every match yellow.
fn match_rules(pattern: String) -> CompiledHighlightRules {
    let mut rule = HighlightRule::new("search".to_string(), pattern);
    rule.background_color = Some("#f6d32d".to_string());
    CompiledHighlightRules::compile_with_options(&[rule], &[], false)
}

/// Terminal search dialog for VTE terminals
pub struct TerminalSearchDialog {
    dialog: adw::Dialog,
    search_entry: SearchEntry,
    case_sensitive: CheckButton,
    regex_toggle: CheckButton,
    highlight_all: CheckButton,
    match_label: Label,
    terminal: Terminal,
    current_search: Rc<RefCell<String>>,
    prev_btn: Button,
    next_btn: Button,
    highlighter: Rc<SearchHighlight>,
    toolbar_view: adw::ToolbarView,
    /// Parent widget for presenting the dialog.
    parent: Option<gtk4::Widget>,
}

impl TerminalSearchDialog {
    /// Creates a new terminal search dialog
    #[must_use]
    pub fn new(parent: Option<&gtk4::Window>, terminal: Terminal) -> Self {
        let dialog = adw::Dialog::builder()
            .title(i18n("Search in Terminal"))
            .presentation_mode(adw::DialogPresentationMode::BottomSheet)
            .follows_content_size(true)
            .build();

        let parent_widget: Option<gtk4::Widget> =
            parent.map(|p| p.clone().upcast::<gtk4::Widget>());

        // Header bar with standard window close button (×) per GNOME HIG
        let header = adw::HeaderBar::new();

        // Create main content
        let content = GtkBox::new(Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // Use ToolbarView for adw::Dialog
        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&content));
        dialog.set_child(Some(&toolbar_view));

        // Search entry
        let search_entry = SearchEntry::builder()
            .placeholder_text(i18n("Search text…"))
            .hexpand(true)
            .build();
        content.append(&search_entry);

        // Options row
        let options_box = GtkBox::new(Orientation::Horizontal, 12);

        let case_sensitive = CheckButton::builder().label(i18n("Case sensitive")).build();
        options_box.append(&case_sensitive);

        let regex_toggle = CheckButton::builder()
            .label(i18n("Regex"))
            .tooltip_text(i18n("Use regular expression pattern"))
            .build();
        options_box.append(&regex_toggle);

        let highlight_all = CheckButton::builder()
            .label(i18n("Highlight All"))
            .tooltip_text(i18n("Highlight all matches in terminal"))
            .active(true)
            .build();
        options_box.append(&highlight_all);

        content.append(&options_box);

        // Navigation row
        let nav_box = GtkBox::new(Orientation::Horizontal, 6);

        let prev_btn = Button::builder()
            .icon_name("go-up-symbolic")
            .tooltip_text(i18n("Previous match"))
            .build();
        prev_btn.update_property(&[gtk4::accessible::Property::Label(&i18n("Previous match"))]);
        nav_box.append(&prev_btn);

        let next_btn = Button::builder()
            .icon_name("go-down-symbolic")
            .tooltip_text(i18n("Next match"))
            .build();
        next_btn.update_property(&[gtk4::accessible::Property::Label(&i18n("Next match"))]);
        nav_box.append(&next_btn);

        let match_label = Label::builder()
            .label(i18n("Enter text to search"))
            .hexpand(true)
            .halign(gtk4::Align::Start)
            .build();
        nav_box.append(&match_label);

        content.append(&nav_box);

        let current_search = Rc::new(RefCell::new(String::new()));

        let highlighter = Rc::new(SearchHighlight::new(&terminal));

        let dialog = Self {
            dialog,
            search_entry,
            case_sensitive,
            regex_toggle,
            highlight_all,
            match_label,
            terminal,
            current_search,
            prev_btn,
            next_btn,
            highlighter,
            toolbar_view,
            parent: parent_widget,
        };

        dialog.setup_signals();
        dialog
    }

    /// Sets up signal handlers for the dialog
    fn setup_signals(&self) {
        // Clear terminal search highlights whenever the dialog closes, by any
        // means — the header-bar × button, Escape, or a programmatic close.
        // The terminal gets its rows back from under the sheet (issue #442).
        let terminal_close = self.terminal.clone();
        let highlighter_close = Rc::clone(&self.highlighter);
        self.dialog.connect_closed(move |_| {
            highlighter_close.remove();
            terminal_close.set_margin_bottom(0);
            terminal_close.search_set_regex(None, 0);
        });

        // Search on text change
        let terminal = self.terminal.clone();
        let case_sensitive = self.case_sensitive.clone();
        let regex_toggle = self.regex_toggle.clone();
        let highlight_all = self.highlight_all.clone();
        let match_label = self.match_label.clone();
        let current_search = self.current_search.clone();
        let highlighter = Rc::clone(&self.highlighter);

        self.search_entry.connect_search_changed(move |entry| {
            let text = entry.text();
            if text.is_empty() {
                match_label.set_text(&i18n("Enter text to search"));
                *current_search.borrow_mut() = String::new();
                highlighter.set(None);
                terminal.search_set_regex(None, 0);
                return;
            }

            *current_search.borrow_mut() = text.to_string();
            Self::perform_search(
                &terminal,
                &highlighter,
                &text,
                case_sensitive.is_active(),
                regex_toggle.is_active(),
                highlight_all.is_active(),
                &match_label,
            );
        });

        // Re-search when any toggle changes
        let make_toggle_handler = |term: Terminal,
                                   hl_layer: Rc<SearchHighlight>,
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
                        &hl_layer,
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
            Rc::clone(&self.highlighter),
            self.search_entry.clone(),
            self.case_sensitive.clone(),
            self.regex_toggle.clone(),
            self.highlight_all.clone(),
            self.match_label.clone(),
        ));

        self.regex_toggle.connect_toggled(make_toggle_handler(
            self.terminal.clone(),
            Rc::clone(&self.highlighter),
            self.search_entry.clone(),
            self.case_sensitive.clone(),
            self.regex_toggle.clone(),
            self.highlight_all.clone(),
            self.match_label.clone(),
        ));

        self.highlight_all.connect_toggled({
            let highlighter = Rc::clone(&self.highlighter);
            let search_entry = self.search_entry.clone();
            let case_sensitive = self.case_sensitive.clone();
            let regex_toggle = self.regex_toggle.clone();
            move |btn| {
                let text = search_entry.text();
                if text.is_empty() {
                    return;
                }

                // Only toggle the highlight layer without navigating
                highlighter.set(btn.is_active().then(|| {
                    search_pattern(&text, case_sensitive.is_active(), regex_toggle.is_active())
                }));
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
        let dialog_escape = self.dialog.clone();
        let key_controller = gtk4::EventControllerKey::new();
        key_controller.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                // Highlights are cleared by the `connect_closed` handler above.
                dialog_escape.close();
                return gtk4::glib::Propagation::Stop;
            }
            gtk4::glib::Propagation::Proceed
        });
        self.dialog.add_controller(key_controller);
    }

    /// Performs a search in the terminal
    fn perform_search(
        terminal: &Terminal,
        highlighter: &SearchHighlight,
        text: &str,
        case_sensitive: bool,
        is_regex: bool,
        highlight_all: bool,
        match_label: &Label,
    ) {
        let pattern = search_pattern(text, case_sensitive, is_regex);

        let regex_result = vte4::Regex::for_search(&pattern, PCRE2_MULTILINE);

        if let Ok(regex) = regex_result {
            terminal.search_set_regex(Some(&regex), 0);
            terminal.search_set_wrap_around(true);

            highlighter.set(highlight_all.then_some(pattern));

            if terminal.search_find_next() {
                match_label.set_text(&i18n("Found matches"));
            } else {
                match_label.set_text(&i18n("No matches found"));
            }
        } else {
            highlighter.set(None);
            if is_regex {
                match_label.set_text(&i18n("Invalid regex pattern"));
            } else {
                match_label.set_text(&i18n("Search error"));
            }
            terminal.search_set_regex(None, 0);
        }
    }

    /// Shows the dialog
    pub fn show(&self) {
        self.dialog.present(self.parent.as_ref());
        self.search_entry.grab_focus();

        // The sheet covers the bottom of the window. Shrink the terminal by the
        // sheet's height so it never hides a match, even on the last row
        // (issue #442).
        let (_, sheet_height, _, _) = self.toolbar_view.measure(
            Orientation::Vertical,
            self.parent.as_ref().map_or(-1, |p| p.width()),
        );
        self.terminal.set_margin_bottom(sheet_height + SHEET_SLACK);
    }

    /// Returns the underlying dialog
    #[must_use]
    pub const fn dialog(&self) -> &adw::Dialog {
        &self.dialog
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_search_is_escaped_and_case_insensitive_by_default() {
        assert_eq!(search_pattern("a.b", false, false), r"(?i)a\.b");
        assert_eq!(search_pattern("a.b", true, false), r"a\.b");
        assert_eq!(search_pattern("a.b", true, true), "a.b");
    }

    #[test]
    fn highlight_rule_finds_every_match_on_a_line() {
        let rules = match_rules(search_pattern("err", false, false));
        let line = "Err x ERR y err";
        let starts: Vec<usize> = rules.find_matches(line).iter().map(|m| m.start).collect();
        assert_eq!(starts, [0, 6, 12]);
        assert!(
            rules
                .find_matches(line)
                .iter()
                .all(|m| m.background_rgb.is_some())
        );
    }

    /// Opening the sheet reserves its height below the terminal, closing gives
    /// it back, and a search selects the match (issues #441, #442).
    #[test]
    #[ignore = "initialises GTK: needs a display and its own process"]
    fn sheet_reserves_room_and_search_selects_match() {
        if gtk4::init().is_err() || adw::init().is_err() {
            return;
        }
        let pump = |ms: u64| {
            let ctx = gtk4::glib::MainContext::default();
            let end = std::time::Instant::now() + std::time::Duration::from_millis(ms);
            while std::time::Instant::now() < end {
                ctx.iteration(false);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        };
        let terminal = Terminal::new();
        terminal.set_vexpand(true);
        let overlay = gtk4::Overlay::new();
        let host = GtkBox::new(Orientation::Vertical, 0);
        host.append(&terminal);
        overlay.set_child(Some(&host));
        let window = adw::ApplicationWindow::builder()
            .default_width(700)
            .default_height(500)
            .content(&overlay)
            .build();
        window.present();
        terminal.feed(b"first foo\r\nsecond\r\nthird foo FOO\r\n");
        pump(300);

        let dialog = TerminalSearchDialog::new(Some(window.upcast_ref()), terminal.clone());
        dialog.show();
        dialog.search_entry.set_text("foo");
        pump(300);
        assert!(
            terminal.margin_bottom() > 0,
            "sheet height must be reserved"
        );
        assert!(terminal.has_selection(), "the search must select a match");
        assert_eq!(
            terminal.text_selected(vte4::Format::Text).as_deref(),
            Some("foo")
        );
        if let Ok(ms) = std::env::var("RC_SHOT_PAUSE_MS") {
            pump(ms.parse().unwrap_or(0));
        }

        dialog.dialog.force_close();
        pump(200);
        assert_eq!(terminal.margin_bottom(), 0, "closing must free the room");
    }
}
