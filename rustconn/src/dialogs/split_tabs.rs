//! "Split with Other Tabs" picker (issue #423).
//!
//! An `AdwAlertDialog` listing the tabs that can join a split, one check row
//! each. Check rows are ordinary focusable widgets, so the whole choice works
//! from the keyboard (Tab, Space, Enter) and a screen reader reads the tab name
//! and protocol of every row. Nothing is created here: the caller receives the
//! chosen session ids, in tab order, and builds the layout.

use std::rc::Rc;

use adw::prelude::*;
use libadwaita as adw;
use uuid::Uuid;

use crate::i18n::{i18n, i18n_f};

/// A tab offered to the picker.
pub struct SplitTabChoice {
    pub session_id: Uuid,
    pub name: String,
    pub protocol: String,
}

/// Asks which of `choices` should be split together with `owner_name`.
///
/// `on_confirm` runs once, with the checked session ids in the order of
/// `choices`, and only when at least one tab is checked. Cancelling or
/// closing the dialog runs nothing.
pub fn show_split_tabs_dialog(
    parent: &impl IsA<gtk4::Widget>,
    owner_name: &str,
    choices: Vec<SplitTabChoice>,
    on_confirm: impl FnOnce(Vec<Uuid>) + 'static,
) {
    let body = i18n_f(
        "Choose the tabs to show next to “{}”. They are arranged in a grid and keep their connections.",
        &[owner_name],
    );
    let dialog = adw::AlertDialog::new(Some(&i18n("Split with Other Tabs")), Some(&body));

    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::None);
    list.add_css_class("boxed-list");
    list.update_property(&[gtk4::accessible::Property::Label(&i18n("Tabs to split"))]);

    let mut checks: Vec<(Uuid, gtk4::CheckButton)> = Vec::with_capacity(choices.len());
    for choice in choices {
        let row = adw::ActionRow::new();
        row.set_title(&gtk4::glib::markup_escape_text(&choice.name));
        row.set_subtitle(&gtk4::glib::markup_escape_text(
            &choice.protocol.to_uppercase(),
        ));
        let check = gtk4::CheckButton::new();
        check.set_valign(gtk4::Align::Center);
        check.update_property(&[gtk4::accessible::Property::Label(&choice.name)]);
        row.add_prefix(&check);
        // Clicking anywhere on the row toggles the box.
        row.set_activatable_widget(Some(&check));
        list.append(&row);
        checks.push((choice.session_id, check));
    }

    let scroller = gtk4::ScrolledWindow::builder()
        .child(&list)
        .propagate_natural_height(true)
        .max_content_height(360)
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .build();
    dialog.set_extra_child(Some(&scroller));

    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.add_response("split", &i18n("Split"));
    dialog.set_default_response(Some("split"));
    dialog.set_close_response("cancel");
    dialog.set_response_appearance("split", adw::ResponseAppearance::Suggested);
    dialog.set_response_enabled("split", false);

    let checks = Rc::new(checks);
    for (_, check) in checks.iter() {
        let dialog = dialog.downgrade();
        let checks = Rc::clone(&checks);
        check.connect_toggled(move |_| {
            if let Some(dialog) = dialog.upgrade() {
                dialog.set_response_enabled("split", checks.iter().any(|(_, c)| c.is_active()));
            }
        });
    }

    let on_confirm = std::cell::RefCell::new(Some(on_confirm));
    dialog.connect_response(None, move |_, response| {
        if response != "split" {
            return;
        }
        let chosen: Vec<Uuid> = checks
            .iter()
            .filter(|(_, c)| c.is_active())
            .map(|(id, _)| *id)
            .collect();
        if chosen.is_empty() {
            return;
        }
        if let Some(callback) = on_confirm.borrow_mut().take() {
            callback(chosen);
        }
    });

    dialog.present(Some(parent));
}
