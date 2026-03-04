//! Automation tab for the connection dialog
//!
//! Contains the Expect Rules section (auto-respond to terminal patterns),
//! a pattern tester, and pre-connect / post-disconnect task configuration.

use crate::i18n::i18n;
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, CheckButton, Entry, Label, ListBox, Orientation, ScrolledWindow,
    SpinButton,
};
use libadwaita as adw;
use rustconn_core::automation::builtin_templates;

/// Creates the combined Automation tab (Expect Rules + Tasks).
#[allow(clippy::type_complexity, clippy::too_many_lines)]
pub(super) fn create_automation_combined_tab() -> (
    GtkBox,
    ListBox,
    Button,
    GtkBox,
    Entry,
    Label,
    CheckButton,
    Entry,
    SpinButton,
    CheckButton,
    CheckButton,
    CheckButton,
    Entry,
    SpinButton,
    CheckButton,
) {
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

    // === Expect Rules Section ===
    let rules_group = adw::PreferencesGroup::builder()
        .title(i18n("Expect Rules"))
        .description(i18n("Auto-respond to terminal patterns (priority order)"))
        .build();

    let rules_scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .min_content_height(120)
        .build();

    let expect_rules_list = ListBox::builder()
        .selection_mode(gtk4::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    expect_rules_list.set_placeholder(Some(&Label::new(Some(&i18n("No expect rules")))));
    rules_scrolled.set_child(Some(&expect_rules_list));

    rules_group.add(&rules_scrolled);

    let rules_button_box = GtkBox::new(Orientation::Horizontal, 8);
    rules_button_box.set_halign(gtk4::Align::End);
    rules_button_box.set_margin_top(8);

    let template_menu_button = gtk4::MenuButton::builder()
        .label(&i18n("From Template"))
        .tooltip_text(i18n("Add rules from a built-in template"))
        .build();

    let template_popover = gtk4::Popover::new();
    let template_list_box = GtkBox::new(Orientation::Vertical, 4);
    template_list_box.set_margin_top(8);
    template_list_box.set_margin_bottom(8);
    template_list_box.set_margin_start(8);
    template_list_box.set_margin_end(8);

    for template in builtin_templates() {
        let btn = Button::builder()
            .label(template.name)
            .css_classes(["flat"])
            .tooltip_text(template.description)
            .build();
        template_list_box.append(&btn);
    }
    template_popover.set_child(Some(&template_list_box));
    template_menu_button.set_popover(Some(&template_popover));

    let add_rule_button = Button::builder()
        .label(&i18n("Add Rule"))
        .css_classes(["suggested-action"])
        .build();
    rules_button_box.append(&template_menu_button);
    rules_button_box.append(&add_rule_button);

    rules_group.add(&rules_button_box);
    content.append(&rules_group);

    // Pattern tester
    let tester_group = adw::PreferencesGroup::builder()
        .title(i18n("Pattern Tester"))
        .build();

    let test_entry = Entry::builder()
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .placeholder_text(&i18n("Test text against patterns"))
        .build();

    let test_row = adw::ActionRow::builder().title(i18n("Test Input")).build();
    test_row.add_suffix(&test_entry);
    tester_group.add(&test_row);

    let result_label = Label::builder()
        .label(&i18n("Enter text to test"))
        .halign(gtk4::Align::Start)
        .wrap(true)
        .css_classes(["dim-label"])
        .build();

    let result_row = adw::ActionRow::builder().title(i18n("Result")).build();
    result_row.add_suffix(&result_label);
    tester_group.add(&result_row);

    content.append(&tester_group);

    // === Pre-Connect Task Section ===
    let (
        pre_connect_group,
        pre_connect_enabled_check,
        pre_connect_command_entry,
        pre_connect_timeout_spin,
        pre_connect_abort_check,
        pre_connect_first_only_check,
    ) = create_task_section(&i18n("Pre-Connect Task"), true);
    content.append(&pre_connect_group);

    // === Post-Disconnect Task Section ===
    let (
        post_disconnect_group,
        post_disconnect_enabled_check,
        post_disconnect_command_entry,
        post_disconnect_timeout_spin,
        _post_disconnect_abort_check,
        post_disconnect_last_only_check,
    ) = create_task_section(&i18n("Post-Disconnect Task"), false);
    content.append(&post_disconnect_group);

    clamp.set_child(Some(&content));
    scrolled.set_child(Some(&clamp));

    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.append(&scrolled);

    (
        vbox,
        expect_rules_list,
        add_rule_button,
        template_list_box,
        test_entry,
        result_label,
        pre_connect_enabled_check,
        pre_connect_command_entry,
        pre_connect_timeout_spin,
        pre_connect_abort_check,
        pre_connect_first_only_check,
        post_disconnect_enabled_check,
        post_disconnect_command_entry,
        post_disconnect_timeout_spin,
        post_disconnect_last_only_check,
    )
}

/// Creates a task section (pre-connect or post-disconnect).
///
/// Uses libadwaita components following GNOME HIG.
pub(super) fn create_task_section(
    title: &str,
    is_pre_connect: bool,
) -> (
    adw::PreferencesGroup,
    CheckButton,
    Entry,
    SpinButton,
    CheckButton,
    CheckButton,
) {
    let description = if is_pre_connect {
        i18n("Run command before connecting. Supports ${variable} substitution.")
    } else {
        i18n("Run command after disconnecting. Supports ${variable} substitution.")
    };

    let group = adw::PreferencesGroup::builder()
        .title(title)
        .description(description)
        .build();

    // Enable checkbox
    let enabled_check = CheckButton::builder().valign(gtk4::Align::Center).build();

    let enable_row = adw::ActionRow::builder()
        .title(i18n("Enable Task"))
        .activatable_widget(&enabled_check)
        .build();
    enable_row.add_suffix(&enabled_check);
    group.add(&enable_row);

    // Command entry
    let command_entry = Entry::builder()
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .placeholder_text(i18n("e.g., /path/to/script.sh or vpn-connect ${host}"))
        .sensitive(false)
        .build();

    let command_row = adw::ActionRow::builder()
        .title(i18n("Command"))
        .subtitle(i18n(
            "Shell command to execute (supports ${variable} syntax)",
        ))
        .build();
    command_row.add_suffix(&command_entry);
    group.add(&command_row);

    // Timeout
    let timeout_adj = gtk4::Adjustment::new(0.0, 0.0, 300_000.0, 1000.0, 5000.0, 0.0);
    let timeout_spin = SpinButton::builder()
        .adjustment(&timeout_adj)
        .climb_rate(1.0)
        .digits(0)
        .valign(gtk4::Align::Center)
        .sensitive(false)
        .build();

    let timeout_row = adw::ActionRow::builder()
        .title(i18n("Timeout (ms)"))
        .subtitle(i18n("0 = no timeout"))
        .build();
    timeout_row.add_suffix(&timeout_spin);
    group.add(&timeout_row);

    // Abort on failure (pre-connect only)
    let abort_check = CheckButton::builder()
        .valign(gtk4::Align::Center)
        .active(true)
        .sensitive(false)
        .build();

    if is_pre_connect {
        let abort_row = adw::ActionRow::builder()
            .title(i18n("Abort on Failure"))
            .subtitle(i18n("Cancel connection if this task fails"))
            .activatable_widget(&abort_check)
            .build();
        abort_row.add_suffix(&abort_check);
        group.add(&abort_row);
    }

    // Condition checkbox
    let condition_check = CheckButton::builder()
        .valign(gtk4::Align::Center)
        .sensitive(false)
        .build();

    let (condition_title, condition_subtitle) = if is_pre_connect {
        (
            i18n("First Connection Only"),
            i18n("Only run when opening the first connection in a folder (useful for VPN setup)"),
        )
    } else {
        (
            i18n("Last Connection Only"),
            i18n("Only run when closing the last connection in a folder (useful for cleanup)"),
        )
    };

    let condition_row = adw::ActionRow::builder()
        .title(condition_title)
        .subtitle(condition_subtitle)
        .activatable_widget(&condition_check)
        .build();
    condition_row.add_suffix(&condition_check);
    group.add(&condition_row);

    // Connect enabled checkbox to enable/disable other fields
    let command_entry_clone = command_entry.clone();
    let timeout_spin_clone = timeout_spin.clone();
    let abort_check_clone = abort_check.clone();
    let condition_check_clone = condition_check.clone();
    enabled_check.connect_toggled(move |check| {
        let enabled = check.is_active();
        command_entry_clone.set_sensitive(enabled);
        timeout_spin_clone.set_sensitive(enabled);
        abort_check_clone.set_sensitive(enabled);
        condition_check_clone.set_sensitive(enabled);
    });

    (
        group,
        enabled_check,
        command_entry,
        timeout_spin,
        abort_check,
        condition_check,
    )
}
