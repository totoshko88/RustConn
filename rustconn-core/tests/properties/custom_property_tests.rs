//! Property-based tests for custom properties
//!
//! **Feature: rustconn-enhancements, Property 22: Custom Property Type Preservation**
//! **Validates: Requirements 10.6**

use proptest::prelude::*;
use rustconn_core::{CustomProperty, PropertyType};

/// Strategy for generating valid property names
fn arb_property_name() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_]{0,31}".prop_map(|s| s)
}

/// Strategy for generating property values
fn arb_property_value() -> impl Strategy<Value = String> {
    // Generate various types of values including empty, simple text, and URLs
    prop_oneof![
        Just(String::new()),
        "[a-zA-Z0-9 _-]{1,100}",
        "https?://[a-z0-9]+\\.[a-z]{2,4}(/[a-z0-9_-]*)*",
    ]
}

/// Strategy for generating property types
fn arb_property_type() -> impl Strategy<Value = PropertyType> {
    prop_oneof![
        Just(PropertyType::Text),
        Just(PropertyType::Url),
        Just(PropertyType::Protected),
    ]
}

/// Strategy for generating a complete CustomProperty
fn arb_custom_property() -> impl Strategy<Value = CustomProperty> {
    (
        arb_property_name(),
        arb_property_value(),
        arb_property_type(),
    )
        .prop_map(|(name, value, property_type)| CustomProperty {
            name,
            value,
            property_type,
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn-enhancements, Property 22: Custom Property Type Preservation**
    /// **Validates: Requirements 10.6**
    ///
    /// For any custom property with a specific type, serializing to JSON and
    /// deserializing should preserve the property type.
    #[test]
    fn custom_property_type_preservation_json(prop in arb_custom_property()) {
        // Serialize to JSON
        let json_str = serde_json::to_string(&prop)
            .expect("CustomProperty should serialize to JSON");

        // Deserialize back from JSON
        let deserialized: CustomProperty = serde_json::from_str(&json_str)
            .expect("JSON should deserialize back to CustomProperty");

        // Verify type is preserved
        prop_assert_eq!(
            prop.property_type, deserialized.property_type,
            "Property type should be preserved through JSON serialization"
        );
        prop_assert_eq!(
            prop.name, deserialized.name,
            "Property name should be preserved"
        );
        prop_assert_eq!(
            prop.value, deserialized.value,
            "Property value should be preserved"
        );
    }

    /// **Feature: rustconn-enhancements, Property 22: Custom Property Type Preservation**
    /// **Validates: Requirements 10.6**
    ///
    /// For any custom property with a specific type, serializing to TOML and
    /// deserializing should preserve the property type.
    #[test]
    fn custom_property_type_preservation_toml(prop in arb_custom_property()) {
        // Serialize to TOML
        let toml_str = toml::to_string(&prop)
            .expect("CustomProperty should serialize to TOML");

        // Deserialize back from TOML
        let deserialized: CustomProperty = toml::from_str(&toml_str)
            .expect("TOML should deserialize back to CustomProperty");

        // Verify type is preserved
        prop_assert_eq!(
            prop.property_type, deserialized.property_type,
            "Property type should be preserved through TOML serialization"
        );
        prop_assert_eq!(
            prop.name, deserialized.name,
            "Property name should be preserved"
        );
        prop_assert_eq!(
            prop.value, deserialized.value,
            "Property value should be preserved"
        );
    }

    /// Additional test: Full round-trip equality
    ///
    /// For any custom property, serializing and deserializing should produce
    /// an equivalent property.
    #[test]
    fn custom_property_full_round_trip(prop in arb_custom_property()) {
        // JSON round-trip
        let json_str = serde_json::to_string(&prop)
            .expect("CustomProperty should serialize to JSON");
        let json_deserialized: CustomProperty = serde_json::from_str(&json_str)
            .expect("JSON should deserialize back to CustomProperty");
        prop_assert_eq!(prop.clone(), json_deserialized, "CustomProperty should round-trip through JSON");

        // TOML round-trip
        let toml_str = toml::to_string(&prop)
            .expect("CustomProperty should serialize to TOML");
        let toml_deserialized: CustomProperty = toml::from_str(&toml_str)
            .expect("TOML should deserialize back to CustomProperty");
        prop_assert_eq!(prop, toml_deserialized, "CustomProperty should round-trip through TOML");
    }
}
