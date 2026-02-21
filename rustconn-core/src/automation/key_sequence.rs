//! Key sequence system for automated keystrokes
//!
//! This module provides the key sequence system for sending automated keystrokes
//! after connection establishment. It supports:
//! - Text literals
//! - Special keys (Enter, Tab, Escape, function keys, etc.)
//! - Wait commands for timing
//! - Variable references for dynamic content

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::variables::{VariableManager, VariableScope};

/// Errors that can occur during key sequence operations
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum KeySequenceError {
    /// Invalid key sequence syntax
    #[error("Invalid key sequence syntax: {0}")]
    InvalidSyntax(String),

    /// Unknown special key
    #[error("Unknown special key: {0}")]
    UnknownKey(String),

    /// Invalid wait duration
    #[error("Invalid wait duration: {0}")]
    InvalidWaitDuration(String),

    /// Unclosed brace in key sequence
    #[error("Unclosed brace starting at position {0}")]
    UnclosedBrace(usize),

    /// Empty key name
    #[error("Empty key name at position {0}")]
    EmptyKeyName(usize),

    /// Variable substitution error
    #[error("Variable error: {0}")]
    VariableError(String),
}

/// Result type for key sequence operations
pub type KeySequenceResult<T> = std::result::Result<T, KeySequenceError>;

/// Special keys that can be sent in a key sequence
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SpecialKey {
    /// Enter/Return key
    Enter,
    /// Tab key
    Tab,
    /// Escape key
    Escape,
    /// Backspace key
    Backspace,
    /// Delete key
    Delete,
    /// Up arrow
    Up,
    /// Down arrow
    Down,
    /// Left arrow
    Left,
    /// Right arrow
    Right,
    /// Home key
    Home,
    /// End key
    End,
    /// Page Up key
    PageUp,
    /// Page Down key
    PageDown,
    /// Insert key
    Insert,
    /// F1 function key
    F1,
    /// F2 function key
    F2,
    /// F3 function key
    F3,
    /// F4 function key
    F4,
    /// F5 function key
    F5,
    /// F6 function key
    F6,
    /// F7 function key
    F7,
    /// F8 function key
    F8,
    /// F9 function key
    F9,
    /// F10 function key
    F10,
    /// F11 function key
    F11,
    /// F12 function key
    F12,
    /// Ctrl+C
    CtrlC,
    /// Ctrl+D
    CtrlD,
    /// Ctrl+Z
    CtrlZ,
    /// Ctrl+A
    CtrlA,
    /// Ctrl+E
    CtrlE,
    /// Ctrl+L
    CtrlL,
    /// Space key
    Space,
}

impl SpecialKey {
    /// Parses a special key from its string representation
    ///
    /// # Errors
    ///
    /// Returns `KeySequenceError::UnknownKey` if the key name is not recognized.
    pub fn parse(s: &str) -> KeySequenceResult<Self> {
        let key = match s.to_uppercase().as_str() {
            "ENTER" | "RETURN" => Self::Enter,
            "TAB" => Self::Tab,
            "ESCAPE" | "ESC" => Self::Escape,
            "BACKSPACE" | "BS" => Self::Backspace,
            "DELETE" | "DEL" => Self::Delete,
            "UP" => Self::Up,
            "DOWN" => Self::Down,
            "LEFT" => Self::Left,
            "RIGHT" => Self::Right,
            "HOME" => Self::Home,
            "END" => Self::End,
            "PAGEUP" | "PGUP" => Self::PageUp,
            "PAGEDOWN" | "PGDN" => Self::PageDown,
            "INSERT" | "INS" => Self::Insert,
            "F1" => Self::F1,
            "F2" => Self::F2,
            "F3" => Self::F3,
            "F4" => Self::F4,
            "F5" => Self::F5,
            "F6" => Self::F6,
            "F7" => Self::F7,
            "F8" => Self::F8,
            "F9" => Self::F9,
            "F10" => Self::F10,
            "F11" => Self::F11,
            "F12" => Self::F12,
            "CTRL+C" | "CTRLC" | "CTRL-C" => Self::CtrlC,
            "CTRL+D" | "CTRLD" | "CTRL-D" => Self::CtrlD,
            "CTRL+Z" | "CTRLZ" | "CTRL-Z" => Self::CtrlZ,
            "CTRL+A" | "CTRLA" | "CTRL-A" => Self::CtrlA,
            "CTRL+E" | "CTRLE" | "CTRL-E" => Self::CtrlE,
            "CTRL+L" | "CTRLL" | "CTRL-L" => Self::CtrlL,
            "SPACE" => Self::Space,
            _ => return Err(KeySequenceError::UnknownKey(s.to_string())),
        };
        Ok(key)
    }

    /// Returns the string representation of this special key
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Enter => "ENTER",
            Self::Tab => "TAB",
            Self::Escape => "ESCAPE",
            Self::Backspace => "BACKSPACE",
            Self::Delete => "DELETE",
            Self::Up => "UP",
            Self::Down => "DOWN",
            Self::Left => "LEFT",
            Self::Right => "RIGHT",
            Self::Home => "HOME",
            Self::End => "END",
            Self::PageUp => "PAGEUP",
            Self::PageDown => "PAGEDOWN",
            Self::Insert => "INSERT",
            Self::F1 => "F1",
            Self::F2 => "F2",
            Self::F3 => "F3",
            Self::F4 => "F4",
            Self::F5 => "F5",
            Self::F6 => "F6",
            Self::F7 => "F7",
            Self::F8 => "F8",
            Self::F9 => "F9",
            Self::F10 => "F10",
            Self::F11 => "F11",
            Self::F12 => "F12",
            Self::CtrlC => "CTRL+C",
            Self::CtrlD => "CTRL+D",
            Self::CtrlZ => "CTRL+Z",
            Self::CtrlA => "CTRL+A",
            Self::CtrlE => "CTRL+E",
            Self::CtrlL => "CTRL+L",
            Self::Space => "SPACE",
        }
    }
}

impl fmt::Display for SpecialKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{{}}}", self.as_str())
    }
}

/// A key sequence element
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyElement {
    /// Plain text to be typed
    Text(String),
    /// A special key (Enter, Tab, etc.)
    SpecialKey(SpecialKey),
    /// Wait for specified milliseconds
    Wait(u32),
    /// Variable reference to be substituted
    Variable(String),
}

impl fmt::Display for KeyElement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(text) => {
                // Escape special characters in text
                for ch in text.chars() {
                    match ch {
                        '{' => write!(f, "{{")?,
                        '}' => write!(f, "}}")?,
                        '$' => write!(f, "$$")?,
                        _ => write!(f, "{ch}")?,
                    }
                }
                Ok(())
            }
            Self::SpecialKey(key) => write!(f, "{key}"),
            Self::Wait(ms) => write!(f, "{{WAIT:{ms}}}"),
            Self::Variable(name) => write!(f, "${{{name}}}"),
        }
    }
}

/// A parsed key sequence
///
/// Key sequences support the following syntax:
/// - Plain text: `hello world`
/// - Special keys: `{ENTER}`, `{TAB}`, `{F1}`
/// - Wait commands: `{WAIT:1000}` (milliseconds)
/// - Variable references: `${username}`
/// - Escaped braces: `{{` for `{`, `}}` for `}`
/// - Escaped dollar: `$$` for `$`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct KeySequence {
    /// The elements that make up this key sequence
    pub elements: Vec<KeyElement>,
}

impl KeySequence {
    /// Creates a new empty key sequence
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a key sequence from a vector of elements
    #[must_use]
    pub const fn from_elements(elements: Vec<KeyElement>) -> Self {
        Self { elements }
    }

    /// Parses a key sequence string
    ///
    /// # Syntax
    ///
    /// - Plain text: `hello world`
    /// - Special keys: `{ENTER}`, `{TAB}`, `{F1}`
    /// - Wait commands: `{WAIT:1000}` (milliseconds)
    /// - Variable references: `${username}`
    /// - Escaped braces: `{{` for literal `{`, `}}` for literal `}`
    /// - Escaped dollar: `$$` for literal `$`
    ///
    /// # Errors
    ///
    /// Returns an error if the syntax is invalid.
    pub fn parse(input: &str) -> KeySequenceResult<Self> {
        let mut elements = Vec::new();
        let mut current_text = String::new();
        let chars: Vec<char> = input.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            match chars[i] {
                '{' => {
                    // Check for escaped brace
                    if i + 1 < chars.len() && chars[i + 1] == '{' {
                        current_text.push('{');
                        i += 2;
                        continue;
                    }

                    // Flush current text
                    if !current_text.is_empty() {
                        elements.push(KeyElement::Text(std::mem::take(&mut current_text)));
                    }

                    // Find closing brace
                    let start = i;
                    i += 1;
                    let content_start = i;

                    while i < chars.len() && chars[i] != '}' {
                        i += 1;
                    }

                    if i >= chars.len() {
                        return Err(KeySequenceError::UnclosedBrace(start));
                    }

                    let content: String = chars[content_start..i].iter().collect();
                    if content.is_empty() {
                        return Err(KeySequenceError::EmptyKeyName(start));
                    }

                    // Parse the content
                    let element = Self::parse_brace_content(&content, start)?;
                    elements.push(element);

                    i += 1; // Skip closing brace
                }
                '}' => {
                    // Check for escaped brace
                    if i + 1 < chars.len() && chars[i + 1] == '}' {
                        current_text.push('}');
                        i += 2;
                        continue;
                    }
                    // Unmatched closing brace - treat as literal
                    current_text.push('}');
                    i += 1;
                }
                '$' => {
                    // Check for escaped dollar
                    if i + 1 < chars.len() && chars[i + 1] == '$' {
                        current_text.push('$');
                        i += 2;
                        continue;
                    }

                    // Check for variable reference ${...}
                    if i + 1 < chars.len() && chars[i + 1] == '{' {
                        // Flush current text
                        if !current_text.is_empty() {
                            elements.push(KeyElement::Text(std::mem::take(&mut current_text)));
                        }

                        let start = i;
                        i += 2; // Skip ${
                        let var_start = i;

                        while i < chars.len() && chars[i] != '}' {
                            i += 1;
                        }

                        if i >= chars.len() {
                            return Err(KeySequenceError::UnclosedBrace(start));
                        }

                        let var_name: String = chars[var_start..i].iter().collect();
                        if var_name.is_empty() {
                            return Err(KeySequenceError::EmptyKeyName(start));
                        }

                        // Validate variable name
                        if !Self::is_valid_variable_name(&var_name) {
                            return Err(KeySequenceError::InvalidSyntax(format!(
                                "Invalid variable name: {var_name}"
                            )));
                        }

                        elements.push(KeyElement::Variable(var_name));
                    } else {
                        // Just a dollar sign
                        current_text.push('$');
                    }
                    i += 1;
                }
                ch => {
                    current_text.push(ch);
                    i += 1;
                }
            }
        }

        // Flush remaining text
        if !current_text.is_empty() {
            elements.push(KeyElement::Text(current_text));
        }

        Ok(Self { elements })
    }

    /// Parses content inside braces (special key or wait command)
    fn parse_brace_content(content: &str, position: usize) -> KeySequenceResult<KeyElement> {
        // Check for WAIT command
        if let Some(duration_str) = content.strip_prefix("WAIT:") {
            let duration: u32 = duration_str
                .parse()
                .map_err(|_| KeySequenceError::InvalidWaitDuration(duration_str.to_string()))?;
            return Ok(KeyElement::Wait(duration));
        }

        // Try to parse as special key
        match SpecialKey::parse(content) {
            Ok(key) => Ok(KeyElement::SpecialKey(key)),
            Err(KeySequenceError::UnknownKey(name)) => Err(KeySequenceError::InvalidSyntax(
                format!("Unknown key or command '{name}' at position {position}"),
            )),
            Err(e) => Err(e),
        }
    }

    /// Validates a variable name
    fn is_valid_variable_name(name: &str) -> bool {
        let mut chars = name.chars();
        match chars.next() {
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
            _ => return false,
        }
        chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    }

    /// Validates the key sequence
    ///
    /// # Errors
    ///
    /// Returns an error if the sequence contains invalid elements.
    pub fn validate(&self) -> KeySequenceResult<()> {
        for element in &self.elements {
            if let KeyElement::Variable(name) = element
                && !Self::is_valid_variable_name(name)
            {
                return Err(KeySequenceError::InvalidSyntax(format!(
                    "Invalid variable name: {name}"
                )));
            }
        }
        Ok(())
    }

    /// Returns true if this key sequence is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// Returns the number of elements in this key sequence
    #[must_use]
    pub fn len(&self) -> usize {
        self.elements.len()
    }

    /// Extracts all variable names referenced in this key sequence
    #[must_use]
    pub fn variable_references(&self) -> Vec<&str> {
        self.elements
            .iter()
            .filter_map(|e| {
                if let KeyElement::Variable(name) = e {
                    Some(name.as_str())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Substitutes variables in the key sequence using the provided manager
    ///
    /// # Errors
    ///
    /// Returns an error if variable substitution fails.
    pub fn substitute_variables(
        &self,
        manager: &VariableManager,
        scope: VariableScope,
    ) -> KeySequenceResult<Self> {
        let mut new_elements = Vec::with_capacity(self.elements.len());

        for element in &self.elements {
            match element {
                KeyElement::Variable(name) => {
                    // Resolve the variable and convert to text
                    match manager.resolve(name, scope) {
                        Ok(value) => {
                            if !value.is_empty() {
                                new_elements.push(KeyElement::Text(value));
                            }
                        }
                        Err(e) => {
                            return Err(KeySequenceError::VariableError(e.to_string()));
                        }
                    }
                }
                KeyElement::Text(text) => {
                    // Also substitute any ${var} patterns in text
                    match manager.substitute_for_command(text, scope) {
                        Ok(substituted) => {
                            if !substituted.is_empty() {
                                new_elements.push(KeyElement::Text(substituted));
                            }
                        }
                        Err(e) => {
                            return Err(KeySequenceError::VariableError(e.to_string()));
                        }
                    }
                }
                other => {
                    new_elements.push(other.clone());
                }
            }
        }

        Ok(Self {
            elements: new_elements,
        })
    }

    /// Substitutes variables using an Arc-wrapped manager (for async contexts)
    ///
    /// # Errors
    ///
    /// Returns an error if variable substitution fails.
    pub fn substitute_variables_arc(
        &self,
        manager: &Arc<VariableManager>,
        scope: VariableScope,
    ) -> KeySequenceResult<Self> {
        self.substitute_variables(manager.as_ref(), scope)
    }
}

impl fmt::Display for KeySequence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for element in &self.elements {
            write!(f, "{element}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_plain_text() {
        let seq = KeySequence::parse("hello world").unwrap();
        assert_eq!(seq.elements.len(), 1);
        assert_eq!(seq.elements[0], KeyElement::Text("hello world".to_string()));
    }

    #[test]
    fn test_parse_special_key() {
        let seq = KeySequence::parse("{ENTER}").unwrap();
        assert_eq!(seq.elements.len(), 1);
        assert_eq!(seq.elements[0], KeyElement::SpecialKey(SpecialKey::Enter));
    }

    #[test]
    fn test_parse_multiple_special_keys() {
        let seq = KeySequence::parse("{TAB}{ENTER}").unwrap();
        assert_eq!(seq.elements.len(), 2);
        assert_eq!(seq.elements[0], KeyElement::SpecialKey(SpecialKey::Tab));
        assert_eq!(seq.elements[1], KeyElement::SpecialKey(SpecialKey::Enter));
    }

    #[test]
    fn test_parse_wait_command() {
        let seq = KeySequence::parse("{WAIT:1000}").unwrap();
        assert_eq!(seq.elements.len(), 1);
        assert_eq!(seq.elements[0], KeyElement::Wait(1000));
    }

    #[test]
    fn test_parse_variable() {
        let seq = KeySequence::parse("${username}").unwrap();
        assert_eq!(seq.elements.len(), 1);
        assert_eq!(
            seq.elements[0],
            KeyElement::Variable("username".to_string())
        );
    }

    #[test]
    fn test_parse_mixed_sequence() {
        let seq = KeySequence::parse("user{TAB}${password}{ENTER}").unwrap();
        assert_eq!(seq.elements.len(), 4);
        assert_eq!(seq.elements[0], KeyElement::Text("user".to_string()));
        assert_eq!(seq.elements[1], KeyElement::SpecialKey(SpecialKey::Tab));
        assert_eq!(
            seq.elements[2],
            KeyElement::Variable("password".to_string())
        );
        assert_eq!(seq.elements[3], KeyElement::SpecialKey(SpecialKey::Enter));
    }

    #[test]
    fn test_parse_escaped_braces() {
        let seq = KeySequence::parse("{{literal}}").unwrap();
        assert_eq!(seq.elements.len(), 1);
        assert_eq!(seq.elements[0], KeyElement::Text("{literal}".to_string()));
    }

    #[test]
    fn test_parse_escaped_dollar() {
        let seq = KeySequence::parse("$$100").unwrap();
        assert_eq!(seq.elements.len(), 1);
        assert_eq!(seq.elements[0], KeyElement::Text("$100".to_string()));
    }

    #[test]
    fn test_parse_unclosed_brace() {
        let result = KeySequence::parse("{ENTER");
        assert!(matches!(result, Err(KeySequenceError::UnclosedBrace(_))));
    }

    #[test]
    fn test_parse_empty_key_name() {
        let result = KeySequence::parse("{}");
        assert!(matches!(result, Err(KeySequenceError::EmptyKeyName(_))));
    }

    #[test]
    fn test_parse_unknown_key() {
        let result = KeySequence::parse("{UNKNOWN}");
        assert!(matches!(result, Err(KeySequenceError::InvalidSyntax(_))));
    }

    #[test]
    fn test_parse_invalid_wait_duration() {
        let result = KeySequence::parse("{WAIT:abc}");
        assert!(matches!(
            result,
            Err(KeySequenceError::InvalidWaitDuration(_))
        ));
    }

    #[test]
    fn test_to_string_round_trip() {
        #[allow(clippy::literal_string_with_formatting_args)]
        let original = "user{TAB}${password}{ENTER}{WAIT:500}";
        let seq = KeySequence::parse(original).unwrap();
        let serialized = seq.to_string();
        let reparsed = KeySequence::parse(&serialized).unwrap();
        assert_eq!(seq, reparsed);
    }

    #[test]
    fn test_special_key_case_insensitive() {
        let seq1 = KeySequence::parse("{enter}").unwrap();
        let seq2 = KeySequence::parse("{ENTER}").unwrap();
        let seq3 = KeySequence::parse("{Enter}").unwrap();
        assert_eq!(seq1, seq2);
        assert_eq!(seq2, seq3);
    }

    #[test]
    fn test_all_special_keys() {
        let keys = [
            "ENTER",
            "TAB",
            "ESCAPE",
            "BACKSPACE",
            "DELETE",
            "UP",
            "DOWN",
            "LEFT",
            "RIGHT",
            "HOME",
            "END",
            "PAGEUP",
            "PAGEDOWN",
            "INSERT",
            "F1",
            "F2",
            "F3",
            "F4",
            "F5",
            "F6",
            "F7",
            "F8",
            "F9",
            "F10",
            "F11",
            "F12",
            "CTRL+C",
            "CTRL+D",
            "CTRL+Z",
            "CTRL+A",
            "CTRL+E",
            "CTRL+L",
            "SPACE",
        ];

        for key in keys {
            let input = format!("{{{key}}}");
            let result = KeySequence::parse(&input);
            assert!(result.is_ok(), "Failed to parse key: {key}");
        }
    }

    #[test]
    fn test_variable_references() {
        let seq = KeySequence::parse("${user}@${host}").unwrap();
        let refs = seq.variable_references();
        assert_eq!(refs.len(), 2);
        assert!(refs.contains(&"user"));
        assert!(refs.contains(&"host"));
    }

    #[test]
    fn test_empty_sequence() {
        let seq = KeySequence::parse("").unwrap();
        assert!(seq.is_empty());
        assert_eq!(seq.len(), 0);
    }

    #[test]
    fn test_validate_valid_sequence() {
        let seq = KeySequence::parse("${valid_name}").unwrap();
        assert!(seq.validate().is_ok());
    }

    #[test]
    fn test_substitute_variables() {
        let mut manager = VariableManager::new();
        manager.set_global(crate::Variable::new("user", "admin"));
        manager.set_global(crate::Variable::new("pass", "secret"));

        let seq = KeySequence::parse("${user}{TAB}${pass}{ENTER}").unwrap();
        let substituted = seq
            .substitute_variables(&manager, VariableScope::Global)
            .unwrap();

        assert_eq!(substituted.elements.len(), 4);
        assert_eq!(
            substituted.elements[0],
            KeyElement::Text("admin".to_string())
        );
        assert_eq!(
            substituted.elements[1],
            KeyElement::SpecialKey(SpecialKey::Tab)
        );
        assert_eq!(
            substituted.elements[2],
            KeyElement::Text("secret".to_string())
        );
        assert_eq!(
            substituted.elements[3],
            KeyElement::SpecialKey(SpecialKey::Enter)
        );
    }

    #[test]
    fn test_substitute_undefined_variable() {
        let manager = VariableManager::new();
        let seq = KeySequence::parse("${undefined}").unwrap();
        let result = seq.substitute_variables(&manager, VariableScope::Global);
        assert!(matches!(result, Err(KeySequenceError::VariableError(_))));
    }

    #[test]
    fn test_key_aliases() {
        // Test alternative key names
        assert_eq!(
            KeySequence::parse("{RETURN}").unwrap(),
            KeySequence::parse("{ENTER}").unwrap()
        );
        assert_eq!(
            KeySequence::parse("{ESC}").unwrap(),
            KeySequence::parse("{ESCAPE}").unwrap()
        );
        assert_eq!(
            KeySequence::parse("{BS}").unwrap(),
            KeySequence::parse("{BACKSPACE}").unwrap()
        );
        assert_eq!(
            KeySequence::parse("{DEL}").unwrap(),
            KeySequence::parse("{DELETE}").unwrap()
        );
        assert_eq!(
            KeySequence::parse("{PGUP}").unwrap(),
            KeySequence::parse("{PAGEUP}").unwrap()
        );
        assert_eq!(
            KeySequence::parse("{PGDN}").unwrap(),
            KeySequence::parse("{PAGEDOWN}").unwrap()
        );
        assert_eq!(
            KeySequence::parse("{INS}").unwrap(),
            KeySequence::parse("{INSERT}").unwrap()
        );
    }

    #[test]
    fn test_ctrl_key_variants() {
        // Test different Ctrl key formats
        assert_eq!(
            KeySequence::parse("{CTRL+C}").unwrap(),
            KeySequence::parse("{CTRLC}").unwrap()
        );
        assert_eq!(
            KeySequence::parse("{CTRL-C}").unwrap(),
            KeySequence::parse("{CTRL+C}").unwrap()
        );
    }
}
