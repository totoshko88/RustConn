//! Password generator dialog
//!
//! Provides a dialog for generating secure passwords with configurable options.
//! Migrated to use libadwaita components for GNOME HIG compliance.

use crate::i18n::{i18n, i18n_f};
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Adjustment, Box as GtkBox, Button, Entry, Label, LevelBar, Orientation, Scale, SpinButton,
};
use libadwaita as adw;
use rustconn_core::password_generator::{
    PasswordGenerator, PasswordGeneratorConfig, PasswordStrength, estimate_crack_time,
};
use std::cell::RefCell;
use std::rc::Rc;

/// Shows the password generator dialog
pub fn show_password_generator_dialog(parent: Option<&impl IsA<gtk4::Window>>) {
    let window = adw::Window::builder()
        .title(i18n("Password Generator"))
        .modal(true)
        .default_width(600)
        .default_height(500)
        .resizable(true)
        .build();

    if let Some(p) = parent {
        window.set_transient_for(Some(p));
    }

    window.set_size_request(350, 300);

    // Header bar (GNOME HIG)
    let (header, close_btn, copy_btn) = crate::dialogs::widgets::dialog_header("Close", "Copy");

    // Close button handler
    let window_clone = window.clone();
    close_btn.connect_clicked(move |_| {
        window_clone.close();
    });

    // Scrollable content with clamp
    let scrolled = gtk4::ScrolledWindow::builder()
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

    clamp.set_child(Some(&content));
    scrolled.set_child(Some(&clamp));

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&scrolled));
    window.set_content(Some(&toolbar_view));

    // === Password Display Group ===
    let password_group = adw::PreferencesGroup::builder()
        .title(i18n("Generated Password"))
        .build();

    let password_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .hexpand(true)
        .build();
    let password_entry = Entry::builder()
        .hexpand(true)
        .editable(false)
        .css_classes(["monospace"])
        .build();
    let generate_btn = Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text(i18n("Generate new password"))
        .valign(gtk4::Align::Center)
        .build();
    password_box.append(&password_entry);
    password_box.append(&generate_btn);
    password_group.add(&password_box);

    content.append(&password_group);

    // === Strength Indicator Group ===
    let strength_group = adw::PreferencesGroup::builder()
        .title(i18n("Strength Analysis"))
        .build();

    // Strength bar row
    let strength_bar = LevelBar::builder()
        .min_value(0.0)
        .max_value(5.0)
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .build();
    strength_bar.add_offset_value("very-weak", 1.0);
    strength_bar.add_offset_value("weak", 2.0);
    strength_bar.add_offset_value("fair", 3.0);
    strength_bar.add_offset_value("strong", 4.0);
    strength_bar.add_offset_value("very-strong", 5.0);

    let strength_label = Label::builder()
        .label(i18n("Strong"))
        .width_chars(12)
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        .build();

    let strength_row = adw::ActionRow::builder().title(i18n("Strength")).build();
    strength_row.add_suffix(&strength_bar);
    strength_row.add_suffix(&strength_label);
    strength_group.add(&strength_row);

    // Entropy row
    let entropy_label = Label::builder()
        .label(i18n("0 bits"))
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        .css_classes(["dim-label"])
        .build();
    let entropy_row = adw::ActionRow::builder()
        .title(i18n("Entropy"))
        .subtitle(i18n("Measure of randomness"))
        .build();
    entropy_row.add_suffix(&entropy_label);
    strength_group.add(&entropy_row);

    // Crack time row
    let crack_time_label = Label::builder()
        .label(i18n("instant"))
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        .css_classes(["dim-label"])
        .build();
    let crack_time_row = adw::ActionRow::builder()
        .title(i18n("Crack time"))
        .subtitle(i18n("At 10 billion guesses/sec"))
        .build();
    crack_time_row.add_suffix(&crack_time_label);
    strength_group.add(&crack_time_row);

    content.append(&strength_group);

    // === Length Group ===
    let length_group = adw::PreferencesGroup::builder()
        .title(i18n("Length"))
        .build();

    let length_adj = Adjustment::new(16.0, 4.0, 128.0, 1.0, 4.0, 0.0);
    let length_spin = SpinButton::builder()
        .adjustment(&length_adj)
        .climb_rate(1.0)
        .digits(0)
        .valign(gtk4::Align::Center)
        .build();
    let length_scale = Scale::builder()
        .adjustment(&length_adj)
        .hexpand(true)
        .draw_value(false)
        .valign(gtk4::Align::Center)
        .build();

    let length_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .hexpand(true)
        .build();
    length_box.append(&length_scale);
    length_box.append(&length_spin);

    let length_row = adw::ActionRow::builder()
        .title(i18n("Characters"))
        .subtitle(i18n("Recommended: 16+ for important accounts"))
        .build();
    length_row.add_suffix(&length_box);
    length_group.add(&length_row);

    content.append(&length_group);

    // === Character Sets Group ===
    let charset_group = adw::PreferencesGroup::builder()
        .title(i18n("Character Sets"))
        .description(i18n("Select which characters to include"))
        .build();

    // Lowercase
    let lowercase_row = adw::SwitchRow::builder()
        .title(i18n("Lowercase"))
        .subtitle("a-z")
        .active(true)
        .build();
    charset_group.add(&lowercase_row);

    // Uppercase
    let uppercase_row = adw::SwitchRow::builder()
        .title(i18n("Uppercase"))
        .subtitle("A-Z")
        .active(true)
        .build();
    charset_group.add(&uppercase_row);

    // Digits
    let digits_row = adw::SwitchRow::builder()
        .title(i18n("Digits"))
        .subtitle("0-9")
        .active(true)
        .build();
    charset_group.add(&digits_row);

    // Special
    let special_row = adw::SwitchRow::builder()
        .title(i18n("Special"))
        .subtitle("!@#$%^&amp;*")
        .active(true)
        .build();
    charset_group.add(&special_row);

    // Extended special
    let extended_row = adw::SwitchRow::builder()
        .title(i18n("Extended"))
        .subtitle("()[]{}|;:,.&lt;&gt;?/")
        .active(false)
        .build();
    charset_group.add(&extended_row);

    content.append(&charset_group);

    // === Options Group ===
    let options_group = adw::PreferencesGroup::builder()
        .title(i18n("Options"))
        .build();

    // Exclude ambiguous
    let ambiguous_row = adw::SwitchRow::builder()
        .title(i18n("Exclude ambiguous"))
        .subtitle(i18n("Avoid 0O, 1lI to prevent confusion"))
        .active(false)
        .build();
    options_group.add(&ambiguous_row);

    content.append(&options_group);

    // === Security Tips Group ===
    let tips_group = adw::PreferencesGroup::builder()
        .title(i18n("Security Tips"))
        .build();

    let tips = [
        (
            "Use 16+ characters",
            "For critical accounts like banking, email",
        ),
        (
            "Never reuse passwords",
            "Each service should have unique password",
        ),
        ("Use password manager", "Don't store in plain text files"),
        ("Enable 2FA", "Add extra layer of security when available"),
    ];

    for (title, subtitle) in tips {
        let tip_row = adw::ActionRow::builder()
            .title(title)
            .subtitle(subtitle)
            .build();

        let icon = gtk4::Image::from_icon_name("emblem-ok-symbolic");
        icon.set_valign(gtk4::Align::Center);
        icon.add_css_class("success");
        tip_row.add_prefix(&icon);

        tips_group.add(&tip_row);
    }

    content.append(&tips_group);

    // State
    let generator = Rc::new(RefCell::new(PasswordGenerator::with_defaults()));

    // Helper to build config from UI state
    let build_config = {
        let length_spin = length_spin.clone();
        let lowercase_row = lowercase_row.clone();
        let uppercase_row = uppercase_row.clone();
        let digits_row = digits_row.clone();
        let special_row = special_row.clone();
        let extended_row = extended_row.clone();
        let ambiguous_row = ambiguous_row.clone();

        move || {
            #[allow(clippy::cast_sign_loss)]
            let length = length_spin.value() as usize;

            PasswordGeneratorConfig::new()
                .with_length(length)
                .with_lowercase(lowercase_row.is_active())
                .with_uppercase(uppercase_row.is_active())
                .with_digits(digits_row.is_active())
                .with_special(special_row.is_active())
                .with_extended_special(extended_row.is_active())
                .with_exclude_ambiguous(ambiguous_row.is_active())
        }
    };

    // Helper to update strength display
    let update_display = {
        let strength_bar = strength_bar.clone();
        let strength_label = strength_label.clone();
        let entropy_label = entropy_label.clone();
        let crack_time_label = crack_time_label.clone();
        let generator = generator.clone();

        Rc::new(move |password: &str| {
            let pw_gen = generator.borrow();
            let entropy = pw_gen.calculate_entropy(password);
            let strength = pw_gen.evaluate_strength(password);

            let level = match strength {
                PasswordStrength::VeryWeak => 1.0,
                PasswordStrength::Weak => 2.0,
                PasswordStrength::Fair => 3.0,
                PasswordStrength::Strong => 4.0,
                PasswordStrength::VeryStrong => 5.0,
            };
            strength_bar.set_value(level);
            strength_label.set_text(&i18n(strength.description()));
            entropy_label.set_text(&i18n_f("{} bits", &[&format!("{entropy:.0}")]));

            let crack_time = estimate_crack_time(entropy, 10_000_000_000.0);
            crack_time_label.set_text(&i18n(&crack_time));
        })
    };

    // Helper to generate password
    let generate_password = {
        let password_entry = password_entry.clone();
        let strength_label = strength_label.clone();
        let strength_bar = strength_bar.clone();
        let entropy_label = entropy_label.clone();
        let crack_time_label = crack_time_label.clone();
        let generator = generator.clone();
        let build_config = build_config.clone();
        let update_display = update_display.clone();

        Rc::new(move || {
            let config = build_config();
            generator.borrow_mut().set_config(config);

            match generator.borrow().generate() {
                Ok(password) => {
                    password_entry.set_text(&password);
                    update_display(&password);
                }
                Err(e) => {
                    password_entry.set_text("");
                    strength_label.set_text(&e.to_string());
                    strength_bar.set_value(0.0);
                    entropy_label.set_text(&i18n("0 bits"));
                    crack_time_label.set_text(&i18n("N/A"));
                }
            }
        })
    };

    // Connect signals
    let password_entry_clone = password_entry.clone();
    let window_clone = window.clone();
    copy_btn.connect_clicked(move |_| {
        let text = password_entry_clone.text().to_string();
        if !text.is_empty() {
            let display = gtk4::prelude::WidgetExt::display(&window_clone);
            display.clipboard().set_text(&text);
        }
    });

    let generate_password_clone = generate_password.clone();
    generate_btn.connect_clicked(move |_| {
        generate_password_clone();
    });

    let generate_password_clone = generate_password.clone();
    length_adj.connect_value_changed(move |_| {
        generate_password_clone();
    });

    // Connect switch changes - SwitchRow uses notify::active signal
    let connect_switch_row = |row: &adw::SwitchRow, generate_fn: Rc<dyn Fn()>| {
        row.connect_active_notify(move |_| {
            generate_fn();
        });
    };

    connect_switch_row(&lowercase_row, generate_password.clone());
    connect_switch_row(&uppercase_row, generate_password.clone());
    connect_switch_row(&digits_row, generate_password.clone());
    connect_switch_row(&special_row, generate_password.clone());
    connect_switch_row(&extended_row, generate_password.clone());
    connect_switch_row(&ambiguous_row, generate_password.clone());

    // Generate initial password
    generate_password();

    window.present();
}
