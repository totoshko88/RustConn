//! Snippet dialog for creating and editing command snippets
//!
//! Provides a GTK4 dialog for creating/editing snippets with variable definitions
//! and category assignment.
//!
//! Updated for GTK 4.10+ compatibility using Window instead of Dialog.

use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, Entry, Frame, Grid, HeaderBar, Label, ListBox, ListBoxRow, Orientation,
    ScrolledWindow, TextView, Window,
};
use regex::Regex;
use rustconn_core::models::{Snippet, SnippetVariable};
use std::cell::RefCell;
use std::rc::Rc;
use uuid::Uuid;

/// Snippet dialog for creating/editing snippets
pub struct SnippetDialog {
    window: Window,
    name_entry: Entry,
    description_entry: Entry,
    category_entry: Entry,
    tags_entry: Entry,
    command_view: TextView,
    variables_list: ListBox,
    add_var_button: Button,
    save_btn: Button,
    editing_id: Rc<RefCell<Option<Uuid>>>,
    variables: Rc<RefCell<Vec<VariableRow>>>,
    on_save: super::SnippetCallback,
}

/// Represents a variable row in the dialog
///
/// Stores the GTK widgets for a single variable entry, including:
/// - Name (read-only, auto-detected from command)
/// - Description (optional, user-editable)
/// - Default value (optional, user-editable)
struct VariableRow {
    /// The variable name (e.g., "host", "user")
    name: String,
    /// The `ListBoxRow` widget containing this variable
    row: ListBoxRow,
    /// Entry widget for the variable name (read-only display)
    name_entry: Entry,
    /// Entry widget for the variable description
    desc_entry: Entry,
    /// Entry widget for the default value
    default_entry: Entry,
}

impl SnippetDialog {
    /// Creates a new snippet dialog
    #[must_use]
    pub fn new(parent: Option<&Window>) -> Self {
        // Create window instead of deprecated Dialog
        let window = Window::builder()
            .title("New Snippet")
            .modal(true)
            .default_width(550)
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

        // === Basic Info Section ===
        let (basic_frame, name_entry, description_entry, category_entry, tags_entry) =
            Self::create_basic_section();
        content.append(&basic_frame);

        // === Command Section ===
        let (command_frame, command_view) = Self::create_command_section();
        content.append(&command_frame);

        // === Variables Section ===
        let (variables_frame, variables_list, add_var_button) = Self::create_variables_section();
        content.append(&variables_frame);

        // Connect command text changes to auto-detect variables
        let vars_list = variables_list.clone();
        let variables = Rc::new(RefCell::new(Vec::new()));
        let vars_clone = variables.clone();

        let buffer = command_view.buffer();
        buffer.connect_changed(move |buf| {
            let (start, end) = buf.bounds();
            let text = buf.text(&start, &end, false);
            Self::auto_detect_variables(&text, &vars_list, &vars_clone);
        });

        let on_save: super::SnippetCallback = Rc::new(RefCell::new(None));

        // Connect cancel button
        let window_clone = window.clone();
        let on_save_clone = on_save.clone();
        cancel_btn.connect_clicked(move |_| {
            if let Some(ref cb) = *on_save_clone.borrow() {
                cb(None);
            }
            window_clone.close();
        });

        Self {
            window,
            name_entry,
            description_entry,
            category_entry,
            tags_entry,
            command_view,
            variables_list,
            add_var_button,
            save_btn,
            editing_id: Rc::new(RefCell::new(None)),
            variables,
            on_save,
        }
    }

    fn create_basic_section() -> (Frame, Entry, Entry, Entry, Entry) {
        let grid = Grid::builder()
            .row_spacing(8)
            .column_spacing(12)
            .margin_top(8)
            .margin_bottom(8)
            .margin_start(8)
            .margin_end(8)
            .build();

        let mut row = 0;

        // Name
        let name_label = Label::builder()
            .label("Name:")
            .halign(gtk4::Align::End)
            .build();
        let name_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("Snippet name")
            .build();
        grid.attach(&name_label, 0, row, 1, 1);
        grid.attach(&name_entry, 1, row, 1, 1);
        row += 1;

        // Description
        let desc_label = Label::builder()
            .label("Description:")
            .halign(gtk4::Align::End)
            .build();
        let description_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("Optional description")
            .build();
        grid.attach(&desc_label, 0, row, 1, 1);
        grid.attach(&description_entry, 1, row, 1, 1);
        row += 1;

        // Category
        let cat_label = Label::builder()
            .label("Category:")
            .halign(gtk4::Align::End)
            .build();
        let category_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("e.g., System, Network, Database")
            .build();
        grid.attach(&cat_label, 0, row, 1, 1);
        grid.attach(&category_entry, 1, row, 1, 1);
        row += 1;

        // Tags
        let tags_label = Label::builder()
            .label("Tags:")
            .halign(gtk4::Align::End)
            .build();
        let tags_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("tag1, tag2, ...")
            .build();
        grid.attach(&tags_label, 0, row, 1, 1);
        grid.attach(&tags_entry, 1, row, 1, 1);

        let frame = Frame::builder().label("Snippet Info").child(&grid).build();

        (
            frame,
            name_entry,
            description_entry,
            category_entry,
            tags_entry,
        )
    }

    fn create_command_section() -> (Frame, TextView) {
        let vbox = GtkBox::new(Orientation::Vertical, 4);
        vbox.set_margin_top(8);
        vbox.set_margin_bottom(8);
        vbox.set_margin_start(8);
        vbox.set_margin_end(8);

        let hint = Label::builder()
            .label("Use ${variable_name} for placeholders")
            .halign(gtk4::Align::Start)
            .css_classes(["dim-label"])
            .build();
        vbox.append(&hint);

        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Automatic)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .min_content_height(80)
            .build();

        let command_view = TextView::builder()
            .monospace(true)
            .wrap_mode(gtk4::WrapMode::Word)
            .build();
        scrolled.set_child(Some(&command_view));

        vbox.append(&scrolled);

        let frame = Frame::builder().label("Command").child(&vbox).build();

        (frame, command_view)
    }

    fn create_variables_section() -> (Frame, ListBox, Button) {
        let vbox = GtkBox::new(Orientation::Vertical, 8);
        vbox.set_margin_top(8);
        vbox.set_margin_bottom(8);
        vbox.set_margin_start(8);
        vbox.set_margin_end(8);

        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .min_content_height(100)
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

        let add_var_button = Button::builder().label("Add Variable").build();
        button_box.append(&add_var_button);

        vbox.append(&button_box);

        let frame = Frame::builder().label("Variables").child(&vbox).build();

        (frame, variables_list, add_var_button)
    }

    fn auto_detect_variables(
        command: &str,
        list: &ListBox,
        variables: &Rc<RefCell<Vec<VariableRow>>>,
    ) {
        // Extract variable names from ${var_name} patterns
        let re = Regex::new(r"\$\{([a-zA-Z_][a-zA-Z0-9_]*)\}").unwrap();
        let mut found_vars: Vec<String> = Vec::new();

        for cap in re.captures_iter(command) {
            let var_name = cap.get(1).unwrap().as_str().to_string();
            if !found_vars.contains(&var_name) {
                found_vars.push(var_name);
            }
        }

        // Check existing variables and add new ones
        let mut vars = variables.borrow_mut();
        let existing_names: Vec<String> = vars.iter().map(|v| v.name.clone()).collect();

        for var_name in found_vars {
            if !existing_names.contains(&var_name) {
                // Add new variable row
                let row = Self::create_variable_row(&var_name, None, None);
                list.append(&row.row);
                vars.push(row);
            }
        }
    }

    fn create_variable_row(
        name: &str,
        description: Option<&str>,
        default: Option<&str>,
    ) -> VariableRow {
        let hbox = GtkBox::new(Orientation::Horizontal, 8);
        hbox.set_margin_top(8);
        hbox.set_margin_bottom(8);
        hbox.set_margin_start(8);
        hbox.set_margin_end(8);

        let grid = Grid::builder()
            .row_spacing(4)
            .column_spacing(8)
            .hexpand(true)
            .build();

        // Variable name (read-only display)
        let name_label = Label::builder()
            .label("Name:")
            .halign(gtk4::Align::End)
            .build();
        let name_entry = Entry::builder()
            .text(name)
            .editable(false)
            .css_classes(["monospace"])
            .build();
        grid.attach(&name_label, 0, 0, 1, 1);
        grid.attach(&name_entry, 1, 0, 1, 1);

        // Description
        let desc_label = Label::builder()
            .label("Description:")
            .halign(gtk4::Align::End)
            .build();
        let desc_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("Variable description")
            .build();
        if let Some(d) = description {
            desc_entry.set_text(d);
        }
        grid.attach(&desc_label, 0, 1, 1, 1);
        grid.attach(&desc_entry, 1, 1, 1, 1);

        // Default value
        let default_label = Label::builder()
            .label("Default:")
            .halign(gtk4::Align::End)
            .build();
        let default_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("Default value")
            .build();
        if let Some(d) = default {
            default_entry.set_text(d);
        }
        grid.attach(&default_label, 0, 2, 1, 1);
        grid.attach(&default_entry, 1, 2, 1, 1);

        hbox.append(&grid);

        let row = ListBoxRow::builder().child(&hbox).build();

        VariableRow {
            name: name.to_string(),
            row,
            name_entry,
            desc_entry,
            default_entry,
        }
    }

    /// Populates the dialog with an existing snippet for editing
    pub fn set_snippet(&self, snippet: &Snippet) {
        self.window.set_title(Some("Edit Snippet"));
        *self.editing_id.borrow_mut() = Some(snippet.id);

        self.name_entry.set_text(&snippet.name);
        if let Some(ref desc) = snippet.description {
            self.description_entry.set_text(desc);
        }
        if let Some(ref cat) = snippet.category {
            self.category_entry.set_text(cat);
        }
        self.tags_entry.set_text(&snippet.tags.join(", "));

        // Set command
        self.command_view.buffer().set_text(&snippet.command);

        // Clear and populate variables
        while let Some(row) = self.variables_list.row_at_index(0) {
            self.variables_list.remove(&row);
        }
        self.variables.borrow_mut().clear();

        for var in &snippet.variables {
            let row = Self::create_variable_row(
                &var.name,
                var.description.as_deref(),
                var.default_value.as_deref(),
            );
            self.variables_list.append(&row.row);
            self.variables.borrow_mut().push(row);
        }
    }

    /// Validates the input fields
    ///
    /// Validates that name and command fields are non-empty.
    /// Called before building a snippet to ensure data integrity.
    ///
    /// # Returns
    /// - `Ok(())` if all required fields are valid
    /// - `Err(String)` with a descriptive error message if validation fails
    pub fn validate(&self) -> Result<(), String> {
        let name = self.name_entry.text();
        if name.trim().is_empty() {
            return Err("Snippet name is required".to_string());
        }

        let buffer = self.command_view.buffer();
        let (start, end) = buffer.bounds();
        let command = buffer.text(&start, &end, false);
        if command.trim().is_empty() {
            return Err("Command is required".to_string());
        }

        Ok(())
    }

    /// Shows an error message in an alert dialog
    ///
    /// Displays a modal alert dialog with the given error message.
    pub fn show_error(&self, message: &str) {
        let alert = gtk4::AlertDialog::builder()
            .message("Validation Error")
            .detail(message)
            .modal(true)
            .build();
        alert.show(Some(&self.window));
    }

    /// Wires the add variable button to add new variable rows
    ///
    /// When clicked, prompts for a variable name and adds a new row
    /// with description and `default_value` fields.
    fn wire_add_var_button(&self) {
        let variables_list = self.variables_list.clone();
        let variables = self.variables.clone();

        self.add_var_button.connect_clicked(move |_| {
            // Create a simple dialog to get the variable name
            // For now, we'll use a counter-based name
            let var_count = variables.borrow().len();
            let var_name = format!("var{}", var_count + 1);

            // Create and add the variable row
            let row = Self::create_variable_row(&var_name, None, None);
            variables_list.append(&row.row);
            variables.borrow_mut().push(row);
        });
    }

    /// Adds a variable row manually with specified values
    ///
    /// Used for programmatically adding variables with description and default values.
    pub fn add_variable(&self, name: &str, description: Option<&str>, default_value: Option<&str>) {
        let row = Self::create_variable_row(name, description, default_value);
        self.variables_list.append(&row.row);
        self.variables.borrow_mut().push(row);
    }

    /// Builds a Snippet from the dialog fields
    ///
    /// Constructs a complete Snippet object from all dialog fields including:
    /// - Name and command (required)
    /// - Description and category (optional)
    /// - Tags (comma-separated)
    /// - Variables with description and `default_value` fields
    ///
    /// # Returns
    /// - `Some(Snippet)` with all fields populated from the dialog
    /// - Preserves the editing ID if editing an existing snippet
    #[must_use]
    pub fn build_snippet(&self) -> Option<Snippet> {
        Self::build_snippet_from_fields(
            &self.name_entry,
            &self.description_entry,
            &self.category_entry,
            &self.tags_entry,
            &self.command_view,
            &self.variables,
            &self.editing_id,
        )
    }

    /// Runs the dialog and calls the callback with the result
    ///
    /// Connects the Save button to `validate()` and `build_snippet()` methods,
    /// then presents the dialog window.
    pub fn run<F: Fn(Option<Snippet>) + 'static>(&self, cb: F) {
        // Store callback
        *self.on_save.borrow_mut() = Some(Box::new(cb));

        // Wire up the add variable button
        self.wire_add_var_button();

        // Connect save button directly using stored reference
        let window = self.window.clone();
        let on_save = self.on_save.clone();
        let name_entry = self.name_entry.clone();
        let description_entry = self.description_entry.clone();
        let category_entry = self.category_entry.clone();
        let tags_entry = self.tags_entry.clone();
        let command_view = self.command_view.clone();
        let variables = self.variables.clone();
        let editing_id = self.editing_id.clone();

        self.save_btn.connect_clicked(move |_| {
            // Validate
            let name = name_entry.text();
            if name.trim().is_empty() {
                let alert = gtk4::AlertDialog::builder()
                    .message("Validation Error")
                    .detail("Snippet name is required")
                    .modal(true)
                    .build();
                alert.show(Some(&window));
                return;
            }

            let buffer = command_view.buffer();
            let (start, end) = buffer.bounds();
            let command = buffer.text(&start, &end, false);
            if command.trim().is_empty() {
                let alert = gtk4::AlertDialog::builder()
                    .message("Validation Error")
                    .detail("Command is required")
                    .modal(true)
                    .build();
                alert.show(Some(&window));
                return;
            }

            // Build snippet
            let snippet = Self::build_snippet_from_fields(
                &name_entry,
                &description_entry,
                &category_entry,
                &tags_entry,
                &command_view,
                &variables,
                &editing_id,
            );

            if let Some(ref cb) = *on_save.borrow() {
                cb(snippet);
            }
            window.close();
        });

        self.window.present();
    }

    /// Builds a Snippet from the provided field references
    ///
    /// Helper method to avoid code duplication between `run()` closure and `build_snippet()`.
    fn build_snippet_from_fields(
        name_entry: &Entry,
        description_entry: &Entry,
        category_entry: &Entry,
        tags_entry: &Entry,
        command_view: &TextView,
        variables: &Rc<RefCell<Vec<VariableRow>>>,
        editing_id: &Rc<RefCell<Option<Uuid>>>,
    ) -> Option<Snippet> {
        let name = name_entry.text().trim().to_string();
        let buffer = command_view.buffer();
        let (start, end) = buffer.bounds();
        let command = buffer.text(&start, &end, false).to_string();

        let mut snippet = Snippet::new(name, command);

        // Description
        let desc = description_entry.text();
        if !desc.trim().is_empty() {
            snippet.description = Some(desc.trim().to_string());
        }

        // Category
        let cat = category_entry.text();
        if !cat.trim().is_empty() {
            snippet.category = Some(cat.trim().to_string());
        }

        // Tags
        let tags_text = tags_entry.text();
        if !tags_text.trim().is_empty() {
            snippet.tags = tags_text
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }

        // Variables
        let vars = variables.borrow();
        snippet.variables = vars
            .iter()
            .map(|v| {
                let desc = v.desc_entry.text();
                let default = v.default_entry.text();
                SnippetVariable {
                    name: v.name_entry.text().to_string(),
                    description: if desc.trim().is_empty() {
                        None
                    } else {
                        Some(desc.trim().to_string())
                    },
                    default_value: if default.trim().is_empty() {
                        None
                    } else {
                        Some(default.trim().to_string())
                    },
                }
            })
            .collect();

        // Preserve ID if editing
        if let Some(id) = *editing_id.borrow() {
            snippet.id = id;
        }

        Some(snippet)
    }

    /// Returns a reference to the underlying window
    #[must_use]
    pub const fn window(&self) -> &Window {
        &self.window
    }
}
