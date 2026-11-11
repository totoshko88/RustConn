//! Property-based tests for SSH password-prompt detection.
//!
//! **Feature: release-0.17.5, Property 1: prompt detection (#194)**
//!
//! `looks_like_password_prompt` is true for all supported localized prompts and
//! `pass:`/trailing-spaces/no-trailing-space; false for `passphrase for key`.

use proptest::prelude::*;
use rustconn_core::connection::looks_like_password_prompt;

/// Every password-prompt suffix recognized by the implementation.
///
/// Kept in sync with `rustconn-core/src/connection/ssh_prompt.rs`. Each entry is
/// a suffix that, when it terminates a (trimmed) line, marks it as a password
/// prompt. Stored lowercase because the function lowercases before matching.
const PASSWORD_SUFFIXES: &[&str] = &[
    // English + PAM `'s password:` form (covered by the `password:` tail)
    "password:",
    "'s password:",
    // German
    "passwort:",
    "kennwort:",
    // French (with and without the space before the colon)
    "mot de passe:",
    "mot de passe :",
    // Spanish
    "contraseña:",
    // Portuguese
    "senha:",
    // Ukrainian / Belarusian
    "пароль:",
    // Polish
    "hasło:",
    // Czech / Slovak
    "heslo:",
    // Dutch
    "wachtwoord:",
    // Swedish / Danish
    "lösenord:",
    "adgangskode:",
    // Chinese (simplified + traditional, half- and full-width colon)
    "密码:",
    "密码：",
    "密碼:",
    "密碼：",
    // Japanese
    "パスワード:",
    "パスワード：",
    // Korean
    "비밀번호:",
    "비밀번호：",
    // Generic colon-terminated prompt (catch-all for PAM)
    "pass:",
];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Property 1 (positive): for every supported localized suffix, the function
    /// returns true regardless of arbitrary leading text and arbitrary trailing
    /// whitespace.
    ///
    /// The leading prefix is drawn from a space-free, ASCII charset so it can
    /// never accidentally spell `passphrase for` (which would legitimately make
    /// the line a passphrase prompt and flip the expected result).
    #[test]
    fn prop_localized_prompt_detected(
        idx in 0usize..PASSWORD_SUFFIXES.len(),
        prefix in "[a-zA-Z0-9@._'-]{0,30}",
        trailing in "[ \t]{0,6}",
    ) {
        let suffix = PASSWORD_SUFFIXES[idx];
        let line = format!("{prefix}{suffix}{trailing}");
        prop_assert!(
            looks_like_password_prompt(&line),
            "expected password prompt for line {line:?} (suffix {suffix:?})"
        );
    }

    /// Property 1 (positive, case-insensitive): the same holds when the suffix
    /// uses upper-case where ASCII permits, since matching lowercases first.
    #[test]
    fn prop_detection_is_case_insensitive(
        idx in 0usize..PASSWORD_SUFFIXES.len(),
        prefix in "[A-Za-z]{0,20}",
    ) {
        let suffix = PASSWORD_SUFFIXES[idx];
        let upper_suffix = suffix.to_uppercase();
        let line = format!("{prefix}{upper_suffix}");
        prop_assert!(
            looks_like_password_prompt(&line),
            "expected case-insensitive match for line {line:?}"
        );
    }

    /// Property 1 (negative): any line containing `passphrase for key` is never
    /// treated as a password prompt, even when it would otherwise end in a
    /// recognized suffix.
    #[test]
    fn prop_passphrase_prompt_rejected(
        prefix in "[a-zA-Z0-9 '/._-]{0,30}",
        suffix_idx in 0usize..PASSWORD_SUFFIXES.len(),
    ) {
        // Build a line that embeds the passphrase marker and *also* ends with a
        // real password suffix — the passphrase guard must win.
        let tail = PASSWORD_SUFFIXES[suffix_idx];
        let line = format!("{prefix}passphrase for key '/home/u/.ssh/id_ed25519' {tail}");
        prop_assert!(
            !looks_like_password_prompt(&line),
            "passphrase prompt must be rejected, got match for {line:?}"
        );
    }
}
