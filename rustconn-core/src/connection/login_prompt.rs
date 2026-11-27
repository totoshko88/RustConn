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

/// What the auto-fill should do after looking at one terminal line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginAction {
    /// Type the account name ([`LoginSequence::username`]) and a newline.
    SendUsername,
    /// Type the password and a newline.
    SendPassword,
    /// The login prompt came back after credentials were sent; stop typing.
    Stop,
}

/// Which credential the sequence is still waiting to send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoginStage {
    Username,
    Password,
    Done,
}

/// The one-shot login auto-fill sequence: username, then password, each sent
/// at most once.
///
/// Fed the line under the cursor every time the terminal changes, it decides
/// what to type. Kept GUI-free so the sequencing can be tested without VTE.
///
/// A username prompt while the password is still pending means the device
/// started over, and the sequence stops rather than retry (a wrong stored
/// password must not walk an account into a lockout). The exception is the
/// prompt line it has just answered: the terminal is re-read within
/// milliseconds of sending the name, so it still shows `login: `, or
/// `login: ubuntu` once the server echoes the name back. That line is the
/// device waiting on our own answer, not a new prompt, and is ignored until
/// the cursor has left it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginSequence {
    stage: LoginStage,
    username: Option<String>,
    send_password: bool,
    /// The answered username prompt line, trailing padding removed.
    answered_line: Option<String>,
    /// Whether a line other than the answered prompt (or its echo) has been
    /// seen since the username was sent.
    left_answered_line: bool,
}

impl LoginSequence {
    /// Starts a sequence for the credentials that are available.
    ///
    /// Without a username the sequence waits for the password prompt straight
    /// away (SSH, or a device that only asks for a password).
    #[must_use]
    pub fn new(username: Option<String>, send_password: bool) -> Self {
        let stage = if username.is_some() {
            LoginStage::Username
        } else if send_password {
            LoginStage::Password
        } else {
            LoginStage::Done
        };
        Self {
            stage,
            username,
            send_password,
            answered_line: None,
            left_answered_line: false,
        }
    }

    /// The account name to type for [`LoginAction::SendUsername`].
    #[must_use]
    pub fn username(&self) -> Option<&str> {
        self.username.as_deref()
    }

    /// Returns `true` once nothing more will be typed.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.stage == LoginStage::Done
    }

    /// Ends the sequence early (timeout, closed session).
    pub fn finish(&mut self) {
        self.stage = LoginStage::Done;
    }

    /// Examines the line under the cursor and returns what to type, if
    /// anything. Each action is returned at most once per sequence.
    pub fn step(&mut self, matcher: &LoginPromptMatcher, line: &str) -> Option<LoginAction> {
        if self.stage == LoginStage::Done {
            return None;
        }
        let own_answer = self.is_answered_line(line);
        if !own_answer {
            self.left_answered_line = true;
        }
        let prompt = matcher.classify(line)?;

        match (self.stage, prompt) {
            (LoginStage::Username, LoginPrompt::Username) => {
                self.answered_line = Some(line.trim_end().to_owned());
                self.left_answered_line = false;
                self.stage = if self.send_password {
                    LoginStage::Password
                } else {
                    LoginStage::Done
                };
                Some(LoginAction::SendUsername)
            }
            // The device went straight to the password (or the account name
            // travelled with the command line, as `telnet -l user` does).
            (LoginStage::Username | LoginStage::Password, LoginPrompt::Password) => {
                self.stage = LoginStage::Done;
                self.send_password.then_some(LoginAction::SendPassword)
            }
            // Still the line we answered: the name is on its way or echoed.
            (LoginStage::Password, LoginPrompt::Username)
                if own_answer && !self.left_answered_line =>
            {
                None
            }
            // ponytail: text-only — a rejection whose identical new prompt
            // arrives before any other line is observed reads as the echo, so
            // the sequence keeps waiting (types nothing) until the idle
            // deadline. Compare the cursor row if that ever bites.
            (LoginStage::Password, LoginPrompt::Username) => {
                self.stage = LoginStage::Done;
                Some(LoginAction::Stop)
            }
            (LoginStage::Done, _) => None,
        }
    }

    /// Returns `true` if `line` is the answered username prompt, bare or
    /// followed by (part of) the echoed account name.
    fn is_answered_line(&self, line: &str) -> bool {
        let (Some(answered), Some(user)) = (&self.answered_line, &self.username) else {
            return false;
        };
        line.trim_end()
            .strip_prefix(answered.as_str())
            .is_some_and(|rest| user.starts_with(rest.trim_start()))
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

    /// Feeds `lines` in order and collects every action returned.
    fn run(
        seq: &mut LoginSequence,
        matcher: &LoginPromptMatcher,
        lines: &[&str],
    ) -> Vec<LoginAction> {
        lines.iter().filter_map(|l| seq.step(matcher, l)).collect()
    }

    #[test]
    fn echoed_username_on_the_prompt_line_still_gets_the_password() {
        // inetutils telnetd: the prompt line is re-read before and after the
        // server echoes the name onto it.
        let matcher = LoginPromptMatcher::default();
        let mut seq = LoginSequence::new(Some("ubuntu".into()), true);
        let actions = run(
            &mut seq,
            &matcher,
            &[
                "ip-172-31-6-28 login: ",
                "ip-172-31-6-28 login: ",
                "ip-172-31-6-28 login: ub",
                "ip-172-31-6-28 login: ubuntu",
                "Password: ",
            ],
        );
        assert_eq!(
            actions,
            [LoginAction::SendUsername, LoginAction::SendPassword]
        );
        assert!(seq.is_done());
    }

    #[test]
    fn echo_is_ignored_with_a_substring_custom_prompt() {
        // A custom needle matches the echoed line too ("login: ubuntu").
        let matcher = LoginPromptMatcher::new(Some("login:"), None);
        let mut seq = LoginSequence::new(Some("ubuntu".into()), true);
        let actions = run(
            &mut seq,
            &matcher,
            &["host login:", "host login: ubuntu", "Password:"],
        );
        assert_eq!(
            actions,
            [LoginAction::SendUsername, LoginAction::SendPassword]
        );
    }

    #[test]
    fn new_login_prompt_after_the_username_stops() {
        let matcher = LoginPromptMatcher::default();
        let mut seq = LoginSequence::new(Some("ubuntu".into()), true);
        let actions = run(
            &mut seq,
            &matcher,
            &[
                "host login: ",
                "host login: ubuntu",
                "Login incorrect",
                "host login: ",
                "Password: ",
            ],
        );
        assert_eq!(actions, [LoginAction::SendUsername, LoginAction::Stop]);
        assert!(seq.is_done());
    }

    #[test]
    fn differently_worded_login_prompt_stops_without_an_intervening_line() {
        let matcher = LoginPromptMatcher::default();
        let mut seq = LoginSequence::new(Some("admin".into()), true);
        let actions = run(&mut seq, &matcher, &["login:", "Username:"]);
        assert_eq!(actions, [LoginAction::SendUsername, LoginAction::Stop]);
    }

    #[test]
    fn line_carrying_another_name_is_not_our_echo() {
        let matcher = LoginPromptMatcher::new(Some("login:"), None);
        let mut seq = LoginSequence::new(Some("ubuntu".into()), true);
        let actions = run(&mut seq, &matcher, &["login:", "login: root"]);
        assert_eq!(actions, [LoginAction::SendUsername, LoginAction::Stop]);
    }

    #[test]
    fn nothing_is_typed_after_the_password() {
        let matcher = LoginPromptMatcher::default();
        let mut seq = LoginSequence::new(Some("ubuntu".into()), true);
        let actions = run(
            &mut seq,
            &matcher,
            &[
                "host login: ",
                "Password: ",
                "Login incorrect",
                "host login: ",
                "Password: ",
            ],
        );
        assert_eq!(
            actions,
            [LoginAction::SendUsername, LoginAction::SendPassword]
        );
    }

    #[test]
    fn device_that_goes_straight_to_the_password() {
        let matcher = LoginPromptMatcher::default();
        let mut seq = LoginSequence::new(Some("admin".into()), true);
        assert_eq!(
            run(&mut seq, &matcher, &["Password:"]),
            [LoginAction::SendPassword]
        );

        let mut seq = LoginSequence::new(None, true);
        assert_eq!(
            run(&mut seq, &matcher, &["Password:", "Password:"]),
            [LoginAction::SendPassword]
        );
    }

    #[test]
    fn username_prompt_without_a_username_stops() {
        let matcher = LoginPromptMatcher::default();
        let mut seq = LoginSequence::new(None, true);
        assert_eq!(run(&mut seq, &matcher, &["login:"]), [LoginAction::Stop]);
    }

    #[test]
    fn username_only_sequence_ends_after_the_name() {
        let matcher = LoginPromptMatcher::default();
        let mut seq = LoginSequence::new(Some("admin".into()), false);
        assert_eq!(
            run(&mut seq, &matcher, &["login:", "Password:"]),
            [LoginAction::SendUsername]
        );
        assert!(seq.is_done());
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
