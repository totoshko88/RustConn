//! The virtual "Trash" row at the bottom of the sidebar tree.
//!
//! Like Favorites, it is not a real group: its ID is a literal and it exists only
//! while the trash holds something. Its children carry IDs that are deliberately
//! *not* bare UUIDs, so every handler that resolves a row with
//! `Uuid::parse_str(&item.id())` skips them instead of acting on a deleted
//! connection (connect, edit, delete, drag).

use uuid::Uuid;

/// ID of the virtual "Trash" group.
pub const TRASH_GROUP_ID: &str = "__trash__";

/// Kind of item held in the trash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrashKind {
    Connection,
    Group,
}

impl TrashKind {
    /// The word `win.undo-delete` expects in its `"type:uuid"` target.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Connection => "connection",
            Self::Group => "group",
        }
    }
}

/// One trashed item as the sidebar shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashEntry {
    pub kind: TrashKind,
    pub id: Uuid,
    pub name: String,
    /// Protocol string for connections, empty for groups.
    pub protocol: String,
    pub host: String,
}

/// Row ID of a trashed item: `__trash__:<kind>:<uuid>`.
#[must_use]
pub fn trash_item_id(kind: TrashKind, id: Uuid) -> String {
    format!("{TRASH_GROUP_ID}:{}:{id}", kind.as_str())
}

/// Splits a row ID made by [`trash_item_id`]; `None` for any other ID.
#[must_use]
pub fn parse_trash_item_id(row_id: &str) -> Option<(TrashKind, Uuid)> {
    let rest = row_id.strip_prefix(TRASH_GROUP_ID)?.strip_prefix(':')?;
    let (kind, id) = rest.split_once(':')?;
    let kind = match kind {
        "connection" => TrashKind::Connection,
        "group" => TrashKind::Group,
        _ => return None,
    };
    Some((kind, Uuid::parse_str(id).ok()?))
}

/// The `win.undo-delete` target that restores a trashed row.
#[must_use]
pub fn restore_target(kind: TrashKind, id: Uuid) -> String {
    format!("{}:{id}", kind.as_str())
}

/// Splits a `"type:uuid"` action target made by [`restore_target`], which
/// `win.purge-trash-item` takes as well; `None` for anything else.
#[must_use]
pub fn parse_item_target(target: &str) -> Option<(TrashKind, Uuid)> {
    let (kind, id) = target.split_once(':')?;
    let kind = match kind {
        "connection" => TrashKind::Connection,
        "group" => TrashKind::Group,
        _ => return None,
    };
    Some((kind, Uuid::parse_str(id).ok()?))
}

/// Whether the Trash row is shown: only while there is something in it.
#[must_use]
pub const fn row_visible(entries: &[TrashEntry]) -> bool {
    !entries.is_empty()
}

/// Groups first, then connections, each by case-insensitive name.
pub fn sort_entries(entries: &mut [TrashEntry]) {
    entries.sort_by(|a, b| {
        let kind_rank = |k: TrashKind| u8::from(k == TrashKind::Connection);
        kind_rank(a.kind)
            .cmp(&kind_rank(b.kind))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.id.cmp(&b.id))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: TrashKind, name: &str) -> TrashEntry {
        TrashEntry {
            kind,
            id: Uuid::new_v4(),
            name: name.to_string(),
            protocol: String::new(),
            host: String::new(),
        }
    }

    #[test]
    fn row_id_round_trips_and_is_not_a_bare_uuid() {
        let id = Uuid::new_v4();
        for kind in [TrashKind::Connection, TrashKind::Group] {
            let row_id = trash_item_id(kind, id);
            assert!(Uuid::parse_str(&row_id).is_err());
            assert_eq!(parse_trash_item_id(&row_id), Some((kind, id)));
        }
    }

    #[test]
    fn foreign_ids_are_not_trash_ids() {
        assert_eq!(parse_trash_item_id(TRASH_GROUP_ID), None);
        assert_eq!(parse_trash_item_id("__pinned__"), None);
        assert_eq!(parse_trash_item_id(&Uuid::new_v4().to_string()), None);
        assert_eq!(parse_trash_item_id("__trash__:other:x"), None);
        assert_eq!(parse_trash_item_id("__trash__:group:not-a-uuid"), None);
    }

    #[test]
    fn restore_target_matches_undo_delete_format() {
        let id = Uuid::new_v4();
        assert_eq!(
            restore_target(TrashKind::Connection, id),
            format!("connection:{id}")
        );
        assert_eq!(restore_target(TrashKind::Group, id), format!("group:{id}"));
    }

    #[test]
    fn item_target_round_trips_and_rejects_other_strings() {
        let id = Uuid::new_v4();
        for kind in [TrashKind::Connection, TrashKind::Group] {
            assert_eq!(
                parse_item_target(&restore_target(kind, id)),
                Some((kind, id))
            );
        }
        assert_eq!(parse_item_target(&id.to_string()), None);
        assert_eq!(parse_item_target(&format!("other:{id}")), None);
        assert_eq!(parse_item_target("connection:not-a-uuid"), None);
        // A sidebar row ID is not an action target.
        assert_eq!(
            parse_item_target(&trash_item_id(TrashKind::Connection, id)),
            None
        );
    }

    #[test]
    fn row_shows_only_when_trash_has_entries() {
        assert!(!row_visible(&[]));
        assert!(row_visible(&[entry(TrashKind::Connection, "a")]));
    }

    #[test]
    fn entries_sort_groups_first_then_by_name() {
        let mut v = vec![
            entry(TrashKind::Connection, "beta"),
            entry(TrashKind::Group, "Zed"),
            entry(TrashKind::Connection, "Alpha"),
        ];
        sort_entries(&mut v);
        let names: Vec<_> = v.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Zed", "Alpha", "beta"]);
    }

    /// The row appears with its children, survives a refresh with the same
    /// expansion, and disappears when the trash is empty.
    #[test]
    #[ignore = "initialises GTK: needs a display and its own process; run alone with `cargo test -p rustconn --bin rustconn -- --ignored --exact <this test path>`"]
    fn sidebar_trash_row_follows_the_trash() {
        use crate::sidebar::{ConnectionItem, ConnectionSidebar};
        use gtk4::prelude::*;

        if gtk4::init().is_err() {
            return;
        }
        let sidebar = ConnectionSidebar::new();
        let store = sidebar.store();
        let trash_rows = || {
            (0..store.n_items())
                .filter_map(|i| store.item(i).and_downcast::<ConnectionItem>())
                .filter(|item| item.id() == TRASH_GROUP_ID)
                .count()
        };

        sidebar.set_trash_row(&[], false);
        assert_eq!(trash_rows(), 0, "an empty trash shows no row");

        let one = entry(TrashKind::Connection, "gone");
        let two = entry(TrashKind::Group, "folder");
        sidebar.set_trash_row(&[two.clone(), one.clone()], false);
        assert_eq!(trash_rows(), 1);
        let row = store
            .item(store.n_items() - 1)
            .and_downcast::<ConnectionItem>()
            .expect("the trash row closes the tree");
        assert_eq!(row.id(), TRASH_GROUP_ID);
        assert_eq!(row.children().map(|c| c.n_items()), Some(2));

        sidebar.set_trash_row(&[one], false);
        assert_eq!(trash_rows(), 1, "a refresh replaces, never duplicates");

        sidebar.set_trash_row(&[], false);
        assert_eq!(trash_rows(), 0, "restoring the last item removes the row");
    }
}
