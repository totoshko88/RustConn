//! Dynamic folder executor for script-generated connections.
//!
//! Executes a user-defined script and parses the JSON output into
//! [`DynamicConnectionEntry`] objects that become read-only connections
//! inside a group.

use std::process::Stdio;
use std::time::Instant;

use thiserror::Error;
use uuid::Uuid;

use crate::models::{
    Connection, DynamicConnectionEntry, DynamicFolderConfig, DynamicFolderResult, ProtocolType,
};

/// Errors from dynamic folder operations
#[derive(Debug, Error)]
pub enum DynamicFolderError {
    /// Script execution failed
    #[error("Script execution failed: {0}")]
    ExecutionFailed(String),

    /// Script timed out
    #[error("Script timed out after {0} seconds")]
    Timeout(u64),

    /// Script returned non-zero exit code
    #[error("Script exited with code {code}: {stderr}")]
    NonZeroExit {
        /// Exit code
        code: i32,
        /// Stderr output
        stderr: String,
    },

    /// Failed to parse script output as JSON
    #[error("Failed to parse script output: {0}")]
    ParseError(String),

    /// Script produced empty output
    #[error("Script produced no output")]
    EmptyOutput,

    /// I/O error
    #[error("I/O error: {0}")]
    Io(String),
}

/// Result type for dynamic folder operations
pub type DynamicFolderResult2 = Result<DynamicFolderResult, DynamicFolderError>;

/// Executes a dynamic folder script and parses the output.
///
/// The script is run via `sh -c` and must output a JSON array of
/// [`DynamicConnectionEntry`] objects to stdout.
///
/// # Errors
///
/// Returns an error if the script fails, times out, or produces invalid output.
pub async fn execute_script(config: &DynamicFolderConfig) -> DynamicFolderResult2 {
    let start = Instant::now();

    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c").arg(&config.script);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    // Prevent the script from inheriting stdin
    cmd.stdin(Stdio::null());

    if let Some(ref dir) = config.working_directory {
        cmd.current_dir(dir);
    }

    let child = cmd
        .spawn()
        .map_err(|e| DynamicFolderError::Io(e.to_string()))?;

    let output = tokio::time::timeout(config.timeout(), child.wait_with_output())
        .await
        .map_err(|_| DynamicFolderError::Timeout(config.timeout_secs))?
        .map_err(|e| DynamicFolderError::ExecutionFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let code = output.status.code().unwrap_or(-1);
        return Err(DynamicFolderError::NonZeroExit { code, stderr });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stdout = stdout.trim();

    if stdout.is_empty() {
        return Err(DynamicFolderError::EmptyOutput);
    }

    let (entries, warnings) = parse_entries(stdout)?;
    let duration = start.elapsed();

    Ok(DynamicFolderResult {
        entries,
        warnings,
        duration,
    })
}

/// Parses JSON output into connection entries with validation warnings.
fn parse_entries(
    json: &str,
) -> Result<(Vec<DynamicConnectionEntry>, Vec<String>), DynamicFolderError> {
    let raw_entries: Vec<DynamicConnectionEntry> =
        serde_json::from_str(json).map_err(|e| DynamicFolderError::ParseError(e.to_string()))?;

    let mut entries = Vec::with_capacity(raw_entries.len());
    let mut warnings = Vec::new();

    for (i, entry) in raw_entries.into_iter().enumerate() {
        if entry.name.trim().is_empty() {
            warnings.push(format!("Entry {i}: skipped — empty name"));
            continue;
        }
        if entry.host.trim().is_empty() {
            warnings.push(format!("Entry {i} ({}): skipped — empty host", entry.name));
            continue;
        }
        entries.push(entry);
    }

    Ok((entries, warnings))
}

/// Converts a [`DynamicConnectionEntry`] into a [`Connection`] assigned to the given group.
///
/// The connection gets a deterministic UUID derived from the group ID and entry content,
/// so repeated refreshes produce stable IDs for the same entries.
#[must_use]
pub fn entry_to_connection(entry: &DynamicConnectionEntry, group_id: Uuid) -> Connection {
    let id = stable_connection_id(group_id, entry);

    let protocol_type = match entry.protocol.to_lowercase().as_str() {
        "ssh" => ProtocolType::Ssh,
        "rdp" => ProtocolType::Rdp,
        "vnc" => ProtocolType::Vnc,
        "spice" => ProtocolType::Spice,
        "telnet" => ProtocolType::Telnet,
        "mosh" => ProtocolType::Mosh,
        _ => ProtocolType::Ssh,
    };

    let default_port = match protocol_type {
        ProtocolType::Ssh | ProtocolType::Sftp | ProtocolType::Mosh => 22,
        ProtocolType::Rdp => 3389,
        ProtocolType::Vnc | ProtocolType::Spice => 5900,
        ProtocolType::Telnet => 23,
        _ => 22,
    };
    let port = entry.port.unwrap_or(default_port);

    let mut conn = match protocol_type {
        ProtocolType::Ssh => Connection::new_ssh(entry.name.clone(), entry.host.clone(), port),
        ProtocolType::Rdp => Connection::new_rdp(entry.name.clone(), entry.host.clone(), port),
        ProtocolType::Vnc => Connection::new_vnc(entry.name.clone(), entry.host.clone(), port),
        ProtocolType::Spice => Connection::new_spice(entry.name.clone(), entry.host.clone(), port),
        ProtocolType::Telnet => {
            Connection::new_telnet(entry.name.clone(), entry.host.clone(), port)
        }
        ProtocolType::Mosh => Connection::new_mosh(entry.name.clone(), entry.host.clone(), port),
        _ => Connection::new_ssh(entry.name.clone(), entry.host.clone(), port),
    };

    conn.id = id;
    conn.group_id = Some(group_id);
    conn.username = entry.username.clone();
    conn.tags = entry.tags.clone();
    conn.description = entry.description.clone();
    conn.is_dynamic = true;

    conn
}

/// Generates a stable UUID for a dynamic connection entry.
///
/// The UUID is deterministic based on group_id + name + host + protocol,
/// so repeated refreshes produce stable IDs for the same entries.
#[must_use]
pub fn stable_connection_id(group_id: Uuid, entry: &DynamicConnectionEntry) -> Uuid {
    // UUID v5 (SHA-1, namespaced) is spec-defined and stable across Rust
    // versions and toolchains — unlike DefaultHasher, whose output may change
    // between releases. The group acts as the namespace; the entry's
    // identity-defining fields form the name. NUL separators keep fields from
    // bleeding into each other (e.g. "ab"+"c" vs "a"+"bc").
    let key = format!("{}\u{0}{}\u{0}{}", entry.name, entry.host, entry.protocol);
    Uuid::new_v5(&group_id, key.as_bytes())
}

/// Generates a stable UUID for a dynamic sub-group.
///
/// Derived from the parent group's id (as the namespace) and the segment name,
/// so the same `group` path in a dynamic entry resolves to the same sub-group
/// across refreshes. This is what lets the refresh be idempotent: it never
/// creates a second "web-servers" folder next to the one it made last time.
#[must_use]
pub fn stable_subgroup_id(parent_id: Uuid, segment: &str) -> Uuid {
    Uuid::new_v5(&parent_id, segment.as_bytes())
}

/// The connections and sub-groups a dynamic-folder refresh should materialise.
///
/// A refresh is "delete the folder's old dynamic content, then apply this plan".
/// Producing it in core keeps the sub-group-path logic in one place instead of
/// duplicated across the GUI and CLI refresh paths.
#[derive(Debug, Default)]
pub struct DynamicRefreshPlan {
    /// Sub-groups that must exist before the connections are created, ordered
    /// parent-before-child so a caller can create them in sequence. Already
    /// deduplicated; a caller still skips any that already exist by id.
    pub subgroups: Vec<crate::models::ConnectionGroup>,
    /// The dynamic connections, each already assigned to its target group
    /// (the base folder, or a sub-group when the entry carried a `group` path).
    pub connections: Vec<Connection>,
}

/// Builds the [`DynamicRefreshPlan`] for a dynamic folder.
///
/// Each entry's optional `group` field is a `/`-separated path *relative to the
/// base folder* (e.g. `web-servers/production`). Empty path segments are
/// ignored, so `a//b`, a leading or trailing `/`, and a blank string all behave
/// sensibly. Sub-group ids are derived deterministically with
/// [`stable_subgroup_id`], so the plan is idempotent across refreshes.
///
/// The connections keep the stable id scheme of [`entry_to_connection`], namespaced
/// by their *target* group, so moving an entry between sub-groups changes its id
/// (it is a different connection in a different folder) but re-listing it in the
/// same place does not.
#[must_use]
pub fn plan_dynamic_refresh(
    base_group_id: Uuid,
    entries: &[DynamicConnectionEntry],
) -> DynamicRefreshPlan {
    use std::collections::HashSet;

    let mut subgroups: Vec<crate::models::ConnectionGroup> = Vec::new();
    let mut seen_subgroups: HashSet<Uuid> = HashSet::new();
    let mut connections = Vec::with_capacity(entries.len());

    for entry in entries {
        // Resolve the target group, creating each path segment's sub-group.
        let mut parent_id = base_group_id;
        if let Some(ref path) = entry.group {
            for segment in path.split('/').map(str::trim).filter(|s| !s.is_empty()) {
                let sub_id = stable_subgroup_id(parent_id, segment);
                if seen_subgroups.insert(sub_id) {
                    let mut group =
                        crate::models::ConnectionGroup::with_parent(segment.to_string(), parent_id);
                    group.id = sub_id;
                    subgroups.push(group);
                }
                parent_id = sub_id;
            }
        }
        connections.push(entry_to_connection(entry, parent_id));
    }

    DynamicRefreshPlan {
        subgroups,
        connections,
    }
}

/// Returns the ids of refresh-created sub-groups under `base` that are now empty.
///
/// A refresh deletes its old connections and re-creates them from the entries'
/// `group` paths. When an entry stops naming a path, or names a different one,
/// the sub-group it used to live in is left behind empty — so a folder whose
/// upstream reorganises accumulates dead folders forever.
///
/// Only sub-groups the refresh itself created are candidates, identified by the
/// property that makes the refresh idempotent in the first place: their id equals
/// [`stable_subgroup_id`] of their parent and name. A folder the *user* made by
/// hand inside a dynamic folder has a random id, does not match, and is never
/// touched however empty it is — deleting a user's folder to tidy up is not this
/// function's business.
///
/// Results are ordered deepest-first, so deleting them in order empties a parent
/// before it is itself considered — a two-level path (`web/production`) whose
/// entries all disappeared is removed in full by one pass.
#[must_use]
pub fn empty_dynamic_subgroup_ids(
    base: Uuid,
    groups: &[crate::models::ConnectionGroup],
    connections: &[Connection],
) -> Vec<Uuid> {
    // Depth of each descendant, so the sweep can run bottom-up.
    let mut depth: std::collections::HashMap<Uuid, usize> = std::collections::HashMap::new();
    depth.insert(base, 0);
    for id in descendant_group_ids(base, groups) {
        if id == base {
            continue;
        }
        // `descendant_group_ids` is breadth-first, so a parent's depth is already
        // known by the time its child is reached.
        if let Some(group) = groups.iter().find(|g| g.id == id)
            && let Some(parent) = group.parent_id
            && let Some(parent_depth) = depth.get(&parent).copied()
        {
            depth.insert(id, parent_depth + 1);
        }
    }

    let mut candidates: Vec<(usize, Uuid)> = Vec::new();
    for group in groups {
        let Some(&group_depth) = depth.get(&group.id) else {
            continue;
        };
        if group.id == base {
            continue;
        }
        let Some(parent) = group.parent_id else {
            continue;
        };
        // Created by a refresh, not by the user.
        if stable_subgroup_id(parent, &group.name) != group.id {
            continue;
        }
        candidates.push((group_depth, group.id));
    }

    // Deepest first, so a child is considered before the parent it empties.
    candidates.sort_by_key(|(group_depth, _)| std::cmp::Reverse(*group_depth));

    let mut removed: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
    let mut result = Vec::new();
    for (_, id) in candidates {
        let has_connection = connections.iter().any(|c| c.group_id == Some(id));
        let has_live_child = groups
            .iter()
            .any(|g| g.parent_id == Some(id) && !removed.contains(&g.id));
        if !has_connection && !has_live_child {
            removed.insert(id);
            result.push(id);
        }
    }
    result
}

/// Collects `base` and every group descended from it.
///
/// A dynamic refresh removes its old connections before applying the new plan.
/// Once entries can land in sub-groups, "the folder's connections" are no longer
/// only those directly under the base group — they can be under any descendant
/// sub-group the previous refresh created. This returns the whole subtree so the
/// caller deletes stale dynamic connections wherever a refresh may have put them.
#[must_use]
pub fn descendant_group_ids(base: Uuid, groups: &[crate::models::ConnectionGroup]) -> Vec<Uuid> {
    let mut result = vec![base];
    let mut i = 0;
    // Breadth-first over the parent pointers. Bounded by the group count, so a
    // malformed parent cycle cannot loop forever.
    while i < result.len() {
        let current = result[i];
        for g in groups {
            if g.parent_id == Some(current) && !result.contains(&g.id) {
                result.push(g.id);
            }
        }
        i += 1;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_entries_valid() {
        let json = r#"[
            {"name": "web-01", "host": "10.0.0.1"},
            {"name": "web-02", "host": "10.0.0.2", "port": 2222, "username": "admin"}
        ]"#;

        let (entries, warnings) = parse_entries(json).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(warnings.is_empty());
        assert_eq!(entries[0].name, "web-01");
        assert_eq!(entries[0].protocol, "ssh");
        assert_eq!(entries[1].port, Some(2222));
        assert_eq!(entries[1].username.as_deref(), Some("admin"));
    }

    #[test]
    fn test_parse_entries_skips_invalid() {
        let json = r#"[
            {"name": "", "host": "10.0.0.1"},
            {"name": "valid", "host": ""},
            {"name": "good", "host": "10.0.0.3"}
        ]"#;

        let (entries, warnings) = parse_entries(json).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "good");
        assert_eq!(warnings.len(), 2);
    }

    #[test]
    fn test_parse_entries_invalid_json() {
        let result = parse_entries("not json");
        assert!(result.is_err());
    }

    #[test]
    fn test_stable_id_deterministic() {
        let group_id = Uuid::new_v4();
        let entry = DynamicConnectionEntry {
            name: "test".to_string(),
            host: "10.0.0.1".to_string(),
            port: None,
            protocol: "ssh".to_string(),
            username: None,
            group: None,
            tags: Vec::new(),
            description: None,
        };

        let id1 = stable_connection_id(group_id, &entry);
        let id2 = stable_connection_id(group_id, &entry);
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_stable_id_differs_for_different_entries() {
        let group_id = Uuid::new_v4();
        let entry1 = DynamicConnectionEntry {
            name: "web-01".to_string(),
            host: "10.0.0.1".to_string(),
            port: None,
            protocol: "ssh".to_string(),
            username: None,
            group: None,
            tags: Vec::new(),
            description: None,
        };
        let entry2 = DynamicConnectionEntry {
            name: "web-02".to_string(),
            host: "10.0.0.2".to_string(),
            port: None,
            protocol: "ssh".to_string(),
            username: None,
            group: None,
            tags: Vec::new(),
            description: None,
        };

        let id1 = stable_connection_id(group_id, &entry1);
        let id2 = stable_connection_id(group_id, &entry2);
        assert_ne!(id1, id2);
    }

    fn entry_in(name: &str, host: &str, group: Option<&str>) -> DynamicConnectionEntry {
        DynamicConnectionEntry {
            name: name.to_string(),
            host: host.to_string(),
            port: None,
            protocol: "ssh".to_string(),
            username: None,
            group: group.map(str::to_string),
            tags: Vec::new(),
            description: None,
        }
    }

    #[test]
    fn plan_places_entries_in_stable_subgroups() {
        let base = Uuid::new_v4();
        let entries = vec![
            entry_in("web-01", "10.0.0.1", Some("web/prod")),
            entry_in("web-02", "10.0.0.2", Some("web/prod")),
            entry_in("db-01", "10.0.1.1", Some("db")),
            entry_in("flat", "10.0.2.1", None),
        ];

        let plan = plan_dynamic_refresh(base, &entries);

        // "web", "web/prod", "db" — three sub-groups, deduplicated.
        assert_eq!(plan.subgroups.len(), 3, "web, web/prod, db");
        let web = stable_subgroup_id(base, "web");
        let web_prod = stable_subgroup_id(web, "prod");
        let db = stable_subgroup_id(base, "db");
        assert!(
            plan.subgroups
                .iter()
                .any(|g| g.id == web && g.parent_id == Some(base))
        );
        assert!(
            plan.subgroups
                .iter()
                .any(|g| g.id == web_prod && g.parent_id == Some(web))
        );
        assert!(
            plan.subgroups
                .iter()
                .any(|g| g.id == db && g.parent_id == Some(base))
        );

        // Connections land in the leaf group; the flat one stays in the base.
        let by_name = |n: &str| {
            plan.connections
                .iter()
                .find(|c| c.name == n)
                .unwrap()
                .group_id
        };
        assert_eq!(by_name("web-01"), Some(web_prod));
        assert_eq!(by_name("db-01"), Some(db));
        assert_eq!(by_name("flat"), Some(base));
    }

    #[test]
    fn plan_is_idempotent_across_refreshes() {
        let base = Uuid::new_v4();
        let entries = vec![entry_in("web-01", "10.0.0.1", Some("web/prod"))];

        let first = plan_dynamic_refresh(base, &entries);
        let second = plan_dynamic_refresh(base, &entries);

        let ids = |p: &DynamicRefreshPlan| {
            let mut v: Vec<Uuid> = p.subgroups.iter().map(|g| g.id).collect();
            v.sort();
            v
        };
        assert_eq!(ids(&first), ids(&second), "sub-group ids are stable");
        assert_eq!(
            first.connections[0].id, second.connections[0].id,
            "connection ids are stable across refreshes"
        );
    }

    #[test]
    fn plan_ignores_blank_path_segments() {
        let base = Uuid::new_v4();
        // Leading, trailing and doubled separators must not create empty groups.
        let entries = vec![entry_in("h", "10.0.0.1", Some("/a//b/"))];

        let plan = plan_dynamic_refresh(base, &entries);

        assert_eq!(plan.subgroups.len(), 2, "only a and a/b");
        let a = stable_subgroup_id(base, "a");
        let a_b = stable_subgroup_id(a, "b");
        assert_eq!(plan.connections[0].group_id, Some(a_b));
    }

    #[test]
    fn descendant_group_ids_walks_the_whole_subtree() {
        use crate::models::ConnectionGroup;
        let base = Uuid::new_v4();

        let mut child = ConnectionGroup::with_parent("child".to_string(), base);
        let child_id = child.id;
        let mut grandchild = ConnectionGroup::with_parent("gc".to_string(), child_id);
        let gc_id = grandchild.id;
        let unrelated = ConnectionGroup::new("other".to_string());

        // Give the two we care about deterministic ids for the assertion.
        child.id = child_id;
        grandchild.id = gc_id;

        let groups = vec![child, grandchild, unrelated.clone()];
        let subtree = descendant_group_ids(base, &groups);

        assert!(subtree.contains(&base));
        assert!(subtree.contains(&child_id));
        assert!(subtree.contains(&gc_id));
        assert!(
            !subtree.contains(&unrelated.id),
            "an unrelated root is excluded"
        );
    }

    /// Builds the sub-group a refresh would create for `segment` under `parent`,
    /// with the deterministic id that marks it as refresh-created.
    fn refresh_made_subgroup(parent: Uuid, segment: &str) -> crate::models::ConnectionGroup {
        let mut group = crate::models::ConnectionGroup::with_parent(segment.to_string(), parent);
        group.id = stable_subgroup_id(parent, segment);
        group
    }

    #[test]
    fn empty_refresh_made_subgroups_are_swept() {
        let base = Uuid::new_v4();
        let web = refresh_made_subgroup(base, "web");
        let db = refresh_made_subgroup(base, "db");

        // `web` still holds a connection; `db` no longer does.
        let mut kept = Connection::new(
            "srv".to_string(),
            "srv.example.com".to_string(),
            22,
            crate::models::ProtocolConfig::Ssh(crate::models::SshConfig::default()),
        );
        kept.group_id = Some(web.id);

        let db_id = db.id;
        let stale = empty_dynamic_subgroup_ids(base, &[web, db], &[kept]);

        assert_eq!(stale, vec![db_id], "only the empty sub-group is swept");
    }

    #[test]
    fn a_user_made_subgroup_is_never_swept() {
        let base = Uuid::new_v4();
        // Random id — the marker of a folder the user created by hand.
        let manual = crate::models::ConnectionGroup::with_parent("mine".to_string(), base);

        let stale = empty_dynamic_subgroup_ids(base, &[manual], &[]);

        assert!(
            stale.is_empty(),
            "an empty folder the user made is theirs to delete, not ours"
        );
    }

    #[test]
    fn a_nested_empty_path_is_swept_deepest_first() {
        let base = Uuid::new_v4();
        let web = refresh_made_subgroup(base, "web");
        let prod = refresh_made_subgroup(web.id, "production");

        let stale = empty_dynamic_subgroup_ids(base, &[web.clone(), prod.clone()], &[]);

        assert_eq!(
            stale,
            vec![prod.id, web.id],
            "the child is reported before the parent, so one pass removes both"
        );
    }

    #[test]
    fn a_subgroup_with_a_live_child_is_kept() {
        let base = Uuid::new_v4();
        let web = refresh_made_subgroup(base, "web");
        let prod = refresh_made_subgroup(web.id, "production");

        let mut kept = Connection::new(
            "srv".to_string(),
            "srv.example.com".to_string(),
            22,
            crate::models::ProtocolConfig::Ssh(crate::models::SshConfig::default()),
        );
        kept.group_id = Some(prod.id);

        let stale = empty_dynamic_subgroup_ids(base, &[web, prod], &[kept]);

        assert!(
            stale.is_empty(),
            "a parent whose child still holds a connection stays"
        );
    }
}
