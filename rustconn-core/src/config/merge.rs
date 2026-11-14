//! Three-way merge of the connection lists that several processes share.
//!
//! `connections.toml`, `groups.toml` and `trash.toml` are rewritten whole from
//! whatever one process holds in memory. With the GUI running and a
//! `rustconn-cli add` in between, the GUI's next save used to replace the file
//! and silently drop what the CLI had written: a lost update. Merging by id
//! against the last state this process saw on disk keeps both sides' changes.
//!
//! Three lists meet here:
//!
//! * `base`: what this process last saw on disk (loaded, reloaded or written),
//! * `mine`: what it holds in memory now,
//! * `theirs`: what is on disk now.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::models::{Connection, ConnectionGroup};

/// The last on-disk state a process has seen, shared between its writer and
/// the code that reloads from disk.
///
/// A plain `Vec`, not a map: the merge indexes it itself, and the file order is
/// the order a reader sees.
pub type SharedBase<T> = Arc<Mutex<Vec<T>>>;

/// Wraps `items` as a [`SharedBase`].
#[must_use]
pub fn shared_base<T>(items: Vec<T>) -> SharedBase<T> {
    Arc::new(Mutex::new(items))
}

/// Takes `base` unless a save holds it, so a reload on the UI thread never waits
/// for a disk write. A poisoned lock still yields its list: it is a plain `Vec`
/// that no panic can leave half-built.
#[must_use]
pub fn try_lock_base<T>(base: &SharedBase<T>) -> Option<MutexGuard<'_, Vec<T>>> {
    match base.try_lock() {
        Ok(guard) => Some(guard),
        Err(TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
        Err(TryLockError::WouldBlock) => None,
    }
}

/// Whether two lists hold the same entries, whatever their order.
#[must_use]
pub fn same_entries<T: Mergeable>(a: &[T], b: &[T]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let by_id: HashMap<Uuid, &T> = a.iter().map(|item| (item.merge_id(), item)).collect();
    b.iter().all(|item| {
        by_id
            .get(&item.merge_id())
            .is_some_and(|other| *other == item)
    })
}

/// An entry that can be told apart by id and ordered by recency.
pub trait Mergeable: Clone + PartialEq {
    /// The identity that survives edits.
    fn merge_id(&self) -> Uuid;
    /// When this entry last changed; the newer one wins a conflict.
    fn merge_stamp(&self) -> DateTime<Utc>;
}

impl Mergeable for Connection {
    fn merge_id(&self) -> Uuid {
        self.id
    }
    fn merge_stamp(&self) -> DateTime<Utc> {
        self.updated_at
    }
}

impl Mergeable for ConnectionGroup {
    fn merge_id(&self) -> Uuid {
        self.id
    }
    fn merge_stamp(&self) -> DateTime<Utc> {
        self.updated_at
    }
}

/// A trashed connection: the entry and when it was trashed.
impl Mergeable for (Connection, DateTime<Utc>) {
    fn merge_id(&self) -> Uuid {
        self.0.id
    }
    fn merge_stamp(&self) -> DateTime<Utc> {
        self.1
    }
}

/// A trashed group: the entry and when it was trashed.
impl Mergeable for (ConnectionGroup, DateTime<Utc>) {
    fn merge_id(&self) -> Uuid {
        self.0.id
    }
    fn merge_stamp(&self) -> DateTime<Utc> {
        self.1
    }
}

/// Merges `mine` and `theirs` against their common `base`, by id.
///
/// Per id, with `b`, `m` and `t` the entry in each list:
///
/// * both sides hold it: the same entry stays; one that only one side changed
///   takes that side's version; one both changed takes the newer
///   [`Mergeable::merge_stamp`], and `mine` on a tie;
/// * only `mine` holds it: it stays when it is new here (not in `base`) or was
///   edited here; it is dropped when the other side deleted it and it is
///   unchanged here;
/// * only `theirs` holds it: it stays when it is new there (not in `base`); it
///   is dropped when it was deleted here, even if the other side edited it, so
///   a deletion the user made is honoured (the trash still holds the entry).
///
/// The result keeps `mine`'s order, then appends what only `theirs` added in
/// its own order, so the written file does not shuffle between saves.
#[must_use]
pub fn merge_three_way<T: Mergeable>(base: &[T], mine: &[T], theirs: &[T]) -> Vec<T> {
    let index = |items: &'_ [T]| -> HashMap<Uuid, usize> {
        items
            .iter()
            .enumerate()
            .map(|(position, item)| (item.merge_id(), position))
            .collect()
    };
    let base_at = index(base);
    let theirs_at = index(theirs);
    let mine_at = index(mine);

    let mut merged = Vec::with_capacity(mine.len().max(theirs.len()));

    for m in mine {
        let id = m.merge_id();
        let b = base_at.get(&id).map(|&i| &base[i]);
        match theirs_at.get(&id).map(|&i| &theirs[i]) {
            Some(t) => merged.push(resolve_both(b, m, t)),
            // Gone from disk. Unchanged here means the other side deleted it.
            None => match b {
                Some(b) if b == m => {}
                _ => merged.push(m.clone()),
            },
        }
    }

    for t in theirs {
        let id = t.merge_id();
        if mine_at.contains_key(&id) {
            continue;
        }
        // Absent here: new on disk when `base` never had it, otherwise deleted here.
        if !base_at.contains_key(&id) {
            merged.push(t.clone());
        }
    }

    merged
}

/// The version of an entry both sides still hold.
fn resolve_both<T: Mergeable>(base: Option<&T>, mine: &T, theirs: &T) -> T {
    if mine == theirs {
        return mine.clone();
    }
    match base {
        Some(b) if b == mine => theirs.clone(),
        Some(b) if b == theirs => mine.clone(),
        _ => {
            if theirs.merge_stamp() > mine.merge_stamp() {
                theirs.clone()
            } else {
                mine.clone()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn conn(name: &str) -> Connection {
        Connection::new_ssh(name.to_owned(), "host".to_owned(), 22)
    }

    fn names(items: &[Connection]) -> Vec<&str> {
        items.iter().map(|c| c.name.as_str()).collect()
    }

    #[test]
    fn external_addition_survives_a_save_from_a_stale_process() {
        let a = conn("a");
        let added_by_cli = conn("cli");
        // The GUI loaded [a]; the CLI then wrote [a, cli]; the GUI saves [a].
        let merged = merge_three_way(
            std::slice::from_ref(&a),
            std::slice::from_ref(&a),
            &[a.clone(), added_by_cli],
        );
        assert_eq!(names(&merged), ["a", "cli"]);
    }

    #[test]
    fn addition_here_is_kept() {
        let a = conn("a");
        let b = conn("b");
        let merged = merge_three_way(
            std::slice::from_ref(&a),
            &[a.clone(), b],
            std::slice::from_ref(&a),
        );
        assert_eq!(names(&merged), ["a", "b"]);
    }

    #[test]
    fn deletion_here_is_honoured() {
        let a = conn("a");
        let b = conn("b");
        let merged = merge_three_way(
            &[a.clone(), b.clone()],
            std::slice::from_ref(&a),
            &[a.clone(), b],
        );
        assert_eq!(names(&merged), ["a"]);
    }

    #[test]
    fn deletion_there_is_applied_when_unchanged_here() {
        let a = conn("a");
        let b = conn("b");
        let merged = merge_three_way(&[a.clone(), b.clone()], &[a.clone(), b], &[a]);
        assert_eq!(names(&merged), ["a"]);
    }

    #[test]
    fn edit_here_beats_a_deletion_there() {
        let a = conn("a");
        let mut b = conn("b");
        let original = b.clone();
        b.name = "b-edited".to_owned();
        let merged = merge_three_way(
            &[a.clone(), original],
            &[a.clone(), b],
            std::slice::from_ref(&a),
        );
        assert_eq!(names(&merged), ["a", "b-edited"]);
    }

    #[test]
    fn deletion_here_beats_an_edit_there() {
        let a = conn("a");
        let original = conn("b");
        let mut edited = original.clone();
        edited.name = "b-edited".to_owned();
        let merged = merge_three_way(
            &[a.clone(), original],
            std::slice::from_ref(&a),
            &[a.clone(), edited],
        );
        assert_eq!(names(&merged), ["a"]);
    }

    #[test]
    fn edit_on_one_side_only_is_taken() {
        let original = conn("a");
        let mut edited = original.clone();
        edited.name = "a-edited".to_owned();

        let theirs_edit = merge_three_way(
            std::slice::from_ref(&original),
            std::slice::from_ref(&original),
            std::slice::from_ref(&edited),
        );
        assert_eq!(names(&theirs_edit), ["a-edited"]);

        let my_edit = merge_three_way(
            std::slice::from_ref(&original),
            std::slice::from_ref(&edited),
            std::slice::from_ref(&original),
        );
        assert_eq!(names(&my_edit), ["a-edited"]);
    }

    #[test]
    fn conflicting_edits_take_the_newer_and_mine_on_a_tie() {
        let original = conn("a");
        let mut mine = original.clone();
        mine.name = "mine".to_owned();
        let mut theirs = original.clone();
        theirs.name = "theirs".to_owned();
        theirs.updated_at = mine.updated_at + Duration::seconds(5);

        let newer_there = merge_three_way(
            std::slice::from_ref(&original),
            std::slice::from_ref(&mine),
            std::slice::from_ref(&theirs),
        );
        assert_eq!(names(&newer_there), ["theirs"]);

        theirs.updated_at = mine.updated_at;
        let tie = merge_three_way(
            std::slice::from_ref(&original),
            std::slice::from_ref(&mine),
            std::slice::from_ref(&theirs),
        );
        assert_eq!(names(&tie), ["mine"]);
    }

    #[test]
    fn without_a_base_nothing_is_lost() {
        // A process that never saw the file (first save, empty base) must not
        // delete what is on disk.
        let a = conn("a");
        let b = conn("b");
        let merged = merge_three_way(&[], std::slice::from_ref(&a), std::slice::from_ref(&b));
        assert_eq!(names(&merged), ["a", "b"]);
    }

    #[test]
    fn order_follows_mine_then_new_entries() {
        let a = conn("a");
        let b = conn("b");
        let c = conn("c");
        let merged = merge_three_way(&[a.clone(), b.clone()], &[b.clone(), a.clone()], &[a, b, c]);
        assert_eq!(names(&merged), ["b", "a", "c"]);
    }

    #[test]
    fn merging_twice_changes_nothing() {
        let a = conn("a");
        let cli = conn("cli");
        let first = merge_three_way(
            std::slice::from_ref(&a),
            std::slice::from_ref(&a),
            &[a.clone(), cli],
        );
        let second = merge_three_way(&first, &first, &first);
        assert_eq!(first, second);
    }

    #[test]
    fn trash_entries_merge_by_id_with_the_trashed_time() {
        let now = Utc::now();
        let a = (conn("a"), now);
        let b = (conn("b"), now);
        let merged = merge_three_way(
            std::slice::from_ref(&a),
            std::slice::from_ref(&a),
            &[a.clone(), b.clone()],
        );
        assert_eq!(merged, vec![a, b]);
    }
}
