//! Property tests for shell path escaping (security-sensitive).
//!
//! `escape_path` output is handed to a POSIX shell when files are dragged
//! onto a terminal — an escaping bug is an injection vector, so the
//! round-trip is verified against a real `sh`.

use proptest::prelude::*;
use rustconn_core::shell_escape::{escape_path, escape_paths};

/// Arbitrary path-ish strings including quotes, metacharacters, unicode.
/// NUL is excluded (cannot appear in process arguments).
fn arb_path() -> impl Strategy<Value = String> {
    proptest::string::string_regex("[ -~а-яА-Я'\"`$\\\\;&|<>()*?#~\n\t]{0,40}")
        .expect("valid regex")
        .prop_filter("no NUL", |s| !s.contains('\0'))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Round-trip through a real POSIX shell: `printf %s <escaped>` must
    /// reproduce the original string byte-for-byte (no expansion, no
    /// word-splitting, no command execution).
    #[test]
    fn escaped_path_round_trips_through_sh(path in arb_path()) {
        let escaped = escape_path(&path);
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("printf %s {escaped}"))
            .output()
            .expect("sh is available on POSIX test hosts");
        prop_assert!(output.status.success(), "sh failed for {escaped:?}");
        prop_assert_eq!(
            String::from_utf8_lossy(&output.stdout).into_owned(),
            path,
            "escaped form must round-trip"
        );
    }

    /// Structural invariant: outside of the `'\''` idiom, the escaped
    /// string is a series of single-quoted segments, so it never exposes
    /// an unquoted metacharacter.
    #[test]
    fn escaped_path_is_fully_single_quoted(path in arb_path()) {
        let escaped = escape_path(&path);
        prop_assert!(escaped.starts_with('\''), "must start with a quote");
        prop_assert!(escaped.ends_with('\''), "must end with a quote");
        // Removing the escape idiom must leave no backslashes behind
        let without_idiom = escaped.replace("'\\''", "");
        prop_assert!(
            !without_idiom.contains('\\') || path.contains('\\'),
            "backslashes outside the quote idiom must come from the input"
        );
    }

    /// Joining multiple paths keeps each one independently quoted.
    #[test]
    fn escaped_paths_join_preserves_count(
        paths in proptest::collection::vec(arb_path(), 1..4)
    ) {
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let joined = escape_paths(&refs);
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("for a in {joined}; do printf '%s\\n--SEP--\\n' \"$a\"; done"))
            .output()
            .expect("sh is available on POSIX test hosts");
        prop_assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let count = stdout.matches("--SEP--").count();
        prop_assert_eq!(count, paths.len(), "argument count must be preserved");
    }
}
