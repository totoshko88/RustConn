//! Property-based tests for highlight rules: regex validation, serde round-trip,
//! match-position correctness, and the viewport rows the overlay draws them on.

use proptest::prelude::*;
use rustconn_core::highlight::{CompiledHighlightRules, builtin_defaults, viewport_rows};
use rustconn_core::models::HighlightRule;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------

/// Generates a syntactically valid regex pattern (simple literal or character class).
fn valid_regex_pattern() -> impl Strategy<Value = String> {
    prop_oneof![
        // Simple word literals — always valid regex
        "[a-zA-Z]{1,12}",
        // Word boundary patterns like \bWORD\b
        "[a-zA-Z]{1,8}".prop_map(|w| format!(r"\b{w}\b")),
        // Character class patterns
        Just("[a-z]+".to_string()),
        Just("[0-9]+".to_string()),
        Just("(?i)error".to_string()),
        Just(r"\d{1,5}".to_string()),
    ]
}

/// Generates an invalid regex pattern that will fail `Regex::new()`.
fn invalid_regex_pattern() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("(unclosed".to_string()),
        Just("[unclosed".to_string()),
        Just("*leading_star".to_string()),
        Just(r"\p{BadUnicode}".to_string()),
        Just(")bad".to_string()),
    ]
}

/// Optional CSS hex color (#RRGGBB).
fn optional_hex_color() -> impl Strategy<Value = Option<String>> {
    prop::option::of(
        prop::collection::vec(
            prop::sample::select(vec![
                '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f',
            ]),
            6..=6,
        )
        .prop_map(|chars| format!("#{}", chars.into_iter().collect::<String>())),
    )
}

/// Generates a `HighlightRule` with a valid regex pattern.
fn valid_highlight_rule() -> impl Strategy<Value = HighlightRule> {
    (
        "[a-zA-Z0-9 _-]{1,20}", // name
        valid_regex_pattern(),
        optional_hex_color(),
        optional_hex_color(),
        any::<bool>(),
    )
        .prop_map(|(name, pattern, fg, bg, enabled)| HighlightRule {
            id: Uuid::new_v4(),
            name,
            pattern,
            foreground_color: fg,
            background_color: bg,
            enabled,
        })
}

// ---------------------------------------------------------------------------
// Proptest 14: Valid regex passes validate_pattern(), invalid does not
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn valid_regex_passes_validate_pattern(pattern in valid_regex_pattern()) {
        let rule = HighlightRule {
            id: Uuid::new_v4(),
            name: "test".to_string(),
            pattern,
            foreground_color: None,
            background_color: None,
            enabled: true,
        };
        prop_assert!(rule.validate_pattern().is_ok());
    }

    #[test]
    fn invalid_regex_fails_validate_pattern(pattern in invalid_regex_pattern()) {
        let rule = HighlightRule {
            id: Uuid::new_v4(),
            name: "test".to_string(),
            pattern,
            foreground_color: None,
            background_color: None,
            enabled: true,
        };
        prop_assert!(rule.validate_pattern().is_err());
    }
}

// ---------------------------------------------------------------------------
// Proptest 15: Serde round-trip for HighlightRule
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn serde_roundtrip_preserves_highlight_rule(rule in valid_highlight_rule()) {
        let json = serde_json::to_string(&rule).map_err(|e| {
            TestCaseError::fail(format!("serialization failed: {e}"))
        })?;
        let restored: HighlightRule = serde_json::from_str(&json).map_err(|e| {
            TestCaseError::fail(format!("deserialization failed: {e}"))
        })?;
        prop_assert_eq!(&rule.id, &restored.id);
        prop_assert_eq!(&rule.name, &restored.name);
        prop_assert_eq!(&rule.pattern, &restored.pattern);
        prop_assert_eq!(&rule.foreground_color, &restored.foreground_color);
        prop_assert_eq!(&rule.background_color, &restored.background_color);
        prop_assert_eq!(rule.enabled, restored.enabled);
    }
}

// ---------------------------------------------------------------------------
// Proptest 16: Matching positions are correct for known patterns
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn matching_positions_correct_for_literal(
        prefix in "[a-z]{0,10}",
        keyword in "[A-Z]{1,6}",
        suffix in "[a-z]{0,10}",
    ) {
        let line = format!("{prefix}{keyword}{suffix}");

        let rule = HighlightRule {
            id: Uuid::new_v4(),
            name: "literal".to_string(),
            pattern: keyword.clone(),
            foreground_color: Some("#FF0000".to_string()),
            background_color: None,
            enabled: true,
        };

        let compiled = CompiledHighlightRules::compile(&[], &[rule]);
        let matches = compiled.find_matches(&line);

        // The keyword must appear at least once (it's a literal substring)
        prop_assert!(!matches.is_empty(), "expected at least one match in '{line}'");

        for m in &matches {
            // Byte offsets must be within the line
            prop_assert!(m.start <= m.end);
            prop_assert!(m.end <= line.len());
            // The matched slice must equal the keyword
            prop_assert_eq!(&line[m.start..m.end], keyword.as_str());
        }
    }
}

// ---------------------------------------------------------------------------
// Proptest 17: Viewport rows follow VTE's whole-pixel scrolling (issue #343)
// ---------------------------------------------------------------------------

proptest! {
    /// Whatever the scroll position and cell height, the first row is shifted
    /// up by less than one row, the partly visible bottom row is added exactly
    /// when there is a shift, and the grid's top lands on the pixel VTE
    /// scrolled to.
    #[test]
    fn viewport_rows_shift_stays_within_one_row(
        value in 0.0f64..1e6,
        cell_height in 1.0f64..64.0,
        row_count in 1i64..200,
    ) {
        let shown = viewport_rows(value, cell_height, row_count);

        prop_assert!(
            (0.0..cell_height).contains(&shown.y_offset),
            "y_offset {} is outside 0..{cell_height}",
            shown.y_offset
        );
        prop_assert!(shown.rows == row_count || shown.rows == row_count + 1);
        prop_assert_eq!(shown.rows == row_count, shown.y_offset == 0.0);

        let grid_top = shown.first_row as f64 * cell_height + shown.y_offset;
        let vte_top = (value * cell_height).round();
        prop_assert!(
            (grid_top - vte_top).abs() <= 1e-6 * vte_top.max(1.0),
            "grid top at {grid_top} px, VTE scrolled to {vte_top} px"
        );
    }

    /// With the whole-pixel cell heights VTE reports the arithmetic is exact:
    /// the grid's top is the rounded scroll offset, and the first row is the
    /// scroll value's whole rows, or one more once the view is within half a
    /// pixel of the next row, which the rounding then lands on.
    #[test]
    fn viewport_rows_matches_vte_for_whole_pixel_cells(
        value in 0.0f64..1e6,
        cell_px in 1u8..=64,
        row_count in 1i64..200,
    ) {
        let cell_height = f64::from(cell_px);
        let shown = viewport_rows(value, cell_height, row_count);

        prop_assert!(shown.y_offset.fract() == 0.0, "offset {} px", shown.y_offset);
        let vte_top = (value * cell_height).round() as i64;
        prop_assert_eq!(
            shown.first_row * i64::from(cell_px) + shown.y_offset as i64,
            vte_top
        );

        // How far into its row the view rests, and where VTE's rounding tips
        // over onto the next row. A margin either side keeps float noise in
        // `value * cell_height` out of the comparison.
        let whole_rows = value.floor() as i64;
        let into_row = value.fract() * cell_height;
        let tips_over_at = cell_height - 0.5;
        if into_row < tips_over_at - 1e-6 {
            prop_assert_eq!(shown.first_row, whole_rows);
        } else if into_row > tips_over_at + 1e-6 {
            prop_assert_eq!(shown.first_row, whole_rows + 1);
            prop_assert!(shown.y_offset == 0.0, "offset {} px", shown.y_offset);
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[test]
fn builtin_defaults_all_have_valid_patterns() {
    let defaults = builtin_defaults();
    for rule in &defaults {
        assert!(
            rule.validate_pattern().is_ok(),
            "built-in rule '{}' has invalid pattern '{}'",
            rule.name,
            rule.pattern,
        );
    }
}

#[test]
fn disabled_rule_produces_no_matches() {
    let rule = HighlightRule {
        id: Uuid::new_v4(),
        name: "disabled".to_string(),
        pattern: "XYZZY".to_string(),
        foreground_color: Some("#FF0000".to_string()),
        background_color: None,
        enabled: false,
    };
    let compiled = CompiledHighlightRules::compile(&[], &[rule]);
    let matches = compiled.find_matches("this line has XYZZY in it");
    // Only built-in defaults should match; none of them match "XYZZY"
    assert!(
        matches.iter().all(|m| {
            let slice = &"this line has XYZZY in it"[m.start..m.end];
            slice != "XYZZY"
        }),
        "disabled rule should produce no matches for its pattern",
    );
}

#[test]
fn invalid_regex_rule_is_skipped_in_compile() {
    let bad_rule = HighlightRule {
        id: Uuid::new_v4(),
        name: "bad".to_string(),
        pattern: "(unclosed".to_string(),
        foreground_color: None,
        background_color: None,
        enabled: true,
    };
    let good_rule = HighlightRule {
        id: Uuid::new_v4(),
        name: "good".to_string(),
        pattern: "OK".to_string(),
        foreground_color: Some("#00FF00".to_string()),
        background_color: None,
        enabled: true,
    };
    let compiled = CompiledHighlightRules::compile(&[bad_rule, good_rule], &[]);
    let matches = compiled.find_matches("OK here");
    // The good rule should still match despite the bad rule being skipped
    assert!(!matches.is_empty());
    assert_eq!(matches[0].start, 0);
    assert_eq!(matches[0].end, 2);
}

#[test]
fn foreground_hex_color_resolves_to_rgb() {
    // A #RRGGBB foreground colour must round-trip into a parsed RGB on the
    // match, otherwise the overlay has nothing to draw (issue #343).
    let rule = HighlightRule {
        id: Uuid::new_v4(),
        name: "info".to_string(),
        pattern: r"(?i)\bINFO\b".to_string(),
        foreground_color: Some("#0000FF".to_string()),
        background_color: None,
        enabled: true,
    };
    let compiled = CompiledHighlightRules::compile(&[], &[rule]);
    let matches = compiled.find_matches("service INFO ready");
    let info = matches
        .iter()
        .find(|m| m.foreground_rgb.is_some())
        .expect("the INFO rule should produce a foreground match");
    assert_eq!(info.foreground_rgb, Some((0.0, 0.0, 1.0)));
    assert_eq!(info.background_rgb, None);
}

#[test]
fn background_hex_color_resolves_to_rgb() {
    let rule = HighlightRule {
        id: Uuid::new_v4(),
        name: "hit".to_string(),
        pattern: "HIT".to_string(),
        foreground_color: None,
        background_color: Some("#FF0000".to_string()),
        enabled: true,
    };
    let compiled = CompiledHighlightRules::compile(&[], &[rule]);
    let m = compiled
        .find_matches("a HIT here")
        .into_iter()
        .find(|m| m.background_rgb.is_some())
        .expect("the HIT rule should produce a background match");
    assert_eq!(m.background_rgb, Some((1.0, 0.0, 0.0)));
}

#[test]
fn invalid_color_leaves_rgb_unset_but_still_matches() {
    // A bracket-RGB literal from another tool ("[0,0,255]") is not a valid
    // #RRGGBB colour, so no RGB is produced — but the rule still matches its
    // pattern rather than crashing (issue #343).
    let rule = HighlightRule {
        id: Uuid::new_v4(),
        name: "bad-color".to_string(),
        pattern: "INFO".to_string(),
        foreground_color: Some("[0,0,255]".to_string()),
        background_color: None,
        enabled: true,
    };
    let compiled = CompiledHighlightRules::compile(&[], &[rule]);
    let m = compiled
        .find_matches("INFO line")
        .into_iter()
        .find(|m| {
            // The literal "INFO" match, not a built-in default.
            m.start == 0 && m.end == 4
        })
        .expect("the pattern should still match");
    assert_eq!(m.foreground_rgb, None);
}

#[test]
fn compile_without_builtin_defaults_drops_error_and_warning() {
    // With built-ins suppressed and no user rules, nothing matches ERROR /
    // WARNING (issue #343).
    let compiled = CompiledHighlightRules::compile_with_options(&[], &[], false);
    let matches = compiled.find_matches("ERROR and WARNING here");
    assert!(
        matches.is_empty(),
        "no built-in defaults should apply when suppressed, got {matches:?}"
    );
}

#[test]
fn compile_with_builtin_defaults_still_matches_error() {
    let compiled = CompiledHighlightRules::compile_with_options(&[], &[], true);
    let matches = compiled.find_matches("ERROR here");
    assert!(
        !matches.is_empty(),
        "the built-in ERROR default should match when enabled"
    );
    // And the built-in ERROR rule carries a red foreground.
    assert!(matches.iter().any(|m| m.foreground_rgb.is_some()));
    // Sanity: builtin_defaults still exposes four rules.
    assert_eq!(builtin_defaults().len(), 4);
}
