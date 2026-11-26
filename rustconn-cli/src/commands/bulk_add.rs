//! `add --from-file`: bulk-add connections from a YAML or JSON document.
//!
//! The document reuses the on-disk serde shape of `Connection` and
//! `ConnectionGroup`, read through one YAML parser (YAML is a JSON superset).
//! The input layer here is more lenient than the connections file, and only
//! here: a hand-written entry may leave out `id`, `created_at` and
//! `updated_at` (filled with a fresh UUID and the current time), `protocol`
//! (taken from `protocol_config.type`), `port` (the protocol's default) and
//! `host` (empty, for Kubernetes and Serial). Two keys the model does not have
//! are accepted the way `add --group` and `add --jump-host` take them: `group`
//! (a group name, created when missing) and `jump_host` (a connection name or
//! UUID). The `Connection` serde contract itself is unchanged, so a config file
//! missing an `id` still fails to load instead of getting a new id on each load.

use std::collections::HashMap;
use std::path::Path;

use chrono::Utc;
use rustconn_core::config::ConfigManager;
use rustconn_core::models::{Connection, ConnectionGroup};
use serde_yaml::{Mapping, Value};
use uuid::Uuid;

use super::add::apply_jump_host_id;
use crate::error::CliError;
use crate::util::{
    create_config_manager, default_port_for_protocol, find_connection, find_or_create_group_id,
    parse_protocol_type,
};

/// Bulk-adds connections from a YAML or JSON file, or from stdin when `path`
/// is `-`.
///
/// The document is either a top-level object with a `connections` list (and
/// an optional `groups` list), or a bare list of connections. Every valid,
/// non-duplicate connection is added in one save; the command reports how many
/// were added, names each skipped entry with its reason (duplicate name, failed
/// validation, unknown jump host) and each entry whose `id` was already in use
/// and was reassigned, without aborting the batch on the first bad entry.
///
/// # Errors
///
/// Returns [`CliError::Config`] when the input cannot be read, the document
/// cannot be parsed (the message names the serde path of the bad field), or
/// the merged set cannot be saved.
pub(super) fn cmd_add_from_file(config_path: Option<&Path>, path: &str) -> Result<(), CliError> {
    let content = if path == "-" {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| CliError::Config(format!("Failed to read stdin: {e}")))?;
        buf
    } else {
        std::fs::read_to_string(path)
            .map_err(|e| CliError::Config(format!("Failed to read {path}: {e}")))?
    };

    let document = parse_document(&content)?;
    if document.connections.is_empty() {
        return Err(CliError::Config(
            "No connections found in the input. Provide a 'connections' list or a bare list."
                .into(),
        ));
    }

    let config_manager = create_config_manager(config_path)?;
    let mut groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Config(format!("Failed to load groups: {e}")))?;
    let mut connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;
    let initial_group_count = groups.len();

    let mut reassigned: Vec<String> = Vec::new();
    let group_id_remap = merge_groups(&mut groups, document.groups, &mut reassigned);

    let mut added = 0usize;
    let mut skipped: Vec<String> = Vec::new();
    for entry in document.connections {
        match add_entry(entry, &mut connections, &mut groups, &group_id_remap) {
            Ok(note) => {
                reassigned.extend(note);
                added += 1;
            }
            Err(reason) => skipped.push(reason),
        }
    }

    let new_groups = groups.len() - initial_group_count;
    if new_groups > 0 {
        config_manager
            .save_groups(&groups)
            .map_err(|e| CliError::Config(format!("Failed to save groups: {e}")))?;
    }
    if added > 0 {
        config_manager
            .save_connections(&connections)
            .map_err(|e| CliError::Config(format!("Failed to save connections: {e}")))?;
    }

    println!("Added {added} connection(s) from {path}.");
    if new_groups > 0 {
        println!("Created {new_groups} group(s).");
    }
    if !reassigned.is_empty() {
        println!("Reassigned {} id(s):", reassigned.len());
        for note in &reassigned {
            println!("  - {note}");
        }
    }
    if !skipped.is_empty() {
        println!("Skipped {} entry/entries:", skipped.len());
        for reason in &skipped {
            println!("  - {reason}");
        }
    }

    Ok(())
}

/// Merges the file's groups into `groups` by name (case-insensitive) and
/// returns the file-id → kept-id remap for the connections.
///
/// A group whose name already exists is kept, and the file's id for it maps
/// to the existing group's id, so a connection referring to it is not
/// orphaned. A new group whose id is already taken gets a fresh one, mapped
/// the same way and noted in `reassigned`.
fn merge_groups(
    groups: &mut Vec<ConnectionGroup>,
    incoming: Vec<ConnectionGroup>,
    reassigned: &mut Vec<String>,
) -> HashMap<Uuid, Uuid> {
    let mut remap = HashMap::new();
    for mut group in incoming {
        if let Some(existing) = groups
            .iter()
            .find(|g| g.name.eq_ignore_ascii_case(&group.name))
        {
            if group.id != existing.id {
                remap.insert(group.id, existing.id);
            }
            continue;
        }
        if groups.iter().any(|g| g.id == group.id) {
            let fresh = Uuid::new_v4();
            reassigned.push(format!(
                "group '{}' — id {} is already in use; assigned {fresh}",
                group.name, group.id
            ));
            remap.insert(group.id, fresh);
            group.id = fresh;
        }
        groups.push(group);
    }
    remap
}

/// Adds one entry to `connections`, resolving its group and jump host.
///
/// Returns the note for a reassigned id, if any; `Err` is the reason the entry
/// was skipped. Nothing is changed for a skipped entry: the group is created
/// last, once nothing else can fail.
fn add_entry(
    entry: BulkEntry,
    connections: &mut Vec<Connection>,
    groups: &mut Vec<ConnectionGroup>,
    group_id_remap: &HashMap<Uuid, Uuid>,
) -> Result<Option<String>, String> {
    let BulkEntry {
        mut connection,
        group,
        jump_host,
    } = entry;
    let name = connection.name.clone();

    if let Some(gid) = connection.group_id
        && let Some(canonical) = group_id_remap.get(&gid)
    {
        connection.group_id = Some(*canonical);
    }
    if group.is_some() && connection.group_id.is_some() {
        return Err(format!(
            "'{name}' — give either 'group' (a name) or 'group_id', not both"
        ));
    }
    ConfigManager::validate_connection(&connection)
        .map_err(|e| format!("'{name}' — invalid: {e}"))?;
    if connections
        .iter()
        .any(|c| c.name.eq_ignore_ascii_case(&name))
    {
        return Err(format!(
            "'{name}' — a connection with that name already exists"
        ));
    }
    let mut note = None;
    if connections.iter().any(|c| c.id == connection.id) {
        let fresh = Uuid::new_v4();
        note = Some(format!(
            "'{name}' — id {} is already in use; assigned {fresh}",
            connection.id
        ));
        connection.id = fresh;
    }
    // Resolved against what is saved plus the entries added before this one,
    // so a bastion defined earlier in the same file can be named.
    if let Some(reference) = jump_host.as_deref() {
        let jump_id = find_connection(connections, reference)
            .map_err(|e| format!("'{name}' — jump host '{reference}': {e}"))?
            .id;
        apply_jump_host_id(&mut connection, jump_id).map_err(|e| format!("'{name}' — {e}"))?;
    }
    if let Some(group_name) = group.as_deref() {
        let id =
            find_or_create_group_id(groups, group_name).map_err(|e| format!("'{name}' — {e}"))?;
        connection.group_id = Some(id);
    }
    connections.push(connection);
    Ok(note)
}

/// One connection of the document, with the CLI-only keys taken out.
struct BulkEntry {
    connection: Connection,
    /// `group`: a group name, resolved like `add --group`.
    group: Option<String>,
    /// `jump_host`: a connection name or UUID, resolved like `add --jump-host`.
    jump_host: Option<String>,
}

/// A parsed bulk-add document.
struct BulkDocument {
    connections: Vec<BulkEntry>,
    groups: Vec<ConnectionGroup>,
}

/// The object shape after normalisation, parsed strictly.
#[derive(serde::Deserialize)]
struct StrictObject {
    connections: Vec<Connection>,
    #[serde(default)]
    groups: Vec<ConnectionGroup>,
}

/// Parses and normalises a bulk-add document.
///
/// The shape is chosen from the top-level value, so a parse failure is
/// reported against the shape the document actually has: an object with a
/// `connections` key gets the object-shape error, a list gets the list error.
///
/// # Errors
///
/// [`CliError::Config`] for a YAML syntax error (with line and column), a
/// top-level value of neither shape, or an entry that does not fit the model
/// (with the serde path of the offending field).
fn parse_document(content: &str) -> Result<BulkDocument, CliError> {
    let root: Value = serde_yaml::from_str(content)
        .map_err(|e| CliError::Config(format!("Failed to parse the bulk-add document: {e}")))?;

    match root {
        Value::Mapping(mut map) => {
            let Some(raw_connections) = map.remove("connections") else {
                return Err(CliError::Config(
                    "Invalid bulk-add document: the top-level object has no 'connections' key. \
                     Expected an object with a 'connections' list (and an optional 'groups' \
                     list), or a list of connections"
                        .into(),
                ));
            };
            let raw_connections = expect_list(raw_connections, "connections")?;
            let raw_groups = match map.remove("groups") {
                None | Some(Value::Null) => Vec::new(),
                Some(value) => expect_list(value, "groups")?,
            };

            let mut entries = Vec::with_capacity(raw_connections.len());
            let mut extras = Vec::with_capacity(raw_connections.len());
            for (index, value) in raw_connections.into_iter().enumerate() {
                let (value, extra) = prepare_connection(value, &format!("connections[{index}]"))?;
                entries.push(value);
                extras.push(extra);
            }
            let mut groups = Vec::with_capacity(raw_groups.len());
            for (index, value) in raw_groups.into_iter().enumerate() {
                groups.push(prepare_group(value, &format!("groups[{index}]"))?);
            }

            let mut normalised = Mapping::new();
            normalised.insert("connections".into(), Value::Sequence(entries));
            normalised.insert("groups".into(), Value::Sequence(groups));
            let strict: StrictObject = reparse(&Value::Mapping(normalised)).map_err(|message| {
                CliError::Config(format!(
                    "Invalid bulk-add document (an object with a 'connections' list): \
                         {message}"
                ))
            })?;
            Ok(BulkDocument {
                connections: zip_entries(strict.connections, extras),
                groups: strict.groups,
            })
        }
        Value::Sequence(raw_connections) => {
            let mut entries = Vec::with_capacity(raw_connections.len());
            let mut extras = Vec::with_capacity(raw_connections.len());
            for (index, value) in raw_connections.into_iter().enumerate() {
                let (value, extra) = prepare_connection(value, &format!("[{index}]"))?;
                entries.push(value);
                extras.push(extra);
            }
            let strict: Vec<Connection> =
                reparse(&Value::Sequence(entries)).map_err(|message| {
                    CliError::Config(format!(
                        "Invalid bulk-add document (a list of connections): {message}"
                    ))
                })?;
            Ok(BulkDocument {
                connections: zip_entries(strict, extras),
                groups: Vec::new(),
            })
        }
        Value::Null => Ok(BulkDocument {
            connections: Vec::new(),
            groups: Vec::new(),
        }),
        other => Err(CliError::Config(format!(
            "Invalid bulk-add document: expected an object with a 'connections' list, or a \
             list of connections; found {}",
            kind(&other)
        ))),
    }
}

/// The CLI-only keys of one entry: `group` and `jump_host`.
type Extras = (Option<String>, Option<String>);

fn zip_entries(connections: Vec<Connection>, extras: Vec<Extras>) -> Vec<BulkEntry> {
    connections
        .into_iter()
        .zip(extras)
        .map(|(connection, (group, jump_host))| BulkEntry {
            connection,
            group,
            jump_host,
        })
        .collect()
}

/// Takes the CLI-only keys out of a connection entry and fills the fields a
/// hand-written file need not carry. `at` is the entry's path, for errors.
fn prepare_connection(value: Value, at: &str) -> Result<(Value, Extras), CliError> {
    let Value::Mapping(mut map) = value else {
        return Err(CliError::Config(format!(
            "Invalid bulk-add document: {at}: expected a connection object, found {}",
            kind(&value)
        )));
    };

    let group = take_string(&mut map, "group", at)?;
    let jump_host = take_string(&mut map, "jump_host", at)?;

    fill_identity(&mut map);
    if is_missing(&map, "protocol")
        && let Some(tag) = map
            .get("protocol_config")
            .and_then(|config| config.get("type"))
            .and_then(Value::as_str)
    {
        // `ProtocolType` is the `ProtocolConfig` tag in lowercase
        // (`ZeroTrust` -> `zerotrust`, `WinRm` -> `winrm`).
        let protocol = tag.to_lowercase();
        map.insert("protocol".into(), Value::String(protocol));
    }
    if is_missing(&map, "port")
        && let Some(protocol) = map.get("protocol").and_then(Value::as_str)
        && let Ok(protocol) = parse_protocol_type(protocol)
    {
        let port = u64::from(default_port_for_protocol(protocol));
        map.insert("port".into(), Value::Number(port.into()));
    }
    if is_missing(&map, "host") {
        // Kubernetes and Serial have no network host; for every other protocol
        // validation reports the empty host as the reason the entry is skipped.
        map.insert("host".into(), Value::String(String::new()));
    }

    Ok((Value::Mapping(map), (group, jump_host)))
}

/// Fills `id` and `created_at` of a group entry the same way.
fn prepare_group(value: Value, at: &str) -> Result<Value, CliError> {
    let Value::Mapping(mut map) = value else {
        return Err(CliError::Config(format!(
            "Invalid bulk-add document: {at}: expected a group object, found {}",
            kind(&value)
        )));
    };
    fill_identity(&mut map);
    Ok(Value::Mapping(map))
}

/// Gives an entry a fresh `id` and the current time as `created_at` and
/// `updated_at` where the file leaves them out (or sets them to null).
fn fill_identity(map: &mut Mapping) {
    if is_missing(map, "id") {
        map.insert("id".into(), Value::String(Uuid::new_v4().to_string()));
    }
    let now = Utc::now().to_rfc3339();
    for key in ["created_at", "updated_at"] {
        if is_missing(map, key) {
            map.insert(key.into(), Value::String(now.clone()));
        }
    }
}

fn is_missing(map: &Mapping, key: &str) -> bool {
    map.get(key).is_none_or(Value::is_null)
}

/// Removes `key` from an entry and returns it as a string, if present.
fn take_string(map: &mut Mapping, key: &str, at: &str) -> Result<Option<String>, CliError> {
    match map.remove(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(other) => Err(CliError::Config(format!(
            "Invalid bulk-add document: {at}.{key}: expected a string, found {}",
            kind(&other)
        ))),
    }
}

/// Deserialises a normalised document strictly, through the text parser so
/// the error carries the serde path (`connections[0].protocol_config: missing
/// field `type``). Deserialising a `Value` directly reports no path.
///
/// The position is dropped from the message: it would point into the
/// normalised copy, not the user's file. The path identifies the field.
fn reparse<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T, String> {
    let text = serde_yaml::to_string(value).map_err(|e| e.to_string())?;
    serde_yaml::from_str(&text).map_err(|e| message_without_location(&e))
}

fn message_without_location(err: &serde_yaml::Error) -> String {
    let full = err.to_string();
    if let Some(location) = err.location() {
        let suffix = format!(" at line {} column {}", location.line(), location.column());
        if let Some(message) = full.strip_suffix(&suffix) {
            return message.to_string();
        }
    }
    full
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "nothing",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Sequence(_) => "a list",
        Value::Mapping(_) => "an object",
        Value::Tagged(_) => "a tagged value",
    }
}

fn expect_list(value: Value, key: &str) -> Result<Vec<Value>, CliError> {
    match value {
        Value::Sequence(items) => Ok(items),
        Value::Null => Ok(Vec::new()),
        other => Err(CliError::Config(format!(
            "Invalid bulk-add document: '{key}' must be a list, found {}",
            kind(&other)
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error_of(content: &str) -> String {
        match parse_document(content) {
            Ok(_) => panic!("expected a parse error"),
            Err(e) => e.to_string(),
        }
    }

    #[test]
    fn object_shape_failure_reports_the_object_error_with_its_path() {
        let message = error_of(
            r#"{"connections": [{"name": "a", "host": "h", "protocol_config": {"Ssh": {}}}]}"#,
        );
        assert!(
            message.contains("an object with a 'connections' list"),
            "{message}"
        );
        assert!(
            message.contains("connections[0].protocol_config"),
            "{message}"
        );
        assert!(!message.contains("expected a sequence"), "{message}");
    }

    #[test]
    fn list_shape_failure_reports_the_list_error() {
        let message = error_of(r#"[{"name": "a", "host": "h", "protocol_config": {}}]"#);
        assert!(message.contains("a list of connections"), "{message}");
        assert!(message.contains("[0].protocol_config"), "{message}");
    }

    #[test]
    fn object_without_connections_key_says_so() {
        let message = error_of(r#"{"name": "a", "host": "h"}"#);
        assert!(message.contains("no 'connections' key"), "{message}");
    }

    #[test]
    fn missing_identity_fields_are_filled() {
        let document = parse_document(
            "- name: a\n  host: h\n  protocol_config:\n    type: Ssh\n\
             - name: b\n  host: h\n  protocol_config:\n    type: Ssh\n",
        )
        .expect("lenient entries parse");
        assert_eq!(document.connections.len(), 2);
        let first = &document.connections[0].connection;
        let second = &document.connections[1].connection;
        assert_ne!(first.id, second.id);
        assert_eq!(first.port, 22);
        assert_eq!(first.protocol, rustconn_core::models::ProtocolType::Ssh);
    }

    #[test]
    fn provided_id_and_cli_keys_are_kept() {
        let document = parse_document(
            "connections:\n  - id: 7f1c2b4e-0000-4000-8000-000000000001\n    name: a\n    \
             host: h\n    group: Web\n    jump_host: bastion\n    protocol_config:\n      \
             type: Rdp\n",
        )
        .expect("entry parses");
        let entry = &document.connections[0];
        assert_eq!(
            entry.connection.id.to_string(),
            "7f1c2b4e-0000-4000-8000-000000000001"
        );
        assert_eq!(entry.connection.port, 3389);
        assert_eq!(entry.group.as_deref(), Some("Web"));
        assert_eq!(entry.jump_host.as_deref(), Some("bastion"));
    }
}
