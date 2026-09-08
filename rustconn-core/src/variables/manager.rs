//! Variable manager for resolution and substitution
//!
//! This module provides the `VariableManager` which handles variable storage,
//! resolution across scopes, and substitution in strings.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

use super::{MAX_NESTING_DEPTH, Variable, VariableError, VariableResult, VariableScope};

/// Cached regex for variable extraction: matches `${var_name}` patterns
pub static VARIABLE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\$\{([a-zA-Z_][a-zA-Z0-9_]*)\}").expect("VARIABLE_REGEX is a valid regex pattern")
});

/// Outcome of [`VariableManager::substitute_for_terminal_input`].
///
/// Carries the names that could not be resolved alongside the text, because a
/// terminal answer that silently lost a reference is indistinguishable from a
/// wrong one at the far end — the caller needs to be able to say which variable
/// is missing (issue #257).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct TerminalSubstitution {
    /// The input with every *defined* `${...}` reference replaced by its value.
    /// The buffer is scrubbed on drop because it may contain a credential.
    pub text: Zeroizing<String>,
    /// Names nothing defined; still present as `${name}` in `text`.
    pub unresolved: Vec<String>,
}

/// Variable manager for resolution and substitution
///
/// Manages variables at different scopes and provides methods for:
/// - Resolving single variable references
/// - Substituting all variables in a string
/// - Parsing variable references from strings
/// - Detecting circular references
#[derive(Debug, Default)]
pub struct VariableManager {
    /// Global variables available to all connections
    global_vars: HashMap<String, Variable>,
    /// Connection-scoped variables indexed by connection ID
    connection_vars: HashMap<Uuid, HashMap<String, Variable>>,
}

impl VariableManager {
    /// Creates a new empty `VariableManager`
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    // ========== Variable Management ==========

    /// Sets a global variable
    pub fn set_global(&mut self, variable: Variable) {
        self.global_vars.insert(variable.name.clone(), variable);
    }

    /// Sets a connection-scoped variable
    pub fn set_connection(&mut self, connection_id: Uuid, variable: Variable) {
        self.connection_vars
            .entry(connection_id)
            .or_default()
            .insert(variable.name.clone(), variable);
    }

    /// Gets a global variable by name
    #[must_use]
    pub fn get_global(&self, name: &str) -> Option<&Variable> {
        self.global_vars.get(name)
    }

    /// Gets a connection-scoped variable by name
    #[must_use]
    pub fn get_connection(&self, connection_id: Uuid, name: &str) -> Option<&Variable> {
        self.connection_vars
            .get(&connection_id)
            .and_then(|vars| vars.get(name))
    }

    /// Removes a global variable
    pub fn remove_global(&mut self, name: &str) -> Option<Variable> {
        self.global_vars.remove(name)
    }

    /// Removes a connection-scoped variable
    pub fn remove_connection(&mut self, connection_id: Uuid, name: &str) -> Option<Variable> {
        self.connection_vars
            .get_mut(&connection_id)
            .and_then(|vars| vars.remove(name))
    }

    /// Lists all global variables
    #[must_use]
    pub fn list_global(&self) -> Vec<&Variable> {
        self.global_vars.values().collect()
    }

    /// Lists all connection-scoped variables
    #[must_use]
    pub fn list_connection(&self, connection_id: Uuid) -> Vec<&Variable> {
        self.connection_vars
            .get(&connection_id)
            .map(|vars| vars.values().collect())
            .unwrap_or_default()
    }

    // ========== Resolution ==========

    /// Resolves a variable reference to its value
    ///
    /// Resolution follows the scope chain from most specific to least specific:
    /// - For `Connection` scope: Connection -> Global
    /// - For `Global` scope: Global only
    ///
    /// # Arguments
    ///
    /// * `name` - The variable name to resolve
    /// * `scope` - The starting scope for resolution
    ///
    /// # Returns
    ///
    /// The resolved variable value, or an error if not found.
    ///
    /// # Errors
    ///
    /// Returns `VariableError::Undefined` if the variable is not found in any scope.
    pub fn resolve(&self, name: &str, scope: VariableScope) -> VariableResult<String> {
        self.resolve_with_depth(name, scope, 0, &mut HashSet::new())
    }

    /// Internal resolution with depth tracking and cycle detection
    fn resolve_with_depth(
        &self,
        name: &str,
        scope: VariableScope,
        depth: usize,
        visited: &mut HashSet<String>,
    ) -> VariableResult<String> {
        if depth > MAX_NESTING_DEPTH {
            return Err(VariableError::MaxDepthExceeded(MAX_NESTING_DEPTH));
        }

        if visited.contains(name) {
            return Err(VariableError::CircularReference(name.to_string()));
        }

        // Look up the variable in the scope chain
        let variable = self.lookup_in_scope_chain(name, scope);

        match variable {
            Some(var) => {
                // Check if the value contains nested variable references
                let refs = Self::parse_references(&var.value)?;
                if refs.is_empty() {
                    Ok(var.value.clone())
                } else {
                    // Resolve nested references
                    visited.insert(name.to_string());
                    let result =
                        self.substitute_with_depth(&var.value, scope, depth + 1, visited)?;
                    visited.remove(name);
                    Ok(result)
                }
            }
            // A user-defined variable always wins. Only when nothing is defined
            // do we fall back to the built-in dynamic variables (date/time and
            // `ENV_*`), so a user who defines e.g. `TIMESTAMP` keeps their value.
            None => Self::resolve_builtin(name)
                .ok_or_else(|| VariableError::Undefined(name.to_string())),
        }
    }

    /// Resolves a built-in dynamic variable, or `None` if `name` is not one.
    ///
    /// These need no definition and are computed at substitution time:
    ///
    /// | Name | Value |
    /// |------|-------|
    /// | `DATE_Y` | local year, e.g. `2026` |
    /// | `DATE_M` | local month, zero-padded (`01`–`12`) |
    /// | `DATE_D` | local day of month, zero-padded (`01`–`31`) |
    /// | `TIME_H` | local hour, 24h, zero-padded (`00`–`23`) |
    /// | `TIME_M` | local minute, zero-padded (`00`–`59`) |
    /// | `TIME_S` | local second, zero-padded (`00`–`60`) |
    /// | `TIMESTAMP` | seconds since the Unix epoch |
    /// | `ENV_<NAME>` | value of the `<NAME>` environment variable |
    ///
    /// The names mirror Ásbrú Connection Manager's `<DATE_Y>` / `<ENV:name>`
    /// masks, adapted to RustConn's `${...}` syntax — `ENV_HOME` rather than
    /// `ENV:HOME`, because the reference regex does not admit a colon and
    /// forking the syntax for one case is not worth it.
    ///
    /// An `ENV_` reference to an unset variable resolves to the empty string
    /// (not `None`), matching how a shell expands `${HOME}` — otherwise the
    /// caller's undefined-variable handling would leave the literal `${ENV_X}`
    /// in place, which is never what the user meant.
    fn resolve_builtin(name: &str) -> Option<String> {
        use chrono::{Datelike, Local, Timelike};

        if let Some(env_name) = name.strip_prefix("ENV_") {
            // An empty suffix (`${ENV_}`) is not a real environment reference.
            if env_name.is_empty() {
                return None;
            }
            return Some(std::env::var(env_name).unwrap_or_default());
        }

        // Computed once per call. Within a single substitute() pass the sub-second
        // drift between two date/time references is irrelevant, and TIMESTAMP is
        // whole seconds regardless.
        let now = Local::now();
        let value = match name {
            "DATE_Y" => format!("{:04}", now.year()),
            "DATE_M" => format!("{:02}", now.month()),
            "DATE_D" => format!("{:02}", now.day()),
            "TIME_H" => format!("{:02}", now.hour()),
            "TIME_M" => format!("{:02}", now.minute()),
            "TIME_S" => format!("{:02}", now.second()),
            "TIMESTAMP" => now.timestamp().to_string(),
            _ => return None,
        };
        Some(value)
    }

    /// Looks up a variable in the scope chain
    fn lookup_in_scope_chain(&self, name: &str, scope: VariableScope) -> Option<&Variable> {
        match scope {
            VariableScope::Global => self.global_vars.get(name),
            VariableScope::Connection(conn_id) => self
                .connection_vars
                .get(&conn_id)
                .and_then(|vars| vars.get(name))
                .or_else(|| self.global_vars.get(name)),
        }
    }

    // ========== Substitution ==========

    /// Substitutes all variable references in a string
    ///
    /// Variable references use the `${variable_name}` syntax.
    ///
    /// # Arguments
    ///
    /// * `input` - The string containing variable references
    /// * `scope` - The scope for variable resolution
    ///
    /// # Returns
    ///
    /// The string with all variables substituted, or an error.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - A referenced variable is undefined (logs warning, uses empty string)
    /// - Circular reference is detected
    /// - Maximum nesting depth is exceeded
    pub fn substitute(&self, input: &str, scope: VariableScope) -> VariableResult<String> {
        self.substitute_with_depth(input, scope, 0, &mut HashSet::new())
    }

    /// Internal substitution with depth tracking
    fn substitute_with_depth(
        &self,
        input: &str,
        scope: VariableScope,
        depth: usize,
        visited: &mut HashSet<String>,
    ) -> VariableResult<String> {
        if depth > MAX_NESTING_DEPTH {
            return Err(VariableError::MaxDepthExceeded(MAX_NESTING_DEPTH));
        }

        let mut result = input.to_string();
        let mut undefined_vars = Vec::new();

        // Find all variable references
        let refs = Self::parse_references(input)?;

        for var_name in refs {
            match self.resolve_with_depth(&var_name, scope, depth, visited) {
                Ok(value) => {
                    let pattern = format!("${{{var_name}}}");
                    result = result.replace(&pattern, &value);
                }
                Err(VariableError::Undefined(_)) => {
                    // Log warning and use empty string for undefined variables
                    undefined_vars.push(var_name.clone());
                    let pattern = format!("${{{var_name}}}");
                    result = result.replace(&pattern, "");
                }
                Err(e) => return Err(e),
            }
        }

        // Log warnings for undefined variables (in production, use proper logging)
        #[cfg(debug_assertions)]
        for var in &undefined_vars {
            tracing::debug!("Undefined variable: {var}");
        }

        Ok(result)
    }

    // ========== Parsing ==========

    /// Parses variable references from a string
    ///
    /// Extracts all variable names from `${variable_name}` patterns.
    ///
    /// # Arguments
    ///
    /// * `input` - The string to parse
    ///
    /// # Returns
    ///
    /// A vector of unique variable names found in the string.
    ///
    /// # Errors
    ///
    /// Returns `VariableError::InvalidSyntax` for malformed variable references.
    pub fn parse_references(input: &str) -> VariableResult<Vec<String>> {
        let re = Self::variable_regex();
        let mut variables = Vec::new();
        let mut seen = HashSet::new();

        for cap in re.captures_iter(input) {
            if let Some(var_name) = cap.get(1) {
                let name = var_name.as_str().to_string();
                if name.is_empty() {
                    return Err(VariableError::EmptyName);
                }
                if !seen.contains(&name) {
                    seen.insert(name.clone());
                    variables.push(name);
                }
            }
        }

        Ok(variables)
    }

    /// Returns the regex for matching variable references
    fn variable_regex() -> &'static Regex {
        &VARIABLE_REGEX
    }

    // ========== Validation ==========

    /// Detects circular references in the variable definitions
    ///
    /// # Errors
    ///
    /// Returns `VariableError::CircularReference` if a cycle is detected.
    pub fn detect_cycles(&self) -> VariableResult<()> {
        // Check global variables for cycles
        for name in self.global_vars.keys() {
            let mut visited = HashSet::new();
            self.check_cycle_from(name, VariableScope::Global, &mut visited)?;
        }

        // Check connection variables for cycles
        for (conn_id, vars) in &self.connection_vars {
            for name in vars.keys() {
                let mut visited = HashSet::new();
                self.check_cycle_from(name, VariableScope::Connection(*conn_id), &mut visited)?;
            }
        }

        Ok(())
    }

    /// Checks for cycles starting from a specific variable
    fn check_cycle_from(
        &self,
        name: &str,
        scope: VariableScope,
        visited: &mut HashSet<String>,
    ) -> VariableResult<()> {
        if visited.contains(name) {
            return Err(VariableError::CircularReference(name.to_string()));
        }

        if let Some(var) = self.lookup_in_scope_chain(name, scope) {
            let refs = Self::parse_references(&var.value)?;
            if !refs.is_empty() {
                visited.insert(name.to_string());
                for ref_name in refs {
                    self.check_cycle_from(&ref_name, scope, visited)?;
                }
                visited.remove(name);
            }
        }

        Ok(())
    }

    // ========== Command-Safe Substitution ==========

    /// Substitutes variables and validates the result is safe for use as a
    /// command argument.
    ///
    /// This method performs the same substitution as [`substitute`], but
    /// additionally checks that the resolved values do not contain characters
    /// that could cause unexpected behavior when passed as command-line
    /// arguments.
    ///
    /// # Arguments
    ///
    /// * `input` - The string containing variable references
    /// * `scope` - The scope for variable resolution
    ///
    /// # Errors
    ///
    /// Returns `VariableError::UnsafeValue` if any resolved variable contains
    /// null bytes, newlines, or control characters.
    pub fn substitute_for_command(
        &self,
        input: &str,
        scope: VariableScope,
    ) -> VariableResult<String> {
        let refs = Self::parse_references(input)?;
        let mut result = input.to_string();

        for var_name in &refs {
            match self.resolve(var_name, scope) {
                Ok(value) => {
                    Self::validate_command_value(var_name, &value)?;
                    let pattern = format!("${{{var_name}}}");
                    result = result.replace(&pattern, &value);
                }
                Err(VariableError::Undefined(_)) => {
                    let pattern = format!("${{{var_name}}}");
                    result = result.replace(&pattern, "");
                }
                Err(e) => return Err(e),
            }
        }

        Ok(result)
    }

    /// Substitutes only the variables that are defined, leaving unknown
    /// `${...}` references untouched.
    ///
    /// Same command-safety validation as [`Self::substitute_for_command`], but
    /// undefined references are kept verbatim instead of being blanked. This is
    /// what a user-authored shell command needs: `${my_var}` must resolve from
    /// the variable scope chain, while `${HOME}` or `${1}` must survive for the
    /// shell to expand (issue #151).
    ///
    /// # Errors
    ///
    /// Returns `VariableError::UnsafeValue` if a resolved value contains null
    /// bytes, newlines, control characters or shell metacharacters, and
    /// `VariableError::EmptyName` for a malformed reference.
    pub fn substitute_defined_for_command(
        &self,
        input: &str,
        scope: VariableScope,
    ) -> VariableResult<String> {
        let refs = Self::parse_references(input)?;
        let mut result = input.to_string();

        for var_name in &refs {
            match self.resolve(var_name, scope) {
                Ok(value) => {
                    Self::validate_command_value(var_name, &value)?;
                    let pattern = format!("${{{var_name}}}");
                    result = result.replace(&pattern, &value);
                }
                // Undefined: leave the placeholder for the shell to expand.
                Err(VariableError::Undefined(_)) => {}
                Err(e) => return Err(e),
            }
        }

        Ok(result)
    }

    // ========== Terminal-Input Substitution ==========

    /// Substitutes variables in text that will be typed into a terminal.
    ///
    /// The result is written straight to a PTY rather than handed to a shell,
    /// so a shell metacharacter is an ordinary character here and a password
    /// containing `$`, `!` or `;` has to survive intact — which is why this
    /// exists next to [`Self::substitute_for_command`] instead of reusing it
    /// (issue #257). Only what would corrupt the exchange is rejected; see
    /// [`Self::validate_terminal_value`].
    ///
    /// Undefined references are left verbatim and reported in
    /// [`TerminalSubstitution::unresolved`], so a caller can name the missing
    /// variable in a log line instead of silently sending an empty answer.
    ///
    /// # Errors
    ///
    /// Returns `VariableError::UnsafeValue` if a resolved value contains a null
    /// byte, a line break or another control character, plus whatever
    /// [`Self::resolve`] reports for a circular or too deeply nested reference.
    pub fn substitute_for_terminal_input(
        &self,
        input: &str,
        scope: VariableScope,
    ) -> VariableResult<TerminalSubstitution> {
        let refs = Self::parse_references(input)?;
        let mut text = Zeroizing::new(input.to_string());
        let mut unresolved = Vec::new();

        for var_name in &refs {
            match self.resolve(var_name, scope) {
                Ok(value) => {
                    let value = Zeroizing::new(value);
                    Self::validate_terminal_value(var_name, &value)?;
                    let pattern = format!("${{{var_name}}}");
                    let replacement = Zeroizing::new(text.replace(&pattern, &value));
                    text.zeroize();
                    text = replacement;
                }
                // Undefined: keep the placeholder so the caller can report it.
                Err(VariableError::Undefined(_)) => unresolved.push(var_name.clone()),
                Err(e) => return Err(e),
            }
        }

        Ok(TerminalSubstitution { text, unresolved })
    }

    /// Validates that a resolved value can be typed into a terminal.
    ///
    /// Deliberately narrower than [`Self::validate_command_value`]: nothing is
    /// being passed to a shell, so metacharacters are allowed. A line break is
    /// not, because the caller sends the text to a PTY as a single answer and an
    /// embedded newline would submit input the user never wrote — for a
    /// credential prompt that means submitting a truncated secret and then
    /// feeding the remainder to whatever asks next. Tab is kept: it is what a
    /// field-by-field login sequence uses.
    ///
    /// # Errors
    ///
    /// Returns `VariableError::UnsafeValue` if the value contains a null byte,
    /// `\n`, `\r`, or another control character.
    fn validate_terminal_value(name: &str, value: &str) -> VariableResult<()> {
        if value.contains('\0') {
            return Err(VariableError::UnsafeValue {
                name: name.to_string(),
                reason: "contains null byte".to_string(),
            });
        }

        // Check newlines *before* the general control-character test so the
        // error message says "contains newline characters" — a far more
        // actionable diagnostic for someone who pasted a multi-line password —
        // rather than the generic "contains control characters". Both `\n` and
        // `\r` are control chars, so without this early return the generic arm
        // below would catch them with a less helpful message.
        if value.contains('\n') || value.contains('\r') {
            return Err(VariableError::UnsafeValue {
                name: name.to_string(),
                reason: "contains newline characters".to_string(),
            });
        }

        if value.chars().any(|c| c.is_control() && c != '\t') {
            return Err(VariableError::UnsafeValue {
                name: name.to_string(),
                reason: "contains control characters".to_string(),
            });
        }

        Ok(())
    }

    /// Validates that a resolved variable value is safe for command arguments.
    ///
    /// Rejects values containing null bytes, newlines, carriage returns, and
    /// other control characters (except tab, which is allowed).
    ///
    /// # Errors
    ///
    /// Returns `VariableError::UnsafeValue` if the value contains unsafe
    /// characters.
    fn validate_command_value(name: &str, value: &str) -> VariableResult<()> {
        // Shell metacharacters that could enable injection via `sh -c`
        const SHELL_META: &[char] = &[';', '|', '&', '`', '$', '(', ')', '<', '>', '!'];

        if value.contains('\0') {
            return Err(VariableError::UnsafeValue {
                name: name.to_string(),
                reason: "contains null byte".to_string(),
            });
        }

        if value.contains('\n') || value.contains('\r') {
            return Err(VariableError::UnsafeValue {
                name: name.to_string(),
                reason: "contains newline characters".to_string(),
            });
        }

        // Reject control characters (ASCII 0x00–0x1F) except tab (0x09)
        if value.chars().any(|c| c.is_control() && c != '\t') {
            return Err(VariableError::UnsafeValue {
                name: name.to_string(),
                reason: "contains control characters".to_string(),
            });
        }

        // Reject shell metacharacters to prevent injection
        if value.chars().any(|c| SHELL_META.contains(&c)) {
            return Err(VariableError::UnsafeValue {
                name: name.to_string(),
                reason: "contains shell metacharacters".to_string(),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_manager() -> VariableManager {
        let mut manager = VariableManager::new();

        // Set up global variables
        manager.set_global(Variable::new("global_var", "global_value"));
        manager.set_global(Variable::new("user", "admin"));
        manager.set_global(Variable::new("host", "example.com"));

        manager
    }

    #[test]
    fn test_resolve_global_variable() {
        let manager = create_test_manager();

        let result = manager
            .resolve("global_var", VariableScope::Global)
            .unwrap();
        assert_eq!(result, "global_value");
    }

    #[test]
    fn test_resolve_undefined_variable() {
        let manager = create_test_manager();

        let result = manager.resolve("undefined", VariableScope::Global);
        assert!(matches!(result, Err(VariableError::Undefined(_))));
    }

    #[test]
    fn test_resolve_with_connection_scope() {
        let mut manager = create_test_manager();
        let conn_id = Uuid::new_v4();

        // Connection variable overrides global
        manager.set_connection(conn_id, Variable::new("user", "conn_user"));

        // Connection scope should return connection variable
        let result = manager
            .resolve("user", VariableScope::Connection(conn_id))
            .unwrap();
        assert_eq!(result, "conn_user");

        // Global variable should still be accessible through the chain
        let result = manager
            .resolve("host", VariableScope::Connection(conn_id))
            .unwrap();
        assert_eq!(result, "example.com");
    }

    #[test]
    fn test_substitute_simple() {
        let manager = create_test_manager();

        let result = manager
            .substitute("ssh ${user}@${host}", VariableScope::Global)
            .unwrap();
        assert_eq!(result, "ssh admin@example.com");
    }

    #[test]
    fn test_substitute_undefined_uses_empty() {
        let manager = create_test_manager();

        let result = manager
            .substitute("value: ${undefined}", VariableScope::Global)
            .unwrap();
        assert_eq!(result, "value: ");
    }

    #[test]
    fn test_substitute_no_variables() {
        let manager = create_test_manager();

        let result = manager
            .substitute("plain text", VariableScope::Global)
            .unwrap();
        assert_eq!(result, "plain text");
    }

    // ===== Terminal-input substitution (issue #257) =====

    #[test]
    fn terminal_input_keeps_shell_metacharacters_in_a_password() {
        // The value goes to a PTY, not to `sh -c`, so every one of the
        // characters `validate_command_value` rejects must survive.
        let mut manager = VariableManager::new();
        manager.set_global(Variable::new("password", r"a;b|c&d`e$f(g)h<i>j!k"));

        let out = manager
            .substitute_for_terminal_input("${password}\n", VariableScope::Global)
            .unwrap();

        assert_eq!(out.text.as_str(), "a;b|c&d`e$f(g)h<i>j!k\n");
        assert!(out.unresolved.is_empty());
    }

    #[test]
    fn terminal_input_rejects_a_value_with_a_line_break() {
        // A newline inside the value would submit the answer early.
        let mut manager = VariableManager::new();
        manager.set_global(Variable::new("password", "first\nsecond"));

        let err = manager
            .substitute_for_terminal_input("${password}\n", VariableScope::Global)
            .err()
            .expect("line break must be rejected");

        assert!(matches!(err, VariableError::UnsafeValue { .. }), "{err:?}");
    }

    #[test]
    fn terminal_input_reports_undefined_names_and_keeps_the_placeholder() {
        let manager = create_test_manager();

        let out = manager
            .substitute_for_terminal_input("${password}\n", VariableScope::Global)
            .unwrap();

        // Not blanked: the caller has to be able to say what is missing.
        assert_eq!(out.text.as_str(), "${password}\n");
        assert_eq!(out.unresolved, vec!["password".to_string()]);
    }

    #[test]
    fn terminal_input_resolves_a_builtin_from_the_connection_scope() {
        // How the automation path supplies ${password}: a connection-scoped
        // built-in shadows a same-named global for that connection only.
        let conn_id = Uuid::new_v4();
        let mut manager = VariableManager::new();
        manager.set_global(Variable::new("password", "global-value"));
        manager.set_connection(conn_id, Variable::new_secret("password", "conn-value"));

        let out = manager
            .substitute_for_terminal_input("${password}\n", VariableScope::Connection(conn_id))
            .unwrap();
        assert_eq!(out.text.as_str(), "conn-value\n");

        let out = manager
            .substitute_for_terminal_input("${password}\n", VariableScope::Global)
            .unwrap();
        assert_eq!(out.text.as_str(), "global-value\n");
    }

    #[test]
    fn terminal_input_leaves_text_without_references_alone() {
        let manager = create_test_manager();

        let out = manager
            .substitute_for_terminal_input("yes\n", VariableScope::Global)
            .unwrap();

        assert_eq!(out.text.as_str(), "yes\n");
        assert!(out.unresolved.is_empty());
    }

    #[test]
    fn terminal_input_allows_tab_but_not_other_control_characters() {
        let mut manager = VariableManager::new();
        manager.set_global(Variable::new("user", "ad\tmin"));
        assert!(
            manager
                .substitute_for_terminal_input("${user}", VariableScope::Global)
                .is_ok()
        );

        manager.set_global(Variable::new("user", "ad\u{7}min"));
        assert!(
            manager
                .substitute_for_terminal_input("${user}", VariableScope::Global)
                .is_err()
        );
    }

    #[test]
    fn test_parse_references_simple() {
        let refs = VariableManager::parse_references("ssh ${user}@${host}").unwrap();
        assert_eq!(refs.len(), 2);
        assert!(refs.contains(&"user".to_string()));
        assert!(refs.contains(&"host".to_string()));
    }

    #[test]
    fn test_parse_references_duplicates() {
        let refs = VariableManager::parse_references("${var} and ${var}").unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0], "var");
    }

    #[test]
    fn test_parse_references_no_variables() {
        let refs = VariableManager::parse_references("plain text").unwrap();
        assert!(refs.is_empty());
    }

    #[test]
    fn test_parse_references_invalid_format() {
        // These should not be parsed as variables
        let refs = VariableManager::parse_references("$var ${} ${123}").unwrap();
        assert!(refs.is_empty());
    }

    #[test]
    fn test_variable_management() {
        let mut manager = VariableManager::new();
        let conn_id = Uuid::new_v4();

        // Test set and get
        manager.set_global(Variable::new("g1", "v1"));
        manager.set_connection(conn_id, Variable::new("c1", "v3"));

        assert_eq!(manager.get_global("g1").unwrap().value, "v1");
        assert_eq!(manager.get_connection(conn_id, "c1").unwrap().value, "v3");

        // Test list
        assert_eq!(manager.list_global().len(), 1);
        assert_eq!(manager.list_connection(conn_id).len(), 1);

        // Test remove
        manager.remove_global("g1");
        assert!(manager.get_global("g1").is_none());
    }

    #[test]
    fn test_nested_variable_resolution() {
        let mut manager = VariableManager::new();

        // Set up nested variables: greeting -> ${salutation} ${name}
        manager.set_global(Variable::new("name", "World"));
        manager.set_global(Variable::new("salutation", "Hello"));
        manager.set_global(Variable::new("greeting", "${salutation}, ${name}!"));

        let result = manager.resolve("greeting", VariableScope::Global).unwrap();
        assert_eq!(result, "Hello, World!");
    }

    #[test]
    fn test_deeply_nested_resolution() {
        let mut manager = VariableManager::new();

        // Create a chain: a -> ${b} -> ${c} -> ${d} -> value
        manager.set_global(Variable::new("d", "final_value"));
        manager.set_global(Variable::new("c", "${d}"));
        manager.set_global(Variable::new("b", "${c}"));
        manager.set_global(Variable::new("a", "${b}"));

        let result = manager.resolve("a", VariableScope::Global).unwrap();
        assert_eq!(result, "final_value");
    }

    #[test]
    fn test_circular_reference_detection() {
        let mut manager = VariableManager::new();

        // Create a cycle: a -> ${b} -> ${a}
        manager.set_global(Variable::new("a", "${b}"));
        manager.set_global(Variable::new("b", "${a}"));

        let result = manager.resolve("a", VariableScope::Global);
        assert!(matches!(result, Err(VariableError::CircularReference(_))));
    }

    #[test]
    fn test_self_reference_detection() {
        let mut manager = VariableManager::new();

        // Self-reference: a -> ${a}
        manager.set_global(Variable::new("a", "${a}"));

        let result = manager.resolve("a", VariableScope::Global);
        assert!(matches!(result, Err(VariableError::CircularReference(_))));
    }

    #[test]
    fn test_detect_cycles_method() {
        let mut manager = VariableManager::new();

        // No cycles
        manager.set_global(Variable::new("a", "value"));
        manager.set_global(Variable::new("b", "${a}"));
        assert!(manager.detect_cycles().is_ok());

        // Add a cycle
        manager.set_global(Variable::new("a", "${b}"));
        assert!(manager.detect_cycles().is_err());
    }

    #[test]
    fn test_max_depth_exceeded() {
        let mut manager = VariableManager::new();

        // Create a chain longer than MAX_NESTING_DEPTH
        for i in 0..=super::MAX_NESTING_DEPTH + 2 {
            let name = format!("var{i}");
            let value = if i == 0 {
                "final".to_string()
            } else {
                format!("${{var{}}}", i - 1)
            };
            manager.set_global(Variable::new(name, value));
        }

        let result = manager.resolve(
            &format!("var{}", super::MAX_NESTING_DEPTH + 2),
            VariableScope::Global,
        );
        assert!(matches!(result, Err(VariableError::MaxDepthExceeded(_))));
    }

    #[test]
    fn test_substitute_for_command_safe_value() {
        let manager = create_test_manager();
        let result = manager
            .substitute_for_command("ssh ${user}@${host}", VariableScope::Global)
            .unwrap();
        assert_eq!(result, "ssh admin@example.com");
    }

    #[test]
    fn test_substitute_for_command_rejects_null_byte() {
        let mut manager = VariableManager::new();
        manager.set_global(Variable::new("evil", "value\0injected"));

        let result = manager.substitute_for_command("${evil}", VariableScope::Global);
        assert!(matches!(result, Err(VariableError::UnsafeValue { .. })));
    }

    #[test]
    fn test_substitute_for_command_rejects_newline() {
        let mut manager = VariableManager::new();
        manager.set_global(Variable::new("evil", "value\ninjected"));

        let result = manager.substitute_for_command("${evil}", VariableScope::Global);
        assert!(matches!(result, Err(VariableError::UnsafeValue { .. })));
    }

    #[test]
    fn test_substitute_for_command_rejects_carriage_return() {
        let mut manager = VariableManager::new();
        manager.set_global(Variable::new("evil", "value\rinjected"));

        let result = manager.substitute_for_command("${evil}", VariableScope::Global);
        assert!(matches!(result, Err(VariableError::UnsafeValue { .. })));
    }

    #[test]
    fn test_substitute_for_command_rejects_control_chars() {
        let mut manager = VariableManager::new();
        manager.set_global(Variable::new("evil", "value\x07bell"));

        let result = manager.substitute_for_command("${evil}", VariableScope::Global);
        assert!(matches!(result, Err(VariableError::UnsafeValue { .. })));
    }

    #[test]
    fn test_substitute_for_command_allows_tab() {
        let mut manager = VariableManager::new();
        manager.set_global(Variable::new("with_tab", "value\twith_tab"));

        let result = manager
            .substitute_for_command("${with_tab}", VariableScope::Global)
            .unwrap();
        assert_eq!(result, "value\twith_tab");
    }

    /// Issue #151: a Custom Command template like `rustdesk --connect ${id}`
    /// must pick up the connection-local variable `id`.
    #[test]
    fn test_substitute_defined_for_command_resolves_connection_variable() {
        let conn_id = Uuid::new_v4();
        let mut manager = VariableManager::new();
        manager.set_connection(conn_id, Variable::new("id", "123456789"));

        let result = manager
            .substitute_defined_for_command(
                "rustdesk --connect ${id}",
                VariableScope::Connection(conn_id),
            )
            .unwrap();
        assert_eq!(result, "rustdesk --connect 123456789");
    }

    /// Undefined references stay literal so the shell can still expand them
    /// (`${HOME}`), unlike `substitute_for_command` which blanks them.
    #[test]
    fn test_substitute_defined_for_command_keeps_unknown_placeholders() {
        let manager = create_test_manager();
        let result = manager
            .substitute_defined_for_command("cp ${HOME}/f ${host}:/tmp", VariableScope::Global)
            .unwrap();
        assert_eq!(result, "cp ${HOME}/f example.com:/tmp");
    }

    #[test]
    fn test_substitute_defined_for_command_rejects_shell_metacharacters() {
        let mut manager = VariableManager::new();
        manager.set_global(Variable::new("evil", "1; rm -rf /"));

        let result = manager.substitute_defined_for_command("echo ${evil}", VariableScope::Global);
        assert!(matches!(result, Err(VariableError::UnsafeValue { .. })));
    }

    #[test]
    fn test_substitute_for_command_rejects_shell_metacharacters() {
        let mut manager = VariableManager::new();
        manager.set_global(Variable::new(
            "complex",
            "user@host:port/path?query=1&other=2",
        ));

        let result = manager.substitute_for_command("${complex}", VariableScope::Global);
        assert!(result.is_err(), "should reject value containing '&'");
    }

    #[test]
    fn test_substitute_for_command_allows_safe_special_chars() {
        let mut manager = VariableManager::new();
        manager.set_global(Variable::new("safe", "user@host:port/path?query=1"));

        let result = manager
            .substitute_for_command("${safe}", VariableScope::Global)
            .unwrap();
        assert_eq!(result, "user@host:port/path?query=1");
    }

    #[test]
    fn test_substitute_for_command_undefined_uses_empty() {
        let manager = VariableManager::new();
        let result = manager
            .substitute_for_command("prefix_${undefined}_suffix", VariableScope::Global)
            .unwrap();
        assert_eq!(result, "prefix__suffix");
    }

    // ===== Built-in dynamic variables (date/time + ENV_*) =====

    #[test]
    fn builtin_date_and_time_resolve_to_well_formed_values() {
        let manager = VariableManager::new();
        let out = manager
            .substitute(
                "${DATE_Y}-${DATE_M}-${DATE_D} ${TIME_H}:${TIME_M}:${TIME_S}",
                VariableScope::Global,
            )
            .unwrap();

        // Shape check rather than exact value — the clock moves. Year is 4
        // digits, every other field is exactly 2, joined by the literals.
        let re = regex::Regex::new(r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$").unwrap();
        assert!(re.is_match(&out), "unexpected date/time render: {out}");
    }

    #[test]
    fn builtin_timestamp_is_a_positive_epoch_second_count() {
        let manager = VariableManager::new();
        let out = manager
            .substitute("${TIMESTAMP}", VariableScope::Global)
            .unwrap();
        let secs: i64 = out.parse().expect("TIMESTAMP must be an integer");
        // Any real clock is well past the 2020 epoch second.
        assert!(secs > 1_577_836_800, "timestamp too small: {secs}");
    }

    #[test]
    fn builtin_env_resolves_a_set_variable() {
        // SAFETY of the test: reads only, never mutates process env.
        // PATH is always set in any environment that runs the test suite.
        let expected = std::env::var("PATH").expect("PATH is set during tests");
        let manager = VariableManager::new();
        let out = manager
            .substitute("${ENV_PATH}", VariableScope::Global)
            .unwrap();
        assert_eq!(out, expected);
    }

    #[test]
    fn builtin_env_of_an_unset_variable_is_empty_not_literal() {
        let manager = VariableManager::new();
        let out = manager
            .substitute(
                "[${ENV_RUSTCONN_DEFINITELY_UNSET_XYZ}]",
                VariableScope::Global,
            )
            .unwrap();
        // Resolves to empty (shell-like), not left as the literal placeholder.
        assert_eq!(out, "[]");
    }

    #[test]
    fn a_user_variable_shadows_a_builtin_of_the_same_name() {
        let mut manager = VariableManager::new();
        manager.set_global(Variable::new("TIMESTAMP", "user-wins"));
        let out = manager
            .substitute("${TIMESTAMP}", VariableScope::Global)
            .unwrap();
        assert_eq!(out, "user-wins");
    }

    #[test]
    fn an_empty_env_suffix_is_not_a_builtin() {
        // `${ENV_}` is not a real environment reference — it stays undefined
        // and is blanked by substitute() like any other unknown name.
        let manager = VariableManager::new();
        assert!(matches!(
            manager.resolve("ENV_", VariableScope::Global),
            Err(VariableError::Undefined(_))
        ));
    }

    #[test]
    fn builtins_resolve_in_command_and_terminal_paths_too() {
        let manager = VariableManager::new();

        // Command path: a date is metacharacter-free, so it validates.
        let cmd = manager
            .substitute_for_command("log-${DATE_Y}.txt", VariableScope::Global)
            .unwrap();
        assert!(
            regex::Regex::new(r"^log-\d{4}\.txt$")
                .unwrap()
                .is_match(&cmd)
        );

        // Terminal-input path: a bare timestamp typed into a PTY.
        let term = manager
            .substitute_for_terminal_input("${TIMESTAMP}", VariableScope::Global)
            .unwrap();
        assert!(term.unresolved.is_empty());
        assert!(term.text.as_str().chars().all(|c| c.is_ascii_digit()));
    }
}
