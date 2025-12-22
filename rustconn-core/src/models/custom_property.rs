//! Custom property model for connections.
//!
//! This module provides custom metadata fields that can be attached to connections,
//! supporting text, URL, and protected (encrypted) field types.

use serde::{Deserialize, Serialize};

/// Custom property field type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropertyType {
    /// Plain text field
    #[default]
    Text,
    /// URL field (can be rendered as clickable link)
    Url,
    /// Protected field (encrypted storage, masked display)
    Protected,
}

/// A custom property attached to a connection
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomProperty {
    /// Property name/key
    pub name: String,
    /// Property value (encrypted if protected type)
    pub value: String,
    /// Type of the property
    #[serde(default)]
    pub property_type: PropertyType,
}

impl CustomProperty {
    /// Creates a new text property
    #[must_use]
    pub fn new_text(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            property_type: PropertyType::Text,
        }
    }

    /// Creates a new URL property
    #[must_use]
    pub fn new_url(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            property_type: PropertyType::Url,
        }
    }

    /// Creates a new protected property
    #[must_use]
    pub fn new_protected(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            property_type: PropertyType::Protected,
        }
    }

    /// Returns true if this is a protected property
    #[must_use]
    pub const fn is_protected(&self) -> bool {
        matches!(self.property_type, PropertyType::Protected)
    }

    /// Returns true if this is a URL property
    #[must_use]
    pub const fn is_url(&self) -> bool {
        matches!(self.property_type, PropertyType::Url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_text_property() {
        let prop = CustomProperty::new_text("notes", "Some notes");
        assert_eq!(prop.name, "notes");
        assert_eq!(prop.value, "Some notes");
        assert_eq!(prop.property_type, PropertyType::Text);
        assert!(!prop.is_protected());
        assert!(!prop.is_url());
    }

    #[test]
    fn test_new_url_property() {
        let prop = CustomProperty::new_url("docs", "https://example.com");
        assert_eq!(prop.name, "docs");
        assert_eq!(prop.value, "https://example.com");
        assert_eq!(prop.property_type, PropertyType::Url);
        assert!(!prop.is_protected());
        assert!(prop.is_url());
    }

    #[test]
    fn test_new_protected_property() {
        let prop = CustomProperty::new_protected("api_key", "secret123");
        assert_eq!(prop.name, "api_key");
        assert_eq!(prop.value, "secret123");
        assert_eq!(prop.property_type, PropertyType::Protected);
        assert!(prop.is_protected());
        assert!(!prop.is_url());
    }

    #[test]
    fn test_property_type_default() {
        assert_eq!(PropertyType::default(), PropertyType::Text);
    }

    #[test]
    fn test_serialization_round_trip() {
        let prop = CustomProperty::new_protected("secret", "value123");
        let json = serde_json::to_string(&prop).unwrap();
        let deserialized: CustomProperty = serde_json::from_str(&json).unwrap();
        assert_eq!(prop, deserialized);
    }
}
