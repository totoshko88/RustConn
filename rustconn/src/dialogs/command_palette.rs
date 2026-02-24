//! Command Palette dialog — VS Code-style quick launcher.
//!
//! - Empty query → recent connections (sorted by `last_connected`)
//! - `>` prefix → application commands
//! - `@` prefix → filter by tag
//! - `#` prefix → filter by group
//! - Plain text → fuzzy search connections

use crate::i18n::i18n;
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, EventControllerKey, Image, Label, ListBox, ListBoxRow, Orientation,
    ScrolledWindow,
};
use libadwaita as adw;
use rustconn_core::models::{Connection, ConnectionGroup};
use rustconn_core::search::command_palette::{
    CommandPaletteAction, PaletteItem, PaletteMode, builtin_commands, parse_palette_input,
};
use rustconn_core::{SearchEngine, SearchQuery, get_protocol_icon_by_name};
use std::cell::RefCell;
use std::rc::Rc;

/// Callback type for when a palette action is selected
pub type PaletteCallback = Rc<RefCell<Option<Box<dyn Fn(CommandPaletteAction)>>>>;

/// Command Palette dialog
pub struct CommandPaletteDialog {
    dialog: adw::Dialog,
    search_entry: gtk4::SearchEntry,
    list_box: ListBox,
    items: Rc<RefCell<Vec<PaletteItem>>>,
    connections: Rc<RefCell<Vec<Connection>>>,
    groups: Rc<RefCell<Vec<ConnectionGroup>>>,
    on_action: PaletteCallback,
    search_engine: Rc<SearchEngine>,
    parent: Option<gtk4::Widget>,
}

impl CommandPaletteDialog {
    /// Creates a new Command Palette dialog
    #[must_use]
    pub fn new(parent: Option<&impl IsA<gtk4::Window>>) -> Self {
        let dialog = adw::Dialog::builder()
            .title("")
            .content_width(500)
            .content_height(400)
            .build();

        // Main vertical layout
        let vbox = GtkBox::new(Orientation::Vertical, 0);

        // Search entry at top
        let search_entry = gtk4::SearchEntry::builder()
            .placeholder_text(i18n("Search connections, > commands, @ tags, # groups"))
            .hexpand(true)
            .margin_top(12)
            .margin_start(12)
            .margin_end(12)
            .margin_bottom(6)
            .build();
        search_entry.set_tooltip_text(Some(&i18n("Command Palette")));
        vbox.append(&search_entry);

        // Scrolled list
        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .build();

        let clamp = adw::Clamp::builder()
            .maximum_size(500)
            .tightening_threshold(400)
            .build();

        let list_box = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .css_classes(["boxed-list"])
            .margin_start(12)
            .margin_end(12)
            .margin_bottom(12)
            .build();

        list_box.set_placeholder(Some(
            &Label::builder()
                .label(i18n("No results"))
                .css_classes(["dim-label"])
                .margin_top(24)
                .margin_bottom(24)
                .build(),
        ));

        clamp.set_child(Some(&list_box));
        scrolled.set_child(Some(&clamp));
        vbox.append(&scrolled);

        dialog.set_child(Some(&vbox));

        let stored_parent: Option<gtk4::Widget> =
            parent.map(|p| p.clone().upcast::<gtk4::Window>().upcast::<gtk4::Widget>());

        let palette = Self {
            dialog: dialog.clone(),
            search_entry: search_entry.clone(),
            list_box: list_box.clone(),
            items: Rc::new(RefCell::new(Vec::new())),
            connections: Rc::new(RefCell::new(Vec::new())),
            groups: Rc::new(RefCell::new(Vec::new())),
            on_action: Rc::new(RefCell::new(None)),
            search_engine: Rc::new(SearchEngine::new()),
            parent: stored_parent,
        };

        // Wire up search-changed → rebuild list
        {
            let items = palette.items.clone();
            let connections = palette.connections.clone();
            let groups = palette.groups.clone();
            let list_box_clone = list_box.clone();
            let engine = palette.search_engine.clone();
            search_entry.connect_search_changed(move |entry| {
                let text = entry.text().to_string();
                let new_items =
                    Self::compute_items(&text, &connections.borrow(), &groups.borrow(), &engine);
                Self::populate_list(&list_box_clone, &new_items);
                *items.borrow_mut() = new_items;
                // Auto-select first row
                if let Some(first) = list_box_clone.row_at_index(0) {
                    list_box_clone.select_row(Some(&first));
                }
            });
        }

        // Enter on search entry → activate selected row
        {
            let list_box_clone = list_box.clone();
            let items_clone = palette.items.clone();
            let on_action = palette.on_action.clone();
            let dialog_clone = dialog.clone();
            let key_controller = EventControllerKey::new();
            key_controller.connect_key_pressed(move |_, keyval, _, _| {
                match keyval {
                    gtk4::gdk::Key::Escape => {
                        dialog_clone.close();
                        gtk4::glib::Propagation::Stop
                    }
                    gtk4::gdk::Key::Return | gtk4::gdk::Key::KP_Enter => {
                        Self::activate_selected(
                            &list_box_clone,
                            &items_clone,
                            &on_action,
                            &dialog_clone,
                        );
                        gtk4::glib::Propagation::Stop
                    }
                    gtk4::gdk::Key::Down => {
                        // Move selection down
                        if let Some(row) = list_box_clone.selected_row() {
                            let idx = row.index();
                            if let Some(next) = list_box_clone.row_at_index(idx + 1) {
                                list_box_clone.select_row(Some(&next));
                            }
                        }
                        gtk4::glib::Propagation::Stop
                    }
                    gtk4::gdk::Key::Up => {
                        // Move selection up
                        if let Some(row) = list_box_clone.selected_row() {
                            let idx = row.index();
                            if idx > 0
                                && let Some(prev) = list_box_clone.row_at_index(idx - 1)
                            {
                                list_box_clone.select_row(Some(&prev));
                            }
                        }
                        gtk4::glib::Propagation::Stop
                    }
                    _ => gtk4::glib::Propagation::Proceed,
                }
            });
            search_entry.add_controller(key_controller);
        }

        // Double-click / row-activated on list → activate
        {
            let items_clone = palette.items.clone();
            let on_action = palette.on_action.clone();
            let dialog_clone = dialog.clone();
            list_box.connect_row_activated(move |_, row| {
                let idx = row.index();
                if idx >= 0 {
                    let items_ref = items_clone.borrow();
                    #[allow(clippy::cast_sign_loss)]
                    if let Some(item) = items_ref.get(idx as usize) {
                        if let Some(ref cb) = *on_action.borrow() {
                            cb(item.action.clone());
                        }
                        dialog_clone.close();
                    }
                }
            });
        }

        palette
    }

    /// Sets the connections available for searching
    pub fn set_connections(&self, connections: Vec<Connection>) {
        *self.connections.borrow_mut() = connections;
    }

    /// Sets the groups available for searching
    pub fn set_groups(&self, groups: Vec<ConnectionGroup>) {
        *self.groups.borrow_mut() = groups;
    }

    /// Registers a callback for when an action is selected
    pub fn connect_on_action<F>(&self, callback: F)
    where
        F: Fn(CommandPaletteAction) + 'static,
    {
        *self.on_action.borrow_mut() = Some(Box::new(callback));
    }

    /// Opens the palette with optional initial text (e.g. ">" for commands mode)
    pub fn present_with_prefix(&self, prefix: &str) {
        self.search_entry.set_text(prefix);
        // Trigger initial population
        let items = Self::compute_items(
            prefix,
            &self.connections.borrow(),
            &self.groups.borrow(),
            &self.search_engine,
        );
        Self::populate_list(&self.list_box, &items);
        *self.items.borrow_mut() = items;
        if let Some(first) = self.list_box.row_at_index(0) {
            self.list_box.select_row(Some(&first));
        }
        self.dialog
            .present(self.parent.as_ref().map(|w| w as &gtk4::Widget));
        // Focus the search entry after presenting
        self.search_entry.grab_focus();
    }

    /// Opens the palette in default (connections) mode
    pub fn present(&self) {
        self.present_with_prefix("");
    }

    /// Computes the palette items based on current input
    fn compute_items(
        input: &str,
        connections: &[Connection],
        groups: &[ConnectionGroup],
        engine: &SearchEngine,
    ) -> Vec<PaletteItem> {
        let (mode, query) = parse_palette_input(input);
        match mode {
            PaletteMode::Commands => Self::filter_commands(query),
            PaletteMode::Tags => Self::filter_by_tag(query, connections),
            PaletteMode::Groups => Self::filter_by_group(query, connections, groups),
            PaletteMode::Connections => {
                Self::search_connections(query, connections, groups, engine)
            }
        }
    }

    /// Filters built-in commands by query
    fn filter_commands(query: &str) -> Vec<PaletteItem> {
        let mut cmds = builtin_commands();
        if query.is_empty() {
            cmds.sort_by(|a, b| b.priority.cmp(&a.priority));
        } else {
            let engine = SearchEngine::new();
            cmds.retain(|item| engine.fuzzy_score(query, &item.label) > 0.0);
            cmds.sort_by(|a, b| {
                let engine = SearchEngine::new();
                let sa = engine.fuzzy_score(query, &a.label);
                let sb = engine.fuzzy_score(query, &b.label);
                sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        cmds
    }

    /// Filters connections by tag
    fn filter_by_tag(query: &str, connections: &[Connection]) -> Vec<PaletteItem> {
        if query.is_empty() {
            // Show all unique tags
            let mut tags: Vec<String> = connections
                .iter()
                .flat_map(|c| c.tags.iter().cloned())
                .collect();
            tags.sort();
            tags.dedup();
            return tags
                .into_iter()
                .map(|tag| {
                    PaletteItem::new(
                        format!("@{tag}"),
                        CommandPaletteAction::GtkAction(format!("win.search:@{tag}")),
                    )
                    .with_icon("tag-symbolic")
                })
                .collect();
        }
        let query_lower = query.to_lowercase();
        connections
            .iter()
            .filter(|c| {
                c.tags
                    .iter()
                    .any(|t| t.to_lowercase().contains(&query_lower))
            })
            .map(|c| Self::connection_to_item(c))
            .collect()
    }

    /// Filters connections by group
    fn filter_by_group(
        query: &str,
        connections: &[Connection],
        groups: &[ConnectionGroup],
    ) -> Vec<PaletteItem> {
        if query.is_empty() {
            // Show all groups
            return groups
                .iter()
                .map(|g| {
                    PaletteItem::new(
                        format!("#{}", g.name),
                        CommandPaletteAction::GtkAction(format!("win.search:#{}", g.name)),
                    )
                    .with_icon("folder-symbolic")
                })
                .collect();
        }
        let query_lower = query.to_lowercase();
        // Find matching groups
        let matching_group_ids: Vec<_> = groups
            .iter()
            .filter(|g| g.name.to_lowercase().contains(&query_lower))
            .map(|g| g.id)
            .collect();
        connections
            .iter()
            .filter(|c| {
                c.group_id
                    .is_some_and(|gid| matching_group_ids.contains(&gid))
            })
            .map(|c| Self::connection_to_item(c))
            .collect()
    }

    /// Searches connections using the fuzzy search engine
    fn search_connections(
        query: &str,
        connections: &[Connection],
        groups: &[ConnectionGroup],
        engine: &SearchEngine,
    ) -> Vec<PaletteItem> {
        if query.is_empty() {
            // Show recent connections (sorted by last_connected desc)
            let mut recent: Vec<_> = connections.to_vec();
            recent.sort_by(|a, b| b.last_connected.cmp(&a.last_connected));
            recent.truncate(20);
            return recent.iter().map(|c| Self::connection_to_item(c)).collect();
        }

        let search_query = SearchQuery::with_text(query);
        let results = engine.search(&search_query, connections, groups);
        results
            .iter()
            .filter_map(|r| {
                connections
                    .iter()
                    .find(|c| c.id == r.connection_id)
                    .map(|c| Self::connection_to_item(c))
            })
            .take(20)
            .collect()
    }

    /// Converts a Connection to a PaletteItem
    fn connection_to_item(conn: &Connection) -> PaletteItem {
        let desc = if let Some(ref user) = conn.username {
            format!("{}@{}:{}", user, conn.host, conn.port)
        } else {
            format!("{}:{}", conn.host, conn.port)
        };
        let icon = get_protocol_icon_by_name(&conn.protocol.to_string());
        PaletteItem::new(conn.name.clone(), CommandPaletteAction::Connect(conn.id))
            .with_description(desc)
            .with_icon(icon)
    }

    /// Populates the ListBox with palette items
    fn populate_list(list_box: &ListBox, items: &[PaletteItem]) {
        // Clear existing rows
        while let Some(row) = list_box.row_at_index(0) {
            list_box.remove(&row);
        }
        for item in items {
            let row = Self::create_row(item);
            list_box.append(&row);
        }
    }

    /// Creates a ListBoxRow for a palette item
    fn create_row(item: &PaletteItem) -> ListBoxRow {
        let row = ListBoxRow::new();
        let hbox = GtkBox::new(Orientation::Horizontal, 8);
        hbox.set_margin_top(6);
        hbox.set_margin_bottom(6);
        hbox.set_margin_start(8);
        hbox.set_margin_end(8);

        // Icon
        if let Some(ref icon_name) = item.icon {
            let icon = Image::from_icon_name(icon_name);
            icon.set_pixel_size(16);
            icon.add_css_class("dim-label");
            hbox.append(&icon);
        }

        // Label + description
        let text_box = GtkBox::new(Orientation::Vertical, 1);
        text_box.set_hexpand(true);

        let label = Label::builder()
            .label(&item.label)
            .halign(gtk4::Align::Start)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        text_box.append(&label);

        if let Some(ref desc) = item.description {
            let desc_label = Label::builder()
                .label(desc)
                .halign(gtk4::Align::Start)
                .ellipsize(gtk4::pango::EllipsizeMode::End)
                .css_classes(["dim-label", "caption"])
                .build();
            text_box.append(&desc_label);
        }

        hbox.append(&text_box);
        row.set_child(Some(&hbox));
        row.set_tooltip_text(Some(&item.label));
        row
    }

    /// Activates the currently selected item
    fn activate_selected(
        list_box: &ListBox,
        items: &Rc<RefCell<Vec<PaletteItem>>>,
        on_action: &PaletteCallback,
        dialog: &adw::Dialog,
    ) {
        if let Some(row) = list_box.selected_row() {
            let idx = row.index();
            if idx >= 0 {
                let items_ref = items.borrow();
                #[allow(clippy::cast_sign_loss)]
                if let Some(item) = items_ref.get(idx as usize) {
                    if let Some(ref cb) = *on_action.borrow() {
                        cb(item.action.clone());
                    }
                    dialog.close();
                }
            }
        }
    }
}
