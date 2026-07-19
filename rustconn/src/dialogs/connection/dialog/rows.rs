//! Dynamic list rows: custom properties, expect rules, local variables
//!
//! Mechanically split out of `dialog.rs` (pure code motion).

#![allow(
    clippy::similar_names,
    reason = "module-wide override for legacy code; refactored case by case"
)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, CheckButton, DropDown, Entry, Grid, Label, ListBox, ListBoxRow,
    Orientation, PasswordEntry, SpinButton, StringList,
};
use rustconn_core::automation::{ExpectRule, builtin_templates};
use rustconn_core::models::{CustomProperty, PropertyType};
use rustconn_core::variables::Variable;
use uuid::Uuid;

use super::{ConnectionDialog, CustomPropertyRow, ExpectRuleRow, LocalVariableRow};
use crate::i18n::i18n;

impl ConnectionDialog {
    /// Creates a custom property row widget
    pub(super) fn create_custom_property_row(
        property: Option<&CustomProperty>,
    ) -> CustomPropertyRow {
        let main_box = GtkBox::new(Orientation::Vertical, 8);
        main_box.set_margin_top(12);
        main_box.set_margin_bottom(12);
        main_box.set_margin_start(12);
        main_box.set_margin_end(12);

        let grid = Grid::builder()
            .row_spacing(6)
            .column_spacing(8)
            .hexpand(true)
            .build();

        // Row 0: Name and delete button
        let name_label = Label::builder()
            .label(i18n("Name:"))
            .halign(gtk4::Align::End)
            .build();
        let name_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text(i18n("Property name (e.g., asset_tag, docs_url)"))
            .build();

        let delete_button = Button::builder()
            .icon_name("user-trash-symbolic")
            .css_classes(["destructive-action", "flat"])
            .tooltip_text(i18n("Delete property"))
            .build();
        delete_button
            .update_property(&[gtk4::accessible::Property::Label(&i18n("Delete property"))]);

        grid.attach(&name_label, 0, 0, 1, 1);
        grid.attach(&name_entry, 1, 0, 1, 1);
        grid.attach(&delete_button, 2, 0, 1, 1);

        // Row 1: Type dropdown
        let type_label = Label::builder()
            .label(i18n("Type:"))
            .halign(gtk4::Align::End)
            .build();
        let type_list = StringList::new(&[&i18n("Text"), &i18n("URL"), &i18n("Protected")]);
        let type_dropdown = DropDown::new(Some(type_list), gtk4::Expression::NONE);
        type_dropdown.set_selected(0);
        type_dropdown.set_tooltip_text(Some(&i18n(
            "Text: Plain text\nURL: Clickable link\nProtected: Masked/encrypted value",
        )));

        grid.attach(&type_label, 0, 1, 1, 1);
        grid.attach(&type_dropdown, 1, 1, 2, 1);

        // Row 2: Value (regular entry)
        let value_label = Label::builder()
            .label(i18n("Value:"))
            .halign(gtk4::Align::End)
            .build();
        let value_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text(i18n("Property value"))
            .build();

        // Row 2: Value (password entry for protected type)
        let secret_entry = PasswordEntry::builder()
            .hexpand(true)
            .placeholder_text(i18n("Protected value (masked)"))
            .show_peek_icon(true)
            .build();

        grid.attach(&value_label, 0, 2, 1, 1);
        grid.attach(&value_entry, 1, 2, 2, 1);
        // Secret entry is hidden initially, will be shown when type is Protected
        grid.attach(&secret_entry, 1, 3, 2, 1);
        secret_entry.set_visible(false);

        // Connect type dropdown to show/hide appropriate value entry
        let value_entry_clone = value_entry.clone();
        let secret_entry_clone = secret_entry.clone();
        type_dropdown.connect_selected_notify(move |dropdown| {
            let is_protected = dropdown.selected() == 2; // Protected is index 2
            value_entry_clone.set_visible(!is_protected);
            secret_entry_clone.set_visible(is_protected);
        });

        main_box.append(&grid);

        // Populate from existing property if provided
        if let Some(prop) = property {
            name_entry.set_text(&prop.name);
            let type_idx = match prop.property_type {
                PropertyType::Text => 0,
                PropertyType::Url => 1,
                PropertyType::Protected => 2,
            };
            type_dropdown.set_selected(type_idx);

            if prop.is_protected() {
                secret_entry.set_text(&prop.value);
                value_entry.set_visible(false);
                secret_entry.set_visible(true);
            } else {
                value_entry.set_text(&prop.value);
            }
        }

        let row = ListBoxRow::builder().child(&main_box).build();

        CustomPropertyRow {
            row,
            name_entry,
            type_dropdown,
            value_entry,
            secret_entry,
            delete_button,
        }
    }

    /// Wires up the add custom property button
    pub(super) fn wire_add_custom_property_button(
        add_button: &Button,
        properties_list: &ListBox,
        custom_properties: &Rc<RefCell<Vec<CustomProperty>>>,
    ) {
        let list_clone = properties_list.clone();
        let props_clone = custom_properties.clone();

        add_button.connect_clicked(move |_| {
            let prop_row = Self::create_custom_property_row(None);

            // Add a new empty property to the list
            let new_prop = CustomProperty::new_text("", "");
            props_clone.borrow_mut().push(new_prop);
            let prop_index = props_clone.borrow().len() - 1;

            // Connect delete button
            let list_for_delete = list_clone.clone();
            let props_for_delete = props_clone.clone();
            let row_widget = prop_row.row.clone();
            prop_row.delete_button.connect_clicked(move |_| {
                // Find and remove the property by matching the row index
                if let Ok(idx) = usize::try_from(row_widget.index())
                    && idx < props_for_delete.borrow().len()
                {
                    props_for_delete.borrow_mut().remove(idx);
                }
                list_for_delete.remove(&row_widget);
            });

            // Connect entry changes to update the property
            Self::connect_custom_property_changes(&prop_row, &props_clone, prop_index);

            list_clone.append(&prop_row.row);
        });
    }

    /// Connects entry changes to update the custom property in the list
    pub(super) fn connect_custom_property_changes(
        prop_row: &CustomPropertyRow,
        custom_properties: &Rc<RefCell<Vec<CustomProperty>>>,
        initial_index: usize,
    ) {
        // We need to track the row to find its current index
        let row_widget = prop_row.row.clone();

        // Name entry
        let props_for_name = custom_properties.clone();
        let row_for_name = row_widget.clone();
        prop_row.name_entry.connect_changed(move |entry| {
            let text = entry.text().to_string();
            if let Ok(idx) = usize::try_from(row_for_name.index())
                && let Some(prop) = props_for_name.borrow_mut().get_mut(idx)
            {
                prop.name = text;
            }
        });

        // Type dropdown
        let props_for_type = custom_properties.clone();
        let row_for_type = row_widget.clone();
        prop_row
            .type_dropdown
            .connect_selected_notify(move |dropdown| {
                let prop_type = match dropdown.selected() {
                    1 => PropertyType::Url,
                    2 => PropertyType::Protected,
                    _ => PropertyType::Text,
                };
                if let Ok(idx) = usize::try_from(row_for_type.index())
                    && let Some(prop) = props_for_type.borrow_mut().get_mut(idx)
                {
                    prop.property_type = prop_type;
                }
            });

        // Value entry (for Text and URL types)
        let props_for_value = custom_properties.clone();
        let row_for_value = row_widget.clone();
        prop_row.value_entry.connect_changed(move |entry| {
            let text = entry.text().to_string();
            if let Ok(idx) = usize::try_from(row_for_value.index())
                && let Some(prop) = props_for_value.borrow_mut().get_mut(idx)
                && !prop.is_protected()
            {
                prop.value = text;
            }
        });

        // Secret entry (for Protected type)
        let props_for_secret = custom_properties.clone();
        let row_for_secret = row_widget;
        prop_row.secret_entry.connect_changed(move |entry| {
            let text = entry.text().to_string();
            if let Ok(idx) = usize::try_from(row_for_secret.index())
                && let Some(prop) = props_for_secret.borrow_mut().get_mut(idx)
                && prop.is_protected()
            {
                prop.value = text;
            }
        });

        // Suppress unused variable warning
        let _ = initial_index;
    }

    /// Creates an expect rule row widget
    #[expect(
        clippy::too_many_lines,
        reason = "long match/dispatch over many enum variants; splitting per variant only relocates the boilerplate"
    )]
    pub(super) fn create_expect_rule_row(rule: Option<&ExpectRule>) -> ExpectRuleRow {
        let main_box = GtkBox::new(Orientation::Vertical, 6);
        main_box.set_margin_top(12);
        main_box.set_margin_bottom(12);
        main_box.set_margin_start(12);
        main_box.set_margin_end(12);

        // Row 0: Action buttons (delete, move up/down) — top-right for visibility
        let action_box = GtkBox::new(Orientation::Horizontal, 4);
        action_box.set_halign(gtk4::Align::End);

        let move_up_button = Button::builder()
            .icon_name("go-up-symbolic")
            .css_classes(["flat"])
            .tooltip_text(i18n("Move up (higher priority)"))
            .build();
        move_up_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Move rule up"))]);
        let move_down_button = Button::builder()
            .icon_name("go-down-symbolic")
            .css_classes(["flat"])
            .tooltip_text(i18n("Move down (lower priority)"))
            .build();
        move_down_button
            .update_property(&[gtk4::accessible::Property::Label(&i18n("Move rule down"))]);
        let delete_button = Button::builder()
            .icon_name("user-trash-symbolic")
            .css_classes(["flat"])
            .tooltip_text(i18n("Delete rule"))
            .build();
        delete_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Delete rule"))]);
        action_box.append(&move_up_button);
        action_box.append(&move_down_button);
        action_box.append(&delete_button);
        main_box.append(&action_box);

        // Row 1: Pattern entry (full width)
        let pattern_box = GtkBox::new(Orientation::Horizontal, 6);
        let pattern_label = Label::builder()
            .label(i18n("Pattern:"))
            .halign(gtk4::Align::End)
            .width_chars(10)
            .build();
        let pattern_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text(i18n("Regex pattern (e.g., password:\\s*$)"))
            .tooltip_text(i18n("Regular expression to match against terminal output"))
            .build();
        pattern_box.append(&pattern_label);
        pattern_box.append(&pattern_entry);
        main_box.append(&pattern_box);

        // Row 2: Response entry + "Insert Variable" button
        let response_box = GtkBox::new(Orientation::Horizontal, 6);
        let response_label = Label::builder()
            .label(i18n("Response:"))
            .halign(gtk4::Align::End)
            .width_chars(10)
            .build();
        let response_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text(i18n("Text to send (e.g., ${password}\\n)"))
            .tooltip_text(i18n(
                "Response to send when pattern matches. Use ${password}, ${username}, or ${VAR_NAME} for variables.",
            ))
            .build();

        // "Insert Variable" button with popover
        let var_menu_button = gtk4::MenuButton::builder()
            .icon_name("list-add-symbolic")
            .css_classes(["flat"])
            .tooltip_text(i18n("Insert a variable placeholder"))
            .build();
        var_menu_button
            .update_property(&[gtk4::accessible::Property::Label(&i18n("Insert variable"))]);

        let var_popover = gtk4::Popover::new();
        var_popover.set_size_request(220, -1);
        let var_list = GtkBox::new(Orientation::Vertical, 2);
        var_list.set_margin_top(6);
        var_list.set_margin_bottom(6);
        var_list.set_margin_start(6);
        var_list.set_margin_end(6);

        let builtin_header = Label::builder()
            .label(i18n("Built-in"))
            .halign(gtk4::Align::Start)
            .css_classes(["dim-label", "caption"])
            .build();
        var_list.append(&builtin_header);

        for (var_name, var_desc) in [
            ("${password}", i18n("Connection password")),
            ("${username}", i18n("Connection username")),
            ("${host}", i18n("Connection host")),
            ("${port}", i18n("Connection port")),
        ] {
            let btn = Button::builder()
                .label(var_name)
                .css_classes(["flat"])
                .tooltip_text(&var_desc)
                .build();
            let entry_clone = response_entry.clone();
            let var = var_name.to_string();
            btn.connect_clicked(move |btn| {
                let pos = entry_clone.position();
                entry_clone.insert_text(&var, &mut pos.clone());
                #[expect(
    clippy::cast_possible_wrap,
    reason = "value range fits the target signed type by construction in this code path"
)]
                entry_clone.set_position(pos + var.len() as i32);
                if let Some(popover) = btn
                    .ancestor(gtk4::Popover::static_type())
                    .and_then(|w| w.downcast::<gtk4::Popover>().ok())
                {
                    popover.popdown();
                }
            });
            var_list.append(&btn);
        }

        let special_header = Label::builder()
            .label(i18n("Special"))
            .halign(gtk4::Align::Start)
            .css_classes(["dim-label", "caption"])
            .margin_top(4)
            .build();
        var_list.append(&special_header);

        let newline_btn = Button::builder()
            .label("\\n")
            .css_classes(["flat"])
            .tooltip_text(i18n("Newline (Enter key)"))
            .build();
        {
            let entry_clone = response_entry.clone();
            newline_btn.connect_clicked(move |btn| {
                let pos = entry_clone.position();
                entry_clone.insert_text("\\n", &mut pos.clone());
                entry_clone.set_position(pos + 2);
                if let Some(popover) = btn
                    .ancestor(gtk4::Popover::static_type())
                    .and_then(|w| w.downcast::<gtk4::Popover>().ok())
                {
                    popover.popdown();
                }
            });
        }
        var_list.append(&newline_btn);

        var_popover.set_child(Some(&var_list));
        var_menu_button.set_popover(Some(&var_popover));

        response_box.append(&response_label);
        response_box.append(&response_entry);
        response_box.append(&var_menu_button);
        main_box.append(&response_box);

        // Row 3: Priority, Timeout, Enabled, One-shot — compact horizontal row
        let settings_box = GtkBox::new(Orientation::Horizontal, 8);
        settings_box.set_halign(gtk4::Align::Start);

        let priority_label = Label::builder()
            .label(i18n("Priority:"))
            .css_classes(["dim-label", "caption"])
            .build();
        let priority_adj = gtk4::Adjustment::new(0.0, -1000.0, 1000.0, 1.0, 10.0, 0.0);
        let priority_spin = SpinButton::builder()
            .adjustment(&priority_adj)
            .climb_rate(1.0)
            .digits(0)
            .width_chars(5)
            .tooltip_text(i18n("Higher priority rules are checked first"))
            .build();

        let timeout_label = Label::builder()
            .label(i18n("Timeout:"))
            .css_classes(["dim-label", "caption"])
            .build();
        let timeout_adj = gtk4::Adjustment::new(0.0, 0.0, 60000.0, 100.0, 1000.0, 0.0);
        let timeout_spin = SpinButton::builder()
            .adjustment(&timeout_adj)
            .climb_rate(1.0)
            .digits(0)
            .width_chars(6)
            .tooltip_text(i18n("Timeout in milliseconds (0 = no timeout)"))
            .build();

        let enabled_check = CheckButton::builder()
            .label(i18n("Enabled"))
            .active(true)
            .build();

        let one_shot_check = CheckButton::builder()
            .label(i18n("One-shot"))
            .active(true)
            .tooltip_text(i18n("Fire only once, then remove the rule"))
            .build();

        settings_box.append(&priority_label);
        settings_box.append(&priority_spin);
        settings_box.append(&timeout_label);
        settings_box.append(&timeout_spin);
        settings_box.append(&enabled_check);
        settings_box.append(&one_shot_check);
        main_box.append(&settings_box);

        // Row 4: Regex validation label
        let validation_label = Label::builder()
            .halign(gtk4::Align::Start)
            .css_classes(["error"])
            .visible(false)
            .build();
        main_box.append(&validation_label);

        // Wire regex validation on pattern entry
        let validation_label_clone = validation_label.clone();
        pattern_entry.connect_changed(move |entry| {
            let text = entry.text().to_string();
            if text.is_empty() {
                validation_label_clone.set_visible(false);
                entry.remove_css_class("error");
            } else {
                match regex::Regex::new(&text) {
                    Ok(_) => {
                        validation_label_clone.set_visible(false);
                        entry.remove_css_class("error");
                    }
                    Err(e) => {
                        validation_label_clone.set_text(&e.to_string());
                        validation_label_clone.set_visible(true);
                        entry.add_css_class("error");
                    }
                }
            }
        });

        // Populate from existing rule if provided
        let id = rule.map_or_else(Uuid::new_v4, |r| {
            pattern_entry.set_text(&r.pattern);
            response_entry.set_text(&r.response);
            priority_spin.set_value(f64::from(r.priority));
            timeout_spin.set_value(f64::from(r.timeout_ms.unwrap_or(0)));
            enabled_check.set_active(r.enabled);
            one_shot_check.set_active(r.one_shot);
            r.id
        });

        let row = ListBoxRow::builder().child(&main_box).build();

        ExpectRuleRow {
            row,
            id,
            pattern_entry,
            response_entry,
            priority_spin,
            timeout_spin,
            enabled_check,
            one_shot_check,
            delete_button,
            move_up_button,
            move_down_button,
        }
    }

    /// Wires up the add expect rule button
    pub(super) fn wire_add_expect_rule_button(
        add_button: &Button,
        expect_rules_list: &ListBox,
        expect_rules: &Rc<RefCell<Vec<ExpectRule>>>,
    ) {
        let list_clone = expect_rules_list.clone();
        let rules_clone = expect_rules.clone();

        add_button.connect_clicked(move |_| {
            let rule_row = Self::create_expect_rule_row(None);
            let rule_id = rule_row.id;

            // Add a new empty rule to the list
            let new_rule = ExpectRule::with_id(rule_id, "", "");
            rules_clone.borrow_mut().push(new_rule);

            // Connect delete button
            let list_for_delete = list_clone.clone();
            let rules_for_delete = rules_clone.clone();
            let row_widget = rule_row.row.clone();
            let delete_id = rule_id;
            rule_row.delete_button.connect_clicked(move |_| {
                list_for_delete.remove(&row_widget);
                rules_for_delete.borrow_mut().retain(|r| r.id != delete_id);
            });

            // Connect move up button
            let list_for_up = list_clone.clone();
            let rules_for_up = rules_clone.clone();
            let row_for_up = rule_row.row.clone();
            let up_id = rule_id;
            rule_row.move_up_button.connect_clicked(move |_| {
                Self::move_rule_up(&list_for_up, &rules_for_up, &row_for_up, up_id);
            });

            // Connect move down button
            let list_for_down = list_clone.clone();
            let rules_for_down = rules_clone.clone();
            let row_for_down = rule_row.row.clone();
            let down_id = rule_id;
            rule_row.move_down_button.connect_clicked(move |_| {
                Self::move_rule_down(&list_for_down, &rules_for_down, &row_for_down, down_id);
            });

            // Connect entry changes to update the rule
            Self::connect_rule_entry_changes(&rule_row, &rules_clone);

            list_clone.append(&rule_row.row);
        });
    }

    /// Wires up template picker buttons to add preset rules
    pub(super) fn wire_template_buttons(
        template_list_box: &GtkBox,
        expect_rules_list: &ListBox,
        expect_rules: &Rc<RefCell<Vec<ExpectRule>>>,
    ) {
        let templates = builtin_templates();
        let mut child = template_list_box.first_child();
        let mut idx = 0;

        while let Some(widget) = child {
            let next = widget.next_sibling();
            if let Some(btn) = widget.downcast_ref::<Button>()
                && idx < templates.len()
            {
                let list_clone = expect_rules_list.clone();
                let rules_clone = expect_rules.clone();
                let template_idx = idx;

                btn.connect_clicked(move |btn| {
                    let templates = builtin_templates();
                    if template_idx >= templates.len() {
                        return;
                    }
                    let template = &templates[template_idx];
                    let new_rules = template.rules();

                    for rule in &new_rules {
                        let rule_row = Self::create_expect_rule_row(Some(rule));

                        // Connect delete button
                        let list_for_delete = list_clone.clone();
                        let rules_for_delete = rules_clone.clone();
                        let row_widget = rule_row.row.clone();
                        let delete_id = rule_row.id;
                        rule_row.delete_button.connect_clicked(move |_| {
                            list_for_delete.remove(&row_widget);
                            rules_for_delete.borrow_mut().retain(|r| r.id != delete_id);
                        });

                        // Connect move buttons
                        let list_for_up = list_clone.clone();
                        let rules_for_up = rules_clone.clone();
                        let row_for_up = rule_row.row.clone();
                        let up_id = rule_row.id;
                        rule_row.move_up_button.connect_clicked(move |_| {
                            Self::move_rule_up(&list_for_up, &rules_for_up, &row_for_up, up_id);
                        });

                        let list_for_down = list_clone.clone();
                        let rules_for_down = rules_clone.clone();
                        let row_for_down = rule_row.row.clone();
                        let down_id = rule_row.id;
                        rule_row.move_down_button.connect_clicked(move |_| {
                            Self::move_rule_down(
                                &list_for_down,
                                &rules_for_down,
                                &row_for_down,
                                down_id,
                            );
                        });

                        Self::connect_rule_entry_changes(&rule_row, &rules_clone);

                        rules_clone.borrow_mut().push(rule.clone());
                        list_clone.append(&rule_row.row);
                    }

                    // Close the popover
                    if let Some(popover) = btn
                        .ancestor(gtk4::Popover::static_type())
                        .and_then(|w| w.downcast::<gtk4::Popover>().ok())
                    {
                        popover.popdown();
                    }
                });

                idx += 1;
            }
            child = next;
        }
    }

    /// Connects entry changes to update the rule in the list
    pub(super) fn connect_rule_entry_changes(
        rule_row: &ExpectRuleRow,
        expect_rules: &Rc<RefCell<Vec<ExpectRule>>>,
    ) {
        let rule_id = rule_row.id;

        // Pattern entry
        let rules_for_pattern = expect_rules.clone();
        let pattern_id = rule_id;
        rule_row.pattern_entry.connect_changed(move |entry| {
            let text = entry.text().to_string();
            if let Some(rule) = rules_for_pattern
                .borrow_mut()
                .iter_mut()
                .find(|r| r.id == pattern_id)
            {
                rule.pattern = text;
            }
        });

        // Response entry
        let rules_for_response = expect_rules.clone();
        let response_id = rule_id;
        rule_row.response_entry.connect_changed(move |entry| {
            let text = entry.text().to_string();
            if let Some(rule) = rules_for_response
                .borrow_mut()
                .iter_mut()
                .find(|r| r.id == response_id)
            {
                rule.response = text;
            }
        });

        // Priority spin
        let rules_for_priority = expect_rules.clone();
        let priority_id = rule_id;
        rule_row.priority_spin.connect_value_changed(move |spin| {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "value range fits the target type by construction in this code path"
            )]
            let value = spin.value() as i32;
            if let Some(rule) = rules_for_priority
                .borrow_mut()
                .iter_mut()
                .find(|r| r.id == priority_id)
            {
                rule.priority = value;
            }
        });

        // Timeout spin
        let rules_for_timeout = expect_rules.clone();
        let timeout_id = rule_id;
        rule_row.timeout_spin.connect_value_changed(move |spin| {
            #[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "value range fits the target type and is non-negative by construction in this code path"
)]
            let value = spin.value() as u32;
            if let Some(rule) = rules_for_timeout
                .borrow_mut()
                .iter_mut()
                .find(|r| r.id == timeout_id)
            {
                rule.timeout_ms = if value == 0 { None } else { Some(value) };
            }
        });

        // Enabled checkbox
        let rules_for_enabled = expect_rules.clone();
        let enabled_id = rule_id;
        rule_row.enabled_check.connect_toggled(move |check| {
            let enabled = check.is_active();
            if let Some(rule) = rules_for_enabled
                .borrow_mut()
                .iter_mut()
                .find(|r| r.id == enabled_id)
            {
                rule.enabled = enabled;
            }
        });

        // One-shot checkbox
        let rules_for_one_shot = expect_rules.clone();
        let one_shot_id = rule_id;
        rule_row.one_shot_check.connect_toggled(move |check| {
            let one_shot = check.is_active();
            if let Some(rule) = rules_for_one_shot
                .borrow_mut()
                .iter_mut()
                .find(|r| r.id == one_shot_id)
            {
                rule.one_shot = one_shot;
            }
        });
    }

    /// Moves a rule up in the list (increases priority)
    pub(super) fn move_rule_up(
        list: &ListBox,
        rules: &Rc<RefCell<Vec<ExpectRule>>>,
        row: &ListBoxRow,
        _rule_id: Uuid,
    ) {
        let index = row.index();
        if index <= 0 {
            return;
        }

        // Remove and re-insert the row
        list.remove(row);
        let new_index = index - 1;
        list.insert(row, new_index);

        // Update the rules vector
        #[expect(
            clippy::cast_sign_loss,
            reason = "value is non-negative by construction in this code path"
        )]
        let idx = index as usize;
        let mut rules_vec = rules.borrow_mut();
        if idx < rules_vec.len() {
            rules_vec.swap(idx, idx - 1);
        }
    }

    /// Moves a rule down in the list (decreases priority)
    pub(super) fn move_rule_down(
        list: &ListBox,
        rules: &Rc<RefCell<Vec<ExpectRule>>>,
        row: &ListBoxRow,
        _rule_id: Uuid,
    ) {
        let index = row.index();
        let rules_len = rules.borrow().len();
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_possible_wrap,
            reason = "value range fits both signed and unsigned target types by construction in this code path"
        )]
        if index < 0 || index >= (rules_len as i32 - 1) {
            return;
        }

        // Remove and re-insert the row
        list.remove(row);
        let new_index = index + 1;
        list.insert(row, new_index);

        // Update the rules vector
        #[expect(
            clippy::cast_sign_loss,
            reason = "value is non-negative by construction in this code path"
        )]
        let idx = index as usize;
        let mut rules_vec = rules.borrow_mut();
        if idx + 1 < rules_vec.len() {
            rules_vec.swap(idx, idx + 1);
        }
    }

    /// Wires up the pattern tester
    pub(super) fn wire_pattern_tester(
        test_entry: &Entry,
        result_label: &Label,
        expect_rules: &Rc<RefCell<Vec<ExpectRule>>>,
    ) {
        let rules_clone = expect_rules.clone();
        let result_clone = result_label.clone();

        test_entry.connect_changed(move |entry| {
            let test_text = entry.text().to_string();
            if test_text.is_empty() {
                result_clone.set_text(&i18n("Enter text above to test patterns"));
                result_clone.remove_css_class("success");
                result_clone.remove_css_class("error");
                result_clone.add_css_class("dim-label");
                return;
            }

            let rules = rules_clone.borrow();
            let mut matched = false;

            // Sort rules by priority (highest first) for testing
            let mut sorted_rules: Vec<_> = rules
                .iter()
                .filter(|r| r.enabled && !r.pattern.is_empty())
                .collect();
            sorted_rules.sort_by_key(|b| std::cmp::Reverse(b.priority));

            for rule in sorted_rules {
                match regex::Regex::new(&rule.pattern) {
                    Ok(re) => {
                        if re.is_match(&test_text) {
                            result_clone.set_text(&format!(
                                "✓ Matched pattern: \"{}\"\n  Response: \"{}\"",
                                rule.pattern, rule.response
                            ));
                            result_clone.remove_css_class("dim-label");
                            result_clone.remove_css_class("error");
                            result_clone.add_css_class("success");
                            matched = true;
                            break;
                        }
                    }
                    Err(e) => {
                        result_clone
                            .set_text(&format!("✗ Invalid pattern \"{}\": {}", rule.pattern, e));
                        result_clone.remove_css_class("dim-label");
                        result_clone.remove_css_class("success");
                        result_clone.add_css_class("error");
                        return;
                    }
                }
            }

            if !matched {
                result_clone.set_text(&i18n("No patterns matched"));
                result_clone.remove_css_class("success");
                result_clone.remove_css_class("error");
                result_clone.add_css_class("dim-label");
            }
        });
    }

    /// Creates a local variable row widget
    pub(super) fn create_local_variable_row(
        variable: Option<&Variable>,
        is_inherited: bool,
    ) -> LocalVariableRow {
        let main_box = GtkBox::new(Orientation::Vertical, 8);
        main_box.set_margin_top(12);
        main_box.set_margin_bottom(12);
        main_box.set_margin_start(12);
        main_box.set_margin_end(12);

        let grid = Grid::builder()
            .row_spacing(6)
            .column_spacing(8)
            .hexpand(true)
            .build();

        // Row 0: Name and Delete button
        let name_label = Label::builder()
            .label(i18n("Name:"))
            .halign(gtk4::Align::End)
            .build();
        let name_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("variable_name")
            .editable(!is_inherited)
            .build();

        if is_inherited {
            name_entry.add_css_class("dim-label");
        }

        let delete_button = Button::builder()
            .icon_name("user-trash-symbolic")
            .css_classes(["destructive-action", "flat"])
            .tooltip_text(if is_inherited {
                i18n("Remove override")
            } else {
                i18n("Delete variable")
            })
            .build();
        delete_button.update_property(&[gtk4::accessible::Property::Label(&if is_inherited {
            i18n("Remove variable override")
        } else {
            i18n("Delete variable")
        })]);

        grid.attach(&name_label, 0, 0, 1, 1);
        grid.attach(&name_entry, 1, 0, 1, 1);
        grid.attach(&delete_button, 2, 0, 1, 1);

        // Row 1: Value (regular entry)
        let value_label = Label::builder()
            .label(i18n("Value:"))
            .halign(gtk4::Align::End)
            .build();
        let value_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text(i18n("Variable value"))
            .build();

        grid.attach(&value_label, 0, 1, 1, 1);
        grid.attach(&value_entry, 1, 1, 2, 1);

        // Row 2: Secret value (password entry, initially hidden)
        let secret_label = Label::builder()
            .label(i18n("Secret Value:"))
            .halign(gtk4::Align::End)
            .visible(false)
            .build();
        let secret_entry = PasswordEntry::builder()
            .hexpand(true)
            .placeholder_text(i18n("Secret value (masked)"))
            .show_peek_icon(true)
            .visible(false)
            .build();

        grid.attach(&secret_label, 0, 2, 1, 1);
        grid.attach(&secret_entry, 1, 2, 2, 1);

        // Row 3: Is Secret checkbox
        let is_secret_check = CheckButton::builder()
            .label(i18n("Secret (mask value)"))
            .build();

        grid.attach(&is_secret_check, 1, 3, 2, 1);

        // Row 4: Description
        let desc_label = Label::builder()
            .label(i18n("Description:"))
            .halign(gtk4::Align::End)
            .build();
        let description_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text(i18n("Optional description"))
            .build();

        grid.attach(&desc_label, 0, 4, 1, 1);
        grid.attach(&description_entry, 1, 4, 2, 1);

        // Add inherited indicator if applicable
        if is_inherited {
            let inherited_label = Label::builder()
                .label(i18n("(Inherited from global - override value below)"))
                .halign(gtk4::Align::Start)
                .css_classes(["dim-label"])
                .build();
            grid.attach(&inherited_label, 1, 5, 2, 1);
        }

        main_box.append(&grid);

        // Connect is_secret checkbox to toggle value/secret entry visibility
        let value_entry_clone = value_entry.clone();
        let secret_entry_clone = secret_entry.clone();
        let value_label_clone = value_label.clone();
        let secret_label_clone = secret_label.clone();
        is_secret_check.connect_toggled(move |check| {
            let is_secret = check.is_active();
            value_label_clone.set_visible(!is_secret);
            value_entry_clone.set_visible(!is_secret);
            secret_label_clone.set_visible(is_secret);
            secret_entry_clone.set_visible(is_secret);

            // Transfer value between entries when toggling
            if is_secret {
                let value = value_entry_clone.text();
                secret_entry_clone.set_text(&value);
                value_entry_clone.set_text("");
            } else {
                let value = secret_entry_clone.text();
                value_entry_clone.set_text(&value);
                secret_entry_clone.set_text("");
            }
        });

        // Populate from existing variable if provided
        if let Some(var) = variable {
            name_entry.set_text(&var.name);
            if var.is_secret {
                is_secret_check.set_active(true);
                secret_entry.set_text(&var.value);
            } else {
                value_entry.set_text(&var.value);
            }
            if let Some(ref desc) = var.description {
                description_entry.set_text(desc);
            }
        }

        let row = ListBoxRow::builder().child(&main_box).build();

        LocalVariableRow {
            row,
            name_entry,
            value_entry,
            secret_entry,
            is_secret_check,
            description_entry,
            delete_button,
            is_inherited,
        }
    }

    /// Wires up the add variable button
    pub(super) fn wire_add_variable_button(
        add_button: &Button,
        variables_list: &ListBox,
        variables_rows: &Rc<RefCell<Vec<LocalVariableRow>>>,
    ) {
        let list_clone = variables_list.clone();
        let rows_clone = variables_rows.clone();

        add_button.connect_clicked(move |_| {
            let var_row = Self::create_local_variable_row(None, false);

            // Connect delete button
            let list_for_delete = list_clone.clone();
            let rows_for_delete = rows_clone.clone();
            let row_widget = var_row.row.clone();
            var_row.delete_button.connect_clicked(move |_| {
                list_for_delete.remove(&row_widget);
                let mut rows = rows_for_delete.borrow_mut();
                rows.retain(|r| r.row != row_widget);
            });

            list_clone.append(&var_row.row);
            rows_clone.borrow_mut().push(var_row);
        });
    }

    /// Collects all local variables from the dialog
    pub(super) fn collect_local_variables(
        variables_rows: &Rc<RefCell<Vec<LocalVariableRow>>>,
    ) -> HashMap<String, Variable> {
        let rows = variables_rows.borrow();
        let mut vars = HashMap::new();

        for row in rows.iter() {
            let name = row.name_entry.text().trim().to_string();
            if name.is_empty() {
                continue;
            }

            let is_secret = row.is_secret_check.is_active();
            let value = if is_secret {
                row.secret_entry.text().to_string()
            } else {
                row.value_entry.text().to_string()
            };

            let desc = row.description_entry.text();
            let description = if desc.trim().is_empty() {
                None
            } else {
                Some(desc.trim().to_string())
            };

            let mut var = Variable::new(name.clone(), value);
            var.is_secret = is_secret;
            var.description = description;
            vars.insert(name, var);
        }

        vars
    }
}
