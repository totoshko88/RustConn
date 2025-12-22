//! Variables dialog for managing global and local variables
//!
//! Provides a GTK4 dialog for creating, editing, and deleting variables
//! with support for secret variable masking.
//!
//! Updated for GTK 4.10+ compatibility using Window instead of Dialog.

use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, CheckButton, Entry, Frame, Grid, HeaderBar, Label, ListBox, ListBoxRow,
    Orientation, PasswordEntry, ScrolledWindow, Window,
};
use rustconn_core::variables::Variable;
use std::cell::RefCell;
use std::rc::Rc;

use super::VariablesCallback;

/// Variables dialog for managing global variables
pub struct VariablesDialog {
    window: Window,
    variables_list: ListBox,
    add_button: Button,
    variables: Rc<RefCell<Vec<VariableRow>>>,
    on_save: VariablesCallback,
}

/// Represents a variable row in the dialog
struct VariableRow {
    /// The row widget
    row: ListBoxRow,
    /// Entry for variable name
    name_entry: Entry,
    /// Entry for variable value (regular)
    value_entry: Entry,
    /// Entry for secret value (password)
    secret_entry: PasswordEntry,
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
    pub fn new(parent: Option<&Window>) -> Self {
        let window = Window::builder()
            .title("Global Variables")
            .modal(true)
            .default_width(600)
            .default_height(500)
            .build();

        if let Some(p) = parent {
            window.set_transient_for(Some(p));
        }

        // Create header bar with Cancel/Save buttons
        let header = HeaderBar::new();
        let cancel_btn = Button::builder().label("Cancel").build();
        let save_btn = Button::builder()
            .label("Save")
            .css_classes(["suggested-action"])
            .build();
        header.pack_start(&cancel_btn);
        header.pack_end(&save_btn);
        window.set_titlebar(Some(&header));

        // Create main content area
        let content = GtkBox::new(Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);
        window.set_child(Some(&content));

        // Info label
        let info_label = Label::builder()
            .label("Define variables that can be used in connections with ${variable_name} syntax.")
            .halign(gtk4::Align::Start)
            .wrap(true)
            .css_classes(["dim-label"])
            .build();
        content.append(&info_label);

        // Variables list frame
        let (frame, variables_list, add_button) = Self::create_variables_section();
        content.append(&frame);

        let on_save: VariablesCallback = Rc::new(RefCell::new(None));
        let variables: Rc<RefCell<Vec<VariableRow>>> = Rc::new(RefCell::new(Vec::new()));

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
        }
    }

    /// Creates the variables section with list and add button
    fn create_variables_section() -> (Frame, ListBox, Button) {
        let vbox = GtkBox::new(Orientation::Vertical, 8);
        vbox.set_margin_top(8);
        vbox.set_margin_bottom(8);
        vbox.set_margin_start(8);
        vbox.set_margin_end(8);

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

        vbox.append(&scrolled);

        let button_box = GtkBox::new(Orientation::Horizontal, 8);
        button_box.set_halign(gtk4::Align::End);

        let add_button = Button::builder()
            .label("Add Variable")
            .css_classes(["suggested-action"])
            .build();
        button_box.append(&add_button);

        vbox.append(&button_box);

        let frame = Frame::builder()
            .label("Variables")
            .child(&vbox)
            .vexpand(true)
            .build();

        (frame, variables_list, add_button)
    }

    /// Creates a variable row widget
    fn create_variable_row(variable: Option<&Variable>) -> VariableRow {
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
            .label("Name:")
            .halign(gtk4::Align::End)
            .build();
        let name_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("variable_name")
            .build();
        let delete_button = Button::builder()
            .icon_name("user-trash-symbolic")
            .css_classes(["destructive-action", "flat"])
            .tooltip_text("Delete variable")
            .build();

        grid.attach(&name_label, 0, 0, 1, 1);
        grid.attach(&name_entry, 1, 0, 1, 1);
        grid.attach(&delete_button, 2, 0, 1, 1);

        // Row 1: Value (regular entry)
        let value_label = Label::builder()
            .label("Value:")
            .halign(gtk4::Align::End)
            .build();
        let value_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("Variable value")
            .build();

        grid.attach(&value_label, 0, 1, 1, 1);
        grid.attach(&value_entry, 1, 1, 2, 1);

        // Row 2: Secret value (password entry, initially hidden)
        let secret_label = Label::builder()
            .label("Secret Value:")
            .halign(gtk4::Align::End)
            .visible(false)
            .build();
        let secret_entry = PasswordEntry::builder()
            .hexpand(true)
            .placeholder_text("Secret value (masked)")
            .show_peek_icon(true)
            .visible(false)
            .build();

        grid.attach(&secret_label, 0, 2, 1, 1);
        grid.attach(&secret_entry, 1, 2, 2, 1);

        // Row 3: Is Secret checkbox
        let is_secret_check = CheckButton::builder().label("Secret (mask value)").build();

        grid.attach(&is_secret_check, 1, 3, 2, 1);

        // Row 4: Description
        let desc_label = Label::builder()
            .label("Description:")
            .halign(gtk4::Align::End)
            .build();
        let description_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("Optional description")
            .build();

        grid.attach(&desc_label, 0, 4, 1, 1);
        grid.attach(&description_entry, 1, 4, 2, 1);

        main_box.append(&grid);

        // Connect is_secret checkbox to toggle value/secret entry visibility
        let value_entry_clone = value_entry.clone();
        let secret_entry_clone = secret_entry.clone();
        let value_label_clone = value_label;
        let secret_label_clone = secret_label;
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
        let var_row = Self::create_variable_row(variable);

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

        self.add_button.connect_clicked(move |_| {
            let var_row = Self::create_variable_row(None);

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
    pub const fn window(&self) -> &Window {
        &self.window
    }
}
