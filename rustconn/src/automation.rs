//! Automation manager for terminal sessions
//!
//! This module provides "Expect"-like functionality for terminal sessions,
//! allowing automatic responses to specific text patterns in the output.

use regex::Regex;
use std::cell::RefCell;
use std::rc::Rc;
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

/// Manages automation for a terminal session
pub struct AutomationSession {
    terminal: Terminal,
    triggers: Rc<RefCell<Vec<Trigger>>>,
    // Buffer to store recent output for matching
    buffer: RefCell<String>,
}

impl AutomationSession {
    pub fn new(terminal: Terminal, triggers: Vec<Trigger>) -> Self {
        let session = Self {
            terminal: terminal.clone(),
            triggers: Rc::new(RefCell::new(triggers)),
            buffer: RefCell::new(String::new()),
        };

        session.setup_listener();
        session
    }

    fn setup_listener(&self) {
        let triggers = self.triggers.clone();
        let terminal = self.terminal.clone();

        // Debug: Print loaded triggers
        /*
        {
            let t = triggers.borrow();
            println!("DEBUG: Automation listener setup with {} triggers", t.len());
            for trigger in t.iter() {
                println!("DEBUG: Trigger pattern: '{}'", trigger.pattern);
            }
        }
        */

        // Also listen to cursor moves, as typing often just moves the cursor
        let triggers_cursor = self.triggers.clone();
        let _terminal_cursor = self.terminal.clone();
        terminal.connect_cursor_moved(move |terminal| {
             let (row, _col) = terminal.cursor_position();
             
             // Check for matches on cursor move too
             if let (Some(text), _) = terminal.text_range_format(Format::Text, row, 0, row, -1) {
                let line = text.as_str();
                if !line.trim().is_empty() {
                    let mut triggers_ref = triggers_cursor.borrow_mut();
                    if !triggers_ref.is_empty() {
                        triggers_ref.retain(|trigger| {
                            if trigger.pattern.is_match(line) {
                                // println!("DEBUG: MATCHED (cursor) pattern '{}' on line '{}'", trigger.pattern, line);
                                terminal.feed_child(trigger.response.as_bytes());
                                !trigger.one_shot
                            } else {
                                true
                            }
                        });
                    }
                }
             }
        });

        terminal.connect_contents_changed(move |terminal| {
            // Get current cursor position
            let (row, _col) = terminal.cursor_position();
            
            // Scan a window of lines around the cursor
            let start_row = row.saturating_sub(10);
            let end_row = row;

            for r in start_row..=end_row {
                // Try to read the line
                // Note: col -1 means "end of line"
                match terminal.text_range_format(Format::Text, r, 0, r, -1) {
                    (Some(text), _) => {
                        let line = text.as_str();
                        
                        if !line.trim().is_empty() {
                            // println!("DEBUG: Line {} (cursor at {},{}): [{}]", r, row, _col, line);
                            
                            let mut triggers_ref = triggers.borrow_mut();
                            if triggers_ref.is_empty() {
                                continue;
                            }

                            triggers_ref.retain(|trigger| {
                                // Check if pattern matches
                                let matched = trigger.pattern.is_match(line);
                                if matched {
                                    // println!("DEBUG: MATCHED pattern '{}' on line '{}'", trigger.pattern, line);
                                    terminal.feed_child(trigger.response.as_bytes());
                                }
                                !matched || !trigger.one_shot
                            });
                        }
                    }
                    (None, _) => {}
                }
            }
        });
    }
}
