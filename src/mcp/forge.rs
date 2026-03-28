//! Forge (Forgemax) integration for MCP server management.
//!
//! This module provides configuration generation and process preparation for
//! Forgemax, an external MCP server multiplexer. Patina can delegate MCP server
//! lifecycle management to Forge by generating a `forge.toml` configuration from
//! its own MCP server entries.
//!
//! # Overview
//!
//! - [`ForgeSettings`] controls whether Forge integration is enabled and its parameters.
//! - [`generate_forge_toml`] converts Patina's [`McpServerEntry`] map into Forge's TOML format.
//! - [`ForgeContext`] bundles all state needed to spawn a Forge process.
//! - [`detect_forge_binary`] searches `PATH` for the `forgemax` binary.
//!
//! # Example
//!
//! ```no_run
//! use patina::mcp::forge::{ForgeSettings, generate_forge_toml};
//! use std::collections::HashMap;
//!
//! let settings = ForgeSettings::default();
//! let servers = HashMap::new();
//! let toml = generate_forge_toml(&servers, &settings);
//! assert!(toml.contains("[sandbox]"));
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::auth::resolve_bearer_token;
use super::config::{McpAuthConfig, McpServerEntry};

/// Forge integration settings.
///
/// Controls whether the Forgemax binary is used for MCP server management,
/// where to find it, and sandbox parameters for tool execution.
///
/// # Defaults
///
/// Forge is disabled by default. When enabled without an explicit `binary_path`,
/// the binary is discovered via `PATH` lookup.
///
/// # Examples
///
/// ```
/// use patina::mcp::forge::ForgeSettings;
///
/// let settings = ForgeSettings::default();
/// assert!(!settings.enabled);
/// assert!(settings.binary_path.is_none());
/// ```
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ForgeSettings {
    /// Whether Forge integration is enabled.
    pub enabled: bool,
    /// Explicit path to the `forgemax` binary. If `None`, discovered via `PATH`.
    pub binary_path: Option<PathBuf>,
    /// Path to a pre-existing `forge.toml`. If `None`, one is generated.
    pub config_path: Option<PathBuf>,
    /// Whether to auto-generate `forge.toml` from MCP server config.
    pub auto_generate: bool,
    /// Per-server timeout in seconds (written into generated TOML).
    pub timeout_secs: Option<u64>,
    /// Sandbox settings for Forge tool execution.
    pub sandbox: ForgeSandboxSettings,
}

/// Sandbox settings for Forge tool execution.
///
/// These values are written into the `[sandbox]` section of the generated
/// `forge.toml` configuration.
///
/// # Defaults
///
/// | Field | Default |
/// |-------|---------|
/// | `timeout_secs` | 5 |
/// | `max_heap_mb` | 64 |
/// | `max_concurrent` | 8 |
/// | `execution_mode` | `"in_process"` |
///
/// # Examples
///
/// ```
/// use patina::mcp::forge::ForgeSandboxSettings;
///
/// let sandbox = ForgeSandboxSettings::default();
/// assert_eq!(sandbox.timeout_secs, 5);
/// assert_eq!(sandbox.max_heap_mb, 64);
/// assert_eq!(sandbox.max_concurrent, 8);
/// assert_eq!(sandbox.execution_mode, "in_process");
/// ```
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ForgeSandboxSettings {
    /// Sandbox timeout in seconds.
    pub timeout_secs: u64,
    /// Maximum heap size in megabytes.
    pub max_heap_mb: usize,
    /// Maximum number of concurrent tool executions.
    pub max_concurrent: usize,
    /// Execution mode (e.g., `"in_process"`, `"isolated"`).
    pub execution_mode: String,
}

impl Default for ForgeSandboxSettings {
    fn default() -> Self {
        Self {
            timeout_secs: 5,
            max_heap_mb: 64,
            max_concurrent: 8,
            execution_mode: "in_process".to_string(),
        }
    }
}

/// Searches `PATH` for the `forgemax` binary.
///
/// Uses `which` on Unix and `where` on Windows to locate the binary.
/// Returns `Some(path)` if found, `None` otherwise.
///
/// # Examples
///
/// ```no_run
/// use patina::mcp::forge::detect_forge_binary;
///
/// if let Some(path) = detect_forge_binary() {
///     println!("Found forgemax at: {}", path.display());
/// }
/// ```
#[must_use]
pub fn detect_forge_binary() -> Option<PathBuf> {
    #[cfg(unix)]
    let command = "which";
    #[cfg(windows)]
    let command = "where";

    let output = Command::new(command).arg("forgemax").output().ok()?;

    if output.status.success() {
        let path_str = String::from_utf8_lossy(&output.stdout);
        let path = PathBuf::from(path_str.trim());
        if path.as_os_str().is_empty() {
            None
        } else {
            Some(path)
        }
    } else {
        None
    }
}

/// Returns `true` if Forge is enabled and the binary can be found.
///
/// Checks in order:
/// 1. `settings.enabled` must be `true`
/// 2. If `settings.binary_path` is set, that path must exist
/// 3. Otherwise, `forgemax` must be discoverable on `PATH`
///
/// # Examples
///
/// ```
/// use patina::mcp::forge::{ForgeSettings, is_forge_available};
///
/// let settings = ForgeSettings::default();
/// // Disabled by default, so always false
/// assert!(!is_forge_available(&settings));
/// ```
#[must_use]
pub fn is_forge_available(settings: &ForgeSettings) -> bool {
    if !settings.enabled {
        return false;
    }

    if let Some(ref path) = settings.binary_path {
        path.exists()
    } else {
        detect_forge_binary().is_some()
    }
}

// ─── TOML generation internals ──────────────────────────────────────────────

/// TOML-serializable representation of a single Forge server entry.
#[derive(Debug, Serialize)]
struct ForgeServerEntry {
    transport: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    headers: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout_secs: Option<u64>,
}

/// TOML-serializable sandbox section.
#[derive(Debug, Serialize)]
struct ForgeSandboxToml {
    timeout_secs: u64,
    max_heap_mb: usize,
    max_concurrent: usize,
    execution_mode: String,
}

/// Top-level Forge TOML document.
#[derive(Debug, Serialize)]
struct ForgeTomlDoc {
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    servers: HashMap<String, ForgeServerEntry>,
    sandbox: ForgeSandboxToml,
}

/// Generates a `forge.toml` configuration string from MCP server entries.
///
/// Converts each enabled [`McpServerEntry`] into Forge's server format:
/// - Entries with a `url` produce `transport = "sse"` (or the explicit `transport_type`)
/// - Entries with a `command` produce `transport = "stdio"`
/// - Disabled entries are skipped
/// - Bearer auth tokens are resolved and added as `Authorization` headers
///
/// The `[sandbox]` section is always included with values from `settings.sandbox`.
///
/// # Arguments
///
/// * `servers` - Map of server name to MCP server entry
/// * `settings` - Forge settings controlling timeout and sandbox parameters
///
/// # Examples
///
/// ```
/// use patina::mcp::forge::{ForgeSettings, generate_forge_toml};
/// use patina::mcp::config::McpServerEntry;
/// use std::collections::HashMap;
///
/// let mut servers = HashMap::new();
/// servers.insert("test".to_string(), serde_json::from_str::<McpServerEntry>(
///     r#"{"command":"echo","args":["hello"]}"#
/// ).unwrap());
///
/// let toml = generate_forge_toml(&servers, &ForgeSettings::default());
/// assert!(toml.contains("transport = \"stdio\""));
/// ```
#[must_use]
pub fn generate_forge_toml(
    servers: &HashMap<String, McpServerEntry>,
    settings: &ForgeSettings,
) -> String {
    let mut forge_servers = HashMap::new();

    // Sort server names for deterministic output
    let mut names: Vec<&String> = servers.keys().collect();
    names.sort();

    for name in names {
        let entry = &servers[name];

        // Skip disabled servers
        if entry.is_disabled() {
            continue;
        }

        let transport = if entry.url.is_some() {
            entry.transport_type.as_deref().unwrap_or("sse").to_string()
        } else {
            "stdio".to_string()
        };

        let command = if entry.url.is_none() && !entry.command.is_empty() {
            Some(entry.command.clone())
        } else {
            None
        };

        let args = if entry.url.is_none() && !entry.args.is_empty() {
            Some(entry.args.clone())
        } else {
            None
        };

        // Build headers from existing headers + auth
        let mut merged_headers: HashMap<String, String> = HashMap::new();

        if let Some(ref hdrs) = entry.headers {
            merged_headers.extend(hdrs.clone());
        }

        // Resolve bearer auth into Authorization header
        if let Some(McpAuthConfig::Bearer { ref token }) = entry.auth {
            match resolve_bearer_token(token) {
                Ok(resolved) => {
                    merged_headers
                        .insert("Authorization".to_string(), format!("Bearer {resolved}"));
                }
                Err(e) => {
                    tracing::warn!(
                        server = %name,
                        error = %e,
                        "Failed to resolve bearer token for forge.toml generation"
                    );
                }
            }
        }

        let headers = if merged_headers.is_empty() {
            None
        } else {
            Some(merged_headers)
        };

        forge_servers.insert(
            name.clone(),
            ForgeServerEntry {
                transport,
                command,
                args,
                url: entry.url.clone(),
                headers,
                timeout_secs: settings.timeout_secs,
            },
        );
    }

    let doc = ForgeTomlDoc {
        servers: forge_servers,
        sandbox: ForgeSandboxToml {
            timeout_secs: settings.sandbox.timeout_secs,
            max_heap_mb: settings.sandbox.max_heap_mb,
            max_concurrent: settings.sandbox.max_concurrent,
            execution_mode: settings.sandbox.execution_mode.clone(),
        },
    };

    toml::to_string_pretty(&doc).unwrap_or_default()
}

/// Collects and merges environment variables from all MCP server entries.
///
/// When multiple servers define the same environment variable with different
/// values, the last value wins and a warning is logged.
///
/// # Arguments
///
/// * `servers` - Map of server name to MCP server entry
///
/// # Returns
///
/// A merged map of environment variable name to value.
///
/// # Examples
///
/// ```
/// use patina::mcp::config::McpServerEntry;
/// use std::collections::HashMap;
///
/// let servers = HashMap::new();
/// // With no servers, returns empty map
/// ```
#[must_use]
pub fn collect_merged_env(servers: &HashMap<String, McpServerEntry>) -> HashMap<String, String> {
    let mut merged: HashMap<String, String> = HashMap::new();

    // Sort for deterministic merge order
    let mut names: Vec<&String> = servers.keys().collect();
    names.sort();

    for name in names {
        let entry = &servers[name];
        if entry.is_disabled() {
            continue;
        }
        for (key, value) in &entry.env {
            if let Some(existing) = merged.get(key) {
                if existing != value {
                    tracing::warn!(
                        key = %key,
                        server = %name,
                        existing_value = %existing,
                        new_value = %value,
                        "Environment variable conflict across MCP servers — using latest value"
                    );
                }
            }
            merged.insert(key.clone(), value.clone());
        }
    }

    merged
}

/// Prepared context for starting a Forge process.
///
/// Created via [`ForgeContext::prepare`], this struct holds the generated config
/// file path, binary location, merged environment variables, and server count.
///
/// # Lifecycle
///
/// 1. Call [`ForgeContext::prepare`] to generate `forge.toml` and collect env vars
/// 2. Use [`ForgeContext::command`], [`ForgeContext::args`], and [`ForgeContext::env`]
///    to spawn the process
/// 3. Call [`ForgeContext::cleanup`] when done to remove the temporary config file
#[derive(Debug)]
pub struct ForgeContext {
    config_path: PathBuf,
    binary_path: PathBuf,
    merged_env: HashMap<String, String>,
    server_count: usize,
}

impl ForgeContext {
    /// Prepares a Forge context by generating config and collecting environment.
    ///
    /// Generates a `forge.toml` file at `{session_dir}/forge.toml`, collects
    /// merged environment variables from all server entries, and resolves the
    /// binary path.
    ///
    /// # Arguments
    ///
    /// * `settings` - Forge settings (must have a resolvable binary path)
    /// * `servers` - MCP server entries to include in the generated config
    /// * `session_dir` - Directory where the generated `forge.toml` will be written
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The Forge binary cannot be found
    /// - The config file cannot be written to disk
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use patina::mcp::forge::{ForgeSettings, ForgeContext};
    /// use std::collections::HashMap;
    /// use std::path::Path;
    ///
    /// # fn example() -> anyhow::Result<()> {
    /// let settings = ForgeSettings { enabled: true, ..Default::default() };
    /// let servers = HashMap::new();
    /// let ctx = ForgeContext::prepare(&settings, &servers, Path::new("/tmp/session"))?;
    /// println!("Forge manages {} servers", ctx.server_count());
    /// # Ok(())
    /// # }
    /// ```
    pub fn prepare(
        settings: &ForgeSettings,
        servers: &HashMap<String, McpServerEntry>,
        session_dir: &Path,
    ) -> anyhow::Result<Self> {
        // Resolve binary path
        let binary_path = if let Some(ref path) = settings.binary_path {
            if path.exists() {
                path.clone()
            } else {
                anyhow::bail!("Configured forgemax binary not found: {}", path.display());
            }
        } else {
            detect_forge_binary()
                .ok_or_else(|| anyhow::anyhow!("forgemax binary not found on PATH"))?
        };

        // Count enabled servers
        let server_count = servers.values().filter(|e| !e.is_disabled()).count();

        // Generate config
        let toml_content = generate_forge_toml(servers, settings);

        // Write config to session directory
        let config_path = if let Some(ref explicit) = settings.config_path {
            explicit.clone()
        } else {
            session_dir.join("forge.toml")
        };

        std::fs::create_dir_all(config_path.parent().unwrap_or(session_dir))?;
        std::fs::write(&config_path, &toml_content)?;

        // Collect merged env
        let merged_env = collect_merged_env(servers);

        Ok(Self {
            config_path,
            binary_path,
            merged_env,
            server_count,
        })
    }

    /// Returns the path to the Forge binary.
    #[must_use]
    pub fn command(&self) -> &Path {
        &self.binary_path
    }

    /// Returns the arguments to pass to the Forge binary.
    ///
    /// Format: `["serve", "--config", "<config_path>"]`
    #[must_use]
    pub fn args(&self) -> Vec<String> {
        vec![
            "serve".to_string(),
            "--config".to_string(),
            self.config_path.to_string_lossy().to_string(),
        ]
    }

    /// Returns the merged environment variables from all server entries.
    #[must_use]
    pub fn env(&self) -> &HashMap<String, String> {
        &self.merged_env
    }

    /// Returns the number of enabled servers Forge manages.
    #[must_use]
    pub fn server_count(&self) -> usize {
        self.server_count
    }

    /// Removes the generated config file from disk.
    ///
    /// Safe to call multiple times; ignores errors if the file is already gone.
    pub fn cleanup(&self) {
        let _ = std::fs::remove_file(&self.config_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Helper ─────────────────────────────────────────────────────────────

    /// Creates an `McpServerEntry` for testing.
    fn make_entry(json: &str) -> McpServerEntry {
        serde_json::from_str(json).expect("invalid test JSON for McpServerEntry")
    }

    // ─── ForgeSettings tests ────────────────────────────────────────────────

    #[test]
    fn test_forge_settings_default() {
        let settings = ForgeSettings::default();
        assert!(!settings.enabled);
        assert!(settings.binary_path.is_none());
        assert!(settings.config_path.is_none());
        assert!(!settings.auto_generate);
        assert!(settings.timeout_secs.is_none());
    }

    #[test]
    fn test_forge_settings_deserialize_minimal() {
        let json = r#"{"enabled": true}"#;
        let settings: ForgeSettings = serde_json::from_str(json).unwrap();
        assert!(settings.enabled);
        assert!(settings.binary_path.is_none());
        assert!(!settings.auto_generate);
    }

    #[test]
    fn test_forge_settings_deserialize_full() {
        let json = r#"{
            "enabled": true,
            "binary_path": "/usr/local/bin/forgemax",
            "config_path": "/tmp/forge.toml",
            "auto_generate": true,
            "timeout_secs": 30,
            "sandbox": {
                "timeout_secs": 10,
                "max_heap_mb": 128,
                "max_concurrent": 16,
                "execution_mode": "isolated"
            }
        }"#;
        let settings: ForgeSettings = serde_json::from_str(json).unwrap();
        assert!(settings.enabled);
        assert_eq!(
            settings.binary_path.as_deref(),
            Some(Path::new("/usr/local/bin/forgemax"))
        );
        assert_eq!(
            settings.config_path.as_deref(),
            Some(Path::new("/tmp/forge.toml"))
        );
        assert!(settings.auto_generate);
        assert_eq!(settings.timeout_secs, Some(30));
        assert_eq!(settings.sandbox.timeout_secs, 10);
        assert_eq!(settings.sandbox.max_heap_mb, 128);
        assert_eq!(settings.sandbox.max_concurrent, 16);
        assert_eq!(settings.sandbox.execution_mode, "isolated");
    }

    // ─── ForgeSandboxSettings tests ─────────────────────────────────────────

    #[test]
    fn test_sandbox_settings_default() {
        let sandbox = ForgeSandboxSettings::default();
        assert_eq!(sandbox.timeout_secs, 5);
        assert_eq!(sandbox.max_heap_mb, 64);
        assert_eq!(sandbox.max_concurrent, 8);
        assert_eq!(sandbox.execution_mode, "in_process");
    }

    // ─── generate_forge_toml tests ──────────────────────────────────────────

    #[test]
    fn test_generate_forge_toml_stdio_server() {
        let mut servers = HashMap::new();
        servers.insert(
            "filesystem".to_string(),
            make_entry(r#"{"command":"npx","args":["-y","@mcp/server-fs"]}"#),
        );

        let settings = ForgeSettings::default();
        let toml_str = generate_forge_toml(&servers, &settings);

        assert!(toml_str.contains("[servers.filesystem]"));
        assert!(toml_str.contains(r#"transport = "stdio""#));
        assert!(toml_str.contains(r#"command = "npx""#));
        assert!(toml_str.contains("-y"));
        assert!(toml_str.contains("@mcp/server-fs"));
    }

    #[test]
    fn test_generate_forge_toml_sse_server() {
        let mut servers = HashMap::new();
        servers.insert(
            "remote".to_string(),
            make_entry(r#"{"url":"http://localhost:8080/sse"}"#),
        );

        let settings = ForgeSettings::default();
        let toml_str = generate_forge_toml(&servers, &settings);

        assert!(toml_str.contains("[servers.remote]"));
        assert!(toml_str.contains(r#"transport = "sse""#));
        assert!(toml_str.contains(r#"url = "http://localhost:8080/sse""#));
        // Should NOT have command/args for SSE
        assert!(!toml_str.contains("command ="));
    }

    #[test]
    fn test_generate_forge_toml_multiple_servers() {
        let mut servers = HashMap::new();
        servers.insert(
            "fs".to_string(),
            make_entry(r#"{"command":"fs-server","args":["--root","/tmp"]}"#),
        );
        servers.insert("git".to_string(), make_entry(r#"{"command":"git-server"}"#));

        let settings = ForgeSettings::default();
        let toml_str = generate_forge_toml(&servers, &settings);

        assert!(toml_str.contains("[servers.fs]"));
        assert!(toml_str.contains("[servers.git]"));
    }

    #[test]
    fn test_generate_forge_toml_skips_disabled() {
        let mut servers = HashMap::new();
        servers.insert(
            "active".to_string(),
            make_entry(r#"{"command":"active-server"}"#),
        );
        servers.insert(
            "inactive".to_string(),
            make_entry(r#"{"command":"inactive-server","disabled":true}"#),
        );

        let settings = ForgeSettings::default();
        let toml_str = generate_forge_toml(&servers, &settings);

        assert!(toml_str.contains("[servers.active]"));
        assert!(!toml_str.contains("[servers.inactive]"));
        assert!(!toml_str.contains("inactive-server"));
    }

    #[test]
    fn test_generate_forge_toml_with_bearer_auth() {
        // Set env var for token resolution
        std::env::set_var("FORGE_TEST_TOKEN", "secret-123");

        let mut servers = HashMap::new();
        servers.insert(
            "authed".to_string(),
            make_entry(
                r#"{"url":"http://localhost:9090/mcp","auth":{"type":"bearer","token":"$FORGE_TEST_TOKEN"}}"#,
            ),
        );

        let settings = ForgeSettings::default();
        let toml_str = generate_forge_toml(&servers, &settings);

        assert!(toml_str.contains("Authorization"));
        assert!(toml_str.contains("Bearer secret-123"));

        std::env::remove_var("FORGE_TEST_TOKEN");
    }

    #[test]
    fn test_generate_forge_toml_with_headers() {
        let mut servers = HashMap::new();
        servers.insert(
            "custom".to_string(),
            make_entry(
                r#"{"url":"http://localhost:9090/mcp","headers":{"X-Custom":"value","Accept":"application/json"}}"#,
            ),
        );

        let settings = ForgeSettings::default();
        let toml_str = generate_forge_toml(&servers, &settings);

        assert!(toml_str.contains("X-Custom"));
        assert!(toml_str.contains("value"));
        assert!(toml_str.contains("Accept"));
        assert!(toml_str.contains("application/json"));
    }

    #[test]
    fn test_generate_forge_toml_empty() {
        let servers: HashMap<String, McpServerEntry> = HashMap::new();
        let settings = ForgeSettings::default();
        let toml_str = generate_forge_toml(&servers, &settings);

        // Should have sandbox section but no servers section
        assert!(toml_str.contains("[sandbox]"));
        assert!(!toml_str.contains("[servers"));
    }

    #[test]
    fn test_generate_forge_toml_sandbox_settings() {
        let servers: HashMap<String, McpServerEntry> = HashMap::new();
        let mut settings = ForgeSettings::default();
        settings.sandbox.timeout_secs = 10;
        settings.sandbox.max_heap_mb = 256;
        settings.sandbox.max_concurrent = 4;
        settings.sandbox.execution_mode = "isolated".to_string();

        let toml_str = generate_forge_toml(&servers, &settings);

        assert!(toml_str.contains("timeout_secs = 10"));
        assert!(toml_str.contains("max_heap_mb = 256"));
        assert!(toml_str.contains("max_concurrent = 4"));
        assert!(toml_str.contains(r#"execution_mode = "isolated""#));
    }

    #[test]
    fn test_generate_forge_toml_with_timeout() {
        let mut servers = HashMap::new();
        servers.insert("srv".to_string(), make_entry(r#"{"command":"my-server"}"#));

        let settings = ForgeSettings {
            timeout_secs: Some(45),
            ..Default::default()
        };

        let toml_str = generate_forge_toml(&servers, &settings);

        assert!(toml_str.contains("timeout_secs = 45"));
    }

    #[test]
    fn test_generate_forge_toml_streamable_http_transport() {
        let mut servers = HashMap::new();
        servers.insert(
            "modern".to_string(),
            make_entry(r#"{"url":"http://localhost:8080/mcp","type":"streamable-http"}"#),
        );

        let settings = ForgeSettings::default();
        let toml_str = generate_forge_toml(&servers, &settings);

        assert!(toml_str.contains(r#"transport = "streamable-http""#));
    }

    // ─── collect_merged_env tests ───────────────────────────────────────────

    #[test]
    fn test_collect_merged_env() {
        let mut servers = HashMap::new();
        servers.insert(
            "a".to_string(),
            make_entry(r#"{"command":"a-server","env":{"KEY_A":"val_a","SHARED":"from_a"}}"#),
        );
        servers.insert(
            "b".to_string(),
            make_entry(r#"{"command":"b-server","env":{"KEY_B":"val_b","SHARED":"from_b"}}"#),
        );

        let merged = collect_merged_env(&servers);

        assert_eq!(merged.get("KEY_A"), Some(&"val_a".to_string()));
        assert_eq!(merged.get("KEY_B"), Some(&"val_b".to_string()));
        // SHARED is present (last writer wins — "b" comes after "a" alphabetically)
        assert!(merged.contains_key("SHARED"));
        assert_eq!(merged.get("SHARED"), Some(&"from_b".to_string()));
    }

    #[test]
    fn test_collect_merged_env_no_conflict() {
        let mut servers = HashMap::new();
        servers.insert(
            "x".to_string(),
            make_entry(r#"{"command":"x-server","env":{"KEY_X":"val_x"}}"#),
        );
        servers.insert(
            "y".to_string(),
            make_entry(r#"{"command":"y-server","env":{"KEY_Y":"val_y"}}"#),
        );

        let merged = collect_merged_env(&servers);

        assert_eq!(merged.len(), 2);
        assert_eq!(merged.get("KEY_X"), Some(&"val_x".to_string()));
        assert_eq!(merged.get("KEY_Y"), Some(&"val_y".to_string()));
    }

    #[test]
    fn test_collect_merged_env_warns_on_conflict() {
        // This test verifies that conflicting env vars result in the last-writer winning.
        // The warning is logged via tracing (not captured here, but the behavior is correct).
        let mut servers = HashMap::new();
        servers.insert(
            "first".to_string(),
            make_entry(r#"{"command":"first","env":{"CONFLICT":"value_first"}}"#),
        );
        servers.insert(
            "second".to_string(),
            make_entry(r#"{"command":"second","env":{"CONFLICT":"value_second"}}"#),
        );

        let merged = collect_merged_env(&servers);

        // "second" sorts after "first", so "second" wins
        assert_eq!(merged.get("CONFLICT"), Some(&"value_second".to_string()));
        assert_eq!(merged.len(), 1);
    }

    #[test]
    fn test_collect_merged_env_skips_disabled() {
        let mut servers = HashMap::new();
        servers.insert(
            "active".to_string(),
            make_entry(r#"{"command":"active","env":{"ACTIVE_KEY":"yes"}}"#),
        );
        servers.insert(
            "disabled".to_string(),
            make_entry(r#"{"command":"disabled","disabled":true,"env":{"DISABLED_KEY":"no"}}"#),
        );

        let merged = collect_merged_env(&servers);

        assert_eq!(merged.len(), 1);
        assert_eq!(merged.get("ACTIVE_KEY"), Some(&"yes".to_string()));
        assert!(!merged.contains_key("DISABLED_KEY"));
    }

    #[test]
    fn test_collect_merged_env_empty_servers() {
        let servers: HashMap<String, McpServerEntry> = HashMap::new();
        let merged = collect_merged_env(&servers);
        assert!(merged.is_empty());
    }

    // ─── ForgeContext tests ─────────────────────────────────────────────────

    #[test]
    fn test_forge_context_args() {
        let ctx = ForgeContext {
            config_path: PathBuf::from("/tmp/session/forge.toml"),
            binary_path: PathBuf::from("/usr/local/bin/forgemax"),
            merged_env: HashMap::new(),
            server_count: 3,
        };

        let args = ctx.args();
        assert_eq!(args.len(), 3);
        assert_eq!(args[0], "serve");
        assert_eq!(args[1], "--config");
        assert_eq!(args[2], "/tmp/session/forge.toml");
    }

    #[test]
    fn test_forge_context_server_count() {
        let ctx = ForgeContext {
            config_path: PathBuf::from("/tmp/forge.toml"),
            binary_path: PathBuf::from("/usr/bin/forgemax"),
            merged_env: HashMap::new(),
            server_count: 5,
        };

        assert_eq!(ctx.server_count(), 5);
    }

    #[test]
    fn test_forge_context_command() {
        let ctx = ForgeContext {
            config_path: PathBuf::from("/tmp/forge.toml"),
            binary_path: PathBuf::from("/opt/forgemax"),
            merged_env: HashMap::new(),
            server_count: 0,
        };

        assert_eq!(ctx.command(), Path::new("/opt/forgemax"));
    }

    #[test]
    fn test_forge_context_env() {
        let mut env = HashMap::new();
        env.insert("API_KEY".to_string(), "secret".to_string());
        env.insert("PORT".to_string(), "3000".to_string());

        let ctx = ForgeContext {
            config_path: PathBuf::from("/tmp/forge.toml"),
            binary_path: PathBuf::from("/usr/bin/forgemax"),
            merged_env: env,
            server_count: 1,
        };

        assert_eq!(ctx.env().len(), 2);
        assert_eq!(ctx.env().get("API_KEY"), Some(&"secret".to_string()));
        assert_eq!(ctx.env().get("PORT"), Some(&"3000".to_string()));
    }

    #[test]
    fn test_forge_context_cleanup() {
        let dir = tempfile::TempDir::new().unwrap();
        let config_path = dir.path().join("forge.toml");
        std::fs::write(&config_path, "test content").unwrap();
        assert!(config_path.exists());

        let ctx = ForgeContext {
            config_path: config_path.clone(),
            binary_path: PathBuf::from("/usr/bin/forgemax"),
            merged_env: HashMap::new(),
            server_count: 0,
        };

        ctx.cleanup();
        assert!(!config_path.exists());
    }

    #[test]
    fn test_forge_context_cleanup_idempotent() {
        let ctx = ForgeContext {
            config_path: PathBuf::from("/tmp/nonexistent-forge-config.toml"),
            binary_path: PathBuf::from("/usr/bin/forgemax"),
            merged_env: HashMap::new(),
            server_count: 0,
        };

        // Should not panic even if file doesn't exist
        ctx.cleanup();
        ctx.cleanup();
    }

    #[test]
    fn test_forge_context_prepare_writes_config() {
        let dir = tempfile::TempDir::new().unwrap();

        // Create a fake binary so the path check passes
        let fake_binary = dir.path().join("forgemax");
        std::fs::write(&fake_binary, "#!/bin/sh\n").unwrap();

        let settings = ForgeSettings {
            enabled: true,
            binary_path: Some(fake_binary.clone()),
            config_path: None,
            auto_generate: true,
            timeout_secs: None,
            sandbox: ForgeSandboxSettings::default(),
        };

        let mut servers = HashMap::new();
        servers.insert(
            "test".to_string(),
            make_entry(r#"{"command":"echo","args":["hello"]}"#),
        );

        let session_dir = dir.path().join("session");
        std::fs::create_dir_all(&session_dir).unwrap();

        let ctx = ForgeContext::prepare(&settings, &servers, &session_dir).unwrap();

        // Config file should exist
        let config_path = session_dir.join("forge.toml");
        assert!(config_path.exists());

        // Read and verify content
        let content = std::fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("[servers.test]"));
        assert!(content.contains(r#"transport = "stdio""#));
        assert!(content.contains("[sandbox]"));

        assert_eq!(ctx.server_count(), 1);
        assert_eq!(ctx.command(), fake_binary.as_path());

        ctx.cleanup();
        assert!(!config_path.exists());
    }

    #[test]
    fn test_forge_context_prepare_missing_binary() {
        let dir = tempfile::TempDir::new().unwrap();

        let settings = ForgeSettings {
            enabled: true,
            binary_path: Some(PathBuf::from("/nonexistent/forgemax")),
            config_path: None,
            auto_generate: true,
            timeout_secs: None,
            sandbox: ForgeSandboxSettings::default(),
        };

        let servers = HashMap::new();
        let result = ForgeContext::prepare(&settings, &servers, dir.path());

        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("not found"),
            "Error should mention binary not found"
        );
    }

    // ─── is_forge_available tests ───────────────────────────────────────────

    #[test]
    fn test_is_forge_available_disabled() {
        let settings = ForgeSettings::default();
        assert!(!is_forge_available(&settings));
    }

    #[test]
    fn test_is_forge_available_explicit_path_missing() {
        let settings = ForgeSettings {
            enabled: true,
            binary_path: Some(PathBuf::from("/nonexistent/forgemax")),
            ..Default::default()
        };
        assert!(!is_forge_available(&settings));
    }

    #[test]
    fn test_is_forge_available_explicit_path_exists() {
        let dir = tempfile::TempDir::new().unwrap();
        let fake_binary = dir.path().join("forgemax");
        std::fs::write(&fake_binary, "#!/bin/sh\n").unwrap();

        let settings = ForgeSettings {
            enabled: true,
            binary_path: Some(fake_binary),
            ..Default::default()
        };
        assert!(is_forge_available(&settings));
    }

    // ─── generate_forge_toml round-trip parse test ──────────────────────────

    #[test]
    fn test_generate_forge_toml_is_valid_toml() {
        let mut servers = HashMap::new();
        servers.insert(
            "stdio".to_string(),
            make_entry(r#"{"command":"echo","args":["hello"]}"#),
        );
        servers.insert(
            "sse".to_string(),
            make_entry(r#"{"url":"http://localhost:8080/sse"}"#),
        );

        let settings = ForgeSettings {
            timeout_secs: Some(30),
            ..Default::default()
        };

        let toml_str = generate_forge_toml(&servers, &settings);

        // Must be parseable as valid TOML
        let parsed: toml::Value = toml::from_str(&toml_str)
            .unwrap_or_else(|e| panic!("Generated TOML is invalid: {e}\n\n{toml_str}"));

        // Verify structure
        let servers_table = parsed.get("servers").expect("missing [servers]");
        assert!(servers_table.get("stdio").is_some());
        assert!(servers_table.get("sse").is_some());

        let sandbox = parsed.get("sandbox").expect("missing [sandbox]");
        assert_eq!(
            sandbox.get("timeout_secs").and_then(|v| v.as_integer()),
            Some(5)
        );
    }

    #[test]
    fn test_generate_forge_toml_bearer_auth_literal_token() {
        let mut servers = HashMap::new();
        servers.insert(
            "api".to_string(),
            make_entry(
                r#"{"url":"http://api.example.com/mcp","auth":{"type":"bearer","token":"literal-token-value"}}"#,
            ),
        );

        let settings = ForgeSettings::default();
        let toml_str = generate_forge_toml(&servers, &settings);

        assert!(toml_str.contains("Authorization"));
        assert!(toml_str.contains("Bearer literal-token-value"));
    }

    #[test]
    fn test_generate_forge_toml_headers_merged_with_auth() {
        std::env::set_var("FORGE_MERGE_TEST_TOKEN", "tok-999");

        let mut servers = HashMap::new();
        servers.insert(
            "merged".to_string(),
            make_entry(
                r#"{"url":"http://example.com/mcp","headers":{"X-Custom":"abc"},"auth":{"type":"bearer","token":"$FORGE_MERGE_TEST_TOKEN"}}"#,
            ),
        );

        let settings = ForgeSettings::default();
        let toml_str = generate_forge_toml(&servers, &settings);

        assert!(toml_str.contains("X-Custom"));
        assert!(toml_str.contains("abc"));
        assert!(toml_str.contains("Authorization"));
        assert!(toml_str.contains("Bearer tok-999"));

        std::env::remove_var("FORGE_MERGE_TEST_TOKEN");
    }
}
