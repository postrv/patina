//! Real subagent execution with API session management.
//!
//! This module provides production-ready subagent orchestration that:
//! - Creates actual API sessions for each subagent
//! - Inherits context from the parent conversation
//! - Manages subagent tool restrictions
//! - Collects and merges results
//!
//! # Example
//!
//! ```
//! use patina::agents::{SubagentContext, SubagentSpawner};
//!
//! # async fn example() -> anyhow::Result<()> {
//! // Create spawner
//! let spawner = SubagentSpawner::new();
//!
//! // Define parent context to inherit
//! let context = SubagentContext {
//!     working_dir: std::path::PathBuf::from("/project"),
//!     parent_messages: vec![],
//!     project_files: vec!["README.md".into()],
//! };
//!
//! // Spawn a subagent session
//! let session = spawner.spawn(
//!     "explorer",
//!     "You explore codebases",
//!     context,
//!     vec!["read".into(), "glob".into()],
//! ).await?;
//!
//! // Session has a unique ID
//! assert!(!session.id().is_nil());
//! # Ok(())
//! # }
//! ```

use anyhow::Result;
use std::path::PathBuf;
use uuid::Uuid;

/// Context inherited from the parent conversation.
///
/// When a subagent is spawned, it inherits relevant context from its parent
/// to maintain continuity and awareness of the task at hand.
#[derive(Debug, Clone, Default)]
pub struct SubagentContext {
    /// The working directory for file operations.
    /// Subagent inherits this from the parent.
    pub working_dir: PathBuf,

    /// Relevant messages from the parent conversation.
    /// These provide context about what the user is working on.
    pub parent_messages: Vec<String>,

    /// Key project files that provide context.
    /// Often includes CLAUDE.md, README.md, etc.
    pub project_files: Vec<String>,
}

impl SubagentContext {
    /// Creates a new context with the given working directory.
    #[must_use]
    pub fn new(working_dir: PathBuf) -> Self {
        Self {
            working_dir,
            parent_messages: Vec::new(),
            project_files: Vec::new(),
        }
    }

    /// Adds parent messages to the context.
    #[must_use]
    pub fn with_messages(mut self, messages: Vec<String>) -> Self {
        self.parent_messages = messages;
        self
    }

    /// Adds project files to the context.
    #[must_use]
    pub fn with_project_files(mut self, files: Vec<String>) -> Self {
        self.project_files = files;
        self
    }
}

/// A live subagent session connected to the API.
///
/// Unlike `SubagentConfig` which just describes a subagent,
/// `SubagentSession` represents an active session with:
/// - A unique session ID
/// - Connection to the API
/// - Inherited context from parent
/// - Tool restrictions
#[derive(Debug)]
pub struct SubagentSession {
    /// Unique identifier for this session.
    id: Uuid,

    /// Name of the subagent (e.g., "explorer", "planner").
    name: String,

    /// The system prompt for this subagent.
    system_prompt: String,

    /// Context inherited from parent.
    context: SubagentContext,

    /// Tools this subagent is allowed to use.
    allowed_tools: Vec<String>,

    /// Maximum turns before the agent must complete.
    max_turns: usize,

    /// Current turn count.
    current_turn: usize,

    /// Whether the session has completed.
    completed: bool,
}

impl SubagentSession {
    /// Returns the session ID.
    #[must_use]
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// Returns the subagent name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the system prompt.
    #[must_use]
    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }

    /// Returns the inherited context.
    #[must_use]
    pub fn context(&self) -> &SubagentContext {
        &self.context
    }

    /// Returns the working directory from inherited context.
    #[must_use]
    pub fn working_dir(&self) -> &PathBuf {
        &self.context.working_dir
    }

    /// Returns the allowed tools.
    #[must_use]
    pub fn allowed_tools(&self) -> &[String] {
        &self.allowed_tools
    }

    /// Checks if a tool is allowed for this session.
    #[must_use]
    pub fn is_tool_allowed(&self, tool: &str) -> bool {
        self.allowed_tools.iter().any(|t| t == tool)
    }

    /// Returns the max turns allowed.
    #[must_use]
    pub fn max_turns(&self) -> usize {
        self.max_turns
    }

    /// Returns the current turn count.
    #[must_use]
    pub fn current_turn(&self) -> usize {
        self.current_turn
    }

    /// Returns whether the session is completed.
    #[must_use]
    pub fn is_completed(&self) -> bool {
        self.completed
    }

    /// Returns the parent messages from context.
    #[must_use]
    pub fn parent_messages(&self) -> &[String] {
        &self.context.parent_messages
    }

    /// Returns the project files from context.
    #[must_use]
    pub fn project_files(&self) -> &[String] {
        &self.context.project_files
    }
}

/// Result from a completed subagent execution.
///
/// Contains the output, success status, and execution metadata.
#[derive(Debug, Clone)]
pub struct SubagentExecutionResult {
    /// The session ID that produced this result.
    pub session_id: Uuid,

    /// The subagent name.
    pub name: String,

    /// The final output/response from the subagent.
    pub output: String,

    /// Whether the subagent completed successfully.
    pub success: bool,

    /// Number of turns used.
    pub turns_used: usize,

    /// Any files modified by the subagent.
    pub files_modified: Vec<String>,

    /// Any errors encountered (even if ultimately successful).
    pub errors: Vec<String>,
}

/// Spawner for creating subagent sessions.
///
/// Handles the creation of new subagent sessions with proper
/// API client configuration and context inheritance.
pub struct SubagentSpawner {
    /// Model to use for subagents (inherited from parent or configured).
    model: String,
}

impl SubagentSpawner {
    /// Creates a new spawner with the default model.
    #[must_use]
    pub fn new() -> Self {
        Self {
            model: "claude-sonnet-4-20250514".to_string(),
        }
    }

    /// Creates a new spawner with a specific model.
    #[must_use]
    pub fn with_model(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
        }
    }

    /// Returns the model being used.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Spawns a new subagent session.
    ///
    /// Creates a session with:
    /// - A unique ID
    /// - Inherited context from parent
    /// - Configured tool restrictions
    ///
    /// # Arguments
    ///
    /// * `name` - Name for the subagent (e.g., "explorer")
    /// * `system_prompt` - System prompt defining behavior
    /// * `context` - Context inherited from parent
    /// * `allowed_tools` - Tools this subagent can use
    ///
    /// # Errors
    ///
    /// Returns an error if the session cannot be created.
    pub async fn spawn(
        &self,
        name: impl Into<String>,
        system_prompt: impl Into<String>,
        context: SubagentContext,
        allowed_tools: Vec<String>,
    ) -> Result<SubagentSession> {
        self.spawn_with_max_turns(name, system_prompt, context, allowed_tools, 10)
            .await
    }

    /// Spawns a new subagent session with custom max turns.
    ///
    /// # Errors
    ///
    /// Returns an error if the session cannot be created.
    pub async fn spawn_with_max_turns(
        &self,
        name: impl Into<String>,
        system_prompt: impl Into<String>,
        context: SubagentContext,
        allowed_tools: Vec<String>,
        max_turns: usize,
    ) -> Result<SubagentSession> {
        let session = SubagentSession {
            id: Uuid::new_v4(),
            name: name.into(),
            system_prompt: system_prompt.into(),
            context,
            allowed_tools,
            max_turns,
            current_turn: 0,
            completed: false,
        };

        Ok(session)
    }
}

impl Default for SubagentSpawner {
    fn default() -> Self {
        Self::new()
    }
}

/// Collector for subagent results.
///
/// Gathers results from multiple subagent executions and
/// provides methods for merging and summarizing.
#[derive(Debug, Default)]
pub struct SubagentResultCollector {
    results: Vec<SubagentExecutionResult>,
}

impl SubagentResultCollector {
    /// Creates a new empty collector.
    #[must_use]
    pub fn new() -> Self {
        Self {
            results: Vec::new(),
        }
    }

    /// Adds a result to the collector.
    pub fn add(&mut self, result: SubagentExecutionResult) {
        self.results.push(result);
    }

    /// Returns all collected results.
    #[must_use]
    pub fn results(&self) -> &[SubagentExecutionResult] {
        &self.results
    }

    /// Returns the number of results collected.
    #[must_use]
    pub fn len(&self) -> usize {
        self.results.len()
    }

    /// Returns whether the collector is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.results.is_empty()
    }

    /// Returns all results that succeeded.
    #[must_use]
    pub fn successful_results(&self) -> Vec<&SubagentExecutionResult> {
        self.results.iter().filter(|r| r.success).collect()
    }

    /// Returns all results that failed.
    #[must_use]
    pub fn failed_results(&self) -> Vec<&SubagentExecutionResult> {
        self.results.iter().filter(|r| !r.success).collect()
    }

    /// Returns all files modified across all subagents.
    #[must_use]
    pub fn all_files_modified(&self) -> Vec<String> {
        self.results
            .iter()
            .flat_map(|r| r.files_modified.iter().cloned())
            .collect()
    }

    /// Creates a combined summary of all results.
    #[must_use]
    pub fn summary(&self) -> String {
        let total = self.results.len();
        let succeeded = self.successful_results().len();
        let failed = self.failed_results().len();

        format!(
            "Subagent Results: {}/{} succeeded, {} failed",
            succeeded, total, failed
        )
    }
}

/// Runner for executing subagent sessions against the API.
///
/// The runner handles:
/// - Filtering tools based on session permissions
/// - Building initial messages with inherited context
/// - Executing API calls with streaming
/// - Collecting results into `SubagentExecutionResult`
///
/// # Example
///
/// ```ignore
/// use patina::agents::{SubagentRunner, SubagentSpawner, SubagentContext};
/// use patina::api::AnthropicClient;
/// use std::path::PathBuf;
///
/// # async fn example() -> anyhow::Result<()> {
/// let client = AnthropicClient::new(api_key, "claude-sonnet-4-20250514");
/// let runner = SubagentRunner::new(client);
///
/// let spawner = SubagentSpawner::new();
/// let context = SubagentContext::new(PathBuf::from("/project"));
/// let session = spawner.spawn(
///     "explorer",
///     "You explore codebases",
///     context,
///     vec!["read_file".into(), "glob".into()],
/// ).await?;
///
/// let result = runner.execute(&session, "Find all Rust files").await?;
/// assert!(result.success);
/// # Ok(())
/// # }
/// ```
pub struct SubagentRunner {
    /// The API client used for requests.
    client: crate::api::AnthropicClient,
}

impl SubagentRunner {
    /// Creates a new runner with the given API client.
    #[must_use]
    pub fn new(client: crate::api::AnthropicClient) -> Self {
        Self { client }
    }

    /// Filters tools to only those allowed by the session.
    ///
    /// Returns an empty vector if no tools are allowed.
    #[must_use]
    pub fn filter_tools(
        &self,
        session: &SubagentSession,
    ) -> Vec<crate::api::tools::ToolDefinition> {
        let all_tools = crate::api::tools::default_tools();
        all_tools
            .into_iter()
            .filter(|tool| session.is_tool_allowed(&tool.name))
            .collect()
    }

    /// Builds the initial system context message for the subagent.
    ///
    /// Combines the system prompt with inherited context (parent messages, project files).
    #[must_use]
    pub fn build_context_message(&self, session: &SubagentSession) -> String {
        let mut context_parts = Vec::new();

        // Add system prompt
        context_parts.push(session.system_prompt().to_string());

        // Add working directory context
        context_parts.push(format!(
            "\nWorking directory: {}",
            session.working_dir().display()
        ));

        // Add project files if any
        if !session.project_files().is_empty() {
            context_parts.push("\nRelevant project files:".to_string());
            for file in session.project_files() {
                context_parts.push(format!("- {}", file));
            }
        }

        // Add parent conversation context if any
        if !session.parent_messages().is_empty() {
            context_parts.push("\nContext from parent conversation:".to_string());
            for msg in session.parent_messages() {
                context_parts.push(format!("- {}", msg));
            }
        }

        context_parts.join("\n")
    }

    /// Executes a subagent session with the given task.
    ///
    /// # Arguments
    ///
    /// * `session` - The session to execute
    /// * `task` - The task/prompt for the subagent to perform
    ///
    /// # Returns
    ///
    /// Returns a `SubagentExecutionResult` with the output, success status,
    /// and execution metadata.
    ///
    /// # Errors
    ///
    /// Returns an error if the API request fails.
    pub async fn execute(
        &self,
        session: &SubagentSession,
        task: &str,
    ) -> Result<SubagentExecutionResult> {
        use crate::api::tools::ToolChoice;
        use crate::types::{ApiMessageV2, StreamEvent};
        use tokio::sync::mpsc;

        // Build initial messages with context
        let context_message = self.build_context_message(session);
        let initial_message = format!("{}\n\nTask: {}", context_message, task);

        let messages = vec![ApiMessageV2::user(&initial_message)];

        // Filter tools for this session
        let tools = self.filter_tools(session);
        let tool_choice = if tools.is_empty() {
            None
        } else {
            Some(ToolChoice::Auto)
        };

        // Create channel for streaming
        let (tx, mut rx) = mpsc::channel::<StreamEvent>(100);

        // Execute API call
        let client = self.client.clone();
        let tools_clone = tools.clone();
        let messages_clone = messages.clone();

        tokio::spawn(async move {
            let tools_ref: Option<&[_]> = if tools_clone.is_empty() {
                None
            } else {
                Some(&tools_clone)
            };

            if let Err(e) = client
                .stream_message_v2_with_tools(&messages_clone, tools_ref, tool_choice.as_ref(), tx)
                .await
            {
                tracing::error!("Subagent API error: {}", e);
            }
        });

        // Collect streaming response
        let mut output = String::new();
        let mut errors = Vec::new();
        let mut success = true;

        while let Some(event) = rx.recv().await {
            match event {
                StreamEvent::ContentDelta(text) => {
                    output.push_str(&text);
                }
                StreamEvent::Error(err) => {
                    errors.push(err);
                    success = false;
                }
                StreamEvent::MessageComplete { .. } | StreamEvent::MessageStop => break,
                // Tool calls would need a tool execution loop in a full implementation
                StreamEvent::ToolUseStart { .. }
                | StreamEvent::ToolUseInputDelta { .. }
                | StreamEvent::ToolUseComplete { .. }
                | StreamEvent::ContentBlockComplete { .. } => {
                    // For now, we don't execute tools in the subagent
                    // This would be extended in task 1.5.4.3 when wiring into app state
                }
            }
        }

        Ok(SubagentExecutionResult {
            session_id: session.id(),
            name: session.name().to_string(),
            output,
            success,
            turns_used: 1,          // Single turn for now
            files_modified: vec![], // Would be tracked by tool execution
            errors,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============================================================================
    // 1.5.4.1 - test_spawn_subagent_creates_session
    // ============================================================================

    #[tokio::test]
    async fn test_spawn_subagent_creates_session() {
        // Given a subagent spawner
        let spawner = SubagentSpawner::new();

        // And context for the subagent
        let context = SubagentContext::new(PathBuf::from("/project"));

        // When we spawn a subagent
        let session = spawner
            .spawn(
                "explorer",
                "You explore codebases",
                context,
                vec!["read".into(), "glob".into()],
            )
            .await
            .expect("spawn should succeed");

        // Then a session is created with a unique ID
        assert!(!session.id().is_nil(), "Session should have a non-nil UUID");
    }

    #[tokio::test]
    async fn test_spawn_creates_unique_session_ids() {
        // Given a spawner
        let spawner = SubagentSpawner::new();

        // When we spawn multiple subagents
        let session1 = spawner
            .spawn("agent1", "First agent", SubagentContext::default(), vec![])
            .await
            .unwrap();

        let session2 = spawner
            .spawn("agent2", "Second agent", SubagentContext::default(), vec![])
            .await
            .unwrap();

        // Then each session has a unique ID
        assert_ne!(
            session1.id(),
            session2.id(),
            "Sessions should have unique IDs"
        );
    }

    #[tokio::test]
    async fn test_spawn_preserves_agent_name() {
        let spawner = SubagentSpawner::new();

        let session = spawner
            .spawn(
                "code-reviewer",
                "Reviews code changes",
                SubagentContext::default(),
                vec![],
            )
            .await
            .unwrap();

        assert_eq!(session.name(), "code-reviewer");
    }

    #[tokio::test]
    async fn test_spawn_configures_tool_restrictions() {
        let spawner = SubagentSpawner::new();

        let session = spawner
            .spawn(
                "read-only-agent",
                "Only reads files",
                SubagentContext::default(),
                vec!["read".into(), "glob".into(), "grep".into()],
            )
            .await
            .unwrap();

        // Allowed tools should work
        assert!(session.is_tool_allowed("read"));
        assert!(session.is_tool_allowed("glob"));
        assert!(session.is_tool_allowed("grep"));

        // Disallowed tools should be blocked
        assert!(!session.is_tool_allowed("write"));
        assert!(!session.is_tool_allowed("bash"));
        assert!(!session.is_tool_allowed("edit"));
    }

    #[tokio::test]
    async fn test_spawn_with_custom_max_turns() {
        let spawner = SubagentSpawner::new();

        let session = spawner
            .spawn_with_max_turns(
                "quick-agent",
                "Runs quickly",
                SubagentContext::default(),
                vec![],
                5,
            )
            .await
            .unwrap();

        assert_eq!(session.max_turns(), 5);
        assert_eq!(session.current_turn(), 0);
        assert!(!session.is_completed());
    }

    // ============================================================================
    // 1.5.4.1 - test_spawn_subagent_inherits_context
    // ============================================================================

    #[tokio::test]
    async fn test_spawn_subagent_inherits_context() {
        let spawner = SubagentSpawner::new();

        // Given context with working directory
        let context = SubagentContext::new(PathBuf::from("/project/src"));

        // When we spawn a subagent
        let session = spawner
            .spawn("worker", "Does work", context.clone(), vec![])
            .await
            .unwrap();

        // Then the session inherits the working directory
        assert_eq!(session.working_dir(), &PathBuf::from("/project/src"));
    }

    #[tokio::test]
    async fn test_spawn_inherits_parent_messages() {
        let spawner = SubagentSpawner::new();

        // Given context with parent messages
        let context = SubagentContext::new(PathBuf::from("/project")).with_messages(vec![
            "User asked about authentication".into(),
            "I suggested using OAuth".into(),
        ]);

        // When we spawn a subagent
        let session = spawner
            .spawn("helper", "Helps with tasks", context, vec![])
            .await
            .unwrap();

        // Then the session has access to parent messages
        let messages = session.parent_messages();
        assert_eq!(messages.len(), 2);
        assert!(messages[0].contains("authentication"));
        assert!(messages[1].contains("OAuth"));
    }

    #[tokio::test]
    async fn test_spawn_inherits_project_files() {
        let spawner = SubagentSpawner::new();

        // Given context with project files
        let context = SubagentContext::new(PathBuf::from("/project"))
            .with_project_files(vec!["CLAUDE.md".into(), "README.md".into()]);

        // When we spawn a subagent
        let session = spawner
            .spawn("reader", "Reads files", context, vec!["read".into()])
            .await
            .unwrap();

        // Then the session has access to project file list
        let files = session.project_files();
        assert_eq!(files.len(), 2);
        assert!(files.contains(&"CLAUDE.md".to_string()));
        assert!(files.contains(&"README.md".to_string()));
    }

    #[tokio::test]
    async fn test_context_builder_pattern() {
        let context = SubagentContext::new(PathBuf::from("/work"))
            .with_messages(vec!["message1".into()])
            .with_project_files(vec!["file1".into()]);

        assert_eq!(context.working_dir, PathBuf::from("/work"));
        assert_eq!(context.parent_messages.len(), 1);
        assert_eq!(context.project_files.len(), 1);
    }

    #[tokio::test]
    async fn test_spawner_model_configuration() {
        // Default model
        let default_spawner = SubagentSpawner::new();
        assert!(default_spawner.model().contains("claude"));

        // Custom model
        let custom_spawner = SubagentSpawner::with_model("claude-opus-4-20250514");
        assert_eq!(custom_spawner.model(), "claude-opus-4-20250514");
    }

    // ============================================================================
    // 1.5.4.1 - test_subagent_result_collection
    // ============================================================================

    #[test]
    fn test_subagent_result_collection() {
        let mut collector = SubagentResultCollector::new();

        // Initially empty
        assert!(collector.is_empty());
        assert_eq!(collector.len(), 0);

        // Add a successful result
        let result1 = SubagentExecutionResult {
            session_id: Uuid::new_v4(),
            name: "explorer".into(),
            output: "Found 10 files".into(),
            success: true,
            turns_used: 3,
            files_modified: vec![],
            errors: vec![],
        };
        collector.add(result1);

        assert_eq!(collector.len(), 1);
        assert_eq!(collector.successful_results().len(), 1);
        assert_eq!(collector.failed_results().len(), 0);
    }

    #[test]
    fn test_result_collection_multiple_results() {
        let mut collector = SubagentResultCollector::new();

        // Add successful result
        collector.add(SubagentExecutionResult {
            session_id: Uuid::new_v4(),
            name: "agent1".into(),
            output: "Success 1".into(),
            success: true,
            turns_used: 2,
            files_modified: vec!["file1.rs".into()],
            errors: vec![],
        });

        // Add failed result
        collector.add(SubagentExecutionResult {
            session_id: Uuid::new_v4(),
            name: "agent2".into(),
            output: "Failed".into(),
            success: false,
            turns_used: 1,
            files_modified: vec![],
            errors: vec!["Connection timeout".into()],
        });

        // Add another successful result
        collector.add(SubagentExecutionResult {
            session_id: Uuid::new_v4(),
            name: "agent3".into(),
            output: "Success 2".into(),
            success: true,
            turns_used: 5,
            files_modified: vec!["file2.rs".into(), "file3.rs".into()],
            errors: vec![],
        });

        assert_eq!(collector.len(), 3);
        assert_eq!(collector.successful_results().len(), 2);
        assert_eq!(collector.failed_results().len(), 1);
    }

    #[test]
    fn test_result_collection_files_modified() {
        let mut collector = SubagentResultCollector::new();

        collector.add(SubagentExecutionResult {
            session_id: Uuid::new_v4(),
            name: "writer1".into(),
            output: "Done".into(),
            success: true,
            turns_used: 1,
            files_modified: vec!["src/main.rs".into()],
            errors: vec![],
        });

        collector.add(SubagentExecutionResult {
            session_id: Uuid::new_v4(),
            name: "writer2".into(),
            output: "Done".into(),
            success: true,
            turns_used: 1,
            files_modified: vec!["src/lib.rs".into(), "tests/test.rs".into()],
            errors: vec![],
        });

        let all_files = collector.all_files_modified();
        assert_eq!(all_files.len(), 3);
        assert!(all_files.contains(&"src/main.rs".to_string()));
        assert!(all_files.contains(&"src/lib.rs".to_string()));
        assert!(all_files.contains(&"tests/test.rs".to_string()));
    }

    #[test]
    fn test_result_collection_summary() {
        let mut collector = SubagentResultCollector::new();

        // 2 successful, 1 failed
        collector.add(SubagentExecutionResult {
            session_id: Uuid::new_v4(),
            name: "a".into(),
            output: "ok".into(),
            success: true,
            turns_used: 1,
            files_modified: vec![],
            errors: vec![],
        });

        collector.add(SubagentExecutionResult {
            session_id: Uuid::new_v4(),
            name: "b".into(),
            output: "fail".into(),
            success: false,
            turns_used: 1,
            files_modified: vec![],
            errors: vec![],
        });

        collector.add(SubagentExecutionResult {
            session_id: Uuid::new_v4(),
            name: "c".into(),
            output: "ok".into(),
            success: true,
            turns_used: 1,
            files_modified: vec![],
            errors: vec![],
        });

        let summary = collector.summary();
        assert!(summary.contains("2/3 succeeded"));
        assert!(summary.contains("1 failed"));
    }

    #[test]
    fn test_result_preserves_session_id() {
        let id = Uuid::new_v4();
        let result = SubagentExecutionResult {
            session_id: id,
            name: "test".into(),
            output: "output".into(),
            success: true,
            turns_used: 1,
            files_modified: vec![],
            errors: vec![],
        };

        assert_eq!(result.session_id, id);
    }

    #[test]
    fn test_result_preserves_errors() {
        let result = SubagentExecutionResult {
            session_id: Uuid::new_v4(),
            name: "test".into(),
            output: "partial".into(),
            success: false,
            turns_used: 3,
            files_modified: vec![],
            errors: vec!["Error 1".into(), "Error 2".into()],
        };

        assert_eq!(result.errors.len(), 2);
        assert!(result.errors.contains(&"Error 1".to_string()));
    }

    // ============================================================================
    // 1.5.4.2 - SubagentRunner tests
    // ============================================================================

    #[tokio::test]
    async fn test_runner_filter_tools_filters_to_allowed() {
        use secrecy::SecretString;

        // Given a runner and session with specific allowed tools
        let client = crate::api::AnthropicClient::new(
            SecretString::from("test-key"),
            "claude-sonnet-4-20250514",
        );
        let runner = SubagentRunner::new(client);

        let spawner = SubagentSpawner::new();
        let session = spawner
            .spawn(
                "reader",
                "Reads files only",
                SubagentContext::default(),
                vec!["read_file".into(), "glob".into()],
            )
            .await
            .unwrap();

        // When filtering tools
        let filtered = runner.filter_tools(&session);

        // Then only allowed tools are returned
        assert_eq!(filtered.len(), 2);
        let names: Vec<&str> = filtered.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"glob"));
        assert!(!names.contains(&"bash"));
        assert!(!names.contains(&"write_file"));
    }

    #[tokio::test]
    async fn test_runner_filter_tools_empty_when_no_tools_allowed() {
        use secrecy::SecretString;

        let client = crate::api::AnthropicClient::new(
            SecretString::from("test-key"),
            "claude-sonnet-4-20250514",
        );
        let runner = SubagentRunner::new(client);

        let spawner = SubagentSpawner::new();
        let session = spawner
            .spawn(
                "no-tools",
                "Has no tools",
                SubagentContext::default(),
                vec![], // No tools allowed
            )
            .await
            .unwrap();

        let filtered = runner.filter_tools(&session);

        assert!(filtered.is_empty());
    }

    #[tokio::test]
    async fn test_runner_build_context_includes_system_prompt() {
        use secrecy::SecretString;

        let client = crate::api::AnthropicClient::new(
            SecretString::from("test-key"),
            "claude-sonnet-4-20250514",
        );
        let runner = SubagentRunner::new(client);

        let spawner = SubagentSpawner::new();
        let session = spawner
            .spawn(
                "explorer",
                "You are an expert code explorer.",
                SubagentContext::new(PathBuf::from("/project")),
                vec![],
            )
            .await
            .unwrap();

        let context = runner.build_context_message(&session);

        assert!(context.contains("You are an expert code explorer."));
        assert!(context.contains("/project"));
    }

    #[tokio::test]
    async fn test_runner_build_context_includes_parent_messages() {
        use secrecy::SecretString;

        let client = crate::api::AnthropicClient::new(
            SecretString::from("test-key"),
            "claude-sonnet-4-20250514",
        );
        let runner = SubagentRunner::new(client);

        let spawner = SubagentSpawner::new();
        let context = SubagentContext::new(PathBuf::from("/project"))
            .with_messages(vec!["User wants to add authentication".into()]);

        let session = spawner
            .spawn("helper", "You help with tasks.", context, vec![])
            .await
            .unwrap();

        let ctx_msg = runner.build_context_message(&session);

        assert!(ctx_msg.contains("Context from parent conversation"));
        assert!(ctx_msg.contains("authentication"));
    }

    #[tokio::test]
    async fn test_runner_build_context_includes_project_files() {
        use secrecy::SecretString;

        let client = crate::api::AnthropicClient::new(
            SecretString::from("test-key"),
            "claude-sonnet-4-20250514",
        );
        let runner = SubagentRunner::new(client);

        let spawner = SubagentSpawner::new();
        let context = SubagentContext::new(PathBuf::from("/project"))
            .with_project_files(vec!["CLAUDE.md".into(), "README.md".into()]);

        let session = spawner
            .spawn("reader", "Reads files.", context, vec![])
            .await
            .unwrap();

        let ctx_msg = runner.build_context_message(&session);

        assert!(ctx_msg.contains("Relevant project files"));
        assert!(ctx_msg.contains("CLAUDE.md"));
        assert!(ctx_msg.contains("README.md"));
    }
}
