//! Permission system for tool execution.
//!
//! This module provides a Claude Code-like permission system that prompts
//! users before executing potentially sensitive tools.
//!
//! # Architecture
//!
//! ```text
//! Tool Execution Request
//!     ↓
//! PermissionManager::check()
//!     ├─ Rule allows → Execute
//!     ├─ Rule denies → Return Denied
//!     └─ No rule → Return NeedsPrompt
//! ```
//!
//! # Example
//!
//! ```no_run
//! use patina::permissions::{PermissionManager, PermissionDecision, PermissionRule};
//!
//! let mut manager = PermissionManager::new();
//!
//! // Add a rule to always allow git commands
//! manager.add_rule(PermissionRule::new("Bash", Some("git *"), true));
//!
//! // Check if a tool execution is allowed
//! let decision = manager.check("Bash", Some("git status"));
//! assert!(matches!(decision, PermissionDecision::Allowed));
//! ```

pub mod patterns;

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use patterns::matches_pattern;

/// The decision result from checking permissions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    /// The tool is allowed by a persistent rule.
    Allowed,
    /// The tool is denied by a persistent rule.
    Denied,
    /// No rule exists - user prompt required.
    NeedsPrompt,
    /// Allowed for this session only (temporary grant).
    SessionGrant,
}

/// User response to a permission prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionResponse {
    /// Allow this tool execution once (session grant).
    AllowOnce,
    /// Allow this tool pattern always (persistent rule).
    AllowAlways,
    /// Deny this tool execution.
    Deny,
}

/// A permission rule that controls tool execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRule {
    /// Pattern to match tool names (e.g., "Bash", "Read", "mcp__*").
    pub tool_pattern: String,
    /// Optional pattern to match tool input (e.g., "git:*", "npm *").
    pub input_pattern: Option<String>,
    /// Whether this rule allows (true) or denies (false) execution.
    pub allow: bool,
}

impl PermissionRule {
    /// Creates a new permission rule.
    ///
    /// # Arguments
    ///
    /// * `tool_pattern` - Pattern to match tool names
    /// * `input_pattern` - Optional pattern to match tool input
    /// * `allow` - Whether to allow or deny matching tools
    #[must_use]
    pub fn new(tool_pattern: impl Into<String>, input_pattern: Option<&str>, allow: bool) -> Self {
        Self {
            tool_pattern: tool_pattern.into(),
            input_pattern: input_pattern.map(String::from),
            allow,
        }
    }

    /// Checks if this rule matches the given tool and input.
    #[must_use]
    pub fn matches(&self, tool_name: &str, tool_input: Option<&str>) -> bool {
        // Check tool pattern
        if !matches_pattern(&self.tool_pattern, tool_name) {
            return false;
        }

        // If rule has input pattern, check it
        match (&self.input_pattern, tool_input) {
            (Some(pattern), Some(input)) => matches_pattern(pattern, input),
            (Some(_), None) => false, // Rule requires input but none provided
            (None, _) => true,        // No input pattern required
        }
    }
}

/// Configuration for permission storage.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PermissionConfig {
    /// Persistent permission rules.
    #[serde(default)]
    pub rules: Vec<PermissionRule>,
}

/// Session-based permission grant with optional expiry.
#[derive(Debug, Clone)]
struct SessionGrant {
    /// The tool name that was granted.
    tool_name: String,
    /// The specific input that was granted (if any).
    tool_input: Option<String>,
    /// When this grant expires (None = session lifetime).
    expires_at: Option<SystemTime>,
}

impl SessionGrant {
    fn new(tool_name: String, tool_input: Option<String>, duration: Option<Duration>) -> Self {
        Self {
            tool_name,
            tool_input,
            expires_at: duration.map(|d| SystemTime::now() + d),
        }
    }

    fn is_expired(&self) -> bool {
        self.expires_at
            .map(|exp| SystemTime::now() > exp)
            .unwrap_or(false)
    }

    fn matches(&self, tool_name: &str, tool_input: Option<&str>) -> bool {
        if self.is_expired() {
            return false;
        }

        if self.tool_name != tool_name {
            return false;
        }

        match (&self.tool_input, tool_input) {
            (Some(grant_input), Some(check_input)) => grant_input == check_input,
            (None, _) => true, // Grant applies to any input
            (Some(_), None) => false,
        }
    }
}

/// Manages tool execution permissions.
///
/// The `PermissionManager` maintains both persistent rules (stored in config)
/// and session grants (temporary permissions for the current session).
pub struct PermissionManager {
    /// Persistent permission rules.
    rules: Vec<PermissionRule>,
    /// Session-based grants (cleared on restart).
    session_grants: Vec<SessionGrant>,
    /// Path to the permissions config file.
    config_path: Option<PathBuf>,
    /// Whether to skip all permission checks.
    skip_permissions: bool,
    /// Tool-specific deny counts for rate limiting prompts.
    deny_counts: HashMap<String, u32>,
}

impl Default for PermissionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PermissionManager {
    /// Creates a new permission manager with no rules.
    #[must_use]
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            session_grants: Vec::new(),
            config_path: None,
            skip_permissions: false,
            deny_counts: HashMap::new(),
        }
    }

    /// Creates a permission manager from a config file.
    ///
    /// If the file doesn't exist, creates an empty manager that will
    /// save to that path when rules are added.
    ///
    /// # Errors
    ///
    /// Returns an error if the config file exists but cannot be parsed.
    pub fn from_config_file(path: PathBuf) -> Result<Self> {
        let rules = if path.exists() {
            let content = fs::read_to_string(&path)?;
            let config: PermissionConfig = toml::from_str(&content)?;
            config.rules
        } else {
            Vec::new()
        };

        Ok(Self {
            rules,
            session_grants: Vec::new(),
            config_path: Some(path),
            skip_permissions: false,
            deny_counts: HashMap::new(),
        })
    }

    /// Returns the default permissions config path.
    ///
    /// # Errors
    ///
    /// Returns an error if the config directory cannot be determined.
    pub fn default_config_path() -> Result<PathBuf> {
        let config_dir = directories::ProjectDirs::from("com", "patina", "patina")
            .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?;

        Ok(config_dir.config_dir().join("permissions.toml"))
    }

    /// Sets whether to skip all permission checks.
    ///
    /// When enabled, all tools are allowed without prompting.
    /// This is used with the `--dangerously-skip-permissions` flag.
    pub fn set_skip_permissions(&mut self, skip: bool) {
        self.skip_permissions = skip;
    }

    /// Returns whether permission checks are being skipped.
    #[must_use]
    pub fn skip_permissions(&self) -> bool {
        self.skip_permissions
    }

    /// Checks if a tool execution is allowed.
    ///
    /// The check order is:
    /// 1. If skip_permissions is true, return Allowed
    /// 2. Check persistent rules (deny rules first, then allow rules)
    /// 3. Check session grants
    /// 4. Return NeedsPrompt if no rule matches
    #[must_use]
    pub fn check(&self, tool_name: &str, tool_input: Option<&str>) -> PermissionDecision {
        // Check skip_permissions flag
        if self.skip_permissions {
            return PermissionDecision::Allowed;
        }

        // Check deny rules first (deny takes precedence)
        for rule in &self.rules {
            if !rule.allow && rule.matches(tool_name, tool_input) {
                debug!(
                    tool = %tool_name,
                    input = ?tool_input,
                    rule = ?rule,
                    "Permission denied by rule"
                );
                return PermissionDecision::Denied;
            }
        }

        // Check allow rules
        for rule in &self.rules {
            if rule.allow && rule.matches(tool_name, tool_input) {
                debug!(
                    tool = %tool_name,
                    input = ?tool_input,
                    rule = ?rule,
                    "Permission allowed by rule"
                );
                return PermissionDecision::Allowed;
            }
        }

        // Check session grants
        for grant in &self.session_grants {
            if grant.matches(tool_name, tool_input) {
                debug!(
                    tool = %tool_name,
                    input = ?tool_input,
                    "Permission allowed by session grant"
                );
                return PermissionDecision::SessionGrant;
            }
        }

        // No matching rule - prompt needed
        debug!(
            tool = %tool_name,
            input = ?tool_input,
            "No permission rule found - prompt required"
        );
        PermissionDecision::NeedsPrompt
    }

    /// Adds a persistent permission rule.
    ///
    /// If a config path is set, the rules will be saved to disk.
    pub fn add_rule(&mut self, rule: PermissionRule) {
        self.rules.push(rule);
        self.save_if_configured();
    }

    /// Adds a session grant for a specific tool execution.
    ///
    /// Session grants are not persisted and are cleared on restart.
    pub fn add_session_grant(&mut self, tool_name: &str, tool_input: Option<&str>) {
        let grant = SessionGrant::new(tool_name.to_string(), tool_input.map(String::from), None);
        self.session_grants.push(grant);
    }

    /// Adds a session grant with a time limit.
    pub fn add_timed_session_grant(
        &mut self,
        tool_name: &str,
        tool_input: Option<&str>,
        duration: Duration,
    ) {
        let grant = SessionGrant::new(
            tool_name.to_string(),
            tool_input.map(String::from),
            Some(duration),
        );
        self.session_grants.push(grant);
    }

    /// Handles a user's response to a permission prompt.
    ///
    /// This method:
    /// - For `AllowOnce`: Adds a session grant
    /// - For `AllowAlways`: Adds a persistent rule
    /// - For `Deny`: Does nothing (the caller should handle denial)
    pub fn handle_response(
        &mut self,
        tool_name: &str,
        tool_input: Option<&str>,
        response: PermissionResponse,
    ) {
        match response {
            PermissionResponse::AllowOnce => {
                self.add_session_grant(tool_name, tool_input);
            }
            PermissionResponse::AllowAlways => {
                // Create a rule that matches this specific tool/input
                let rule = PermissionRule::new(tool_name, tool_input, true);
                self.add_rule(rule);
            }
            PermissionResponse::Deny => {
                // Track denial count for rate limiting
                *self.deny_counts.entry(tool_name.to_string()).or_insert(0) += 1;
            }
        }
    }

    /// Returns the number of times a tool has been denied.
    #[must_use]
    pub fn deny_count(&self, tool_name: &str) -> u32 {
        self.deny_counts.get(tool_name).copied().unwrap_or(0)
    }

    /// Clears all session grants.
    pub fn clear_session_grants(&mut self) {
        self.session_grants.clear();
    }

    /// Clears expired session grants.
    pub fn cleanup_expired_grants(&mut self) {
        self.session_grants.retain(|g| !g.is_expired());
    }

    /// Returns all persistent rules.
    #[must_use]
    pub fn rules(&self) -> &[PermissionRule] {
        &self.rules
    }

    /// Saves rules to the config file if a path is configured.
    fn save_if_configured(&self) {
        if let Some(ref path) = self.config_path {
            if let Err(e) = self.save_to_file(path) {
                warn!(error = %e, path = %path.display(), "Failed to save permissions config");
            }
        }
    }

    /// Saves rules to a file.
    fn save_to_file(&self, path: &PathBuf) -> Result<()> {
        let config = PermissionConfig {
            rules: self.rules.clone(),
        };
        let content = toml::to_string_pretty(&config)?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(path, content)?;
        debug!(path = %path.display(), "Saved permissions config");
        Ok(())
    }
}

/// Information about a pending permission request.
#[derive(Debug, Clone)]
pub struct PermissionRequest {
    /// The tool name requesting permission.
    pub tool_name: String,
    /// The tool input (for display).
    pub tool_input: Option<String>,
    /// A human-readable description of what the tool will do.
    pub description: String,
}

impl PermissionRequest {
    /// Creates a new permission request.
    #[must_use]
    pub fn new(tool_name: &str, tool_input: Option<&str>, description: &str) -> Self {
        Self {
            tool_name: tool_name.to_string(),
            tool_input: tool_input.map(String::from),
            description: description.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // =========================================================================
    // PermissionRule tests
    // =========================================================================

    #[test]
    fn test_rule_matches_exact_tool() {
        let rule = PermissionRule::new("Bash", None, true);
        assert!(rule.matches("Bash", None));
        assert!(rule.matches("Bash", Some("git status")));
        assert!(!rule.matches("Read", None));
    }

    #[test]
    fn test_rule_matches_tool_with_input_pattern() {
        let rule = PermissionRule::new("Bash", Some("git *"), true);
        assert!(rule.matches("Bash", Some("git status")));
        assert!(rule.matches("Bash", Some("git commit -m 'test'")));
        assert!(!rule.matches("Bash", Some("npm install")));
        assert!(!rule.matches("Bash", None));
    }

    #[test]
    fn test_rule_matches_wildcard_tool() {
        let rule = PermissionRule::new("mcp__*", None, true);
        assert!(rule.matches("mcp__jetbrains__build", None));
        assert!(rule.matches("mcp__narsil__scan", None));
        assert!(!rule.matches("Bash", None));
    }

    #[test]
    fn test_rule_deny() {
        let rule = PermissionRule::new("Bash", Some("rm -rf *"), false);
        assert!(rule.matches("Bash", Some("rm -rf /")));
        assert!(!rule.allow);
    }

    // =========================================================================
    // PermissionManager tests
    // =========================================================================

    #[test]
    fn test_manager_no_rules_needs_prompt() {
        let manager = PermissionManager::new();
        let decision = manager.check("Bash", Some("ls"));
        assert_eq!(decision, PermissionDecision::NeedsPrompt);
    }

    #[test]
    fn test_manager_allow_rule() {
        let mut manager = PermissionManager::new();
        manager.add_rule(PermissionRule::new("Read", None, true));

        let decision = manager.check("Read", Some("/path/to/file"));
        assert_eq!(decision, PermissionDecision::Allowed);
    }

    #[test]
    fn test_manager_deny_rule() {
        let mut manager = PermissionManager::new();
        manager.add_rule(PermissionRule::new("Bash", Some("sudo *"), false));

        let decision = manager.check("Bash", Some("sudo rm"));
        assert_eq!(decision, PermissionDecision::Denied);
    }

    #[test]
    fn test_manager_deny_takes_precedence() {
        let mut manager = PermissionManager::new();
        // Allow all bash
        manager.add_rule(PermissionRule::new("Bash", None, true));
        // But deny sudo
        manager.add_rule(PermissionRule::new("Bash", Some("sudo *"), false));

        // sudo should be denied even though Bash is allowed
        let decision = manager.check("Bash", Some("sudo rm"));
        assert_eq!(decision, PermissionDecision::Denied);

        // Regular bash should be allowed
        let decision = manager.check("Bash", Some("ls"));
        assert_eq!(decision, PermissionDecision::Allowed);
    }

    #[test]
    fn test_manager_session_grant() {
        let mut manager = PermissionManager::new();
        manager.add_session_grant("Bash", Some("ls"));

        let decision = manager.check("Bash", Some("ls"));
        assert_eq!(decision, PermissionDecision::SessionGrant);

        // Different input should need prompt
        let decision = manager.check("Bash", Some("pwd"));
        assert_eq!(decision, PermissionDecision::NeedsPrompt);
    }

    #[test]
    fn test_manager_skip_permissions() {
        let mut manager = PermissionManager::new();
        manager.set_skip_permissions(true);

        // Everything should be allowed
        let decision = manager.check("Bash", Some("rm -rf /"));
        assert_eq!(decision, PermissionDecision::Allowed);
    }

    #[test]
    fn test_manager_handle_allow_once() {
        let mut manager = PermissionManager::new();
        manager.handle_response("Bash", Some("ls"), PermissionResponse::AllowOnce);

        let decision = manager.check("Bash", Some("ls"));
        assert_eq!(decision, PermissionDecision::SessionGrant);

        // Should not create persistent rule
        assert!(manager.rules().is_empty());
    }

    #[test]
    fn test_manager_handle_allow_always() {
        let mut manager = PermissionManager::new();
        manager.handle_response("Bash", Some("git *"), PermissionResponse::AllowAlways);

        // Should create persistent rule
        assert_eq!(manager.rules().len(), 1);

        let decision = manager.check("Bash", Some("git status"));
        assert_eq!(decision, PermissionDecision::Allowed);
    }

    #[test]
    fn test_manager_handle_deny() {
        let mut manager = PermissionManager::new();
        manager.handle_response("Bash", Some("rm"), PermissionResponse::Deny);

        // Should track deny count
        assert_eq!(manager.deny_count("Bash"), 1);

        // Should not create any rule
        assert!(manager.rules().is_empty());
    }

    #[test]
    fn test_manager_clear_session_grants() {
        let mut manager = PermissionManager::new();
        manager.add_session_grant("Bash", Some("ls"));
        manager.add_session_grant("Read", None);

        manager.clear_session_grants();

        let decision = manager.check("Bash", Some("ls"));
        assert_eq!(decision, PermissionDecision::NeedsPrompt);
    }

    // =========================================================================
    // Config persistence tests
    // =========================================================================

    #[test]
    fn test_config_save_and_load() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("permissions.toml");

        // Create manager and add rules
        let mut manager = PermissionManager::from_config_file(config_path.clone()).unwrap();
        manager.add_rule(PermissionRule::new("Bash", Some("git *"), true));
        manager.add_rule(PermissionRule::new("Read", None, true));

        // Load in a new manager
        let loaded = PermissionManager::from_config_file(config_path).unwrap();
        assert_eq!(loaded.rules().len(), 2);

        // Verify rules work
        let decision = loaded.check("Bash", Some("git status"));
        assert_eq!(decision, PermissionDecision::Allowed);
    }

    #[test]
    fn test_config_empty_file() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("permissions.toml");

        // File doesn't exist yet
        let manager = PermissionManager::from_config_file(config_path).unwrap();
        assert!(manager.rules().is_empty());
    }

    // =========================================================================
    // PermissionRequest tests
    // =========================================================================

    #[test]
    fn test_permission_request_creation() {
        let request = PermissionRequest::new("Bash", Some("git status"), "Run git status command");

        assert_eq!(request.tool_name, "Bash");
        assert_eq!(request.tool_input, Some("git status".to_string()));
        assert_eq!(request.description, "Run git status command");
    }

    // =========================================================================
    // Timed session grant tests
    // =========================================================================

    #[test]
    fn test_timed_grant_expires() {
        let grant = SessionGrant::new(
            "Bash".to_string(),
            None,
            Some(Duration::from_millis(1)), // Very short duration
        );

        // Wait for expiry
        std::thread::sleep(Duration::from_millis(10));

        assert!(grant.is_expired());
        assert!(!grant.matches("Bash", None));
    }

    #[test]
    fn test_timed_grant_not_expired() {
        let grant = SessionGrant::new(
            "Bash".to_string(),
            None,
            Some(Duration::from_secs(3600)), // 1 hour
        );

        assert!(!grant.is_expired());
        assert!(grant.matches("Bash", None));
    }
}
