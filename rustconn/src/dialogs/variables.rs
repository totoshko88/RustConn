//! Variables dialog for managing global and local variables
//!
//! Provides a GTK4 dialog for creating, editing, and deleting variables
//! with support for secret variable masking.
//!
//! Updated for GTK 4.10+ compatibility using Window instead of Dialog.
//! Uses libadwaita components for GNOME HIG compliance.

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
use std::rc::Rc;

use super::VariablesCallback;

/// Shared settings reference for variable rows
type SharedSettings = Rc<RefCell<Option<AppSettings>>>;

/// Variables dialog for managing global variables
pub struct VariablesDialog {
    window: adw::Window,
    variables_list: ListBox,
    add_button: Button,
    variables: Rc<RefCell<Vec<VariableRow>>>,
    on_save: VariablesCallback,
    settings: SharedSettings,
}

/// Represents a variable row in the dialog
struct VariableRow {
    /// The row widget
    row: ListBoxRow,
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
    /// Delete button
    delete_button: Button,
}

impl VariablesDialog {
    /// Creates a new variables dialog for global variables
    #[must_use]
    pub fn new(parent: Option<&gtk4::Window>) -> Self {
        let window = adw::Window::builder()
            .title(i18n("Global Variables"))
            .modal(true)
            .default_width(500)
            .default_height(400)
            .build();

        if let Some(p) = parent {
            window.set_transient_for(Some(p));
        }

        window.set_size_request(320, 280);

        // Header bar (GNOME HIG)
        let (header, cancel_btn, save_btn) =
            crate::dialogs::widgets::dialog_header("Cancel", "Save");

        // Cancel button handler
        let window_clone = window.clone();
        cancel_btn.connect_clicked(move |_| {
            window_clone.close();
        });

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

        // Use ToolbarView for adw::Window
        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&clamp));
        window.set_content(Some(&toolbar_view));

        // Variables list in PreferencesGroup
        let (group, variables_list, add_button) = Self::create_variables_section();
        content.append(&group);

        let on_save: VariablesCallback = Rc::new(RefCell::new(None));
        let variables: Rc<RefCell<Vec<VariableRow>>> = Rc::new(RefCell::new(Vec::new()));
        let settings: SharedSettings = Rc::new(RefCell::new(None));

        // Connect cancel button
        let window_clone = window.clone();
        let on_save_clone = on_save.clone();
        cancel_btn.connect_clicked(move |_| {
            if let Some(ref cb) = *on_save_clone.borrow() {
                cb(None);
            }
            window_clone.close();
        });

        // Connect save button
        let window_clone = window.clone();
        let on_save_clone = on_save.clone();
        let variables_clone = variables.clone();
        save_btn.connect_clicked(move |_| {
            let vars = Self::collect_variables(&variables_clone);
            if let Some(ref cb) = *on_save_clone.borrow() {
                cb(Some(vars));
            }
            window_clone.close();
        });

        Self {
            window,
            variables_list,
            add_button,
            variables,
            on_save,
            settings,
        }
    }

    /// Creates the variables section with list and add button
    fn create_variables_section() -> (adw::PreferencesGroup, ListBox, Button) {
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

        let button_box = GtkBox::new(Orientation::Horizontal, 8);
        button_box.set_halign(gtk4::Align::End);
        button_box.set_margin_top(12);

        let add_button = Button::builder()
            .label(i18n("Add Variable"))
            .css_classes(["suggested-action"])
            .build();
        button_box.append(&add_button);

        group.add(&button_box);

        (group, variables_list, add_button)
    }

    /// Creates a variable row widget
    fn create_variable_row(variable: Option<&Variable>, settings: &SharedSettings) -> VariableRow {
        let main_box = GtkBox::new(Orientation::Vertical, 8);
        main_box.set_margin_top(8);
        main_box.set_margin_bottom(8);
        main_box.set_margin_start(8);
        main_box.set_margin_end(8);

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
            .build();
        let delete_button = Button::builder()
            .icon_name("user-trash-symbolic")
            .css_classes(["destructive-action", "flat"])
            .tooltip_text(i18n("Delete variable"))
            .build();

        grid.attach(&name_label, 0, 0, 1, 1);
        grid.attach(&name_entry, 1, 0, 1, 1);
        grid.attach(&delete_button, 2, 0, 1, 1);

        // Row 1: Value — single row, switches between plain and secret mode
        let value_label = Label::builder()
            .label(i18n("Value:"))
            .halign(gtk4::Align::End)
            .build();
        // Plain value entry (visible when not secret)
        let value_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("Variable value")
            .build();
        // Secret value entry (visible when secret, with masked input)
        let secret_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("Password value")
            .visibility(false)
            .build();
        // Show/Hide toggle button (secret mode only)
        let show_hide_btn = Button::builder()
            .icon_name("view-reveal-symbolic")
            .tooltip_text(i18n("Show/hide password"))
            .build();
        // Load from Vault button (secret mode only)
        let load_vault_btn = Button::builder()
            .icon_name("document-open-symbolic")
            .tooltip_text(i18n("Load password from vault"))
            .build();
        // Secret row: entry + show/hide + load buttons
        let secret_buttons_box = GtkBox::new(Orientation::Horizontal, 4);
        secret_buttons_box.append(&secret_entry);
        secret_buttons_box.append(&show_hide_btn);
        secret_buttons_box.append(&load_vault_btn);
        secret_buttons_box.set_hexpand(true);
        secret_buttons_box.set_visible(false);

        grid.attach(&value_label, 0, 1, 1, 1);
        grid.attach(&value_entry, 1, 1, 2, 1);
        grid.attach(&secret_buttons_box, 1, 1, 2, 1);

        // Row 2: Is Secret checkbox
        let is_secret_check = CheckButton::builder()
            .label(i18n("Secret (mask value)"))
            .build();

        grid.attach(&is_secret_check, 1, 2, 2, 1);

        // Row 3: Description
        let desc_label = Label::builder()
            .label(i18n("Description:"))
            .halign(gtk4::Align::End)
            .build();
        let description_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("Optional description")
            .build();

        grid.attach(&desc_label, 0, 3, 1, 1);
        grid.attach(&description_entry, 1, 3, 2, 1);

        main_box.append(&grid);

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
        let settings_for_load = settings.clone();
        load_vault_btn.connect_clicked(move |btn| {
            let var_name = name_entry_for_load.text().to_string();
            if var_name.trim().is_empty() {
                return;
            }
            let entry_clone = secret_entry_for_load.clone();
            let btn_clone = btn.clone();
            let settings_snap = settings_for_load.borrow().clone();

            btn.set_sensitive(false);
            btn.set_icon_name("content-loading-symbolic");

            crate::utils::spawn_blocking_with_callback(
                move || {
                    if let Some(ref s) = settings_snap {
                        crate::state::load_variable_from_vault(&s.secrets, &var_name)
                    } else {
                        // No settings — fall back to libsecret
                        let lookup_key = rustconn_core::variable_secret_key(&var_name);
                        let backend = rustconn_core::secret::LibSecretBackend::new("rustconn");
                        crate::async_utils::with_runtime(|rt| {
                            let creds = rt
                                .block_on(rustconn_core::secret::SecretBackend::retrieve(
                                    &backend,
                                    &lookup_key,
                                ))
                                .map_err(|e| format!("{e}"))?;
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
                            if let Some(window) = btn_clone.root().and_downcast::<gtk4::Window>() {
                                crate::toast::show_toast_on_window(
                                    &window,
                                    "Failed to load secret from \
                                     vault. Check secret backend \
                                     in Settings.",
                                    crate::toast::ToastType::Error,
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
        is_secret_check.connect_toggled(move |check| {
            let is_secret = check.is_active();
            value_entry_clone.set_visible(!is_secret);
            secret_buttons_clone.set_visible(is_secret);

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

        VariableRow {
            row,
            name_entry,
            value_entry,
            secret_entry,
            is_secret_check,
            description_entry,
            delete_button,
        }
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

                let mut var = Variable::new(name, value);
                var.is_secret = is_secret;
                var.description = description;
                Some(var)
            })
            .collect()
    }

    /// Sets the application settings for vault backend selection
    pub fn set_settings(&self, settings: &AppSettings) {
        *self.settings.borrow_mut() = Some(settings.clone());
    }

    /// Sets the initial variables to display
    pub fn set_variables(&self, variables: &[Variable]) {
        // Clear existing rows
        while let Some(row) = self.variables_list.row_at_index(0) {
            self.variables_list.remove(&row);
        }
        self.variables.borrow_mut().clear();

        // Add rows for each variable
        for var in variables {
            self.add_variable_row(Some(var));
        }
    }

    /// Adds a new variable row to the list
    fn add_variable_row(&self, variable: Option<&Variable>) {
        let var_row = Self::create_variable_row(variable, &self.settings);

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

    /// Wires up the add button
    fn wire_add_button(&self) {
        let variables_list = self.variables_list.clone();
        let variables = self.variables.clone();
        let settings = self.settings.clone();

        self.add_button.connect_clicked(move |_| {
            let var_row = Self::create_variable_row(None, &settings);

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
            variables.borrow_mut().push(var_row);
        });
    }

    /// Runs the dialog and calls the callback with the result
    pub fn run<F: Fn(Option<Vec<Variable>>) + 'static>(&self, cb: F) {
        *self.on_save.borrow_mut() = Some(Box::new(cb));
        self.wire_add_button();
        self.window.present();
    }

    /// Returns a reference to the underlying window
    #[must_use]
    pub const fn window(&self) -> &adw::Window {
        &self.window
    }
}
