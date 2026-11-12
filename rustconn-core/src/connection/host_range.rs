//! Numbered ranges for duplicating one connection into many.
//!
//! `host[02-10]` expands to `host02` … `host10`, and `host[2-10]` to `host2` …
//! `host10`: the width of the start value sets the zero padding, the convention
//! of `pdsh` and of Ansible inventories.

use thiserror::Error;

use crate::config::ConfigManager;
use crate::error::ConfigError;
use crate::models::Connection;

/// The most connections one range may create: a full /24, and a ceiling against
/// a typo creating thousands of entries that must then be deleted by hand.
pub const MAX_RANGE_SIZE: u32 = 256;

/// Why a range pattern cannot be used.
#[derive(Debug, Error)]
pub enum HostRangeError {
    /// The pattern is not text with exactly one `[start-end]` number range.
    #[error("expected one number range such as host[02-10]")]
    InvalidPattern,
    /// The range runs backwards.
    #[error("the range start {start} is above its end {end}")]
    Reversed {
        /// First value of the range.
        start: u32,
        /// Last value of the range.
        end: u32,
    },
    /// The range would create more than [`MAX_RANGE_SIZE`] connections.
    #[error("the range covers more than {max} connections")]
    TooLarge {
        /// The limit that was exceeded.
        max: u32,
    },
    /// The name and host patterns expand to different numbers of entries.
    #[error("the name range covers {names} entries but the host range covers {hosts}")]
    SizeMismatch {
        /// Entries in the name range.
        names: usize,
        /// Entries in the host range.
        hosts: usize,
    },
    /// An expanded entry fails connection validation.
    #[error("'{name}' is not a valid connection: {source}")]
    InvalidConnection {
        /// The name of the entry that failed.
        name: String,
        /// Why it failed.
        source: ConfigError,
    },
}

/// A parsed `prefix[start-end]suffix` pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostRange {
    prefix: String,
    start: u32,
    end: u32,
    width: usize,
    suffix: String,
}

impl HostRange {
    /// Parses a pattern such as `host[02-10].example.com`.
    ///
    /// # Errors
    ///
    /// Returns a [`HostRangeError`] unless `pattern` holds exactly one ascending
    /// range of at most [`MAX_RANGE_SIZE`] values.
    pub fn parse(pattern: &str) -> Result<Self, HostRangeError> {
        let (prefix, rest) = pattern
            .split_once('[')
            .ok_or(HostRangeError::InvalidPattern)?;
        let (range, suffix) = rest.split_once(']').ok_or(HostRangeError::InvalidPattern)?;
        let (first, last) = range
            .split_once('-')
            .ok_or(HostRangeError::InvalidPattern)?;
        if [prefix, suffix]
            .iter()
            .any(|part| part.contains(['[', ']']))
        {
            return Err(HostRangeError::InvalidPattern);
        }

        let start = parse_bound(first)?;
        let end = parse_bound(last)?;
        if start > end {
            return Err(HostRangeError::Reversed { start, end });
        }
        if end - start >= MAX_RANGE_SIZE {
            return Err(HostRangeError::TooLarge {
                max: MAX_RANGE_SIZE,
            });
        }
        Ok(Self {
            prefix: prefix.to_string(),
            start,
            end,
            width: first.len(),
            suffix: suffix.to_string(),
        })
    }

    /// Returns every value of the range, in order.
    #[must_use]
    pub fn expand(&self) -> Vec<String> {
        (self.start..=self.end)
            .map(|number| {
                format!(
                    "{}{number:0width$}{}",
                    self.prefix,
                    self.suffix,
                    width = self.width
                )
            })
            .collect()
    }
}

/// Parses one bound, accepting ASCII digits only (`u32::parse` allows a `+`).
fn parse_bound(text: &str) -> Result<u32, HostRangeError> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(HostRangeError::InvalidPattern);
    }
    text.parse().map_err(|_| HostRangeError::InvalidPattern)
}

/// Suggests a name pattern continuing from `name`.
///
/// Covers the next nine numbers after its last run of digits, keeping its
/// padding: `host01` gives `host[02-10]` and `host1` gives `host[2-10]`. `None`
/// when the name holds no usable number.
#[must_use]
pub fn suggest_name_pattern(name: &str) -> Option<String> {
    let end = name.rfind(|c: char| c.is_ascii_digit())? + 1;
    let start = name[..end]
        .rfind(|c: char| !c.is_ascii_digit())
        .map_or(0, |index| {
            index + name[index..].chars().next().map_or(1, char::len_utf8)
        });
    let number: u32 = name[start..end].parse().ok()?;
    let width = end - start;
    let first = number.checked_add(1)?;
    let last = number.checked_add(9)?;
    Some(format!(
        "{}[{first:0width$}-{last:0width$}]{}",
        &name[..start],
        &name[end..]
    ))
}

/// Derives the host pattern for `name_pattern` from `source`'s host.
///
/// Works when the host contains the name, so `host01.example.com` with
/// `host[02-10]` gives `host[02-10].example.com`. `None` otherwise (an IP
/// address, say): guessing could point every copy at the wrong machine.
#[must_use]
pub fn derive_host_pattern(source: &Connection, name_pattern: &str) -> Option<String> {
    if source.name.is_empty() || !source.host.contains(&source.name) {
        return None;
    }
    Some(source.host.replacen(&source.name, name_pattern, 1))
}

/// The connections a range will create.
#[derive(Debug, Clone)]
pub struct RangePlan {
    /// New connections, ready to store, in range order.
    pub created: Vec<Connection>,
    /// Names left out because `name_taken` reported them as existing.
    pub skipped: Vec<String>,
}

/// Plans the copies of `source` that a pair of range patterns describes.
///
/// Nothing is stored. Every entry is validated first, so a bad pattern cannot
/// leave half a range behind. Each copy is otherwise a normal duplicate; see
/// [`Connection::duplicate_as`].
///
/// # Errors
///
/// Returns a [`HostRangeError`] when a pattern is unusable, the two differ in
/// size, or an expanded entry would not be a valid connection.
pub fn plan_range_duplicates(
    source: &Connection,
    name_pattern: &str,
    host_pattern: &str,
    name_taken: impl Fn(&str) -> bool,
) -> Result<RangePlan, HostRangeError> {
    let names = HostRange::parse(name_pattern)?.expand();
    let hosts = HostRange::parse(host_pattern)?.expand();
    if names.len() != hosts.len() {
        return Err(HostRangeError::SizeMismatch {
            names: names.len(),
            hosts: hosts.len(),
        });
    }

    let mut plan = RangePlan {
        created: Vec::with_capacity(names.len()),
        skipped: Vec::new(),
    };
    for (name, host) in names.into_iter().zip(hosts) {
        if name_taken(&name) {
            plan.skipped.push(name);
            continue;
        }
        let mut copy = source.duplicate_as(name);
        copy.host = host;
        // The connection editor's host rule; `validate_connection` does not check it.
        crate::dialog_utils::validate_host(&copy.host)
            .map_err(|e| ConfigError::Validation {
                field: "host".to_string(),
                reason: e.to_string(),
            })
            .and_then(|()| ConfigManager::validate_connection(&copy))
            .map_err(|source| HostRangeError::InvalidConnection {
                name: copy.name.clone(),
                source,
            })?;
        plan.created.push(copy);
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expand(pattern: &str) -> Vec<String> {
        HostRange::parse(pattern).unwrap().expand()
    }

    fn source(name: &str, host: &str) -> Connection {
        Connection::new_ssh(name.to_string(), host.to_string(), 22)
    }

    #[test]
    fn a_padded_start_keeps_its_zeros() {
        assert_eq!(expand("host[08-10]"), ["host08", "host09", "host10"]);
    }

    #[test]
    fn an_unpadded_start_stays_unpadded() {
        assert_eq!(expand("host[8-10]"), ["host8", "host9", "host10"]);
    }

    #[test]
    fn padding_is_not_applied_past_the_start_width() {
        assert_eq!(expand("h[099-100]"), ["h099", "h100"]);
        assert_eq!(expand("h[99-100]"), ["h99", "h100"]);
    }

    #[test]
    fn text_after_the_range_is_kept() {
        assert_eq!(
            expand("h[1-2].example.com"),
            ["h1.example.com", "h2.example.com"]
        );
    }

    #[test]
    fn a_single_value_range_is_allowed() {
        assert_eq!(expand("h[5-5]"), ["h5"]);
    }

    #[test]
    fn malformed_patterns_are_rejected() {
        for pattern in [
            "",
            "host",
            "host[",
            "host]",
            "host[]",
            "host[1]",
            "host[-2]",
            "host[1-]",
            "host[a-b]",
            "host[+1-2]",
            "host[1-2][3-4]",
            "ho]st[1-2]",
            "host[1-99999999999]",
        ] {
            assert!(
                matches!(
                    HostRange::parse(pattern),
                    Err(HostRangeError::InvalidPattern)
                ),
                "{pattern} should be rejected"
            );
        }
    }

    #[test]
    fn a_backwards_range_is_rejected() {
        assert!(matches!(
            HostRange::parse("h[10-2]"),
            Err(HostRangeError::Reversed { start: 10, end: 2 })
        ));
    }

    #[test]
    fn the_size_limit_is_inclusive() {
        assert_eq!(expand("h[1-256]").len(), 256);
        assert!(matches!(
            HostRange::parse("h[1-257]"),
            Err(HostRangeError::TooLarge { .. })
        ));
    }

    #[test]
    fn suggestions_continue_from_the_source_number() {
        assert_eq!(
            suggest_name_pattern("host01").as_deref(),
            Some("host[02-10]")
        );
        assert_eq!(suggest_name_pattern("host1").as_deref(), Some("host[2-10]"));
        assert_eq!(
            suggest_name_pattern("web7.dc").as_deref(),
            Some("web[8-16].dc")
        );
        assert_eq!(suggest_name_pattern("host"), None);
    }

    #[test]
    fn suggestions_are_safe_after_multibyte_text_and_parse_back() {
        let pattern = suggest_name_pattern("сервер01").unwrap();
        assert_eq!(pattern, "сервер[02-10]");
        assert_eq!(expand(&pattern).len(), 9);
    }

    #[test]
    fn a_number_too_large_to_parse_gets_no_suggestion() {
        assert_eq!(suggest_name_pattern("h99999999999"), None);
    }

    #[test]
    fn the_host_pattern_follows_a_host_that_contains_the_name() {
        let conn = source("host01", "host01.example.com");
        assert_eq!(
            derive_host_pattern(&conn, "host[02-10]").as_deref(),
            Some("host[02-10].example.com")
        );
    }

    #[test]
    fn a_host_unrelated_to_the_name_is_not_guessed() {
        let conn = source("host01", "10.0.0.1");
        assert_eq!(derive_host_pattern(&conn, "host[02-10]"), None);
    }

    #[test]
    fn copies_take_their_name_and_host_and_nothing_else_changes() {
        let mut original = source("host01", "host01.example.com");
        original.username = Some("admin".to_string());
        let plan =
            plan_range_duplicates(&original, "host[02-03]", "host[02-03].example.com", |_| {
                false
            })
            .unwrap();

        let names: Vec<&str> = plan.created.iter().map(|c| c.name.as_str()).collect();
        let hosts: Vec<&str> = plan.created.iter().map(|c| c.host.as_str()).collect();
        assert_eq!(names, ["host02", "host03"]);
        assert_eq!(hosts, ["host02.example.com", "host03.example.com"]);
        assert!(plan.created.iter().all(|c| {
            c.id != original.id && c.username.as_deref() == Some("admin") && c.port == 22
        }));
    }

    #[test]
    fn existing_names_are_skipped_and_keep_their_number() {
        let original = source("host01", "host01.example.com");
        let plan = plan_range_duplicates(&original, "host[01-03]", "h[01-03].example.com", |n| {
            n == "host01"
        })
        .unwrap();
        assert_eq!(plan.skipped, ["host01"]);
        assert_eq!(plan.created[0].name, "host02");
        assert_eq!(plan.created[0].host, "h02.example.com");
    }

    #[test]
    fn name_and_host_ranges_must_match_in_size() {
        let original = source("host01", "10.0.0.1");
        assert!(matches!(
            plan_range_duplicates(&original, "h[1-3]", "10.0.0.[1-2]", |_| false),
            Err(HostRangeError::SizeMismatch { names: 3, hosts: 2 })
        ));
    }

    #[test]
    fn a_host_without_a_range_is_rejected_rather_than_repeated() {
        let original = source("host01", "10.0.0.1");
        assert!(matches!(
            plan_range_duplicates(&original, "h[1-3]", "10.0.0.1", |_| false),
            Err(HostRangeError::InvalidPattern)
        ));
    }

    #[test]
    fn an_invalid_expanded_host_fails_the_whole_plan() {
        let original = source("host01", "10.0.0.1");
        assert!(matches!(
            plan_range_duplicates(&original, "h[1-2]", "bad host[1-2]", |_| false),
            Err(HostRangeError::InvalidConnection { .. })
        ));
    }
}
