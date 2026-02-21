//! Search logic for the sidebar
use gtk4::prelude::*;
use gtk4::{glib, Button, EventControllerKey, Label, Orientation, Popover, SearchEntry};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use crate::sidebar_types::MAX_SEARCH_HISTORY;

/// Creates the search help popover
pub fn create_search_help_popover() -> Popover {
    let popover = Popover::new();
    let box_container = gtk4::Box::new(Orientation::Vertical, 6);
    box_container.set_margin_start(12);
    box_container.set_margin_end(12);
    box_container.set_margin_top(12);
    box_container.set_margin_bottom(12);

    let title = Label::builder()
        .label("<b>Search Syntax</b>")
        .use_markup(true)
        .halign(gtk4::Align::Start)
        .build();
    title.add_css_class("heading");
    box_container.append(&title);

    let help_text = "\
• name: Search by name
• @username: Search by username
• #tag: Search by tag
• 1.2.3.4: Search by IP
• protocol:ssh: Filter by protocol
• group:name: Search in group";

    let label = Label::new(Some(help_text));
    label.set_halign(gtk4::Align::Start);
    box_container.append(&label);

    popover.set_child(Some(&box_container));
    popover
}

/// Sets up search entry hints and history navigation
///
/// Handles Up/Down arrow keys for cycling through search history:
/// - Down when empty: opens history popover
/// - Up when empty: opens history popover
/// - Up/Down when popover is open: cycles through history items inline
pub fn setup_search_entry_hints(
    search_entry: &SearchEntry,
    entry_clone: &SearchEntry,
    history_popover: &Popover,
    search_history: &Rc<RefCell<Vec<String>>>,
) {
    let controller = EventControllerKey::new();
    let history_clone = search_history.clone();
    let entry_weak = entry_clone.downgrade();
    let popover_weak = history_popover.downgrade();
    // Track current position in history for Up/Down cycling (-1 = not navigating)
    let history_index: Rc<RefCell<i32>> = Rc::new(RefCell::new(-1));
    let original_text: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));

    controller.connect_key_pressed(move |_controller, key, _code, _state| {
        let Some(entry) = entry_weak.upgrade() else {
            return glib::Propagation::Proceed;
        };

        match key {
            gtk4::gdk::Key::Down => {
                let history = history_clone.borrow();
                if history.is_empty() {
                    return glib::Propagation::Proceed;
                }

                if entry.text().is_empty() && *history_index.borrow() < 0 {
                    // First press on empty entry — open popover
                    if let Some(popover) = popover_weak.upgrade() {
                        popover.popup();
                    }
                    return glib::Propagation::Stop;
                }

                // Navigate forward (toward newer / empty)
                let mut idx = history_index.borrow_mut();
                if *idx > 0 {
                    *idx -= 1;
                    let text = &history[*idx as usize];
                    entry.set_text(text);
                    entry.set_position(-1);
                } else if *idx == 0 {
                    // Back to original text
                    *idx = -1;
                    entry.set_text(&original_text.borrow());
                    entry.set_position(-1);
                }
                glib::Propagation::Stop
            }
            gtk4::gdk::Key::Up => {
                let history = history_clone.borrow();
                if history.is_empty() {
                    return glib::Propagation::Proceed;
                }

                let mut idx = history_index.borrow_mut();
                if *idx < 0 {
                    // Start navigating — save current text
                    *original_text.borrow_mut() = entry.text().to_string();
                    *idx = 0;
                } else if (*idx as usize) < history.len() - 1 {
                    *idx += 1;
                } else {
                    return glib::Propagation::Stop;
                }

                let text = &history[*idx as usize];
                entry.set_text(text);
                entry.set_position(-1);

                // Show popover as visual hint
                if let Some(popover) = popover_weak.upgrade() {
                    if !popover.is_visible() {
                        popover.popup();
                    }
                }
                glib::Propagation::Stop
            }
            gtk4::gdk::Key::Escape => {
                // Reset history navigation on Escape
                if *history_index.borrow() >= 0 {
                    *history_index.borrow_mut() = -1;
                    entry.set_text(&original_text.borrow());
                    entry.set_position(-1);
                    if let Some(popover) = popover_weak.upgrade() {
                        popover.popdown();
                    }
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            }
            _ => {
                // Any other key resets history navigation
                if *history_index.borrow() >= 0 {
                    *history_index.borrow_mut() = -1;
                }
                glib::Propagation::Proceed
            }
        }
    });

    search_entry.add_controller(controller);
}

/// Creates the search history popover
pub fn create_history_popover(
    _parent: &SearchEntry,
    search_history: Rc<RefCell<Vec<String>>>,
) -> Popover {
    let popover = Popover::new();

    let list_box = gtk4::ListBox::new();
    list_box.set_selection_mode(gtk4::SelectionMode::None);
    list_box.add_css_class("boxed-list");

    let scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .max_content_height(200)
        .propagate_natural_height(true)
        .child(&list_box)
        .build();

    popover.set_child(Some(&scroll));

    let history_clone = search_history;
    let list_box_clone = list_box.clone();
    let popover_weak = popover.downgrade();

    // Populate rows each time the popover becomes visible
    popover.connect_visible_notify(move |popover| {
        if !popover.is_visible() {
            return;
        }

        // Clear previous rows
        while let Some(child) = list_box_clone.first_child() {
            list_box_clone.remove(&child);
        }

        let history = history_clone.borrow();
        for query in history.iter().take(MAX_SEARCH_HISTORY) {
            let row = gtk4::ListBoxRow::new();
            let label = Label::new(Some(query));
            label.set_halign(gtk4::Align::Start);
            label.set_margin_start(6);
            label.set_margin_end(6);
            label.set_margin_top(4);
            label.set_margin_bottom(4);
            row.set_child(Some(&label));
            list_box_clone.append(&row);
        }
    });

    // Connect row activation to fill search entry and close popover
    list_box.connect_row_activated(move |_, row| {
        if let Some(label) = row.child().and_then(|c| c.downcast::<Label>().ok()) {
            let text = label.text();
            // Walk up to find the SearchEntry sibling via the popover parent
            if let Some(pop) = popover_weak.upgrade() {
                if let Some(parent) = pop.parent() {
                    if let Ok(entry) = parent.downcast::<SearchEntry>() {
                        entry.set_text(&text);
                        entry.set_position(-1);
                    }
                }
                pop.popdown();
            }
        }
    });

    popover
}

/// Updates search entry with current protocol filters
pub fn update_search_with_filters(
    filters: &HashSet<String>,
    search_entry: &SearchEntry,
    programmatic_flag: &Rc<RefCell<bool>>,
) {
    // Set flag to prevent recursive clearing
    *programmatic_flag.borrow_mut() = true;

    if filters.is_empty() {
        // Clear search if no filters
        search_entry.set_text("");
    } else if filters.len() == 1 {
        // Single protocol filter - use standard search syntax
        // Safe: we just checked filters.len() == 1, so next() will succeed
        if let Some(protocol) = filters.iter().next() {
            let query = format!("protocol:{}", protocol.to_lowercase());
            search_entry.set_text(&query);
        }
    } else {
        // Multiple protocol filters - use special syntax that filter_connections can recognize
        let mut protocols: Vec<String> = filters.iter().cloned().collect();
        protocols.sort();
        let query = format!("protocols:{}", protocols.join(","));
        search_entry.set_text(&query);
    }

    // Reset flag after a short delay or immediately?
    // The original code likely resets it in the changed handler or assumes
    // the text change triggers the handler which checks the flag.
    // Yes, the handler checks the flag and returns.
    // The flag needs to be unset somewhere?
    // Ah, `search_entry.set_text` is synchronous, so the handler runs immediately.
    // So we can unset it after.
    *programmatic_flag.borrow_mut() = false;
}

/// Adds a search query to the history
pub fn add_to_history(search_history: &Rc<RefCell<Vec<String>>>, query: &str) {
    if query.trim().is_empty() {
        return;
    }

    let mut history = search_history.borrow_mut();

    // Remove if already exists (to move to front)
    history.retain(|q| q != query);

    // Add to front
    history.insert(0, query.to_string());

    // Trim to max size
    history.truncate(MAX_SEARCH_HISTORY);
}

/// Toggles a protocol filter and updates the search
pub fn toggle_protocol_filter(
    protocol: &str,
    button: &Button,
    active_filters: &Rc<RefCell<HashSet<String>>>,
    buttons: &Rc<RefCell<std::collections::HashMap<String, Button>>>,
    search_entry: &SearchEntry,
    programmatic_flag: &Rc<RefCell<bool>>,
) {
    let mut filters = active_filters.borrow_mut();

    if filters.contains(protocol) {
        // Remove filter
        filters.remove(protocol);
        button.remove_css_class("suggested-action");
    } else {
        // Add filter
        filters.insert(protocol.to_string());
        button.add_css_class("suggested-action");
    }

    // Update visual feedback for all buttons when multiple filters are active
    let filter_count = filters.len();
    if filter_count > 1 {
        // Multiple filters active - add special styling to show AND relationship
        for (filter_name, filter_button) in buttons.borrow().iter() {
            if filters.contains(filter_name) {
                filter_button.add_css_class("filter-active-multiple");
            } else {
                filter_button.remove_css_class("filter-active-multiple");
            }
        }
    } else {
        // Single or no filters - remove multiple filter styling
        for filter_button in buttons.borrow().values() {
            filter_button.remove_css_class("filter-active-multiple");
        }
    }

    // Update search with protocol filters
    update_search_with_filters(&filters, search_entry, programmatic_flag);
}

/// Highlights matching text with Pango markup
pub fn highlight_match(text: &str, query: &str) -> String {
    if query.trim().is_empty() {
        return glib::markup_escape_text(text).to_string();
    }

    // Escape the query for regex usage
    let escaped_query = regex::escape(query);
    let regex = match regex::RegexBuilder::new(&format!("(?i){}", escaped_query)).build() {
        Ok(r) => r,
        Err(_) => return glib::markup_escape_text(text).to_string(),
    };

    let mut last_end = 0;
    let mut result = String::new();

    for mat in regex.find_iter(text) {
        let start = mat.start();
        let end = mat.end();

        let before = &text[last_end..start];
        let matched = &text[start..end];

        result.push_str(&glib::markup_escape_text(before));
        result.push_str("<b>");
        result.push_str(&glib::markup_escape_text(matched));
        result.push_str("</b>");

        last_end = end;
    }

    result.push_str(&glib::markup_escape_text(&text[last_end..]));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_highlight_match() {
        // Simple match
        assert_eq!(highlight_match("Hello World", "ell"), "H<b>ell</b>o World");

        // Case insensitive
        assert_eq!(
            highlight_match("Hello World", "world"),
            "Hello <b>World</b>"
        );

        // No match
        assert_eq!(highlight_match("No match", "foo"), "No match");

        // Match at start
        assert_eq!(
            highlight_match("Start match", "start"),
            "<b>Start</b> match"
        );

        // Match at end
        assert_eq!(highlight_match("End match", "match"), "End <b>match</b>");

        // Multiple matches
        assert_eq!(
            highlight_match("foo bar foo", "foo"),
            "<b>foo</b> bar <b>foo</b>"
        );

        // HTML escaping
        assert_eq!(
            highlight_match("<b>Bold</b>", "old"),
            "&lt;b&gt;B<b>old</b>&lt;/b&gt;"
        );
    }
}
