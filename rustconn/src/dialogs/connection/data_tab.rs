//! Data tab for the connection dialog
//!
//! Contains the Variables and Custom Properties sections, allowing users
//! to define connection-scoped variables and arbitrary metadata.

use crate::i18n::i18n;
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, Label, ListBox, Orientation, ScrolledWindow};
use libadwaita as adw;

/// Creates the Data tab combining Variables and Custom Properties.
///
/// Uses libadwaita components following GNOME HIG.
#[allow(clippy::type_complexity)]
pub(super) fn create_data_tab() -> (GtkBox, ListBox, Button, ListBox, Button) {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .vexpand(true)
        .build();

    let clamp = adw::Clamp::builder()
        .maximum_size(600)
        .tightening_threshold(400)
        .build();

    let content = GtkBox::new(Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    // === Variables Section ===
    let variables_group = adw::PreferencesGroup::builder()
        .title(i18n("Local Variables"))
        .description(i18n("Use ${variable_name} syntax in connection fields"))
        .build();

    let variables_scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .min_content_height(150)
        .build();

    let variables_list = ListBox::builder()
        .selection_mode(gtk4::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    variables_list.set_placeholder(Some(&Label::new(Some(&i18n("No variables defined")))));
    variables_scrolled.set_child(Some(&variables_list));

    variables_group.add(&variables_scrolled);

    let var_button_box = GtkBox::new(Orientation::Horizontal, 8);
    var_button_box.set_halign(gtk4::Align::End);
    var_button_box.set_margin_top(8);

    let add_variable_button = Button::builder()
        .label(&i18n("Add Variable"))
        .css_classes(["suggested-action"])
        .build();
    var_button_box.append(&add_variable_button);

    variables_group.add(&var_button_box);
    content.append(&variables_group);

    // === Custom Properties Section ===
    let properties_group = adw::PreferencesGroup::builder()
        .title(i18n("Custom Properties"))
        .description(i18n(
            "Text, URL (clickable), or Protected (masked) metadata",
        ))
        .build();

    let properties_scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .min_content_height(150)
        .build();

    let properties_list = ListBox::builder()
        .selection_mode(gtk4::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    properties_list.set_placeholder(Some(&Label::new(Some(&i18n("No custom properties")))));
    properties_scrolled.set_child(Some(&properties_list));

    properties_group.add(&properties_scrolled);

    let prop_button_box = GtkBox::new(Orientation::Horizontal, 8);
    prop_button_box.set_halign(gtk4::Align::End);
    prop_button_box.set_margin_top(8);

    let add_property_button = Button::builder()
        .label(&i18n("Add Property"))
        .css_classes(["suggested-action"])
        .build();
    prop_button_box.append(&add_property_button);

    properties_group.add(&prop_button_box);
    content.append(&properties_group);

    clamp.set_child(Some(&content));
    scrolled.set_child(Some(&clamp));

    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.append(&scrolled);

    (
        vbox,
        variables_list,
        add_variable_button,
        properties_list,
        add_property_button,
    )
}
