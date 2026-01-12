//! Form validation utilities for dialogs
//!
//! This module provides reusable validation logic and visual feedback
//! for form fields in dialogs.

use gtk4::prelude::*;
use gtk4::{Entry, Label};
use std::cell::RefCell;
use std::rc::Rc;

/// CSS class applied to fields with validation errors
pub const ERROR_CSS_CLASS: &str = "error";

/// CSS class applied to fields with validation warnings
pub const WARNING_CSS_CLASS: &str = "warning";

/// CSS class applied to fields that passed validation
pub const SUCCESS_CSS_CLASS: &str = "success";

/// Result of a validation check
#[derive(Debug, Clone)]
pub enum ValidationResult {
    /// Field is valid
    Valid,
    /// Field has a warning (non-blocking)
    Warning(String),
    /// Field has an error (blocking)
    Error(String),
}

impl ValidationResult {
    /// Returns true if the result is valid (no errors)
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        matches!(self, Self::Valid | Self::Warning(_))
    }

    /// Returns true if the result is an error
    #[must_use]
    pub const fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }

    /// Returns the message if there is one
    #[must_use]
    pub fn message(&self) -> Option<&str> {
        match self {
            Self::Valid => None,
            Self::Warning(msg) | Self::Error(msg) => Some(msg),
        }
    }
}

/// A validator function type
pub type ValidatorFn = Box<dyn Fn(&str) -> ValidationResult>;

/// Validates that a field is not empty
#[must_use]
pub fn required(field_name: &str) -> ValidatorFn {
    let name = field_name.to_string();
    Box::new(move |value: &str| {
        if value.trim().is_empty() {
            ValidationResult::Error(format!("{name} is required"))
        } else {
            ValidationResult::Valid
        }
    })
}

/// Validates that a field matches a minimum length
#[must_use]
pub fn min_length(field_name: &str, min: usize) -> ValidatorFn {
    let name = field_name.to_string();
    Box::new(move |value: &str| {
        if value.len() < min {
            ValidationResult::Error(format!("{name} must be at least {min} characters"))
        } else {
            ValidationResult::Valid
        }
    })
}

/// Validates that a field matches a maximum length
#[must_use]
pub fn max_length(field_name: &str, max: usize) -> ValidatorFn {
    let name = field_name.to_string();
    Box::new(move |value: &str| {
        if value.len() > max {
            ValidationResult::Error(format!("{name} must be at most {max} characters"))
        } else {
            ValidationResult::Valid
        }
    })
}

/// Validates that a field is a valid hostname or IP address
#[must_use]
pub fn hostname() -> ValidatorFn {
    Box::new(|value: &str| {
        let value = value.trim();
        if value.is_empty() {
            return ValidationResult::Error("Host is required".to_string());
        }

        // Basic hostname validation
        // Allow: alphanumeric, hyphens, dots, and IPv6 brackets
        let is_valid = value.chars().all(|c| {
            c.is_alphanumeric() || c == '.' || c == '-' || c == ':' || c == '[' || c == ']'
        });

        if !is_valid {
            ValidationResult::Error("Invalid hostname format".to_string())
        } else if value.starts_with('-') || value.ends_with('-') {
            ValidationResult::Error("Hostname cannot start or end with a hyphen".to_string())
        } else {
            ValidationResult::Valid
        }
    })
}

/// Validates that a field is a valid port number
#[must_use]
pub fn port_number() -> ValidatorFn {
    Box::new(|value: &str| {
        let value = value.trim();
        if value.is_empty() {
            return ValidationResult::Error("Port is required".to_string());
        }

        match value.parse::<u16>() {
            Ok(port) if port > 0 => ValidationResult::Valid,
            Ok(_) => ValidationResult::Error("Port must be greater than 0".to_string()),
            Err(_) => ValidationResult::Error("Invalid port number".to_string()),
        }
    })
}

/// Validates that a field matches a regex pattern
///
/// # Panics
///
/// This function will return an error validator if the regex pattern is invalid,
/// rather than panicking at runtime.
#[must_use]
pub fn pattern(field_name: &str, regex: &str, error_message: &str) -> ValidatorFn {
    let name = field_name.to_string();
    let msg = error_message.to_string();

    // Compile regex at validator creation time, handle invalid patterns gracefully
    match regex::Regex::new(regex) {
        Ok(re) => Box::new(move |value: &str| {
            if re.is_match(value) {
                ValidationResult::Valid
            } else {
                ValidationResult::Error(format!("{name}: {msg}"))
            }
        }),
        Err(e) => {
            // Log the error and return a validator that always fails
            tracing::error!("Invalid regex pattern '{}': {}", regex, e);
            let pattern = regex.to_string();
            Box::new(move |_value: &str| {
                ValidationResult::Error(format!("Invalid validation pattern: {pattern}"))
            })
        }
    }
}

/// Combines multiple validators, returning the first error found
#[must_use]
pub fn all_of(validators: Vec<ValidatorFn>) -> ValidatorFn {
    Box::new(move |value: &str| {
        for validator in &validators {
            let result = validator(value);
            if result.is_error() {
                return result;
            }
        }
        ValidationResult::Valid
    })
}

/// A field validator that can be attached to GTK entries
pub struct FieldValidator {
    /// The entry being validated
    entry: Entry,
    /// Optional error label to display messages
    error_label: Option<Label>,
    /// Validators to run
    validators: Vec<ValidatorFn>,
    /// Current validation state
    is_valid: Rc<RefCell<bool>>,
    /// Callback when validation state changes
    on_change: Rc<RefCell<Option<Box<dyn Fn(bool)>>>>,
}

impl FieldValidator {
    /// Creates a new field validator for an entry
    #[must_use]
    pub fn new(entry: &Entry) -> Self {
        Self {
            entry: entry.clone(),
            error_label: None,
            validators: Vec::new(),
            is_valid: Rc::new(RefCell::new(true)),
            on_change: Rc::new(RefCell::new(None)),
        }
    }

    /// Sets the error label to display validation messages
    #[must_use]
    pub fn with_error_label(mut self, label: &Label) -> Self {
        self.error_label = Some(label.clone());
        self
    }

    /// Adds a validator
    #[must_use]
    pub fn add_validator(mut self, validator: ValidatorFn) -> Self {
        self.validators.push(validator);
        self
    }

    /// Adds a required validator
    #[must_use]
    pub fn required(self, field_name: &str) -> Self {
        self.add_validator(required(field_name))
    }

    /// Adds a hostname validator
    #[must_use]
    pub fn hostname(self) -> Self {
        self.add_validator(hostname())
    }

    /// Sets a callback for when validation state changes
    pub fn on_change<F: Fn(bool) + 'static>(&self, callback: F) {
        *self.on_change.borrow_mut() = Some(Box::new(callback));
    }

    /// Validates the field and updates visual feedback
    ///
    /// Returns true if the field is valid.
    pub fn validate(&self) -> bool {
        let value = self.entry.text();
        let value_str = value.as_str();

        // Run all validators
        let mut result = ValidationResult::Valid;
        for validator in &self.validators {
            result = validator(value_str);
            if result.is_error() {
                break;
            }
        }

        // Update visual feedback
        self.entry.remove_css_class(ERROR_CSS_CLASS);
        self.entry.remove_css_class(WARNING_CSS_CLASS);
        self.entry.remove_css_class(SUCCESS_CSS_CLASS);

        match &result {
            ValidationResult::Valid => {
                if !value_str.is_empty() {
                    self.entry.add_css_class(SUCCESS_CSS_CLASS);
                }
                if let Some(label) = &self.error_label {
                    label.set_text("");
                    label.set_visible(false);
                }
            }
            ValidationResult::Warning(msg) => {
                self.entry.add_css_class(WARNING_CSS_CLASS);
                if let Some(label) = &self.error_label {
                    label.set_text(msg);
                    label.set_visible(true);
                    label.remove_css_class("error");
                    label.add_css_class("warning");
                }
            }
            ValidationResult::Error(msg) => {
                self.entry.add_css_class(ERROR_CSS_CLASS);
                if let Some(label) = &self.error_label {
                    label.set_text(msg);
                    label.set_visible(true);
                    label.remove_css_class("warning");
                    label.add_css_class("error");
                }
            }
        }

        let is_valid = !result.is_error();
        let was_valid = *self.is_valid.borrow();

        if is_valid != was_valid {
            *self.is_valid.borrow_mut() = is_valid;
            if let Some(callback) = self.on_change.borrow().as_ref() {
                callback(is_valid);
            }
        }

        is_valid
    }

    /// Connects the validator to the entry's changed signal
    pub fn connect(&self) {
        let validator = self.clone_for_signal();
        self.entry.connect_changed(move |_| {
            validator.validate();
        });
    }

    /// Creates a clone suitable for use in signal handlers
    fn clone_for_signal(&self) -> Self {
        Self {
            entry: self.entry.clone(),
            error_label: self.error_label.clone(),
            validators: Vec::new(), // Validators are not cloned, validation uses original
            is_valid: self.is_valid.clone(),
            on_change: self.on_change.clone(),
        }
    }

    /// Returns whether the field is currently valid
    #[must_use]
    pub fn is_valid(&self) -> bool {
        *self.is_valid.borrow()
    }
}

/// A form validator that manages multiple field validators
pub struct FormValidator {
    /// Field validators
    fields: Vec<Rc<FieldValidator>>,
    /// Callback when form validity changes
    on_change: Rc<RefCell<Option<Box<dyn Fn(bool)>>>>,
    /// Current form validity
    is_valid: Rc<RefCell<bool>>,
}

impl FormValidator {
    /// Creates a new form validator
    #[must_use]
    pub fn new() -> Self {
        Self {
            fields: Vec::new(),
            on_change: Rc::new(RefCell::new(None)),
            is_valid: Rc::new(RefCell::new(true)),
        }
    }

    /// Adds a field validator
    pub fn add_field(&mut self, validator: FieldValidator) {
        let validator = Rc::new(validator);
        self.fields.push(validator);
    }

    /// Sets a callback for when form validity changes
    pub fn on_change<F: Fn(bool) + 'static>(&self, callback: F) {
        *self.on_change.borrow_mut() = Some(Box::new(callback));
    }

    /// Validates all fields and returns true if the form is valid
    pub fn validate(&self) -> bool {
        let mut all_valid = true;

        for field in &self.fields {
            if !field.validate() {
                all_valid = false;
            }
        }

        let was_valid = *self.is_valid.borrow();
        if all_valid != was_valid {
            *self.is_valid.borrow_mut() = all_valid;
            if let Some(callback) = self.on_change.borrow().as_ref() {
                callback(all_valid);
            }
        }

        all_valid
    }

    /// Returns whether the form is currently valid
    #[must_use]
    pub fn is_valid(&self) -> bool {
        *self.is_valid.borrow()
    }
}

impl Default for FormValidator {
    fn default() -> Self {
        Self::new()
    }
}

/// CSS styles for validation feedback
pub const VALIDATION_CSS: &str = r"
entry.error {
    border-color: @error_color;
    box-shadow: 0 0 0 1px @error_color;
}

entry.warning {
    border-color: @warning_color;
    box-shadow: 0 0 0 1px @warning_color;
}

entry.success {
    border-color: @success_color;
}

label.error {
    color: @error_color;
    font-size: 0.9em;
}

label.warning {
    color: @warning_color;
    font-size: 0.9em;
}
";
