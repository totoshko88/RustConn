//! Connection tasks for pre/post connection automation
//!
//! This module provides the connection task system for executing commands
//! before connecting and after disconnecting. It supports:
//! - Pre-connect tasks (e.g., VPN setup, tunnel creation)
//! - Post-disconnect tasks (e.g., cleanup, logging)
//! - Conditional execution based on folder connection state
//! - Variable substitution in command strings

use std::collections::HashMap;
use std::process::ExitStatus;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::process::Command;
use uuid::Uuid;

use crate::variables::{VariableManager, VariableScope};

/// Errors that can occur during task operations
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TaskError {
    /// Task command failed with non-zero exit code
    #[error("Task failed with exit code {0}")]
    NonZeroExit(i32),

    /// Task command was terminated by signal
    #[error("Task terminated by signal")]
    Terminated,

    /// Task execution failed
    #[error("Task execution failed: {0}")]
    ExecutionFailed(String),

    /// Variable substitution error
    #[error("Variable error: {0}")]
    VariableError(String),

    /// Task timeout exceeded
    #[error("Task timed out after {0}ms")]
    Timeout(u32),

    /// Invalid task configuration
    #[error("Invalid task configuration: {0}")]
    InvalidConfig(String),

    /// I/O error during task execution
    #[error("I/O error: {0}")]
    IoError(String),
}

/// Result type for task operations
pub type TaskResult<T> = std::result::Result<T, TaskError>;

/// Task execution timing
///
/// Defines when a task should be executed relative to the connection lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskTiming {
    /// Execute before establishing the connection
    #[default]
    PreConnect,
    /// Execute after the connection is terminated
    PostDisconnect,
}

impl TaskTiming {
    /// Returns a human-readable description of the timing
    #[must_use]
    pub const fn description(&self) -> &'static str {
        match self {
            Self::PreConnect => "Pre-connect",
            Self::PostDisconnect => "Post-disconnect",
        }
    }
}

/// Task execution condition
///
/// Defines conditions for when a task should be executed, particularly
/// useful for folder-level tasks that should only run for the first
/// or last connection in a folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct TaskCondition {
    /// Execute only when this is the first active connection in the folder
    #[serde(default)]
    pub only_first_in_folder: bool,
    /// Execute only when this is the last active connection in the folder
    #[serde(default)]
    pub only_last_in_folder: bool,
}

impl TaskCondition {
    /// Creates a new task condition with no restrictions
    #[must_use]
    pub const fn new() -> Self {
        Self {
            only_first_in_folder: false,
            only_last_in_folder: false,
        }
    }

    /// Creates a condition that only executes for the first connection in folder
    #[must_use]
    pub const fn first_in_folder() -> Self {
        Self {
            only_first_in_folder: true,
            only_last_in_folder: false,
        }
    }

    /// Creates a condition that only executes for the last connection in folder
    #[must_use]
    pub const fn last_in_folder() -> Self {
        Self {
            only_first_in_folder: false,
            only_last_in_folder: true,
        }
    }

    /// Returns true if this condition has any restrictions
    #[must_use]
    pub const fn has_restrictions(&self) -> bool {
        self.only_first_in_folder || self.only_last_in_folder
    }

    /// Checks if the task should execute given the folder connection state
    ///
    /// # Arguments
    /// * `is_first` - Whether this is the first active connection in the folder
    /// * `is_last` - Whether this is the last active connection in the folder
    #[must_use]
    pub const fn should_execute(&self, is_first: bool, is_last: bool) -> bool {
        // If no restrictions, always execute
        if !self.only_first_in_folder && !self.only_last_in_folder {
            return true;
        }

        // Check first-in-folder condition
        if self.only_first_in_folder && !is_first {
            return false;
        }

        // Check last-in-folder condition
        if self.only_last_in_folder && !is_last {
            return false;
        }

        true
    }
}

/// A connection task definition
///
/// Connection tasks are commands that can be executed before connecting
/// or after disconnecting. They support variable substitution and
/// conditional execution based on folder state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionTask {
    /// Unique identifier for this task
    pub id: Uuid,
    /// When to execute this task
    pub timing: TaskTiming,
    /// The command to execute (supports variable substitution)
    pub command: String,
    /// Execution conditions
    #[serde(default)]
    pub condition: TaskCondition,
    /// Optional timeout in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u32>,
    /// Whether to abort the connection if this task fails (pre-connect only)
    #[serde(default = "default_abort_on_failure")]
    pub abort_on_failure: bool,
    /// Optional description for documentation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Default value for `abort_on_failure` (true for pre-connect tasks)
const fn default_abort_on_failure() -> bool {
    true
}

impl ConnectionTask {
    /// Creates a new pre-connect task with the given command
    #[must_use]
    pub fn new_pre_connect(command: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            timing: TaskTiming::PreConnect,
            command: command.into(),
            condition: TaskCondition::new(),
            timeout_ms: None,
            abort_on_failure: true,
            description: None,
        }
    }

    /// Creates a new post-disconnect task with the given command
    #[must_use]
    pub fn new_post_disconnect(command: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            timing: TaskTiming::PostDisconnect,
            command: command.into(),
            condition: TaskCondition::new(),
            timeout_ms: None,
            abort_on_failure: false,
            description: None,
        }
    }

    /// Creates a new task with a specific ID
    #[must_use]
    pub fn with_id(id: Uuid, timing: TaskTiming, command: impl Into<String>) -> Self {
        Self {
            id,
            timing,
            command: command.into(),
            condition: TaskCondition::new(),
            timeout_ms: None,
            abort_on_failure: matches!(timing, TaskTiming::PreConnect),
            description: None,
        }
    }

    /// Sets the execution condition for this task
    #[must_use]
    pub const fn with_condition(mut self, condition: TaskCondition) -> Self {
        self.condition = condition;
        self
    }

    /// Sets the timeout for this task
    #[must_use]
    pub const fn with_timeout(mut self, timeout_ms: u32) -> Self {
        self.timeout_ms = Some(timeout_ms);
        self
    }

    /// Sets whether to abort on failure
    #[must_use]
    pub const fn with_abort_on_failure(mut self, abort: bool) -> Self {
        self.abort_on_failure = abort;
        self
    }

    /// Sets the description for this task
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Returns true if this is a pre-connect task
    #[must_use]
    pub const fn is_pre_connect(&self) -> bool {
        matches!(self.timing, TaskTiming::PreConnect)
    }

    /// Returns true if this is a post-disconnect task
    #[must_use]
    pub const fn is_post_disconnect(&self) -> bool {
        matches!(self.timing, TaskTiming::PostDisconnect)
    }

    /// Substitutes variables in the command string
    ///
    /// # Errors
    ///
    /// Returns an error if variable substitution fails.
    pub fn substitute_command(
        &self,
        manager: &VariableManager,
        scope: VariableScope,
    ) -> TaskResult<String> {
        manager
            .substitute(&self.command, scope)
            .map_err(|e| TaskError::VariableError(e.to_string()))
    }

    /// Substitutes variables using an Arc-wrapped manager
    ///
    /// # Errors
    ///
    /// Returns an error if variable substitution fails.
    pub fn substitute_command_arc(
        &self,
        manager: &Arc<VariableManager>,
        scope: VariableScope,
    ) -> TaskResult<String> {
        self.substitute_command(manager.as_ref(), scope)
    }
}

/// Tracks active connections per folder for conditional task execution
#[derive(Debug, Clone, Default)]
pub struct FolderConnectionTracker {
    /// Map of folder ID to count of active connections
    active_counts: HashMap<Uuid, usize>,
}

impl FolderConnectionTracker {
    /// Creates a new empty tracker
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a connection being opened in a folder
    ///
    /// Returns true if this is the first connection in the folder
    pub fn connection_opened(&mut self, folder_id: Option<Uuid>) -> bool {
        let folder_id = folder_id.unwrap_or(Uuid::nil());
        let count = self.active_counts.entry(folder_id).or_insert(0);
        let is_first = *count == 0;
        *count += 1;
        is_first
    }

    /// Records a connection being closed in a folder
    ///
    /// Returns true if this was the last connection in the folder
    pub fn connection_closed(&mut self, folder_id: Option<Uuid>) -> bool {
        let folder_id = folder_id.unwrap_or(Uuid::nil());
        let count = self.active_counts.entry(folder_id).or_insert(0);
        if *count > 0 {
            *count -= 1;
        }
        *count == 0
    }

    /// Returns the number of active connections in a folder
    #[must_use]
    pub fn active_count(&self, folder_id: Option<Uuid>) -> usize {
        let folder_id = folder_id.unwrap_or(Uuid::nil());
        self.active_counts.get(&folder_id).copied().unwrap_or(0)
    }

    /// Returns true if there are any active connections in the folder
    #[must_use]
    pub fn has_active_connections(&self, folder_id: Option<Uuid>) -> bool {
        self.active_count(folder_id) > 0
    }

    /// Clears all tracking data
    pub fn clear(&mut self) {
        self.active_counts.clear();
    }
}

/// Task executor for running connection tasks
///
/// The executor handles variable substitution, command execution,
/// and failure handling for connection tasks.
#[derive(Debug, Clone)]
pub struct TaskExecutor {
    /// Variable manager for substitution
    variable_manager: Arc<VariableManager>,
    /// Folder connection tracker for conditional execution
    folder_tracker: Arc<std::sync::Mutex<FolderConnectionTracker>>,
}

impl TaskExecutor {
    /// Creates a new task executor with the given variable manager
    #[must_use]
    pub fn new(variable_manager: Arc<VariableManager>) -> Self {
        Self {
            variable_manager,
            folder_tracker: Arc::new(std::sync::Mutex::new(FolderConnectionTracker::new())),
        }
    }

    /// Creates a new task executor with a custom folder tracker
    #[must_use]
    pub const fn with_tracker(
        variable_manager: Arc<VariableManager>,
        folder_tracker: Arc<std::sync::Mutex<FolderConnectionTracker>>,
    ) -> Self {
        Self {
            variable_manager,
            folder_tracker,
        }
    }

    /// Returns a reference to the folder tracker
    #[must_use]
    pub const fn folder_tracker(&self) -> &Arc<std::sync::Mutex<FolderConnectionTracker>> {
        &self.folder_tracker
    }

    /// Checks if a task should execute based on its conditions
    const fn should_execute_task(task: &ConnectionTask, is_first: bool, is_last: bool) -> bool {
        // If no folder-based conditions, always execute
        if !task.condition.has_restrictions() {
            return true;
        }

        // For pre-connect tasks, check first-in-folder
        if task.is_pre_connect() && task.condition.only_first_in_folder {
            return is_first;
        }

        // For post-disconnect tasks, check last-in-folder
        if task.is_post_disconnect() && task.condition.only_last_in_folder {
            return is_last;
        }

        // Use the general condition check
        task.condition.should_execute(is_first, is_last)
    }

    /// Executes a task with variable substitution
    ///
    /// # Arguments
    /// * `task` - The task to execute
    /// * `scope` - The variable scope for substitution
    /// * `folder_id` - Optional folder ID for conditional execution
    /// * `is_first` - Whether this is the first connection in the folder
    /// * `is_last` - Whether this is the last connection in the folder
    ///
    /// # Returns
    /// The exit code of the command, or an error if execution failed
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Variable substitution fails
    /// - Command execution fails
    /// - Command returns non-zero exit code (for pre-connect with `abort_on_failure`)
    /// - Command times out
    pub async fn execute(
        &self,
        task: &ConnectionTask,
        scope: VariableScope,
        _folder_id: Option<Uuid>,
        is_first: bool,
        is_last: bool,
    ) -> TaskResult<i32> {
        // Check if task should execute based on conditions
        if !Self::should_execute_task(task, is_first, is_last) {
            return Ok(0); // Skip execution, return success
        }

        // Substitute variables in command
        let command = task.substitute_command_arc(&self.variable_manager, scope)?;

        // Execute the command
        self.execute_command(&command, task.timeout_ms, task.abort_on_failure)
            .await
    }

    /// Executes a command string and returns the exit code
    async fn execute_command(
        &self,
        command: &str,
        timeout_ms: Option<u32>,
        abort_on_failure: bool,
    ) -> TaskResult<i32> {
        // Use sh -c to execute the command string
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command);

        // Spawn the process
        let child = cmd.spawn().map_err(|e| TaskError::IoError(e.to_string()))?;

        // Wait for completion with optional timeout
        let result = if let Some(timeout) = timeout_ms {
            let timeout_duration = std::time::Duration::from_millis(u64::from(timeout));
            match tokio::time::timeout(timeout_duration, child.wait_with_output()).await {
                Ok(output_result) => output_result,
                Err(_) => return Err(TaskError::Timeout(timeout)),
            }
        } else {
            child.wait_with_output().await
        };

        let output = result.map_err(|e| TaskError::IoError(e.to_string()))?;

        // Check exit status
        let exit_code = Self::exit_code_from_status(output.status)?;

        // Handle non-zero exit
        if exit_code != 0 && abort_on_failure {
            return Err(TaskError::NonZeroExit(exit_code));
        }

        Ok(exit_code)
    }

    /// Extracts exit code from process status
    fn exit_code_from_status(status: ExitStatus) -> TaskResult<i32> {
        status.code().ok_or(TaskError::Terminated)
    }

    /// Executes a pre-connect task
    ///
    /// This is a convenience method that handles folder tracking automatically.
    ///
    /// # Errors
    ///
    /// Returns an error if task execution fails and `abort_on_failure` is true.
    ///
    /// # Panics
    ///
    /// Panics if the folder tracker mutex is poisoned.
    pub async fn execute_pre_connect(
        &self,
        task: &ConnectionTask,
        scope: VariableScope,
        folder_id: Option<Uuid>,
    ) -> TaskResult<i32> {
        let is_first = {
            let mut tracker = self.folder_tracker.lock().unwrap();
            tracker.connection_opened(folder_id)
        };

        self.execute(task, scope, folder_id, is_first, false).await
    }

    /// Executes a post-disconnect task
    ///
    /// This is a convenience method that handles folder tracking automatically.
    ///
    /// # Errors
    ///
    /// Returns an error if task execution fails.
    ///
    /// # Panics
    ///
    /// Panics if the folder tracker mutex is poisoned.
    pub async fn execute_post_disconnect(
        &self,
        task: &ConnectionTask,
        scope: VariableScope,
        folder_id: Option<Uuid>,
    ) -> TaskResult<i32> {
        let is_last = {
            let mut tracker = self.folder_tracker.lock().unwrap();
            tracker.connection_closed(folder_id)
        };

        self.execute(task, scope, folder_id, false, is_last).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_timing_description() {
        assert_eq!(TaskTiming::PreConnect.description(), "Pre-connect");
        assert_eq!(TaskTiming::PostDisconnect.description(), "Post-disconnect");
    }

    #[test]
    fn test_task_condition_new() {
        let condition = TaskCondition::new();
        assert!(!condition.only_first_in_folder);
        assert!(!condition.only_last_in_folder);
        assert!(!condition.has_restrictions());
    }

    #[test]
    fn test_task_condition_first_in_folder() {
        let condition = TaskCondition::first_in_folder();
        assert!(condition.only_first_in_folder);
        assert!(!condition.only_last_in_folder);
        assert!(condition.has_restrictions());
    }

    #[test]
    fn test_task_condition_last_in_folder() {
        let condition = TaskCondition::last_in_folder();
        assert!(!condition.only_first_in_folder);
        assert!(condition.only_last_in_folder);
        assert!(condition.has_restrictions());
    }

    #[test]
    fn test_task_condition_should_execute() {
        // No restrictions - always execute
        let no_restrictions = TaskCondition::new();
        assert!(no_restrictions.should_execute(true, true));
        assert!(no_restrictions.should_execute(true, false));
        assert!(no_restrictions.should_execute(false, true));
        assert!(no_restrictions.should_execute(false, false));

        // First in folder only
        let first_only = TaskCondition::first_in_folder();
        assert!(first_only.should_execute(true, true));
        assert!(first_only.should_execute(true, false));
        assert!(!first_only.should_execute(false, true));
        assert!(!first_only.should_execute(false, false));

        // Last in folder only
        let last_only = TaskCondition::last_in_folder();
        assert!(last_only.should_execute(true, true));
        assert!(!last_only.should_execute(true, false));
        assert!(last_only.should_execute(false, true));
        assert!(!last_only.should_execute(false, false));
    }

    #[test]
    fn test_connection_task_new_pre_connect() {
        let task = ConnectionTask::new_pre_connect("echo hello");
        assert!(task.is_pre_connect());
        assert!(!task.is_post_disconnect());
        assert_eq!(task.command, "echo hello");
        assert!(task.abort_on_failure);
    }

    #[test]
    fn test_connection_task_new_post_disconnect() {
        let task = ConnectionTask::new_post_disconnect("cleanup.sh");
        assert!(!task.is_pre_connect());
        assert!(task.is_post_disconnect());
        assert_eq!(task.command, "cleanup.sh");
        assert!(!task.abort_on_failure);
    }

    #[test]
    fn test_connection_task_builders() {
        let task = ConnectionTask::new_pre_connect("test")
            .with_condition(TaskCondition::first_in_folder())
            .with_timeout(5000)
            .with_abort_on_failure(false)
            .with_description("Test task");

        assert!(task.condition.only_first_in_folder);
        assert_eq!(task.timeout_ms, Some(5000));
        assert!(!task.abort_on_failure);
        assert_eq!(task.description, Some("Test task".to_string()));
    }

    #[test]
    fn test_connection_task_serialization() {
        let task = ConnectionTask::new_pre_connect("echo ${var}")
            .with_condition(TaskCondition::first_in_folder())
            .with_timeout(1000);

        let json = serde_json::to_string(&task).unwrap();
        let parsed: ConnectionTask = serde_json::from_str(&json).unwrap();

        assert_eq!(task.id, parsed.id);
        assert_eq!(task.timing, parsed.timing);
        assert_eq!(task.command, parsed.command);
        assert_eq!(task.condition, parsed.condition);
        assert_eq!(task.timeout_ms, parsed.timeout_ms);
        assert_eq!(task.abort_on_failure, parsed.abort_on_failure);
    }

    #[test]
    fn test_folder_connection_tracker() {
        let mut tracker = FolderConnectionTracker::new();
        let folder_id = Some(Uuid::new_v4());

        // First connection
        assert!(tracker.connection_opened(folder_id));
        assert_eq!(tracker.active_count(folder_id), 1);
        assert!(tracker.has_active_connections(folder_id));

        // Second connection
        assert!(!tracker.connection_opened(folder_id));
        assert_eq!(tracker.active_count(folder_id), 2);

        // Close one
        assert!(!tracker.connection_closed(folder_id));
        assert_eq!(tracker.active_count(folder_id), 1);

        // Close last
        assert!(tracker.connection_closed(folder_id));
        assert_eq!(tracker.active_count(folder_id), 0);
        assert!(!tracker.has_active_connections(folder_id));
    }

    #[test]
    fn test_folder_tracker_nil_folder() {
        let mut tracker = FolderConnectionTracker::new();

        // None folder uses Uuid::nil()
        assert!(tracker.connection_opened(None));
        assert_eq!(tracker.active_count(None), 1);
        assert!(tracker.connection_closed(None));
        assert_eq!(tracker.active_count(None), 0);
    }

    #[test]
    fn test_task_substitute_command() {
        let mut manager = VariableManager::new();
        manager.set_global(crate::Variable::new("host", "example.com"));
        manager.set_global(crate::Variable::new("port", "22"));

        let task = ConnectionTask::new_pre_connect("ssh ${host} -p ${port}");
        let result = task.substitute_command(&manager, VariableScope::Global);

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "ssh example.com -p 22");
    }

    #[test]
    fn test_task_substitute_command_undefined_var() {
        // Note: VariableManager.substitute() logs a warning and uses empty string
        // for undefined variables, rather than returning an error
        let manager = VariableManager::new();
        let task = ConnectionTask::new_pre_connect("echo ${undefined}");
        let result = task.substitute_command(&manager, VariableScope::Global);

        // Should succeed with empty string substituted
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "echo ");
    }
}
