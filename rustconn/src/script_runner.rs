//! GUI host adapter for `.rsc` scripts.
//!
//! Runs [`rustconn_core::automation::ScriptEngine`] against a live session in
//! the `TerminalNotebook`. The engine is event-driven; this module supplies the
//! host that performs the side effects (send keys, dialogs, session
//! management, local commands) and re-drives the engine from callbacks.
//!
//! All script activity happens on the GTK main thread; the one blocking piece,
//! a `run` command, executes off-main via `spawn_blocking_with_callback`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use rustconn_core::automation::{
    DEFAULT_TIMEOUT_MS, KeySequence, PendingKind, RUN_TIMEOUT_CEILING_MS, SCRUBBED_ENV_VARS,
    ScriptEngine, ScriptHost, ScriptStep,
};
use uuid::Uuid;

use crate::state::SharedAppState;
use crate::window::types::{
    SessionSplitBridges, SharedMonitoring, SharedNotebook, SharedSidebar, SharedSplitView,
};

/// Handles a script runner needs that live for the whole application: the
/// split-view bridges, the sidebar, monitoring and the main window. Published
/// by `MainWindow::new`.
#[derive(Clone)]
pub struct ScriptRuntime {
    /// Per-session split bridge map.
    pub bridges: SessionSplitBridges,
    /// The window-wide split view.
    pub split_view: SharedSplitView,
    /// Weak handle to the main window (for dialogs and window actions).
    pub main_window: glib::WeakRef<adw::ApplicationWindow>,
    /// The connection sidebar.
    pub sidebar: SharedSidebar,
    /// The monitoring coordinator.
    pub monitoring: SharedMonitoring,
}

impl ScriptRuntime {
    /// Creates the published runtime handles.
    #[must_use]
    pub fn new(
        bridges: SessionSplitBridges,
        split_view: SharedSplitView,
        main_window: glib::WeakRef<adw::ApplicationWindow>,
        sidebar: SharedSidebar,
        monitoring: SharedMonitoring,
    ) -> Self {
        Self {
            bridges,
            split_view,
            main_window,
            sidebar,
            monitoring,
        }
    }
}

/// How a script ended, reported to the caller that started it.
#[derive(Debug, Clone)]
pub enum ScriptOutcome {
    /// Ran to the end without an error.
    Finished,
    /// Stopped by the user.
    Stopped,
    /// Stopped because an instruction failed.
    Failed(String),
}

thread_local! {
    static SCRIPT_RUNTIME: RefCell<Option<Rc<ScriptRuntime>>> = const { RefCell::new(None) };
}

/// Publishes the application-wide script runtime (called by `MainWindow::new`).
pub fn publish_script_runtime(runtime: ScriptRuntime) {
    SCRIPT_RUNTIME.with(|cell| *cell.borrow_mut() = Some(Rc::new(runtime)));
}

/// Returns the published script runtime, if any.
#[must_use]
pub fn script_runtime() -> Option<Rc<ScriptRuntime>> {
    SCRIPT_RUNTIME.with(|cell| cell.borrow().as_ref().map(Rc::clone))
}

// Active scripts keyed by the session they drive.
//
// The map holds the strong `Rc` on purpose. With only a `Weak` here, the
// runner's last strong reference was the local in `start_for_session`, so it
// was freed the moment that function returned — every script died at its first
// asynchronous step (expect, wait, ask, run) and the session saw nothing. The
// entry is removed by `cleanup`, which is what lets the runner be dropped then.
thread_local! {
    static ACTIVE_SCRIPTS: RefCell<HashMap<Uuid, Rc<RefCell<ScriptRunner>>>> =
        RefCell::new(HashMap::new());
}

/// The `ScriptHost` implementation for the GUI.
struct GuiHost {
    notebook: SharedNotebook,
    state: SharedAppState,
    sidebar: SharedSidebar,
    monitoring: SharedMonitoring,
    runtime: Rc<ScriptRuntime>,
    session_id: Uuid,
    engine: Rc<RefCell<ScriptEngine>>,
    log_writer: Rc<RefCell<Option<File>>>,
    /// Re-drives the engine after an async event completes.
    resume: Rc<dyn Fn()>,
}

impl ScriptHost for GuiHost {
    fn send(&mut self, sequence: &KeySequence) {
        self.notebook
            .execute_key_sequence(self.session_id, sequence);
    }

    fn print_message(&mut self, text: &str) {
        // Yellow, and rendered locally: a status line must not be typed into
        // the remote shell, which is where a key sequence would put it.
        let msg = format!("\x1b[33m{text}\x1b[0m\r\n");
        self.notebook
            .feed_local_output(self.session_id, msg.as_bytes());
    }

    fn set_log(&mut self, path: Option<&str>) {
        let mut writer = self.log_writer.borrow_mut();
        *writer = match path {
            Some(path) => {
                let file = File::options().create(true).append(true).open(path);
                match file {
                    Ok(file) => Some(file),
                    Err(e) => {
                        tracing::error!(%e, path, "script log file could not be opened");
                        None
                    }
                }
            }
            None => None,
        };
    }

    fn get_prompt(&mut self) -> Option<String> {
        // Best effort: the terminal's current line, with escape sequences
        // stripped and trailing spaces trimmed.
        let text = self.notebook.get_cursor_line_text(self.session_id)?;
        let cleaned = strip_ansi(&text).trim().to_string();
        (!cleaned.is_empty()).then_some(cleaned)
    }

    fn ask(&mut self, prompt: &str, echo: bool) {
        let engine = Rc::clone(&self.engine);
        let resume = Rc::clone(&self.resume);
        let parent = parent_widget(&self.runtime);
        show_ask_dialog(parent.as_ref(), prompt, echo, move |answer| {
            let mut engine = engine.borrow_mut();
            engine.on_ask_result(answer);
            drop(engine);
            resume();
        });
    }

    fn message(&mut self, text: &str, ok_button: bool) {
        let parent = parent_widget(&self.runtime);
        show_message_dialog(parent.as_ref(), text, ok_button);
    }

    fn start_session(&mut self, selector: &str, manual_login: bool, cluster: Option<&str>) {
        let selector = selector.to_string();
        let engine = Rc::clone(&self.engine);
        let resume = Rc::clone(&self.resume);
        let state = Rc::clone(&self.state);
        let notebook = Rc::clone(&self.notebook);
        let sidebar = Rc::clone(&self.sidebar);
        let monitoring = Rc::clone(&self.monitoring);
        let split_view = Rc::clone(&self.runtime.split_view);

        // `cluster=` is parsed by core but not implemented here. Say so instead of
        // silently starting a plain session. shortcut: reject, implement via
        // `ClusterManager::start_session` if scripts ever need it.
        if let Some(cluster) = cluster {
            let message = format!("start: cluster=<name> is not supported (got `{cluster}`)");
            glib::idle_add_local_once(move || {
                engine.borrow_mut().finish(Some(message));
                resume();
            });
            return;
        }

        // Defer every engine mutation to the main loop: the engine holds its
        // `RefCell` borrow while this host method runs, so touching it here
        // panics with "RefCell already borrowed".
        let Some(connection_id) = resolve_connection_id(&state, &selector) else {
            glib::idle_add_local_once(move || {
                engine
                    .borrow_mut()
                    .finish(Some(format!("Unknown connection: {selector}")));
                resume();
            });
            return;
        };

        let observer = crate::window::types::SessionStartObserver::new(move |session_id| {
            glib::idle_add_local_once(move || {
                let mut engine = engine.borrow_mut();
                engine
                    .on_session_started(Some(session_id.to_string()), Some(session_id.to_string()));
                drop(engine);
                resume();
            });
        });

        crate::window::MainWindow::start_connection_with_credential_resolution_observed(
            state,
            notebook,
            split_view,
            sidebar,
            monitoring,
            connection_id,
            None,
            Some(observer),
        );
        let _ = manual_login;
    }

    fn select_session(&mut self, selector: &str) {
        let selector = selector.to_string();
        let engine = Rc::clone(&self.engine);
        let resume = Rc::clone(&self.resume);
        let notebook = Rc::clone(&self.notebook);
        let bridges = Rc::clone(&self.runtime.bridges);

        let session_id = if let Ok(id) = Uuid::parse_str(&selector) {
            Some(id)
        } else {
            notebook
                .get_all_sessions()
                .into_iter()
                .find(|s| s.name == selector)
                .map(|s| s.id)
        };

        match session_id {
            Some(id) => {
                focus_session(&notebook, &bridges, id);
                glib::idle_add_local_once(move || {
                    let mut engine = engine.borrow_mut();
                    engine.on_session_started(Some(id.to_string()), Some(id.to_string()));
                    drop(engine);
                    resume();
                });
            }
            None => {
                glib::idle_add_local_once(move || {
                    engine
                        .borrow_mut()
                        .finish(Some(format!("No open session named `{selector}`")));
                    resume();
                });
            }
        }
    }

    fn stop_session(&mut self, session: &str) {
        if let Ok(id) = Uuid::parse_str(session) {
            self.notebook.close_tab(id);
        } else if let Some(id) = self
            .notebook
            .get_all_sessions()
            .into_iter()
            .find(|s| s.name == session)
            .map(|s| s.id)
        {
            self.notebook.close_tab(id);
        } else {
            tracing::warn!(session, "script stop: no such session");
        }
    }

    fn split(&mut self, first: &str, second: &str, vertical: bool) {
        let Some(first_id) = session_id_from_ref(&self.notebook, first) else {
            return;
        };
        let Some(second_id) = session_id_from_ref(&self.notebook, second) else {
            return;
        };

        let bridges = Rc::clone(&self.runtime.bridges);
        // Focus the first session, split it, then place the second into the new pane.
        focus_session(&self.notebook, &bridges, first_id);

        let action = if vertical {
            "win.split-vertical"
        } else {
            "win.split-horizontal"
        };
        activate_window_action(&self.runtime.main_window, action, None);

        // Give the split a beat to materialize, then show the second session.
        let bridges2 = Rc::clone(&bridges);
        let notebook2 = Rc::clone(&self.notebook);
        glib::idle_add_local_once(move || {
            let bridge = bridges2.borrow().get(&first_id).cloned();
            if let Some(bridge) = bridge {
                let _ = bridge.show_session(second_id);
            } else {
                // Fallback: switch to the second tab so the user still sees it.
                notebook2.switch_to_tab(second_id);
            }
        });
    }

    fn unsplit(&mut self, session: &str) {
        let Some(id) = session_id_from_ref(&self.notebook, session) else {
            return;
        };
        let bridges = Rc::clone(&self.runtime.bridges);
        focus_session(&self.notebook, &bridges, id);
        activate_window_action(&self.runtime.main_window, "win.unsplit", None);
    }

    fn to_window(&mut self, session: &str) {
        let Some(id) = session_id_from_ref(&self.notebook, session) else {
            return;
        };
        let bridges = Rc::clone(&self.runtime.bridges);
        focus_session(&self.notebook, &bridges, id);
        activate_window_action(
            &self.runtime.main_window,
            "win.detach-session",
            Some(&glib::Variant::from(id.to_string())),
        );
    }

    fn to_tab(&mut self, session: &str) {
        let Some(id) = session_id_from_ref(&self.notebook, session) else {
            return;
        };
        let bridges = Rc::clone(&self.runtime.bridges);
        focus_session(&self.notebook, &bridges, id);
        activate_window_action(&self.runtime.main_window, "win.pop-pane-to-tab", None);
    }

    fn run_command(&mut self, command: &str, timeout_ms: Option<u32>) {
        let engine = Rc::clone(&self.engine);
        let resume = Rc::clone(&self.resume);
        let command = command.to_string();
        crate::utils::spawn_blocking_with_callback(
            move || {
                let result = crate::async_utils::with_runtime(|rt| {
                    rt.block_on(run_sh(&command, timeout_ms))
                });
                result.unwrap_or_else(|e| Err(format!("async runtime unavailable: {e}")))
            },
            move |result: Result<(i32, String), String>| {
                let mut engine = engine.borrow_mut();
                match result {
                    Ok((code, stdout)) => engine.on_run_result(code, stdout),
                    Err(message) => engine.finish(Some(message)),
                }
                drop(engine);
                resume();
            },
        );
    }
}

/// A running script: engine + host + the GUI handles it needs.
pub struct ScriptRunner {
    engine: Rc<RefCell<ScriptEngine>>,
    notebook: SharedNotebook,
    state: SharedAppState,
    sidebar: SharedSidebar,
    monitoring: SharedMonitoring,
    runtime: Rc<ScriptRuntime>,
    session_id: Uuid,
    connection_id: Uuid,
    log_writer: Rc<RefCell<Option<File>>>,
    timer: Option<glib::SourceId>,
    /// Called once when the script finishes, with how it ended.
    on_finish: Option<Rc<dyn Fn(ScriptOutcome)>>,
    /// When the script started, for the elapsed-time report.
    started: Instant,
    /// Whether the script was cancelled by the user rather than failing.
    stopped_by_user: bool,
    finished: bool,
    /// Whether [`Self::cleanup`] already ran, so a cancel after the finish
    /// does not repeat the notification.
    cleaned: bool,
    /// Weak self-reference for callbacks; set after construction.
    this: Weak<RefCell<Self>>,
}

impl ScriptRunner {
    /// Starts a script from its `.rsc` source against a live session.
    ///
    /// Returns an error string when the script fails to parse or the session
    /// already runs another script.
    #[expect(
        clippy::too_many_arguments,
        reason = "start_entry_k the full GUI environment the runner needs; bundling into a struct only restates the field list"
    )]
    pub fn start_for_session(
        source: &str,
        session_id: Uuid,
        connection_id: Uuid,
        state: SharedAppState,
        notebook: SharedNotebook,
        sidebar: SharedSidebar,
        monitoring: SharedMonitoring,
        runtime: Rc<ScriptRuntime>,
        on_finish: Option<Rc<dyn Fn(ScriptOutcome)>>,
    ) -> Result<(), String> {
        if ACTIVE_SCRIPTS.with(|cell| cell.borrow().contains_key(&session_id)) {
            return Err("a script is already running on this session".to_string());
        }

        let engine = ScriptEngine::parse(source).map_err(|e| e.to_string())?;
        let engine = Rc::new(RefCell::new(engine));

        let runner = Rc::new(RefCell::new(Self {
            engine,
            notebook,
            state,
            sidebar,
            monitoring,
            runtime,
            session_id,
            connection_id,
            log_writer: Rc::new(RefCell::new(None)),
            timer: None,
            on_finish,
            started: Instant::now(),
            stopped_by_user: false,
            finished: false,
            cleaned: false,
            this: Weak::new(),
        }));
        runner.borrow_mut().this = Rc::downgrade(&runner);
        runner.borrow_mut().register_observer();

        ACTIVE_SCRIPTS.with(|cell| {
            cell.borrow_mut().insert(session_id, Rc::clone(&runner));
        });

        {
            let mut r = runner.borrow_mut();
            // Seed the engine with what the terminal already shows. A local
            // shell (or a session whose prompt is already on screen) would
            // otherwise never satisfy the script's first `expect` — output
            // that arrived before the observer was registered is not replayed.
            if let Some(text) = r.notebook.get_terminal_text(session_id)
                && !text.is_empty()
            {
                r.engine.borrow_mut().on_output(text.as_bytes());
            }
            r.drive();
        }
        Ok(())
    }

    /// Registers the output observer that feeds the engine.
    fn register_observer(&self) {
        let engine = Rc::clone(&self.engine);
        let log_writer = Rc::clone(&self.log_writer);
        let this = self.this.clone();

        self.notebook
            .add_output_observer(self.session_id, move |chunk| {
                if let Some(writer) = log_writer.borrow_mut().as_mut() {
                    let _ = writer.write_all(chunk);
                }
                let run = {
                    let mut engine = engine.borrow_mut();
                    engine.on_output(chunk)
                };
                if run && let Some(runner) = this.upgrade() {
                    runner.borrow_mut().drive();
                }
            });
    }

    /// Executes steps until the script suspends or finishes, then arms the
    /// next timer.
    fn drive(&mut self) {
        if self.finished {
            return;
        }
        self.pump();
        if !self.finished {
            self.after_pump();
        }
    }

    /// Runs runnable steps.
    fn pump(&mut self) {
        if self.finished {
            return;
        }
        let mut host = GuiHost {
            notebook: Rc::clone(&self.notebook),
            state: Rc::clone(&self.state),
            sidebar: Rc::clone(&self.sidebar),
            monitoring: Rc::clone(&self.monitoring),
            runtime: Rc::clone(&self.runtime),
            session_id: self.session_id,
            engine: Rc::clone(&self.engine),
            log_writer: Rc::clone(&self.log_writer),
            resume: Rc::new({
                let this = self.this.clone();
                move || {
                    if let Some(runner) = this.upgrade() {
                        runner.borrow_mut().drive();
                    }
                }
            }),
        };
        {
            let mut engine = self.engine.borrow_mut();
            engine.pump(&mut host);
            if engine.is_finished() {
                let error = engine.error().map(str::to_string);
                self.finished = true;
                drop(engine);
                self.cleanup(error);
            }
        }
    }

    /// Arms the timer for the step the script is waiting on.
    fn after_pump(&mut self) {
        let (kind, delay_ms) = {
            let engine = self.engine.borrow();
            let kind = engine.pending();
            let delay_ms = match engine.current_step() {
                Some(ScriptStep::Wait(ms)) => u64::from(*ms),
                Some(
                    ScriptStep::Expect { timeout_ms, .. }
                    | ScriptStep::Capture { timeout_ms, .. }
                    | ScriptStep::ReadLine { timeout_ms, .. },
                ) => u64::from(timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS)),
                _ => 0,
            };
            (kind, delay_ms)
        };

        match kind {
            PendingKind::Wait
            | PendingKind::Expect
            | PendingKind::Capture
            | PendingKind::ReadLine => {
                let this = self.this.clone();
                let source =
                    glib::timeout_add_local_once(Duration::from_millis(delay_ms), move || {
                        if let Some(runner) = this.upgrade() {
                            let mut runner = runner.borrow_mut();
                            if !runner.finished {
                                runner.engine.borrow_mut().on_timer();
                                runner.drive();
                            }
                        }
                    });
                self.timer = Some(source);
            }
            _ => {}
        }
    }

    /// Runs when the script finishes (success or error).
    fn cleanup(&mut self, error: Option<String>) {
        if self.cleaned {
            return;
        }
        self.cleaned = true;
        let elapsed = self.started.elapsed();
        if let Some(timer) = self.timer.take() {
            timer.remove();
        }
        // The `Weak` is taken out before the mutable borrow: holding the
        // `borrow()` guard across `borrow_mut()` panics, and a panic inside a
        // GTK signal handler aborts the process.
        // Identity is compared on the pointer, and the borrow is released
        // before the removal: holding one guard across the other panics, and a
        // panic inside a GTK callback aborts the process.
        ACTIVE_SCRIPTS.with(|cell| {
            let mine = cell
                .borrow()
                .get(&self.session_id)
                .is_some_and(|runner| Weak::ptr_eq(&Rc::downgrade(runner), &self.this));
            if mine {
                cell.borrow_mut().remove(&self.session_id);
            }
        });
        let stopped = self.stopped_by_user;
        match error.as_deref() {
            Some(error) if !stopped => {
                tracing::warn!(
                    session = %self.session_id,
                    connection = %self.connection_id,
                    elapsed_ms = elapsed.as_millis(),
                    error,
                    "script finished with error"
                );
                crate::toast::show_error_toast_on_active_window(&crate::i18n::i18n_f(
                    "Script error: {}",
                    &[error],
                ));
            }
            _ => {
                tracing::info!(
                    session = %self.session_id,
                    connection = %self.connection_id,
                    elapsed_ms = elapsed.as_millis(),
                    stopped,
                    "script finished"
                );
            }
        }
        let outcome = if stopped {
            ScriptOutcome::Stopped
        } else {
            match error {
                Some(error) => ScriptOutcome::Failed(error),
                None => ScriptOutcome::Finished,
            }
        };
        if let Some(on_finish) = self.on_finish.take() {
            on_finish(outcome);
        }
    }
}

// =============================================================================
// Helpers
// =============================================================================

/// Strips ANSI escape sequences (used for prompt guessing).
fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_escape = false;
    for c in input.chars() {
        if in_escape {
            if ('@'..='~').contains(&c) {
                in_escape = false;
            }
            continue;
        }
        if c == '\x1b' {
            in_escape = true;
        } else {
            out.push(c);
        }
    }
    out
}

/// Returns the main window as a widget for dialog parenting.
fn parent_widget(runtime: &ScriptRuntime) -> Option<gtk4::Widget> {
    runtime
        .main_window
        .upgrade()
        .map(|w| w.upcast::<gtk4::Widget>())
}

/// Resolves a connection name or UUID to a connection id.
fn resolve_connection_id(state: &SharedAppState, selector: &str) -> Option<Uuid> {
    if let Ok(id) = Uuid::parse_str(selector) {
        return Some(id);
    }
    let Ok(state_ref) = state.try_borrow() else {
        return None;
    };
    state_ref
        .list_connections()
        .into_iter()
        .find(|c| c.name == selector)
        .map(|c| c.id)
}

/// Resolves a session name or UUID to a session id.
fn session_id_from_ref(notebook: &SharedNotebook, selector: &str) -> Option<Uuid> {
    if let Ok(id) = Uuid::parse_str(selector) {
        return Some(id);
    }
    notebook
        .get_all_sessions()
        .into_iter()
        .find(|s| s.name == selector)
        .map(|s| s.id)
}

/// Focuses a session: its pane if it sits in a split, otherwise its tab.
fn focus_session(notebook: &SharedNotebook, bridges: &SessionSplitBridges, session_id: Uuid) {
    if let Some(bridge) = bridges.borrow().get(&session_id) {
        for pane in bridge.pane_ids() {
            if bridge.get_pane_session(pane) == Some(session_id) {
                bridge.set_focused_pane(Some(pane));
                return;
            }
        }
    }
    notebook.switch_to_tab(session_id);
}

/// Activates a window action by name (split/unsplit/detach/pop).
fn activate_window_action(
    main_window: &glib::WeakRef<adw::ApplicationWindow>,
    action: &str,
    parameter: Option<&glib::Variant>,
) {
    if let Some(window) = main_window.upgrade() {
        gtk4::prelude::ActionGroupExt::activate_action(&window, action, parameter);
    } else {
        tracing::warn!(action, "no main window to activate action on");
    }
}

/// Runs `sh -c command` with stdout captured and sensitive env scrubbed.
async fn run_sh(command: &str, timeout_ms: Option<u32>) -> Result<(i32, String), String> {
    use std::process::Stdio;
    use tokio::process::Command;

    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // A timeout drops the `output()` future; without this the child keeps
    // running after the script has already reported it as timed out.
    cmd.kill_on_drop(true);
    for var in SCRUBBED_ENV_VARS {
        cmd.env_remove(var);
    }

    // The engine always supplies a timeout; the fallback keeps this function
    // bounded on its own, so no caller can reintroduce an endless wait.
    let ms = timeout_ms.unwrap_or(RUN_TIMEOUT_CEILING_MS);
    let output: std::process::Output =
        match tokio::time::timeout(Duration::from_millis(u64::from(ms)), cmd.output()).await {
            Ok(out) => out.map_err(|e| format!("Command failed: {e}"))?,
            Err(_) => return Err(format!("Command timed out after {ms}ms")),
        };

    let code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    Ok((code, stdout))
}

/// Shows the script `ask` dialog.
fn show_ask_dialog(
    parent: Option<&gtk4::Widget>,
    prompt: &str,
    echo: bool,
    callback: impl FnOnce(Option<String>) + 'static,
) {
    use adw::prelude::*;
    use gtk4::prelude::*;

    let dialog = adw::AlertDialog::new(Some(prompt), None);

    // `echo` is true for a normal, visible answer and false for `ask ... hide`.
    // The masked branch also turns off the suggestion popover, which would
    // otherwise offer completions of a value the user cannot see.
    let entry = if echo {
        gtk4::Entry::new()
    } else {
        gtk4::Entry::builder()
            .visibility(false)
            .input_purpose(gtk4::InputPurpose::Password)
            .build()
    };
    entry.set_activates_default(true);
    dialog.set_extra_child(Some(&entry));

    dialog.add_response("cancel", &crate::i18n::i18n("Cancel"));
    dialog.add_response("ok", &crate::i18n::i18n("OK"));
    dialog.set_default_response(Some("ok"));
    dialog.set_close_response("cancel");

    let callback = std::cell::RefCell::new(Some(callback));
    dialog.connect_response(None, move |_, response| {
        if let Some(callback) = callback.borrow_mut().take() {
            if response == "ok" {
                let value = entry.text().to_string();
                callback(Some(value));
            } else {
                callback(None);
            }
        }
    });

    if let Some(parent) = parent {
        dialog.present(Some(parent));
    } else {
        dialog.present(None::<&gtk4::Widget>);
    }
}

/// Shows the script `msg`/`message` dialog.
fn show_message_dialog(parent: Option<&gtk4::Widget>, text: &str, ok_button: bool) {
    use adw::prelude::*;

    if text.is_empty() {
        return;
    }
    let dialog = adw::AlertDialog::new(None, Some(text));
    if ok_button {
        dialog.add_response("ok", &crate::i18n::i18n("OK"));
        dialog.set_default_response(Some("ok"));
        dialog.set_close_response("ok");
    }
    if let Some(parent) = parent {
        dialog.present(Some(parent));
    } else {
        dialog.present(None::<&gtk4::Widget>);
    }
}

/// Starts the auto-run scripts configured on a connection's automation.
pub fn maybe_run_auto_scripts(
    state: SharedAppState,
    notebook: SharedNotebook,
    sidebar: SharedSidebar,
    monitoring: SharedMonitoring,
    connection_id: Uuid,
    session_id: Uuid,
) {
    // Deferred by one main-loop turn. The session is created from inside the
    // connection launch, which holds `state.borrow()` for its whole body, so
    // reading the connection here with `try_borrow` would fail and silently
    // skip the script — which is exactly how auto-run looked like a no-op.
    glib::idle_add_local_once(move || {
        start_auto_scripts(
            &state,
            &notebook,
            &sidebar,
            &monitoring,
            connection_id,
            session_id,
            5,
        );
    });
}

/// Starts the auto-run scripts of one connection on one session.
fn start_auto_scripts(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    monitoring: &SharedMonitoring,
    connection_id: Uuid,
    session_id: Uuid,
    retries_left: u8,
) {
    let Ok(state_ref) = state.try_borrow() else {
        // The launch path can still hold the state borrow; a short retry beats
        // silently dropping the script the user attached.
        if retries_left > 0 {
            let state = Rc::clone(state);
            let notebook = Rc::clone(notebook);
            let sidebar = Rc::clone(sidebar);
            let monitoring = Rc::clone(monitoring);
            glib::timeout_add_local_once(Duration::from_millis(150), move || {
                start_auto_scripts(
                    &state,
                    &notebook,
                    &sidebar,
                    &monitoring,
                    connection_id,
                    session_id,
                    retries_left - 1,
                );
            });
        } else {
            tracing::warn!("state busy while starting auto-run scripts; skipped");
        }
        return;
    };
    let Some(conn) = state_ref.get_connection(connection_id) else {
        return;
    };
    let resolved = crate::window::protocols::resolve_automation_for_connection(state, &conn);
    drop(state_ref);

    if resolved.auto_scripts.is_empty() {
        return;
    }
    let Some(runtime) = script_runtime() else {
        tracing::error!("script runtime is not ready; auto-run scripts skipped");
        return;
    };

    let manager =
        rustconn_core::script::ScriptManager::new(state.borrow().config_manager().clone());
    for id in &resolved.auto_scripts {
        // A failure to start is reported, not only logged: the user attached
        // the script and would otherwise see the session open and nothing
        // happen, with the reason buried in a debug log.
        match manager.load_script(id) {
            Ok(source) => {
                if let Err(e) = ScriptRunner::start_for_session(
                    &source,
                    session_id,
                    connection_id,
                    Rc::clone(state),
                    Rc::clone(notebook),
                    Rc::clone(sidebar),
                    Rc::clone(monitoring),
                    Rc::clone(&runtime),
                    None,
                ) {
                    tracing::error!(script = %id, error = %e, "auto-run script failed to start");
                    crate::toast::show_error_toast_on_active_window(&crate::i18n::i18n_f(
                        "Could not start script “{}”: {}",
                        &[id, &e],
                    ));
                }
            }
            Err(e) => {
                tracing::error!(script = %id, error = %e, "auto-run script not found or unreadable");
                crate::toast::show_error_toast_on_active_window(&crate::i18n::i18n_f(
                    "Auto-run script “{}” could not be loaded: {}",
                    &[id, &e.to_string()],
                ));
            }
        }
    }
}

/// Cancels any active script on a session (tab close).
pub fn cancel_script_for_session(session_id: Uuid) {
    stop_script_for_session(session_id);
}

/// Stops the script running on a session, if any.
///
/// Returns `true` when a script was running and has been stopped. The stop is
/// reported to the caller that started the script as
/// [`ScriptOutcome::Stopped`], so a Stop button can show the end state without
/// treating a deliberate cancel as an error.
pub fn stop_script_for_session(session_id: Uuid) -> bool {
    let Some(runner) = ACTIVE_SCRIPTS.with(|cell| cell.borrow().get(&session_id).cloned()) else {
        return false;
    };
    let mut runner = runner.borrow_mut();
    runner.engine.borrow_mut().cancel();
    runner.stopped_by_user = true;
    runner.finished = true;
    runner.cleanup(None);
    true
}

/// Whether a script is currently running on a session.
#[must_use]
pub fn is_script_running(session_id: Uuid) -> bool {
    ACTIVE_SCRIPTS.with(|cell| cell.borrow().contains_key(&session_id))
}
