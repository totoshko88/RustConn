//! Warnings for imported entries that fail the config validators.
//!
//! The `ConfigManager::validate_*` checks used to guard only editing. An
//! import now runs them too, but as warnings only: every entry stays in the
//! result exactly as the importer produced it, so the user sees the problem and
//! fixes it instead of losing data.

use super::{ImportResult, ImportWarning};
use crate::ConfigError;
use crate::config::{ConfigManager, NetworkSettings};
use crate::connection::jump_chain::find_dangling_bastions;

impl ImportResult {
    /// Appends a warning for each imported connection, group or snippet that
    /// fails validation. Never removes or edits an entry.
    ///
    /// Call once, after the importer returned. Each warning names the entry by
    /// connection name or 1-based position and carries no host, user or
    /// password, so it is safe to log.
    #[must_use]
    pub fn with_validation_warnings(mut self) -> Self {
        let mut found = Vec::new();
        let network = NetworkSettings::default();

        for (index, conn) in self.connections.iter().enumerate() {
            if let Err(ConfigError::Validation { field, .. }) =
                ConfigManager::validate_connection(conn)
            {
                found.push(match field.as_str() {
                    "name" => ImportWarning::ConnectionNameEmpty {
                        position: index + 1,
                    },
                    "host" => ImportWarning::ConnectionHostEmpty {
                        connection_name: conn.name.clone(),
                    },
                    _ => ImportWarning::ConnectionPortZero {
                        connection_name: conn.name.clone(),
                    },
                });
            }
            if !find_dangling_bastions(conn, &self.connections, &self.groups, &network).is_empty() {
                found.push(ImportWarning::JumpHostMissing {
                    connection_name: conn.name.clone(),
                });
            }
        }
        for (index, group) in self.groups.iter().enumerate() {
            if ConfigManager::validate_group(group).is_err() {
                found.push(ImportWarning::GroupNameEmpty {
                    position: index + 1,
                });
            }
        }
        for (index, snippet) in self.snippets.iter().enumerate() {
            if ConfigManager::validate_snippet(snippet).is_err() {
                found.push(ImportWarning::SnippetIncomplete {
                    position: index + 1,
                });
            }
        }

        for warning in &found {
            tracing::warn!(%warning, "Imported entry fails validation; kept unchanged");
        }
        self.warnings.extend(found);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::{CsvImporter, ImportSource, SshConfigImporter};
    use crate::models::{Connection, ConnectionGroup, ProtocolConfig, Snippet};
    use uuid::Uuid;

    fn result_with(connections: Vec<Connection>) -> ImportResult {
        ImportResult {
            connections,
            ..ImportResult::default()
        }
    }

    fn with_jump(name: &str, jump: Uuid) -> Connection {
        let mut conn = Connection::new_ssh(name.into(), "h".into(), 22);
        if let ProtocolConfig::Ssh(cfg) = &mut conn.protocol_config {
            cfg.jump_host_id = Some(jump);
        }
        conn
    }

    #[test]
    fn valid_entries_add_no_warning() {
        let r = result_with(vec![Connection::new_ssh("a".into(), "h".into(), 22)])
            .with_validation_warnings();
        assert!(r.warnings.is_empty());
    }

    #[test]
    fn invalid_port_warns_and_keeps_the_connection_unchanged() {
        let conn = Connection::new_ssh("bad-port".into(), "h".into(), 0);
        let r = result_with(vec![conn.clone()]).with_validation_warnings();
        assert_eq!(r.connections, vec![conn]);
        assert_eq!(
            r.warnings,
            vec![ImportWarning::ConnectionPortZero {
                connection_name: "bad-port".into()
            }]
        );
    }

    #[test]
    fn empty_host_and_empty_name_warn() {
        let r = result_with(vec![
            Connection::new_ssh("no-host".into(), "  ".into(), 22),
            Connection::new_ssh(String::new(), "h".into(), 22),
        ])
        .with_validation_warnings();
        assert_eq!(r.connections.len(), 2);
        assert_eq!(
            r.warnings,
            vec![
                ImportWarning::ConnectionHostEmpty {
                    connection_name: "no-host".into()
                },
                ImportWarning::ConnectionNameEmpty { position: 2 },
            ]
        );
    }

    #[test]
    fn unknown_jump_host_warns() {
        let r = result_with(vec![with_jump("via", Uuid::new_v4())]).with_validation_warnings();
        assert_eq!(
            r.warnings,
            vec![ImportWarning::JumpHostMissing {
                connection_name: "via".into()
            }]
        );
    }

    #[test]
    fn jump_host_inside_the_import_is_fine() {
        let bastion = Connection::new_ssh("bastion".into(), "b".into(), 22);
        let conn = with_jump("via", bastion.id);
        let r = result_with(vec![bastion, conn]).with_validation_warnings();
        assert!(r.warnings.is_empty());
    }

    #[test]
    fn empty_group_and_snippet_warn() {
        let mut r = ImportResult::default();
        r.groups.push(ConnectionGroup::new(String::new()));
        r.snippets.push(Snippet::new("s".into(), String::new()));
        let r = r.with_validation_warnings();
        assert_eq!(r.groups.len(), 1);
        assert_eq!(r.snippets.len(), 1);
        assert_eq!(
            r.warnings,
            vec![
                ImportWarning::GroupNameEmpty { position: 1 },
                ImportWarning::SnippetIncomplete { position: 1 },
            ]
        );
    }

    #[test]
    fn unicode_names_are_kept_and_not_flagged() {
        let r = result_with(vec![Connection::new_ssh(
            "сервер-🚀".into(),
            "h".into(),
            22,
        )])
        .with_validation_warnings();
        assert!(r.warnings.is_empty());
        assert_eq!(r.connections[0].name, "сервер-🚀");
    }

    #[test]
    fn empty_result_adds_nothing() {
        let r = ImportResult::default().with_validation_warnings();
        assert!(r.warnings.is_empty());
    }

    #[test]
    fn many_entries_are_all_checked() {
        let conns = (0..5000)
            .map(|i| {
                Connection::new_ssh(format!("c{i}"), "h".into(), if i % 2 == 0 { 0 } else { 22 })
            })
            .collect();
        let r = result_with(conns).with_validation_warnings();
        assert_eq!(r.connections.len(), 5000);
        assert_eq!(r.warnings.len(), 2500);
    }

    #[test]
    fn warning_text_has_no_host() {
        let r = result_with(vec![Connection::new_ssh("n".into(), String::new(), 22)])
            .with_validation_warnings();
        assert_eq!(r.warnings.len(), 1);
        assert!(!r.warnings[0].to_string().contains("secret-host"));
    }

    #[test]
    fn csv_row_with_empty_host_is_kept_with_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("in.csv");
        std::fs::write(
            &path,
            "name,host,port,protocol\nok,h1,22,ssh\nнема,,22,ssh\n",
        )
        .unwrap();
        let r = CsvImporter::new()
            .import_from_path(&path)
            .unwrap()
            .with_validation_warnings();
        let kept = r.connections.iter().filter(|c| c.host.is_empty()).count();
        let warned = r
            .warnings
            .iter()
            .filter(|w| matches!(w, ImportWarning::ConnectionHostEmpty { .. }))
            .count();
        assert_eq!(warned, kept);
    }

    #[test]
    fn empty_file_gives_no_warning() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        std::fs::write(&path, "").unwrap();
        let r = SshConfigImporter::new()
            .import_from_path(&path)
            .unwrap()
            .with_validation_warnings();
        assert!(r.warnings.is_empty());
    }
}
