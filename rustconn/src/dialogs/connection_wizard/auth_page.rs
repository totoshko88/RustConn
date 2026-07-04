//! Step 3: Authentication + Color Profile + Finish
//!
//! Shows auth options for protocols that need them, a color profile
//! selector for VTE-based protocols, and Save/Save & Connect buttons.

use crate::i18n::i18n;
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, Orientation, PasswordEntry, ScrolledWindow, StringList};
use libadwaita as adw;
use rustconn_core::models::{ConnectionThemeOverride, ProtocolType, SshAuthMethod};
use rustconn_core::terminal_themes::TerminalTheme;
use secrecy::SecretString;
use std::cell::RefCell;
use std::rc::Rc;

/// Authentication page — Step 3 of the wizard
pub struct AuthPage {
    pub page: adw::NavigationPage,
    // Auth widgets
    auth_group: adw::PreferencesGroup,
    method_row: adw::ComboRow,
    password_entry: PasswordEntry,
    password_row: adw::ActionRow,
    key_file_row: adw::ActionRow,
    key_file_label: gtk4::Label,
    key_file_path: Rc<RefCell<Option<std::path::PathBuf>>>,
    // Color profile widgets
    appearance_group: adw::PreferencesGroup,
    theme_row: adw::ComboRow,
    icon_row: adw::EntryRow,
    // Summary (for no-auth protocols)
    summary_group: adw::PreferencesGroup,
    summary_protocol_row: adw::ActionRow,
    summary_host_row: adw::ActionRow,
    summary_port_row: adw::ActionRow,
    // Callbacks
    on_save: Rc<RefCell<Option<Box<dyn Fn()>>>>,
    on_connect: Rc<RefCell<Option<Box<dyn Fn()>>>>,
    on_advanced: Rc<RefCell<Option<Box<dyn Fn()>>>>,
    // Auth method list — kept in sync with the ComboRow model
    all_methods: Rc<RefCell<Vec<SshAuthMethod>>>,
}

impl AuthPage {
    /// Creates the authentication/finish page
    #[must_use]
    pub fn new() -> Self {
        let on_save: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let on_connect: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let on_advanced: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));

        let content_box = GtkBox::new(Orientation::Vertical, 12);
        content_box.set_margin_top(12);
        content_box.set_margin_bottom(12);
        content_box.set_margin_start(12);
        content_box.set_margin_end(12);

        let clamp = adw::Clamp::builder()
            .maximum_size(520)
            .child(&content_box)
            .build();

        // === Summary group ===
        let summary_group = adw::PreferencesGroup::builder()
            .title(i18n("Summary"))
            .visible(false)
            .build();

        let summary_protocol_row = adw::ActionRow::builder().title(i18n("Protocol")).build();
        summary_group.add(&summary_protocol_row);

        let summary_host_row = adw::ActionRow::builder().title(i18n("Host")).build();
        summary_group.add(&summary_host_row);

        let summary_port_row = adw::ActionRow::builder().title(i18n("Port")).build();
        summary_group.add(&summary_port_row);

        content_box.append(&summary_group);

        // === Auth group ===
        let auth_group = adw::PreferencesGroup::builder()
            .title(i18n("Authentication"))
            .visible(false)
            .build();

        // Auth method selection via ComboRow (fits any width without overflow)
        // Model starts empty — populated by configure_for_protocol() for the
        // correct protocol, avoiding a brief flash of all SSH methods during
        // page transition animations.
        let all_methods: Rc<RefCell<Vec<SshAuthMethod>>> = Rc::new(RefCell::new(Vec::new()));
        let method_model = StringList::new(&[] as &[&str]);
        let method_row = adw::ComboRow::builder()
            .title(i18n("Method"))
            .model(&method_model)
            .visible(false)
            .build();
        auth_group.add(&method_row);

        let password_entry = PasswordEntry::builder()
            .show_peek_icon(true)
            .hexpand(true)
            .valign(gtk4::Align::Center)
            .build();
        let password_row = adw::ActionRow::builder().title(i18n("Password")).build();
        password_row.add_suffix(&password_entry);
        auth_group.add(&password_row);

        // Key file row (shown when Key File method is selected)
        let key_file_label = gtk4::Label::builder()
            .label(i18n("No file selected"))
            .css_classes(["dim-label"])
            .ellipsize(gtk4::pango::EllipsizeMode::Middle)
            .hexpand(true)
            .xalign(0.0)
            .build();
        let key_file_button = Button::from_icon_name("document-open-symbolic");
        key_file_button.set_tooltip_text(Some(&i18n("Choose key file")));
        key_file_button
            .update_property(&[gtk4::accessible::Property::Label(&i18n("Choose key file"))]);
        let key_file_row = adw::ActionRow::builder()
            .title(i18n("Key File"))
            .visible(false)
            .build();
        key_file_row.add_suffix(&key_file_label);
        key_file_row.add_suffix(&key_file_button);
        auth_group.add(&key_file_row);

        content_box.append(&auth_group);

        // === Appearance group ===
        let appearance_group = adw::PreferencesGroup::builder()
            .title(i18n("Appearance"))
            .visible(false)
            .build();

        let mut theme_names: Vec<String> = vec![i18n("Default")];
        theme_names.extend(TerminalTheme::theme_names());
        let theme_refs: Vec<&str> = theme_names.iter().map(String::as_str).collect();
        let theme_model = StringList::new(&theme_refs);
        let theme_row = adw::ComboRow::builder()
            .title(i18n("Terminal Theme"))
            .subtitle(i18n("Visual color scheme for this connection"))
            .model(&theme_model)
            .selected(0)
            .build();
        appearance_group.add(&theme_row);

        let icon_row = adw::EntryRow::builder().title(i18n("Icon")).build();
        icon_row.set_tooltip_text(Some(&i18n("Emoji or icon name (optional)")));
        appearance_group.add(&icon_row);

        content_box.append(&appearance_group);

        // === Footer (sticky bottom bar) ===
        let footer = GtkBox::new(Orientation::Horizontal, 12);
        footer.set_margin_top(6);
        footer.set_margin_bottom(6);
        footer.set_margin_start(12);
        footer.set_margin_end(12);

        let advanced_btn = Button::with_label(&i18n("Advanced…"));
        advanced_btn.add_css_class("flat");
        advanced_btn.add_css_class("dim-label");
        advanced_btn.set_tooltip_text(Some(&i18n("Open full connection editor")));
        advanced_btn.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Open full connection editor",
        ))]);
        footer.append(&advanced_btn);

        let spacer = GtkBox::new(Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        footer.append(&spacer);

        let save_btn = Button::with_label(&i18n("Save"));
        save_btn.set_tooltip_text(Some(&i18n("Save without connecting")));
        save_btn.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Save without connecting",
        ))]);
        footer.append(&save_btn);

        let connect_btn = Button::with_label(&i18n("Save & Connect"));
        connect_btn.add_css_class("suggested-action");
        connect_btn.set_tooltip_text(Some(&i18n("Save and connect immediately")));
        connect_btn.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Save and connect immediately",
        ))]);
        footer.append(&connect_btn);

        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .child(&clamp)
            .vexpand(true)
            .build();

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&adw::HeaderBar::new());
        toolbar_view.set_content(Some(&scrolled));
        toolbar_view.add_bottom_bar(&footer);

        let page = adw::NavigationPage::builder()
            .title(i18n("Finish"))
            .child(&toolbar_view)
            .build();

        // Wire buttons
        let on_save_c = on_save.clone();
        save_btn.connect_clicked(move |_| {
            if let Some(ref cb) = *on_save_c.borrow() {
                cb();
            }
        });

        let on_connect_c = on_connect.clone();
        connect_btn.connect_clicked(move |_| {
            if let Some(ref cb) = *on_connect_c.borrow() {
                cb();
            }
        });

        let on_advanced_c = on_advanced.clone();
        advanced_btn.connect_clicked(move |_| {
            if let Some(ref cb) = *on_advanced_c.borrow() {
                cb();
            }
        });

        // Auth method ComboRow → show/hide password and key file rows
        let pw_row = password_row.clone();
        let kf_row = key_file_row.clone();
        method_row.connect_notify_local(Some("selected"), move |combo, _| {
            let selected = combo.selected();
            // 0 = Password, 1 = Key File, 2 = SSH Agent, 3 = Security Key
            pw_row.set_visible(selected == 0);
            kf_row.set_visible(selected == 1);
        });

        // Wire key file chooser button
        let key_file_path: Rc<RefCell<Option<std::path::PathBuf>>> = Rc::new(RefCell::new(None));
        let kf_path_clone = key_file_path.clone();
        let kf_label_clone = key_file_label.clone();
        key_file_button.connect_clicked(move |btn| {
            let dialog = gtk4::FileDialog::builder()
                .title(i18n("Select SSH Key File"))
                .modal(true)
                .build();
            let root = btn.root().and_then(|r| r.downcast::<gtk4::Window>().ok());
            let path_ref = kf_path_clone.clone();
            let label_ref = kf_label_clone.clone();
            dialog.open(root.as_ref(), gtk4::gio::Cancellable::NONE, move |result| {
                if let Ok(file) = result
                    && let Some(path) = file.path()
                {
                    let display = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| path.to_string_lossy().to_string());
                    label_ref.set_label(&display);
                    label_ref.remove_css_class("dim-label");
                    *path_ref.borrow_mut() = Some(path);
                }
            });
        });

        Self {
            page,
            auth_group,
            method_row,
            password_entry,
            password_row,
            key_file_row,
            key_file_label,
            key_file_path,
            appearance_group,
            theme_row,
            icon_row,
            summary_group,
            summary_protocol_row,
            summary_host_row,
            summary_port_row,
            on_save,
            on_connect,
            on_advanced,
            all_methods,
        }
    }

    /// Configure for a specific protocol
    pub fn configure_for_protocol(&self, protocol: ProtocolType, host: &str, port: u16) {
        self.auth_group.set_visible(false);
        self.appearance_group.set_visible(false);
        self.summary_group.set_visible(false);

        let has_auth = matches!(
            protocol,
            ProtocolType::Ssh
                | ProtocolType::Mosh
                | ProtocolType::Sftp
                | ProtocolType::Rdp
                | ProtocolType::Vnc
                | ProtocolType::Spice
        );

        let is_vte = matches!(
            protocol,
            ProtocolType::Ssh
                | ProtocolType::Mosh
                | ProtocolType::Sftp
                | ProtocolType::Telnet
                | ProtocolType::Serial
                | ProtocolType::Kubernetes
                | ProtocolType::ZeroTrust
        );

        let is_ssh_family = matches!(
            protocol,
            ProtocolType::Ssh | ProtocolType::Mosh | ProtocolType::Sftp
        );

        if has_auth {
            self.auth_group.set_visible(true);
            if is_ssh_family {
                // Show all 4 methods for SSH family
                self.method_row.set_visible(true);
                let methods = vec![
                    SshAuthMethod::Password,
                    SshAuthMethod::PublicKey,
                    SshAuthMethod::Agent,
                    SshAuthMethod::SecurityKey,
                ];
                let method_names: Vec<String> = vec![
                    i18n("Password"),
                    i18n("Key File"),
                    i18n("SSH Agent"),
                    i18n("Security Key (FIDO2)"),
                ];
                let method_refs: Vec<&str> = method_names.iter().map(String::as_str).collect();
                let model = StringList::new(&method_refs);
                self.method_row.set_model(Some(&model));
                self.method_row.set_selected(0);
                *self.all_methods.borrow_mut() = methods;
            } else {
                // Non-SSH: only Password method — hide the method dropdown
                // since there's only one option (no point showing a single-item combo)
                self.method_row.set_visible(false);
                let methods = vec![SshAuthMethod::Password];
                *self.all_methods.borrow_mut() = methods;
                self.password_row.set_visible(true);
                self.key_file_row.set_visible(false);
            }
            // Reset password and key file state when switching protocols
            self.password_entry.set_text("");
            self.key_file_label.set_label(&i18n("No file selected"));
            self.key_file_label.add_css_class("dim-label");
            *self.key_file_path.borrow_mut() = None;
        } else {
            self.summary_group.set_visible(true);
            self.summary_protocol_row
                .set_subtitle(&protocol.to_string());
            self.summary_host_row.set_visible(!host.is_empty());
            if !host.is_empty() {
                self.summary_host_row.set_subtitle(host);
            }
            // Hide port for protocols where it's not meaningful
            let show_port = port > 0
                && !matches!(
                    protocol,
                    ProtocolType::ZeroTrust
                        | ProtocolType::Kubernetes
                        | ProtocolType::Serial
                        | ProtocolType::Web
                );
            self.summary_port_row.set_visible(show_port);
            if show_port {
                self.summary_port_row.set_subtitle(&port.to_string());
            }
        }

        // Always show appearance group (icon is useful for all protocols)
        // Theme row only for VTE-based protocols
        self.appearance_group.set_visible(true);
        self.theme_row.set_visible(is_vte);

        if has_auth {
            self.page.set_title(&i18n("Authentication"));
        } else {
            self.page.set_title(&i18n("Finish"));
        }
    }

    /// Get selected auth method
    #[must_use]
    pub fn auth_method(&self) -> SshAuthMethod {
        let selected = self.method_row.selected() as usize;
        let methods = self.all_methods.borrow();
        if let Some(method) = methods.get(selected) {
            method.clone()
        } else {
            tracing::warn!(
                selected_index = selected,
                methods_count = methods.len(),
                "Auth method ComboRow selected index out of bounds, defaulting to Password"
            );
            SshAuthMethod::Password
        }
    }

    /// Get password if entered
    #[must_use]
    pub fn password(&self) -> Option<SecretString> {
        let text = self.password_entry.text().to_string();
        if text.is_empty() {
            None
        } else {
            Some(SecretString::new(text.into()))
        }
    }

    /// Get theme override (None = use default)
    #[must_use]
    pub fn theme_override(&self) -> Option<ConnectionThemeOverride> {
        let selected = self.theme_row.selected();
        if selected == 0 {
            None
        } else {
            let themes = TerminalTheme::all_themes();
            themes
                .get(selected as usize - 1)
                .map(|theme| ConnectionThemeOverride {
                    background: Some(theme.background.to_hex()),
                    foreground: Some(theme.foreground.to_hex()),
                    cursor: Some(theme.cursor.to_hex()),
                })
        }
    }

    pub fn connect_save<F: Fn() + 'static>(&self, f: F) {
        *self.on_save.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_save_and_connect<F: Fn() + 'static>(&self, f: F) {
        *self.on_connect.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_advanced<F: Fn() + 'static>(&self, f: F) {
        *self.on_advanced.borrow_mut() = Some(Box::new(f));
    }

    /// Get selected key file path (if Key File auth method is active)
    #[must_use]
    pub fn key_path(&self) -> Option<std::path::PathBuf> {
        self.key_file_path.borrow().clone()
    }

    /// Get custom icon (emoji or icon name) if entered
    #[must_use]
    pub fn icon(&self) -> Option<String> {
        let text = self.icon_row.text().trim().to_string();
        if text.is_empty() { None } else { Some(text) }
    }
}
