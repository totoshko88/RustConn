//! Terminal search bar for finding text in VTE terminals
//!
//! Provides a non-modal search bar for VTE terminals with regex support,
//! highlight all matches, and navigation between matches.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, CheckButton, Label, Orientation, SearchEntry};
use rustconn_core::{CompiledHighlightRules, HighlightRule};
use uuid::Uuid;
use vte4::Terminal;
use vte4::prelude::*;

use crate::i18n::i18n;
use crate::terminal::highlight_overlay::{HighlightOverlay, host_overlay};

/// PCRE2 multiline compile flag. VTE's `search_set_regex()` rejects a regex
/// compiled without it, so a search built with flags 0 never ran.
const PCRE2_MULTILINE: u32 = 0x0000_0400;

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

/// Widget name of the search bar, used to find an open bar on an overlay.
const BAR_NAME: &str = "terminal-search-bar";

/// Non-modal search bar for a VTE terminal (issue #442).
///
/// The bar sits at the top of the `gtk4::Overlay` that hosts the terminal, the
/// same overlay the highlight layer uses, so one code path serves main-window
/// tabs, split panes and detached windows. The overlay's child is pushed down by
/// the bar's height, so the bar never covers a match, and the terminal stays
/// scrollable and selectable while the bar is open.
pub struct TerminalSearchBar;

impl TerminalSearchBar {
    /// Opens the bar on `terminal`, or focuses it when it is already open.
    pub fn show(terminal: &Terminal) {
        let Some(overlay) = host_overlay(terminal) else {
            return;
        };
        if let Some(open) = find_bar(&overlay) {
            if let Some(entry) = find_entry(&open) {
                entry.grab_focus();
            }
            return;
        }
        let Some(content) = overlay.child() else {
            return;
        };

        let search_entry = SearchEntry::builder()
            .placeholder_text(i18n("Search text…"))
            .hexpand(true)
            .build();
        let case_sensitive = CheckButton::builder().label(i18n("Case sensitive")).build();
        let regex_toggle = CheckButton::builder()
            .label(i18n("Regex"))
            .tooltip_text(i18n("Use regular expression pattern"))
            .build();
        let highlight_all = CheckButton::builder()
            .label(i18n("Highlight All"))
            .tooltip_text(i18n("Highlight all matches in terminal"))
            .active(true)
            .build();
        let prev_btn = Button::builder()
            .icon_name("go-up-symbolic")
            .tooltip_text(i18n("Previous match"))
            .build();
        prev_btn.update_property(&[gtk4::accessible::Property::Label(&i18n("Previous match"))]);
        let next_btn = Button::builder()
            .icon_name("go-down-symbolic")
            .tooltip_text(i18n("Next match"))
            .build();
        next_btn.update_property(&[gtk4::accessible::Property::Label(&i18n("Next match"))]);
        let close_btn = Button::builder()
            .icon_name("window-close-symbolic")
            .css_classes(["flat"])
            .tooltip_text(i18n("Close"))
            .build();
        close_btn.update_property(&[gtk4::accessible::Property::Label(&i18n("Close"))]);
        let match_label = Label::builder()
            .label(i18n("Enter text to search"))
            .hexpand(true)
            .halign(gtk4::Align::Start)
            .css_classes(["dim-label"])
            .build();

        let top_row = GtkBox::new(Orientation::Horizontal, 6);
        top_row.append(&search_entry);
        top_row.append(&prev_btn);
        top_row.append(&next_btn);
        top_row.append(&close_btn);
        let options_row = GtkBox::new(Orientation::Horizontal, 12);
        options_row.append(&case_sensitive);
        options_row.append(&regex_toggle);
        options_row.append(&highlight_all);
        options_row.append(&match_label);

        let bar = GtkBox::new(Orientation::Vertical, 6);
        bar.set_widget_name(BAR_NAME);
        bar.set_valign(gtk4::Align::Start);
        bar.set_halign(gtk4::Align::Fill);
        bar.add_css_class("background");
        for set in [
            GtkBox::set_margin_top,
            GtkBox::set_margin_bottom,
            GtkBox::set_margin_start,
            GtkBox::set_margin_end,
        ] {
            set(&bar, 6);
        }
        bar.append(&top_row);
        bar.append(&options_row);

        // The handlers on the entry and the toggles hold `controls` weakly: a
        // strong copy there is an entry → handler → controls → entry cycle that
        // keeps the entry, and the terminal in `controls`, alive for good. The
        // one strong copy lives in `close`, which only the bar's own children
        // and controllers hold, so it goes when the bar does.
        let controls = Rc::new(Controls {
            terminal: terminal.clone(),
            entry: search_entry.clone(),
            case_sensitive: case_sensitive.clone(),
            regex_toggle: regex_toggle.clone(),
            highlight_all: highlight_all.clone(),
            label: match_label,
            highlighter: Rc::new(SearchHighlight::new(terminal)),
        });

        // Close: takes the bar off, gives the rows back, returns focus.
        //
        // The closure lives in controllers attached to the bar, so it holds the
        // bar, the terminal and the content weakly: a strong bar here is a
        // bar → controller → closure → bar cycle that keeps the bar, and every
        // terminal its handlers hold, alive after the tab closes.
        let previous_margin_top = content.margin_top();
        let close = {
            let terminal = terminal.downgrade();
            let overlay = overlay.downgrade();
            let bar = bar.downgrade();
            let content = content.downgrade();
            let controls = Rc::clone(&controls);
            Rc::new(move || {
                controls.highlighter.remove();
                if let Some(content) = content.upgrade() {
                    content.set_margin_top(previous_margin_top);
                }
                if let (Some(overlay), Some(bar)) = (overlay.upgrade(), bar.upgrade()) {
                    overlay.remove_overlay(&bar);
                }
                if let Some(terminal) = terminal.upgrade() {
                    terminal.search_set_regex(None, 0);
                    terminal.grab_focus();
                }
            })
        };
        close_btn.connect_clicked({
            let close = Rc::clone(&close);
            move |_| close()
        });

        search_entry.connect_search_changed({
            let c = Rc::downgrade(&controls);
            move |_| {
                if let Some(c) = c.upgrade() {
                    c.run();
                }
            }
        });
        for toggle in [&case_sensitive, &regex_toggle] {
            toggle.connect_toggled({
                let c = Rc::downgrade(&controls);
                move |_| {
                    if let Some(c) = c.upgrade() {
                        c.run();
                    }
                }
            });
        }
        highlight_all.connect_toggled({
            let c = Rc::downgrade(&controls);
            move |btn| {
                let Some(c) = c.upgrade() else {
                    return;
                };
                let text = c.entry.text();
                if !text.is_empty() {
                    c.highlighter.set(btn.is_active().then(|| {
                        search_pattern(
                            &text,
                            c.case_sensitive.is_active(),
                            c.regex_toggle.is_active(),
                        )
                    }));
                }
            }
        });
        prev_btn.connect_clicked({
            let t = terminal.clone();
            move |_| {
                t.search_find_previous();
            }
        });
        next_btn.connect_clicked({
            let t = terminal.clone();
            move |_| {
                t.search_find_next();
            }
        });
        search_entry.connect_activate({
            let t = terminal.clone();
            move |_| {
                t.search_find_next();
            }
        });
        // Shift+Enter goes to the previous match; Escape closes the bar. The
        // capture phase runs before the entry handles the key itself.
        let keys = gtk4::EventControllerKey::new();
        keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
        keys.connect_key_pressed({
            let t = terminal.clone();
            let close = Rc::clone(&close);
            move |_, key, _, state| {
                use gtk4::gdk::Key;
                if key == Key::Escape {
                    close();
                    return gtk4::glib::Propagation::Stop;
                }
                if matches!(key, Key::Return | Key::KP_Enter | Key::ISO_Enter)
                    && state.contains(gtk4::gdk::ModifierType::SHIFT_MASK)
                {
                    t.search_find_previous();
                    return gtk4::glib::Propagation::Stop;
                }
                gtk4::glib::Propagation::Proceed
            }
        });
        bar.add_controller(keys);

        overlay.add_overlay(&bar);
        // Reserve the bar's height above the terminal so it never hides a row.
        let (_, bar_height, _, _) = bar.measure(Orientation::Vertical, overlay.width().max(-1));
        content.set_margin_top(bar_height);
        search_entry.grab_focus();
    }
}

/// Everything a search needs, shared by the bar's signal handlers.
struct Controls {
    terminal: Terminal,
    entry: SearchEntry,
    case_sensitive: CheckButton,
    regex_toggle: CheckButton,
    highlight_all: CheckButton,
    label: Label,
    highlighter: Rc<SearchHighlight>,
}

impl Controls {
    /// Runs the search for the entry's current text and options.
    fn run(&self) {
        let text = self.entry.text();
        if text.is_empty() {
            self.label.set_text(&i18n("Enter text to search"));
            self.highlighter.set(None);
            self.terminal.search_set_regex(None, 0);
            return;
        }
        perform_search(
            &self.terminal,
            &self.highlighter,
            &text,
            self.case_sensitive.is_active(),
            self.regex_toggle.is_active(),
            self.highlight_all.is_active(),
            &self.label,
        );
    }
}

/// Finds an open search bar on `overlay`.
fn find_bar(overlay: &gtk4::Overlay) -> Option<gtk4::Widget> {
    let mut child = overlay.first_child();
    while let Some(w) = child {
        if w.widget_name() == BAR_NAME {
            return Some(w);
        }
        child = w.next_sibling();
    }
    None
}

/// Finds the search entry inside `bar`.
fn find_entry(bar: &gtk4::Widget) -> Option<SearchEntry> {
    let row = bar.first_child()?;
    row.first_child()?.downcast().ok()
}

/// Performs a search in the terminal.
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
    if let Ok(regex) = vte4::Regex::for_search(&pattern, PCRE2_MULTILINE) {
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

    /// Opening the bar reserves its height above the terminal, a search selects
    /// a match, Shift+Enter is wired, closing gives the room back and returns
    /// focus (issues #441, #442).
    #[test]
    #[ignore = "initialises GTK: needs a display and its own process"]
    fn bar_reserves_room_and_search_selects_match() {
        if gtk4::init().is_err() {
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
        let window = gtk4::Window::builder()
            .default_width(700)
            .default_height(500)
            .child(&overlay)
            .build();
        window.present();
        terminal.feed(b"first foo\r\nsecond\r\nthird foo FOO\r\n");
        // A margin the host had before the bar opened must survive closing it.
        host.set_margin_top(3);
        // `feed()` only queues the bytes: VTE parses them on a later main-loop
        // iteration. A search that runs before that sees an empty buffer and
        // selects nothing, and nothing repeats it. A fixed pause made that a
        // race on a slow runner, so wait until all three lines are parsed (the
        // cursor sits on row 3).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while terminal.cursor_position().1 < 3 {
            assert!(
                std::time::Instant::now() < deadline,
                "VTE did not parse the fed text"
            );
            pump(10);
        }

        TerminalSearchBar::show(&terminal);
        let bar = find_bar(&overlay).expect("bar is on the overlay");
        assert!(host.margin_top() > 3, "bar height must be reserved");
        // A second call reuses the open bar.
        TerminalSearchBar::show(&terminal);
        assert_eq!(
            std::iter::successors(overlay.first_child(), |w| w.next_sibling()).count(),
            3
        );
        let entry = find_entry(&bar).expect("entry");
        entry.set_text("foo");
        // The entry debounces `search-changed`, and on a loaded runner the
        // timer can fire well after a fixed pause, so wait for the selection.
        // On timeout, the status label tells which step did not happen: still
        // "Enter text…" means the search never ran, "No matches found" means
        // VTE ran it and found nothing.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !terminal.has_selection() && std::time::Instant::now() < deadline {
            pump(20);
        }
        let mut labels = Vec::new();
        let mut stack = vec![bar.clone()];
        while let Some(w) = stack.pop() {
            if let Some(l) = w.downcast_ref::<Label>() {
                labels.push(l.text().to_string());
            }
            stack.extend(std::iter::successors(w.first_child(), |c| c.next_sibling()));
        }
        assert!(
            terminal.has_selection(),
            "the search must select a match (bar labels: {labels:?}, text: {:?})",
            terminal
                .text_range_format(vte4::Format::Text, 0, 0, 3, 80)
                .0
                .map(|t| t.to_string())
        );
        assert_eq!(
            terminal.text_selected(vte4::Format::Text).as_deref(),
            Some("foo")
        );
        if let Ok(ms) = std::env::var("RC_SHOT_PAUSE_MS") {
            pump(ms.parse().unwrap_or(0));
        }

        // The close button removes the bar and gives the rows back.
        let close_btn = bar
            .first_child()
            .and_then(|row| row.last_child())
            .and_then(|w| w.downcast::<Button>().ok())
            .expect("close button");
        close_btn.emit_clicked();
        pump(100);
        assert!(find_bar(&overlay).is_none());
        assert_eq!(host.margin_top(), 3, "closing must restore the old margin");
        // The close closure must not keep the bar alive through its own
        // controllers once the overlay lets go of it.
        let weak_bar = bar.downgrade();
        let weak_entry = entry.downgrade();
        drop(close_btn);
        drop(entry);
        drop(bar);
        pump(100);
        assert!(weak_bar.upgrade().is_none(), "closed bar must be freed");
        // Nor may the entry's own handlers keep it, and the terminal, alive.
        assert!(weak_entry.upgrade().is_none(), "closed entry must be freed");
    }
}
