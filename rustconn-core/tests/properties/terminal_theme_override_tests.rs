//! Property-based tests for `ConnectionThemeOverride` validation and serde round-trip.

use proptest::prelude::*;
use rustconn_core::models::ConnectionThemeOverride;

// ---------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------

/// Generates a valid 6-digit hex color string: `#[0-9a-fA-F]{6}`
fn valid_hex_color() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::sample::select(vec![
            '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f', 'A',
            'B', 'C', 'D', 'E', 'F',
        ]),
        6..=6,
    )
    .prop_map(|chars| format!("#{}", chars.into_iter().collect::<String>()))
}

/// Generates an invalid color string that should fail validation.
///
/// Categories:
/// - Missing `#` prefix
/// - Wrong length (too short / too long, but not 6 or 8 hex digits after `#`)
/// - Contains non-hex characters after `#`
fn invalid_color_string() -> impl Strategy<Value = String> {
    prop_oneof![
        // No '#' prefix — 6 hex chars without leading hash
        prop::collection::vec(
            prop::sample::select(vec![
                '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f',
            ]),
            6..=6,
        )
        .prop_map(|chars| chars.into_iter().collect::<String>()),
        // Wrong length — too short (1-5 hex digits after #)
        (1usize..=5).prop_flat_map(|len| {
            prop::collection::vec(
                prop::sample::select(vec![
                    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f',
                ]),
                len..=len,
            )
            .prop_map(|chars| format!("#{}", chars.into_iter().collect::<String>()))
        }),
        // Wrong length — 7 hex digits after # (not 6 or 8)
        prop::collection::vec(
            prop::sample::select(vec![
                '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f',
            ]),
            7..=7,
        )
        .prop_map(|chars| format!("#{}", chars.into_iter().collect::<String>())),
        // Non-hex characters after # with correct length
        "[#][g-zG-Z]{6}",
    ]
}

/// Generates a `ConnectionThemeOverride` with all valid hex colors.
fn valid_theme_override() -> impl Strategy<Value = ConnectionThemeOverride> {
    (
        prop::option::of(valid_hex_color()),
        prop::option::of(valid_hex_color()),
        prop::option::of(valid_hex_color()),
    )
        .prop_map(|(bg, fg, cur)| ConnectionThemeOverride {
            background: bg,
            foreground: fg,
            cursor: cur,
        })
}

// ---------------------------------------------------------------------------
// Proptest 1: Any valid hex `#[0-9a-fA-F]{6}` passes validation
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn valid_hex_passes_validation(color in valid_hex_color()) {
        let theme = ConnectionThemeOverride {
            background: Some(color.clone()),
            foreground: Some(color.clone()),
            cursor: Some(color),
        };
        prop_assert!(theme.validate().is_ok());
    }
}

// ---------------------------------------------------------------------------
// Proptest 2: Invalid string (no #, wrong length, invalid chars) fails
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn invalid_string_fails_validation(bad in invalid_color_string()) {
        // Place the invalid color in each field individually — all must fail
        let theme_bg = ConnectionThemeOverride {
            background: Some(bad.clone()),
            foreground: None,
            cursor: None,
        };
        prop_assert!(theme_bg.validate().is_err());

        let theme_fg = ConnectionThemeOverride {
            background: None,
            foreground: Some(bad.clone()),
            cursor: None,
        };
        prop_assert!(theme_fg.validate().is_err());

        let theme_cur = ConnectionThemeOverride {
            background: None,
            foreground: None,
            cursor: Some(bad),
        };
        prop_assert!(theme_cur.validate().is_err());
    }
}

// ---------------------------------------------------------------------------
// Proptest 3: Serde round-trip — serialize → deserialize = identical result
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn serde_roundtrip_preserves_theme(theme in valid_theme_override()) {
        let json = serde_json::to_string(&theme).map_err(|e| {
            TestCaseError::fail(format!("serialization failed: {e}"))
        })?;
        let restored: ConnectionThemeOverride = serde_json::from_str(&json).map_err(|e| {
            TestCaseError::fail(format!("deserialization failed: {e}"))
        })?;
        prop_assert_eq!(&theme, &restored);
    }
}
