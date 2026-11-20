//! Automation system for `RustConn`
//!
//! This module provides automation capabilities including:
//! - Key sequences for automated keystrokes after connection
//! - Expect-style pattern matching for interactive prompts
//! - Pre/post connection tasks

mod expect;
mod key_sequence;
mod script;
mod tasks;
mod templates;

pub use expect::{
    CompiledRule, ELEVATED_RESPONSE_TEMPLATE, ExpectEngine, ExpectError, ExpectResult, ExpectRule,
    elevated_credentials_rules,
};
pub use key_sequence::{KeyElement, KeySequence, KeySequenceError, KeySequenceResult, SpecialKey};
pub use script::{
    DEFAULT_TIMEOUT_MS, PendingKind, RUN_TIMEOUT_CEILING_MS, ScriptEngine, ScriptHost,
    ScriptParseError, ScriptProgram, ScriptStep, StepRecorder, parse_program, render_script,
};
pub use tasks::{
    ConnectionTask, FolderConnectionTracker, SCRUBBED_ENV_VARS, TaskCondition, TaskError,
    TaskExecutor, TaskResult, TaskTiming,
};
pub use templates::{AutomationTemplate, builtin_templates, templates_for_protocol};
