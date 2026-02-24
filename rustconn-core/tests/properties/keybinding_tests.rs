//! Property tests for keybinding configuration

use proptest::prelude::*;
use rustconn_core::config::keybindings::{
    KeybindingCategory, KeybindingSettings, default_keybindings, is_valid_accelerator,
};

/// Strategy for generating valid GTK accelerator strings
fn accel_strategy() -> impl Strategy<Value = String> {
    prop::sample::select(vec![
        "<Control>a".to_owned(),
        "<Control><Shift>b".to_owned(),
        "<Alt>F2".to_owned(),
        "<Control><Alt>Delete".to_owned(),
        "F1".to_owned(),
        "F11".to_owned(),
        "<Control>comma".to_owned(),
        "<Control>grave".to_owned(),
        "<Control><Shift>h".to_owned(),
        "<Control>Tab".to_owned(),
    ])
}

/// Strategy for generating action names from the default registry
fn action_strategy() -> impl Strategy<Value = String> {
    let actions: Vec<String> = default_keybindings()
        .iter()
        .map(|d| d.action.clone())
        .collect();
    prop::sample::select(actions)
}

proptest! {
    /// Overriding a keybinding and then resetting it returns to default
    #[test]
    fn override_then_reset_returns_default(
        action in action_strategy(),
        accel in accel_strategy(),
    ) {
        let mut settings = KeybindingSettings::default();
        let defaults = default_keybindings();
        let def = defaults.iter().find(|d| d.action == action).unwrap();

        let original = settings.get_accel(def).to_owned();
        settings.overrides.insert(action.clone(), accel.clone());
        prop_assert_eq!(settings.get_accel(def), accel.as_str());

        settings.reset(&action);
        prop_assert_eq!(settings.get_accel(def), original.as_str());
    }

    /// Serialization round-trip preserves overrides
    #[test]
    fn serde_roundtrip_preserves_overrides(
        action in action_strategy(),
        accel in accel_strategy(),
    ) {
        let mut settings = KeybindingSettings::default();
        settings.overrides.insert(action.clone(), accel.clone());

        let json = serde_json::to_string(&settings).unwrap();
        let deserialized: KeybindingSettings = serde_json::from_str(&json).unwrap();

        prop_assert_eq!(&settings, &deserialized);
        prop_assert_eq!(deserialized.overrides.get(&action).unwrap(), &accel);
    }

    /// reset_all clears all overrides regardless of count
    #[test]
    fn reset_all_clears_all(
        overrides in prop::collection::hash_map(
            action_strategy(),
            accel_strategy(),
            0..10,
        ),
    ) {
        let mut settings = KeybindingSettings { overrides };
        settings.reset_all();
        prop_assert!(settings.overrides.is_empty());
        prop_assert!(!settings.has_overrides());
    }

    /// All default accelerators pass validation
    #[test]
    fn all_defaults_valid(_dummy in 0..1u8) {
        for def in default_keybindings() {
            for accel in def.default_accel_list() {
                prop_assert!(
                    is_valid_accelerator(accel),
                    "Invalid default accelerator '{}' for '{}'",
                    accel,
                    def.action,
                );
            }
        }
    }

    /// Empty string is not a valid accelerator
    #[test]
    fn empty_string_invalid(_dummy in 0..1u8) {
        prop_assert!(!is_valid_accelerator(""));
    }

    /// Strings ending with '>' (bare modifier) are invalid
    #[test]
    fn bare_modifier_invalid(
        modifier in prop::sample::select(vec![
            "<Control>".to_owned(),
            "<Shift>".to_owned(),
            "<Alt>".to_owned(),
            "<Super>".to_owned(),
            "<Control><Shift>".to_owned(),
        ]),
    ) {
        prop_assert!(!is_valid_accelerator(&modifier));
    }

    /// Every category has at least one default binding
    #[test]
    fn every_category_has_bindings(_dummy in 0..1u8) {
        let defs = default_keybindings();
        for cat in KeybindingCategory::all() {
            prop_assert!(
                defs.iter().any(|d| d.category == *cat),
                "Category {:?} has no bindings",
                cat,
            );
        }
    }

    /// All action names in defaults are unique
    #[test]
    fn action_names_unique(_dummy in 0..1u8) {
        let defs = default_keybindings();
        let mut seen = std::collections::HashSet::new();
        for def in &defs {
            prop_assert!(
                seen.insert(&def.action),
                "Duplicate action: {}",
                def.action,
            );
        }
    }
}
