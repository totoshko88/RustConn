//! Crash-safe file replacement shared by every writer in the crate.
//!
//! One implementation of "write a sibling temp file, flush it to disk, rename it
//! over the target", so the properties that matter hold everywhere or nowhere:
//! the file is created with its final permissions (never wider, not even for the
//! width of one `write`), its bytes are `fsync`ed before the rename, a failed
//! write leaves no temp file, and the temp name is unique and never reused.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Owner read/write only: the mode for anything that may hold hostnames,
/// usernames or secrets.
pub const MODE_PRIVATE: u32 = 0o600;

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Sibling temp path: `.<name>.<pid>.<nanos>.<n>.tmp` in the target's directory
/// (same filesystem, so the rename is atomic).
fn temp_path(path: &Path) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let name = path
        .file_name()
        .map_or_else(|| "file".to_string(), |n| n.to_string_lossy().into_owned());
    path.with_file_name(format!(
        ".{name}.{}.{nanos}.{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Atomically replaces `path` with `bytes`.
///
/// The temp file is created with `create_new` (a pre-planted file or symlink is
/// an error, not a redirected write) and `mode` already applied, then written,
/// `fsync`ed and renamed over `path`. The parent directory is synced best-effort
/// so the rename itself survives a crash. `mode` is ignored off unix. The parent
/// directory must exist.
///
/// # Errors
///
/// Returns the first `io::Error` from create, write, sync or rename. The temp
/// file is removed on every failure.
pub fn atomic_write(path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
    let tmp = temp_path(path);
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(mode);
        }
        #[cfg(not(unix))]
        let _ = mode;
        let mut file = options.open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
        return result;
    }
    // Best effort: the data is already durable, only the rename is at stake.
    if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty())
        && let Ok(handle) = fs::File::open(dir)
    {
        let _ = handle.sync_all();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leftovers(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| Path::new(n).extension().is_some_and(|e| e == "tmp"))
            .collect()
    }

    #[test]
    fn writes_and_replaces_without_leftovers() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("a.json");
        atomic_write(&path, b"one", MODE_PRIVATE).unwrap();
        atomic_write(&path, b"two", MODE_PRIVATE).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");
        assert!(leftovers(dir.path()).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn created_owner_only_even_over_a_wider_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("a.json");
        fs::write(&path, b"old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        atomic_write(&path, b"new", MODE_PRIVATE).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn failed_rename_removes_the_temp_file() {
        let dir = tempfile::TempDir::new().unwrap();
        // A non-empty directory cannot be replaced by a file: rename fails.
        let target = dir.path().join("target");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("child"), b"x").unwrap();
        assert!(atomic_write(&target, b"data", MODE_PRIVATE).is_err());
        assert!(leftovers(dir.path()).is_empty());
    }

    #[test]
    fn temp_paths_are_unique_and_beside_the_target() {
        let path = PathBuf::from("/tmp/portable.enc");
        let (a, b) = (temp_path(&path), temp_path(&path));
        assert_ne!(a, b);
        assert_eq!(a.parent(), path.parent());
    }
}
