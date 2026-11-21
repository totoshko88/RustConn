//! `ScriptManager` — CRUD over `.rsc` script files.
//!
//! Scripts are plain text files in `<config_dir>/scripts/<id>.rsc`. The id is
//! the file stem and must be a safe filename (alphanumeric, `_`, `-`). Saving
//! validates the source through the `.rsc` parser, so a script that cannot be
//! parsed never reaches disk.

use std::fs;
use std::path::PathBuf;

use crate::automation::ScriptParseError;
use crate::config::ConfigManager;

/// The scripts directory under the OS config root, when one can be resolved.
///
/// Matches what [`ScriptManager`] uses for the default configuration, for
/// callers (the terminal recorder) that have no `ConfigManager` at hand.
#[must_use]
pub fn scripts_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("rustconn").join("scripts"))
}

/// Errors produced by [`ScriptManager`].
#[derive(Debug, thiserror::Error)]
pub enum ScriptError {
    /// Script storage could not be prepared (create dir etc.).
    #[error("Script storage is unavailable: {0}")]
    Storage(String),
    /// A script id was not found on disk.
    #[error("Script `{0}` not found")]
    NotFound(String),
    /// The script source failed to parse.
    #[error("Invalid script source: {0}")]
    InvalidSource(#[from] ScriptParseError),
    /// Underlying filesystem error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Manager for `.rsc` script files.
#[derive(Debug)]
pub struct ScriptManager {
    config_manager: ConfigManager,
}

impl ScriptManager {
    /// Creates a manager over the given config directory.
    #[must_use]
    pub fn new(config_manager: ConfigManager) -> Self {
        Self { config_manager }
    }

    /// The directory that holds script files.
    #[must_use]
    pub fn scripts_dir(&self) -> PathBuf {
        self.config_manager.config_dir().join("scripts")
    }

    /// Ensures the scripts directory exists, returning its path.
    fn ensure_scripts_dir(&self) -> Result<PathBuf, ScriptError> {
        self.config_manager
            .ensure_config_dir()
            .map_err(|e| ScriptError::Storage(format!("cannot prepare config dir: {e}")))?;
        let dir = self.scripts_dir();
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// Lists the ids of all stored scripts, sorted by name.
    ///
    /// A non-existent scripts directory simply yields an empty list.
    #[must_use]
    pub fn list_scripts(&self) -> Vec<String> {
        let dir = self.scripts_dir();
        let Ok(entries) = fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut ids: Vec<String> = entries
            .filter_map(std::result::Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("rsc") {
                    path.file_stem()
                        .and_then(|s| s.to_str())
                        .map(str::to_string)
                } else {
                    None
                }
            })
            .collect();
        ids.sort();
        ids
    }

    /// Checks whether a script id exists.
    #[must_use]
    pub fn has_script(&self, id: &str) -> bool {
        self.path_for(id).is_ok_and(|path| path.is_file())
    }

    /// Loads a script's source text.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError::Storage`] for an id that is not a safe filename,
    /// [`ScriptError::NotFound`] when the script does not exist, or an I/O
    /// error when reading fails.
    pub fn load_script(&self, id: &str) -> Result<String, ScriptError> {
        let path = self.path_for(id)?;
        if !path.is_file() {
            return Err(ScriptError::NotFound(id.to_string()));
        }
        Ok(fs::read_to_string(path)?)
    }

    /// Saves a script, validating its source first.
    ///
    /// The id is the file stem; it must be a safe filename.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError::InvalidSource`] when the source does not parse,
    /// [`ScriptError::Storage`] when the directory cannot be created, or an
    /// I/O error when writing fails.
    pub fn save_script(&self, id: &str, source: &str) -> Result<(), ScriptError> {
        validate_id(id)?;
        // Reject invalid scripts before they reach disk.
        crate::automation::parse_program(source)?;
        let dir = self.ensure_scripts_dir()?;
        let path = dir.join(format!("{id}.rsc"));
        fs::write(path, source)?;
        Ok(())
    }

    /// Deletes a script file.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError::Storage`] for an id that is not a safe filename,
    /// or [`ScriptError::NotFound`] when the script does not exist.
    pub fn delete_script(&self, id: &str) -> Result<(), ScriptError> {
        let path = self.path_for(id)?;
        if !path.is_file() {
            return Err(ScriptError::NotFound(id.to_string()));
        }
        fs::remove_file(path)?;
        Ok(())
    }

    /// Builds the on-disk path for a script id, refusing an id that could
    /// leave the scripts directory.
    ///
    /// Every access goes through here, not only `save_script`: ids also come
    /// from `auto_scripts` in connection and group files, which are imported
    /// and team-synced, so `../x` must not reach `load_script` or
    /// `delete_script` either.
    fn path_for(&self, id: &str) -> Result<PathBuf, ScriptError> {
        validate_id(id)?;
        Ok(self.scripts_dir().join(format!("{id}.rsc")))
    }
}

/// Rejects script ids that could escape the scripts directory.
fn validate_id(id: &str) -> Result<(), ScriptError> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-');
    if ok {
        Ok(())
    } else {
        Err(ScriptError::Storage(format!(
            "invalid script id `{id}` (alphanumeric, '-' and '_' only)"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn manager() -> (ScriptManager, TempDir) {
        let temp = TempDir::new().expect("tempdir");
        let config = ConfigManager::with_config_dir(temp.path().join("config"));
        (ScriptManager::new(config), temp)
    }

    #[test]
    fn save_load_list_delete_round_trip() {
        let (manager, _temp) = manager();
        assert!(manager.list_scripts().is_empty());

        manager.save_script("demo", "send hello{ENTER}\n").unwrap();
        assert!(manager.has_script("demo"));
        assert_eq!(manager.list_scripts(), vec!["demo".to_string()]);
        assert_eq!(manager.load_script("demo").unwrap(), "send hello{ENTER}\n");

        manager.delete_script("demo").unwrap();
        assert!(!manager.has_script("demo"));
        assert!(manager.list_scripts().is_empty());
    }

    #[test]
    fn save_rejects_invalid_source() {
        let (manager, _temp) = manager();
        let err = manager.save_script("bad", "frobnicate x\n").unwrap_err();
        assert!(matches!(err, ScriptError::InvalidSource(_)));
        assert!(!manager.has_script("bad"));
    }

    #[test]
    fn save_rejects_unsafe_id() {
        let (manager, _temp) = manager();
        assert!(manager.save_script("../evil", "send hi\n").is_err());
        assert!(manager.save_script("a/b", "send hi\n").is_err());
        assert!(!manager.has_script("../evil"));
    }

    /// An id from an imported or team-synced `auto_scripts` list must not
    /// read or delete a `.rsc` file outside the library.
    #[test]
    fn load_and_delete_reject_an_escaping_id() {
        let (manager, temp) = manager();
        manager.save_script("inside", "send hi\n").unwrap();
        let outside = temp.path().join("config").join("outside.rsc");
        std::fs::write(&outside, "send hi\n").unwrap();

        assert!(matches!(
            manager.load_script("../outside"),
            Err(ScriptError::Storage(_))
        ));
        assert!(matches!(
            manager.delete_script("../outside"),
            Err(ScriptError::Storage(_))
        ));
        assert!(!manager.has_script("../outside"));
        assert!(outside.is_file(), "the file outside the library survived");
    }

    #[test]
    fn load_missing_is_not_found() {
        let (manager, _temp) = manager();
        assert!(matches!(
            manager.load_script("nope"),
            Err(ScriptError::NotFound(_))
        ));
    }

    #[test]
    fn save_creates_scripts_dir() {
        let (manager, _temp) = manager();
        manager.save_script("x", "send hi\n").unwrap();
        assert!(manager.scripts_dir().is_dir());
    }
}
