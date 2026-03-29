//! Managed settings for enterprise policy enforcement.
//!
//! This module allows organizations to enforce policies such as plugin
//! allowlists/denylists, MCP server restrictions, default models, and
//! permission rules. Settings are loaded from system-wide JSON files
//! and merged in alphabetical order.
//!
//! # Architecture
//!
//! ```text
//! /managed-settings.json          (base settings)
//! /managed-settings.d/*.json      (overlay settings, alphabetical merge)
//!     ↓
//! ManagedSettings
//!     ├─ permissions      → enforced permission rules
//!     ├─ allowed_plugins  → plugin allowlist
//!     ├─ denied_plugins   → plugin denylist
//!     ├─ allowed_mcp_servers → MCP server allowlist
//!     ├─ denied_mcp_servers  → MCP server denylist
//!     ├─ default_model    → enforced model
//!     ├─ allow_managed_hooks_only → restrict hooks
//!     └─ allow_managed_permissions_only → restrict permissions
//! ```
//!
//! # Example
//!
//! ```
//! use patina::enterprise::managed_settings::{
//!     ManagedSettings, ManagedPermissions, ManagedPermissionRule,
//!     is_plugin_allowed, is_mcp_server_allowed, get_enforced_permission_mode,
//!     merge_managed_settings,
//! };
//!
//! let settings = ManagedSettings {
//!     permissions: Some(ManagedPermissions {
//!         default_mode: Some("deny".to_string()),
//!         rules: vec![ManagedPermissionRule {
//!             tool: "Read".to_string(),
//!             pattern: None,
//!             mode: "allow".to_string(),
//!         }],
//!     }),
//!     allowed_plugins: Some(vec!["safe-plugin".to_string()]),
//!     denied_plugins: None,
//!     allowed_mcp_servers: None,
//!     denied_mcp_servers: Some(vec!["untrusted-server".to_string()]),
//!     default_model: Some("claude-3-sonnet".to_string()),
//!     allow_managed_hooks_only: true,
//!     allow_managed_permissions_only: false,
//! };
//!
//! assert!(is_plugin_allowed("safe-plugin", &settings));
//! assert!(!is_plugin_allowed("unknown-plugin", &settings));
//! assert!(!is_mcp_server_allowed("untrusted-server", &settings));
//! assert_eq!(get_enforced_permission_mode(&settings), Some("deny"));
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

/// System-wide managed settings file path.
const MANAGED_SETTINGS_PATH: &str = "/managed-settings.json";

/// Directory containing additional managed settings files.
const MANAGED_SETTINGS_DIR: &str = "/managed-settings.d";

/// A single permission rule enforced by managed settings.
///
/// Each rule maps a tool name (and optional input pattern) to a permission
/// mode that overrides user-level configuration.
///
/// # Example
///
/// ```
/// use patina::enterprise::managed_settings::ManagedPermissionRule;
///
/// let rule = ManagedPermissionRule {
///     tool: "Bash".to_string(),
///     pattern: Some("git *".to_string()),
///     mode: "allow".to_string(),
/// };
/// assert_eq!(rule.tool, "Bash");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagedPermissionRule {
    /// The tool name this rule applies to (e.g., "Bash", "Read").
    pub tool: String,
    /// Optional input pattern to match (e.g., "git *").
    pub pattern: Option<String>,
    /// Permission mode: "ask", "allow", or "deny".
    pub mode: String,
}

/// Managed permission configuration enforced by the organization.
///
/// Contains a default mode and a list of specific rules that override it.
///
/// # Example
///
/// ```
/// use patina::enterprise::managed_settings::{ManagedPermissions, ManagedPermissionRule};
///
/// let perms = ManagedPermissions {
///     default_mode: Some("ask".to_string()),
///     rules: vec![],
/// };
/// assert_eq!(perms.default_mode.as_deref(), Some("ask"));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagedPermissions {
    /// Default permission mode: "ask", "allow", or "deny".
    pub default_mode: Option<String>,
    /// Enforced permission rules that override user preferences.
    #[serde(default)]
    pub rules: Vec<ManagedPermissionRule>,
}

/// Enterprise managed settings for organization policy enforcement.
///
/// These settings are loaded from system-wide configuration files and
/// take precedence over user-level settings. Organizations use these
/// to enforce security policies, restrict tool access, and standardize
/// configurations across all users.
///
/// # Example
///
/// ```
/// use patina::enterprise::managed_settings::ManagedSettings;
///
/// let settings = ManagedSettings::default();
/// assert!(settings.permissions.is_none());
/// assert!(!settings.allow_managed_hooks_only);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ManagedSettings {
    /// Enforced permission rules.
    pub permissions: Option<ManagedPermissions>,
    /// Plugin allowlist. If `Some`, only these plugins may be loaded.
    pub allowed_plugins: Option<Vec<String>>,
    /// Plugin denylist. These plugins are always blocked.
    pub denied_plugins: Option<Vec<String>>,
    /// MCP server allowlist. If `Some`, only these servers may connect.
    pub allowed_mcp_servers: Option<Vec<String>>,
    /// MCP server denylist. These servers are always blocked.
    pub denied_mcp_servers: Option<Vec<String>>,
    /// Enforced default model override.
    pub default_model: Option<String>,
    /// When `true`, only hooks defined in managed settings are allowed.
    #[serde(default)]
    pub allow_managed_hooks_only: bool,
    /// When `true`, only permission rules from managed settings apply.
    #[serde(default)]
    pub allow_managed_permissions_only: bool,
}

/// Loads managed settings from the system-wide configuration file.
///
/// Reads from `/managed-settings.json`. Returns `None` if the file does
/// not exist. Returns an error if the file exists but cannot be parsed.
///
/// # Errors
///
/// Returns an error if the file exists but cannot be read or contains
/// invalid JSON.
///
/// # Example
///
/// ```no_run
/// use patina::enterprise::managed_settings::load_managed_settings;
///
/// match load_managed_settings() {
///     Ok(Some(settings)) => println!("Loaded managed settings"),
///     Ok(None) => println!("No managed settings file found"),
///     Err(e) => eprintln!("Error loading settings: {e}"),
/// }
/// ```
pub fn load_managed_settings() -> Result<Option<ManagedSettings>> {
    load_managed_settings_from(Path::new(MANAGED_SETTINGS_PATH))
}

/// Loads managed settings from a specific file path.
///
/// Returns `None` if the file does not exist. Returns an error if the
/// file exists but cannot be parsed.
///
/// # Errors
///
/// Returns an error if the file exists but cannot be read or contains
/// invalid JSON.
#[must_use = "the loaded settings should be checked and applied"]
pub fn load_managed_settings_from(path: &Path) -> Result<Option<ManagedSettings>> {
    if !path.exists() {
        debug!(path = %path.display(), "Managed settings file not found");
        return Ok(None);
    }

    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read managed settings from {}", path.display()))?;

    let settings: ManagedSettings = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse managed settings from {}", path.display()))?;

    debug!(path = %path.display(), "Loaded managed settings");
    Ok(Some(settings))
}

/// Loads and merges all managed settings from the drop-in directory.
///
/// Reads all `.json` files from `/managed-settings.d/` in alphabetical
/// order and returns them as a vector. Use [`merge_all_managed_settings`]
/// to combine them into a single settings object.
///
/// # Errors
///
/// Returns an error if a file exists but cannot be read or parsed.
///
/// # Example
///
/// ```no_run
/// use patina::enterprise::managed_settings::load_managed_settings_dir;
///
/// match load_managed_settings_dir() {
///     Ok(settings_list) => println!("Loaded {} overlay files", settings_list.len()),
///     Err(e) => eprintln!("Error loading settings directory: {e}"),
/// }
/// ```
pub fn load_managed_settings_dir() -> Result<Vec<ManagedSettings>> {
    load_managed_settings_dir_from(Path::new(MANAGED_SETTINGS_DIR))
}

/// Loads all managed settings from a specific directory.
///
/// Files are loaded in alphabetical order by filename. Only files
/// with a `.json` extension are processed.
///
/// # Errors
///
/// Returns an error if the directory cannot be read or any JSON file
/// within it cannot be parsed.
pub fn load_managed_settings_dir_from(dir: &Path) -> Result<Vec<ManagedSettings>> {
    if !dir.exists() {
        debug!(dir = %dir.display(), "Managed settings directory not found");
        return Ok(Vec::new());
    }

    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .with_context(|| {
            format!(
                "Failed to read managed settings directory {}",
                dir.display()
            )
        })?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                Some(path)
            } else {
                None
            }
        })
        .collect();

    // Sort alphabetically for deterministic merge order
    entries.sort();

    let mut result = Vec::with_capacity(entries.len());
    for path in &entries {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read managed settings from {}", path.display()))?;

        let settings: ManagedSettings = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse managed settings from {}", path.display()))?;

        debug!(path = %path.display(), "Loaded managed settings overlay");
        result.push(settings);
    }

    Ok(result)
}

/// Merges two managed settings, with `overlay` values taking precedence.
///
/// For `Option` fields, the overlay replaces the base if `Some`.
/// For `Vec` fields within `ManagedPermissions`, the overlay's rules
/// are appended to the base's rules.
/// For `bool` fields, overlay `true` takes precedence (OR semantics).
///
/// # Example
///
/// ```
/// use patina::enterprise::managed_settings::{ManagedSettings, merge_managed_settings};
///
/// let base = ManagedSettings {
///     default_model: Some("claude-3-haiku".to_string()),
///     ..Default::default()
/// };
/// let overlay = ManagedSettings {
///     default_model: Some("claude-3-sonnet".to_string()),
///     ..Default::default()
/// };
/// let merged = merge_managed_settings(base, overlay);
/// assert_eq!(merged.default_model.as_deref(), Some("claude-3-sonnet"));
/// ```
#[must_use = "the merged settings should be applied"]
pub fn merge_managed_settings(base: ManagedSettings, overlay: ManagedSettings) -> ManagedSettings {
    ManagedSettings {
        permissions: merge_permissions(base.permissions, overlay.permissions),
        allowed_plugins: overlay.allowed_plugins.or(base.allowed_plugins),
        denied_plugins: overlay.denied_plugins.or(base.denied_plugins),
        allowed_mcp_servers: overlay.allowed_mcp_servers.or(base.allowed_mcp_servers),
        denied_mcp_servers: overlay.denied_mcp_servers.or(base.denied_mcp_servers),
        default_model: overlay.default_model.or(base.default_model),
        allow_managed_hooks_only: base.allow_managed_hooks_only || overlay.allow_managed_hooks_only,
        allow_managed_permissions_only: base.allow_managed_permissions_only
            || overlay.allow_managed_permissions_only,
    }
}

/// Merges all settings from a base and a list of overlays in order.
///
/// This is a convenience function that applies [`merge_managed_settings`]
/// sequentially across all overlays.
///
/// # Example
///
/// ```
/// use patina::enterprise::managed_settings::{ManagedSettings, merge_all_managed_settings};
///
/// let base = ManagedSettings::default();
/// let overlays = vec![
///     ManagedSettings { default_model: Some("model-a".to_string()), ..Default::default() },
///     ManagedSettings { default_model: Some("model-b".to_string()), ..Default::default() },
/// ];
/// let merged = merge_all_managed_settings(base, overlays);
/// assert_eq!(merged.default_model.as_deref(), Some("model-b"));
/// ```
#[must_use = "the merged settings should be applied"]
pub fn merge_all_managed_settings(
    base: ManagedSettings,
    overlays: Vec<ManagedSettings>,
) -> ManagedSettings {
    overlays
        .into_iter()
        .fold(base, merge_managed_settings)
}

/// Checks whether a plugin is allowed by managed settings.
///
/// Enforcement logic:
/// 1. If the plugin is in `denied_plugins`, it is denied.
/// 2. If `allowed_plugins` is `Some`, only listed plugins are allowed.
/// 3. If neither list applies, the plugin is allowed by default.
///
/// # Example
///
/// ```
/// use patina::enterprise::managed_settings::{ManagedSettings, is_plugin_allowed};
///
/// let settings = ManagedSettings {
///     allowed_plugins: Some(vec!["trusted".to_string()]),
///     ..Default::default()
/// };
/// assert!(is_plugin_allowed("trusted", &settings));
/// assert!(!is_plugin_allowed("untrusted", &settings));
/// ```
#[must_use = "the permission check result should be used to gate plugin loading"]
pub fn is_plugin_allowed(name: &str, settings: &ManagedSettings) -> bool {
    // Check denylist first
    if let Some(ref denied) = settings.denied_plugins {
        if denied.iter().any(|d| d == name) {
            warn!(plugin = %name, "Plugin denied by managed settings denylist");
            return false;
        }
    }

    // Check allowlist
    if let Some(ref allowed) = settings.allowed_plugins {
        let is_allowed = allowed.iter().any(|a| a == name);
        if !is_allowed {
            warn!(plugin = %name, "Plugin not in managed settings allowlist");
        }
        return is_allowed;
    }

    // No restrictions
    true
}

/// Checks whether an MCP server is allowed by managed settings.
///
/// Enforcement logic:
/// 1. If the server is in `denied_mcp_servers`, it is denied.
/// 2. If `allowed_mcp_servers` is `Some`, only listed servers are allowed.
/// 3. If neither list applies, the server is allowed by default.
///
/// # Example
///
/// ```
/// use patina::enterprise::managed_settings::{ManagedSettings, is_mcp_server_allowed};
///
/// let settings = ManagedSettings {
///     denied_mcp_servers: Some(vec!["bad-server".to_string()]),
///     ..Default::default()
/// };
/// assert!(!is_mcp_server_allowed("bad-server", &settings));
/// assert!(is_mcp_server_allowed("good-server", &settings));
/// ```
#[must_use = "the permission check result should be used to gate MCP server connections"]
pub fn is_mcp_server_allowed(name: &str, settings: &ManagedSettings) -> bool {
    // Check denylist first
    if let Some(ref denied) = settings.denied_mcp_servers {
        if denied.iter().any(|d| d == name) {
            warn!(mcp_server = %name, "MCP server denied by managed settings denylist");
            return false;
        }
    }

    // Check allowlist
    if let Some(ref allowed) = settings.allowed_mcp_servers {
        let is_allowed = allowed.iter().any(|a| a == name);
        if !is_allowed {
            warn!(mcp_server = %name, "MCP server not in managed settings allowlist");
        }
        return is_allowed;
    }

    // No restrictions
    true
}

/// Returns the enforced default permission mode from managed settings.
///
/// Returns `None` if no managed permissions are configured or if the
/// permissions have no `default_mode` set.
///
/// # Example
///
/// ```
/// use patina::enterprise::managed_settings::{
///     ManagedSettings, ManagedPermissions, get_enforced_permission_mode,
/// };
///
/// let settings = ManagedSettings {
///     permissions: Some(ManagedPermissions {
///         default_mode: Some("deny".to_string()),
///         rules: vec![],
///     }),
///     ..Default::default()
/// };
/// assert_eq!(get_enforced_permission_mode(&settings), Some("deny"));
/// ```
#[must_use = "the enforced mode should be applied to the permission manager"]
pub fn get_enforced_permission_mode(settings: &ManagedSettings) -> Option<&str> {
    settings
        .permissions
        .as_ref()
        .and_then(|p| p.default_mode.as_deref())
}

/// Merges two optional `ManagedPermissions` values.
///
/// When both are `Some`, the overlay's `default_mode` wins (if set),
/// and rules are concatenated (base rules first, overlay rules second).
fn merge_permissions(
    base: Option<ManagedPermissions>,
    overlay: Option<ManagedPermissions>,
) -> Option<ManagedPermissions> {
    match (base, overlay) {
        (None, None) => None,
        (Some(b), None) => Some(b),
        (None, Some(o)) => Some(o),
        (Some(b), Some(o)) => {
            let mut rules = b.rules;
            rules.extend(o.rules);
            Some(ManagedPermissions {
                default_mode: o.default_mode.or(b.default_mode),
                rules,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // =========================================================================
    // Struct construction and default tests
    // =========================================================================

    #[test]
    fn test_managed_settings_default() {
        let settings = ManagedSettings::default();
        assert!(settings.permissions.is_none());
        assert!(settings.allowed_plugins.is_none());
        assert!(settings.denied_plugins.is_none());
        assert!(settings.allowed_mcp_servers.is_none());
        assert!(settings.denied_mcp_servers.is_none());
        assert!(settings.default_model.is_none());
        assert!(!settings.allow_managed_hooks_only);
        assert!(!settings.allow_managed_permissions_only);
    }

    #[test]
    fn test_managed_permission_rule_serialization() {
        let rule = ManagedPermissionRule {
            tool: "Bash".to_string(),
            pattern: Some("git *".to_string()),
            mode: "allow".to_string(),
        };
        let json = serde_json::to_string(&rule).unwrap();
        let deserialized: ManagedPermissionRule = serde_json::from_str(&json).unwrap();
        assert_eq!(rule, deserialized);
    }

    #[test]
    fn test_managed_permissions_serialization() {
        let perms = ManagedPermissions {
            default_mode: Some("deny".to_string()),
            rules: vec![ManagedPermissionRule {
                tool: "Read".to_string(),
                pattern: None,
                mode: "allow".to_string(),
            }],
        };
        let json = serde_json::to_string(&perms).unwrap();
        let deserialized: ManagedPermissions = serde_json::from_str(&json).unwrap();
        assert_eq!(perms, deserialized);
    }

    #[test]
    fn test_managed_settings_full_serialization() {
        let settings = ManagedSettings {
            permissions: Some(ManagedPermissions {
                default_mode: Some("ask".to_string()),
                rules: vec![],
            }),
            allowed_plugins: Some(vec!["plugin-a".to_string()]),
            denied_plugins: Some(vec!["plugin-b".to_string()]),
            allowed_mcp_servers: Some(vec!["server-a".to_string()]),
            denied_mcp_servers: Some(vec!["server-b".to_string()]),
            default_model: Some("claude-3-sonnet".to_string()),
            allow_managed_hooks_only: true,
            allow_managed_permissions_only: true,
        };
        let json = serde_json::to_string_pretty(&settings).unwrap();
        let deserialized: ManagedSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(settings, deserialized);
    }

    // =========================================================================
    // Loading from JSON file
    // =========================================================================

    #[test]
    fn test_load_from_json_file() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("managed-settings.json");

        let settings = ManagedSettings {
            default_model: Some("claude-3-opus".to_string()),
            allow_managed_hooks_only: true,
            ..Default::default()
        };
        let json = serde_json::to_string_pretty(&settings).unwrap();
        fs::write(&path, json).unwrap();

        let loaded = load_managed_settings_from(&path).unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.default_model.as_deref(), Some("claude-3-opus"));
        assert!(loaded.allow_managed_hooks_only);
    }

    #[test]
    fn test_missing_file_returns_none() {
        let result = load_managed_settings_from(Path::new("/nonexistent/managed-settings.json"));
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_malformed_json_returns_error() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("bad-settings.json");
        fs::write(&path, "{ this is not valid json }").unwrap();

        let result = load_managed_settings_from(&path);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Failed to parse"),
            "Error should mention parsing failure: {err_msg}"
        );
    }

    // =========================================================================
    // Merging two settings (overlay wins)
    // =========================================================================

    #[test]
    fn test_merge_overlay_wins_for_model() {
        let base = ManagedSettings {
            default_model: Some("claude-3-haiku".to_string()),
            ..Default::default()
        };
        let overlay = ManagedSettings {
            default_model: Some("claude-3-sonnet".to_string()),
            ..Default::default()
        };
        let merged = merge_managed_settings(base, overlay);
        assert_eq!(merged.default_model.as_deref(), Some("claude-3-sonnet"));
    }

    #[test]
    fn test_merge_overlay_none_keeps_base() {
        let base = ManagedSettings {
            default_model: Some("claude-3-haiku".to_string()),
            allowed_plugins: Some(vec!["base-plugin".to_string()]),
            ..Default::default()
        };
        let overlay = ManagedSettings::default();
        let merged = merge_managed_settings(base, overlay);
        assert_eq!(merged.default_model.as_deref(), Some("claude-3-haiku"));
        assert_eq!(
            merged.allowed_plugins,
            Some(vec!["base-plugin".to_string()])
        );
    }

    #[test]
    fn test_merge_bool_or_semantics() {
        let base = ManagedSettings {
            allow_managed_hooks_only: true,
            allow_managed_permissions_only: false,
            ..Default::default()
        };
        let overlay = ManagedSettings {
            allow_managed_hooks_only: false,
            allow_managed_permissions_only: true,
            ..Default::default()
        };
        let merged = merge_managed_settings(base, overlay);
        // OR semantics: true if either is true
        assert!(merged.allow_managed_hooks_only);
        assert!(merged.allow_managed_permissions_only);
    }

    #[test]
    fn test_merge_permissions_rules_concatenated() {
        let base = ManagedSettings {
            permissions: Some(ManagedPermissions {
                default_mode: Some("ask".to_string()),
                rules: vec![ManagedPermissionRule {
                    tool: "Read".to_string(),
                    pattern: None,
                    mode: "allow".to_string(),
                }],
            }),
            ..Default::default()
        };
        let overlay = ManagedSettings {
            permissions: Some(ManagedPermissions {
                default_mode: Some("deny".to_string()),
                rules: vec![ManagedPermissionRule {
                    tool: "Bash".to_string(),
                    pattern: Some("git *".to_string()),
                    mode: "allow".to_string(),
                }],
            }),
            ..Default::default()
        };
        let merged = merge_managed_settings(base, overlay);
        let perms = merged.permissions.unwrap();
        // Overlay's default_mode wins
        assert_eq!(perms.default_mode.as_deref(), Some("deny"));
        // Rules are concatenated
        assert_eq!(perms.rules.len(), 2);
        assert_eq!(perms.rules[0].tool, "Read");
        assert_eq!(perms.rules[1].tool, "Bash");
    }

    #[test]
    fn test_merge_permissions_base_only() {
        let base = ManagedSettings {
            permissions: Some(ManagedPermissions {
                default_mode: Some("ask".to_string()),
                rules: vec![],
            }),
            ..Default::default()
        };
        let overlay = ManagedSettings::default();
        let merged = merge_managed_settings(base, overlay);
        assert!(merged.permissions.is_some());
        assert_eq!(
            merged.permissions.unwrap().default_mode.as_deref(),
            Some("ask")
        );
    }

    #[test]
    fn test_merge_permissions_overlay_only() {
        let base = ManagedSettings::default();
        let overlay = ManagedSettings {
            permissions: Some(ManagedPermissions {
                default_mode: Some("deny".to_string()),
                rules: vec![],
            }),
            ..Default::default()
        };
        let merged = merge_managed_settings(base, overlay);
        assert!(merged.permissions.is_some());
        assert_eq!(
            merged.permissions.unwrap().default_mode.as_deref(),
            Some("deny")
        );
    }

    // =========================================================================
    // Alphabetical merge from directory
    // =========================================================================

    #[test]
    fn test_load_from_directory_alphabetical() {
        let temp_dir = TempDir::new().unwrap();
        let dir = temp_dir.path().join("managed-settings.d");
        fs::create_dir_all(&dir).unwrap();

        // Create files in non-alphabetical order to verify sorting
        let settings_b = ManagedSettings {
            default_model: Some("model-b".to_string()),
            ..Default::default()
        };
        let settings_a = ManagedSettings {
            default_model: Some("model-a".to_string()),
            ..Default::default()
        };

        fs::write(
            dir.join("02-second.json"),
            serde_json::to_string(&settings_b).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.join("01-first.json"),
            serde_json::to_string(&settings_a).unwrap(),
        )
        .unwrap();

        let loaded = load_managed_settings_dir_from(&dir).unwrap();
        assert_eq!(loaded.len(), 2);
        // First loaded should be 01-first (model-a)
        assert_eq!(loaded[0].default_model.as_deref(), Some("model-a"));
        // Second loaded should be 02-second (model-b)
        assert_eq!(loaded[1].default_model.as_deref(), Some("model-b"));
    }

    #[test]
    fn test_load_from_directory_ignores_non_json() {
        let temp_dir = TempDir::new().unwrap();
        let dir = temp_dir.path().join("managed-settings.d");
        fs::create_dir_all(&dir).unwrap();

        let settings = ManagedSettings::default();
        fs::write(
            dir.join("valid.json"),
            serde_json::to_string(&settings).unwrap(),
        )
        .unwrap();
        fs::write(dir.join("readme.txt"), "not a json file").unwrap();
        fs::write(dir.join("backup.json.bak"), "not json either").unwrap();

        let loaded = load_managed_settings_dir_from(&dir).unwrap();
        assert_eq!(loaded.len(), 1);
    }

    #[test]
    fn test_load_from_nonexistent_directory() {
        let result = load_managed_settings_dir_from(Path::new("/nonexistent/managed-settings.d"));
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_merge_all_settings_sequential() {
        let base = ManagedSettings {
            default_model: Some("model-base".to_string()),
            allow_managed_hooks_only: false,
            ..Default::default()
        };
        let overlays = vec![
            ManagedSettings {
                default_model: Some("model-a".to_string()),
                allow_managed_hooks_only: true,
                ..Default::default()
            },
            ManagedSettings {
                default_model: Some("model-b".to_string()),
                ..Default::default()
            },
        ];
        let merged = merge_all_managed_settings(base, overlays);
        // Last overlay wins for model
        assert_eq!(merged.default_model.as_deref(), Some("model-b"));
        // OR semantics: hooks was set to true in first overlay
        assert!(merged.allow_managed_hooks_only);
    }

    // =========================================================================
    // Plugin allowlist enforcement
    // =========================================================================

    #[test]
    fn test_plugin_allowlist_permits_listed() {
        let settings = ManagedSettings {
            allowed_plugins: Some(vec!["safe-plugin".to_string(), "trusted".to_string()]),
            ..Default::default()
        };
        assert!(is_plugin_allowed("safe-plugin", &settings));
        assert!(is_plugin_allowed("trusted", &settings));
    }

    #[test]
    fn test_plugin_allowlist_denies_unlisted() {
        let settings = ManagedSettings {
            allowed_plugins: Some(vec!["safe-plugin".to_string()]),
            ..Default::default()
        };
        assert!(!is_plugin_allowed("unknown-plugin", &settings));
    }

    #[test]
    fn test_plugin_no_restrictions_allows_all() {
        let settings = ManagedSettings::default();
        assert!(is_plugin_allowed("any-plugin", &settings));
        assert!(is_plugin_allowed("another-plugin", &settings));
    }

    // =========================================================================
    // Plugin denylist enforcement
    // =========================================================================

    #[test]
    fn test_plugin_denylist_blocks_listed() {
        let settings = ManagedSettings {
            denied_plugins: Some(vec!["bad-plugin".to_string()]),
            ..Default::default()
        };
        assert!(!is_plugin_allowed("bad-plugin", &settings));
    }

    #[test]
    fn test_plugin_denylist_allows_unlisted() {
        let settings = ManagedSettings {
            denied_plugins: Some(vec!["bad-plugin".to_string()]),
            ..Default::default()
        };
        assert!(is_plugin_allowed("good-plugin", &settings));
    }

    #[test]
    fn test_plugin_denylist_takes_precedence_over_allowlist() {
        let settings = ManagedSettings {
            allowed_plugins: Some(vec!["plugin-x".to_string()]),
            denied_plugins: Some(vec!["plugin-x".to_string()]),
            ..Default::default()
        };
        // Denylist takes precedence
        assert!(!is_plugin_allowed("plugin-x", &settings));
    }

    // =========================================================================
    // MCP server allowlist/denylist
    // =========================================================================

    #[test]
    fn test_mcp_server_allowlist_permits_listed() {
        let settings = ManagedSettings {
            allowed_mcp_servers: Some(vec!["narsil".to_string(), "jetbrains".to_string()]),
            ..Default::default()
        };
        assert!(is_mcp_server_allowed("narsil", &settings));
        assert!(is_mcp_server_allowed("jetbrains", &settings));
    }

    #[test]
    fn test_mcp_server_allowlist_denies_unlisted() {
        let settings = ManagedSettings {
            allowed_mcp_servers: Some(vec!["narsil".to_string()]),
            ..Default::default()
        };
        assert!(!is_mcp_server_allowed("untrusted-server", &settings));
    }

    #[test]
    fn test_mcp_server_denylist_blocks_listed() {
        let settings = ManagedSettings {
            denied_mcp_servers: Some(vec!["evil-server".to_string()]),
            ..Default::default()
        };
        assert!(!is_mcp_server_allowed("evil-server", &settings));
    }

    #[test]
    fn test_mcp_server_denylist_allows_unlisted() {
        let settings = ManagedSettings {
            denied_mcp_servers: Some(vec!["evil-server".to_string()]),
            ..Default::default()
        };
        assert!(is_mcp_server_allowed("good-server", &settings));
    }

    #[test]
    fn test_mcp_server_no_restrictions_allows_all() {
        let settings = ManagedSettings::default();
        assert!(is_mcp_server_allowed("any-server", &settings));
    }

    #[test]
    fn test_mcp_server_denylist_takes_precedence_over_allowlist() {
        let settings = ManagedSettings {
            allowed_mcp_servers: Some(vec!["server-x".to_string()]),
            denied_mcp_servers: Some(vec!["server-x".to_string()]),
            ..Default::default()
        };
        assert!(!is_mcp_server_allowed("server-x", &settings));
    }

    // =========================================================================
    // Permission mode enforcement
    // =========================================================================

    #[test]
    fn test_enforced_permission_mode_returns_mode() {
        let settings = ManagedSettings {
            permissions: Some(ManagedPermissions {
                default_mode: Some("deny".to_string()),
                rules: vec![],
            }),
            ..Default::default()
        };
        assert_eq!(get_enforced_permission_mode(&settings), Some("deny"));
    }

    #[test]
    fn test_enforced_permission_mode_none_when_no_permissions() {
        let settings = ManagedSettings::default();
        assert_eq!(get_enforced_permission_mode(&settings), None);
    }

    #[test]
    fn test_enforced_permission_mode_none_when_no_default_mode() {
        let settings = ManagedSettings {
            permissions: Some(ManagedPermissions {
                default_mode: None,
                rules: vec![],
            }),
            ..Default::default()
        };
        assert_eq!(get_enforced_permission_mode(&settings), None);
    }

    // =========================================================================
    // allow_managed_hooks_only enforcement
    // =========================================================================

    #[test]
    fn test_allow_managed_hooks_only_default_false() {
        let settings = ManagedSettings::default();
        assert!(!settings.allow_managed_hooks_only);
    }

    #[test]
    fn test_allow_managed_hooks_only_set_true() {
        let settings = ManagedSettings {
            allow_managed_hooks_only: true,
            ..Default::default()
        };
        assert!(settings.allow_managed_hooks_only);
    }

    #[test]
    fn test_allow_managed_hooks_only_from_json() {
        let json = r#"{"allow_managed_hooks_only": true}"#;
        let settings: ManagedSettings = serde_json::from_str(json).unwrap();
        assert!(settings.allow_managed_hooks_only);
    }

    #[test]
    fn test_allow_managed_hooks_only_missing_defaults_false() {
        let json = r#"{}"#;
        let settings: ManagedSettings = serde_json::from_str(json).unwrap();
        assert!(!settings.allow_managed_hooks_only);
    }

    // =========================================================================
    // Edge cases
    // =========================================================================

    #[test]
    fn test_empty_allowlist_denies_all() {
        let settings = ManagedSettings {
            allowed_plugins: Some(vec![]),
            ..Default::default()
        };
        assert!(!is_plugin_allowed("any-plugin", &settings));
    }

    #[test]
    fn test_empty_denylist_allows_all() {
        let settings = ManagedSettings {
            denied_plugins: Some(vec![]),
            ..Default::default()
        };
        assert!(is_plugin_allowed("any-plugin", &settings));
    }

    #[test]
    fn test_merge_all_with_empty_overlays() {
        let base = ManagedSettings {
            default_model: Some("base-model".to_string()),
            ..Default::default()
        };
        let merged = merge_all_managed_settings(base, vec![]);
        assert_eq!(merged.default_model.as_deref(), Some("base-model"));
    }

    #[test]
    fn test_load_settings_with_all_fields_from_json() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("full-settings.json");

        let json = r#"{
            "permissions": {
                "default_mode": "ask",
                "rules": [
                    {"tool": "Bash", "pattern": "git *", "mode": "allow"},
                    {"tool": "Bash", "pattern": "rm *", "mode": "deny"}
                ]
            },
            "allowed_plugins": ["plugin-a", "plugin-b"],
            "denied_plugins": ["plugin-c"],
            "allowed_mcp_servers": ["server-a"],
            "denied_mcp_servers": ["server-b"],
            "default_model": "claude-3-opus",
            "allow_managed_hooks_only": true,
            "allow_managed_permissions_only": true
        }"#;

        fs::write(&path, json).unwrap();

        let loaded = load_managed_settings_from(&path).unwrap().unwrap();
        assert_eq!(loaded.default_model.as_deref(), Some("claude-3-opus"));
        assert!(loaded.allow_managed_hooks_only);
        assert!(loaded.allow_managed_permissions_only);
        assert_eq!(
            loaded.allowed_plugins,
            Some(vec!["plugin-a".to_string(), "plugin-b".to_string()])
        );
        assert_eq!(loaded.denied_plugins, Some(vec!["plugin-c".to_string()]));
        assert_eq!(
            loaded.allowed_mcp_servers,
            Some(vec!["server-a".to_string()])
        );
        assert_eq!(
            loaded.denied_mcp_servers,
            Some(vec!["server-b".to_string()])
        );

        let perms = loaded.permissions.unwrap();
        assert_eq!(perms.default_mode.as_deref(), Some("ask"));
        assert_eq!(perms.rules.len(), 2);
        assert_eq!(perms.rules[0].tool, "Bash");
        assert_eq!(perms.rules[0].pattern.as_deref(), Some("git *"));
        assert_eq!(perms.rules[0].mode, "allow");
        assert_eq!(perms.rules[1].mode, "deny");
    }

    #[test]
    fn test_directory_merge_produces_correct_final_state() {
        let temp_dir = TempDir::new().unwrap();
        let dir = temp_dir.path().join("managed-settings.d");
        fs::create_dir_all(&dir).unwrap();

        // Base file
        let base_path = temp_dir.path().join("managed-settings.json");
        let base = ManagedSettings {
            default_model: Some("base-model".to_string()),
            denied_plugins: Some(vec!["blocked".to_string()]),
            ..Default::default()
        };
        fs::write(&base_path, serde_json::to_string(&base).unwrap()).unwrap();

        // Overlay 01
        let overlay1 = ManagedSettings {
            allowed_mcp_servers: Some(vec!["narsil".to_string()]),
            allow_managed_hooks_only: true,
            ..Default::default()
        };
        fs::write(
            dir.join("01-security.json"),
            serde_json::to_string(&overlay1).unwrap(),
        )
        .unwrap();

        // Overlay 02
        let overlay2 = ManagedSettings {
            default_model: Some("enforced-model".to_string()),
            ..Default::default()
        };
        fs::write(
            dir.join("02-model.json"),
            serde_json::to_string(&overlay2).unwrap(),
        )
        .unwrap();

        // Load and merge
        let loaded_base = load_managed_settings_from(&base_path).unwrap().unwrap();
        let overlays = load_managed_settings_dir_from(&dir).unwrap();
        let final_settings = merge_all_managed_settings(loaded_base, overlays);

        assert_eq!(
            final_settings.default_model.as_deref(),
            Some("enforced-model")
        );
        assert_eq!(
            final_settings.denied_plugins,
            Some(vec!["blocked".to_string()])
        );
        assert_eq!(
            final_settings.allowed_mcp_servers,
            Some(vec!["narsil".to_string()])
        );
        assert!(final_settings.allow_managed_hooks_only);
    }
}
