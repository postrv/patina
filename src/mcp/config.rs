//! MCP server configuration loading.
//!
//! Loads `.mcp.json` (project-local) and `~/.claude.json` (user-global) config files
//! in the Claude Code compatible format, merges them, and returns a map of server entries.
//!
//! # Config Format
//!
//! ```json
//! {
//!   "mcpServers": {
//!     "server-name": {
//!       "command": "npx",
//!       "args": ["-y", "@modelcontextprotocol/server-filesystem"],
//!       "env": { "KEY": "value" },
//!       "disabled": false
//!     }
//!   }
//! }
//! ```
//!
//! # Merge Behavior
//!
//! Project-local config (`.mcp.json`) overrides user-global config (`~/.claude.json`)
//! for servers with the same name. A project can disable a user-global server by
//! setting `"disabled": true`.
//!
//! # Example
//!
//! ```no_run
//! use patina::mcp::config::load_mcp_config;
//! use std::path::Path;
//!
//! # fn example() -> anyhow::Result<()> {
//! let configs = load_mcp_config(Path::new("."))?;
//! for (name, entry) in &configs {
//!     println!("Server '{}': command={}", name, entry.command);
//! }
//! # Ok(())
//! # }
//! ```

use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// Claude Code compatible MCP server entry.
///
/// Uses a flat structure where `command`, `args`, and `env` are top-level fields.
/// If `url` is present, the server uses SSE transport instead of stdio.
///
/// # Examples
///
/// ```
/// use patina::mcp::config::McpServerEntry;
///
/// let json = r#"{"command":"npx","args":["-y","server"]}"#;
/// let entry: McpServerEntry = serde_json::from_str(json).unwrap();
/// assert_eq!(entry.command, "npx");
/// assert!(!entry.is_disabled());
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct McpServerEntry {
    /// Command to run for stdio transport.
    #[serde(default)]
    pub command: String,

    /// Arguments for the command.
    #[serde(default)]
    pub args: Vec<String>,

    /// Environment variables for the server process.
    #[serde(default)]
    pub env: HashMap<String, String>,

    /// URL for SSE transport (if present, stdio fields are ignored).
    #[serde(default)]
    pub url: Option<String>,

    /// HTTP headers for SSE transport.
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,

    /// Whether this server is disabled. Absent means enabled.
    #[serde(default)]
    pub disabled: bool,
}

impl McpServerEntry {
    /// Returns `true` if this server entry is disabled.
    #[must_use]
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// Returns `true` if this entry uses SSE transport.
    #[must_use]
    pub fn is_sse(&self) -> bool {
        self.url.is_some()
    }

    /// Returns `true` if this entry uses stdio transport.
    #[must_use]
    pub fn is_stdio(&self) -> bool {
        self.url.is_none()
    }
}

/// Root config file format matching Claude Code's `.mcp.json`.
///
/// # Examples
///
/// ```
/// use patina::mcp::config::McpConfigFile;
///
/// let json = r#"{"mcpServers":{}}"#;
/// let config: McpConfigFile = serde_json::from_str(json).unwrap();
/// assert!(config.mcp_servers.is_empty());
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct McpConfigFile {
    /// Map of server name to server entry.
    #[serde(rename = "mcpServers", default)]
    pub mcp_servers: HashMap<String, McpServerEntry>,
}

/// Loads and merges MCP configuration from project-local and user-global files.
///
/// 1. Loads `.mcp.json` from `working_dir` (project-local)
/// 2. Loads `~/.claude.json` (user-global)
/// 3. Merges: project-local overrides user-global for same server names
/// 4. Filters out disabled entries
///
/// Missing config files are silently ignored (returns empty map).
///
/// # Arguments
///
/// * `working_dir` - The project root directory to search for `.mcp.json`
///
/// # Errors
///
/// Returns an error if a config file exists but contains invalid JSON.
///
/// # Examples
///
/// ```no_run
/// use patina::mcp::config::load_mcp_config;
/// use std::path::Path;
///
/// # fn example() -> anyhow::Result<()> {
/// let configs = load_mcp_config(Path::new("."))?;
/// println!("Found {} MCP servers", configs.len());
/// # Ok(())
/// # }
/// ```
pub fn load_mcp_config(working_dir: &Path) -> Result<HashMap<String, McpServerEntry>> {
    let project_config = load_config_file(&working_dir.join(".mcp.json"))?;
    let user_config = load_user_global_config()?;
    let merged = merge_configs(project_config, user_config);
    Ok(merged)
}

/// Loads MCP configuration from a specific project directory only.
///
/// Unlike [`load_mcp_config`], this does NOT load the user-global `~/.claude.json`.
/// Useful for isolated testing.
///
/// # Errors
///
/// Returns an error if a config file exists but contains invalid JSON.
pub fn load_project_config(working_dir: &Path) -> Result<HashMap<String, McpServerEntry>> {
    let project_config = load_config_file(&working_dir.join(".mcp.json"))?;
    // Filter out disabled
    let merged = merge_configs(project_config, HashMap::new());
    Ok(merged)
}

/// Loads a single config file, returning an empty map if the file doesn't exist.
///
/// # Errors
///
/// Returns an error if the file exists but contains invalid JSON.
fn load_config_file(path: &Path) -> Result<HashMap<String, McpServerEntry>> {
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    let config: McpConfigFile = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

    Ok(config.mcp_servers)
}

/// Loads the user-global config from `~/.claude.json`.
///
/// Returns an empty map if the file doesn't exist or the home directory
/// cannot be determined.
///
/// # Errors
///
/// Returns an error if the file exists but contains invalid JSON.
fn load_user_global_config() -> Result<HashMap<String, McpServerEntry>> {
    let base_dirs = match directories::BaseDirs::new() {
        Some(d) => d,
        None => return Ok(HashMap::new()),
    };
    load_config_file(&base_dirs.home_dir().join(".claude.json"))
}

/// Merges project-local and user-global configs.
///
/// Project-local entries override user-global entries with the same name.
/// Disabled entries are filtered out from the final result.
#[must_use]
fn merge_configs(
    project: HashMap<String, McpServerEntry>,
    user_global: HashMap<String, McpServerEntry>,
) -> HashMap<String, McpServerEntry> {
    let mut merged = user_global;

    // Project overrides user-global
    for (name, entry) in project {
        merged.insert(name, entry);
    }

    // Filter out disabled entries
    merged.retain(|_, entry| !entry.is_disabled());

    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_stdio_server() {
        let json = r#"{"command":"npx","args":["-y","server"]}"#;
        let entry: McpServerEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.command, "npx");
        assert_eq!(entry.args, vec!["-y", "server"]);
        assert!(!entry.is_disabled());
        assert!(entry.is_stdio());
    }

    #[test]
    fn deserialize_env_variables() {
        let json =
            r#"{"command":"node","args":["server.js"],"env":{"API_KEY":"secret","PORT":"3000"}}"#;
        let entry: McpServerEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.env.get("API_KEY"), Some(&"secret".to_string()));
        assert_eq!(entry.env.get("PORT"), Some(&"3000".to_string()));
    }

    #[test]
    fn deserialize_disabled_absent_means_enabled() {
        let json = r#"{"command":"echo"}"#;
        let entry: McpServerEntry = serde_json::from_str(json).unwrap();
        assert!(!entry.is_disabled());
    }

    #[test]
    fn deserialize_disabled_true() {
        let json = r#"{"command":"echo","disabled":true}"#;
        let entry: McpServerEntry = serde_json::from_str(json).unwrap();
        assert!(entry.is_disabled());
    }

    #[test]
    fn deserialize_sse_server() {
        let json =
            r#"{"url":"http://localhost:8080/sse","headers":{"Authorization":"Bearer token"}}"#;
        let entry: McpServerEntry = serde_json::from_str(json).unwrap();
        assert!(entry.is_sse());
        assert!(!entry.is_stdio());
        assert_eq!(entry.url.as_deref(), Some("http://localhost:8080/sse"));
        let headers = entry.headers.as_ref().unwrap();
        assert_eq!(
            headers.get("Authorization"),
            Some(&"Bearer token".to_string())
        );
    }

    #[test]
    fn deserialize_config_file() {
        let json = r#"{
            "mcpServers": {
                "filesystem": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-filesystem"]
                }
            }
        }"#;
        let config: McpConfigFile = serde_json::from_str(json).unwrap();
        assert_eq!(config.mcp_servers.len(), 1);
        assert!(config.mcp_servers.contains_key("filesystem"));
    }

    #[test]
    fn deserialize_empty_servers() {
        let json = r#"{"mcpServers":{}}"#;
        let config: McpConfigFile = serde_json::from_str(json).unwrap();
        assert!(config.mcp_servers.is_empty());
    }

    #[test]
    fn deserialize_multiple_servers() {
        let json = r#"{
            "mcpServers": {
                "fs": {"command": "fs-server"},
                "git": {"command": "git-server", "args": ["--verbose"]}
            }
        }"#;
        let config: McpConfigFile = serde_json::from_str(json).unwrap();
        assert_eq!(config.mcp_servers.len(), 2);
        assert!(config.mcp_servers.contains_key("fs"));
        assert!(config.mcp_servers.contains_key("git"));
    }

    #[test]
    fn merge_project_overrides_user_global() {
        let mut user_global = HashMap::new();
        user_global.insert(
            "server".to_string(),
            McpServerEntry {
                command: "old-command".to_string(),
                args: vec![],
                env: HashMap::new(),
                url: None,
                headers: None,
                disabled: false,
            },
        );

        let mut project = HashMap::new();
        project.insert(
            "server".to_string(),
            McpServerEntry {
                command: "new-command".to_string(),
                args: vec!["--flag".to_string()],
                env: HashMap::new(),
                url: None,
                headers: None,
                disabled: false,
            },
        );

        let merged = merge_configs(project, user_global);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged["server"].command, "new-command");
    }

    #[test]
    fn merge_combines_disjoint_servers() {
        let mut user_global = HashMap::new();
        user_global.insert(
            "fs".to_string(),
            McpServerEntry {
                command: "fs-server".to_string(),
                args: vec![],
                env: HashMap::new(),
                url: None,
                headers: None,
                disabled: false,
            },
        );

        let mut project = HashMap::new();
        project.insert(
            "git".to_string(),
            McpServerEntry {
                command: "git-server".to_string(),
                args: vec![],
                env: HashMap::new(),
                url: None,
                headers: None,
                disabled: false,
            },
        );

        let merged = merge_configs(project, user_global);
        assert_eq!(merged.len(), 2);
        assert!(merged.contains_key("fs"));
        assert!(merged.contains_key("git"));
    }

    #[test]
    fn merge_empty_project_uses_global() {
        let mut user_global = HashMap::new();
        user_global.insert(
            "server".to_string(),
            McpServerEntry {
                command: "global-server".to_string(),
                args: vec![],
                env: HashMap::new(),
                url: None,
                headers: None,
                disabled: false,
            },
        );

        let merged = merge_configs(HashMap::new(), user_global);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged["server"].command, "global-server");
    }

    #[test]
    fn merge_disabled_in_project_disables_global_server() {
        let mut user_global = HashMap::new();
        user_global.insert(
            "server".to_string(),
            McpServerEntry {
                command: "global-server".to_string(),
                args: vec![],
                env: HashMap::new(),
                url: None,
                headers: None,
                disabled: false,
            },
        );

        let mut project = HashMap::new();
        project.insert(
            "server".to_string(),
            McpServerEntry {
                command: String::new(),
                args: vec![],
                env: HashMap::new(),
                url: None,
                headers: None,
                disabled: true,
            },
        );

        let merged = merge_configs(project, user_global);
        assert!(merged.is_empty());
    }
}
