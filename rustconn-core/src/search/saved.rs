//! Saved searches: a named sidebar query the user can re-apply with one click.
//!
//! The sidebar encodes everything — free text, `#tag`, `group:` and the
//! protocol check boxes (`protocol:`/`protocols:`) — in one query string, so a
//! saved search is just a name plus that string. Applying one means putting the
//! string back into the search entry, which makes the result identical to
//! typing it by construction.
//!
//! Stored in `UiSettings` (the settings file), not in `connections.toml`, so
//! versions that do not know the field keep reading both files.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Most saved searches kept; the sidebar menu is not meant to scroll forever.
pub const MAX_SAVED_SEARCHES: usize = 50;

/// Longest accepted name, in characters.
pub const MAX_SAVED_SEARCH_NAME_CHARS: usize = 64;

/// Why a saved search could not be added or renamed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SavedSearchError {
    /// The name is empty after trimming.
    #[error("The name of a saved search cannot be empty")]
    EmptyName,
    /// The name is longer than [`MAX_SAVED_SEARCH_NAME_CHARS`].
    #[error("The name of a saved search is too long")]
    NameTooLong,
    /// The query is empty after trimming: there is nothing to save.
    #[error("There is no search to save")]
    EmptyQuery,
    /// Another saved search already has this name (compared ignoring case).
    #[error("A saved search with this name already exists")]
    DuplicateName,
    /// [`MAX_SAVED_SEARCHES`] reached.
    #[error("Too many saved searches")]
    LimitReached,
    /// No saved search has this id.
    #[error("Saved search not found")]
    NotFound,
}

/// One saved search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSearch {
    /// Stable id, so a rename or delete never hits the wrong row.
    pub id: Uuid,
    /// Name shown in the sidebar.
    pub name: String,
    /// The exact text the search entry held when it was saved.
    pub query: String,
}

fn clean_name(
    name: &str,
    others: &[SavedSearch],
    skip: Option<Uuid>,
) -> Result<String, SavedSearchError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(SavedSearchError::EmptyName);
    }
    if name.chars().count() > MAX_SAVED_SEARCH_NAME_CHARS {
        return Err(SavedSearchError::NameTooLong);
    }
    let lower = name.to_lowercase();
    if others
        .iter()
        .any(|s| Some(s.id) != skip && s.name.to_lowercase() == lower)
    {
        return Err(SavedSearchError::DuplicateName);
    }
    Ok(name.to_string())
}

/// Adds a saved search and returns its id.
///
/// # Errors
///
/// Returns a [`SavedSearchError`] for an empty/long/duplicate name, an empty
/// query, or when the list is full.
pub fn add_saved_search(
    list: &mut Vec<SavedSearch>,
    name: &str,
    query: &str,
) -> Result<Uuid, SavedSearchError> {
    let query = query.trim();
    if query.is_empty() {
        return Err(SavedSearchError::EmptyQuery);
    }
    if list.len() >= MAX_SAVED_SEARCHES {
        return Err(SavedSearchError::LimitReached);
    }
    let name = clean_name(name, list, None)?;
    let id = Uuid::new_v4();
    list.push(SavedSearch {
        id,
        name,
        query: query.to_string(),
    });
    Ok(id)
}

/// Renames a saved search; its query stays.
///
/// # Errors
///
/// Returns a [`SavedSearchError`] for a bad name or an unknown id.
pub fn rename_saved_search(
    list: &mut [SavedSearch],
    id: Uuid,
    name: &str,
) -> Result<(), SavedSearchError> {
    let name = clean_name(name, list, Some(id))?;
    let item = list
        .iter_mut()
        .find(|s| s.id == id)
        .ok_or(SavedSearchError::NotFound)?;
    item.name = name;
    Ok(())
}

/// Deletes a saved search. Returns `false` when the id is unknown.
pub fn remove_saved_search(list: &mut Vec<SavedSearch>, id: Uuid) -> bool {
    let before = list.len();
    list.retain(|s| s.id != id);
    list.len() != before
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::settings::UiSettings;

    #[test]
    fn add_trims_and_rejects_bad_input() {
        let mut list = Vec::new();
        let id = add_saved_search(&mut list, "  Prod SSH ", " protocol:ssh #prod ").unwrap();
        assert_eq!(list[0].id, id);
        assert_eq!(list[0].name, "Prod SSH");
        assert_eq!(list[0].query, "protocol:ssh #prod");

        assert_eq!(
            add_saved_search(&mut list, " ", "x"),
            Err(SavedSearchError::EmptyName)
        );
        assert_eq!(
            add_saved_search(&mut list, "A", "  "),
            Err(SavedSearchError::EmptyQuery)
        );
        assert_eq!(
            add_saved_search(&mut list, "prod ssh", "x"),
            Err(SavedSearchError::DuplicateName)
        );
        let long = "x".repeat(MAX_SAVED_SEARCH_NAME_CHARS + 1);
        assert_eq!(
            add_saved_search(&mut list, &long, "x"),
            Err(SavedSearchError::NameTooLong)
        );
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn limit_is_enforced() {
        let mut list = Vec::new();
        for i in 0..MAX_SAVED_SEARCHES {
            add_saved_search(&mut list, &format!("s{i}"), "q").unwrap();
        }
        assert_eq!(
            add_saved_search(&mut list, "extra", "q"),
            Err(SavedSearchError::LimitReached)
        );
    }

    #[test]
    fn rename_and_remove() {
        let mut list = Vec::new();
        let a = add_saved_search(&mut list, "A", "qa").unwrap();
        let b = add_saved_search(&mut list, "B", "qb").unwrap();
        // Renaming to its own name (other case) is fine; to another's is not.
        rename_saved_search(&mut list, a, "a").unwrap();
        assert_eq!(
            rename_saved_search(&mut list, a, "b"),
            Err(SavedSearchError::DuplicateName)
        );
        assert_eq!(
            rename_saved_search(&mut list, Uuid::new_v4(), "Z"),
            Err(SavedSearchError::NotFound)
        );
        assert_eq!(list[0].query, "qa");
        assert!(remove_saved_search(&mut list, b));
        assert!(!remove_saved_search(&mut list, b));
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn round_trips_through_settings_toml() {
        let mut ui = UiSettings::default();
        add_saved_search(
            &mut ui.saved_searches,
            "Prod \"SSH\"",
            "protocols:ssh,mosh #prod",
        )
        .unwrap();
        add_saved_search(&mut ui.saved_searches, "Ünï", "group:дев").unwrap();
        let text = toml::to_string(&ui).unwrap();
        let back: UiSettings = toml::from_str(&text).unwrap();
        assert_eq!(back.saved_searches, ui.saved_searches);
    }

    #[test]
    fn old_settings_without_the_field_load() {
        let ui: UiSettings = toml::from_str("language = \"uk\"\n").unwrap();
        assert!(ui.saved_searches.is_empty());
        // And an empty list is not written, so an older build sees no new key.
        assert!(!toml::to_string(&ui).unwrap().contains("saved_searches"));
    }

    #[test]
    fn applying_equals_typing() {
        use crate::search::SearchEngine;
        let mut list = Vec::new();
        let typed = "protocol:ssh #prod web";
        let id = add_saved_search(&mut list, "Web", typed).unwrap();
        let applied = &list.iter().find(|s| s.id == id).unwrap().query;

        let conns: Vec<_> = crate::search::benchmark::generate_test_connections(40);
        let refs: Vec<_> = conns.iter().collect();
        let run = |q: &str| {
            SearchEngine::new()
                .search(&SearchEngine::parse_query(q).unwrap(), &refs, &[])
                .into_iter()
                .map(|r| r.connection_id)
                .collect::<Vec<_>>()
        };
        assert_eq!(run(applied), run(typed));
    }
}
