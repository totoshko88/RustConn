//! Script recording — captures terminal input into a `.rsc` file.
//!
//! This is Phase 0 of the scripting feature. It records what the *user* types
//! and writes a readable text file; it deliberately does not replay. The
//! replay runner and the pattern-matching verbs arrive later and parse the
//! same format this produces, which is why the format lives in
//! `rustconn-core::automation::script` and not here.
//!
//! # Why `commit` and suppression
//!
//! VTE raises `commit` for every byte sent to the PTY — whether a human typed
//! it or RustConn sent it. `feed_child` raises it too. Recording the raw
//! stream would therefore capture automatic logins, and an expect rule that
//! answers `${password}` would write that password to disk in clear text.
//!
//! Every automated send is wrapped in [`suppress_recording`], which is a
//! thread-local depth counter rather than a field on the notebook: the send
//! paths live in three different owners (`AutomationSession` holds a
//! `Terminal`, not the notebook), and GTK is single-threaded, so a counter is
//! both reachable from all of them and impossible to leak across sessions.
//!
//! # Known limitation
//!
//! The recorder sees input, not intent. It cannot know that a command needed
//! to wait for a prompt first, so a recorded script is a blind keystroke
//! replay and will desync against a host that answers at a different speed.
//! Prompts seen during recording are emitted as `#` comments suggesting a
//! `wait`, which the user uncomments by hand.

use std::cell::Cell;
use std::path::PathBuf;

use rustconn_core::automation::{StepRecorder, render_script};

use super::*;

thread_local! {
    /// Nesting depth of automated sends on this thread.
    ///
    /// A depth rather than a bool because `suppress_recording` wraps a closure
    /// and callers legitimately nest — an expect rule firing inside a macro
    /// must not un-suppress when the inner scope ends.
    static AUTOMATED_SEND_DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// Whether the current thread is inside an automated send.
///
/// Read by the recorder's `commit` callback; `true` means the bytes belong to
/// RustConn, not to the user.
pub(crate) fn in_automated_send() -> bool {
    AUTOMATED_SEND_DEPTH.get() > 0
}

/// RAII counterpart to the depth increment.
///
/// Without this, a panic inside a wrapped send would leave the depth raised
/// for the rest of the process and silently disable script recording for the
/// rest of the session. The codebase does use `catch_unwind` around session
/// setup, so a panic unwinding through here is a real path, not a theoretical
/// one.
struct SuppressionGuard;

impl Drop for SuppressionGuard {
    fn drop(&mut self) {
        AUTOMATED_SEND_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

/// Runs `body` with script recording suppressed.
///
/// Wrap every send that RustConn originates — key sequences, macros, snippets,
/// expect-rule responses. Without this an automatic login ends up written to
/// disk in clear text.
pub(crate) fn suppress_recording<T>(body: impl FnOnce() -> T) -> T {
    AUTOMATED_SEND_DEPTH.with(|depth| depth.set(depth.get() + 1));
    let _guard = SuppressionGuard;
    body()
}

/// Why a recording could not be saved.
///
/// The user's keystrokes live only in memory, so a failure here destroys the
/// recording. It is reported rather than logged: a file that silently does not
/// appear is indistinguishable from a feature that did not work.
#[derive(Debug, thiserror::Error)]
pub enum ScriptSaveError {
    /// No recording was running, so there was nothing to save.
    #[error("no script recording is in progress")]
    NoRecording,

    /// The config directory could not be resolved.
    #[error("the scripts directory could not be located")]
    NoScriptsDir,

    /// The scripts directory could not be created.
    #[error("the scripts directory could not be created: {0}")]
    CreateDir(#[source] std::io::Error),

    /// The file could not be written.
    #[error("the script file could not be written: {0}")]
    Write(#[source] std::io::Error),

    /// The rendered recording was rejected by the `.rsc` parser.
    #[error("the recorded script could not be parsed: {0}")]
    Invalid(#[source] rustconn_core::automation::ScriptParseError),
}

/// A recording in progress.
pub struct ActiveScriptRecording {
    /// Session whose input is being captured.
    session_id: Uuid,
    /// Name written into the file header and used for the filename.
    name: String,
    /// Accumulator that turns raw keystrokes into steps.
    recorder: StepRecorder,
}

/// Outcome of stopping a recording.
pub struct RecordedScript {
    /// File that was written.
    pub path: PathBuf,
    /// Number of steps captured.
    pub step_count: usize,
}

/// Finishes a recording whose session is going away.
///
/// Called from the tab-close path with a recording still open. The keystrokes
/// only exist in memory, so they are written out rather than dropped: a tab
/// closed by the user mid-recording is a deliberate stop, not a crash, and it
/// also stops the shared menu label from being left showing "Stop Script
/// Recording" on every other terminal.
///
/// Returns `None` when `session_id` was not the one being recorded, or when
/// the save failed.
///
/// Takes the state cells rather than `&self` because the tab-close handler
/// captures cloned `Rc`s, not the notebook — the same pattern every other
/// teardown in `setup_tab_view_signals` uses.
pub(crate) fn finalize_for_session(
    recording: &Rc<RefCell<Option<ActiveScriptRecording>>>,
    menu_section: &Rc<gio::Menu>,
    session_id: Uuid,
) -> Option<RecordedScript> {
    let is_this_session = recording
        .borrow()
        .as_ref()
        .is_some_and(|r| r.session_id == session_id);
    if !is_this_session {
        return None;
    }
    let recorded = match stop_recording(recording) {
        Ok(recorded) => recorded,
        Err(e) => {
            // The tab is already gone, so there is nothing left to show the
            // error against. Log it rather than swallow it: the recording is
            // genuinely gone and that is worth a line in the log.
            tracing::error!(%e, "Lost a script recording when its session closed");
            return None;
        }
    };
    rebuild_menu_section(menu_section, false);
    Some(recorded)
}

/// Shared body of [`TerminalNotebook::rebuild_script_menu_section`].
///
/// The last three entries are the read-only network tools. They live here
/// rather than in the application menu because they act on the terminal under
/// the pointer; choosing the terminal and the tool in one gesture makes the
/// target unambiguous.
fn rebuild_menu_section(section: &Rc<gio::Menu>, recording: bool) {
    section.remove_all();
    if recording {
        section.append(
            Some(&crate::i18n::i18n("Stop Script Recording")),
            Some("win.stop-record-script"),
        );
    } else {
        section.append(
            Some(&crate::i18n::i18n("Record Script…")),
            Some("win.record-script"),
        );
    }
    section.append(
        Some(&crate::i18n::i18n("Share Session (read-only)")),
        Some("win.share-session"),
    );
    section.append(
        Some(&crate::i18n::i18n("Start TFTP Server")),
        Some("win.start-tftp"),
    );
    section.append(
        Some(&crate::i18n::i18n("Edit Remote File…")),
        Some("win.edit-remote-file"),
    );
}

/// Shared body of [`TerminalNotebook::stop_script_recording`].
fn stop_recording(
    state: &Rc<RefCell<Option<ActiveScriptRecording>>>,
) -> Result<RecordedScript, ScriptSaveError> {
    let active = state
        .borrow_mut()
        .take()
        .ok_or(ScriptSaveError::NoRecording)?;
    let steps = active.recorder.finish();

    let dir = rustconn_core::script::scripts_dir().ok_or(ScriptSaveError::NoScriptsDir)?;
    std::fs::create_dir_all(&dir).map_err(ScriptSaveError::CreateDir)?;

    // The connection name alone would collide on the second recording of the
    // same session (and "rt" says nothing). The local timestamp makes each
    // recording distinct and sorts chronologically; the suffix guards the
    // one-in-a-second case.
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let base = format!("{}-{stamp}", sanitize_script_name(&active.name));
    let path = available_script_path(&dir, &base);
    let contents = render_script(&active.name, &steps, &[RECORDING_NOTICE.to_string()]);
    // The recorder writes what the engine reads; a parse failure here would
    // hand the user a file the Scripts dialog cannot open, so refuse to save.
    rustconn_core::automation::parse_program(&contents).map_err(ScriptSaveError::Invalid)?;
    // 0600: typed passwords end up in the file, so keep it private to the user.
    rustconn_core::fs_atomic::atomic_write(&path, contents.as_bytes(), 0o600)
        .map_err(ScriptSaveError::Write)?;

    tracing::info!(?path, steps = steps.len(), "Script recording written");
    Ok(RecordedScript {
        path,
        step_count: steps.len(),
    })
}

impl TerminalNotebook {
    /// Starts recording user input on `session_id`.
    ///
    /// Returns `false` if the session has no terminal, or if a recording is
    /// already running — one notebook-wide recording at a time keeps the
    /// commit subscription to a single registration.
    pub fn start_script_recording(&self, session_id: Uuid, name: &str) -> bool {
        if self.get_terminal(session_id).is_none() {
            tracing::warn!(%session_id, "Cannot record script: no terminal for session");
            return false;
        }
        if self.is_script_recording() {
            tracing::warn!("A script recording is already in progress");
            return false;
        }

        *self.script_recording.borrow_mut() = Some(ActiveScriptRecording {
            session_id,
            name: name.to_string(),
            recorder: StepRecorder::new(),
        });

        let recording = Rc::clone(&self.script_recording);
        let relays = Rc::clone(&self.pty_relays);
        self.connect_commit(session_id, move |text| {
            if in_automated_send() {
                return;
            }
            // `borrow_mut` here rather than in the signature: `connect_commit`
            // takes an `Fn`, and this runs inside VTE's own signal emission. The
            // binding has to outlive the use — a temporary `RefMut` would be
            // dropped while `active` still borrowed from it.
            let mut active_slot = recording.borrow_mut();
            let Some(active) = active_slot.as_mut() else {
                return;
            };
            if active.session_id != session_id {
                return;
            }
            // No echo means a password prompt: keep the keystrokes off disk.
            let echoing = relays
                .borrow()
                .get(&session_id)
                .is_none_or(|relay| relay.echo_enabled());
            if echoing {
                active.recorder.record(text);
            } else {
                active.recorder.record_hidden(text);
            }
        });

        self.set_script_marker(session_id, true);
        tracing::info!(%session_id, "Script recording started");
        true
    }

    /// Stops recording and writes the script to disk.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptSaveError::NoRecording`] if nothing was being recorded,
    /// or a directory/write error. The recording is consumed either way: a
    /// failed save drops the steps rather than leaving them to be written by a
    /// second, later call.
    pub fn stop_script_recording(&self) -> Result<RecordedScript, ScriptSaveError> {
        let session_id = self
            .script_recording
            .borrow()
            .as_ref()
            .map(|r| r.session_id);
        if let Some(session_id) = session_id {
            self.set_script_marker(session_id, false);
        }
        stop_recording(&self.script_recording)
    }
    /// Shows or hides the recording marker in the tab title, so a recording
    /// that captures keystrokes (passwords included) is never invisible.
    fn set_script_marker(&self, session_id: Uuid, on: bool) {
        if let Some(page) = self.sessions.borrow().get(&session_id) {
            let title = page.title().to_string();
            page.set_title(&with_title_marker(
                &title,
                SCRIPT_TITLE_MARKER,
                on,
                &[
                    BROADCAST_TITLE_MARKER,
                    SHARED_TITLE_MARKER,
                    SCRIPT_TITLE_MARKER,
                ],
            ));
        }
    }

    /// Whether a recording is currently running.
    #[must_use]
    pub fn is_script_recording(&self) -> bool {
        self.script_recording.borrow().is_some()
    }

    /// Finishes a recording whose session is going away.
    ///
    /// See [`finalize_for_session`], which this delegates to.
    pub fn finalize_recording_for_session(&self, session_id: Uuid) -> Option<RecordedScript> {
        finalize_for_session(
            &self.script_recording,
            &self.script_menu_section,
            session_id,
        )
    }

    /// Rewrites the shared script menu section for the current state.
    ///
    /// One live `gio::Menu` shared by every terminal, same as the snippet
    /// section: rebuilding it in place means all open terminals show the new
    /// label without each holding its own model. The item is a single toggle,
    /// so its label is what tells the user which way it goes.
    pub fn rebuild_script_menu_section(&self) {
        rebuild_menu_section(&self.script_menu_section, self.is_script_recording());
    }

    /// Steps captured so far, for a live count in the UI.
    #[must_use]
    pub fn recorded_step_count(&self) -> usize {
        self.script_recording
            .borrow()
            .as_ref()
            .map_or(0, |r| r.recorder.len())
    }
}

/// Comment emitted at the top of every recorded file.
const RECORDING_NOTICE: &str = "Recorded by RustConn. Review before running: \
steps are a blind replay, and a host that answers at a different pace will desync. Input typed while the \
terminal did not echo (a password prompt) is replaced by ${password}.";

/// Makes a connection name safe to use as a filename.
///
/// Path separators become `-`, so a name like `web/prod` cannot escape the
/// scripts directory. Leading and trailing `.` and `-` are then trimmed so a
/// name that was entirely traversal (`../../etc/passwd`) cannot leave a
/// leading `..` behind.
fn sanitize_script_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches(['-', '.']);
    if trimmed.is_empty() {
        "session".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Picks `base.rsc`, then `base-1.rsc`, so a recording never overwrites an
/// earlier file whose name is already taken.
fn available_script_path(dir: &std::path::Path, base: &str) -> PathBuf {
    let mut path = dir.join(format!("{base}.rsc"));
    let mut suffix = 1u32;
    while path.exists() {
        path = dir.join(format!("{base}-{suffix}.rsc"));
        suffix += 1;
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_names_do_not_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("rt-20261119-120000.rsc"), b"x").unwrap();
        let path = available_script_path(dir.path(), "rt-20261119-120000");
        assert_eq!(
            path.file_name().unwrap().to_string_lossy(),
            "rt-20261119-120000-1.rsc"
        );
    }

    #[test]
    fn sanitizes_path_separators() {
        assert_eq!(sanitize_script_name("web/prod"), "web-prod");
        assert_eq!(sanitize_script_name("../../etc/passwd"), "etc-passwd");
    }

    #[test]
    fn falls_back_when_name_is_empty() {
        assert_eq!(sanitize_script_name("///"), "session");
        assert_eq!(sanitize_script_name(".."), "session");
    }

    #[test]
    fn keeps_ordinary_names_readable() {
        assert_eq!(sanitize_script_name("edge-router_01"), "edge-router_01");
    }

    #[test]
    fn suppression_is_nested() {
        assert!(!in_automated_send());
        suppress_recording(|| {
            assert!(in_automated_send());
            suppress_recording(|| {
                assert!(in_automated_send());
            });
            // Inner scope must not un-suppress the outer one.
            assert!(in_automated_send());
        });
        assert!(!in_automated_send());
    }

    #[test]
    fn suppression_survives_a_panic() {
        let result = std::panic::catch_unwind(|| {
            suppress_recording(|| panic!("send blew up"));
        });
        assert!(result.is_err());
        // Otherwise the depth would stay raised and recording would be
        // silently dead for the rest of the process.
        assert!(!in_automated_send());
    }
}
