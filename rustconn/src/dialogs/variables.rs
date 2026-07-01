//! Variables dialog for managing global and local variables
//!
//! Provides a GTK4 dialog for creating, editing, and deleting variables
//! with support for secret variable masking.
//!
//! Uses `adw::Dialog` for GNOME HIG compliance: bottom-sheet on narrow screens,
//! auto-close on Escape, drag-to-close support.

use crate::i18n::i18n;
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, CheckButton, Entry, Grid, Label, ListBox, ListBoxRow, Orientation,
    ScrolledWindow,
};
use libadwaita as adw;
use rustconn_core::config::AppSettings;
use rustconn_core::variables::Variable;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use super::VariablesCallback;

/// Shared settings reference for variable rows
type SharedSettings = Rc<RefCell<Option<AppSettings>>>;

/// Variables dialog for managing global variables
pub struct VariablesDialog {
    dialog: adw::Dialog,
    variables_list: ListBox,
    add_header_btn: Button,
    variables: Rc<RefCell<Vec<VariableRow>>>,
    on_save: VariablesCallback,
    settings: SharedSettings,
    parent: Option<gtk4::Widget>,
}

/// Represents a variable row in the dialog
struct VariableRow {
    /// The row widget
    row: ListBoxRow,
    /// The expander that collapses/expands the row content
    expander: gtk4::Expander,
    /// Entry for variable name
    name_entry: Entry,
    /// Entry for variable value (regular, visible)
    value_entry: Entry,
    /// Entry for secret value (hidden text, with show/hide toggle)
    secret_entry: Entry,
    /// Checkbox for secret flag
    is_secret_check: CheckButton,
    /// Entry for description
    description_entry: Entry,
    /// Entry for custom KeePass entry path
    kdbx_path_entry: Entry,
    /// Entry for custom vault entry name (Bitwarden, 1Password, etc.)
    vault_name_entry: Entry,
    /// Delete button
    delete_button: Button,
}

impl VariablesDialog {
    /// Creates a new variables dialog for global variables
    #[must_use]
    pub fn new(parent: Option<&gtk4::Window>) -> Self {
        let dialog = adw::Dialog::builder()
            .title(i18n("Global Variables"))
            .content_width(600)
            .content_height(580)
            .build();

        // Header bar with Add button (GNOME HIG)
        let header = adw::HeaderBar::new();
        let add_header_btn = Button::from_icon_name("list-add-symbolic");
        add_header_btn.set_tooltip_text(Some(&i18n("Add Variable")));
        add_header_btn.update_property(&[gtk4::accessible::Property::Label(&i18n("Add Variable"))]);
        header.pack_start(&add_header_btn);

        // Create main content area with clamp
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

        // Use ToolbarView for adw::Dialog
        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&clamp));
        dialog.set_child(Some(&toolbar_view));

        // Variables list in PreferencesGroup
        let (group, variables_list) = Self::create_variables_section();
        content.append(&group);

        let on_save: VariablesCallback = Rc::new(RefCell::new(None));
        let variables: Rc<RefCell<Vec<VariableRow>>> = Rc::new(RefCell::new(Vec::new()));
        let settings: SharedSettings = Rc::new(RefCell::new(None));

        // Connect dialog closed to cancel callback
        let on_save_clone = on_save.clone();
        dialog.connect_closed(move |_| {
            if let Some(ref cb) = *on_save_clone.borrow() {
                cb(None);
            }
        });

        // Save button at bottom of content
        let save_box = GtkBox::new(Orientation::Horizontal, 8);
        save_box.set_halign(gtk4::Align::End);
        save_box.set_margin_top(12);
        let save_btn = Button::builder()
            .label(i18n("Save"))
            .css_classes(["suggested-action"])
            .build();
        save_box.append(&save_btn);
        content.append(&save_box);

        // Connect save button
        let dialog_clone = dialog.clone();
        let on_save_clone = on_save.clone();
        let variables_clone = variables.clone();
        save_btn.connect_clicked(move |_| {
            if Self::validate_duplicates(&variables_clone) {
                let vars = Self::collect_variables(&variables_clone);
                if let Some(ref cb) = *on_save_clone.borrow() {
                    cb(Some(vars));
                }
                dialog_clone.close();
            }
        });

        let parent_widget = parent.map(|p| p.clone().upcast::<gtk4::Widget>());

        Self {
            dialog,
            variables_list,
            add_header_btn,
            variables,
            on_save,
            settings,
            parent: parent_widget,
        }
    }

    /// Creates the variables section with list and add button
    fn create_variables_section() -> (adw::PreferencesGroup, ListBox) {
        let group = adw::PreferencesGroup::builder()
            .title(i18n("Variables"))
            .description(i18n(
                "Define variables that can be used in connections \
                 with ${variable_name} syntax",
            ))
            .build();

        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .min_content_height(300)
            .vexpand(true)
            .build();

        let variables_list = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        scrolled.set_child(Some(&variables_list));

        group.add(&scrolled);

        (group, variables_list)
    }

    /// Creates a variable row widget with collapsible expander
    fn create_variable_row(
        variable: Option<&Variable>,
        settings: &SharedSettings,
        expanded: bool,
    ) -> VariableRow {
        let main_box = GtkBox::new(Orientation::Vertical, 0);
        main_box.set_margin_top(6);
        main_box.set_margin_bottom(6);
        main_box.set_margin_start(12);
        main_box.set_margin_end(12);

        // Header row: expander label + delete button
        let header_box = GtkBox::new(Orientation::Horizontal, 8);
        header_box.set_margin_bottom(6);

        let delete_button = Button::builder()
            .icon_name("user-trash-symbolic")
            .css_classes(["destructive-action", "flat"])
            .tooltip_text(i18n("Delete variable"))
            .valign(gtk4::Align::Center)
            .build();
        delete_button
            .update_property(&[gtk4::accessible::Property::Label(&i18n("Delete variable"))]);

        // Content grid (shown when expanded)
        let grid = Grid::builder()
            .row_spacing(6)
            .column_spacing(8)
            .hexpand(true)
            .build();

        // Row 0: Name
        let name_label = Label::builder()
            .label(i18n("Name:"))
            .halign(gtk4::Align::End)
            .build();
        let name_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text(i18n("variable_name"))
            .build();

        grid.attach(&name_label, 0, 0, 1, 1);
        grid.attach(&name_entry, 1, 0, 1, 1);

        // Row 1: Value — single row, switches between plain and secret mode
        let value_label = Label::builder()
            .label(i18n("Value:"))
            .halign(gtk4::Align::End)
            .build();
        // Plain value entry (visible when not secret)
        let value_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text(i18n("Variable value"))
            .build();
        // Secret value entry (visible when secret, with masked input)
        let secret_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text(i18n("Password value"))
            .visibility(false)
            .build();
        // Show/Hide toggle button (secret mode only)
        let show_hide_btn = Button::builder()
            .icon_name("view-reveal-symbolic")
            .tooltip_text(i18n("Show/hide password"))
            .build();
        show_hide_btn.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Toggle password visibility",
        ))]);
        // Load from Vault button (secret mode only)
        let load_vault_btn = Button::builder()
            .icon_name("document-open-symbolic")
            .tooltip_text(i18n("Load password from vault"))
            .build();
        load_vault_btn.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Load password from vault",
        ))]);
        // Secret row: entry + show/hide + load buttons
        let secret_buttons_box = GtkBox::new(Orientation::Horizontal, 4);
        secret_buttons_box.append(&secret_entry);
        secret_buttons_box.append(&show_hide_btn);
        secret_buttons_box.append(&load_vault_btn);
        secret_buttons_box.set_hexpand(true);
        secret_buttons_box.set_visible(false);

        grid.attach(&value_label, 0, 1, 1, 1);
        grid.attach(&value_entry, 1, 1, 1, 1);
        grid.attach(&secret_buttons_box, 1, 1, 1, 1);

        // Row 2: Is Secret checkbox
        let is_secret_check = CheckButton::builder()
            .label(i18n("Secret (mask value)"))
            .build();

        grid.attach(&is_secret_check, 1, 2, 1, 1);

        // Row 3: Description
        let desc_label = Label::builder()
            .label(i18n("Description:"))
            .halign(gtk4::Align::End)
            .build();
        let description_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text(i18n("Optional description"))
            .build();

        grid.attach(&desc_label, 0, 3, 1, 1);
        grid.attach(&description_entry, 1, 3, 1, 1);

        // Row 4: KeePass entry path (visible only for secret variables with KeePass backend)
        let kdbx_path_label = Label::builder()
            .label(i18n("KeePass entry:"))
            .halign(gtk4::Align::End)
            .build();
        let kdbx_path_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text(i18n("e.g. Internet/MyRouter (optional)"))
            .tooltip_text(i18n(
                "Custom KeePass entry path. If set, the password is read from this \
                 existing entry instead of the default RustConn/rustconn/var/ path.",
            ))
            .build();
        kdbx_path_label.set_visible(false);
        kdbx_path_entry.set_visible(false);

        grid.attach(&kdbx_path_label, 0, 4, 1, 1);
        grid.attach(&kdbx_path_entry, 1, 4, 1, 1);

        // Row 5: Vault entry name (visible for secret variables with non-KeePass backends)
        let vault_name_label = Label::builder()
            .label(i18n("Vault entry:"))
            .halign(gtk4::Align::End)
            .build();
        let vault_name_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text(i18n("e.g. AD Credentials (optional)"))
            .tooltip_text(i18n(
                "Existing vault entry name. If set, the password is fetched from this \
                 entry at connection time — no need to enter a password above. \
                 Nothing is written back to the vault.",
            ))
            .build();
        vault_name_label.set_visible(false);
        vault_name_entry.set_visible(false);

        grid.attach(&vault_name_label, 0, 5, 1, 1);
        grid.attach(&vault_name_entry, 1, 5, 1, 1);

        // Build expander with custom label widget showing name + value preview
        let label_box = GtkBox::new(Orientation::Horizontal, 8);
        label_box.set_hexpand(true);

        let name_label_widget = Label::builder()
            .label(Self::build_expander_name(variable))
            .css_classes(["heading"])
            .halign(gtk4::Align::Start)
            .build();

        let value_preview_widget = Label::builder()
            .label(Self::build_expander_value_preview(variable))
            .css_classes(["dim-label"])
            .halign(gtk4::Align::Start)
            .hexpand(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();

        label_box.append(&name_label_widget);
        label_box.append(&value_preview_widget);

        let row_expander = gtk4::Expander::builder()
            .label_widget(&label_box)
            .expanded(expanded)
            .hexpand(true)
            .build();
        row_expander.set_child(Some(&grid));

        header_box.append(&row_expander);
        header_box.append(&delete_button);
        main_box.append(&header_box);

        // Update expander label when name or value changes
        let name_label_for_update = name_label_widget.clone();
        name_entry.connect_changed(move |entry| {
            let name = entry.text().to_string();
            let label = if name.trim().is_empty() {
                i18n("New variable")
            } else {
                name
            };
            name_label_for_update.set_label(&label);
        });

        // Update value preview when value entry changes
        let value_preview_for_plain = value_preview_widget.clone();
        value_entry.connect_changed(move |entry| {
            let val = entry.text().to_string();
            let preview = if val.is_empty() {
                String::new()
            } else {
                format!("= {val}")
            };
            value_preview_for_plain.set_label(&preview);
        });

        // Update value preview when secret checkbox toggles
        let value_preview_for_secret = value_preview_widget.clone();
        let value_entry_for_preview = value_entry.clone();
        is_secret_check.connect_toggled(move |check| {
            if check.is_active() {
                value_preview_for_secret.set_label("= ••••••");
            } else {
                let val = value_entry_for_preview.text().to_string();
                let preview = if val.is_empty() {
                    String::new()
                } else {
                    format!("= {val}")
                };
                value_preview_for_secret.set_label(&preview);
            }
        });

        // Wire Show/Hide toggle — track visibility state in Rc
        let secret_visible = Rc::new(RefCell::new(false));
        let secret_entry_for_toggle = secret_entry.clone();
        let show_hide_btn_clone = show_hide_btn.clone();
        let vis_state = secret_visible.clone();
        show_hide_btn.connect_clicked(move |_| {
            let mut is_vis = vis_state.borrow_mut();
            *is_vis = !*is_vis;
            secret_entry_for_toggle.set_visibility(*is_vis);
            if *is_vis {
                show_hide_btn_clone.set_icon_name("view-conceal-symbolic");
            } else {
                show_hide_btn_clone.set_icon_name("view-reveal-symbolic");
            }
        });

        // Wire Load from Vault button
        let secret_entry_for_load = secret_entry.clone();
        let name_entry_for_load = name_entry.clone();
        let kdbx_path_entry_for_load = kdbx_path_entry.clone();
        let vault_name_entry_for_load = vault_name_entry.clone();
        let settings_for_load = settings.clone();
        load_vault_btn.connect_clicked(move |btn| {
            let var_name = name_entry_for_load.text().to_string();
            if var_name.trim().is_empty() {
                return;
            }
            let entry_clone = secret_entry_for_load.clone();
            let btn_clone = btn.clone();
            let settings_snap = settings_for_load.borrow().clone();
            let custom_path = {
                let text = kdbx_path_entry_for_load.text();
                let trimmed = text.trim().to_string();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            };
            let vault_entry = {
                let text = vault_name_entry_for_load.text();
                let trimmed = text.trim().to_string();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            };

            btn.set_sensitive(false);
            btn.set_icon_name("content-loading-symbolic");

            crate::utils::spawn_blocking_with_callback(
                move || {
                    if let Some(ref s) = settings_snap {
                        crate::state::load_variable_from_vault_with_path(
                            &s.secrets,
                            &var_name,
                            custom_path.as_deref(),
                            vault_entry.as_deref(),
                        )
                    } else {
                        // No settings — fall back to the system keyring
                        let lookup_key = rustconn_core::variable_secret_key(&var_name);
                        // macOS uses the Keychain; LibSecretBackend (oo7) is not
                        // compiled there (R10.1, R10.2).
                        #[cfg(target_os = "macos")]
                        let backend = rustconn_core::secret::MacOsKeychainBackend::new();
                        #[cfg(not(target_os = "macos"))]
                        let backend = rustconn_core::secret::LibSecretBackend::new("rustconn");
                        crate::async_utils::with_runtime(|rt| {
                            let creds = rt.block_on(async {
                                tokio::time::timeout(
                                    std::time::Duration::from_secs(10),
                                    rustconn_core::secret::SecretBackend::retrieve(
                                        &backend,
                                        &lookup_key,
                                    ),
                                )
                                .await
                                .map_err(|_| "Vault retrieve timed out".to_string())?
                                .map_err(|e| format!("{e}"))
                            })?;
                            Ok(creds.and_then(|c| c.expose_password().map(String::from)))
                        })?
                    }
                },
                move |result: Result<Option<String>, String>| {
                    btn_clone.set_sensitive(true);
                    btn_clone.set_icon_name("document-open-symbolic");
                    match result {
                        Ok(Some(pwd)) => {
                            entry_clone.set_text(&pwd);
                        }
                        Ok(None) => {
                            tracing::warn!(
                                "No secret found in vault \
                                 for variable"
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                "Failed to load secret \
                                 from vault: {e}"
                            );
                            if btn_clone.root().is_some() {
                                crate::alert::show_error(
                                    &btn_clone,
                                    &i18n("Could not load secret"),
                                    &i18n(
                                        "Failed to load secret. Check secret backend in Settings.",
                                    ),
                                );
                            } else {
                                tracing::warn!(
                                    "Load vault button is not \
                                     in a window hierarchy"
                                );
                            }
                        }
                    }
                },
            );
        });

        // Connect is_secret checkbox to toggle value/secret visibility
        let value_entry_clone = value_entry.clone();
        let secret_entry_clone = secret_entry.clone();
        let secret_buttons_clone = secret_buttons_box.clone();
        let kdbx_path_label_clone = kdbx_path_label.clone();
        let kdbx_path_entry_clone = kdbx_path_entry.clone();
        let vault_name_label_clone = vault_name_label.clone();
        let vault_name_entry_clone = vault_name_entry.clone();
        let settings_for_toggle = settings.clone();
        is_secret_check.connect_toggled(move |check| {
            let is_secret = check.is_active();
            value_entry_clone.set_visible(!is_secret);
            secret_buttons_clone.set_visible(is_secret);

            // Show KeePass entry path field only when secret AND backend is KeePass
            let show_kdbx = is_secret
                && settings_for_toggle.borrow().as_ref().is_some_and(|s| {
                    s.secrets.kdbx_enabled
                        && matches!(
                            s.secrets.preferred_backend,
                            rustconn_core::config::SecretBackendType::KeePassXc
                                | rustconn_core::config::SecretBackendType::KdbxFile
                        )
                });
            kdbx_path_label_clone.set_visible(show_kdbx);
            kdbx_path_entry_clone.set_visible(show_kdbx);

            // Show vault entry name field when secret AND backend is NOT KeePass
            let show_vault_name = is_secret
                && settings_for_toggle.borrow().as_ref().is_some_and(|s| {
                    matches!(
                        s.secrets.preferred_backend,
                        rustconn_core::config::SecretBackendType::Bitwarden
                            | rustconn_core::config::SecretBackendType::OnePassword
                            | rustconn_core::config::SecretBackendType::Passbolt
                            | rustconn_core::config::SecretBackendType::Pass
                    )
                });
            vault_name_label_clone.set_visible(show_vault_name);
            vault_name_entry_clone.set_visible(show_vault_name);

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
            if let Some(ref kdbx_path) = var.kdbx_entry_path {
                kdbx_path_entry.set_text(kdbx_path);
            }
            if let Some(ref vault_name) = var.vault_entry_name {
                vault_name_entry.set_text(vault_name);
            }
        }

        // Update password placeholder when vault entry name changes — helps
        // the user understand that the password will be fetched from the vault
        // automatically and no manual entry is needed.
        let secret_entry_for_hint = secret_entry.clone();
        vault_name_entry.connect_changed(move |entry| {
            let text = entry.text();
            if text.trim().is_empty() {
                secret_entry_for_hint.set_placeholder_text(Some(&i18n("Password value")));
            } else {
                secret_entry_for_hint
                    .set_placeholder_text(Some(&i18n("Fetched from vault at connect time")));
            }
        });

        let row = ListBoxRow::builder().child(&main_box).build();

        // Show description as tooltip on the row (visible on hover)
        if let Some(var) = variable
            && let Some(ref desc) = var.description
            && !desc.trim().is_empty()
        {
            row.set_tooltip_text(Some(desc));
        }

        // Update tooltip when description changes
        let row_for_tooltip = row.clone();
        description_entry.connect_changed(move |entry| {
            let desc = entry.text().to_string();
            if desc.trim().is_empty() {
                row_for_tooltip.set_tooltip_text(None);
            } else {
                row_for_tooltip.set_tooltip_text(Some(&desc));
            }
        });

        VariableRow {
            row,
            expander: row_expander,
            name_entry,
            value_entry,
            secret_entry,
            is_secret_check,
            description_entry,
            kdbx_path_entry,
            vault_name_entry,
            delete_button,
        }
    }

    /// Builds the expander name label from a variable
    fn build_expander_name(variable: Option<&Variable>) -> String {
        match variable {
            Some(var) if !var.name.trim().is_empty() => var.name.clone(),
            _ => i18n("New variable"),
        }
    }

    /// Builds the value preview for the expander (shown in collapsed state)
    fn build_expander_value_preview(variable: Option<&Variable>) -> String {
        match variable {
            Some(var) if var.is_secret => "= ••••••".to_string(),
            Some(var) if !var.value.is_empty() => format!("= {}", var.value),
            _ => String::new(),
        }
    }

    /// Validates that there are no duplicate variable names.
    /// Returns `true` if validation passes (no duplicates).
    /// Highlights duplicate name entries with error styling.
    fn validate_duplicates(variables: &Rc<RefCell<Vec<VariableRow>>>) -> bool {
        let vars = variables.borrow();
        let mut seen: HashSet<String> = HashSet::new();
        let mut duplicates: HashSet<String> = HashSet::new();

        // First pass: find duplicate names
        for row in vars.iter() {
            let name = row.name_entry.text().trim().to_string();
            if name.is_empty() {
                continue;
            }
            let lower = name.to_lowercase();
            if !seen.insert(lower.clone()) {
                duplicates.insert(lower);
            }
        }

        // Second pass: apply/remove error styling
        for row in vars.iter() {
            let name = row.name_entry.text().trim().to_string();
            let lower = name.to_lowercase();
            if !name.is_empty() && duplicates.contains(&lower) {
                row.name_entry.add_css_class("error");
                // Expand the row so user can see the error
                row.expander.set_expanded(true);
            } else {
                row.name_entry.remove_css_class("error");
            }
        }

        duplicates.is_empty()
    }

    /// Collects all variables from the dialog
    fn collect_variables(variables: &Rc<RefCell<Vec<VariableRow>>>) -> Vec<Variable> {
        let vars = variables.borrow();
        vars.iter()
            .filter_map(|row| {
                let name = row.name_entry.text().trim().to_string();
                if name.is_empty() {
                    return None;
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

                let kdbx_path = row.kdbx_path_entry.text();
                let kdbx_entry_path = if kdbx_path.trim().is_empty() {
                    None
                } else {
                    Some(kdbx_path.trim().to_string())
                };

                let vault_name = row.vault_name_entry.text();
                let vault_entry_name = if vault_name.trim().is_empty() {
                    None
                } else {
                    Some(vault_name.trim().to_string())
                };

                let mut var = Variable::new(name, value);
                var.is_secret = is_secret;
                var.description = description;
                var.kdbx_entry_path = kdbx_entry_path;
                var.vault_entry_name = vault_entry_name;
                Some(var)
            })
            .collect()
    }

    /// Sets the application settings for vault backend selection
    pub fn set_settings(&self, settings: &AppSettings) {
        *self.settings.borrow_mut() = Some(settings.clone());
    }

    /// Sets the initial variables to display (all collapsed)
    pub fn set_variables(&self, variables: &[Variable]) {
        // Clear existing rows
        while let Some(row) = self.variables_list.row_at_index(0) {
            self.variables_list.remove(&row);
        }
        self.variables.borrow_mut().clear();

        // Add rows for each variable (collapsed)
        for var in variables {
            self.add_variable_row(Some(var), false);
        }
    }

    /// Adds a new variable row to the list
    fn add_variable_row(&self, variable: Option<&Variable>, expanded: bool) {
        let var_row = Self::create_variable_row(variable, &self.settings, expanded);

        // Connect delete button
        let variables_list = self.variables_list.clone();
        let variables = self.variables.clone();
        let row_widget = var_row.row.clone();
        var_row.delete_button.connect_clicked(move |_| {
            // Remove from list widget
            variables_list.remove(&row_widget);

            // Remove from variables vec
            let mut vars = variables.borrow_mut();
            vars.retain(|r| r.row != row_widget);
        });

        self.variables_list.append(&var_row.row);
        self.variables.borrow_mut().push(var_row);
    }

    /// Wires up the add button in the header bar
    fn wire_add_button(&self) {
        let variables_list = self.variables_list.clone();
        let variables = self.variables.clone();
        let settings = self.settings.clone();

        self.add_header_btn.connect_clicked(move |_| {
            // Collapse all existing rows
            for row in variables.borrow().iter() {
                row.expander.set_expanded(false);
            }

            // Create new row expanded
            let var_row = Self::create_variable_row(None, &settings, true);

            // Connect delete button
            let list_clone = variables_list.clone();
            let vars_clone = variables.clone();
            let row_widget = var_row.row.clone();
            var_row.delete_button.connect_clicked(move |_| {
                list_clone.remove(&row_widget);
                let mut vars = vars_clone.borrow_mut();
                vars.retain(|r| r.row != row_widget);
            });

            variables_list.append(&var_row.row);

            // Focus the name entry of the new row
            var_row.name_entry.grab_focus();

            variables.borrow_mut().push(var_row);
        });
    }

    /// Runs the dialog and calls the callback with the result
    pub fn run<F: Fn(Option<Vec<Variable>>) + 'static>(&self, cb: F) {
        *self.on_save.borrow_mut() = Some(Box::new(cb));
        self.wire_add_button();
        self.dialog
            .present(self.parent.as_ref().map(|w| w as &gtk4::Widget));
    }

    /// Returns a reference to the underlying dialog
    #[must_use]
    pub const fn dialog(&self) -> &adw::Dialog {
        &self.dialog
    }
}
