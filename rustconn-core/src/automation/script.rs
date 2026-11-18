//! `.rsc` scripting engine — Asbru-CM / SecureCRT-style automation.
//!
//! # Why a state machine and not a blocking loop
//!
//! GTK has one main thread. A script that does `expect "password:"` cannot
//! block it or the UI freezes. So the runner is event-driven: every step
//! either completes immediately (`send`) or hands control back to the caller
//! (`wait` → timer, `expect`/`capture` → output, `ask` → dialog, `run` →
//! task completion, `start`/`select` → session start) and is resumed by a
//! callback. This module owns the parsing and the state machine; the GUI
//! (`rustconn::script_runner`) supplies the host that performs the side
//! effects.
//!
//! # The consumption cursor
//!
//! `expect`/`capture`/`readline` match against output that arrived since the
//! previous step consumed. On a match the buffer is trimmed to the end of the
//! match, so the next step sees only output that arrived after this one
//! completed.
//!
//! # Control flow
//!
//! Scripts are label-based: `:name` defines a label, `goto name` jumps, and
//! `if_goto $var /regex/ name [else]` jumps conditionally. Combined with
//! `set` a script can build counters and loops. An iteration cap prevents a
//! malformed jump loop from hanging the application.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::automation::{KeyElement, KeySequence};
use crate::variables::VariableManager;

/// A `$name` or `${name}` reference to a script variable.
static VARIABLE_REF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\$(?:\{([A-Za-z_][A-Za-z0-9_]*)\}|([A-Za-z_][A-Za-z0-9_]*))")
        .expect("static regex")
});

/// Built-ins a script may use. `CWD` lets a log path be absolute without a
/// literal OS path in the script; `ENV_*` stays out, as in `substitute`.
fn script_builtin(name: &str) -> Option<String> {
    if name == "CWD" {
        return std::env::current_dir()
            .ok()
            .map(|path| path.to_string_lossy().into_owned());
    }
    crate::variables::builtin_temporal(name)
}

/// How long a step with no explicit timeout waits before giving up.
pub const DEFAULT_TIMEOUT_MS: u32 = 10_000;

/// How long a `run` with no explicit timeout may take before it is killed.
///
/// Without a bound a stalled command (`curl` to an unreachable host) kept the
/// script waiting forever, and stopping the script did not stop the command.
/// Same ceiling as a connection's pre-connect task.
pub const RUN_TIMEOUT_CEILING_MS: u32 = 60_000;

/// Upper bound on instructions executed before a script is killed. Guard
/// against runaway `goto` loops, which would otherwise spin the GTK thread.
const MAX_STEPS_EXECUTED: u64 = 1_000_000;

/// A single instruction in a `.rsc` script.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScriptStep {
    /// Sends a key sequence to the session (text plus `{ENTER}`-style keys).
    Send(KeySequence),
    /// Pauses for `ms` milliseconds.
    Wait(u32),
    /// Waits until `pattern` matches the output (or times out).
    Expect {
        /// Regex to match against the buffered output.
        pattern: String,
        /// Timeout in milliseconds; `None` uses [`DEFAULT_TIMEOUT_MS`].
        timeout_ms: Option<u32>,
    },
    /// Waits until `pattern` matches, then stores capture group 1.
    Capture {
        /// Regex to match against the buffered output.
        pattern: String,
        /// Script variable receiving capture group 1.
        variable: String,
        /// Timeout in milliseconds; `None` uses [`DEFAULT_TIMEOUT_MS`].
        timeout_ms: Option<u32>,
    },
    /// Waits until a newline arrives, then stores the line (without `\n`).
    ReadLine {
        /// Script variable receiving the line.
        variable: String,
        /// Timeout in milliseconds; `None` uses [`DEFAULT_TIMEOUT_MS`].
        timeout_ms: Option<u32>,
    },
    /// Answers the device's pager prompt with a space while a pattern wait is
    /// pending, so devices that cannot disable paging still dump the whole
    /// configuration.
    Pager {
        /// Regex matching the pager prompt, e.g. `--More--`.
        pattern: String,
    },
    /// Guesses the current prompt (host-side) and stores it.
    GetPrompt {
        /// Script variable receiving the guessed prompt.
        variable: String,
    },
    /// Sets or clears the output log file for the running script.
    Log {
        /// `Some(path)` starts logging to `path`; `None` stops.
        path: Option<String>,
    },
    /// Prints an ANSI-coloured message on the terminal.
    Print(String),
    /// Asks the user for input (optionally masked) and stores the answer.
    Ask {
        /// Prompt text shown in the dialog.
        prompt: String,
        /// Whether the input is masked (secret).
        echo: bool,
        /// Pre-filled answer; the user can edit it before accepting.
        default: Option<String>,
        /// Script variable receiving the answer.
        variable: String,
    },
    /// Shows a message window (optionally with an OK button).
    Message {
        /// Message text; empty hides any open message window.
        text: String,
        /// Whether to show an OK button.
        ok_button: bool,
    },
    /// Starts a session, optionally in a cluster, optionally with manual login.
    StartSession {
        /// Connection name or UUID.
        selector: String,
        /// Set the manual-login flag on the started session.
        manual_login: bool,
        /// Cluster name, when starting inside a cluster.
        cluster: Option<String>,
        /// Script variable receiving the started session's UUID.
        uuid_var: Option<String>,
        /// Script variable receiving the started session's temporary UUID.
        tmp_uuid_var: Option<String>,
    },
    /// Selects an already-running session.
    SelectSession {
        /// Connection name or UUID.
        selector: String,
        /// Script variable receiving the selected session's UUID.
        uuid_var: Option<String>,
        /// Script variable receiving the selected session's temporary UUID.
        tmp_uuid_var: Option<String>,
    },
    /// Stops and closes the session named by a script variable.
    StopSession {
        /// Script variable holding the session UUID to close.
        session_var: String,
    },
    /// Splits two sessions horizontally (default) or vertically.
    Split {
        /// Script variable holding the first session UUID.
        first_var: String,
        /// Script variable holding the second session UUID.
        second_var: String,
        /// `true` = vertical split, `false` = horizontal.
        vertical: bool,
    },
    /// Removes the split layout containing the given session.
    Unsplit {
        /// Script variable holding the session UUID.
        session_var: String,
    },
    /// Moves a session out of its tab into its own window.
    ToWindow {
        /// Script variable holding the session UUID.
        session_var: String,
    },
    /// Returns a session from a split pane to its own tab.
    ToTab {
        /// Script variable holding the session UUID.
        session_var: String,
    },
    /// Runs a local command via the task executor (scrubbed environment).
    Run {
        /// Command line to execute.
        command: String,
        /// Timeout in milliseconds; `None` uses the executor default.
        timeout_ms: Option<u32>,
        /// Script variable receiving the command's stdout.
        out_var: Option<String>,
        /// Script variable receiving the command's exit code.
        code_var: Option<String>,
    },
    /// Sets a script variable.
    Set {
        /// Variable name.
        var: String,
        /// Value (after `${...}` substitution).
        value: String,
    },
    /// Jumps to a label.
    Goto {
        /// Label name.
        label: String,
    },
    /// Jumps to one of two labels depending on whether `var` matches a regex.
    IfGoto {
        /// Variable whose value is tested.
        var: String,
        /// Regex matched against the variable's value.
        pattern: String,
        /// Label to jump to on match.
        then_label: String,
        /// Label to jump to on no match (`None` = fall through).
        else_label: Option<String>,
    },
}

/// A parsed script: ordered steps plus resolved label indexes.
#[derive(Debug, Clone)]
pub struct ScriptProgram {
    pub(crate) steps: Vec<ScriptStep>,
    pub(crate) labels: HashMap<String, usize>,
}

impl ScriptProgram {
    /// The number of instructions in the program.
    #[must_use]
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// Whether the program contains no instructions.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// The instructions, in execution order.
    #[must_use]
    pub fn steps(&self) -> &[ScriptStep] {
        &self.steps
    }
}

/// Errors produced while parsing a `.rsc` script.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScriptParseError {
    /// A line could not be understood.
    #[error("line {line}: {message}")]
    InvalidSyntax {
        /// 1-based line number in the script source.
        line: usize,
        /// Human-readable explanation.
        message: String,
    },
    /// An unknown command name was used.
    #[error("line {line}: unknown command `{command}`")]
    UnknownCommand {
        /// 1-based line number in the script source.
        line: usize,
        /// The unknown command token.
        command: String,
    },
    /// A label was defined twice.
    #[error("line {line}: duplicate label `{label}`")]
    DuplicateLabel {
        /// 1-based line number in the script source.
        line: usize,
        /// The duplicated label name.
        label: String,
    },
    /// A `goto`/`if_goto` referenced a label that does not exist.
    #[error("undefined label `{label}`")]
    UndefinedLabel {
        /// The missing label name.
        label: String,
    },
    /// A required argument was missing.
    #[error("line {line}: {message}")]
    MissingArgument {
        /// 1-based line number in the script source.
        line: usize,
        /// Human-readable explanation.
        message: String,
    },
    /// A key sequence failed to parse.
    #[error("line {line}: invalid key sequence: {message}")]
    InvalidKeySequence {
        /// 1-based line number in the script source.
        line: usize,
        /// The underlying key-sequence error.
        message: String,
    },
    /// A regex failed to compile.
    #[error("line {line}: invalid regex: {message}")]
    InvalidRegex {
        /// 1-based line number in the script source.
        line: usize,
        /// The regex error.
        message: String,
    },
    /// A duration was not a positive integer.
    #[error("line {line}: invalid duration: {message}")]
    InvalidDuration {
        /// 1-based line number in the script source.
        line: usize,
        /// Human-readable explanation.
        message: String,
    },
}

/// What a suspended script is waiting on. `None` means it is runnable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingKind {
    /// No suspension; the engine can execute more steps.
    None,
    /// Waiting for a `wait` timer.
    Wait,
    /// Waiting for an `expect` pattern (or its timeout).
    Expect,
    /// Waiting for a `capture` pattern (or its timeout).
    Capture,
    /// Waiting for a `readline` newline (or its timeout).
    ReadLine,
    /// Waiting for the user to answer an `ask` dialog.
    Ask,
    /// Waiting for a `run` command to finish.
    Run,
    /// Waiting for a `start` session to be created.
    StartSession,
    /// Waiting for a `select` session to be resolved.
    SelectSession,
}

/// Host services a script needs. The GUI implements this against the live
/// notebook; tests use a recording fake.
pub trait ScriptHost {
    /// Sends a key sequence to the session the script runs on.
    fn send(&mut self, sequence: &KeySequence);

    /// Prints an ANSI-coloured message on the terminal.
    fn print_message(&mut self, text: &str);

    /// Sets or clears the script's output log file.
    ///
    /// Returns `false` when `Some(path)` could not be opened; the script then
    /// stops instead of pretending the capture was written. Clearing the log
    /// (`None`) always succeeds.
    fn set_log(&mut self, path: Option<&str>) -> bool;

    /// Guesses the current prompt of the session.
    fn get_prompt(&mut self) -> Option<String>;

    /// Shows an input dialog; the answer arrives via
    /// [`ScriptEngine::on_ask_result`]. A `default` is offered as the
    /// pre-filled, editable answer.
    fn ask(&mut self, prompt: &str, echo: bool, default: Option<&str>);

    /// Shows a message window.
    fn message(&mut self, text: &str, ok_button: bool);

    /// Starts a session; the result arrives via
    /// [`ScriptEngine::on_session_started`].
    fn start_session(&mut self, selector: &str, manual_login: bool, cluster: Option<&str>);

    /// Selects a running session; the result arrives via
    /// [`ScriptEngine::on_session_started`].
    fn select_session(&mut self, selector: &str);

    /// Stops and closes a session.
    fn stop_session(&mut self, session: &str);

    /// Splits two sessions into a shared layout.
    fn split(&mut self, first: &str, second: &str, vertical: bool);

    /// Removes the split layout containing a session.
    fn unsplit(&mut self, session: &str);

    /// Moves a session to its own window.
    fn to_window(&mut self, session: &str);

    /// Returns a session from a split pane to its own tab.
    fn to_tab(&mut self, session: &str);

    /// Runs a local command; the result arrives via
    /// [`ScriptEngine::on_run_result`]. The engine always passes a timeout —
    /// the script's own, or [`RUN_TIMEOUT_CEILING_MS`].
    fn run_command(&mut self, command: &str, timeout_ms: Option<u32>);
}

/// A running `.rsc` script.
///
/// The caller drives it: call [`ScriptEngine::pump`] with a host until
/// [`ScriptEngine::pending`] is not `None` or the script finishes, then feed
/// output (`on_output`), timers (`on_timer`) and dialog/task results back.
///
/// A variable may hold an `ask … hide` answer, so `Debug` names variables
/// without their values, and every value is zeroized when it is replaced or
/// the engine is dropped.
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent flags (masked ask, cancelled, finished, pager reply); a state enum would conflate them"
)]
pub struct ScriptEngine {
    program: ScriptProgram,
    pc: usize,
    buffer: String,
    variables: HashMap<String, String>,
    /// Compiled `expect`/`capture`/`if_goto` patterns, keyed by source. A
    /// pattern is re-tried on every output chunk and `if_goto` may run in a
    /// loop, so compiling each time would dominate the GTK thread.
    regex_cache: HashMap<String, Regex>,
    /// Variables currently holding an `ask … hide` answer. `run` and `log`
    /// refuse them: a command line is readable by every local user in the
    /// process table, and a log path becomes a file name and a terminal
    /// message.
    hidden_vars: std::collections::HashSet<String>,
    /// Whether the pending `ask` masks its answer.
    ask_hidden: bool,
    pending: PendingKind,
    canceled: bool,
    finished: bool,
    error: Option<String>,
    steps_executed: u64,

    // Pending-step targets.
    capture_var: Option<String>,
    readline_var: Option<String>,
    ask_var: Option<String>,
    run_out_var: Option<String>,
    run_code_var: Option<String>,
    session_uuid_var: Option<String>,
    session_tmp_uuid_var: Option<String>,
    expect_pattern: Option<String>,
    /// Regex of the device's pager prompt, set by `pager`.
    pager_pattern: Option<String>,
    /// A pager prompt was answered; `pump` must send a space before the
    /// suspended step runs again.
    pager_armed: bool,
    /// Unfinished last line of the log stream, held back so a pager prompt
    /// split across two chunks is still recognised.
    log_tail: String,
    /// Start of a multi-byte character cut off at the end of a chunk.
    log_partial: Vec<u8>,
}

impl ScriptEngine {
    /// Parses a `.rsc` source and builds an engine for it.
    ///
    /// # Errors
    ///
    /// Returns the first parse error encountered.
    pub fn parse(source: &str) -> Result<Self, ScriptParseError> {
        let program = parse_program(source)?;
        Ok(Self::new(program))
    }

    /// Creates an engine from an already-parsed program.
    #[must_use]
    pub fn new(program: ScriptProgram) -> Self {
        Self {
            program,
            pc: 0,
            buffer: String::new(),
            variables: HashMap::new(),
            regex_cache: HashMap::new(),
            hidden_vars: std::collections::HashSet::new(),
            ask_hidden: false,
            pending: PendingKind::None,
            canceled: false,
            finished: false,
            error: None,
            steps_executed: 0,
            capture_var: None,
            readline_var: None,
            ask_var: None,
            run_out_var: None,
            run_code_var: None,
            session_uuid_var: None,
            session_tmp_uuid_var: None,
            expect_pattern: None,
            pager_pattern: None,
            pager_armed: false,
            log_tail: String::new(),
            log_partial: Vec::new(),
        }
    }

    /// Whether the script has run to completion or been cancelled.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// The terminal error, if the script failed.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// What the script is currently waiting on.
    #[must_use]
    pub const fn pending(&self) -> PendingKind {
        self.pending
    }

    /// The step currently being executed, if any.
    #[must_use]
    pub fn current_step(&self) -> Option<&ScriptStep> {
        if self.finished || self.canceled {
            None
        } else {
            self.program.steps.get(self.pc)
        }
    }

    /// A value from the script's variable map.
    #[must_use]
    pub fn variable(&self, name: &str) -> Option<&str> {
        self.variables.get(name).map(String::as_str)
    }

    /// The buffered (unconsumed) output.
    #[must_use]
    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    /// Runs steps until the script suspends on an async step or finishes.
    pub fn pump(&mut self, host: &mut dyn ScriptHost) {
        // Bounded loop: every iteration either advances `pc` or suspends.
        while !self.finished && !self.canceled && self.pending == PendingKind::None {
            if self.pager_armed {
                self.pager_armed = false;
                // The paused step runs again on the buffer with the pager text
                // removed, so it can match the real prompt or arm another page.
                host.send(&KeySequence::parse(" ").unwrap_or_default());
            }
            if self.steps_executed >= MAX_STEPS_EXECUTED {
                self.finish(Some(format!(
                    "Script exceeded the {MAX_STEPS_EXECUTED}-instruction limit (runaway loop?)"
                )));
                return;
            }
            self.steps_executed += 1;
            if self.pc >= self.program.steps.len() {
                self.finish(None);
                return;
            }
            self.execute_current(host);
        }
    }

    /// Executes exactly one step.
    #[expect(
        clippy::too_many_lines,
        reason = "long match over the many DSL verbs; splitting per verb only relocates the boilerplate"
    )]
    fn execute_current(&mut self, host: &mut dyn ScriptHost) {
        let step = self.program.steps[self.pc].clone();
        match step {
            ScriptStep::Send(sequence) => {
                let sequence = self.substitute_sequence(&sequence);
                host.send(&sequence);
                self.advance();
            }
            ScriptStep::Wait(_) => {
                self.pending = PendingKind::Wait;
            }
            ScriptStep::Expect {
                pattern,
                timeout_ms,
            } => {
                self.expect_pattern = Some(pattern.clone());
                if !self.try_satisfy_expect(&pattern) {
                    self.pending = PendingKind::Expect;
                    let _ = timeout_ms;
                }
            }
            ScriptStep::Capture {
                pattern,
                variable,
                timeout_ms,
            } => {
                self.expect_pattern = Some(pattern.clone());
                self.capture_var = Some(variable);
                if !self.try_satisfy_capture(&pattern) {
                    self.pending = PendingKind::Capture;
                    let _ = timeout_ms;
                }
            }
            ScriptStep::ReadLine {
                variable,
                timeout_ms,
            } => {
                self.readline_var = Some(variable);
                if !self.try_satisfy_readline() {
                    self.pending = PendingKind::ReadLine;
                    let _ = timeout_ms;
                }
            }
            ScriptStep::Pager { pattern } => {
                self.pager_pattern = Some(pattern);
                self.advance();
            }
            ScriptStep::GetPrompt { variable } => {
                if let Some(prompt) = host.get_prompt() {
                    self.set_var(variable, prompt);
                }
                self.advance();
            }
            ScriptStep::Log { path } => {
                let path = match path {
                    Some(path) => match self.substitute_for_log(&path) {
                        Ok(path) => Some(path),
                        Err(reason) => {
                            self.finish(Some(format!("Refusing to log: {reason}")));
                            return;
                        }
                    },
                    None => None,
                };
                if host.set_log(path.as_deref()) {
                    self.advance();
                } else {
                    let path = path.unwrap_or_default();
                    self.finish(Some(format!("Could not open the script log file: {path}")));
                }
            }
            ScriptStep::Print(text) => {
                let text = self.substitute(&text);
                host.print_message(&text);
                self.advance();
            }
            ScriptStep::Ask {
                prompt,
                echo,
                default,
                variable,
            } => {
                let prompt = self.substitute(&prompt);
                // A default built from a hidden answer would show it in clear.
                if default.as_ref().is_some_and(|d| self.mentions_hidden(d)) {
                    self.finish(Some(
                        "Refusing to offer a hidden answer as an ask default".to_string(),
                    ));
                    return;
                }
                let default = default.as_ref().map(|d| self.substitute(d));
                self.ask_var = Some(variable);
                self.ask_hidden = !echo;
                self.pending = PendingKind::Ask;
                host.ask(&prompt, echo, default.as_deref());
            }
            ScriptStep::Message { text, ok_button } => {
                let text = self.substitute(&text);
                host.message(&text, ok_button);
                self.advance();
            }
            ScriptStep::StartSession {
                selector,
                manual_login,
                cluster,
                uuid_var,
                tmp_uuid_var,
            } => {
                let selector = self.substitute(&selector);
                let cluster = cluster.as_ref().map(|c| self.substitute(c));
                self.session_uuid_var = uuid_var;
                self.session_tmp_uuid_var = tmp_uuid_var;
                self.pending = PendingKind::StartSession;
                host.start_session(&selector, manual_login, cluster.as_deref());
            }
            ScriptStep::SelectSession {
                selector,
                uuid_var,
                tmp_uuid_var,
            } => {
                let selector = self.substitute(&selector);
                self.session_uuid_var = uuid_var;
                self.session_tmp_uuid_var = tmp_uuid_var;
                self.pending = PendingKind::SelectSession;
                host.select_session(&selector);
            }
            ScriptStep::StopSession { session_var } => {
                let session = self.variable_value(&session_var);
                host.stop_session(&session);
                self.advance();
            }
            ScriptStep::Split {
                first_var,
                second_var,
                vertical,
            } => {
                let first = self.variable_value(&first_var);
                let second = self.variable_value(&second_var);
                host.split(&first, &second, vertical);
                self.advance();
            }
            ScriptStep::Unsplit { session_var } => {
                let session = self.variable_value(&session_var);
                host.unsplit(&session);
                self.advance();
            }
            ScriptStep::ToWindow { session_var } => {
                let session = self.variable_value(&session_var);
                host.to_window(&session);
                self.advance();
            }
            ScriptStep::ToTab { session_var } => {
                let session = self.variable_value(&session_var);
                host.to_tab(&session);
                self.advance();
            }
            ScriptStep::Run {
                command,
                timeout_ms,
                out_var,
                code_var,
            } => {
                // The command goes to `sh -c`, and a variable may hold text the
                // remote host printed (`capture`, `readline`). Substituting it
                // unchecked would let that host run commands on this machine.
                let command = match self.substitute_for_shell(&command) {
                    Ok(command) => command,
                    Err(reason) => {
                        self.finish(Some(format!("Refusing to run command: {reason}")));
                        return;
                    }
                };
                self.run_out_var = out_var;
                self.run_code_var = code_var;
                self.pending = PendingKind::Run;
                host.run_command(&command, Some(timeout_ms.unwrap_or(RUN_TIMEOUT_CEILING_MS)));
            }
            ScriptStep::Set { var, value } => {
                // A value built from a hidden answer is as hidden as it.
                let hidden = self.mentions_hidden(&value);
                let value = self.substitute(&value);
                self.set_var(var.clone(), value);
                if hidden {
                    self.hidden_vars.insert(var);
                }
                self.advance();
            }
            ScriptStep::Goto { label } => match self.jump(&label) {
                Some(pc) => self.pc = pc,
                None => self.finish(Some(format!("Undefined label `{label}`"))),
            },
            ScriptStep::IfGoto {
                var,
                pattern,
                then_label,
                else_label,
            } => {
                let value = self.variable_value(&var);
                let matched = self
                    .compiled(&pattern)
                    .is_some_and(|re| re.is_match(&value));
                let target = if matched {
                    Some(then_label)
                } else {
                    else_label
                };
                match target {
                    Some(label) => match self.jump(&label) {
                        Some(pc) => self.pc = pc,
                        None => self.finish(Some(format!("Undefined label `{label}`"))),
                    },
                    None => self.advance(),
                }
            }
        }
    }

    /// Handles a chunk of raw session output.
    ///
    /// Appends to the buffer, then if the current step is waiting on a
    /// pattern, tries to satisfy it. Returns `true` if the pending step was
    /// satisfied, so the caller should pump again.
    pub fn on_output(&mut self, data: &[u8]) -> bool {
        if self.canceled || self.finished {
            return false;
        }
        self.buffer.push_str(&String::from_utf8_lossy(data));
        let satisfied = match self.pending {
            PendingKind::Expect => self
                .expect_pattern
                .clone()
                .map(|p| self.try_satisfy_expect(&p))
                .unwrap_or(false),
            PendingKind::Capture => self
                .expect_pattern
                .clone()
                .map(|p| self.try_satisfy_capture(&p))
                .unwrap_or(false),
            PendingKind::ReadLine => self.try_satisfy_readline(),
            _ => false,
        };
        if satisfied {
            self.pending = PendingKind::None;
            return true;
        }
        // A pager prompt gets a space instead of failing the wait; the
        // suspended step then runs again with the pager text removed.
        if matches!(
            self.pending,
            PendingKind::Expect | PendingKind::Capture | PendingKind::ReadLine
        ) && self.answer_pager()
        {
            self.pending = PendingKind::None;
            return true;
        }
        false
    }

    /// Returns the bytes of a session-output chunk that belong in the script
    /// log: text matched by the `pager` expression is dropped. Without a
    /// `pager` the chunk passes through unchanged. The unfinished last line
    /// is held back until the next chunk or [`Self::flush_log_output`].
    pub fn filter_log_output(&mut self, data: &[u8]) -> Vec<u8> {
        let regex = self
            .pager_pattern
            .clone()
            .and_then(|pattern| self.compiled(&pattern));
        let Some(regex) = regex else {
            let mut out = self.flush_log_output();
            out.extend_from_slice(data);
            return out;
        };
        let mut bytes = std::mem::take(&mut self.log_partial);
        bytes.extend_from_slice(data);
        // A character split between two chunks must not become U+FFFD.
        if let Err(error) = std::str::from_utf8(&bytes)
            && error.error_len().is_none()
        {
            self.log_partial = bytes.split_off(error.valid_up_to());
        }
        let mut text = std::mem::take(&mut self.log_tail);
        text.push_str(&String::from_utf8_lossy(&bytes));
        let text = regex.replace_all(&text, "").into_owned();
        let keep = text.rfind('\n').map_or(0, |i| i + 1);
        self.log_tail = text[keep..].to_string();
        text.as_bytes()[..keep].to_vec()
    }

    /// Returns the held-back last line of the log stream.
    pub fn flush_log_output(&mut self) -> Vec<u8> {
        let mut out = std::mem::take(&mut self.log_tail).into_bytes();
        out.append(&mut self.log_partial);
        out
    }

    /// Removes one pager prompt from the buffer and arms a space. Returns
    /// whether a `pager` pattern is set and matched.
    fn answer_pager(&mut self) -> bool {
        let Some(pattern) = self.pager_pattern.clone() else {
            return false;
        };
        let Some(regex) = self.compiled(&pattern) else {
            return false;
        };
        let Some(range) = regex
            .find(&self.buffer)
            .map(|found| found.start()..found.end())
        else {
            return false;
        };
        self.buffer.replace_range(range, "");
        self.pager_armed = true;
        true
    }

    /// Fires when a `wait` timer expires or a pattern-wait times out.
    pub fn on_timer(&mut self) {
        if self.canceled || self.finished {
            return;
        }
        match self.pending {
            PendingKind::Wait => {
                self.pc += 1;
                self.pending = PendingKind::None;
            }
            PendingKind::Expect | PendingKind::Capture | PendingKind::ReadLine => {
                let pattern = self.expect_pattern.clone().unwrap_or_default();
                self.finish(Some(format!("Timeout waiting for: {pattern}")));
            }
            _ => {}
        }
    }

    /// Delivers an `ask` dialog result.
    pub fn on_ask_result(&mut self, value: Option<String>) {
        if self.pending != PendingKind::Ask || self.finished {
            return;
        }
        match value {
            Some(value) => {
                if let Some(var) = self.ask_var.take() {
                    let hidden = self.ask_hidden;
                    self.set_var(var.clone(), value);
                    // After `set_var`, which clears the mark for any write.
                    if hidden {
                        self.hidden_vars.insert(var);
                    }
                }
                self.pc += 1;
                self.pending = PendingKind::None;
            }
            None => self.finish(Some("Ask dialog cancelled".to_string())),
        }
    }

    /// Delivers a `run` command result.
    pub fn on_run_result(&mut self, code: i32, stdout: String) {
        if self.pending != PendingKind::Run || self.finished {
            return;
        }
        if let Some(var) = self.run_out_var.take() {
            self.set_var(var, stdout);
        }
        if let Some(var) = self.run_code_var.take() {
            self.set_var(var, code.to_string());
        }
        self.pc += 1;
        self.pending = PendingKind::None;
    }

    /// Delivers a `start`/`select` session result.
    pub fn on_session_started(&mut self, uuid: Option<String>, tmp_uuid: Option<String>) {
        if self.finished {
            return;
        }
        match self.pending {
            PendingKind::StartSession | PendingKind::SelectSession => {}
            _ => return,
        }
        if let Some(var) = self.session_uuid_var.take()
            && let Some(uuid) = uuid
        {
            self.set_var(var, uuid);
        }
        if let Some(var) = self.session_tmp_uuid_var.take()
            && let Some(tmp_uuid) = tmp_uuid
        {
            self.set_var(var, tmp_uuid);
        }
        self.pc += 1;
        self.pending = PendingKind::None;
    }

    /// Cancels the script.
    pub fn cancel(&mut self) {
        self.canceled = true;
        self.finished = true;
        self.pending = PendingKind::None;
    }

    /// Resolves a label to an instruction index.
    fn jump(&self, label: &str) -> Option<usize> {
        self.program.labels.get(label).copied()
    }

    /// Advances to the next instruction.
    fn advance(&mut self) {
        self.pc += 1;
    }

    /// Marks the script finished, optionally with an error.
    pub fn finish(&mut self, error: Option<String>) {
        self.finished = true;
        self.pending = PendingKind::None;
        self.error = error;
    }

    /// Sets a variable, zeroizing the value it replaces.
    fn set_var(&mut self, name: String, value: String) {
        // A new value is not the hidden answer any more; `on_ask_result`
        // re-marks it when it is.
        self.hidden_vars.remove(&name);
        if let Some(mut old) = self.variables.insert(name, value) {
            old.zeroize();
        }
    }

    /// Reads a variable, falling back to the empty string.
    fn variable_value(&self, name: &str) -> String {
        self.variables.get(name).cloned().unwrap_or_default()
    }

    /// Substitutes `$name` / `${name}` references using the script's variables,
    /// the date/time built-ins and `CWD`. `ENV_*` is deliberately left literal:
    /// a script must not pull an environment secret onto a command line.
    fn substitute(&self, text: &str) -> String {
        VARIABLE_REF
            .replace_all(text, |caps: &regex::Captures<'_>| {
                let name = caps.get(1).or_else(|| caps.get(2)).map(|m| m.as_str());
                match name {
                    Some(name) => self
                        .variables
                        .get(name)
                        .cloned()
                        .or_else(|| script_builtin(name))
                        .unwrap_or_else(|| caps[0].to_string()),
                    None => caps[0].to_string(),
                }
            })
            .into_owned()
    }

    /// True when `text` references a variable that holds a hidden answer.
    fn mentions_hidden(&self, text: &str) -> bool {
        VARIABLE_REF.captures_iter(text).any(|caps| {
            caps.get(1)
                .or_else(|| caps.get(2))
                .is_some_and(|m| self.hidden_vars.contains(m.as_str()))
        })
    }

    /// [`Self::substitute`] for a command line handed to `sh -c`.
    ///
    /// Every substituted value must pass the same check `TaskExecutor`
    /// applies to its variables (no shell metacharacters, newlines or
    /// control characters); the first value that fails is reported. A
    /// reference to an unknown variable is left as written, exactly like
    /// [`Self::substitute`] — it is script text, not remote input.
    fn substitute_for_shell(&self, text: &str) -> Result<String, String> {
        let mut rejected: Option<String> = None;
        let command = VARIABLE_REF
            .replace_all(text, |caps: &regex::Captures<'_>| {
                let name = caps.get(1).or_else(|| caps.get(2)).map(|m| m.as_str());
                let Some((name, value)) = name.and_then(|n| {
                    self.variables
                        .get(n)
                        .cloned()
                        .or_else(|| script_builtin(n))
                        .map(|value| (n, value))
                }) else {
                    return caps[0].to_string();
                };
                if rejected.is_none() {
                    if self.hidden_vars.contains(name) {
                        rejected = Some(format!(
                            "variable '{name}' holds a hidden answer and is never put on a command line"
                        ));
                    } else if let Err(e) = VariableManager::validate_command_value(name, &value) {
                        rejected = Some(e.to_string());
                    }
                }
                value
            })
            .into_owned();
        match rejected {
            Some(reason) => Err(reason),
            None => Ok(command),
        }
    }

    /// [`Self::substitute`] for a log path.
    ///
    /// A hidden `ask` answer is refused here for the same reason `run`
    /// refuses it: a path ends up in a file name, a terminal message and
    /// possibly a backup, so a secret must not become part of it.
    fn substitute_for_log(&self, text: &str) -> Result<String, String> {
        let mut rejected: Option<String> = None;
        let path = VARIABLE_REF
            .replace_all(text, |caps: &regex::Captures<'_>| {
                let name = caps.get(1).or_else(|| caps.get(2)).map(|m| m.as_str());
                let Some((name, value)) = name.and_then(|n| {
                    self.variables
                        .get(n)
                        .cloned()
                        .or_else(|| script_builtin(n))
                        .map(|value| (n, value))
                }) else {
                    return caps[0].to_string();
                };
                if rejected.is_none() && self.hidden_vars.contains(name) {
                    rejected = Some(format!(
                        "variable '{name}' holds a hidden answer and is never used in a log path"
                    ));
                }
                value
            })
            .into_owned();
        match rejected {
            Some(reason) => Err(reason),
            None => Ok(path),
        }
    }

    /// Substitutes `${name}` key elements with script variables. Elements that
    /// are not script variables are passed through for the host to resolve.
    fn substitute_sequence(&self, sequence: &KeySequence) -> KeySequence {
        let mut elements = Vec::with_capacity(sequence.len());
        for element in &sequence.elements {
            match element {
                KeyElement::Variable(name) => {
                    if let Some(value) = self.variables.get(name)
                        && !value.is_empty()
                    {
                        elements.push(KeyElement::Text(value.clone()));
                    }
                }
                other => elements.push(other.clone()),
            }
        }
        KeySequence::from_elements(elements)
    }

    /// Returns `pattern` compiled, from the cache when it was seen before.
    /// `None` for an invalid pattern — the parser rejects those, so this only
    /// happens for a program built by hand.
    fn compiled(&mut self, pattern: &str) -> Option<Regex> {
        if let Some(regex) = self.regex_cache.get(pattern) {
            return Some(regex.clone());
        }
        let regex = Regex::new(pattern).ok()?;
        self.regex_cache.insert(pattern.to_string(), regex.clone());
        Some(regex)
    }

    /// Tries to satisfy the current `expect` step against the buffer.
    fn try_satisfy_expect(&mut self, pattern: &str) -> bool {
        let Some(regex) = self.compiled(pattern) else {
            self.finish(Some(format!("Invalid regex: {pattern}")));
            return true;
        };
        if let Some(m) = regex.captures(&self.buffer) {
            if let Some(m) = m.get(0) {
                self.buffer.drain(..m.end());
            }
            self.advance();
            true
        } else {
            false
        }
    }

    /// Tries to satisfy the current `capture` step against the buffer.
    fn try_satisfy_capture(&mut self, pattern: &str) -> bool {
        let Some(regex) = self.compiled(pattern) else {
            self.finish(Some(format!("Invalid regex: {pattern}")));
            return true;
        };
        // Copy what is needed out of the match first: `set_var` takes `&mut
        // self`, and the captures borrow `self.buffer`.
        let Some((group, end)) = regex.captures(&self.buffer).map(|captures| {
            let group = captures.get(1).map(|g| g.as_str().to_string());
            let end = captures.get(0).map_or(0, |m| m.end());
            (group, end)
        }) else {
            return false;
        };
        if let Some(group) = group
            && let Some(variable) = self.capture_var.clone()
        {
            self.set_var(variable, group);
        }
        self.buffer.drain(..end);
        self.advance();
        true
    }

    /// Tries to satisfy the current `readline` step against the buffer.
    fn try_satisfy_readline(&mut self) -> bool {
        if let Some(pos) = self.buffer.find('\n') {
            let line = self.buffer[..pos].to_string();
            self.buffer.drain(..=pos);
            if let Some(variable) = self.readline_var.clone() {
                self.set_var(variable, line);
            }
            self.advance();
            true
        } else {
            false
        }
    }
}

impl std::fmt::Debug for ScriptEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut names: Vec<&str> = self.variables.keys().map(String::as_str).collect();
        names.sort_unstable();
        f.debug_struct("ScriptEngine")
            .field("steps", &self.program.len())
            .field("pc", &self.pc)
            .field("pending", &self.pending)
            .field("finished", &self.finished)
            .field("canceled", &self.canceled)
            .field("error", &self.error)
            .field("variables", &names)
            .field("buffered_bytes", &self.buffer.len())
            .finish_non_exhaustive()
    }
}

impl Drop for ScriptEngine {
    fn drop(&mut self) {
        for value in self.variables.values_mut() {
            value.zeroize();
        }
        self.buffer.zeroize();
    }
}

/// Parses `.rsc` source text into a [`ScriptProgram`].
///
/// # Format
///
/// One instruction per line. Blank lines and `#`/`;` comments are ignored.
/// `:name` defines a label. Arguments are whitespace-separated; an argument
/// containing spaces must be quoted with `"` or `'`.
///
/// # Errors
///
/// Returns the first parse error encountered.
pub fn parse_program(source: &str) -> Result<ScriptProgram, ScriptParseError> {
    let mut steps = Vec::new();
    let mut labels = HashMap::new();

    // First pass: gather steps and label positions.
    for (index, raw) in source.lines().enumerate() {
        let line = index + 1;
        let text = strip_comment(raw).trim();
        if text.is_empty() || text.starts_with(';') {
            continue;
        }
        if let Some(label) = text.strip_prefix(':') {
            let label = label.trim();
            if !valid_label(label) {
                return Err(ScriptParseError::InvalidSyntax {
                    line,
                    message: format!("invalid label `{label}`"),
                });
            }
            if labels.insert(label.to_string(), steps.len()).is_some() {
                return Err(ScriptParseError::DuplicateLabel {
                    line,
                    label: label.to_string(),
                });
            }
            continue;
        }
        let step = parse_line(text, line)?;
        steps.push(step);
    }

    // Second pass: validate every referenced label exists.
    for step in &steps {
        match step {
            ScriptStep::Goto { label } => {
                if !labels.contains_key(label) {
                    return Err(ScriptParseError::UndefinedLabel {
                        label: label.clone(),
                    });
                }
            }
            ScriptStep::IfGoto {
                then_label,
                else_label,
                ..
            } => {
                if !labels.contains_key(then_label) {
                    return Err(ScriptParseError::UndefinedLabel {
                        label: then_label.clone(),
                    });
                }
                if let Some(label) = else_label
                    && !labels.contains_key(label)
                {
                    return Err(ScriptParseError::UndefinedLabel {
                        label: label.clone(),
                    });
                }
            }
            _ => {}
        }
    }

    Ok(ScriptProgram { steps, labels })
}

/// Strips a `#` comment and trailing whitespace.
///
/// A `#` only starts a comment at the beginning of the line or after
/// whitespace, so `expect /#/` and `expect "#"` are patterns, not comments.
fn strip_comment(line: &str) -> &str {
    let mut previous_is_space = true;
    for (i, c) in line.char_indices() {
        if c == '#' && previous_is_space {
            return &line[..i];
        }
        previous_is_space = c.is_whitespace();
    }
    line
}

/// Removes one pair of surrounding quotes, if the whole string is quoted.
///
/// `send`, `print`, `msg` and `log` take their argument as the rest of the
/// line so that spaces need no escaping; that means the quotes themselves
/// reach them and would otherwise be sent to the terminal or printed.
fn unquote(text: &str) -> String {
    let trimmed = text.trim();
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    if (first == '"' || first == '\'') && trimmed.len() >= 2 && trimmed.ends_with(first) {
        return trimmed[first.len_utf8()..trimmed.len() - first.len_utf8()].to_string();
    }
    trimmed.to_string()
}

/// Whether a string is a valid label name.
fn valid_label(label: &str) -> bool {
    !label.is_empty()
        && label
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
}

/// Splits a line into whitespace-separated tokens, respecting quotes.
fn tokenize(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted: Option<char> = None;

    for c in line.chars() {
        match quoted {
            Some(q) => {
                if c == q {
                    quoted = None;
                } else {
                    current.push(c);
                }
            }
            None => {
                if (c == '"' || c == '\'') && current.is_empty() {
                    quoted = Some(c);
                } else if c.is_whitespace() {
                    if !current.is_empty() {
                        tokens.push(std::mem::take(&mut current));
                    }
                } else {
                    current.push(c);
                }
            }
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// Parses a single instruction line.
#[expect(
    clippy::too_many_lines,
    reason = "long match over the many DSL verbs; splitting per verb only relocates the boilerplate"
)]
fn parse_line(line: &str, line_no: usize) -> Result<ScriptStep, ScriptParseError> {
    let tokens = tokenize(line);
    let Some(command) = tokens.first() else {
        return Err(ScriptParseError::InvalidSyntax {
            line: line_no,
            message: "empty instruction".to_string(),
        });
    };

    let arg = |i: usize| -> Result<String, ScriptParseError> {
        tokens
            .get(i)
            .cloned()
            .ok_or_else(|| ScriptParseError::MissingArgument {
                line: line_no,
                message: format!("missing argument for `{command}`"),
            })
    };

    let duration = |text: &str, what: &str| -> Result<Option<u32>, ScriptParseError> {
        let Ok(seconds) = text.parse::<u32>() else {
            return Err(ScriptParseError::InvalidDuration {
                line: line_no,
                message: format!("{what} must be a positive number of seconds, got `{text}`"),
            });
        };
        Ok(Some(seconds.saturating_mul(1000)))
    };

    let pattern = |text: &str| -> String {
        if text.len() >= 2 && text.starts_with('/') && text.ends_with('/') {
            text[1..text.len() - 1].to_string()
        } else {
            text.to_string()
        }
    };

    // Normalizes a variable-name argument: strips a leading `$` or `${…}`
    // sigil so `stop $tmp` and `stop tmp` both name the variable `tmp`.
    let var_name = |text: &str| -> String {
        let t = text.trim();
        if let Some(inner) = t.strip_prefix("${").and_then(|s| s.strip_suffix('}')) {
            inner.to_string()
        } else if let Some(s) = t.strip_prefix('$') {
            s.to_string()
        } else {
            t.to_string()
        }
    };

    match command.as_str() {
        "send" => {
            let rest = unquote(&line["send".len()..]);
            let sequence =
                KeySequence::parse(&rest).map_err(|e| ScriptParseError::InvalidKeySequence {
                    line: line_no,
                    message: e.to_string(),
                })?;
            Ok(ScriptStep::Send(sequence))
        }
        "wait" => {
            let raw = arg(1)?;
            let ms: u32 = raw.parse().map_err(|_| ScriptParseError::InvalidDuration {
                line: line_no,
                message: format!("wait duration must be milliseconds, got `{raw}`"),
            })?;
            Ok(ScriptStep::Wait(ms))
        }
        "expect" => {
            let pattern = pattern(&arg(1)?);
            let timeout_ms = match tokens.get(2) {
                Some(t) => duration(t, "expect timeout")?,
                None => None,
            };
            // Validate the pattern now so a typo fails at parse time.
            Regex::new(&pattern).map_err(|e| ScriptParseError::InvalidRegex {
                line: line_no,
                message: e.to_string(),
            })?;
            Ok(ScriptStep::Expect {
                pattern,
                timeout_ms,
            })
        }
        "capture" => {
            let pattern = pattern(&arg(1)?);
            let variable = var_name(&arg(2)?);
            let timeout_ms = match tokens.get(3) {
                Some(t) => duration(t, "capture timeout")?,
                None => None,
            };
            Regex::new(&pattern).map_err(|e| ScriptParseError::InvalidRegex {
                line: line_no,
                message: e.to_string(),
            })?;
            Ok(ScriptStep::Capture {
                pattern,
                variable,
                timeout_ms,
            })
        }
        "readline" => {
            let variable = var_name(&arg(1)?);
            let timeout_ms = match tokens.get(2) {
                Some(t) => duration(t, "readline timeout")?,
                None => None,
            };
            Ok(ScriptStep::ReadLine {
                variable,
                timeout_ms,
            })
        }
        "pager" => {
            let pattern = pattern(&arg(1)?);
            Regex::new(&pattern).map_err(|e| ScriptParseError::InvalidRegex {
                line: line_no,
                message: e.to_string(),
            })?;
            Ok(ScriptStep::Pager { pattern })
        }
        "getprompt" => {
            let variable = var_name(&arg(1)?);
            Ok(ScriptStep::GetPrompt { variable })
        }
        "log" => {
            let rest = unquote(&line["log".len()..]);
            let path = if rest.is_empty() { None } else { Some(rest) };
            Ok(ScriptStep::Log { path })
        }
        "print" => {
            let rest = unquote(&line["print".len()..]);
            Ok(ScriptStep::Print(rest))
        }
        "ask" => {
            let prompt = arg(1)?;
            let mut echo = true;
            let mut default = None;
            let mut variable = None;
            let mut i = 2;
            while let Some(token) = tokens.get(i) {
                match token.as_str() {
                    "hide" => echo = false,
                    "default" => {
                        let value = tokens.get(i + 1).cloned().ok_or_else(|| {
                            ScriptParseError::MissingArgument {
                                line: line_no,
                                message: "default needs a value".to_string(),
                            }
                        })?;
                        default = Some(value);
                        i += 1;
                    }
                    _ => variable = Some(var_name(token)),
                }
                i += 1;
            }
            // `ask "Prompt"` derives the variable from the prompt text.
            let variable = variable.unwrap_or_else(|| var_name(&prompt));
            if variable.is_empty() {
                return Err(ScriptParseError::MissingArgument {
                    line: line_no,
                    message: "ask needs a variable".to_string(),
                });
            }
            Ok(ScriptStep::Ask {
                prompt,
                echo,
                default,
                variable,
            })
        }
        "msg" | "message" => {
            let rest = line[command.len()..].trim();
            let ok_button = tokens.iter().any(|t| t == "ok");
            let text = if ok_button && rest.ends_with(" ok") {
                unquote(rest[..rest.len() - 3].trim_end())
            } else {
                unquote(rest)
            };
            Ok(ScriptStep::Message { text, ok_button })
        }
        "start" => {
            let selector = arg(1)?;
            let manual_login = tokens.iter().any(|t| t == "manual");
            let cluster = token_option(&tokens, "cluster");
            let (uuid_var, tmp_uuid_var) = result_vars(&tokens)?;
            Ok(ScriptStep::StartSession {
                selector,
                manual_login,
                cluster,
                uuid_var,
                tmp_uuid_var,
            })
        }
        "select" => {
            let selector = arg(1)?;
            let (uuid_var, tmp_uuid_var) = result_vars(&tokens)?;
            Ok(ScriptStep::SelectSession {
                selector,
                uuid_var,
                tmp_uuid_var,
            })
        }
        "stop" => {
            let session_var = var_name(&arg(1)?);
            Ok(ScriptStep::StopSession { session_var })
        }
        "split" => {
            let first_raw = arg(1)?;
            let second_raw = arg(2)?;
            let vertical = tokens.iter().any(|t| t == "vertical");
            let (first, second) = split_pair(&first_raw, &second_raw);
            let first = var_name(&first);
            let second = var_name(&second);
            Ok(ScriptStep::Split {
                first_var: first,
                second_var: second,
                vertical,
            })
        }
        "unsplit" => {
            let session_var = var_name(&arg(1)?);
            Ok(ScriptStep::Unsplit { session_var })
        }
        "towindow" => {
            let session_var = var_name(&arg(1)?);
            Ok(ScriptStep::ToWindow { session_var })
        }
        "totab" => {
            let session_var = var_name(&arg(1)?);
            Ok(ScriptStep::ToTab { session_var })
        }
        "run" => {
            let command = arg(1)?;
            let mut timeout_ms = None;
            let mut out_var = None;
            let mut code_var = None;
            for token in tokens.iter().skip(2) {
                if let Some(value) = token.strip_prefix("out=") {
                    out_var = Some(var_name(value));
                } else if let Some(value) = token.strip_prefix("code=") {
                    code_var = Some(var_name(value));
                } else {
                    timeout_ms = duration(token, "run timeout")?;
                }
            }
            Ok(ScriptStep::Run {
                command,
                timeout_ms,
                out_var,
                code_var,
            })
        }
        "set" => {
            let var = var_name(&arg(1)?);
            let rest = line[line.find("set").map_or(0, |i| i + 3)..].trim();
            let value = rest
                .char_indices()
                .find(|(_, c)| c.is_whitespace())
                .map(|(i, _)| rest[i..].trim_start().to_string())
                .unwrap_or_default();
            Ok(ScriptStep::Set { var, value })
        }
        "goto" => {
            let label = arg(1)?;
            Ok(ScriptStep::Goto { label })
        }
        "if_goto" => {
            let var = var_name(&arg(1)?);
            let pattern = pattern(&arg(2)?);
            let then_label = arg(3)?;
            Regex::new(&pattern).map_err(|e| ScriptParseError::InvalidRegex {
                line: line_no,
                message: e.to_string(),
            })?;
            let else_label = match tokens.get(4) {
                Some(t) if t == "else" => tokens.get(5).cloned(),
                Some(t) => Some(t.clone()),
                None => None,
            };
            Ok(ScriptStep::IfGoto {
                var,
                pattern,
                then_label,
                else_label,
            })
        }
        other => Err(ScriptParseError::UnknownCommand {
            line: line_no,
            command: other.to_string(),
        }),
    }
}

/// Reads a `key=value` token option.
fn token_option(tokens: &[String], key: &str) -> Option<String> {
    tokens
        .iter()
        .find_map(|t| t.strip_prefix(&format!("{key}=")).map(str::to_string))
}

/// Reads an optional `-> v1,v2` result-variable pair.
fn result_vars(tokens: &[String]) -> Result<(Option<String>, Option<String>), ScriptParseError> {
    let pair = tokens.iter().position(|t| t == "->").map(|i| {
        let mut rest = tokens[i + 1..].join(",");
        if rest.is_empty() {
            rest = tokens.get(i + 1).cloned().unwrap_or_default();
        }
        rest
    });
    match pair {
        Some(pair) => {
            let mut parts = pair.split(',');
            let first = parts.next().filter(|s| !s.is_empty()).map(str::to_string);
            let second = parts.next().filter(|s| !s.is_empty()).map(str::to_string);
            Ok((
                first.map(|v| v.trim_start_matches('$').to_string()),
                second.map(|v| v.trim_start_matches('$').to_string()),
            ))
        }
        None => Ok((None, None)),
    }
}

/// Splits `first` (possibly `a,b`) and `second` into two session references.
fn split_pair(first: &str, second: &str) -> (String, String) {
    if let Some((a, b)) = first.split_once(',') {
        (a.trim().to_string(), b.trim().to_string())
    } else {
        (first.to_string(), second.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal recording host for engine tests.
    #[derive(Debug, Default)]
    struct FakeHost {
        events: Vec<String>,
        prompt: Option<String>,
        log_fails: bool,
    }

    impl ScriptHost for FakeHost {
        fn send(&mut self, sequence: &KeySequence) {
            self.events.push(format!("send:{sequence}"));
        }
        fn print_message(&mut self, text: &str) {
            self.events.push(format!("print:{text}"));
        }
        fn set_log(&mut self, path: Option<&str>) -> bool {
            self.events.push(format!("log:{}", path.unwrap_or("")));
            path.is_none() || !self.log_fails
        }
        fn get_prompt(&mut self) -> Option<String> {
            self.prompt.clone()
        }
        fn ask(&mut self, prompt: &str, echo: bool, default: Option<&str>) {
            self.events
                .push(format!("ask:{prompt}:{echo}:{}", default.unwrap_or("")));
        }
        fn message(&mut self, text: &str, ok_button: bool) {
            self.events.push(format!("msg:{text}:{ok_button}"));
        }
        fn start_session(&mut self, selector: &str, manual_login: bool, cluster: Option<&str>) {
            self.events.push(format!(
                "start:{selector}:{manual_login}:{}",
                cluster.unwrap_or("")
            ));
        }
        fn select_session(&mut self, selector: &str) {
            self.events.push(format!("select:{selector}"));
        }
        fn stop_session(&mut self, session: &str) {
            self.events.push(format!("stop:{session}"));
        }
        fn split(&mut self, first: &str, second: &str, vertical: bool) {
            self.events
                .push(format!("split:{first}:{second}:{vertical}"));
        }
        fn unsplit(&mut self, session: &str) {
            self.events.push(format!("unsplit:{session}"));
        }
        fn to_window(&mut self, session: &str) {
            self.events.push(format!("towindow:{session}"));
        }
        fn to_tab(&mut self, session: &str) {
            self.events.push(format!("totab:{session}"));
        }
        fn run_command(&mut self, command: &str, timeout_ms: Option<u32>) {
            self.events.push(format!("run:{command}:{timeout_ms:?}"));
        }
    }

    fn run_script(source: &str, host: &mut FakeHost) -> ScriptEngine {
        let mut engine = ScriptEngine::parse(source).expect("script should parse");
        engine.pump(host);
        engine
    }

    /// A `pager` pattern answers the device's pager prompt with a space and
    /// keeps the same `expect` waiting, so a device that cannot disable
    /// paging still reaches its final prompt.
    #[test]
    fn pager_answers_the_more_prompt_and_keeps_waiting() {
        let mut host = FakeHost::default();
        let mut engine =
            ScriptEngine::parse("pager \"---- More .*----\"\nexpect \"R1#\\s*$\" 30\n").unwrap();
        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::Expect);

        assert!(engine.on_output(b"line 1\r\n---- More ( Press 'Q' to break ) ----"));
        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::Expect);
        assert_eq!(host.events, vec!["send: "]);

        assert!(engine.on_output(b"line 2\r\n---- More ( Press 'Q' to break ) ----"));
        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::Expect);
        assert_eq!(host.events, vec!["send: ", "send: "]);

        assert!(engine.on_output(b"line 3\r\nR1# "));
        engine.pump(&mut host);
        assert!(engine.is_finished());
        assert_eq!(engine.error(), None);
    }

    #[test]
    fn parses_simple_script() {
        let program =
            parse_program("send hello{ENTER}\nwait 500\nexpect \"password:\" 5\n").unwrap();
        assert_eq!(program.len(), 3);
        assert!(matches!(program.steps()[0], ScriptStep::Send(_)));
        assert!(matches!(program.steps()[1], ScriptStep::Wait(500)));
        assert!(matches!(
            program.steps()[2],
            ScriptStep::Expect { ref pattern, timeout_ms: Some(5000) } if pattern == "password:"
        ));
    }

    #[test]
    fn send_and_wait_run_synchronously() {
        let mut host = FakeHost::default();
        let mut engine = run_script("send hello{ENTER}\nwait 200\nsend done\n", &mut host);
        // send, wait (pending), then send.
        assert_eq!(engine.pending(), PendingKind::Wait);
        engine.on_timer();
        engine.pump(&mut host);
        assert!(engine.is_finished());
        assert_eq!(host.events, vec!["send:hello{ENTER}", "send:done"]);
    }

    #[test]
    fn expect_consumes_buffer_and_advances() {
        let mut host = FakeHost::default();
        let mut engine = ScriptEngine::parse("expect \"password: \"\nsend ok{ENTER}\n").unwrap();
        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::Expect);
        assert!(!engine.on_output(b"login: user\r\n"));
        assert!(engine.on_output(b"password: "));
        engine.pump(&mut host);
        assert!(engine.is_finished());
        assert_eq!(host.events, vec!["send:ok{ENTER}"]);
        // Output before the match is consumed along with it.
        assert!(engine.buffer().is_empty());
    }

    #[test]
    fn capture_stores_group_one() {
        let mut host = FakeHost::default();
        let mut engine = ScriptEngine::parse("capture /(\\d+)/ count\nprint $count\n").unwrap();
        engine.pump(&mut host);
        engine.on_output(b"bytes: 42");
        engine.pump(&mut host);
        assert!(engine.is_finished());
        assert_eq!(engine.variable("count"), Some("42"));
        assert_eq!(host.events, vec!["print:42"]);
    }

    #[test]
    fn readline_stores_line() {
        let mut host = FakeHost::default();
        let mut engine = ScriptEngine::parse("readline line\nprint $line\n").unwrap();
        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::ReadLine);
        engine.on_output(b"first\r\n");
        engine.pump(&mut host);
        assert!(engine.is_finished());
        assert_eq!(engine.variable("line"), Some("first\r"));
    }

    #[test]
    fn expect_timeout_finishes_with_error() {
        let mut host = FakeHost::default();
        let mut engine = ScriptEngine::parse("expect \"never\" 1\n").unwrap();
        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::Expect);
        engine.on_timer();
        assert!(engine.is_finished());
        assert!(engine.error().is_some());
    }

    #[test]
    fn goto_and_if_goto_control_flow() {
        let mut host = FakeHost::default();
        let mut engine = ScriptEngine::parse(
            "set n 0\n:loop\nif_goto $n /^2$/ done\ngoto loop\n:done\nprint fin\n",
        )
        .unwrap();
        engine.pump(&mut host);
        // Never advances n, so the loop runs to the iteration cap.
        assert!(engine.is_finished());
        assert!(engine.error().is_some());
        assert!(engine.error().unwrap().contains("limit"));
    }

    #[test]
    fn goto_skips_intervening_steps() {
        let mut host = FakeHost::default();
        let mut engine =
            ScriptEngine::parse("set x 1\ngoto done\nprint skipped\n:done\nprint fin\n").unwrap();
        engine.pump(&mut host);
        assert!(engine.is_finished());
        assert_eq!(host.events, vec!["print:fin"]);
    }

    #[test]
    fn if_goto_jumps_on_match_and_falls_through_otherwise() {
        let mut host = FakeHost::default();
        let mut engine = ScriptEngine::parse(
            "set x foo\nif_goto $x /foo/ yes\nprint no\ngoto end\n:yes\nprint yes\n:end\n",
        )
        .unwrap();
        engine.pump(&mut host);
        assert!(engine.is_finished());
        assert_eq!(host.events, vec!["print:yes"]);

        let mut host = FakeHost::default();
        let mut engine = ScriptEngine::parse(
            "set x bar\nif_goto $x /foo/ yes\ngoto end\n:yes\nprint yes\n:end\nprint no\n",
        )
        .unwrap();
        engine.pump(&mut host);
        assert!(engine.is_finished());
        assert_eq!(host.events, vec!["print:no"]);
    }

    #[test]
    fn ask_suspends_and_resumes() {
        let mut host = FakeHost::default();
        let mut engine = ScriptEngine::parse("ask \"Value:\" hide val\nprint $val\n").unwrap();
        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::Ask);
        assert_eq!(host.events, vec!["ask:Value::false:"]);
        engine.on_ask_result(Some("hunter2".to_string()));
        engine.pump(&mut host);
        assert!(engine.is_finished());
        assert_eq!(host.events, vec!["ask:Value::false:", "print:hunter2"]);
    }

    #[test]
    fn ask_offers_a_substituted_default() {
        let mut host = FakeHost::default();
        let mut engine = ScriptEngine::parse(
            "set vlan 500\nask \"VLAN:\" default $vlan hide value\nprint $value\n",
        )
        .unwrap();
        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::Ask);
        assert_eq!(host.events, vec!["ask:VLAN::false:500"]);
        engine.on_ask_result(Some("600".to_string()));
        engine.pump(&mut host);
        assert!(engine.is_finished());
        assert_eq!(host.events, vec!["ask:VLAN::false:500", "print:600"]);
    }

    #[test]
    fn ask_default_without_a_variable_derives_it_from_the_prompt() {
        let mut host = FakeHost::default();
        let mut engine = ScriptEngine::parse("ask \"Port\" default \"22\"\nprint $Port\n").unwrap();
        engine.pump(&mut host);
        assert_eq!(host.events, vec!["ask:Port:true:22"]);
        engine.on_ask_result(Some("22".to_string()));
        engine.pump(&mut host);
        assert_eq!(engine.variable("Port"), Some("22"));
    }

    #[test]
    fn ask_cancel_finishes_with_error() {
        let mut host = FakeHost::default();
        let mut engine = ScriptEngine::parse("ask \"Value:\" val\n").unwrap();
        engine.pump(&mut host);
        engine.on_ask_result(None);
        assert!(engine.is_finished());
        assert!(engine.error().is_some());
    }

    #[test]
    fn start_session_uses_result_vars() {
        let mut host = FakeHost::default();
        let mut engine =
            ScriptEngine::parse("start \"My Server\" -> uuid,tmp\nstop $tmp\n").unwrap();
        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::StartSession);
        assert_eq!(host.events, vec!["start:My Server:false:"]);
        engine.on_session_started(Some("u1".to_string()), Some("t1".to_string()));
        engine.pump(&mut host);
        assert!(engine.is_finished());
        assert_eq!(engine.variable("uuid"), Some("u1"));
        assert_eq!(engine.variable("tmp"), Some("t1"));
        assert_eq!(host.events, vec!["start:My Server:false:", "stop:t1"]);
    }

    #[test]
    fn pending_uses_verbatim_key_sequence() {
        let mut host = FakeHost::default();
        let engine = run_script("send ls -la {ENTER}\n", &mut host);
        assert!(engine.is_finished());
        assert!(host.events[0].starts_with("send:ls -la "));
    }

    #[test]
    fn unknown_command_is_an_error() {
        let err = ScriptEngine::parse("frobnicate x\n").unwrap_err();
        assert!(matches!(err, ScriptParseError::UnknownCommand { .. }));
    }

    #[test]
    fn duplicate_label_is_an_error() {
        let err = ScriptEngine::parse(":a\n:a\n").unwrap_err();
        assert!(matches!(err, ScriptParseError::DuplicateLabel { .. }));
    }

    #[test]
    fn undefined_label_is_an_error() {
        let err = ScriptEngine::parse("goto nowhere\n").unwrap_err();
        assert!(matches!(err, ScriptParseError::UndefinedLabel { .. }));
    }

    #[test]
    fn invalid_regex_is_an_error() {
        let err = ScriptEngine::parse("expect \"([unclosed\"\n").unwrap_err();
        assert!(matches!(err, ScriptParseError::InvalidRegex { .. }));
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let program = parse_program("# heading\n; another comment\n\nsend hi\n   \n").unwrap();
        assert_eq!(program.len(), 1);
    }

    #[test]
    fn openwrt_uptime_example_captures_uptime() {
        let mut host = FakeHost::default();
        let mut engine = ScriptEngine::parse(
            "expect \"#\" 15\nsend \"uptime{ENTER}\"\ncapture /up\\s+([^,]+)/ up 10\nprint \"Uptime: $up\"\n",
        )
        .expect("openwrt uptime example parses");

        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::Expect);

        // The OpenWrt root prompt, already on screen when the script starts.
        assert!(engine.on_output(b"root@OpenWrt:~# "));
        engine.pump(&mut host);
        assert_eq!(host.events, vec!["send:uptime{ENTER}"]);
        assert_eq!(engine.pending(), PendingKind::Capture);

        engine.on_output(b" 14:30:00 up 2 days,  3:21,  load average: 0.00, 0.01, 0.05");
        engine.pump(&mut host);

        assert!(engine.is_finished());
        assert_eq!(engine.variable("up"), Some("2 days"));
        assert_eq!(host.events[1], "print:Uptime: 2 days");
    }

    #[test]
    fn openwrt_uptime_ignores_echoed_command() {
        let mut host = FakeHost::default();
        let mut engine = ScriptEngine::parse(
            "expect \"#\" 15\nsend \"uptime{ENTER}\"\ncapture /up\\s+([^,]+)/ up 10\nprint \"Uptime: $up\"\n",
        )
        .expect("openwrt uptime example parses");

        engine.pump(&mut host);
        engine.on_output(b"root@OpenWrtMx4300:~# ");
        engine.pump(&mut host);

        // The shell echoes the command before printing its result. `up\s`
        // cannot match "uptime", so the capture still waits.
        assert!(!engine.on_output(b"uptime\r\n"));
        assert_eq!(engine.pending(), PendingKind::Capture);

        assert!(engine.on_output(b" 14:30:00 up 2 days,  3:21,  load average: 0.01, 0.01, 0.00"));
        engine.pump(&mut host);

        assert!(engine.is_finished());
        assert_eq!(engine.variable("up"), Some("2 days"));
    }

    #[test]
    fn variable_substitution_in_run_and_print() {
        let mut host = FakeHost::default();
        let mut engine = ScriptEngine::parse(
            "set user alice\nprint Hello $user\nrun \"echo $user\" 5 out=o code=c\n",
        )
        .unwrap();
        engine.pump(&mut host);
        assert_eq!(
            host.events,
            vec!["print:Hello alice", "run:echo alice:Some(5000)"]
        );
        engine.on_run_result(0, "alice\n".to_string());
        engine.pump(&mut host);
        assert!(engine.is_finished());
        assert_eq!(engine.variable("o"), Some("alice\n"));
        assert_eq!(engine.variable("c"), Some("0"));
    }

    /// Text the remote host printed must never become a local shell command:
    /// a captured value carrying a metacharacter stops the script before
    /// `run` reaches the host.
    #[test]
    fn run_refuses_a_captured_value_with_shell_metacharacters() {
        let mut host = FakeHost::default();
        let mut engine =
            ScriptEngine::parse("capture /name=(\\S+)/ name\nrun \"echo ${name}\"\n").unwrap();
        engine.pump(&mut host);
        engine.on_output(b"name=x;touch\t/tmp/pwned ");
        engine.pump(&mut host);

        assert!(engine.is_finished());
        assert!(
            engine
                .error()
                .is_some_and(|e| e.contains("Refusing to run command")),
            "{:?}",
            engine.error()
        );
        assert!(host.events.iter().all(|e| !e.starts_with("run:")));
    }

    /// Each class the shared validator rejects is rejected here too, and a
    /// plain value still runs.
    #[test]
    fn run_validates_every_substituted_value() {
        for bad in ["a|b", "a&b", "`id`", "$(id)", "a>b", "a\nb", "a\u{7}b"] {
            let mut host = FakeHost::default();
            let mut engine = ScriptEngine::parse("run \"echo ${v}\"\n").unwrap();
            engine.variables.insert("v".to_string(), bad.to_string());
            engine.pump(&mut host);
            assert!(engine.error().is_some(), "{bad:?} must be rejected");
            assert!(host.events.is_empty(), "{bad:?} reached the host");
        }

        let mut host = FakeHost::default();
        let mut engine = ScriptEngine::parse("run \"ping -c1 ${v}\"\n").unwrap();
        engine
            .variables
            .insert("v".to_string(), "db-01.example.com".to_string());
        engine.pump(&mut host);
        // No `<sec>` given: the ceiling applies, never an unbounded wait.
        assert_eq!(
            host.events,
            vec![format!(
                "run:ping -c1 db-01.example.com:Some({RUN_TIMEOUT_CEILING_MS})"
            )]
        );
    }

    /// M-PUBLIC-DEBUG: an `ask … hide` answer must not reach a `{:?}` — the
    /// engine sits inside GUI structs that are logged with `Debug`.
    #[test]
    fn debug_names_variables_without_their_values() {
        let mut host = FakeHost::default();
        let mut engine = ScriptEngine::parse("ask \"Password:\" hide pw\nwait 1\n").unwrap();
        engine.pump(&mut host);
        engine.on_ask_result(Some("hunter2-secret".to_string()));
        engine.pump(&mut host);

        let debug = format!("{engine:?}");
        assert!(debug.contains("pw"), "{debug}");
        assert!(!debug.contains("hunter2-secret"), "{debug}");
        // The value is still usable by the script itself.
        assert_eq!(engine.variable("pw"), Some("hunter2-secret"));
    }

    /// An `ask … hide` answer reaches `send` (that is what it is for) but
    /// never a `run` command line, which other local users can read in the
    /// process table — even when it has no shell metacharacter in it.
    #[test]
    fn a_hidden_answer_is_sent_but_never_run() {
        let mut host = FakeHost::default();
        let mut engine = ScriptEngine::parse(
            "ask \"Password:\" hide pw\nsend \"${pw}{ENTER}\"\nrun \"echo ${pw}\"\n",
        )
        .unwrap();
        engine.pump(&mut host);
        engine.on_ask_result(Some("hunter2".to_string()));
        engine.pump(&mut host);

        assert!(host.events.iter().any(|e| e.starts_with("send:hunter2")));
        assert!(host.events.iter().all(|e| !e.starts_with("run:")));
        assert!(
            engine.error().is_some_and(|e| e.contains("hidden answer")),
            "{:?}",
            engine.error()
        );

        // Overwriting the variable with an ordinary value lifts the mark, and
        // a visible answer was never marked.
        let mut host = FakeHost::default();
        let mut engine = ScriptEngine::parse(
            "ask \"Password:\" hide v\nset v plain\nask \"Host:\" h\nrun \"echo ${v} ${h}\"\n",
        )
        .unwrap();
        engine.pump(&mut host);
        engine.on_ask_result(Some("secret".to_string()));
        engine.pump(&mut host);
        engine.on_ask_result(Some("db01".to_string()));
        engine.pump(&mut host);
        assert!(engine.error().is_none(), "{:?}", engine.error());
        assert!(
            host.events
                .iter()
                .any(|e| e.starts_with("run:echo plain db01"))
        );
    }

    /// `send` and `print` are not shell contexts, so the guard does not apply
    /// to them: a value with `;` still reaches the terminal unchanged.
    #[test]
    fn send_and_print_are_not_shell_checked() {
        let mut host = FakeHost::default();
        let mut engine = ScriptEngine::parse("print $v\n").unwrap();
        engine.variables.insert("v".to_string(), "a;b".to_string());
        engine.pump(&mut host);
        assert!(engine.error().is_none());
        assert_eq!(host.events, vec!["print:a;b"]);
    }

    /// A `log` path may be built from script variables and the clock, which is
    /// what lets a backup script name its file after the host and the date.
    #[test]
    fn log_path_substitutes_variables_and_the_clock() {
        let mut host = FakeHost::default();
        let mut engine =
            ScriptEngine::parse("set name backup\nlog \"${name}-${DATE_Y}.log\"\n").unwrap();
        engine.pump(&mut host);

        let path = host
            .events
            .iter()
            .find_map(|event| event.strip_prefix("log:"))
            .expect("the log step runs");
        let year = path
            .strip_prefix("backup-")
            .and_then(|rest| rest.strip_suffix(".log"))
            .expect("both references resolve");
        assert_eq!(year.len(), 4);
        assert!(year.chars().all(|c| c.is_ascii_digit()), "{year}");
    }

    /// `${CWD}` makes the default backup path absolute without putting an OS
    /// path in the script, so the question can show where the file will land.
    #[test]
    fn cwd_builtin_resolves_to_the_process_directory() {
        let mut host = FakeHost::default();
        let mut engine = ScriptEngine::parse("log \"${CWD}/backup.log\"\n").unwrap();
        engine.pump(&mut host);
        let cwd = std::env::current_dir().expect("current directory");
        assert_eq!(
            host.events,
            vec![format!("log:{}/backup.log", cwd.display())]
        );
    }

    /// A hidden answer must not become a file name, a terminal message or a
    /// backup path, exactly as it must not reach a `run` command line.
    #[test]
    fn log_path_rejects_a_hidden_answer() {
        let mut host = FakeHost::default();
        let mut engine =
            ScriptEngine::parse("ask \"Secret:\" hide secret\nlog \"${secret}.log\"\n").unwrap();
        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::Ask);
        engine.on_ask_result(Some("hunter2".to_string()));
        engine.pump(&mut host);

        assert!(engine.is_finished());
        assert!(
            engine
                .error()
                .is_some_and(|error| error.contains("hidden answer")),
            "{:?}",
            engine.error()
        );
        assert!(!host.events.iter().any(|event| event.starts_with("log:")));
    }

    /// Copying a hidden answer with `set` must not launder it.
    #[test]
    fn a_copy_of_a_hidden_answer_stays_hidden() {
        let mut host = FakeHost::default();
        let mut engine = ScriptEngine::parse(
            "ask \"Secret:\" hide secret\nset copy \"x-${secret}\"\nlog \"${copy}.log\"\n",
        )
        .unwrap();
        engine.pump(&mut host);
        engine.on_ask_result(Some("hunter2".to_string()));
        engine.pump(&mut host);
        assert!(engine.is_finished());
        assert!(
            engine
                .error()
                .is_some_and(|error| error.contains("hidden answer")),
            "{:?}",
            engine.error()
        );
        assert!(!host.events.iter().any(|event| event.starts_with("log:")));
    }
    /// An `ask` default must not show a hidden answer in clear text.
    #[test]
    fn ask_default_never_offers_a_hidden_answer() {
        let mut host = FakeHost::default();
        let mut engine = ScriptEngine::parse(
            "ask \"Secret:\" hide secret\nask \"Again:\" default $secret again\n",
        )
        .unwrap();
        engine.pump(&mut host);
        engine.on_ask_result(Some("hunter2".to_string()));
        engine.pump(&mut host);
        assert!(
            host.events.iter().all(|event| !event.contains("hunter2")),
            "{:?}",
            host.events
        );
        assert!(engine.is_finished());
        assert!(
            engine
                .error()
                .is_some_and(|error| error.contains("hidden answer")),
            "{:?}",
            engine.error()
        );
    }
    /// The script log must not carry the text the `pager` expression removes.
    #[test]
    fn log_output_drops_pager_prompts() {
        let mut engine = ScriptEngine::parse("pager \"---- More ----\"\nprint \"x\"\n").unwrap();
        let mut host = FakeHost::default();
        engine.pump(&mut host);
        let mut log = Vec::new();
        for chunk in ["line1\r\nline2\r\n---- Mo", "re ----", "line3\r\nrouter#"] {
            log.extend(engine.filter_log_output(chunk.as_bytes()));
        }
        log.extend(engine.flush_log_output());
        let log = String::from_utf8(log).unwrap();
        assert!(!log.contains("More"), "{log:?}");
        assert_eq!(log, "line1\r\nline2\r\nline3\r\nrouter#");
    }

    /// A multi-byte character cut between two chunks survives in the log.
    #[test]
    fn log_output_keeps_a_split_character() {
        let mut engine = ScriptEngine::parse("pager \"---- More ----\"\nprint \"x\"\n").unwrap();
        let bytes = "ж\n".as_bytes();
        let mut log = engine.filter_log_output(&bytes[..1]);
        log.extend(engine.filter_log_output(&bytes[1..]));
        log.extend(engine.flush_log_output());
        assert_eq!(String::from_utf8(log).unwrap(), "ж\n");
    }

    /// Without `pager` the log is a byte-for-byte copy.
    #[test]
    fn log_output_is_untouched_without_pager() {
        let mut engine = ScriptEngine::parse("print \"x\"\n").unwrap();
        assert_eq!(
            engine.filter_log_output(b"a---- More ----b"),
            b"a---- More ----b"
        );
    }

    /// A failed capture must not leave the script claiming it saved something.
    #[test]
    fn a_log_that_cannot_be_opened_stops_the_script() {
        let mut host = FakeHost {
            log_fails: true,
            ..FakeHost::default()
        };
        let mut engine = ScriptEngine::parse("log \"x.log\"\nprint \"after\"\n").unwrap();
        engine.pump(&mut host);

        assert!(engine.is_finished());
        assert!(
            engine.error().is_some_and(|e| e.contains("Could not open")),
            "{:?}",
            engine.error()
        );
        assert!(!host.events.iter().any(|e| e.starts_with("print:")));
    }

    /// `ENV_*` is a variable-manager built-in, not a script one: expanding it
    /// here would let a script pull an environment secret past the scrubbed
    /// environment of `run`.
    #[test]
    fn scripts_do_not_expand_environment_builtins() {
        let mut host = FakeHost::default();
        let mut engine = ScriptEngine::parse("print \"${ENV_HOME}\"\n").unwrap();
        engine.pump(&mut host);
        assert_eq!(host.events, vec!["print:${ENV_HOME}"]);
    }
}
