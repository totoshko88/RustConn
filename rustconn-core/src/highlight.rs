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
    /// Built-in default rules (ERROR, WARNING, CRITICAL, FATAL) are always
    /// prepended to the global set so they apply unless overridden.
    #[must_use]
    pub fn compile(global_rules: &[HighlightRule], per_conn_rules: &[HighlightRule]) -> Self {
        // Start with built-in defaults, then append user-supplied globals.
        let mut merged: Vec<&HighlightRule> = Vec::new();

        let defaults = builtin_defaults();
        for rule in &defaults {
            merged.push(rule);
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
    use super::parse_hex_color;

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
}
