//! GUI-free login-prompt detection for interactive terminal protocols.
//!
//! Telnet and Serial consoles have no authentication protocol: the device
//! simply prints a prompt and expects the credentials to be typed. Network
//! gear is wildly inconsistent about the wording — a Huawei OLT MA5800 writes
//! `>>User name:`, a Huawei S6700 writes `Username:`, Datacom switches write
//! `login:` (issue #254).
//!
//! [`looks_like_username_prompt`] recognizes the common forms out of the box.
//! When a device uses something else, a connection (or its group) can supply
//! literal expected text, and [`LoginPromptMatcher`] then matches that instead.
//!
//! The matching logic lives here so it can be unit/property-tested without
//! gtk/vte. The GUI layer extracts the relevant line (the line under the
//! cursor) and delegates the decision to this module.

use super::ssh_prompt::looks_like_password_prompt;

/// Which credential a matched prompt is asking for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginPrompt {
    /// The device is asking for the account name.
    Username,
    /// The device is asking for the password.
    Password,
}

/// Line endings recognized as a username prompt without any configuration.
///
/// Matched against the trimmed, lowercased line with `ends_with`, so
/// `>>User name:` and `  Username: ` both hit. Kept deliberately short:
/// every extra needle is another way to mistake ordinary output for a
/// prompt and type an account name into a live session.
pub const DEFAULT_USERNAME_PROMPTS: &[&str] = &[
    // English (and everything network gear ships in practice)
    "login:",
    "login as:",
    "username:",
    "user name:",
    "user:",
    "user id:",
    // German
    "benutzername:",
    "anmeldung:",
    // French
    "nom d'utilisateur:",
    "utilisateur:",
    // Spanish / Portuguese
    "usuario:",
    "usuário:",
    // Ukrainian
    "ім'я користувача:",
    "користувач:",
];

/// Endings that look like a username prompt but are not one.
///
/// `Last login:` is printed by essentially every Unix MOTD and ends with
/// `login:`. It is normally followed by a date on the same line (so the
/// `ends_with` test fails anyway), but a line wrapped right after the colon
/// would otherwise be answered with the account name.
const USERNAME_PROMPT_EXCEPTIONS: &[&str] = &["last login:", "last failed login:"];

/// Returns `true` if `line` looks like a username/login prompt.
///
/// The line is trimmed and lowercased before matching, so terminal grid
/// padding and capitalization do not affect the result. Returns `false` for
/// MOTD lines such as `Last login:` (see [`USERNAME_PROMPT_EXCEPTIONS`]).
#[must_use]
pub fn looks_like_username_prompt(line: &str) -> bool {
    let l = line.trim().to_lowercase();
    if l.is_empty() {
        return false;
    }

    if USERNAME_PROMPT_EXCEPTIONS.iter().any(|e| l.ends_with(e)) {
        return false;
    }

    DEFAULT_USERNAME_PROMPTS.iter().any(|p| l.ends_with(p))
}

/// Decides which credential a terminal line is asking for.
///
/// Built from the optional per-connection expected texts. An absent (or
/// blank) expected text falls back to the built-in matchers —
/// [`looks_like_username_prompt`] and
/// [`looks_like_password_prompt`].
///
/// A supplied expected text is matched as a **case-insensitive substring**,
/// not a regex: the user is expected to paste the literal wording the device
/// prints. Substring rather than suffix so that `User name:` still matches
/// `>>User name:` — devices like to decorate their prompts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoginPromptMatcher {
    /// Lowercased expected text for the username prompt, if configured.
    username_needle: Option<String>,
    /// Lowercased expected text for the password prompt, if configured.
    password_needle: Option<String>,
}

impl LoginPromptMatcher {
    /// Creates a matcher from the optional expected texts of a connection.
    ///
    /// Blank or whitespace-only strings are treated as "not configured".
    #[must_use]
    pub fn new(username_prompt: Option<&str>, password_prompt: Option<&str>) -> Self {
        Self {
            username_needle: normalize_needle(username_prompt),
            password_needle: normalize_needle(password_prompt),
        }
    }

    /// Returns `true` if neither prompt was customized.
    #[must_use]
    pub const fn is_default(&self) -> bool {
        self.username_needle.is_none() && self.password_needle.is_none()
    }

    /// Returns `true` if `line` asks for the username.
    #[must_use]
    pub fn matches_username(&self, line: &str) -> bool {
        match &self.username_needle {
            Some(needle) => contains_ignore_case(line, needle),
            None => looks_like_username_prompt(line),
        }
    }

    /// Returns `true` if `line` asks for the password.
    ///
    /// With no configured expected text this is
    /// [`looks_like_password_prompt`],
    /// which also rejects key-passphrase prompts.
    #[must_use]
    pub fn matches_password(&self, line: &str) -> bool {
        match &self.password_needle {
            Some(needle) => contains_ignore_case(line, needle),
            None => looks_like_password_prompt(line),
        }
    }

    /// Classifies `line`, preferring the password prompt on a tie.
    ///
    /// A tie is possible when a device prints both words on one line or when
    /// the user configures overlapping expected texts. Password wins because
    /// it is the later stage of the login: answering it with the account name
    /// would send the wrong secret, while the reverse merely repeats a step
    /// the device is no longer waiting for.
    #[must_use]
    pub fn classify(&self, line: &str) -> Option<LoginPrompt> {
        if self.matches_password(line) {
            Some(LoginPrompt::Password)
        } else if self.matches_username(line) {
            Some(LoginPrompt::Username)
        } else {
            None
        }
    }
}

/// Trims a configured expected text and lowercases it, mapping blank to `None`.
fn normalize_needle(value: Option<&str>) -> Option<String> {
    let trimmed = value?.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_lowercase())
    }
}

/// Case-insensitive substring test against the trimmed line.
///
/// `needle` must already be lowercased (see [`normalize_needle`]).
fn contains_ignore_case(line: &str, needle: &str) -> bool {
    line.trim().to_lowercase().contains(needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_device_prompts_from_issue_254() {
        // Huawei OLT MA5800
        assert!(looks_like_username_prompt(">>User name:"));
        // Huawei S6700
        assert!(looks_like_username_prompt("Username:"));
        // Datacom
        assert!(looks_like_username_prompt("login:"));
    }

    #[test]
    fn ignores_padding_and_case() {
        assert!(looks_like_username_prompt("   USERNAME:   "));
        assert!(looks_like_username_prompt("Login As:"));
    }

    #[test]
    fn rejects_motd_last_login() {
        assert!(!looks_like_username_prompt("Last login:"));
        assert!(!looks_like_username_prompt("Last failed login:"));
        assert!(!looks_like_username_prompt(
            "Last login: Mon Aug  3 10:00:00 2026"
        ));
    }

    #[test]
    fn rejects_unrelated_output() {
        assert!(!looks_like_username_prompt(""));
        assert!(!looks_like_username_prompt("   "));
        assert!(!looks_like_username_prompt("Trying 192.0.2.1..."));
        assert!(!looks_like_username_prompt("Password:"));
    }

    #[test]
    fn default_matcher_delegates_to_builtin_matchers() {
        let matcher = LoginPromptMatcher::default();
        assert!(matcher.is_default());
        assert_eq!(matcher.classify("Username:"), Some(LoginPrompt::Username));
        assert_eq!(matcher.classify("Password:"), Some(LoginPrompt::Password));
        assert_eq!(matcher.classify("uptime is 4 days"), None);
    }

    #[test]
    fn blank_expected_text_is_not_configured() {
        let matcher = LoginPromptMatcher::new(Some("   "), Some(""));
        assert!(matcher.is_default());
        assert_eq!(matcher.classify("login:"), Some(LoginPrompt::Username));
    }

    #[test]
    fn custom_username_text_replaces_the_defaults() {
        let matcher = LoginPromptMatcher::new(Some("Enter account"), None);
        assert!(!matcher.is_default());
        assert_eq!(
            matcher.classify("Enter account >"),
            Some(LoginPrompt::Username)
        );
        // The built-in needles no longer apply once overridden.
        assert_eq!(matcher.classify("Username:"), None);
    }

    #[test]
    fn custom_text_matches_as_substring() {
        let matcher = LoginPromptMatcher::new(Some("User name:"), None);
        assert_eq!(
            matcher.classify(">>User name:"),
            Some(LoginPrompt::Username)
        );
    }

    #[test]
    fn custom_password_text_replaces_the_defaults() {
        let matcher = LoginPromptMatcher::new(None, Some("Secret code"));
        assert_eq!(
            matcher.classify("Secret code ??"),
            Some(LoginPrompt::Password)
        );
        assert_eq!(matcher.classify("Password:"), None);
    }

    #[test]
    fn password_wins_when_both_match() {
        let matcher = LoginPromptMatcher::new(Some("login"), Some("password"));
        assert_eq!(
            matcher.classify("login password:"),
            Some(LoginPrompt::Password)
        );
    }

    #[test]
    fn default_password_matcher_still_rejects_passphrase_prompts() {
        let matcher = LoginPromptMatcher::default();
        assert_eq!(
            matcher.classify("Enter passphrase for key '/home/u/.ssh/id_ed25519':"),
            None
        );
    }

    #[test]
    fn default_password_matcher_rejects_password_change_prompts() {
        let matcher = LoginPromptMatcher::default();
        assert_eq!(matcher.classify("Old Password:"), None);
        assert_eq!(matcher.classify("(current) UNIX password:"), None);
        assert_eq!(matcher.classify("New password:"), None);
        assert_eq!(matcher.classify("Retype new password:"), None);
    }
}
