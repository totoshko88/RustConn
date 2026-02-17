//! Empty state widgets for showing helpful messages when content is empty
//!
//! Provides consistent empty state UI following GNOME HIG patterns using adw::StatusPage.

use gtk4::prelude::*;
use gtk4::{Button, Widget};
use libadwaita as adw;

use crate::i18n::i18n;

/// Creates an empty state widget with icon, title, description, and optional action
///
/// # Arguments
///
/// * `icon_name` - The icon to display (symbolic icon name)
/// * `title` - The main title text
/// * `description` - A helpful description or suggestion
/// * `action_label` - Optional button label for a primary action
/// * `action_name` - Optional action name to trigger when button is clicked
///
/// # Returns
///
/// A `Widget` containing the empty state UI (adw::StatusPage)
#[must_use]
pub fn create_empty_state(
    icon_name: &str,
    title: &str,
    description: &str,
    action_label: Option<&str>,
    action_name: Option<&str>,
) -> Widget {
    let page = adw::StatusPage::builder()
        .icon_name(icon_name)
        .title(title)
        .description(description)
        .vexpand(true)
        .hexpand(true)
        .build();

    // Optional action button
    if let (Some(label), Some(action)) = (action_label, action_name) {
        let button = Button::builder()
            .label(label)
            .action_name(action)
            .css_classes(["suggested-action", "pill"])
            .halign(gtk4::Align::Center)
            .build();
        page.set_child(Some(&button));
    }

    page.upcast()
}

/// Creates an empty state for no connections
#[must_use]
pub fn no_connections() -> Widget {
    create_empty_state(
        "network-server-symbolic",
        &i18n("No Connections"),
        &i18n("Create your first connection to get started"),
        Some(&i18n("New Connection")),
        Some("win.new-connection"),
    )
}

/// Creates an empty state for no search results
#[must_use]
pub fn no_search_results(query: &str) -> Widget {
    create_empty_state(
        "edit-find-symbolic",
        &i18n("No Results Found"),
        &format!("{} \"{query}\"", i18n("No connections match")),
        None,
        None,
    )
}

/// Creates an empty state for no sessions
#[must_use]
pub fn no_sessions() -> Widget {
    create_empty_state(
        "utilities-terminal-symbolic",
        &i18n("No Active Sessions"),
        &i18n("Connect to a server to start a session"),
        Some(&i18n("Quick Connect")),
        Some("win.quick-connect"),
    )
}

/// Creates an empty state for no groups
#[must_use]
pub fn no_groups() -> Widget {
    create_empty_state(
        "folder-symbolic",
        &i18n("No Groups"),
        &i18n("Create groups to organize your connections"),
        Some(&i18n("New Group")),
        Some("win.new-group"),
    )
}

/// Creates an empty state for no snippets
#[must_use]
pub fn no_snippets() -> Widget {
    create_empty_state(
        "edit-paste-symbolic",
        &i18n("No Snippets"),
        &i18n("Create command snippets for quick access"),
        Some(&i18n("New Snippet")),
        Some("win.new-snippet"),
    )
}

/// Creates an empty state for no templates
#[must_use]
pub fn no_templates() -> Widget {
    create_empty_state(
        "document-new-symbolic",
        &i18n("No Templates"),
        &i18n("Create templates for common connection configurations"),
        Some(&i18n("New Template")),
        Some("win.new-template"),
    )
}

/// Creates an empty state for no history
#[must_use]
pub fn no_history() -> Widget {
    create_empty_state(
        "document-open-recent-symbolic",
        &i18n("No Connection History"),
        &i18n("Your recent connections will appear here"),
        None,
        None,
    )
}

/// Creates a loading state widget
#[must_use]
pub fn loading_state(message: &str) -> Widget {
    let page = adw::StatusPage::builder()
        .title(i18n("Loading..."))
        .description(message)
        .vexpand(true)
        .hexpand(true)
        .build();

    let spinner = gtk4::Spinner::builder()
        .spinning(true)
        .halign(gtk4::Align::Center)
        .build();
    spinner.set_size_request(48, 48);

    // StatusPage icon property expects an icon name or paintable, not a widget.
    // So we use set_child for the spinner.
    // Wait, StatusPage with spinner is not standard. Usually Spinner is child.
    // But StatusPage takes one child.
    // If we want Title + Description + Spinner, we need a Box as child containing Spinner?
    // Actually StatusPage has `child` property.
    // Let's put spinner in child.

    page.set_child(Some(&spinner));

    page.upcast()
}
