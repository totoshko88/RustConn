//! Compiled highlight-rule engine for regex-based terminal text highlighting.
//!
//! [`CompiledHighlightRules`] merges global and per-connection
//! [`HighlightRule`](crate::models::HighlightRule) sets, compiles their regex
//! patterns once, and exposes [`find_matches`](CompiledHighlightRules::find_matches)
//! to locate all matching regions in a line of terminal output.

use regex::{Regex, RegexSet};
use tracing::warn;
use uuid::Uuid;

use crate::models::HighlightRule;

// ---------------------------------------------------------------------------
// Rgb / colour parsing
// ---------------------------------------------------------------------------

/// Pre-parsed RGB colour with each channel in the `0.0..=1.0` range, ready to
/// pass straight to cairo without re-parsing on every repaint.
pub type Rgb = (f64, f64, f64);

/// Parses a CSS hex colour string (`#RRGGBB`) into [`Rgb`] floats in `0.0..=1.0`.
///
/// Returns `None` when the input is not a `#` followed by exactly six hex digits.
#[must_use]
pub fn parse_hex_color(hex: &str) -> Option<Rgb> {
    let hex = hex.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some((
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
    ))
}

// ---------------------------------------------------------------------------
// Terminal column geometry
// ---------------------------------------------------------------------------

/// Returns the number of terminal cells a character occupies: 0, 1, or 2.
///
/// A VTE terminal lays text out on a fixed grid where each cell is one
/// [`char_width`](https://gnome.pages.gitlab.gnome.org) wide. A combining mark
/// adds nothing to the cell it decorates (width 0), most characters take one
/// cell, and East-Asian wide / fullwidth characters take two. The overlay that
/// draws highlight rectangles must count in these cells, not in `char`s, or a
/// rectangle drawn after a wide character lands half a cell too far left.
///
/// This is a pragmatic approximation of Unicode UAX#11, covering the ranges that
/// actually appear in terminal output (CJK, Hangul, kana, fullwidth forms, the
/// main emoji block and common combining blocks) without pulling in a
/// Unicode-width table crate. Emoji are treated as width 2, matching how most
/// terminals render them. A multi-scalar emoji sequence (ZWJ or regional-
/// indicator pairs) is counted per scalar, and rarer wide blocks are missed, so
/// a highlight over such a glyph can still be off by a cell — the documented
/// limit.
#[must_use]
fn char_cell_width(c: char) -> usize {
    let cp = c as u32;
    // Combining marks and zero-width characters occupy no cell of their own.
    let is_zero_width = matches!(cp,
        0x0300..=0x036F   // Combining Diacritical Marks
        | 0x1AB0..=0x1AFF // Combining Diacritical Marks Extended
        | 0x1DC0..=0x1DFF // Combining Diacritical Marks Supplement
        | 0x20D0..=0x20FF // Combining Diacritical Marks for Symbols
        | 0xFE20..=0xFE2F // Combining Half Marks
        | 0x200B          // Zero Width Space
        | 0x200C..=0x200F // ZWNJ, ZWJ, LRM, RLM
        | 0xFEFF,         // Zero Width No-Break Space (BOM)
    );
    if is_zero_width {
        return 0;
    }
    // East-Asian wide and fullwidth ranges occupy two cells.
    let is_wide = matches!(cp,
        0x1100..=0x115F   // Hangul Jamo
        | 0x2E80..=0x303E // CJK Radicals, Kangxi, CJK symbols/punctuation
        | 0x3041..=0x33FF // Hiragana, Katakana, CJK symbols, enclosed
        | 0x3400..=0x4DBF // CJK Extension A
        | 0x4E00..=0x9FFF // CJK Unified Ideographs
        | 0xA000..=0xA4CF // Yi
        | 0xAC00..=0xD7A3 // Hangul Syllables
        | 0xF900..=0xFAFF // CJK Compatibility Ideographs
        | 0xFE30..=0xFE4F // CJK Compatibility Forms
        | 0xFF00..=0xFF60 // Fullwidth Forms
        | 0xFFE0..=0xFFE6 // Fullwidth signs
        | 0x1F300..=0x1FAFF // Emoji / symbols (approx.)
        | 0x20000..=0x3FFFD, // CJK Extension B+ and supplementary ideographic plane
    );
    if is_wide { 2 } else { 1 }
}

/// Converts a byte offset within `line` to its terminal column (0-based).
///
/// Sums the [`char_cell_width`] of every character before `byte_offset`, so the
/// result is the cell the character at that offset starts in — the value the
/// overlay multiplies by the cell width to place a highlight rectangle. A byte
/// offset past the end of the line clamps to the line's total column width.
///
/// Wide characters count as two columns and combining marks as zero, unlike a
/// plain `chars().count()`, which is why a match after a CJK glyph is no longer
/// drawn half a cell off (issue #343).
#[must_use]
pub fn byte_offset_to_column(line: &str, byte_offset: usize) -> usize {
    line.char_indices()
        .take_while(|(idx, _)| *idx < byte_offset)
        .map(|(_, c)| char_cell_width(c))
        .sum()
}

// ---------------------------------------------------------------------------
// HighlightMatch
// ---------------------------------------------------------------------------

/// A single highlighted region within a line of text.
///
/// Colours are pre-parsed into [`Rgb`] at compile time, so the value is `Copy`
/// and `find_matches` does not allocate on the hot repaint path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HighlightMatch {
    /// Byte offset of the match start within the line.
    pub start: usize,
    /// Byte offset of the match end (exclusive) within the line.
    pub end: usize,
    /// Optional pre-parsed foreground (text) colour.
    pub foreground_rgb: Option<Rgb>,
    /// Optional pre-parsed background colour.
    pub background_rgb: Option<Rgb>,
}

// ---------------------------------------------------------------------------
// CompiledRule (internal)
// ---------------------------------------------------------------------------

/// A single rule whose regex has been successfully compiled.
struct CompiledRule {
    regex: Regex,
    name: String,
    pattern: String,
    foreground_rgb: Option<Rgb>,
    background_rgb: Option<Rgb>,
}

// ---------------------------------------------------------------------------
// CompiledHighlightRules
// ---------------------------------------------------------------------------

/// Pre-compiled set of highlight rules ready for matching.
///
/// Created via [`compile`](Self::compile) which merges global and per-connection
/// rule lists.  Per-connection rules take priority: if a per-connection rule
/// shares the same `id` as a global rule, the per-connection version wins.
pub struct CompiledHighlightRules {
    rules: Vec<CompiledRule>,
    /// Pre-compiled `RegexSet` used to quickly determine which rules match a
    /// given line before running the individual (heavier) `Regex` objects.
    regex_set: RegexSet,
}

impl CompiledHighlightRules {
    /// Compiles global and per-connection highlight rules into a single set.
    ///
    /// Per-connection rules take priority: when a per-connection rule has the
    /// same `id` as a global rule, only the per-connection version is kept.
    /// Disabled rules and rules with invalid regex patterns are silently
    /// skipped (invalid patterns produce a `tracing::warn!`).
    ///
    /// Built-in default rules (ERROR, WARNING, CRITICAL, FATAL) are prepended
    /// to the global set so they apply unless overridden. Use
    /// [`compile_with_options`](Self::compile_with_options) to omit them.
    #[must_use]
    pub fn compile(global_rules: &[HighlightRule], per_conn_rules: &[HighlightRule]) -> Self {
        Self::compile_with_options(global_rules, per_conn_rules, true)
    }

    /// Compiles highlight rules, optionally including the built-in defaults.
    ///
    /// Identical to [`compile`](Self::compile) except that when
    /// `include_builtin_defaults` is `false` the built-in
    /// ERROR/WARNING/CRITICAL/FATAL rules are not prepended, so a user who has
    /// turned automatic highlighting off (issue #343) sees only their own
    /// rules. Per-connection rules still override globals by matching `id`.
    #[must_use]
    pub fn compile_with_options(
        global_rules: &[HighlightRule],
        per_conn_rules: &[HighlightRule],
        include_builtin_defaults: bool,
    ) -> Self {
        // Start with built-in defaults (unless suppressed), then append
        // user-supplied globals.
        let mut merged: Vec<&HighlightRule> = Vec::new();

        let defaults = builtin_defaults();
        if include_builtin_defaults {
            for rule in &defaults {
                merged.push(rule);
            }
        }
        for rule in global_rules {
            merged.push(rule);
        }

        // Per-connection rules override globals with the same id.
        let per_conn_ids: std::collections::HashSet<Uuid> =
            per_conn_rules.iter().map(|r| r.id).collect();

        merged.retain(|r| !per_conn_ids.contains(&r.id));

        for rule in per_conn_rules {
            merged.push(rule);
        }

        // Compile enabled rules; skip disabled or invalid-regex ones.
        let mut compiled = Vec::new();
        for rule in &merged {
            if !rule.enabled {
                continue;
            }
            match Regex::new(&rule.pattern) {
                Ok(regex) => {
                    compiled.push(CompiledRule {
                        regex,
                        name: rule.name.clone(),
                        pattern: rule.pattern.clone(),
                        // Parse colours once at compile time, not on every repaint.
                        foreground_rgb: rule.foreground_color.as_deref().and_then(parse_hex_color),
                        background_rgb: rule.background_color.as_deref().and_then(parse_hex_color),
                    });
                }
                Err(e) => {
                    warn!(
                        rule_name = %rule.name,
                        pattern = %rule.pattern,
                        "Skipping highlight rule with invalid regex: {e}"
                    );
                }
            }
        }

        // Build a RegexSet from the compiled patterns for fast initial filtering.
        let regex_set = RegexSet::new(compiled.iter().map(|r| r.pattern.as_str()))
            .unwrap_or_else(|_| RegexSet::empty());

        Self {
            rules: compiled,
            regex_set,
        }
    }

    /// Finds all highlight matches in the given `line`.
    ///
    /// Returns a [`Vec<HighlightMatch>`] sorted by start position.  When
    /// multiple rules match the same region the later rule in the compiled
    /// list wins (per-connection rules appear after globals).
    #[must_use]
    pub fn find_matches(&self, line: &str) -> Vec<HighlightMatch> {
        let mut matches = Vec::new();
        // Use RegexSet to quickly determine which rules match this line,
        // then only run the individual regexes for those rules.
        for idx in self.regex_set.matches(line) {
            let rule = &self.rules[idx];
            for m in rule.regex.find_iter(line) {
                matches.push(HighlightMatch {
                    start: m.start(),
                    end: m.end(),
                    foreground_rgb: rule.foreground_rgb,
                    background_rgb: rule.background_rgb,
                });
            }
        }
        matches.sort_by_key(|m| m.start);
        matches
    }

    /// Returns the source pattern strings and names of all compiled rules.
    ///
    /// Useful for registering patterns with external regex engines (e.g. VTE
    /// PCRE2) that cannot reuse the Rust [`Regex`] objects directly.
    #[must_use]
    pub fn source_patterns(&self) -> Vec<SourcePattern<'_>> {
        self.rules
            .iter()
            .map(|r| SourcePattern {
                name: &r.name,
                pattern: &r.pattern,
            })
            .collect()
    }
}

/// A borrowed view of a compiled rule's name and regex pattern.
#[derive(Debug)]
pub struct SourcePattern<'a> {
    /// Human-readable rule name.
    pub name: &'a str,
    /// The regex pattern string.
    pub pattern: &'a str,
}

// ---------------------------------------------------------------------------
// Built-in default rules
// ---------------------------------------------------------------------------

/// Returns the built-in default highlight rules.
///
/// - `ERROR`    — red foreground
/// - `WARNING`  — yellow foreground
/// - `CRITICAL` — red background
/// - `FATAL`    — red background
#[must_use]
pub fn builtin_defaults() -> Vec<HighlightRule> {
    // Deterministic UUIDs so the defaults are stable across restarts and can
    // be overridden by per-connection rules with the same id.
    let error_id = Uuid::from_bytes([
        0xBD, 0x01, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x01,
    ]);
    let warning_id = Uuid::from_bytes([
        0xBD, 0x01, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x02,
    ]);
    let critical_id = Uuid::from_bytes([
        0xBD, 0x01, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x03,
    ]);
    let fatal_id = Uuid::from_bytes([
        0xBD, 0x01, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x04,
    ]);

    vec![
        HighlightRule {
            id: error_id,
            name: "ERROR".to_string(),
            pattern: r"(?i)\bERROR\b".to_string(),
            foreground_color: Some("#FF0000".to_string()),
            background_color: None,
            enabled: true,
        },
        HighlightRule {
            id: warning_id,
            name: "WARNING".to_string(),
            pattern: r"(?i)\bWARNING\b".to_string(),
            foreground_color: Some("#FFFF00".to_string()),
            background_color: None,
            enabled: true,
        },
        // CRITICAL and FATAL are two separate rules, not one `CRITICAL|FATAL`
        // rule, on purpose: each has its own stable id, so a per-connection
        // override can retarget or disable one without touching the other. They
        // share a look today, but the split keeps that choice the user's.
        HighlightRule {
            id: critical_id,
            name: "CRITICAL".to_string(),
            pattern: r"(?i)\bCRITICAL\b".to_string(),
            foreground_color: None,
            background_color: Some("#FF0000".to_string()),
            enabled: true,
        },
        HighlightRule {
            id: fatal_id,
            name: "FATAL".to_string(),
            pattern: r"(?i)\bFATAL\b".to_string(),
            foreground_color: None,
            background_color: Some("#FF0000".to_string()),
            enabled: true,
        },
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::{byte_offset_to_column, parse_hex_color};

    #[test]
    fn parse_hex_color_valid_colors() {
        assert_eq!(parse_hex_color("#FF0000"), Some((1.0, 0.0, 0.0)));
        assert_eq!(parse_hex_color("#00FF00"), Some((0.0, 1.0, 0.0)));
        assert_eq!(parse_hex_color("#0000FF"), Some((0.0, 0.0, 1.0)));
        assert_eq!(parse_hex_color("#000000"), Some((0.0, 0.0, 0.0)));
        assert_eq!(parse_hex_color("#FFFFFF"), Some((1.0, 1.0, 1.0)));
    }

    #[test]
    fn parse_hex_color_mixed_case() {
        let (r, g, b) = parse_hex_color("#aaBBcc").unwrap();
        let expected_r = f64::from(0xAA) / 255.0;
        let expected_g = f64::from(0xBB) / 255.0;
        let expected_b = f64::from(0xCC) / 255.0;
        assert!((r - expected_r).abs() < f64::EPSILON);
        assert!((g - expected_g).abs() < f64::EPSILON);
        assert!((b - expected_b).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_hex_color_invalid_inputs() {
        assert_eq!(parse_hex_color("FF0000"), None); // missing hash
        assert_eq!(parse_hex_color("#FFF"), None); // too short
        assert_eq!(parse_hex_color("#FF000000"), None); // too long
        assert_eq!(parse_hex_color("#GGHHII"), None); // invalid hex chars
        assert_eq!(parse_hex_color(""), None); // empty
        assert_eq!(parse_hex_color("#"), None); // only hash
    }

    #[test]
    fn byte_offset_to_column_ascii() {
        let line = "ERROR: disk full";
        // One byte per character, so column == byte offset.
        assert_eq!(byte_offset_to_column(line, 0), 0);
        assert_eq!(byte_offset_to_column(line, 5), 5); // just past "ERROR"
        assert_eq!(byte_offset_to_column(line, line.len()), 16);
    }

    #[test]
    fn byte_offset_to_column_past_end_clamps_to_width() {
        let line = "abc";
        // An offset beyond the line clamps to its total column width.
        assert_eq!(byte_offset_to_column(line, 99), 3);
    }

    #[test]
    fn byte_offset_to_column_wide_chars_count_two() {
        // "世" and "界" are CJK ideographs: 3 bytes each, 2 columns each.
        let line = "世界x";
        assert_eq!(line.len(), 7); // 3 + 3 + 1 bytes
        assert_eq!(byte_offset_to_column(line, 0), 0);
        assert_eq!(byte_offset_to_column(line, 3), 2); // after first ideograph
        assert_eq!(byte_offset_to_column(line, 6), 4); // after second ideograph
        assert_eq!(byte_offset_to_column(line, 7), 5); // after the ASCII 'x'
    }

    #[test]
    fn byte_offset_to_column_combining_marks_count_zero() {
        // 'e' (1 byte) followed by U+0301 combining acute accent (2 bytes):
        // the mark adds no column of its own.
        let line = "e\u{0301}x";
        assert_eq!(line.len(), 4); // 1 + 2 + 1 bytes
        assert_eq!(byte_offset_to_column(line, 1), 1); // after 'e'
        assert_eq!(byte_offset_to_column(line, 3), 1); // after the combining mark
        assert_eq!(byte_offset_to_column(line, 4), 2); // after 'x'
    }

    #[test]
    fn byte_offset_to_column_empty_line() {
        assert_eq!(byte_offset_to_column("", 0), 0);
    }
}
