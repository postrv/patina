//! Plugin discovery and registry for TOML-based plugins.
//!
//! This module provides filesystem-based plugin discovery, scanning directories
//! for `rct-plugin.toml` manifests and loading their metadata.
//!
//! # Example
//!
//! ```no_run
//! use patina::plugins::registry::discover_plugins;
//! use std::path::Path;
//!
//! let plugins_dir = Path::new("~/.config/patina/plugins");
//! let plugins = discover_plugins(plugins_dir)?;
//!
//! for plugin in &plugins {
//!     println!("Found plugin: {} v{}", plugin.name, plugin.version);
//! }
//! # Ok::<(), anyhow::Error>(())
//! ```

use crate::plugins::manifest::{Manifest, ManifestError};
use std::path::Path;
use walkdir::WalkDir;

/// The standard manifest filename for Patina plugins.
pub const MANIFEST_FILENAME: &str = "rct-plugin.toml";

/// Maximum directory depth to search for plugins.
const MAX_DISCOVERY_DEPTH: usize = 3;

/// Discovers plugins in a directory by scanning for `rct-plugin.toml` manifests.
///
/// Scans the given directory and its subdirectories (up to 3 levels deep) for
/// plugin manifests. Invalid manifests are logged and skipped. Symlinks are
/// rejected for security (prevents path traversal attacks).
///
/// # Arguments
///
/// * `dir` - The directory to scan for plugins
///
/// # Returns
///
/// A vector of successfully parsed plugin manifests.
///
/// # Errors
///
/// Returns an error only for fatal filesystem issues. Individual plugin
/// parsing errors are logged and skipped (graceful degradation).
///
/// # Security
///
/// - Symlinks are not followed to prevent path traversal attacks
/// - Only files named `rct-plugin.toml` are parsed
/// - Directory traversal is limited to 3 levels deep
///
/// # Example
///
/// ```no_run
/// use patina::plugins::registry::discover_plugins;
/// use std::path::Path;
///
/// let plugins = discover_plugins(Path::new("/path/to/plugins"))?;
/// println!("Discovered {} plugins", plugins.len());
/// # Ok::<(), anyhow::Error>(())
/// ```
pub fn discover_plugins(dir: &Path) -> Result<Vec<Manifest>, ManifestError> {
    let mut manifests = Vec::new();

    if !dir.exists() {
        return Ok(manifests);
    }

    for entry in WalkDir::new(dir)
        .max_depth(MAX_DISCOVERY_DEPTH)
        .follow_links(false) // Security: don't follow symlinks
        .into_iter()
        .filter_map(|e| e.ok())
    {
        // Security: skip symlinks entirely to prevent path traversal
        if entry.path_is_symlink() {
            tracing::debug!(
                "Skipping symlink during plugin discovery: {:?}",
                entry.path()
            );
            continue;
        }

        if entry.file_type().is_file() && entry.file_name() == MANIFEST_FILENAME {
            match Manifest::from_file(entry.path()) {
                Ok(manifest) => {
                    tracing::debug!(
                        "Discovered plugin: {} v{} at {:?}",
                        manifest.name,
                        manifest.version,
                        entry.path()
                    );
                    manifests.push(manifest);
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to parse plugin manifest at {:?}: {}",
                        entry.path(),
                        e
                    );
                }
            }
        }
    }

    Ok(manifests)
}

/// Returns the default plugin directory path.
///
/// On Unix-like systems: `~/.local/share/rct/plugins` (via ProjectDirs)
/// On Windows: `%APPDATA%\rct\plugins`
///
/// Returns `None` if the directory cannot be determined.
#[must_use]
pub fn default_plugins_dir() -> Option<std::path::PathBuf> {
    crate::util::get_plugins_dir()
}

/// Manages plugin lifecycle: loading, unloading, and tracking enabled plugins.
///
/// The `PluginManager` wraps plugin discovery and provides methods to load
/// and unload plugins at runtime. Plugins must be discovered before they
/// can be loaded.
///
/// # Example
///
/// ```no_run
/// use patina::plugins::registry::PluginManager;
/// use std::path::Path;
///
/// let mut manager = PluginManager::new(Path::new("~/.config/patina/plugins"))?;
///
/// // Load a discovered plugin
/// manager.load("my-plugin")?;
/// assert!(manager.is_loaded("my-plugin"));
///
/// // List enabled plugins
/// for name in manager.list_enabled() {
///     println!("Loaded: {}", name);
/// }
///
/// // Unload when done
/// manager.unload("my-plugin");
/// # Ok::<(), anyhow::Error>(())
/// ```
#[derive(Debug)]
pub struct PluginManager {
    /// Directory containing plugins.
    plugins_dir: std::path::PathBuf,
    /// All discovered plugin manifests, keyed by name.
    discovered: std::collections::HashMap<String, Manifest>,
    /// Names of currently loaded/enabled plugins.
    enabled: std::collections::HashSet<String>,
}

/// Errors that can occur during plugin lifecycle operations.
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    /// Plugin was not found in the registry.
    #[error("plugin not found: {0}")]
    NotFound(String),

    /// Plugin is already loaded.
    #[error("plugin already loaded: {0}")]
    AlreadyLoaded(String),

    /// Plugin discovery failed.
    #[error("discovery error: {0}")]
    DiscoveryError(#[from] ManifestError),
}

impl PluginManager {
    /// Creates a new plugin manager and discovers plugins in the given directory.
    ///
    /// # Arguments
    ///
    /// * `plugins_dir` - Directory to scan for plugins
    ///
    /// # Errors
    ///
    /// Returns an error if plugin discovery fails.
    pub fn new(plugins_dir: &Path) -> Result<Self, PluginError> {
        let manifests = discover_plugins(plugins_dir)?;
        let discovered = manifests.into_iter().map(|m| (m.name.clone(), m)).collect();

        Ok(Self {
            plugins_dir: plugins_dir.to_path_buf(),
            discovered,
            enabled: std::collections::HashSet::new(),
        })
    }

    /// Loads a plugin by name, making it active.
    ///
    /// The plugin must have been discovered during initialization.
    ///
    /// # Arguments
    ///
    /// * `name` - Name of the plugin to load
    ///
    /// # Errors
    ///
    /// Returns `PluginError::NotFound` if the plugin doesn't exist.
    /// Returns `PluginError::AlreadyLoaded` if the plugin is already loaded.
    pub fn load(&mut self, name: &str) -> Result<(), PluginError> {
        if !self.discovered.contains_key(name) {
            return Err(PluginError::NotFound(name.to_string()));
        }

        if self.enabled.contains(name) {
            return Err(PluginError::AlreadyLoaded(name.to_string()));
        }

        self.enabled.insert(name.to_string());
        tracing::info!("Loaded plugin: {}", name);
        Ok(())
    }

    /// Unloads a plugin by name.
    ///
    /// Returns `true` if the plugin was loaded and is now unloaded,
    /// `false` if it wasn't loaded.
    pub fn unload(&mut self, name: &str) -> bool {
        let removed = self.enabled.remove(name);
        if removed {
            tracing::info!("Unloaded plugin: {}", name);
        }
        removed
    }

    /// Returns the names of all currently loaded plugins.
    #[must_use]
    pub fn list_enabled(&self) -> Vec<&str> {
        self.enabled.iter().map(String::as_str).collect()
    }

    /// Returns the names of all discovered plugins.
    #[must_use]
    pub fn list_discovered(&self) -> Vec<&str> {
        self.discovered.keys().map(String::as_str).collect()
    }

    /// Checks if a plugin is currently loaded.
    #[must_use]
    pub fn is_loaded(&self, name: &str) -> bool {
        self.enabled.contains(name)
    }

    /// Returns the manifest for a discovered plugin.
    #[must_use]
    pub fn get_manifest(&self, name: &str) -> Option<&Manifest> {
        self.discovered.get(name)
    }

    /// Re-scans the plugins directory for new plugins.
    ///
    /// Newly discovered plugins are added but not automatically loaded.
    /// Existing loaded plugins remain loaded.
    ///
    /// # Errors
    ///
    /// Returns an error if plugin discovery fails.
    pub fn refresh(&mut self) -> Result<(), PluginError> {
        let manifests = discover_plugins(&self.plugins_dir)?;
        for manifest in manifests {
            self.discovered
                .entry(manifest.name.clone())
                .or_insert(manifest);
        }
        Ok(())
    }

    /// Returns the number of discovered plugins.
    #[must_use]
    pub fn discovered_count(&self) -> usize {
        self.discovered.len()
    }

    /// Returns the number of loaded plugins.
    #[must_use]
    pub fn loaded_count(&self) -> usize {
        self.enabled.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_plugin_load_unload() {
        let temp_dir = TempDir::new().unwrap();

        // Create a plugin
        let plugin_dir = temp_dir.path().join("test-plugin");
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(
            plugin_dir.join(MANIFEST_FILENAME),
            r#"name = "test-plugin"
version = "1.0.0"
description = "A test plugin""#,
        )
        .unwrap();

        // Create manager and verify discovery
        let mut manager = PluginManager::new(temp_dir.path()).unwrap();
        assert_eq!(manager.discovered_count(), 1);
        assert_eq!(manager.loaded_count(), 0);

        // Load the plugin
        manager.load("test-plugin").unwrap();
        assert!(manager.is_loaded("test-plugin"));
        assert_eq!(manager.loaded_count(), 1);

        // Verify list_enabled
        let enabled = manager.list_enabled();
        assert_eq!(enabled.len(), 1);
        assert!(enabled.contains(&"test-plugin"));

        // Unload the plugin
        assert!(manager.unload("test-plugin"));
        assert!(!manager.is_loaded("test-plugin"));
        assert_eq!(manager.loaded_count(), 0);
        assert!(manager.list_enabled().is_empty());

        // Unload again returns false
        assert!(!manager.unload("test-plugin"));
    }

    #[test]
    fn test_plugin_load_not_found() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = PluginManager::new(temp_dir.path()).unwrap();

        let result = manager.load("nonexistent");
        assert!(matches!(result, Err(PluginError::NotFound(_))));
    }

    #[test]
    fn test_plugin_load_already_loaded() {
        let temp_dir = TempDir::new().unwrap();

        let plugin_dir = temp_dir.path().join("my-plugin");
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(
            plugin_dir.join(MANIFEST_FILENAME),
            r#"name = "my-plugin"
version = "1.0.0""#,
        )
        .unwrap();

        let mut manager = PluginManager::new(temp_dir.path()).unwrap();
        manager.load("my-plugin").unwrap();

        let result = manager.load("my-plugin");
        assert!(matches!(result, Err(PluginError::AlreadyLoaded(_))));
    }

    #[test]
    fn test_plugin_manager_refresh() {
        let temp_dir = TempDir::new().unwrap();

        // Start with one plugin
        let plugin1_dir = temp_dir.path().join("plugin-one");
        fs::create_dir_all(&plugin1_dir).unwrap();
        fs::write(
            plugin1_dir.join(MANIFEST_FILENAME),
            r#"name = "plugin-one"
version = "1.0.0""#,
        )
        .unwrap();

        let mut manager = PluginManager::new(temp_dir.path()).unwrap();
        assert_eq!(manager.discovered_count(), 1);

        // Add another plugin after initialization
        let plugin2_dir = temp_dir.path().join("plugin-two");
        fs::create_dir_all(&plugin2_dir).unwrap();
        fs::write(
            plugin2_dir.join(MANIFEST_FILENAME),
            r#"name = "plugin-two"
version = "1.0.0""#,
        )
        .unwrap();

        // Refresh should find the new plugin
        manager.refresh().unwrap();
        assert_eq!(manager.discovered_count(), 2);
        assert!(manager.list_discovered().contains(&"plugin-one"));
        assert!(manager.list_discovered().contains(&"plugin-two"));
    }

    #[test]
    fn test_plugin_manager_get_manifest() {
        let temp_dir = TempDir::new().unwrap();

        let plugin_dir = temp_dir.path().join("test-plugin");
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(
            plugin_dir.join(MANIFEST_FILENAME),
            r#"name = "test-plugin"
version = "2.0.0"
description = "Test description""#,
        )
        .unwrap();

        let manager = PluginManager::new(temp_dir.path()).unwrap();

        let manifest = manager.get_manifest("test-plugin").unwrap();
        assert_eq!(manifest.name, "test-plugin");
        assert_eq!(manifest.version, "2.0.0");
        assert_eq!(manifest.description.as_deref(), Some("Test description"));

        assert!(manager.get_manifest("nonexistent").is_none());
    }

    #[test]
    fn test_discover_plugins_finds_manifest() {
        let temp_dir = TempDir::new().unwrap();
        let plugin_dir = temp_dir.path().join("my-plugin");
        fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = r#"
name = "my-plugin"
version = "1.0.0"
description = "Test plugin"
"#;
        fs::write(plugin_dir.join(MANIFEST_FILENAME), manifest).unwrap();

        let discovered = discover_plugins(temp_dir.path()).unwrap();

        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].name, "my-plugin");
        assert_eq!(discovered[0].version, "1.0.0");
    }

    #[test]
    fn test_discover_plugins_empty_directory() {
        let temp_dir = TempDir::new().unwrap();
        let discovered = discover_plugins(temp_dir.path()).unwrap();
        assert!(discovered.is_empty());
    }

    #[test]
    fn test_discover_plugins_nonexistent_directory() {
        let temp_dir = TempDir::new().unwrap();
        let nonexistent = temp_dir.path().join("does-not-exist");
        let discovered = discover_plugins(&nonexistent).unwrap();
        assert!(discovered.is_empty());
    }

    #[test]
    fn test_discover_plugins_skips_invalid_manifests() {
        let temp_dir = TempDir::new().unwrap();

        // Valid plugin
        let valid_dir = temp_dir.path().join("valid");
        fs::create_dir_all(&valid_dir).unwrap();
        fs::write(
            valid_dir.join(MANIFEST_FILENAME),
            r#"name = "valid"
version = "1.0.0""#,
        )
        .unwrap();

        // Invalid plugin (missing version)
        let invalid_dir = temp_dir.path().join("invalid");
        fs::create_dir_all(&invalid_dir).unwrap();
        fs::write(invalid_dir.join(MANIFEST_FILENAME), r#"name = "invalid""#).unwrap();

        let discovered = discover_plugins(temp_dir.path()).unwrap();

        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].name, "valid");
    }

    #[test]
    fn test_manifest_filename_constant() {
        assert_eq!(MANIFEST_FILENAME, "rct-plugin.toml");
    }

    #[test]
    fn test_default_plugins_dir() {
        let dir = default_plugins_dir();
        assert!(dir.is_some(), "Should return plugins directory");
        let path = dir.unwrap();
        assert!(
            path.ends_with("plugins"),
            "Path should end with 'plugins': {:?}",
            path
        );
    }

    /// Tests that symlinked plugin directories are rejected for security.
    #[cfg(unix)]
    #[test]
    fn test_discover_plugins_rejects_symlinks() {
        use std::os::unix::fs::symlink;

        let temp_dir = TempDir::new().unwrap();

        // Create a real plugin outside the plugins directory
        let external_dir = TempDir::new().unwrap();
        let external_plugin = external_dir.path().join("external-plugin");
        fs::create_dir_all(&external_plugin).unwrap();
        fs::write(
            external_plugin.join(MANIFEST_FILENAME),
            r#"name = "external"
version = "1.0.0""#,
        )
        .unwrap();

        // Create a symlink to the external plugin inside our plugins directory
        let symlink_path = temp_dir.path().join("linked-plugin");
        symlink(&external_plugin, &symlink_path).unwrap();

        // Also create a valid non-symlinked plugin
        let valid_dir = temp_dir.path().join("valid-plugin");
        fs::create_dir_all(&valid_dir).unwrap();
        fs::write(
            valid_dir.join(MANIFEST_FILENAME),
            r#"name = "valid"
version = "1.0.0""#,
        )
        .unwrap();

        let discovered = discover_plugins(temp_dir.path()).unwrap();

        // Should only find the valid plugin, not the symlinked one
        assert_eq!(
            discovered.len(),
            1,
            "Should only discover non-symlinked plugin"
        );
        assert_eq!(discovered[0].name, "valid");
    }

    /// Tests that symlinked manifest files are rejected.
    #[cfg(unix)]
    #[test]
    fn test_discover_plugins_rejects_symlinked_manifest() {
        use std::os::unix::fs::symlink;

        let temp_dir = TempDir::new().unwrap();

        // Create a real manifest outside the plugins directory
        let external_dir = TempDir::new().unwrap();
        let external_manifest = external_dir.path().join(MANIFEST_FILENAME);
        fs::write(
            &external_manifest,
            r#"name = "external"
version = "1.0.0""#,
        )
        .unwrap();

        // Create a plugin dir with a symlinked manifest
        let plugin_dir = temp_dir.path().join("linked-manifest-plugin");
        fs::create_dir_all(&plugin_dir).unwrap();
        symlink(&external_manifest, plugin_dir.join(MANIFEST_FILENAME)).unwrap();

        let discovered = discover_plugins(temp_dir.path()).unwrap();

        // Should not discover the plugin with symlinked manifest
        assert!(
            discovered.is_empty(),
            "Should reject symlinked manifest files"
        );
    }
}
