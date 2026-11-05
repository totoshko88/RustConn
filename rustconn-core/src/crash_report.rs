//! Local crash reporting: a panic hook that records a Rust panic to a file, and
//! a startup check that surfaces the most recent one.
//!
//! The hook writes the panic message, location, a captured backtrace and the
//! build version to `$XDG_STATE_HOME/rustconn/crash-<unix-ts>.log` (falling back
//! to the OS data directory where there is no state dir, e.g. macOS). It chains
//! the previous hook so the normal abort/log behaviour is preserved.
//!
//! The report stays **local**. Nothing is sent anywhere automatically: the next
//! launch shows a banner pointing at the file and asks the user to review it
//! before sharing, because a panic message can contain formatted data.
//!
//! This catches Rust panics only. A `SIGSEGV` inside a C library (GTK, VTE,
//! libwayland) does not unwind through this hook and is not recorded here; the
//! user guide says so plainly.

use std::backtrace::Backtrace;
use std::fs;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Once;
use std::time::{SystemTime, UNIX_EPOCH};

/// A crash log left by a previous run, ready to show the user.
#[derive(Debug, Clone)]
pub struct CrashReport {
    /// Absolute path to the crash log file.
    pub path: PathBuf,
    /// Unix timestamp parsed from the file name, for ordering and display.
    pub unix_secs: u64,
}

/// Guards against installing the hook more than once.
static HOOK_INSTALLED: Once = Once::new();

/// The directory crash logs are written to and read from.
///
/// `$XDG_STATE_HOME/rustconn` on Linux (via `dirs::state_dir`), falling back to
/// the OS data-local directory joined with `rustconn` where there is no state
/// directory (macOS, Windows). Returns `None` only when neither is resolvable,
/// in which case crash reporting is silently disabled rather than guessed.
#[must_use]
pub fn crash_dir() -> Option<PathBuf> {
    dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .map(|base| base.join("rustconn"))
}

/// Installs the panic hook. Idempotent: a second call is a no-op.
///
/// Chains the existing hook so the default message and the process abort still
/// happen; this only adds the on-disk record. Call once, early in `main`, before
/// any thread that might panic is spawned.
pub fn install_panic_hook() {
    HOOK_INSTALLED.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // Capture first, so a failure writing the file cannot swallow the
            // panic: the previous hook still runs at the end no matter what.
            write_crash_log(info);
            previous(info);
        }));
    });
}

/// Writes one crash log for `info`. Best-effort: any I/O error is logged and
/// swallowed so the panic path is never derailed by the reporter.
fn write_crash_log(info: &std::panic::PanicHookInfo<'_>) {
    let Some(dir) = crash_dir() else {
        tracing::error!("panic occurred but no crash-log directory could be resolved");
        return;
    };
    if let Err(e) = fs::create_dir_all(&dir) {
        tracing::error!(error = %e, dir = %dir.display(), "could not create the crash-log directory");
        return;
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let path = dir.join(format!("crash-{now}.log"));

    // The panic payload is usually a &str or String; anything else is opaque.
    let message = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<non-string panic payload>".to_string());
    let location = info
        .location()
        .map_or_else(|| "<unknown location>".to_string(), ToString::to_string);
    let backtrace = Backtrace::force_capture();

    let body = format!(
        "RustConn {version} crashed\n\
         time:       {now} (unix seconds)\n\
         location:   {location}\n\
         message:    {message}\n\
         \n\
         backtrace:\n{backtrace}\n",
        version = env!("CARGO_PKG_VERSION"),
    );

    // Also mirror to the log so a user running from a terminal sees it at once.
    tracing::error!(path = %path.display(), %location, "RustConn panicked; wrote a crash log");

    match fs::File::create(&path) {
        Ok(mut file) => {
            if let Err(e) = file.write_all(body.as_bytes()) {
                tracing::error!(error = %e, "could not write the crash log");
            }
        }
        Err(e) => {
            tracing::error!(error = %e, path = %path.display(), "could not create the crash log");
        }
    }
}

/// Parses the timestamp from a `crash-<digits>.log` file name, or `None` when
/// the name is not a crash log. One place so [`take_pending_crash`] and
/// [`clear_crash_reports`] agree on exactly which files are crash logs.
fn crash_timestamp(file_name: &str) -> Option<u64> {
    file_name
        .strip_prefix("crash-")
        .and_then(|rest| rest.strip_suffix(".log"))
        .and_then(|digits| digits.parse::<u64>().ok())
}

/// Returns the most recent crash log, if any, without deleting it.
///
/// The caller shows it to the user and then calls [`clear_crash_reports`] once
/// the banner is dismissed, so the same crash is not reported on every launch.
/// Files whose name does not match `crash-<digits>.log` are ignored.
#[must_use]
pub fn take_pending_crash() -> Option<CrashReport> {
    let dir = crash_dir()?;
    let mut latest: Option<CrashReport> = None;
    for entry in fs::read_dir(&dir).ok()?.flatten() {
        let name = entry.file_name();
        let Some(ts) = name.to_str().and_then(crash_timestamp) else {
            continue;
        };
        if latest.as_ref().is_none_or(|cur| ts > cur.unix_secs) {
            latest = Some(CrashReport {
                path: entry.path(),
                unix_secs: ts,
            });
        }
    }
    latest
}

/// Deletes every crash log. Called once the user has been shown the banner, so a
/// handled crash does not keep reappearing. Best-effort: errors are logged.
pub fn clear_crash_reports() {
    let Some(dir) = crash_dir() else { return };
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let is_crash_log = name.to_str().and_then(crash_timestamp).is_some();
        if is_crash_log && let Err(e) = fs::remove_file(entry.path()) {
            tracing::warn!(error = %e, "could not remove a crash log");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `crash-<ts>.log` name parses to its timestamp; other names are ignored.
    /// Driven through a temp dir so it does not touch the real state directory.
    #[test]
    fn take_pending_picks_the_newest_crash_log() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let dir = tmp.path();
        fs::write(dir.join("crash-100.log"), b"old").unwrap();
        fs::write(dir.join("crash-300.log"), b"new").unwrap();
        fs::write(dir.join("crash-200.log"), b"mid").unwrap();
        fs::write(dir.join("not-a-crash.log"), b"ignore").unwrap();
        fs::write(dir.join("crash-bogus.log"), b"ignore").unwrap();

        let newest = newest_in(dir).expect("a crash log is found");
        assert_eq!(newest.unix_secs, 300);
        assert!(newest.path.ends_with("crash-300.log"));
    }

    #[test]
    fn take_pending_is_none_for_a_dir_without_crash_logs() {
        let tmp = tempfile::tempdir().expect("temp dir");
        fs::write(tmp.path().join("other.txt"), b"x").unwrap();
        assert!(newest_in(tmp.path()).is_none());
    }

    /// Test-only mirror of [`take_pending_crash`] scoped to an explicit dir, so
    /// the scan logic is covered without depending on `$XDG_STATE_HOME`. Shares
    /// [`crash_timestamp`] with the real functions, so a change to what counts
    /// as a crash log is exercised here too.
    fn newest_in(dir: &std::path::Path) -> Option<CrashReport> {
        let mut latest: Option<CrashReport> = None;
        for entry in fs::read_dir(dir).ok()?.flatten() {
            let name = entry.file_name();
            let Some(ts) = name.to_str().and_then(crash_timestamp) else {
                continue;
            };
            if latest.as_ref().is_none_or(|cur| ts > cur.unix_secs) {
                latest = Some(CrashReport {
                    path: entry.path(),
                    unix_secs: ts,
                });
            }
        }
        latest
    }
}
