//! Plugin manifest parsing and validation.
//!
//! This module handles the `rct-plugin.toml` manifest format used to define
//! plugin metadata, capabilities, and configuration.
//!
//! # Example Manifest
//!
//! ```toml
//! name = "my-plugin"
//! version = "1.0.0"
//! description = "A sample plugin"
//!
//! [capabilities]
//! commands = true
//! skills = true
//! ```

use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::Path;
use thiserror::Error;

/// Regex for valid plugin names: lowercase alphanumeric with hyphens.
static NAME_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[a-z][a-z0-9-]*$").unwrap());

/// Regex for valid semver versions.
static VERSION_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\d+\.\d+\.\d+(-[a-zA-Z0-9.]+)?(\+[a-zA-Z0-9.]+)?$").unwrap());

/// Errors that can occur when parsing or validating a manifest.
#[derive(Debug, Error)]
pub enum ManifestError {
    /// A required field is missing.
    #[error("missing required field: {0}")]
    MissingField(String),

    /// The version string is not valid semver.
    #[error("invalid version format: {0}")]
    InvalidVersion(String),

    /// The plugin name is invalid.
    #[error("invalid plugin name: {0}")]
    InvalidName(String),

    /// A capability is invalid or misconfigured.
    #[error("invalid capability: {0}")]
    InvalidCapability(String),

    /// TOML parsing error.
    #[error("TOML parse error: {0}")]
    TomlError(#[from] toml::de::Error),

    /// IO error when reading manifest file.
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Plugin capabilities that can be declared in the manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    /// Plugin provides slash commands.
    Commands,
    /// Plugin provides skills.
    Skills,
    /// Plugin provides tools for the agent.
    Tools,
    /// Plugin provides lifecycle hooks.
    Hooks,
    /// Plugin provides an MCP server.
    Mcp,
}

/// MCP server configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpConfig {
    /// Command to run the MCP server.
    pub command: String,
    /// Arguments to pass to the command.
    pub args: Vec<String>,
    /// Whether to start the server automatically.
    pub auto_start: bool,
}

/// Plugin configuration option.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigOption {
    /// Type of the configuration value.
    pub config_type: String,
    /// Default value (as string).
    pub default: Option<String>,
    /// Description of the option.
    pub description: Option<String>,
}

/// Parsed plugin manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// Plugin name (must be a valid identifier).
    pub name: String,
    /// Plugin version (semver).
    pub version: String,
    /// Optional description.
    pub description: Option<String>,
    /// Optional author.
    pub author: Option<String>,
    /// Optional license.
    pub license: Option<String>,
    /// Optional homepage URL.
    pub homepage: Option<String>,
    /// Optional repository URL.
    pub repository: Option<String>,
    /// Minimum Patina version required.
    pub min_patina_version: Option<String>,
    capabilities: HashMap<Capability, bool>,
    mcp_config: Option<McpConfig>,
    config_options: HashMap<String, ConfigOption>,
    dependencies: HashMap<String, String>,
}

/// Raw TOML structure for deserialization.
#[derive(Debug, Deserialize)]
struct RawManifest {
    name: Option<String>,
    version: Option<String>,
    description: Option<String>,
    author: Option<String>,
    license: Option<String>,
    homepage: Option<String>,
    repository: Option<String>,
    min_patina_version: Option<String>,
    #[serde(default)]
    capabilities: RawCapabilities,
    mcp: Option<RawMcpConfig>,
    #[serde(default)]
    config: HashMap<String, RawConfigOption>,
    #[serde(default)]
    dependencies: HashMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawCapabilities {
    #[serde(default)]
    commands: bool,
    #[serde(default)]
    skills: bool,
    #[serde(default)]
    tools: bool,
    #[serde(default)]
    hooks: bool,
    #[serde(default)]
    mcp: bool,
}

#[derive(Debug, Deserialize)]
struct RawMcpConfig {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    auto_start: bool,
}

#[derive(Debug, Deserialize)]
struct RawConfigOption {
    #[serde(rename = "type")]
    config_type: String,
    default: Option<toml::Value>,
    description: Option<String>,
}

impl Manifest {
    /// Parse a manifest from a TOML string.
    ///
    /// # Errors
    ///
    /// Returns an error if the TOML is invalid or required fields are missing.
    pub fn from_toml(content: &str) -> Result<Self, ManifestError> {
        let raw: RawManifest = toml::from_str(content)?;

        // Validate required fields
        let name = raw
            .name
            .ok_or_else(|| ManifestError::MissingField("name".to_string()))?;

        let version = raw
            .version
            .ok_or_else(|| ManifestError::MissingField("version".to_string()))?;

        // Validate name format
        if !NAME_REGEX.is_match(&name) {
            return Err(ManifestError::InvalidName(format!(
                "'{}' must be lowercase alphanumeric with hyphens",
                name
            )));
        }

        // Validate version format
        if !VERSION_REGEX.is_match(&version) {
            return Err(ManifestError::InvalidVersion(format!(
                "'{}' is not a valid semver version",
                version
            )));
        }

        // Build capabilities map
        let mut capabilities = HashMap::new();
        capabilities.insert(Capability::Commands, raw.capabilities.commands);
        capabilities.insert(Capability::Skills, raw.capabilities.skills);
        capabilities.insert(Capability::Tools, raw.capabilities.tools);
        capabilities.insert(Capability::Hooks, raw.capabilities.hooks);
        capabilities.insert(Capability::Mcp, raw.capabilities.mcp);

        // Validate MCP capability requires MCP config
        if raw.capabilities.mcp && raw.mcp.is_none() {
            return Err(ManifestError::InvalidCapability(
                "mcp capability requires [mcp] configuration section".to_string(),
            ));
        }

        // Convert MCP config
        let mcp_config = raw.mcp.map(|m| McpConfig {
            command: m.command,
            args: m.args,
            auto_start: m.auto_start,
        });

        // Convert config options
        let config_options: HashMap<String, ConfigOption> = raw
            .config
            .into_iter()
            .map(|(k, v)| {
                (
                    k,
                    ConfigOption {
                        config_type: v.config_type,
                        default: v.default.map(|d| toml_value_to_string(&d)),
                        description: v.description,
                    },
                )
            })
            .collect();

        Ok(Self {
            name,
            version,
            description: raw.description,
            author: raw.author,
            license: raw.license,
            homepage: raw.homepage,
            repository: raw.repository,
            min_patina_version: raw.min_patina_version,
            capabilities,
            mcp_config,
            config_options,
            dependencies: raw.dependencies,
        })
    }

    /// Parse a manifest from a file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or the manifest is invalid.
    pub fn from_file(path: &Path) -> Result<Self, ManifestError> {
        let content = fs::read_to_string(path)?;
        Self::from_toml(&content)
    }

    /// Check if a capability is enabled.
    #[must_use]
    pub fn has_capability(&self, capability: Capability) -> bool {
        self.capabilities.get(&capability).copied().unwrap_or(false)
    }

    /// Get the MCP configuration if present.
    #[must_use]
    pub fn mcp_config(&self) -> Option<&McpConfig> {
        self.mcp_config.as_ref()
    }

    /// Get the plugin configuration options.
    #[must_use]
    pub fn config(&self) -> &HashMap<String, ConfigOption> {
        &self.config_options
    }

    /// Get plugin dependencies.
    #[must_use]
    pub fn dependencies(&self) -> &HashMap<String, String> {
        &self.dependencies
    }
}

impl fmt::Display for Manifest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} v{}", self.name, self.version)
    }
}

/// Convert a TOML value to a string representation.
fn toml_value_to_string(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => s.clone(),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => f.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        toml::Value::Datetime(d) => d.to_string(),
        toml::Value::Array(a) => format!("{:?}", a),
        toml::Value::Table(t) => format!("{:?}", t),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name_regex_valid() {
        assert!(NAME_REGEX.is_match("narsil"));
        assert!(NAME_REGEX.is_match("my-plugin"));
        assert!(NAME_REGEX.is_match("plugin123"));
        assert!(NAME_REGEX.is_match("a"));
    }

    #[test]
    fn test_name_regex_invalid() {
        assert!(!NAME_REGEX.is_match("My-Plugin")); // uppercase
        assert!(!NAME_REGEX.is_match("my plugin")); // space
        assert!(!NAME_REGEX.is_match("123plugin")); // starts with number
        assert!(!NAME_REGEX.is_match("_plugin")); // starts with underscore
        assert!(!NAME_REGEX.is_match("")); // empty
    }

    #[test]
    fn test_version_regex_valid() {
        assert!(VERSION_REGEX.is_match("1.0.0"));
        assert!(VERSION_REGEX.is_match("0.1.0"));
        assert!(VERSION_REGEX.is_match("10.20.30"));
        assert!(VERSION_REGEX.is_match("1.0.0-alpha"));
        assert!(VERSION_REGEX.is_match("1.0.0-beta.1"));
        assert!(VERSION_REGEX.is_match("1.0.0+build.123"));
    }

    #[test]
    fn test_version_regex_invalid() {
        assert!(!VERSION_REGEX.is_match("1.0")); // missing patch
        assert!(!VERSION_REGEX.is_match("v1.0.0")); // starts with v
        assert!(!VERSION_REGEX.is_match("not-semver"));
    }
}
