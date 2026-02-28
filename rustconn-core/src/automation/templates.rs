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
        description: "Auto-accept SSH host key fingerprint prompt",
        protocol_hint: "ssh",
        rules_fn: || {
            vec![
                ExpectRule::new(
                    r"Are you sure you want to continue connecting \(yes/no(/\[fingerprint\])?\)\?",
                    "yes\n",
                )
                .with_priority(20),
            ]
        },
    },
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
        description: "Auto-press Enter on 'Press Enter' or 'Press any key' prompts",
        protocol_hint: "",
        rules_fn: || {
            vec![ExpectRule::new(r"(?i)press (enter|any key) to continue", "\n").with_priority(5)]
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
}
