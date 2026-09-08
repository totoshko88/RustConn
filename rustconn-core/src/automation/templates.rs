//! Automation templates — preset expect rules for common scenarios
//!
//! Provides ready-to-use [`ExpectRule`] sets for typical interactive
//! prompts encountered during SSH, Telnet, and Serial connections.

use super::ExpectRule;

/// An automation template with a name, description, and preset rules.
#[derive(Debug, Clone)]
pub struct AutomationTemplate {
    /// Machine-readable identifier (e.g. `"sudo_password"`)
    pub id: &'static str,
    /// Human-readable name for display
    pub name: &'static str,
    /// Short description of what the template does
    pub description: &'static str,
    /// Protocol hint (empty = any protocol)
    pub protocol_hint: &'static str,
    /// Factory function returning fresh rules (each call generates new UUIDs)
    rules_fn: fn() -> Vec<ExpectRule>,
}

impl AutomationTemplate {
    /// Returns a fresh set of expect rules with new UUIDs.
    #[must_use]
    pub fn rules(&self) -> Vec<ExpectRule> {
        (self.rules_fn)()
    }
}

/// Returns all built-in automation templates.
#[must_use]
pub fn builtin_templates() -> &'static [AutomationTemplate] {
    &TEMPLATES
}

/// Returns templates filtered by protocol (empty string matches all).
#[must_use]
pub fn templates_for_protocol(protocol: &str) -> Vec<&'static AutomationTemplate> {
    TEMPLATES
        .iter()
        .filter(|t| t.protocol_hint.is_empty() || t.protocol_hint == protocol)
        .collect()
}

static TEMPLATES: [AutomationTemplate; 5] = [
    AutomationTemplate {
        id: "sudo_password",
        name: "Sudo Password",
        description: "Auto-respond to sudo password prompt with ${password}",
        protocol_hint: "ssh",
        rules_fn: || {
            vec![
                ExpectRule::new(r"\[sudo\] password for \w+:", "${password}\n")
                    .with_priority(10)
                    .with_timeout(30_000),
            ]
        },
    },
    AutomationTemplate {
        id: "ssh_host_key",
        name: "SSH Host Key Confirmation",
        description: "Auto-accept the first-connection host key prompt (new host only)",
        protocol_hint: "ssh",
        rules_fn: || {
            vec![
                // The classic single-line prompt, with or without the
                // fingerprint variant OpenSSH added in 8.x.
                ExpectRule::new(
                    r"Are you sure you want to continue connecting \(yes/no(/\[fingerprint\])?\)\?",
                    "yes\n",
                )
                .with_priority(20),
                // Newer OpenSSH prints the accept prompt on a separate follow-up
                // line: "Please type 'yes', 'no' or the fingerprint:" (issue
                // asbru-cm#145). Answering "yes" accepts the key. This only ever
                // matches on a *first* connection — a changed key produces a
                // different, deliberately un-templated warning (see the block
                // comment below).
                ExpectRule::new(r"(?i)please type 'yes', 'no' or the fingerprint", "yes\n")
                    .with_priority(20),
            ]
        },
    },
    // NOTE ON HOST-KEY CHANGES: Ásbrú ships a template that auto-answers the
    // "REMOTE HOST IDENTIFICATION HAS CHANGED" warning and removes the offending
    // key. RustConn deliberately does not: a changed host key is the exact signal
    // a man-in-the-middle attack would produce, so silently accepting it defeats
    // the protection SSH host-key checking exists to provide. Users who trust a
    // specific rotation must handle it manually (`ssh-keygen -R host`), which
    // keeps the decision explicit rather than automated.
    AutomationTemplate {
        id: "login_prompt",
        name: "Login Prompt",
        description: "Auto-fill username and password at login/password prompts",
        protocol_hint: "",
        rules_fn: || {
            vec![
                ExpectRule::new(r"(?i)login:\s*$", "${username}\n").with_priority(10),
                ExpectRule::new(r"(?i)password:\s*$", "${password}\n").with_priority(9),
            ]
        },
    },
    AutomationTemplate {
        id: "press_enter",
        name: "Press Enter to Continue",
        description: "Auto-press Enter on 'Press Enter' / 'any key to continue' prompts",
        protocol_hint: "",
        rules_fn: || {
            // Matches "Press Enter to continue", "Press any key to continue",
            // and the bare "any key to continue" that pagers and appliance
            // banners emit without a leading "Press" (Ásbrú's default matcher).
            vec![ExpectRule::new(r"(?i)(press enter|any key) to continue", "\n").with_priority(5)]
        },
    },
    AutomationTemplate {
        id: "motd_more",
        name: "MOTD Pager (--More--)",
        description: "Auto-dismiss --More-- pager prompts",
        protocol_hint: "",
        rules_fn: || vec![ExpectRule::new(r"--More--|--more--", " ").with_priority(3)],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_templates_not_empty() {
        assert!(!builtin_templates().is_empty());
    }

    #[test]
    fn test_templates_have_valid_patterns() {
        for template in builtin_templates() {
            for rule in template.rules() {
                rule.validate_pattern().unwrap_or_else(|e| {
                    panic!("Template '{}' has invalid pattern: {e}", template.id)
                });
            }
        }
    }

    #[test]
    fn test_templates_for_ssh() {
        let ssh = templates_for_protocol("ssh");
        assert!(
            ssh.len() >= 2,
            "SSH should have sudo + host key + generic templates"
        );
        assert!(ssh.iter().any(|t| t.id == "sudo_password"));
        assert!(ssh.iter().any(|t| t.id == "ssh_host_key"));
    }

    #[test]
    fn test_templates_generate_unique_ids() {
        let template = &builtin_templates()[0];
        let rules1 = template.rules();
        let rules2 = template.rules();
        assert_ne!(
            rules1[0].id, rules2[0].id,
            "Each call should generate new UUIDs"
        );
    }

    /// Returns the compiled rules of the template with the given id.
    fn compiled_rules(id: &str) -> Vec<regex::Regex> {
        builtin_templates()
            .iter()
            .find(|t| t.id == id)
            .unwrap_or_else(|| panic!("template '{id}' must exist"))
            .rules()
            .iter()
            .map(|r| r.compile_pattern().expect("template regex must compile"))
            .collect()
    }

    #[test]
    fn host_key_template_matches_both_openssh_prompt_forms() {
        let rules = compiled_rules("ssh_host_key");
        // Classic single-line prompt (OpenSSH 7.x and the 8.x fingerprint form).
        let classic = "Are you sure you want to continue connecting (yes/no/[fingerprint])?";
        // Follow-up line newer OpenSSH prints when the first answer is invalid.
        let follow_up = "Please type 'yes', 'no' or the fingerprint:";
        assert!(
            rules.iter().any(|re| re.is_match(classic)),
            "classic host-key prompt must match"
        );
        assert!(
            rules.iter().any(|re| re.is_match(follow_up)),
            "newer 'Please type yes/no/fingerprint' prompt must match"
        );
    }

    #[test]
    fn host_key_template_ignores_a_changed_key_warning() {
        // A changed key is deliberately NOT auto-accepted — none of the
        // host-key rules may match the tampering warning.
        let rules = compiled_rules("ssh_host_key");
        let changed = "@@@ WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED! @@@";
        let offending = "Offending ECDSA key in /home/u/.ssh/known_hosts:42";
        assert!(
            !rules
                .iter()
                .any(|re| re.is_match(changed) || re.is_match(offending)),
            "a changed/offending host key must never be auto-accepted"
        );
    }

    #[test]
    fn press_enter_template_matches_bare_any_key_prompt() {
        let rules = compiled_rules("press_enter");
        for banner in [
            "Press Enter to continue",
            "Press any key to continue",
            "-- Hit any key to continue --",
        ] {
            assert!(
                rules.iter().any(|re| re.is_match(banner)),
                "'{banner}' should match the press-enter template"
            );
        }
    }
}
