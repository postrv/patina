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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

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
