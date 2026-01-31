//! Tests for plugin manifest parsing and validation.
//!
//! These tests define the expected behavior of `rct-plugin.toml` manifest files.

use patina::plugins::manifest::{Capability, Manifest, ManifestError};

const MINIMAL_MANIFEST: &str = r#"
name = "test-plugin"
version = "1.0.0"
"#;

const FULL_MANIFEST: &str = r#"
name = "narsil"
version = "2.1.0"
description = "Code intelligence and security analysis"
author = "Anthropic"
license = "MIT"
homepage = "https://github.com/anthropics/narsil"
repository = "https://github.com/anthropics/narsil"
min_patina_version = "0.3.0"

[capabilities]
commands = true
skills = true
tools = true
hooks = false
mcp = true

[mcp]
command = "narsil-mcp"
args = ["--project", "."]
auto_start = true

[config]
index_on_load = { type = "bool", default = true, description = "Index project on plugin load" }
scan_depth = { type = "int", default = 3, description = "Maximum directory depth for scanning" }
"#;

const MANIFEST_WITH_DEPENDENCIES: &str = r#"
name = "advanced-plugin"
version = "1.0.0"

[dependencies]
narsil = ">=2.0.0"
"#;

#[test]
fn test_parse_minimal_manifest() {
    let manifest = Manifest::from_toml(MINIMAL_MANIFEST).unwrap();

    assert_eq!(manifest.name, "test-plugin");
    assert_eq!(manifest.version, "1.0.0");
    assert!(manifest.description.is_none());
    assert!(manifest.author.is_none());
}

#[test]
fn test_parse_full_manifest() {
    let manifest = Manifest::from_toml(FULL_MANIFEST).unwrap();

    assert_eq!(manifest.name, "narsil");
    assert_eq!(manifest.version, "2.1.0");
    assert_eq!(
        manifest.description.as_deref(),
        Some("Code intelligence and security analysis")
    );
    assert_eq!(manifest.author.as_deref(), Some("Anthropic"));
    assert_eq!(manifest.license.as_deref(), Some("MIT"));
    assert_eq!(manifest.min_patina_version.as_deref(), Some("0.3.0"));
}

#[test]
fn test_parse_capabilities() {
    let manifest = Manifest::from_toml(FULL_MANIFEST).unwrap();

    assert!(manifest.has_capability(Capability::Commands));
    assert!(manifest.has_capability(Capability::Skills));
    assert!(manifest.has_capability(Capability::Tools));
    assert!(!manifest.has_capability(Capability::Hooks));
    assert!(manifest.has_capability(Capability::Mcp));
}

#[test]
fn test_default_capabilities() {
    // Minimal manifest should have no capabilities enabled by default
    let manifest = Manifest::from_toml(MINIMAL_MANIFEST).unwrap();

    assert!(!manifest.has_capability(Capability::Commands));
    assert!(!manifest.has_capability(Capability::Skills));
    assert!(!manifest.has_capability(Capability::Tools));
    assert!(!manifest.has_capability(Capability::Hooks));
    assert!(!manifest.has_capability(Capability::Mcp));
}

#[test]
fn test_parse_mcp_config() {
    let manifest = Manifest::from_toml(FULL_MANIFEST).unwrap();

    let mcp = manifest.mcp_config().expect("MCP config should be present");
    assert_eq!(mcp.command, "narsil-mcp");
    assert_eq!(mcp.args, vec!["--project", "."]);
    assert!(mcp.auto_start);
}

#[test]
fn test_parse_plugin_config() {
    let manifest = Manifest::from_toml(FULL_MANIFEST).unwrap();

    let config = manifest.config();
    assert!(config.contains_key("index_on_load"));
    assert!(config.contains_key("scan_depth"));

    let index_config = &config["index_on_load"];
    assert_eq!(index_config.config_type, "bool");
    assert_eq!(index_config.default.as_ref().unwrap(), "true");
}

#[test]
fn test_parse_dependencies() {
    let manifest = Manifest::from_toml(MANIFEST_WITH_DEPENDENCIES).unwrap();

    let deps = manifest.dependencies();
    assert_eq!(deps.get("narsil"), Some(&">=2.0.0".to_string()));
}

#[test]
fn test_validate_missing_name() {
    let invalid = r#"
version = "1.0.0"
"#;

    let result = Manifest::from_toml(invalid);
    assert!(matches!(result, Err(ManifestError::MissingField(field)) if field == "name"));
}

#[test]
fn test_validate_missing_version() {
    let invalid = r#"
name = "test"
"#;

    let result = Manifest::from_toml(invalid);
    assert!(matches!(result, Err(ManifestError::MissingField(field)) if field == "version"));
}

#[test]
fn test_validate_invalid_version_format() {
    let invalid = r#"
name = "test"
version = "not-semver"
"#;

    let result = Manifest::from_toml(invalid);
    assert!(matches!(result, Err(ManifestError::InvalidVersion(_))));
}

#[test]
fn test_validate_invalid_name() {
    let invalid = r#"
name = "invalid name with spaces"
version = "1.0.0"
"#;

    let result = Manifest::from_toml(invalid);
    assert!(matches!(result, Err(ManifestError::InvalidName(_))));
}

#[test]
fn test_validate_plugin_capabilities_mcp_requires_config() {
    let invalid = r#"
name = "test"
version = "1.0.0"

[capabilities]
mcp = true
"#;

    let result = Manifest::from_toml(invalid);
    assert!(matches!(result, Err(ManifestError::InvalidCapability(msg)) if msg.contains("mcp")));
}

#[test]
fn test_from_file() {
    use std::io::Write;
    use tempfile::NamedTempFile;

    let mut file = NamedTempFile::new().unwrap();
    writeln!(file, "{}", MINIMAL_MANIFEST).unwrap();

    let manifest = Manifest::from_file(file.path()).unwrap();
    assert_eq!(manifest.name, "test-plugin");
}

#[test]
fn test_display_manifest() {
    let manifest = Manifest::from_toml(MINIMAL_MANIFEST).unwrap();
    let display = format!("{}", manifest);

    assert!(display.contains("test-plugin"));
    assert!(display.contains("1.0.0"));
}

#[test]
fn test_manifest_equality() {
    let m1 = Manifest::from_toml(MINIMAL_MANIFEST).unwrap();
    let m2 = Manifest::from_toml(MINIMAL_MANIFEST).unwrap();

    assert_eq!(m1, m2);
}
