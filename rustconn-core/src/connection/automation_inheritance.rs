//! Automation settings inheritance resolution.
//!
//! Resolves expect rules and post-login scripts by walking the group hierarchy
//! from a connection up to the root group. If the connection has its own
//! automation config (non-empty), it takes precedence. Otherwise, the first
//! group in the parent chain with non-empty rules is used.
//!
//! Cycle detection via `HashSet<Uuid>` ensures termination even with
//! malformed parent_id chains.

use std::collections::HashSet;

use uuid::Uuid;

use crate::automation::ExpectRule;
use crate::models::{AutomationConfig, Connection, ConnectionGroup};

/// Finds a group by ID in the slice.
fn find_group(id: Uuid, groups: &[ConnectionGroup]) -> Option<&ConnectionGroup> {
    groups.iter().find(|g| g.id == id)
}

/// Resolves the effective automation config for a connection.
///
/// # Algorithm
///
/// 1. If the connection has non-empty `expect_rules` or `post_login_scripts`,
///    return the connection's own config (no inheritance).
/// 2. Otherwise, walk the group hierarchy collecting rules from the first
///    group that has them.
/// 3. Expect rules, post-login scripts and the two login-prompt texts are
///    resolved independently — rules may come from one group and scripts from
///    another (or the same).
///
/// # Returns
///
/// The effective `AutomationConfig` combining connection-level and inherited settings.
#[must_use]
pub fn resolve_automation(connection: &Connection, groups: &[ConnectionGroup]) -> AutomationConfig {
    let expect_rules = if connection.automation.expect_rules.is_empty() {
        resolve_expect_rules(connection.group_id, groups)
    } else {
        connection.automation.expect_rules.clone()
    };

    let post_login_scripts = if connection.automation.post_login_scripts.is_empty() {
        resolve_post_login_scripts(connection.group_id, groups)
    } else {
        connection.automation.post_login_scripts.clone()
    };

    // A prompt text set on the connection wins; otherwise take the first one
    // found walking up the group chain. Each prompt is resolved on its own, so
    // a group can supply the vendor's username wording while the connection
    // overrides only the password wording.
    let username_prompt = connection
        .automation
        .username_prompt
        .clone()
        .filter(|p| !p.trim().is_empty())
        .or_else(|| resolve_prompt(connection.group_id, groups, |g| &g.username_prompt));

    let password_prompt = connection
        .automation
        .password_prompt
        .clone()
        .filter(|p| !p.trim().is_empty())
        .or_else(|| resolve_prompt(connection.group_id, groups, |g| &g.password_prompt));

    AutomationConfig {
        expect_rules,
        post_login_scripts,
        username_prompt,
        password_prompt,
        login_timeout_secs: connection
            .automation
            .login_timeout_secs
            .or_else(|| resolve_login_timeout(connection.group_id, groups)),
        // Command macros are per-connection and are not inherited from groups
        // (unlike expect_rules / prompts), so carry the connection's own list
        // through unchanged.
        command_macros: connection.automation.command_macros.clone(),
    }
}

/// Walks the group hierarchy to find an inherited login-prompt text.
///
/// `pick` selects which of the two prompt fields to read from a group. Blank
/// values are skipped so an emptied field in a subgroup does not shadow a real
/// one further up. Returns `None` when no group in the chain defines it.
fn resolve_prompt<F>(
    start_group_id: Option<Uuid>,
    groups: &[ConnectionGroup],
    pick: F,
) -> Option<String>
where
    F: Fn(&ConnectionGroup) -> &Option<String>,
{
    let mut visited = HashSet::new();
    let mut current = start_group_id;

    while let Some(gid) = current {
        if !visited.insert(gid) {
            break; // Cycle detected
        }
        let group = find_group(gid, groups)?;
        if let Some(value) = pick(group)
            && !value.trim().is_empty()
        {
            return Some(value.clone());
        }
        current = group.parent_id;
    }

    None
}

/// Walks the group hierarchy to find an inherited login timeout.
///
/// Returns the first `login_timeout_secs` that is `Some` walking up the chain.
fn resolve_login_timeout(start_group_id: Option<Uuid>, groups: &[ConnectionGroup]) -> Option<u32> {
    let mut visited = HashSet::new();
    let mut current = start_group_id;

    while let Some(gid) = current {
        if !visited.insert(gid) {
            break;
        }
        let group = find_group(gid, groups)?;
        if let Some(timeout) = group.login_timeout_secs {
            return Some(timeout);
        }
        current = group.parent_id;
    }

    None
}

/// Walks the group hierarchy to find inherited expect rules.
///
/// Returns the first non-empty `expect_rules` found in the parent chain,
/// or an empty vec if none found.
fn resolve_expect_rules(
    start_group_id: Option<Uuid>,
    groups: &[ConnectionGroup],
) -> Vec<ExpectRule> {
    let mut visited = HashSet::new();
    let mut current = start_group_id;

    while let Some(gid) = current {
        if !visited.insert(gid) {
            break; // Cycle detected
        }
        let Some(group) = find_group(gid, groups) else {
            break;
        };
        if !group.expect_rules.is_empty() {
            return group.expect_rules.clone();
        }
        current = group.parent_id;
    }

    Vec::new()
}

/// Walks the group hierarchy to find inherited post-login scripts.
///
/// Returns the first non-empty `post_login_scripts` found in the parent chain,
/// or an empty vec if none found.
fn resolve_post_login_scripts(
    start_group_id: Option<Uuid>,
    groups: &[ConnectionGroup],
) -> Vec<String> {
    let mut visited = HashSet::new();
    let mut current = start_group_id;

    while let Some(gid) = current {
        if !visited.insert(gid) {
            break; // Cycle detected
        }
        let Some(group) = find_group(gid, groups) else {
            break;
        };
        if !group.post_login_scripts.is_empty() {
            return group.post_login_scripts.clone();
        }
        current = group.parent_id;
    }

    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation::ExpectRule;
    use crate::models::{Connection, ConnectionGroup};

    fn make_rule(pattern: &str, response: &str) -> ExpectRule {
        ExpectRule::new(pattern, response)
    }

    #[test]
    fn connection_with_own_rules_does_not_inherit() {
        let group = ConnectionGroup::new("G".into());
        let mut conn = Connection::new_ssh("test".into(), "host".into(), 22);
        conn.group_id = Some(group.id);
        conn.automation.expect_rules = vec![make_rule("local", "response")];

        let mut group_with_rules = group;
        group_with_rules.expect_rules = vec![make_rule("group", "group-response")];

        let result = resolve_automation(&conn, &[group_with_rules]);
        assert_eq!(result.expect_rules.len(), 1);
        assert_eq!(result.expect_rules[0].pattern, "local");
    }

    #[test]
    fn empty_connection_inherits_from_direct_group() {
        let mut group = ConnectionGroup::new("G".into());
        group.expect_rules = vec![make_rule("password:", "secret{ENTER}")];

        let mut conn = Connection::new_ssh("test".into(), "host".into(), 22);
        conn.group_id = Some(group.id);

        let result = resolve_automation(&conn, &[group]);
        assert_eq!(result.expect_rules.len(), 1);
        assert_eq!(result.expect_rules[0].pattern, "password:");
    }

    #[test]
    fn inherits_from_grandparent_when_parent_empty() {
        let mut root = ConnectionGroup::new("Root".into());
        root.expect_rules = vec![make_rule("yes/no", "yes{ENTER}")];

        let child = ConnectionGroup::with_parent("Child".into(), root.id);

        let mut conn = Connection::new_ssh("test".into(), "host".into(), 22);
        conn.group_id = Some(child.id);

        let result = resolve_automation(&conn, &[root, child]);
        assert_eq!(result.expect_rules.len(), 1);
        assert_eq!(result.expect_rules[0].pattern, "yes/no");
    }

    #[test]
    fn cycle_detection_terminates() {
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();

        let mut group_a = ConnectionGroup::new("A".into());
        group_a.id = id_a;
        group_a.parent_id = Some(id_b);

        let mut group_b = ConnectionGroup::new("B".into());
        group_b.id = id_b;
        group_b.parent_id = Some(id_a);

        let mut conn = Connection::new_ssh("test".into(), "host".into(), 22);
        conn.group_id = Some(id_a);

        let result = resolve_automation(&conn, &[group_a, group_b]);
        assert!(result.expect_rules.is_empty());
    }

    #[test]
    fn post_login_scripts_inherited_independently() {
        let mut group = ConnectionGroup::new("G".into());
        group.post_login_scripts = vec!["echo hello".into()];
        // No expect_rules on group

        let mut conn = Connection::new_ssh("test".into(), "host".into(), 22);
        conn.group_id = Some(group.id);
        conn.automation.expect_rules = vec![make_rule("local", "resp")];
        // No post_login_scripts on connection

        let result = resolve_automation(&conn, &[group]);
        // Expect rules from connection (non-empty)
        assert_eq!(result.expect_rules.len(), 1);
        assert_eq!(result.expect_rules[0].pattern, "local");
        // Post-login scripts inherited from group
        assert_eq!(result.post_login_scripts, vec!["echo hello".to_string()]);
    }

    #[test]
    fn no_group_returns_empty() {
        let mut conn = Connection::new_ssh("test".into(), "host".into(), 22);
        conn.group_id = None;

        let result = resolve_automation(&conn, &[]);
        assert!(result.expect_rules.is_empty());
        assert!(result.post_login_scripts.is_empty());
        assert!(result.username_prompt.is_none());
        assert!(result.password_prompt.is_none());
    }

    #[test]
    fn login_prompts_inherited_from_group() {
        let mut group = ConnectionGroup::new("Huawei OLT".into());
        group.username_prompt = Some(">>User name:".into());
        group.password_prompt = Some(">>User password:".into());

        let mut conn = Connection::new_telnet("olt-1".into(), "10.0.0.1".into(), 23);
        conn.group_id = Some(group.id);

        let result = resolve_automation(&conn, &[group]);
        assert_eq!(result.username_prompt.as_deref(), Some(">>User name:"));
        assert_eq!(result.password_prompt.as_deref(), Some(">>User password:"));
    }

    #[test]
    fn connection_login_prompt_overrides_group() {
        let mut group = ConnectionGroup::new("Vendor".into());
        group.username_prompt = Some("Username:".into());
        group.password_prompt = Some("Password:".into());

        let mut conn = Connection::new_telnet("switch".into(), "10.0.0.2".into(), 23);
        conn.group_id = Some(group.id);
        conn.automation.username_prompt = Some("login:".into());

        let result = resolve_automation(&conn, &[group]);
        // Own username prompt wins, password prompt still inherited.
        assert_eq!(result.username_prompt.as_deref(), Some("login:"));
        assert_eq!(result.password_prompt.as_deref(), Some("Password:"));
    }

    #[test]
    fn blank_login_prompt_does_not_shadow_the_group() {
        let mut group = ConnectionGroup::new("Vendor".into());
        group.username_prompt = Some("User name:".into());

        let mut conn = Connection::new_telnet("switch".into(), "10.0.0.3".into(), 23);
        conn.group_id = Some(group.id);
        conn.automation.username_prompt = Some("   ".into());

        let result = resolve_automation(&conn, &[group]);
        assert_eq!(result.username_prompt.as_deref(), Some("User name:"));
    }

    #[test]
    fn login_prompt_inherited_from_grandparent() {
        let mut root = ConnectionGroup::new("Root".into());
        root.username_prompt = Some("User name:".into());
        let child = ConnectionGroup::with_parent("Child".into(), root.id);

        let mut conn = Connection::new_telnet("olt".into(), "10.0.0.4".into(), 23);
        conn.group_id = Some(child.id);

        let result = resolve_automation(&conn, &[root, child]);
        assert_eq!(result.username_prompt.as_deref(), Some("User name:"));
    }

    #[test]
    fn login_prompt_cycle_detection_terminates() {
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();

        let mut group_a = ConnectionGroup::new("A".into());
        group_a.id = id_a;
        group_a.parent_id = Some(id_b);

        let mut group_b = ConnectionGroup::new("B".into());
        group_b.id = id_b;
        group_b.parent_id = Some(id_a);

        let mut conn = Connection::new_telnet("t".into(), "10.0.0.5".into(), 23);
        conn.group_id = Some(id_a);

        let result = resolve_automation(&conn, &[group_a, group_b]);
        assert!(result.username_prompt.is_none());
    }
}
