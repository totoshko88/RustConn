//! Pure SSH password-prompt detection (GUI-free).
//!
//! The matching logic lives here so it can be unit/property-tested without
//! gtk/vte. The GUI layer (`rustconn`) extracts the relevant line (cursor line
//! or last grid line) and delegates the decision to [`looks_like_password_prompt`].

/// Returns `true` if `line` looks like an SSH password prompt, in any supported UI language.
///
/// The line is trimmed and lowercased before matching, so leading/trailing
/// whitespace (common in terminal grid padding) does not affect the result.
/// Both trailing-space (`"Password: "`) and no-trailing-space (`"Password:"`)
/// forms are recognized.
///
/// Returns `false` for key-passphrase prompts (`passphrase for key`): those
/// require the key passphrase, not the account password.
#[must_use]
pub fn looks_like_password_prompt(line: &str) -> bool {
    let l = line.trim().to_lowercase();

    // Reject passphrase prompts — these need a key passphrase, not the password.
    if l.contains("passphrase for") {
        return false;
    }

    // Reject password-change prompts — these ask the user to type a *new* or
    // *current* password as part of `passwd` or a RADIUS challenge, and answering
    // them with the stored login password is always wrong (issue #254).
    if l.contains("old password")
        || l.contains("current password")
        || l.contains("(current)")
        || l.contains("new password")
        || l.contains("retype password")
        || l.contains("confirm password")
        || l.contains("verify password")
        || l.contains("repeat password")
        // Ukrainian/Russian
        || l.contains("старий пароль")
        || l.contains("старый пароль")
        || l.contains("новий пароль")
        || l.contains("новый пароль")
    {
        return false;
    }

    l.ends_with("password:")
        || l.contains("'s password:")
        // German
        || l.ends_with("passwort:")
        || l.ends_with("kennwort:")
        // French
        || l.ends_with("mot de passe:")
        || l.ends_with("mot de passe :")
        // Spanish
        || l.ends_with("contraseña:")
        // Portuguese
        || l.ends_with("senha:")
        // Ukrainian / Belarusian
        || l.ends_with("пароль:")
        // Polish
        || l.ends_with("hasło:")
        // Czech/Slovak
        || l.ends_with("heslo:")
        // Dutch
        || l.ends_with("wachtwoord:")
        // Swedish/Danish/Norwegian
        || l.ends_with("lösenord:")
        || l.ends_with("adgangskode:")
        // Chinese (half- and full-width colon)
        || l.ends_with("密码:")
        || l.ends_with("密码：")
        || l.ends_with("密碼:")
        || l.ends_with("密碼：")
        // Japanese
        || l.ends_with("パスワード:")
        || l.ends_with("パスワード：")
        // Korean
        || l.ends_with("비밀번호:")
        || l.ends_with("비밀번호：")
        // Generic colon-terminated prompt (catch-all for PAM)
        || l.ends_with("pass:")
}

#[cfg(test)]
mod tests {
    use super::looks_like_password_prompt;

    #[test]
    fn matches_english_prompt_with_and_without_trailing_space() {
        assert!(looks_like_password_prompt("Password:"));
        assert!(looks_like_password_prompt("Password: "));
        assert!(looks_like_password_prompt("user@host's password: "));
    }

    #[test]
    fn matches_pam_generic_pass_prompt() {
        assert!(looks_like_password_prompt("pass:"));
        assert!(looks_like_password_prompt("PASS: "));
    }

    /// Keyboard-interactive prompts come from the server's PAM stack, not from
    /// OpenSSH, so they lack the `user@host's password:` shape. ESXi (issue #364)
    /// is the canonical case: `(root@host) Password:`. These must still match —
    /// the askpass shell helper in `rustconn` mirrors exactly this behavior.
    #[test]
    fn matches_pam_keyboard_interactive_prompts() {
        assert!(looks_like_password_prompt(
            "(root@esxi.example.com) Password:"
        ));
        assert!(looks_like_password_prompt(
            "(root@esxi.example.com) Password: "
        ));
        assert!(looks_like_password_prompt("Password:"));
        assert!(looks_like_password_prompt("Password: "));
    }

    #[test]
    fn matches_localized_prompts() {
        assert!(looks_like_password_prompt("Пароль:"));
        assert!(looks_like_password_prompt("密码："));
        assert!(looks_like_password_prompt("Passwort:"));
    }

    #[test]
    fn rejects_key_passphrase_prompt() {
        assert!(!looks_like_password_prompt(
            "Enter passphrase for key '/home/u/.ssh/id_ed25519':"
        ));
        assert!(!looks_like_password_prompt("Enter passphrase for key:"));
    }

    #[test]
    fn rejects_unrelated_text() {
        assert!(!looks_like_password_prompt(""));
        assert!(!looks_like_password_prompt("Last login: Mon"));
    }

    /// Every localized suffix the implementation supports, with and without a
    /// trailing space. Verifies each branch of the matcher individually so a
    /// dropped language is caught immediately.
    #[test]
    fn matches_all_supported_localizations() {
        let suffixes = [
            "Password:",      // English
            "Passwort:",      // German
            "Kennwort:",      // German (alt)
            "Mot de passe:",  // French (no space)
            "Mot de passe :", // French (space before colon)
            "Contraseña:",    // Spanish
            "Senha:",         // Portuguese
            "Пароль:",        // Ukrainian / Belarusian
            "Hasło:",         // Polish
            "Heslo:",         // Czech / Slovak
            "Wachtwoord:",    // Dutch
            "Lösenord:",      // Swedish
            "Adgangskode:",   // Danish
            "密码:",          // Chinese simplified, half-width colon
            "密码：",         // Chinese simplified, full-width colon
            "密碼:",          // Chinese traditional, half-width colon
            "密碼：",         // Chinese traditional, full-width colon
            "パスワード:",    // Japanese, half-width colon
            "パスワード：",   // Japanese, full-width colon
            "비밀번호:",      // Korean, half-width colon
            "비밀번호：",     // Korean, full-width colon
            "pass:",          // Generic PAM catch-all
        ];
        for suffix in suffixes {
            assert!(
                looks_like_password_prompt(suffix),
                "no trailing space: {suffix:?} should match"
            );
            assert!(
                looks_like_password_prompt(&format!("{suffix} ")),
                "trailing space: {suffix:?} should match"
            );
            assert!(
                looks_like_password_prompt(&format!("host {suffix}")),
                "with leading text: {suffix:?} should match"
            );
        }
    }

    /// Trailing-whitespace padding from terminal grids must not defeat the match.
    #[test]
    fn matches_with_trailing_whitespace_padding() {
        assert!(looks_like_password_prompt("Password:   "));
        assert!(looks_like_password_prompt("Password:\t"));
        assert!(looks_like_password_prompt("  Пароль:  "));
    }

    /// Passphrase prompts stay rejected even when they end in a password suffix.
    #[test]
    fn rejects_passphrase_even_with_password_suffix() {
        assert!(!looks_like_password_prompt(
            "Enter passphrase for key '/home/u/.ssh/id_ed25519': Password:"
        ));
    }

    /// Password-change prompts are rejected so auto-login does not answer them
    /// with the stored login password (which is always wrong for these).
    #[test]
    fn rejects_password_change_prompts() {
        assert!(!looks_like_password_prompt("(current) UNIX password:"));
        assert!(!looks_like_password_prompt("Old Password:"));
        assert!(!looks_like_password_prompt("Current password:"));
        assert!(!looks_like_password_prompt("New password:"));
        assert!(!looks_like_password_prompt("Retype new password:"));
        assert!(!looks_like_password_prompt("Confirm password:"));
        assert!(!looks_like_password_prompt("Verify password:"));
        assert!(!looks_like_password_prompt("Repeat password:"));
        // Ukrainian
        assert!(!looks_like_password_prompt("Старий пароль:"));
        assert!(!looks_like_password_prompt("Новий пароль:"));
    }
}
