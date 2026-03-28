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
//! let result = load_mcp_config(Path::new("."))?;
//! for (name, entry) in &result.servers {
//!     println!("Server '{}': command={}", name, entry.command);
//! }
//! # Ok(())
//! # }
//! ```

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Authentication configuration for MCP servers.
///
/// Supports two modes:
/// - **OAuth**: Full OAuth 2.0 flow with token refresh (requires `auth_url` and `token_url`
///   for non-standard providers).
/// - **Bearer**: Static bearer token, optionally resolved from an environment variable.
///
/// # Examples
///
/// ```
/// use patina::mcp::config::McpAuthConfig;
///
/// let json = r#"{"type":"bearer","token":"$MCP_TOKEN"}"#;
/// let config: McpAuthConfig = serde_json::from_str(json).unwrap();
/// assert!(matches!(config, McpAuthConfig::Bearer { .. }));
/// ```
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum McpAuthConfig {
    /// OAuth 2.0 authentication with automatic token refresh.
    #[serde(rename = "oauth")]
    OAuth {
        /// OAuth client ID.
        client_id: String,
        /// Requested scopes for the OAuth token.
        #[serde(default)]
        scopes: Vec<String>,
        /// Authorization endpoint URL. Required for non-standard OAuth servers.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auth_url: Option<String>,
        /// Token endpoint URL. Required for non-standard OAuth servers.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token_url: Option<String>,
    },
    /// Static bearer token (supports `$ENV_VAR` references).
    #[serde(rename = "bearer")]
    Bearer {
        /// Token value or environment variable reference (e.g., `"$MCP_TOKEN"`).
        token: String,
    },
}

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

    /// Transport type: `"sse"` for legacy SSE, `"streamable-http"` for new protocol.
    /// If absent, auto-detected from the URL (paths ending in `/sse` use legacy SSE).
    #[serde(rename = "type", default)]
    pub transport_type: Option<String>,

    /// Whether this server is disabled. Absent means enabled.
    #[serde(default)]
    pub disabled: bool,

    /// Authentication configuration for this server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<McpAuthConfig>,
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

    /// Returns `true` if this entry uses legacy SSE transport.
    ///
    /// Legacy SSE uses a GET-based SSE stream for server messages and a POST
    /// endpoint (received via an `endpoint` SSE event) for client messages.
    /// Detected by explicit `"type": "sse"` or by URL path ending in `/sse`.
    #[must_use]
    pub fn is_legacy_sse(&self) -> bool {
        match self.transport_type.as_deref() {
            Some("sse") => true,
            Some(_) => false,
            None => self
                .url
                .as_deref()
                .is_some_and(|u| u.trim_end_matches('/').ends_with("/sse")),
        }
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
/// assert!(config.forge.is_none());
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct McpConfigFile {
    /// Map of server name to server entry.
    #[serde(rename = "mcpServers", default)]
    pub mcp_servers: HashMap<String, McpServerEntry>,

    /// Optional Forge gateway configuration.
    #[serde(default)]
    pub forge: Option<super::forge::ForgeSettings>,
}

/// Result of loading MCP configuration, including optional Forge settings.
///
/// Returned by [`load_mcp_config`] and related functions. Contains the merged
/// server map plus any Forge gateway settings from the project-local config.
#[derive(Debug)]
pub struct McpConfigResult {
    /// Merged and filtered MCP server entries.
    pub servers: HashMap<String, McpServerEntry>,
    /// Forge gateway settings from the project-local `.mcp.json`, if present.
    pub forge: Option<super::forge::ForgeSettings>,
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
/// let result = load_mcp_config(Path::new("."))?;
/// println!("Found {} MCP servers", result.servers.len());
/// # Ok(())
/// # }
/// ```
pub fn load_mcp_config(working_dir: &Path) -> Result<McpConfigResult> {
    let project_path = working_dir.join(".mcp.json");
    let (project_servers, project_forge) = load_config_file_full(&project_path)?;
    let user_config = load_user_global_config()?;
    let merged = merge_configs(project_servers, user_config);
    Ok(McpConfigResult {
        servers: merged,
        forge: project_forge,
    })
}

/// Loads MCP configuration with trust verification for project configs.
///
/// If a `trust_store` is provided, project-local `.mcp.json` files are only
/// loaded if they have been explicitly trusted by the user. User-global
/// configuration (`~/.claude.json`) is always trusted.
///
/// When a project config is not trusted, it is silently skipped with a warning
/// log. The caller should prompt the user and call `trust_store.trust()` to
/// approve the config.
///
/// # Errors
///
/// Returns an error if a config file exists but contains invalid JSON.
pub fn load_mcp_config_with_trust(
    working_dir: &Path,
    trust_store: Option<&super::trust::McpTrustStore>,
) -> Result<HashMap<String, McpServerEntry>> {
    let project_config_path = working_dir.join(".mcp.json");
    let project_config = if let Some(store) = trust_store {
        if project_config_path.exists() {
            let content = std::fs::read_to_string(&project_config_path).with_context(|| {
                format!(
                    "Failed to read config file: {}",
                    project_config_path.display()
                )
            })?;
            if store.is_trusted(working_dir, &content) {
                let config: McpConfigFile = serde_json::from_str(&content).with_context(|| {
                    format!(
                        "Failed to parse config file: {}",
                        project_config_path.display()
                    )
                })?;
                config.mcp_servers
            } else {
                tracing::warn!(
                    path = %project_config_path.display(),
                    "Skipping untrusted project MCP config — approve it to enable"
                );
                HashMap::new()
            }
        } else {
            HashMap::new()
        }
    } else {
        // No trust store — load normally (backwards compatible)
        load_config_file(&project_config_path)?
    };

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
    let (servers, _forge) = load_config_file_full(path)?;
    Ok(servers)
}

/// Loads a single config file, returning both the server map and any Forge settings.
///
/// Returns `(empty_map, None)` if the file doesn't exist.
///
/// # Errors
///
/// Returns an error if the file exists but contains invalid JSON.
fn load_config_file_full(
    path: &Path,
) -> Result<(
    HashMap<String, McpServerEntry>,
    Option<super::forge::ForgeSettings>,
)> {
    if !path.exists() {
        return Ok((HashMap::new(), None));
    }

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    let config: McpConfigFile = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

    Ok((config.mcp_servers, config.forge))
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
                transport_type: None,
                disabled: false,
                auth: None,
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
                transport_type: None,
                disabled: false,
                auth: None,
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
                transport_type: None,
                disabled: false,
                auth: None,
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
                transport_type: None,
                disabled: false,
                auth: None,
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
                transport_type: None,
                disabled: false,
                auth: None,
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
                transport_type: None,
                disabled: false,
                auth: None,
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
                transport_type: None,
                disabled: true,
                auth: None,
            },
        );

        let merged = merge_configs(project, user_global);
        assert!(merged.is_empty());
    }

    #[test]
    fn is_legacy_sse_explicit_type() {
        let json = r#"{"url":"http://localhost:8080/mcp","type":"sse"}"#;
        let entry: McpServerEntry = serde_json::from_str(json).unwrap();
        assert!(entry.is_legacy_sse());
    }

    #[test]
    fn is_legacy_sse_auto_detect_from_url() {
        let json = r#"{"url":"http://localhost:8080/sse"}"#;
        let entry: McpServerEntry = serde_json::from_str(json).unwrap();
        assert!(entry.is_legacy_sse());
    }

    #[test]
    fn is_legacy_sse_auto_detect_trailing_slash() {
        let json = r#"{"url":"http://localhost:8080/sse/"}"#;
        let entry: McpServerEntry = serde_json::from_str(json).unwrap();
        assert!(entry.is_legacy_sse());
    }

    #[test]
    fn is_legacy_sse_explicit_streamable_http_overrides_url() {
        let json = r#"{"url":"http://localhost:8080/sse","type":"streamable-http"}"#;
        let entry: McpServerEntry = serde_json::from_str(json).unwrap();
        assert!(!entry.is_legacy_sse());
    }

    #[test]
    fn is_legacy_sse_non_sse_url() {
        let json = r#"{"url":"http://localhost:8080/mcp"}"#;
        let entry: McpServerEntry = serde_json::from_str(json).unwrap();
        assert!(!entry.is_legacy_sse());
    }

    #[test]
    fn is_legacy_sse_stdio_entry() {
        let json = r#"{"command":"mcp-server"}"#;
        let entry: McpServerEntry = serde_json::from_str(json).unwrap();
        assert!(!entry.is_legacy_sse());
    }

    #[test]
    fn deserialize_transport_type_field() {
        let json = r#"{"url":"http://localhost:8080/mcp","type":"sse"}"#;
        let entry: McpServerEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.transport_type.as_deref(), Some("sse"));
    }

    #[test]
    fn test_load_mcp_config_untrusted_project() {
        use super::super::trust::McpTrustStore;

        let dir = tempfile::TempDir::new().unwrap();
        let project_dir = dir.path();

        // Create a .mcp.json in the project
        let config_content = r#"{"mcpServers":{"test":{"command":"echo","args":["hello"]}}}"#;
        std::fs::write(project_dir.join(".mcp.json"), config_content).unwrap();

        // Load with a fresh (empty) trust store — project config should be skipped
        let trust_data_dir = dir.path().join("trust");
        std::fs::create_dir_all(&trust_data_dir).unwrap();
        let store = McpTrustStore::load(&trust_data_dir);

        let result = load_mcp_config_with_trust(project_dir, Some(&store)).unwrap();
        // Project config not trusted → should be empty (ignoring user-global)
        assert!(
            !result.contains_key("test"),
            "Untrusted project config should not be loaded"
        );
    }

    #[test]
    fn test_load_mcp_config_trusted_project() {
        use super::super::trust::McpTrustStore;

        let dir = tempfile::TempDir::new().unwrap();
        let project_dir = dir.path();

        let config_content = r#"{"mcpServers":{"test":{"command":"echo","args":["hello"]}}}"#;
        std::fs::write(project_dir.join(".mcp.json"), config_content).unwrap();

        // Trust the config
        let trust_data_dir = dir.path().join("trust");
        std::fs::create_dir_all(&trust_data_dir).unwrap();
        let mut store = McpTrustStore::load(&trust_data_dir);
        store.trust(project_dir, config_content);

        let result = load_mcp_config_with_trust(project_dir, Some(&store)).unwrap();
        assert!(
            result.contains_key("test"),
            "Trusted project config should be loaded"
        );
    }

    #[test]
    fn test_load_mcp_config_no_trust_store_loads_normally() {
        let dir = tempfile::TempDir::new().unwrap();
        let project_dir = dir.path();

        let config_content = r#"{"mcpServers":{"test":{"command":"echo","args":["hello"]}}}"#;
        std::fs::write(project_dir.join(".mcp.json"), config_content).unwrap();

        // No trust store → backwards compatible, loads everything
        let result = load_mcp_config_with_trust(project_dir, None).unwrap();
        assert!(
            result.contains_key("test"),
            "Without trust store, config should load normally"
        );
    }

    // =========================================================================
    // McpAuthConfig deserialization tests
    // =========================================================================

    #[test]
    fn deserialize_auth_oauth_variant() {
        let json = r#"{
            "type": "oauth",
            "client_id": "my-client",
            "scopes": ["read", "write"],
            "auth_url": "https://auth.example.com/authorize",
            "token_url": "https://auth.example.com/token"
        }"#;
        let config: McpAuthConfig = serde_json::from_str(json).unwrap();
        match config {
            McpAuthConfig::OAuth {
                client_id,
                scopes,
                auth_url,
                token_url,
            } => {
                assert_eq!(client_id, "my-client");
                assert_eq!(scopes, vec!["read", "write"]);
                assert_eq!(
                    auth_url.as_deref(),
                    Some("https://auth.example.com/authorize")
                );
                assert_eq!(token_url.as_deref(), Some("https://auth.example.com/token"));
            }
            McpAuthConfig::Bearer { .. } => panic!("Expected OAuth variant"),
        }
    }

    #[test]
    fn deserialize_auth_bearer_variant() {
        let json = r#"{"type": "bearer", "token": "$MCP_TOKEN"}"#;
        let config: McpAuthConfig = serde_json::from_str(json).unwrap();
        match config {
            McpAuthConfig::Bearer { token } => {
                assert_eq!(token, "$MCP_TOKEN");
            }
            McpAuthConfig::OAuth { .. } => panic!("Expected Bearer variant"),
        }
    }

    #[test]
    fn deserialize_auth_absent_in_server_entry() {
        let json = r#"{"command":"npx","args":["server"]}"#;
        let entry: McpServerEntry = serde_json::from_str(json).unwrap();
        assert!(entry.auth.is_none());
    }

    #[test]
    fn deserialize_auth_present_in_server_entry() {
        let json = r#"{
            "command": "npx",
            "args": ["server"],
            "auth": {
                "type": "bearer",
                "token": "test-bearer-123"
            }
        }"#;
        let entry: McpServerEntry = serde_json::from_str(json).unwrap();
        assert!(entry.auth.is_some());
        match entry.auth.as_ref().unwrap() {
            McpAuthConfig::Bearer { token } => assert_eq!(token, "test-bearer-123"),
            _ => panic!("Expected Bearer variant"),
        }
    }

    #[test]
    fn deserialize_auth_oauth_minimal() {
        let json = r#"{"type": "oauth", "client_id": "my-client"}"#;
        let config: McpAuthConfig = serde_json::from_str(json).unwrap();
        match config {
            McpAuthConfig::OAuth {
                client_id,
                scopes,
                auth_url,
                token_url,
            } => {
                assert_eq!(client_id, "my-client");
                assert!(scopes.is_empty());
                assert!(auth_url.is_none());
                assert!(token_url.is_none());
            }
            McpAuthConfig::Bearer { .. } => panic!("Expected OAuth variant"),
        }
    }

    #[test]
    fn serialize_auth_oauth_skips_none_urls() {
        let config = McpAuthConfig::OAuth {
            client_id: "my-client".to_string(),
            scopes: vec![],
            auth_url: None,
            token_url: None,
        };
        let json = serde_json::to_string(&config).unwrap();
        assert!(!json.contains("auth_url"));
        assert!(!json.contains("token_url"));
    }

    // =========================================================================
    // Forge configuration tests
    // =========================================================================

    #[test]
    fn config_file_with_forge_section() {
        let json = r#"{
            "mcpServers": {"fs": {"command": "fs-server"}},
            "forge": {"enabled": true, "timeout_secs": 45}
        }"#;
        let config: McpConfigFile = serde_json::from_str(json).unwrap();
        assert_eq!(config.mcp_servers.len(), 1);
        let forge = config.forge.unwrap();
        assert!(forge.enabled);
        assert_eq!(forge.timeout_secs, Some(45));
    }

    #[test]
    fn config_file_without_forge_section() {
        let json = r#"{"mcpServers": {"test": {"command": "echo"}}}"#;
        let config: McpConfigFile = serde_json::from_str(json).unwrap();
        assert!(config.forge.is_none());
    }

    #[test]
    fn mcp_config_result_has_forge() {
        let dir = tempfile::TempDir::new().unwrap();
        let config_content = r#"{
            "mcpServers": {"srv": {"command": "echo"}},
            "forge": {"enabled": true}
        }"#;
        std::fs::write(dir.path().join(".mcp.json"), config_content).unwrap();

        let result = load_mcp_config(dir.path()).unwrap();
        assert!(result.servers.contains_key("srv"));
        assert!(result.forge.unwrap().enabled);
    }

    #[test]
    fn mcp_config_result_no_forge_when_absent() {
        let dir = tempfile::TempDir::new().unwrap();
        let config_content = r#"{"mcpServers": {"srv": {"command": "echo"}}}"#;
        std::fs::write(dir.path().join(".mcp.json"), config_content).unwrap();

        let result = load_mcp_config(dir.path()).unwrap();
        assert!(result.servers.contains_key("srv"));
        assert!(result.forge.is_none());
    }
}
