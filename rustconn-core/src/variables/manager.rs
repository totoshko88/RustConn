//! Variable manager for resolution and substitution
//!
//! This module provides the `VariableManager` which handles variable storage,
//! resolution across scopes, and substitution in strings.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use uuid::Uuid;

use super::{Variable, VariableError, VariableResult, VariableScope, MAX_NESTING_DEPTH};

/// Cached regex for variable extraction: matches `${var_name}` patterns
static VARIABLE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\$\{([a-zA-Z_][a-zA-Z0-9_]*)\}").expect("VARIABLE_REGEX is a valid regex pattern")
});

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
    /// Document-scoped variables indexed by document ID
    document_vars: HashMap<Uuid, HashMap<String, Variable>>,
    /// Connection-scoped variables indexed by connection ID
    connection_vars: HashMap<Uuid, HashMap<String, Variable>>,
    /// Mapping from connection ID to document ID for scope chain resolution
    connection_to_document: HashMap<Uuid, Uuid>,
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

    /// Sets a document-scoped variable
    pub fn set_document(&mut self, document_id: Uuid, variable: Variable) {
        self.document_vars
            .entry(document_id)
            .or_default()
            .insert(variable.name.clone(), variable);
    }

    /// Sets a connection-scoped variable
    pub fn set_connection(&mut self, connection_id: Uuid, variable: Variable) {
        self.connection_vars
            .entry(connection_id)
            .or_default()
            .insert(variable.name.clone(), variable);
    }

    /// Associates a connection with a document for scope chain resolution
    pub fn set_connection_document(&mut self, connection_id: Uuid, document_id: Uuid) {
        self.connection_to_document
            .insert(connection_id, document_id);
    }

    /// Gets a global variable by name
    #[must_use]
    pub fn get_global(&self, name: &str) -> Option<&Variable> {
        self.global_vars.get(name)
    }

    /// Gets a document-scoped variable by name
    #[must_use]
    pub fn get_document(&self, document_id: Uuid, name: &str) -> Option<&Variable> {
        self.document_vars
            .get(&document_id)
            .and_then(|vars| vars.get(name))
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

    /// Removes a document-scoped variable
    pub fn remove_document(&mut self, document_id: Uuid, name: &str) -> Option<Variable> {
        self.document_vars
            .get_mut(&document_id)
            .and_then(|vars| vars.remove(name))
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

    /// Lists all document-scoped variables
    #[must_use]
    pub fn list_document(&self, document_id: Uuid) -> Vec<&Variable> {
        self.document_vars
            .get(&document_id)
            .map(|vars| vars.values().collect())
            .unwrap_or_default()
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
    /// - For `Connection` scope: Connection -> Document -> Global
    /// - For `Document` scope: Document -> Global
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
            None => Err(VariableError::Undefined(name.to_string())),
        }
    }

    /// Looks up a variable in the scope chain
    fn lookup_in_scope_chain(&self, name: &str, scope: VariableScope) -> Option<&Variable> {
        match scope {
            VariableScope::Global => self.global_vars.get(name),
            VariableScope::Document(doc_id) => self
                .document_vars
                .get(&doc_id)
                .and_then(|vars| vars.get(name))
                .or_else(|| self.global_vars.get(name)),
            VariableScope::Connection(conn_id) => {
                // First check connection scope
                if let Some(var) = self
                    .connection_vars
                    .get(&conn_id)
                    .and_then(|vars| vars.get(name))
                {
                    return Some(var);
                }

                // Then check document scope if connection is associated with a document
                if let Some(doc_id) = self.connection_to_document.get(&conn_id) {
                    if let Some(var) = self
                        .document_vars
                        .get(doc_id)
                        .and_then(|vars| vars.get(name))
                    {
                        return Some(var);
                    }
                }

                // Finally check global scope
                self.global_vars.get(name)
            }
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
            eprintln!("Warning: Undefined variable: {var}");
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

        // Check document variables for cycles
        for (doc_id, vars) in &self.document_vars {
            for name in vars.keys() {
                let mut visited = HashSet::new();
                self.check_cycle_from(name, VariableScope::Document(*doc_id), &mut visited)?;
            }
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
    fn test_resolve_with_document_scope() {
        let mut manager = create_test_manager();
        let doc_id = Uuid::new_v4();

        // Document variable overrides global
        manager.set_document(doc_id, Variable::new("user", "doc_user"));

        // Document scope should return document variable
        let result = manager
            .resolve("user", VariableScope::Document(doc_id))
            .unwrap();
        assert_eq!(result, "doc_user");

        // Global variable should still be accessible
        let result = manager
            .resolve("host", VariableScope::Document(doc_id))
            .unwrap();
        assert_eq!(result, "example.com");
    }

    #[test]
    fn test_resolve_with_connection_scope() {
        let mut manager = create_test_manager();
        let doc_id = Uuid::new_v4();
        let conn_id = Uuid::new_v4();

        // Set up scope chain
        manager.set_document(doc_id, Variable::new("user", "doc_user"));
        manager.set_connection(conn_id, Variable::new("user", "conn_user"));
        manager.set_connection_document(conn_id, doc_id);

        // Connection scope should return connection variable
        let result = manager
            .resolve("user", VariableScope::Connection(conn_id))
            .unwrap();
        assert_eq!(result, "conn_user");

        // Global variable should still be accessible through chain
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
        let doc_id = Uuid::new_v4();
        let conn_id = Uuid::new_v4();

        // Test set and get
        manager.set_global(Variable::new("g1", "v1"));
        manager.set_document(doc_id, Variable::new("d1", "v2"));
        manager.set_connection(conn_id, Variable::new("c1", "v3"));

        assert_eq!(manager.get_global("g1").unwrap().value, "v1");
        assert_eq!(manager.get_document(doc_id, "d1").unwrap().value, "v2");
        assert_eq!(manager.get_connection(conn_id, "c1").unwrap().value, "v3");

        // Test list
        assert_eq!(manager.list_global().len(), 1);
        assert_eq!(manager.list_document(doc_id).len(), 1);
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
}
