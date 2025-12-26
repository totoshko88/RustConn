//! Automation manager for terminal sessions
//!
//! This module provides "Expect"-like functionality for terminal sessions,
//! allowing automatic responses to specific text patterns in the output.

use gtk4::glib;
use gtk4::glib::ControlFlow;
use regex::Regex;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
use vte4::prelude::*;
use vte4::{Format, Terminal};

/// A trigger rule that matches output and sends input
#[derive(Debug, Clone)]
pub struct Trigger {
    /// Regex pattern to match in terminal output
    pub pattern: Regex,
    /// Text to send when pattern matches
    pub response: String,
    /// Whether this trigger should only fire once
    pub one_shot: bool,
}

/// Shared state for automation triggers
struct AutomationState {
    triggers: Vec<Trigger>,
    /// Track which patterns have been matched (for one-shot)
    matched_patterns: Vec<String>,
    /// Last content to detect changes
    last_content: String,
    /// Counter for polling cycles
    poll_count: u32,
}

/// Manages automation for a terminal session
pub struct AutomationSession {
    #[allow(dead_code)]
    state: Rc<RefCell<AutomationState>>,
}

impl AutomationSession {
    pub fn new(terminal: Terminal, triggers: Vec<Trigger>) -> Self {
        tracing::info!(
            "AutomationSession: Created with {} triggers",
            triggers.len()
        );
        for trigger in &triggers {
            tracing::info!(
                "AutomationSession: Trigger pattern='{}', response='{}'",
                trigger.pattern,
                trigger.response.escape_debug()
            );
        }

        let state = Rc::new(RefCell::new(AutomationState {
            triggers,
            matched_patterns: Vec::new(),
            last_content: String::new(),
            poll_count: 0,
        }));

        // Start polling timer to check terminal content
        let state_clone = state.clone();
        let terminal_weak = terminal.downgrade();

        glib::timeout_add_local(Duration::from_millis(100), move || {
            let Some(terminal) = terminal_weak.upgrade() else {
                return ControlFlow::Break;
            };

            Self::check_terminal_content(&terminal, &state_clone);

            // Continue polling while we have triggers
            let has_triggers = !state_clone.borrow().triggers.is_empty();
            if has_triggers {
                ControlFlow::Continue
            } else {
                tracing::debug!("AutomationSession: No more triggers, stopping polling");
                ControlFlow::Break
            }
        });

        Self { state }
    }

    /// Process escape sequences in response string
    fn process_escapes(s: &str) -> String {
        let mut result = String::with_capacity(s.len());
        let mut chars = s.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.peek() {
                    Some('n') => {
                        result.push('\n');
                        chars.next();
                    }
                    Some('r') => {
                        result.push('\r');
                        chars.next();
                    }
                    Some('t') => {
                        result.push('\t');
                        chars.next();
                    }
                    Some('\\') => {
                        result.push('\\');
                        chars.next();
                    }
                    _ => result.push(c),
                }
            } else {
                result.push(c);
            }
        }

        result
    }

    fn check_terminal_content(terminal: &Terminal, state: &Rc<RefCell<AutomationState>>) {
        let mut state_ref = state.borrow_mut();

        // Skip if no triggers left
        if state_ref.triggers.is_empty() {
            return;
        }

        state_ref.poll_count += 1;

        // Get terminal dimensions and cursor position
        let (cursor_row, cursor_col) = terminal.cursor_position();
        let row_count = terminal.row_count();

        // Read content using text_range_format for the entire visible area
        // Use negative start row to include scrollback, and row_count for end
        let content = if let (Some(text), _) = terminal.text_range_format(
            Format::Text,
            0,             // start row
            0,             // start col
            row_count - 1, // end row (last visible row)
            -1,            // end col (-1 = end of line)
        ) {
            text.to_string()
        } else {
            String::new()
        };

        // Check if content changed
        let content_changed = content != state_ref.last_content;

        // Log periodically or when content changes significantly
        if state_ref.poll_count.is_multiple_of(500) {
            tracing::debug!(
                "AutomationSession: Poll #{}, cursor at ({}, {}), content len {}",
                state_ref.poll_count,
                cursor_row,
                cursor_col,
                content.len()
            );
        }

        // Skip pattern matching if content hasn't changed
        if !content_changed {
            return;
        }

        state_ref.last_content = content.clone();

        let mut to_remove = Vec::new();
        let mut responses_to_send = Vec::new();
        let mut patterns_to_mark = Vec::new();

        // Check each line
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            for (idx, trigger) in state_ref.triggers.iter().enumerate() {
                let pattern_str = trigger.pattern.to_string();

                // Skip if already matched
                if state_ref.matched_patterns.contains(&pattern_str) {
                    continue;
                }

                // Skip if already scheduled for removal
                if to_remove.contains(&idx) {
                    continue;
                }

                // Try matching against both full line and trimmed
                let matches = trigger.pattern.is_match(line) || trigger.pattern.is_match(trimmed);

                if matches {
                    tracing::info!(
                        "AutomationSession: MATCHED pattern '{}' on line '{}'",
                        trigger.pattern,
                        trimmed
                    );

                    let response = Self::process_escapes(&trigger.response);
                    tracing::info!(
                        "AutomationSession: Sending response: '{}'",
                        response.escape_debug()
                    );
                    responses_to_send.push(response);

                    if trigger.one_shot {
                        patterns_to_mark.push(pattern_str);
                        to_remove.push(idx);
                    }
                }
            }
        }

        // Mark patterns as matched
        for pattern in patterns_to_mark {
            state_ref.matched_patterns.push(pattern);
        }

        // Remove matched one-shot triggers (in reverse order)
        for idx in to_remove.into_iter().rev() {
            if idx < state_ref.triggers.len() {
                state_ref.triggers.remove(idx);
            }
        }

        // Drop borrow before sending
        drop(state_ref);

        // Send responses
        for response in responses_to_send {
            terminal.feed_child(response.as_bytes());
        }
    }
}
