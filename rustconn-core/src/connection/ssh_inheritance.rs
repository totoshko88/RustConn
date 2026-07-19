//! SSH settings inheritance resolution.
//!
//! Resolves SSH settings (key path, auth method, proxy jump, agent socket)
//! by walking the group hierarchy from a connection up to the root group.
//! Each function checks the connection-level setting first, then walks
//! the parent chain returning the first `Some` value found.
//!
//! Cycle detection via `HashSet<Uuid>` ensures termination even with
//! malformed parent_id chains.

use std::collections::HashSet;
use std::path::PathBuf;

use uuid::Uuid;

use crate::models::{Connection, ConnectionGroup, ProtocolConfig, SshAuthMethod, SshKeySource};

/// Finds a group by ID in the slice.
fn find_group(id: Uuid, groups: &[ConnectionGroup]) -> Option<&ConnectionGroup> {
    groups.iter().find(|g| g.id == id)
}

/// Walks the group hierarchy starting from `start_group_id`, calling `extract`
/// on each group. Returns the first `Some` value, or `None` if the chain is
/// exhausted or a cycle is detected.
fn walk_group_chain<T>(
    start_group_id: Option<Uuid>,
    groups: &[ConnectionGroup],
    extract: impl Fn(&ConnectionGroup) -> Option<T>,
) -> Option<T> {
    let mut visited = HashSet::new();
    let mut current = start_group_id;

    while let Some(gid) = current {
        if !visited.insert(gid) {
            // Cycle detected
            return None;
        }
        let group = find_group(gid, groups)?;
        if let Some(value) = extract(group) {
            return Some(value);
        }
        current = group.parent_id;
    }

    None
}

/// Extracts the `SshConfig` from a connection's `protocol_config`, if it is
/// an SSH or SFTP variant.
fn ssh_config(connection: &Connection) -> Option<&crate::models::SshConfig> {
    match &connection.protocol_config {
        ProtocolConfig::Ssh(cfg) | ProtocolConfig::Sftp(cfg) => Some(cfg),
        _ => None,
    }
}

/// Resolves the SSH key path for a connection by checking the connection-level
/// setting first, then walking the group hierarchy.
///
/// Returns `Some(path)` if a key file path is found, `None` otherwise.
///
/// # Algorithm
///
/// 1. If the connection has `key_source = File { path }` → return `Some(path)`
/// 2. If `key_source = Agent` or `Default` → return `None` (no file-based key)
/// 3. If `key_source = Inherit` → walk the group chain for `ssh_key_path`
/// 4. If no SSH config exists on the connection → walk the group chain
#[must_use]
pub fn resolve_ssh_key_path(
    connection: &Connection,
    groups: &[ConnectionGroup],
) -> Option<PathBuf> {
    if let Some(cfg) = ssh_config(connection) {
        match &cfg.key_source {
            SshKeySource::File { path } if !path.as_os_str().is_empty() => {
                return Some(path.clone());
            }
            SshKeySource::Agent { .. } => return None,
            SshKeySource::Default => {
                // Legacy connections store key in key_path with key_source=Default
                if let Some(ref key_path) = cfg.key_path
                    && !key_path.as_os_str().is_empty()
                {
                    return Some(key_path.clone());
                }
                return None;
            }
            SshKeySource::Inherit | SshKeySource::File { .. } => {
                // Fall through to group chain walk
            }
        }
    }

    walk_group_chain(connection.group_id, groups, |g| g.ssh_key_path.clone())
}

/// Resolves the SSH authentication method for a connection.
///
/// Returns the connection-level auth method if set and not `Inherit`,
/// otherwise walks the group chain. Falls back to `SshAuthMethod::default()`
/// (Password) if nothing is found.
#[must_use]
pub fn resolve_ssh_auth_method(
    connection: &Connection,
    groups: &[ConnectionGroup],
) -> SshAuthMethod {
    if let Some(cfg) = ssh_config(connection) {
        // If key_source is not Inherit, the connection has its own auth method
        if !matches!(cfg.key_source, SshKeySource::Inherit) {
            return cfg.auth_method.clone();
        }
    }

    walk_group_chain(connection.group_id, groups, |g| g.ssh_auth_method.clone()).unwrap_or_default()
}

/// Resolves the SSH proxy jump setting for a connection.
///
/// Checks the connection's SSH `proxy_jump` first, then walks the group chain
/// for `ssh_proxy_jump`.
#[must_use]
pub fn resolve_ssh_proxy_jump(
    connection: &Connection,
    groups: &[ConnectionGroup],
) -> Option<String> {
    if let Some(cfg) = ssh_config(connection) {
        if cfg.proxy_jump.is_some() {
            return cfg.proxy_jump.clone();
        }
        // Only walk groups if key_source is Inherit (connection delegates to groups)
        if !matches!(cfg.key_source, SshKeySource::Inherit) {
            return None;
        }
    }

    walk_group_chain(connection.group_id, groups, |g| g.ssh_proxy_jump.clone())
}

/// Resolves the SSH jump host connection ID for a connection.
///
/// Checks the connection-level `jump_host_id` first, then walks the group
/// chain for `ssh_jump_host_id`. Connection-level takes precedence.
#[must_use]
pub fn resolve_ssh_jump_host_id(
    connection: &Connection,
    groups: &[ConnectionGroup],
) -> Option<uuid::Uuid> {
    // Check connection-level jump_host_id first
    if let Some(cfg) = ssh_config(connection) {
        if cfg.jump_host_id.is_some() {
            return cfg.jump_host_id;
        }
        // Only walk groups if key_source is Inherit
        if !matches!(cfg.key_source, SshKeySource::Inherit) {
            return None;
        }
    }

    walk_group_chain(connection.group_id, groups, |g| g.ssh_jump_host_id)
}

/// Resolves the SSH agent socket path for a connection.
///
/// Checks the connection's SSH `ssh_agent_socket` first, then walks the group
/// chain for `ssh_agent_socket`.
#[must_use]
pub fn resolve_ssh_agent_socket(
    connection: &Connection,
    groups: &[ConnectionGroup],
) -> Option<String> {
    if let Some(cfg) = ssh_config(connection) {
        if cfg.ssh_agent_socket.is_some() {
            return cfg.ssh_agent_socket.clone();
        }
        // Only walk groups if key_source is Inherit
        if !matches!(cfg.key_source, SshKeySource::Inherit) {
            return None;
        }
    }

    walk_group_chain(connection.group_id, groups, |g| g.ssh_agent_socket.clone())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::models::{Connection, ConnectionGroup, ProtocolConfig, SshAuthMethod, SshKeySource};

    /// Helper: create an SSH connection with Inherit key_source, linked to a group.
    fn ssh_conn_inherit(group_id: Uuid) -> Connection {
        let mut conn = Connection::new_ssh("test".into(), "host".into(), 22);
        conn.group_id = Some(group_id);
        if let ProtocolConfig::Ssh(ref mut cfg) = conn.protocol_config {
            cfg.key_source = SshKeySource::Inherit;
        }
        conn
    }

    // ── 1. Three-level nesting: key set on root, inherited through middle to leaf ──

    #[test]
    fn key_path_inherited_through_three_levels() {
        let mut group_a = ConnectionGroup::new("A".into());
        group_a.ssh_key_path = Some(PathBuf::from("/keys/root"));

        let group_b = ConnectionGroup::with_parent("B".into(), group_a.id);
        let group_c = ConnectionGroup::with_parent("C".into(), group_b.id);

        let conn = ssh_conn_inherit(group_c.id);
        let groups = vec![group_a, group_b, group_c];

        assert_eq!(
            resolve_ssh_key_path(&conn, &groups),
            Some(PathBuf::from("/keys/root"))
        );
    }

    #[test]
    fn auth_method_inherited_through_three_levels() {
        let mut group_a = ConnectionGroup::new("A".into());
        group_a.ssh_auth_method = Some(SshAuthMethod::PublicKey);

        let group_b = ConnectionGroup::with_parent("B".into(), group_a.id);
        let group_c = ConnectionGroup::with_parent("C".into(), group_b.id);

        let conn = ssh_conn_inherit(group_c.id);
        let groups = vec![group_a, group_b, group_c];

        assert_eq!(
            resolve_ssh_auth_method(&conn, &groups),
            SshAuthMethod::PublicKey
        );
    }

    #[test]
    fn proxy_jump_inherited_through_three_levels() {
        let mut group_a = ConnectionGroup::new("A".into());
        group_a.ssh_proxy_jump = Some("bastion.example.com".into());

        let group_b = ConnectionGroup::with_parent("B".into(), group_a.id);
        let group_c = ConnectionGroup::with_parent("C".into(), group_b.id);

        let conn = ssh_conn_inherit(group_c.id);
        let groups = vec![group_a, group_b, group_c];

        assert_eq!(
            resolve_ssh_proxy_jump(&conn, &groups),
            Some("bastion.example.com".into())
        );
    }

    #[test]
    fn agent_socket_inherited_through_three_levels() {
        let mut group_a = ConnectionGroup::new("A".into());
        group_a.ssh_agent_socket = Some("/tmp/agent.sock".into());

        let group_b = ConnectionGroup::with_parent("B".into(), group_a.id);
        let group_c = ConnectionGroup::with_parent("C".into(), group_b.id);

        let conn = ssh_conn_inherit(group_c.id);
        let groups = vec![group_a, group_b, group_c];

        assert_eq!(
            resolve_ssh_agent_socket(&conn, &groups),
            Some("/tmp/agent.sock".into())
        );
    }

    // ── 2. Missing parent: group_id references a non-existent group ──

    #[test]
    fn missing_parent_returns_none_for_key_path() {
        let missing_id = Uuid::new_v4();
        let conn = ssh_conn_inherit(missing_id);

        assert_eq!(resolve_ssh_key_path(&conn, &[]), None);
    }

    #[test]
    fn missing_parent_returns_default_for_auth_method() {
        let missing_id = Uuid::new_v4();
        let conn = ssh_conn_inherit(missing_id);

        assert_eq!(
            resolve_ssh_auth_method(&conn, &[]),
            SshAuthMethod::default()
        );
    }

    #[test]
    fn missing_parent_returns_none_for_proxy_jump() {
        let missing_id = Uuid::new_v4();
        let conn = ssh_conn_inherit(missing_id);

        assert_eq!(resolve_ssh_proxy_jump(&conn, &[]), None);
    }

    #[test]
    fn missing_parent_returns_none_for_agent_socket() {
        let missing_id = Uuid::new_v4();
        let conn = ssh_conn_inherit(missing_id);

        assert_eq!(resolve_ssh_agent_socket(&conn, &[]), None);
    }

    // ── 3. No key in chain: all groups have None ──

    #[test]
    fn no_key_in_chain_returns_none() {
        let group_a = ConnectionGroup::new("A".into());
        let group_b = ConnectionGroup::with_parent("B".into(), group_a.id);
        let group_c = ConnectionGroup::with_parent("C".into(), group_b.id);

        let conn = ssh_conn_inherit(group_c.id);
        let groups = vec![group_a, group_b, group_c];

        assert_eq!(resolve_ssh_key_path(&conn, &groups), None);
    }

    #[test]
    fn no_auth_method_in_chain_returns_default() {
        let group_a = ConnectionGroup::new("A".into());
        let group_b = ConnectionGroup::with_parent("B".into(), group_a.id);

        let conn = ssh_conn_inherit(group_b.id);
        let groups = vec![group_a, group_b];

        assert_eq!(
            resolve_ssh_auth_method(&conn, &groups),
            SshAuthMethod::default()
        );
    }

    // ── 4. Direct connection setting: File key_source returns path directly ──

    #[test]
    fn direct_file_key_source_returns_path() {
        let mut group = ConnectionGroup::new("G".into());
        group.ssh_key_path = Some(PathBuf::from("/group/key"));

        let mut conn = Connection::new_ssh("test".into(), "host".into(), 22);
        conn.group_id = Some(group.id);
        if let ProtocolConfig::Ssh(ref mut cfg) = conn.protocol_config {
            cfg.key_source = SshKeySource::File {
                path: PathBuf::from("/my/key"),
            };
        }

        let groups = vec![group];

        // Connection-level File key takes precedence over group
        assert_eq!(
            resolve_ssh_key_path(&conn, &groups),
            Some(PathBuf::from("/my/key"))
        );
    }

    // ── 5. Middle of chain: key set on B, not on root A ──

    #[test]
    fn key_found_at_middle_group() {
        let group_a = ConnectionGroup::new("A".into());
        let mut group_b = ConnectionGroup::with_parent("B".into(), group_a.id);
        group_b.ssh_key_path = Some(PathBuf::from("/keys/middle"));

        let group_c = ConnectionGroup::with_parent("C".into(), group_b.id);

        let conn = ssh_conn_inherit(group_c.id);
        let groups = vec![group_a, group_b, group_c];

        assert_eq!(
            resolve_ssh_key_path(&conn, &groups),
            Some(PathBuf::from("/keys/middle"))
        );
    }

    #[test]
    fn proxy_jump_found_at_middle_group() {
        let group_a = ConnectionGroup::new("A".into());
        let mut group_b = ConnectionGroup::with_parent("B".into(), group_a.id);
        group_b.ssh_proxy_jump = Some("jump-host".into());

        let group_c = ConnectionGroup::with_parent("C".into(), group_b.id);

        let conn = ssh_conn_inherit(group_c.id);
        let groups = vec![group_a, group_b, group_c];

        assert_eq!(
            resolve_ssh_proxy_jump(&conn, &groups),
            Some("jump-host".into())
        );
    }

    // ── 6. Cycle detection: A → B → A terminates without infinite loop ──

    #[test]
    fn cycle_detection_terminates_for_key_path() {
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();

        let mut group_a = ConnectionGroup::new("A".into());
        group_a.id = id_a;
        group_a.parent_id = Some(id_b);

        let mut group_b = ConnectionGroup::new("B".into());
        group_b.id = id_b;
        group_b.parent_id = Some(id_a);

        let conn = ssh_conn_inherit(id_a);
        let groups = vec![group_a, group_b];

        // Should terminate and return None (no key set, cycle detected)
        assert_eq!(resolve_ssh_key_path(&conn, &groups), None);
    }

    #[test]
    fn cycle_detection_terminates_for_auth_method() {
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();

        let mut group_a = ConnectionGroup::new("A".into());
        group_a.id = id_a;
        group_a.parent_id = Some(id_b);

        let mut group_b = ConnectionGroup::new("B".into());
        group_b.id = id_b;
        group_b.parent_id = Some(id_a);

        let conn = ssh_conn_inherit(id_a);
        let groups = vec![group_a, group_b];

        // Should terminate and return default
        assert_eq!(
            resolve_ssh_auth_method(&conn, &groups),
            SshAuthMethod::default()
        );
    }

    #[test]
    fn cycle_detection_terminates_for_proxy_jump() {
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();

        let mut group_a = ConnectionGroup::new("A".into());
        group_a.id = id_a;
        group_a.parent_id = Some(id_b);

        let mut group_b = ConnectionGroup::new("B".into());
        group_b.id = id_b;
        group_b.parent_id = Some(id_a);

        let conn = ssh_conn_inherit(id_a);
        let groups = vec![group_a, group_b];

        assert_eq!(resolve_ssh_proxy_jump(&conn, &groups), None);
    }

    #[test]
    fn cycle_detection_terminates_for_agent_socket() {
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();

        let mut group_a = ConnectionGroup::new("A".into());
        group_a.id = id_a;
        group_a.parent_id = Some(id_b);

        let mut group_b = ConnectionGroup::new("B".into());
        group_b.id = id_b;
        group_b.parent_id = Some(id_a);

        let conn = ssh_conn_inherit(id_a);
        let groups = vec![group_a, group_b];

        assert_eq!(resolve_ssh_agent_socket(&conn, &groups), None);
    }

    // ── 7. Agent key source: returns None (agent handles keys) ──

    #[test]
    fn agent_key_source_returns_none() {
        let mut group = ConnectionGroup::new("G".into());
        group.ssh_key_path = Some(PathBuf::from("/group/key"));

        let mut conn = Connection::new_ssh("test".into(), "host".into(), 22);
        conn.group_id = Some(group.id);
        if let ProtocolConfig::Ssh(ref mut cfg) = conn.protocol_config {
            cfg.key_source = SshKeySource::Agent {
                fingerprint: "SHA256:abc".into(),
                comment: "my-key".into(),
            };
        }

        let groups = vec![group];

        assert_eq!(resolve_ssh_key_path(&conn, &groups), None);
    }

    // ── 8. Default key source: returns None ──

    #[test]
    fn default_key_source_returns_none() {
        let mut group = ConnectionGroup::new("G".into());
        group.ssh_key_path = Some(PathBuf::from("/group/key"));

        let mut conn = Connection::new_ssh("test".into(), "host".into(), 22);
        conn.group_id = Some(group.id);
        if let ProtocolConfig::Ssh(ref mut cfg) = conn.protocol_config {
            cfg.key_source = SshKeySource::Default;
        }

        let groups = vec![group];

        assert_eq!(resolve_ssh_key_path(&conn, &groups), None);
    }
}
