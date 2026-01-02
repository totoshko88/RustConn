//! Password generator dialog
//!
//! Provides a dialog for generating secure passwords with configurable options.

use gtk4::prelude::*;
use gtk4::{
    Adjustment, Box as GtkBox, Button, CheckButton, Entry, Grid, HeaderBar, Label, LevelBar,
    Orientation, Scale, SpinButton, Window,
};
use rustconn_core::{
    estimate_crack_time, PasswordGenerator, PasswordGeneratorConfig, PasswordStrength,
};
use std::cell::RefCell;
use std::rc::Rc;

/// Shows the password generator dialog
pub fn show_password_generator_dialog(parent: Option<&impl IsA<gtk4::Window>>) {
    let window = Window::builder()
        .title("Password Generator")
        .modal(true)
        .default_width(750)
        .default_height(500)
        .resizable(true)
        .build();

    if let Some(p) = parent {
        window.set_transient_for(Some(p));
    }

    // Header bar
    let header = HeaderBar::new();
    let copy_btn = Button::builder()
        .label("Copy")
        .css_classes(["suggested-action"])
        .build();
    header.pack_start(&copy_btn);
    window.set_titlebar(Some(&header));

    // Content
    let content = GtkBox::new(Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    // Password display
    let password_box = GtkBox::new(Orientation::Horizontal, 6);
    let password_entry = Entry::builder()
        .hexpand(true)
        .editable(false)
        .css_classes(["monospace"])
        .build();
    let generate_btn = Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text("Generate new password")
        .build();
    password_box.append(&password_entry);
    password_box.append(&generate_btn);
    content.append(&password_box);

    // Strength indicator
    let strength_box = GtkBox::new(Orientation::Horizontal, 6);
    let strength_bar = LevelBar::builder()
        .min_value(0.0)
        .max_value(5.0)
        .hexpand(true)
        .build();
    strength_bar.add_offset_value("very-weak", 1.0);
    strength_bar.add_offset_value("weak", 2.0);
    strength_bar.add_offset_value("fair", 3.0);
    strength_bar.add_offset_value("strong", 4.0);
    strength_bar.add_offset_value("very-strong", 5.0);
    let strength_label = Label::builder()
        .label("Strong")
        .width_chars(12)
        .xalign(1.0)
        .build();
    strength_box.append(&strength_bar);
    strength_box.append(&strength_label);
    content.append(&strength_box);

    // Info labels
    let info_box = GtkBox::new(Orientation::Horizontal, 12);
    let entropy_label = Label::builder()
        .label("Entropy: 0 bits")
        .css_classes(["dim-label"])
        .halign(gtk4::Align::Start)
        .hexpand(true)
        .build();
    let crack_time_label = Label::builder()
        .label("Crack time: instant")
        .css_classes(["dim-label"])
        .halign(gtk4::Align::End)
        .build();
    info_box.append(&entropy_label);
    info_box.append(&crack_time_label);
    content.append(&info_box);

    // Options grid
    let options_label = Label::builder()
        .label("Options")
        .halign(gtk4::Align::Start)
        .css_classes(["heading"])
        .margin_top(6)
        .build();
    content.append(&options_label);

    let grid = Grid::builder().row_spacing(6).column_spacing(12).build();

    // Length
    let length_label = Label::builder()
        .label("Length:")
        .halign(gtk4::Align::End)
        .build();
    let length_adj = Adjustment::new(16.0, 4.0, 128.0, 1.0, 4.0, 0.0);
    let length_spin = SpinButton::builder()
        .adjustment(&length_adj)
        .climb_rate(1.0)
        .digits(0)
        .build();
    let length_scale = Scale::builder()
        .adjustment(&length_adj)
        .hexpand(true)
        .draw_value(false)
        .build();
    grid.attach(&length_label, 0, 0, 1, 1);
    grid.attach(&length_spin, 1, 0, 1, 1);
    grid.attach(&length_scale, 2, 0, 1, 1);

    // Character sets
    let lowercase_check = CheckButton::builder()
        .label("Lowercase (a-z)")
        .active(true)
        .build();
    let uppercase_check = CheckButton::builder()
        .label("Uppercase (A-Z)")
        .active(true)
        .build();
    let digits_check = CheckButton::builder()
        .label("Digits (0-9)")
        .active(true)
        .build();
    let special_check = CheckButton::builder()
        .label("Special (!@#$%...)")
        .active(true)
        .build();
    let extended_check = CheckButton::builder()
        .label("Extended (()[]{}...)")
        .active(false)
        .build();
    let ambiguous_check = CheckButton::builder()
        .label("Exclude ambiguous (0O1lI)")
        .active(false)
        .build();

    grid.attach(&lowercase_check, 0, 1, 2, 1);
    grid.attach(&uppercase_check, 2, 1, 1, 1);
    grid.attach(&digits_check, 0, 2, 2, 1);
    grid.attach(&special_check, 2, 2, 1, 1);
    grid.attach(&extended_check, 0, 3, 2, 1);
    grid.attach(&ambiguous_check, 2, 3, 1, 1);

    content.append(&grid);
    window.set_child(Some(&content));

    // State
    let generator = Rc::new(RefCell::new(PasswordGenerator::with_defaults()));

    // Helper to build config from UI state
    let build_config = {
        let length_spin = length_spin.clone();
        let lowercase_check = lowercase_check.clone();
        let uppercase_check = uppercase_check.clone();
        let digits_check = digits_check.clone();
        let special_check = special_check.clone();
        let extended_check = extended_check.clone();
        let ambiguous_check = ambiguous_check.clone();

        move || {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let length = length_spin.value() as usize;

            PasswordGeneratorConfig::new()
                .with_length(length)
                .with_lowercase(lowercase_check.is_active())
                .with_uppercase(uppercase_check.is_active())
                .with_digits(digits_check.is_active())
                .with_special(special_check.is_active())
                .with_extended_special(extended_check.is_active())
                .with_exclude_ambiguous(ambiguous_check.is_active())
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
            let gen = generator.borrow();
            let entropy = gen.calculate_entropy(password);
            let strength = gen.evaluate_strength(password);

            let level = match strength {
                PasswordStrength::VeryWeak => 1.0,
                PasswordStrength::Weak => 2.0,
                PasswordStrength::Fair => 3.0,
                PasswordStrength::Strong => 4.0,
                PasswordStrength::VeryStrong => 5.0,
            };
            strength_bar.set_value(level);
            strength_label.set_text(strength.description());
            entropy_label.set_text(&format!("Entropy: {entropy:.0} bits"));

            let crack_time = estimate_crack_time(entropy, 10_000_000_000.0);
            crack_time_label.set_text(&format!("Crack time: {crack_time}"));
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
                    entropy_label.set_text("Entropy: 0 bits");
                    crack_time_label.set_text("Crack time: N/A");
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

    // Connect checkbox changes
    let connect_checkbox = |check: &CheckButton, generate_fn: Rc<dyn Fn()>| {
        check.connect_toggled(move |_| {
            generate_fn();
        });
    };

    connect_checkbox(&lowercase_check, generate_password.clone());
    connect_checkbox(&uppercase_check, generate_password.clone());
    connect_checkbox(&digits_check, generate_password.clone());
    connect_checkbox(&special_check, generate_password.clone());
    connect_checkbox(&extended_check, generate_password.clone());
    connect_checkbox(&ambiguous_check, generate_password.clone());

    // Generate initial password
    generate_password();

    window.present();
}
