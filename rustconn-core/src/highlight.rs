//! Compiled highlight-rule engine for regex-based terminal text highlighting.
//!
//! [`CompiledHighlightRules`] merges global and per-connection
//! [`HighlightRule`] sets, compiles their regex
//! patterns once, and exposes [`find_matches`](CompiledHighlightRules::find_matches)
//! to locate all matching regions in a line of terminal output.
//!
//! [`byte_offset_to_column`] and [`viewport_rows`] are the grid geometry the
//! terminal overlay places those regions with: the cell a match starts in, and
//! the rows on screen together with the pixel offset VTE draws them at.

use regex::{Regex, RegexSet};
use tracing::warn;
use uuid::Uuid;

use crate::models::HighlightRule;
use crate::terminal_themes::parse_hex_channels;

// ---------------------------------------------------------------------------
// Rgb / colour parsing
// ---------------------------------------------------------------------------

/// Pre-parsed RGB colour with each channel in the `0.0..=1.0` range, ready to
/// pass straight to cairo without re-parsing on every repaint.
pub type Rgb = (f64, f64, f64);

/// Parses a CSS hex colour string (`#RRGGBB`) into [`Rgb`] floats in `0.0..=1.0`.
///
/// Returns `None` when the input is not a `#` followed by exactly six hex digits.
/// The value comes straight from a rule editor's text field and is compiled on
/// every terminal session start, so it goes through
/// [`parse_hex_channels`], the parser every colour field shares, which refuses a
/// multi-byte character instead of panicking on it (issue #343).
#[must_use]
pub fn parse_hex_color(hex: &str) -> Option<Rgb> {
    let digits = hex.strip_prefix('#')?;
    // Six digits only: a highlight colour has no alpha channel.
    if digits.len() != 6 {
        return None;
    }
    let [r, g, b, _] = parse_hex_channels(digits)?;
    Some((
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
    ))
}

/// Normalises the text of a rule editor's colour field into the stored value.
///
/// Surrounding whitespace is dropped and an empty field means "no colour"
/// (`None`). Anything else is kept verbatim, valid or not, so a value the user is
/// still typing survives a save and reopens as typed; [`is_valid_color_input`]
/// is what the editor uses to flag it.
#[must_use]
pub fn normalize_color_input(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Checks a rule pattern with the regex engine [`CompiledHighlightRules::compile`] uses.
///
/// An invalid pattern is skipped at compile time with nothing but a log line, so
/// the rule editors call this to flag it while the user types. An empty pattern
/// compiles; the editors treat it as not filled in yet.
///
/// # Errors
///
/// Returns the regex engine's error when `pattern` does not compile.
pub fn validate_pattern(pattern: &str) -> Result<(), regex::Error> {
    Regex::new(pattern).map(|_| ())
}

/// Whether a rule editor's colour field holds something usable.
///
/// An empty field is valid (it means "no colour"); otherwise the trimmed text
/// must be a `#RRGGBB` value that [`parse_hex_color`] accepts. A rule with an
/// invalid colour still matches, it just draws nothing for that colour, so the
/// editor has to say so — silently drawing nothing is how issue #343 started.
#[must_use]
pub fn is_valid_color_input(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.is_empty() || parse_hex_color(trimmed).is_some()
}

// ---------------------------------------------------------------------------
// Terminal column geometry
// ---------------------------------------------------------------------------

/// Distance between VTE's default tab stops, in cells.
///
/// VTE sets a tab stop every eight columns and a program only moves them with
/// the rarely used HTS/TBC escape sequences, so eight is what a tab in ordinary
/// output means.
const TAB_STOP_WIDTH: usize = 8;

/// Returns the number of terminal cells a character occupies: 0, 1, or 2.
///
/// A VTE terminal lays text out on a fixed grid, and VTE sizes each character
/// with glib: `g_unichar_iszerowidth` makes it zero cells and
/// `g_unichar_iswide` — East-Asian Width W or F — two; everything else takes
/// one. The overlay that draws highlight rectangles has to count the same
/// cells, not `char`s, or a rectangle after a wide character lands a whole cell
/// too far left for every such character before it.
///
/// The wide set is exact: [`WIDE_RANGES`] is every W and F range of Unicode's
/// `EastAsianWidth.txt`, so the `✅` and `❌` that build and test tools print
/// count two cells, as VTE draws them. The zero-width set covers the combining
/// blocks, variation selectors, tags and format characters that terminal
/// output carries — `⚠️` is U+26A0 followed by the variation selector U+FE0F,
/// one cell in VTE — but not the combining marks of every script, so a line of,
/// say, Devanagari can still put a highlight a cell off; that is the documented
/// limit. A multi-scalar emoji sequence is counted per scalar, as VTE counts it.
#[must_use]
fn char_cell_width(c: char) -> usize {
    let cp = u32::from(c);
    // Combining marks and zero-width characters occupy no cell of their own.
    let is_zero_width = matches!(cp,
        0x0300..=0x036F     // Combining Diacritical Marks
        | 0x1160..=0x11FF   // Hangul Jamo medial vowels and final consonants
        | 0x1AB0..=0x1AFF   // Combining Diacritical Marks Extended
        | 0x1DC0..=0x1DFF   // Combining Diacritical Marks Supplement
        | 0x200B..=0x200F   // ZWSP, ZWNJ, ZWJ, LRM, RLM
        | 0x202A..=0x202E   // Bidirectional embeddings and overrides
        | 0x2060..=0x2064   // Word joiner and invisible operators
        | 0x2066..=0x206F   // Bidirectional isolates, deprecated format controls
        | 0x20D0..=0x20FF   // Combining Diacritical Marks for Symbols
        | 0xD7B0..=0xD7FF   // Hangul Jamo Extended-B
        | 0xFE00..=0xFE0F   // Variation selectors; U+FE0F asks for the emoji form
        | 0xFE20..=0xFE2F   // Combining Half Marks
        | 0xFEFF            // Zero Width No-Break Space (BOM)
        | 0xE0001           // Language tag
        | 0xE0020..=0xE007F // Tag characters, as in subdivision flags
        | 0xE0100..=0xE01EF, // Variation Selectors Supplement
    );
    if is_zero_width {
        return 0;
    }
    let is_wide = WIDE_RANGES
        .binary_search_by(|&(first, last)| {
            if last < cp {
                std::cmp::Ordering::Less
            } else if cp < first {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok();
    if is_wide { 2 } else { 1 }
}

/// Every East-Asian Width `W` and `F` range of Unicode 18.0's
/// `EastAsianWidth.txt`, with touching ranges merged: the set
/// `g_unichar_iswide` answers for, and so the characters VTE draws two cells
/// wide.
///
/// To refresh it, take every `W` and `F` line of a newer `EastAsianWidth.txt`
/// and merge ranges that touch. [`char_cell_width`] binary-searches it, so it
/// must stay sorted and disjoint, which a test checks.
const WIDE_RANGES: &[(u32, u32)] = &[
    (0x1100, 0x115F),
    (0x231A, 0x231B),
    (0x2329, 0x232A),
    (0x23E9, 0x23EC),
    (0x23F0, 0x23F0),
    (0x23F3, 0x23F3),
    (0x25FD, 0x25FE),
    (0x2614, 0x2615),
    (0x2630, 0x2637),
    (0x2648, 0x2653),
    (0x267F, 0x267F),
    (0x268A, 0x268F),
    (0x2693, 0x2693),
    (0x26A1, 0x26A1),
    (0x26AA, 0x26AB),
    (0x26BD, 0x26BE),
    (0x26C4, 0x26C5),
    (0x26CE, 0x26CE),
    (0x26D4, 0x26D4),
    (0x26EA, 0x26EA),
    (0x26F2, 0x26F3),
    (0x26F5, 0x26F5),
    (0x26FA, 0x26FA),
    (0x26FD, 0x26FD),
    (0x2705, 0x2705),
    (0x270A, 0x270B),
    (0x2728, 0x2728),
    (0x274C, 0x274C),
    (0x274E, 0x274E),
    (0x2753, 0x2755),
    (0x2757, 0x2757),
    (0x2795, 0x2797),
    (0x27B0, 0x27B0),
    (0x27BF, 0x27BF),
    (0x2B1B, 0x2B1C),
    (0x2B50, 0x2B50),
    (0x2B55, 0x2B55),
    (0x2E80, 0x2E99),
    (0x2E9B, 0x2EF3),
    (0x2F00, 0x2FD5),
    (0x2FF0, 0x303E),
    (0x3041, 0x3096),
    (0x3099, 0x30FF),
    (0x3105, 0x312F),
    (0x3131, 0x318E),
    (0x3190, 0x31E5),
    (0x31EF, 0x321E),
    (0x3220, 0x3247),
    (0x3250, 0xA48C),
    (0xA490, 0xA4C6),
    (0xA960, 0xA97C),
    (0xAC00, 0xD7A3),
    (0xF900, 0xFAFF),
    (0xFE10, 0xFE19),
    (0xFE30, 0xFE52),
    (0xFE54, 0xFE66),
    (0xFE68, 0xFE6B),
    (0xFF01, 0xFF60),
    (0xFFE0, 0xFFE6),
    (0x16FE0, 0x16FE4),
    (0x16FF0, 0x16FF6),
    (0x17000, 0x18CDA),
    (0x18CFF, 0x18D20),
    (0x18D80, 0x18DF2),
    (0x18E00, 0x19191),
    (0x191A0, 0x191D2),
    (0x1AFF0, 0x1AFF3),
    (0x1AFF5, 0x1AFFB),
    (0x1AFFD, 0x1AFFE),
    (0x1B000, 0x1B128),
    (0x1B132, 0x1B132),
    (0x1B150, 0x1B152),
    (0x1B155, 0x1B155),
    (0x1B164, 0x1B168),
    (0x1B170, 0x1B2FB),
    (0x1D300, 0x1D356),
    (0x1D360, 0x1D376),
    (0x1F004, 0x1F004),
    (0x1F0CF, 0x1F0CF),
    (0x1F18E, 0x1F18E),
    (0x1F191, 0x1F19A),
    (0x1F1AE, 0x1F1AE),
    (0x1F200, 0x1F202),
    (0x1F210, 0x1F23B),
    (0x1F240, 0x1F248),
    (0x1F250, 0x1F251),
    (0x1F260, 0x1F265),
    (0x1F300, 0x1F320),
    (0x1F32D, 0x1F335),
    (0x1F337, 0x1F37C),
    (0x1F37E, 0x1F393),
    (0x1F3A0, 0x1F3CA),
    (0x1F3CF, 0x1F3D3),
    (0x1F3E0, 0x1F3F0),
    (0x1F3F4, 0x1F3F4),
    (0x1F3F8, 0x1F43E),
    (0x1F440, 0x1F440),
    (0x1F442, 0x1F4FC),
    (0x1F4FF, 0x1F53D),
    (0x1F54B, 0x1F54E),
    (0x1F550, 0x1F567),
    (0x1F57A, 0x1F57A),
    (0x1F595, 0x1F596),
    (0x1F5A4, 0x1F5A4),
    (0x1F5FB, 0x1F64F),
    (0x1F680, 0x1F6C5),
    (0x1F6CC, 0x1F6CC),
    (0x1F6D0, 0x1F6D2),
    (0x1F6D5, 0x1F6D9),
    (0x1F6DC, 0x1F6DF),
    (0x1F6EB, 0x1F6EC),
    (0x1F6F4, 0x1F6FC),
    (0x1F7DA, 0x1F7DA),
    (0x1F7E0, 0x1F7EB),
    (0x1F7F0, 0x1F7F0),
    (0x1F90C, 0x1F93A),
    (0x1F93C, 0x1F945),
    (0x1F947, 0x1F9FF),
    (0x1FA70, 0x1FA7C),
    (0x1FA80, 0x1FAC6),
    (0x1FAC8, 0x1FAC8),
    (0x1FACC, 0x1FADD),
    (0x1FADF, 0x1FAEB),
    (0x1FAEF, 0x1FAFA),
    (0x20000, 0x2FFFD),
    (0x30000, 0x3FFFD),
];

/// Converts a byte offset within `line` to its terminal column (0-based).
///
/// Walks every character before `byte_offset`, so the result is the cell the
/// character at that offset starts in — the value the overlay multiplies by the
/// cell width to place a highlight rectangle. A byte offset past the end of the
/// line clamps to the line's total column width.
///
/// Wide characters count as two columns and combining marks as zero (see
/// [`char_cell_width`]), unlike a plain `chars().count()`, which is why a match
/// after a CJK glyph is no longer drawn a cell further left for every wide
/// character before it (issue #343).
///
/// A tab advances to the next tab stop. VTE keeps a tab written at the end of a
/// line as a single `'\t'` cell spanning up to that stop, and its text export
/// returns that `'\t'` once, so counting it as one column put every match after
/// a tab — `grep` over indented code, a Java stack trace's `\tat` — up to seven
/// cells too far left.
#[must_use]
pub fn byte_offset_to_column(line: &str, byte_offset: usize) -> usize {
    let mut column = 0;
    for (idx, c) in line.char_indices() {
        if byte_offset <= idx {
            break;
        }
        column = if c == '\t' {
            (column / TAB_STOP_WIDTH + 1) * TAB_STOP_WIDTH
        } else {
            column + char_cell_width(c)
        };
    }
    column
}

// ---------------------------------------------------------------------------
// Terminal row geometry
// ---------------------------------------------------------------------------

/// The furthest scroll position [`viewport_rows`] takes: the last row an `i64`,
/// VTE's own row type, can number. Capping there keeps the pixel offset finite
/// for any real cell height.
const MAX_SCROLL_POSITION: f64 = i64::MAX as f64;

/// The buffer rows a VTE terminal shows, and how far the first one is scrolled
/// past the top of its character grid.
///
/// Returned by [`viewport_rows`]. Visible row `k`, counting from 0, is buffer row
/// `first_row + k`, and its top edge lies `k * cell_height - y_offset` pixels
/// below the top of the grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportRows {
    /// The buffer row at the top of the grid, partly above it while `y_offset`
    /// is not zero.
    pub first_row: i64,
    /// How many pixels of `first_row` are scrolled past the top of the grid, in
    /// `0.0..cell_height`.
    pub y_offset: f64,
    /// How many rows to draw from `first_row`: the grid's row count, plus the
    /// partly visible row at the bottom while `y_offset` is not zero.
    pub rows: i64,
}

/// Works out which buffer rows a VTE terminal shows at a scroll position.
///
/// `scroll_value` is the value of the terminal's vertical adjustment: a row
/// count from the start of the scrollback, fractional while the view rests
/// between two rows, which is where touchpad scrolling and a drag on the
/// scrollbar leave it. `cell_height` is VTE's cell height in pixels, a whole
/// number, and `row_count` the number of rows in its grid.
///
/// This is VTE's own arithmetic (`row_to_pixel()` in `vte.cc`, the same in
/// 0.80.5 and 0.84): VTE scrolls by whole pixels, rounding
/// `scroll_value * cell_height`, and draws buffer row `r` that many pixels above
/// `r * cell_height`. So `y_offset` is what the rounded offset leaves over after
/// whole rows, and `first_row` is those whole rows: the integer part of
/// `scroll_value`, or one more once the view is within half a pixel of the next
/// row, which VTE then draws flush with the top. Truncating the value to whole
/// rows instead drew every highlight in a scrolled-back view up to a row below
/// its text (issue #343).
///
/// A negative, NaN or infinite `scroll_value` counts as 0, the top of the
/// buffer. With no grid to draw — `row_count` not positive, or `cell_height` not
/// a positive finite number — `rows` is 0. The result is exact while
/// `scroll_value * cell_height` stays below `2^53`, some 5 * 10^14 rows of 17 px.
#[must_use]
pub fn viewport_rows(scroll_value: f64, cell_height: f64, row_count: i64) -> ViewportRows {
    let value = if scroll_value.is_finite() && scroll_value > 0.0 {
        scroll_value.min(MAX_SCROLL_POSITION)
    } else {
        0.0
    };
    // VTE's `scroll_delta_pixel()`: `round(scroll_delta * cell_height)`.
    let pixel_offset = (value * cell_height).round();
    if row_count <= 0 || cell_height <= 0.0 || !pixel_offset.is_finite() {
        return ViewportRows {
            first_row: value.floor() as i64,
            y_offset: 0.0,
            rows: 0,
        };
    }
    // Float `%` is exact, so the remainder is in `0.0..cell_height` and the
    // division below lands on a whole number of rows.
    let y_offset = pixel_offset % cell_height;
    let first_row = ((pixel_offset - y_offset) / cell_height).round() as i64;
    let rows = if y_offset > 0.0 {
        row_count.saturating_add(1)
    } else {
        row_count
    };
    ViewportRows {
        first_row,
        y_offset,
        rows,
    }
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
    use super::{
        ViewportRows, WIDE_RANGES, byte_offset_to_column, char_cell_width, is_valid_color_input,
        normalize_color_input, parse_hex_color, validate_pattern, viewport_rows,
    };

    #[test]
    fn validate_pattern_accepts_what_compile_accepts() {
        assert!(validate_pattern(r"(?i)\bINFO\b").is_ok());
        assert!(validate_pattern("").is_ok());
        assert!(validate_pattern("ERROR(").is_err());
        // The literal other tools use is not a regex error, it just matches
        // nothing useful — the colour field is where it goes wrong.
        assert!(validate_pattern("'INFO'").is_ok());
    }

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

    /// Six bytes are not six hex digits: a Cyrillic `а` is two bytes, so
    /// `#0а0ff` passed the length check and then panicked on a slice that split
    /// the character (issue #343). It must be rejected, not crash.
    #[test]
    fn parse_hex_color_rejects_multibyte_input_without_panicking() {
        assert_eq!(parse_hex_color("#0а0ff"), None);
        assert_eq!(parse_hex_color("#ффф"), None);
        assert_eq!(parse_hex_color("#€000"), None);
    }

    /// `u8::from_str_radix` accepts a leading `+`, which used to let `#+f+f+f`
    /// through as `#0F0F0F`.
    #[test]
    fn parse_hex_color_rejects_signs() {
        assert_eq!(parse_hex_color("#+f+f+f"), None);
        assert_eq!(parse_hex_color("#-f-f-f"), None);
    }

    #[test]
    fn normalize_color_input_trims_and_treats_blank_as_none() {
        assert_eq!(normalize_color_input(""), None);
        assert_eq!(normalize_color_input("   "), None);
        assert_eq!(
            normalize_color_input("  #00AAFF "),
            Some("#00AAFF".to_string())
        );
        // An unfinished value is kept as typed; validity is a separate question.
        assert_eq!(normalize_color_input("#00A"), Some("#00A".to_string()));
    }

    #[test]
    fn is_valid_color_input_accepts_blank_and_hex_only() {
        assert!(is_valid_color_input(""));
        assert!(is_valid_color_input("  "));
        assert!(is_valid_color_input("#00aaFF"));
        assert!(is_valid_color_input(" #00AAFF "));
        assert!(!is_valid_color_input("#00A"));
        assert!(!is_valid_color_input("00AAFF"));
        assert!(!is_valid_color_input("[0,0,255]"));
        assert!(!is_valid_color_input("#0а0ff"));
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

    /// VTE returns a tab as one `'\t'` that spans to the next tab stop, so the
    /// column after it is the next multiple of eight, not one more.
    #[test]
    fn byte_offset_to_column_tab_advances_to_next_stop() {
        let line = "\tat Foo.bar";
        assert_eq!(byte_offset_to_column(line, 0), 0);
        assert_eq!(byte_offset_to_column(line, 1), 8); // "at" starts at the first stop

        let line = "ab\tERROR";
        assert_eq!(byte_offset_to_column(line, 2), 2); // the tab itself
        assert_eq!(byte_offset_to_column(line, 3), 8); // "ERROR" after it

        // A tab that starts on a stop still moves a full stop further.
        let line = "12345678\tx";
        assert_eq!(byte_offset_to_column(line, 9), 16);

        // Consecutive tabs, and a wide character before a tab.
        assert_eq!(byte_offset_to_column("\t\tx", 2), 16);
        assert_eq!(byte_offset_to_column("世\tx", 4), 8);
    }

    /// VTE sizes cells with glib, so these follow East-Asian Width: the `✅`
    /// and `❌` that build and test tools print are W, two cells, and `⚠️` is
    /// the one-cell U+26A0 plus a zero-width variation selector. A match after
    /// any of them was drawn a cell off.
    #[test]
    fn byte_offset_to_column_follows_vte_after_status_symbols() {
        for (line, word, column) in [
            ("❌ ERROR: build failed", "ERROR", 3),
            ("✅ passed", "passed", 3),
            ("⚠\u{FE0F} WARNING: disk", "WARNING", 2),
            ("⭐⚡ FATAL", "FATAL", 5),
        ] {
            let offset = line.find(word).unwrap();
            assert_eq!(byte_offset_to_column(line, offset), column, "{line:?}");
        }
    }

    #[test]
    fn char_cell_width_matches_east_asian_width() {
        for (c, width) in [
            ('a', 1),
            ('世', 2),
            ('\u{3000}', 2),  // Ideographic space (F)
            ('\u{1F600}', 2), // Grinning face (W)
            ('\u{1F5A5}', 1), // Desktop computer: text-style, EAW N
            ('\u{1F6E0}', 1), // Hammer and wrench: EAW N
            ('\u{1F1FA}', 1), // Regional indicator: EAW N, a flag is two of them
            ('\u{FE0F}', 0),  // Variation selector-16
            ('\u{200D}', 0),  // Zero width joiner
            ('\u{E0067}', 0), // Tag character in a subdivision flag
            ('\u{1161}', 0),  // Hangul Jamo medial vowel
            ('\u{2060}', 0),  // Word joiner
            ('\u{00AD}', 1),  // Soft hyphen: glib keeps it one cell wide
        ] {
            assert_eq!(char_cell_width(c), width, "{c:?}");
        }
    }

    /// `char_cell_width` binary-searches the table, which is only correct
    /// while the ranges are sorted and do not overlap.
    #[test]
    fn wide_ranges_are_sorted_and_disjoint() {
        for &(first, last) in WIDE_RANGES {
            assert!(first <= last, "{first:#X}..{last:#X}");
        }
        for pair in WIDE_RANGES.windows(2) {
            assert!(pair[0].1 < pair[1].0, "{pair:X?}");
        }
    }

    /// The [`ViewportRows`] the `viewport_rows` tests below expect.
    fn viewport(first_row: i64, y_offset: f64, rows: i64) -> ViewportRows {
        ViewportRows {
            first_row,
            y_offset,
            rows,
        }
    }

    /// At a whole-row position — the view at the bottom, or scrolled back by
    /// whole rows — the grid is drawn as it always was: unshifted, no extra row.
    #[test]
    fn viewport_rows_whole_row_position_is_unshifted() {
        assert_eq!(viewport_rows(120.0, 17.0, 24), viewport(120, 0.0, 24));
        assert_eq!(viewport_rows(0.0, 17.0, 24), viewport(0, 0.0, 24));
    }

    /// Between two rows VTE rounds the offset to whole pixels: 10.5 rows of
    /// 17 px is 178.5 px, drawn as 179, so row 10 sits 9 px above the grid and a
    /// 25th row shows at the bottom (issue #343).
    #[test]
    fn viewport_rows_between_rows_shifts_up_and_adds_the_bottom_row() {
        assert_eq!(viewport_rows(10.5, 17.0, 24), viewport(10, 9.0, 25));
    }

    /// Within half a pixel of the next row the rounding lands on it, and VTE
    /// draws that row flush with the top: 10.99 rows of 17 px is 186.83 px,
    /// drawn as 187, which is 11 whole rows.
    #[test]
    fn viewport_rows_within_half_a_pixel_of_the_next_row_snaps_onto_it() {
        assert_eq!(viewport_rows(10.99, 17.0, 24), viewport(11, 0.0, 24));
    }

    /// A deep scrollback stays exact, and a position past the last row an `i64`
    /// can number saturates instead of overflowing.
    #[test]
    fn viewport_rows_huge_scroll_values_stay_exact_or_saturate() {
        for (value, expected) in [
            (1_000_000_000.5, viewport(1_000_000_000, 9.0, 25)),
            (1e300, viewport(i64::MAX, 0.0, 24)),
            (f64::MAX, viewport(i64::MAX, 0.0, 24)),
        ] {
            assert_eq!(viewport_rows(value, 17.0, 24), expected, "{value}");
        }
    }

    #[test]
    fn viewport_rows_treats_an_invalid_scroll_value_as_the_top() {
        for value in [-3.5, -0.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(
                viewport_rows(value, 17.0, 24),
                viewport(0, 0.0, 24),
                "{value}"
            );
        }
    }

    #[test]
    fn viewport_rows_draws_nothing_without_a_grid() {
        for (cell_height, row_count) in [
            (17.0, 0),
            (17.0, -1),
            (0.0, 24),
            (-17.0, 24),
            (f64::NAN, 24),
            (f64::INFINITY, 24),
        ] {
            let shown = viewport_rows(10.5, cell_height, row_count);
            assert_eq!(shown.rows, 0, "{cell_height} px, {row_count} rows");
            // A literal zero on the right is exact, and clippy's `float_cmp`
            // accepts it in this form where `assert_eq!` would hide it.
            assert!(shown.y_offset == 0.0, "{cell_height} px, {row_count} rows");
        }
    }
}
