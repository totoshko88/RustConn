//! Automation system for `RustConn`
//!
//! This module provides automation capabilities including:
//! - Key sequences for automated keystrokes after connection
//! - Expect-style pattern matching for interactive prompts
//! - Pre/post connection tasks

mod expect;
mod key_sequence;
mod tasks;
mod templates;

pub use expect::{CompiledRule, ExpectEngine, ExpectError, ExpectResult, ExpectRule};
pub use key_sequence::{KeyElement, KeySequence, KeySequenceError, KeySequenceResult, SpecialKey};
pub use tasks::{
    ConnectionTask, FolderConnectionTracker, TaskCondition, TaskError, TaskExecutor, TaskResult,
    TaskTiming,
};
pub use templates::{AutomationTemplate, builtin_templates, templates_for_protocol};
