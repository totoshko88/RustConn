//! Template management module
//!
//! This module provides the `TemplateManager` for CRUD operations on connection templates,
//! with support for protocol filtering, search, and import/export.
//! Also includes predefined templates for common CLI tools.

mod manager;
pub mod predefined;

pub use manager::TemplateManager;
pub use predefined::{
    PREDEFINED_TEMPLATES, PredefinedTemplate, TemplateCategory, all_predefined_templates,
    find_predefined_template, templates_by_category,
};
