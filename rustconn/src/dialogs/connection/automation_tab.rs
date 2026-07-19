//! Automation tab for the connection dialog
//!
//! Contains the Expect Rules section (auto-respond to terminal patterns),
//! a pattern tester, and pre-connect / post-disconnect task configuration.

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, Entry, Label, ListBox, Orientation, ScrolledWindow, SpinButton};
use libadwaita as adw;
use rustconn_core::automation::builtin_templates;

use crate::i18n::i18n;

/// All widgets created by [`create_automation_combined_tab`].
pub(super) struct AutomationTabWidgets {
    /// The outer container added to the view stack.
    pub(super) container: GtkBox,
    /// Expect rules list box.
    pub(super) expect_rules_list: ListBox,
    /// Button to add a new expect rule.
    pub(super) add_expect_rule_button: Button,
    /// Container holding template picker buttons.
    pub(super) template_list_box: GtkBox,
    /// Entry for the pattern tester.
    pub(super) expect_pattern_test_entry: Entry,
    /// Label showing pattern test results.
    pub(super) expect_test_result_label: Label,
    /// Pre-connect task enabled switch.
    pub(super) pre_connect_enabled_switch: adw::SwitchRow,
    /// Pre-connect command entry.
    pub(super) pre_connect_command_entry: Entry,
    /// Pre-connect timeout spin button.
    pub(super) pre_connect_timeout_spin: SpinButton,
    /// Pre-connect abort on failure switch.
    pub(super) pre_connect_abort_switch: adw::SwitchRow,
    /// Pre-connect first-connection-only switch.
    pub(super) pre_connect_first_only_switch: adw::SwitchRow,
    /// Post-disconnect task enabled switch.
    pub(super) post_disconnect_enabled_switch: adw::SwitchRow,
    /// Post-disconnect command entry.
    pub(super) post_disconnect_command_entry: Entry,
    /// Post-disconnect timeout spin button.
    pub(super) post_disconnect_timeout_spin: SpinButton,
    /// Post-disconnect last-connection-only switch.
    pub(super) post_disconnect_last_only_switch: adw::SwitchRow,
}

/// Creates the combined Automation tab (Expect Rules + Tasks).
#[expect(
    clippy::too_many_lines,
    reason = "long match/dispatch over many enum variants; splitting per variant only relocates the boilerplate"
)]
pub(super) fn create_automation_combined_tab() -> AutomationTabWidgets {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .vexpand(true)
        .build();
    scrolled.set_overlay_scrolling(true);

    let clamp = adw::Clamp::builder()
        .maximum_size(600)
        .tightening_threshold(600)
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

    // Info banner about variable substitution (consistent with group dialog)
    let variables_info = Label::builder()
        .label(i18n(
            "Responses support ${password}, ${username}, and ${VARIABLE_NAME} placeholders resolved at connection time",
        ))
        .wrap(true)
        .halign(gtk4::Align::Start)
        .css_classes(["dim-label", "caption"])
        .build();
    variables_info.set_margin_bottom(6);
    rules_group.add(&variables_info);

    let expect_rules_list = ListBox::builder()
        .selection_mode(gtk4::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    expect_rules_list.set_placeholder(Some(&Label::new(Some(&i18n("No expect rules")))));

    // No inner ScrolledWindow — the tab's own scrolled window handles scrolling.
    // This avoids the scroll-in-scroll anti-pattern (GNOME HIG).
    rules_group.add(&expect_rules_list);

    let rules_button_box = GtkBox::new(Orientation::Horizontal, 8);
    rules_button_box.set_halign(gtk4::Align::End);
    rules_button_box.set_margin_top(12);

    let template_menu_button = gtk4::MenuButton::builder()
        .label(i18n("From Template"))
        .tooltip_text(i18n("Add rules from a built-in template"))
        .build();

    let template_popover = gtk4::Popover::new();
    let template_list_box = GtkBox::new(Orientation::Vertical, 4);
    template_list_box.set_margin_top(12);
    template_list_box.set_margin_bottom(12);
    template_list_box.set_margin_start(12);
    template_list_box.set_margin_end(12);

    for template in builtin_templates() {
        // Add protocol hint to SSH-specific templates for consistency with group dialog
        let label = if template.protocol_hint.is_empty() {
            template.name.to_string()
        } else {
            format!(
                "{} ({})",
                template.name,
                template.protocol_hint.to_uppercase()
            )
        };
        let btn = Button::builder()
            .label(&label)
            .css_classes(["flat"])
            .tooltip_text(template.description)
            .build();
        template_list_box.append(&btn);
    }
    template_popover.set_child(Some(&template_list_box));
    // Fixed width prevents layout shifts when different templates are selected
    template_popover.set_size_request(280, -1);
    template_menu_button.set_popover(Some(&template_popover));

    let add_rule_button = Button::builder()
        .label(i18n("Add Rule"))
        .css_classes(["suggested-action"])
        .build();
    rules_button_box.append(&template_menu_button);
    rules_button_box.append(&add_rule_button);

    rules_group.add(&rules_button_box);
    content.append(&rules_group);

    // Pattern tester (collapsible)
    let tester_group = adw::PreferencesGroup::builder().build();
    let tester_expander = adw::ExpanderRow::builder()
        .title(i18n("Pattern Tester"))
        .subtitle(i18n("Test text against expect rule patterns"))
        .show_enable_switch(false)
        .build();

    let test_entry = Entry::builder()
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .placeholder_text(i18n("Test text against patterns"))
        .build();

    let test_row = adw::ActionRow::builder().title(i18n("Test Input")).build();
    test_row.add_suffix(&test_entry);
    tester_expander.add_row(&test_row);

    let result_label = Label::builder()
        .label(i18n("Enter text to test"))
        .halign(gtk4::Align::Start)
        .wrap(true)
        .css_classes(["dim-label"])
        .build();

    let result_row = adw::ActionRow::builder().title(i18n("Result")).build();
    result_row.add_suffix(&result_label);
    tester_expander.add_row(&result_row);

    tester_group.add(&tester_expander);
    content.append(&tester_group);

    // === Pre-Connect Task Section ===
    let (
        pre_connect_group,
        pre_connect_enabled_switch,
        pre_connect_command_entry,
        pre_connect_timeout_spin,
        pre_connect_abort_switch,
        pre_connect_first_only_switch,
    ) = create_task_section(&i18n("Pre-Connect Task"), true);
    content.append(&pre_connect_group);

    // === Post-Disconnect Task Section ===
    let (
        post_disconnect_group,
        post_disconnect_enabled_switch,
        post_disconnect_command_entry,
        post_disconnect_timeout_spin,
        _post_disconnect_abort_switch,
        post_disconnect_last_only_switch,
    ) = create_task_section(&i18n("Post-Disconnect Task"), false);
    content.append(&post_disconnect_group);

    clamp.set_child(Some(&content));
    scrolled.set_child(Some(&clamp));

    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.append(&scrolled);

    AutomationTabWidgets {
        container: vbox,
        expect_rules_list,
        add_expect_rule_button: add_rule_button,
        template_list_box,
        expect_pattern_test_entry: test_entry,
        expect_test_result_label: result_label,
        pre_connect_enabled_switch,
        pre_connect_command_entry,
        pre_connect_timeout_spin,
        pre_connect_abort_switch,
        pre_connect_first_only_switch,
        post_disconnect_enabled_switch,
        post_disconnect_command_entry,
        post_disconnect_timeout_spin,
        post_disconnect_last_only_switch,
    }
}

/// Creates a task section (pre-connect or post-disconnect) wrapped in an `ExpanderRow`.
///
/// Uses libadwaita components following GNOME HIG.
pub(super) fn create_task_section(
    title: &str,
    is_pre_connect: bool,
) -> (
    adw::PreferencesGroup,
    adw::SwitchRow,
    Entry,
    SpinButton,
    adw::SwitchRow,
    adw::SwitchRow,
) {
    let subtitle = if is_pre_connect {
        i18n("Run command before connecting")
    } else {
        i18n("Run command after disconnecting")
    };

    let group = adw::PreferencesGroup::builder().build();
    let expander = adw::ExpanderRow::builder()
        .title(title)
        .subtitle(subtitle)
        .show_enable_switch(false)
        .build();

    // Enable switch
    let enabled_switch = adw::SwitchRow::builder()
        .title(i18n("Enable Task"))
        .active(false)
        .build();
    expander.add_row(&enabled_switch);

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
    expander.add_row(&command_row);

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
    expander.add_row(&timeout_row);

    // Abort on failure (pre-connect only)
    let abort_switch = adw::SwitchRow::builder()
        .title(i18n("Abort on Failure"))
        .subtitle(i18n("Cancel connection if this task fails"))
        .active(true)
        .sensitive(false)
        .build();

    if is_pre_connect {
        expander.add_row(&abort_switch);
    }

    // Condition switch
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

    let condition_switch = adw::SwitchRow::builder()
        .title(condition_title)
        .subtitle(condition_subtitle)
        .active(false)
        .sensitive(false)
        .build();
    expander.add_row(&condition_switch);

    group.add(&expander);

    // Connect enabled switch to enable/disable other fields
    let command_entry_clone = command_entry.clone();
    let timeout_spin_clone = timeout_spin.clone();
    let abort_switch_clone = abort_switch.clone();
    let condition_switch_clone = condition_switch.clone();
    enabled_switch.connect_active_notify(move |switch| {
        let enabled = switch.is_active();
        command_entry_clone.set_sensitive(enabled);
        timeout_spin_clone.set_sensitive(enabled);
        abort_switch_clone.set_sensitive(enabled);
        condition_switch_clone.set_sensitive(enabled);
    });

    (
        group,
        enabled_switch,
        command_entry,
        timeout_spin,
        abort_switch,
        condition_switch,
    )
}
