//! Unit tests for plugin system.
//!
//! These tests verify plugin discovery, loading, and namespacing.
//! Following TDD approach for the plugin system.

use patina::plugins::PluginRegistry;
use std::fs;
use tempfile::TempDir;

/// Helper to create a plugin directory structure
fn create_plugin(dir: &TempDir, plugin_name: &str, manifest: &str) -> std::path::PathBuf {
    let plugin_dir = dir.path().join(plugin_name);
    let claude_plugin_dir = plugin_dir.join(".claude-plugin");
    fs::create_dir_all(&claude_plugin_dir).expect("Should create plugin dirs");
    fs::write(claude_plugin_dir.join("plugin.json"), manifest).expect("Should write manifest");
    plugin_dir
}

/// Helper to add a command to a plugin
fn add_plugin_command(plugin_dir: &std::path::Path, cmd_name: &str, content: &str) {
    let commands_dir = plugin_dir.join("commands");
    fs::create_dir_all(&commands_dir).expect("Should create commands dir");
    fs::write(commands_dir.join(format!("{}.md", cmd_name)), content)
        .expect("Should write command file");
}

/// Helper to add a skill to a plugin
fn add_plugin_skill(plugin_dir: &std::path::Path, skill_name: &str, skill_md: &str) {
    let skills_dir = plugin_dir.join("skills").join(skill_name);
    fs::create_dir_all(&skills_dir).expect("Should create skills dir");
    fs::write(skills_dir.join("SKILL.md"), skill_md).expect("Should write skill file");
}

// =============================================================================
// Test Group: Plugin Discovery
// =============================================================================

/// Tests discovering a single plugin from a directory.
#[test]
fn test_plugin_discovery_single() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let manifest = r#"{
        "name": "test-plugin",
        "version": "1.0.0",
        "description": "A test plugin"
    }"#;

    create_plugin(&temp_dir, "test-plugin", manifest);

    let mut registry = PluginRegistry::new();
    registry
        .load_all(&[temp_dir.path().to_path_buf()])
        .expect("Should load plugins");

    // Plugin should be discovered - we can verify by checking commands
    // Since there are no commands, skills will be empty
    let skills: Vec<_> = registry.all_skills().collect();
    assert!(
        skills.is_empty(),
        "Plugin with no skills should have empty skills"
    );
}

/// Tests discovering multiple plugins from a directory.
#[test]
fn test_plugin_discovery_multiple() {
    let temp_dir = TempDir::new().expect("Should create temp dir");

    let manifest1 = r#"{
        "name": "plugin-one",
        "version": "1.0.0"
    }"#;

    let manifest2 = r#"{
        "name": "plugin-two",
        "version": "2.0.0"
    }"#;

    let plugin1 = create_plugin(&temp_dir, "plugin-one", manifest1);
    let plugin2 = create_plugin(&temp_dir, "plugin-two", manifest2);

    // Add commands to verify both plugins loaded
    add_plugin_command(&plugin1, "cmd1", "Command 1 content");
    add_plugin_command(&plugin2, "cmd2", "Command 2 content");

    let mut registry = PluginRegistry::new();
    registry
        .load_all(&[temp_dir.path().to_path_buf()])
        .expect("Should load plugins");

    // Both commands should be accessible
    assert!(
        registry.get_command("plugin-one:cmd1").is_some(),
        "Should find plugin-one command"
    );
    assert!(
        registry.get_command("plugin-two:cmd2").is_some(),
        "Should find plugin-two command"
    );
}

/// Tests that directories without plugin manifest are ignored.
#[test]
fn test_plugin_discovery_ignores_non_plugins() {
    let temp_dir = TempDir::new().expect("Should create temp dir");

    // Create a valid plugin
    let manifest = r#"{
        "name": "valid-plugin",
        "version": "1.0.0"
    }"#;
    let plugin_dir = create_plugin(&temp_dir, "valid-plugin", manifest);
    add_plugin_command(&plugin_dir, "valid-cmd", "Valid command");

    // Create a non-plugin directory
    let non_plugin = temp_dir.path().join("not-a-plugin");
    fs::create_dir_all(&non_plugin).expect("Should create dir");
    fs::write(non_plugin.join("README.md"), "Not a plugin").expect("Should write file");

    let mut registry = PluginRegistry::new();
    registry
        .load_all(&[temp_dir.path().to_path_buf()])
        .expect("Should load plugins");

    // Only valid plugin command should be found
    assert!(
        registry.get_command("valid-plugin:valid-cmd").is_some(),
        "Should find valid plugin command"
    );
}

/// Tests loading plugins from non-existent directory.
#[test]
fn test_plugin_discovery_nonexistent_dir() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let nonexistent = temp_dir.path().join("does-not-exist");

    let mut registry = PluginRegistry::new();
    let result = registry.load_all(&[nonexistent]);

    assert!(result.is_ok(), "Should succeed for non-existent directory");
}

/// Tests loading plugins from multiple search paths.
#[test]
fn test_plugin_discovery_multiple_paths() {
    let temp_dir1 = TempDir::new().expect("Should create temp dir 1");
    let temp_dir2 = TempDir::new().expect("Should create temp dir 2");

    let manifest1 = r#"{
        "name": "plugin-path1",
        "version": "1.0.0"
    }"#;
    let manifest2 = r#"{
        "name": "plugin-path2",
        "version": "1.0.0"
    }"#;

    let plugin1 = create_plugin(&temp_dir1, "plugin-path1", manifest1);
    let plugin2 = create_plugin(&temp_dir2, "plugin-path2", manifest2);

    add_plugin_command(&plugin1, "from-path1", "Content 1");
    add_plugin_command(&plugin2, "from-path2", "Content 2");

    let mut registry = PluginRegistry::new();
    registry
        .load_all(&[
            temp_dir1.path().to_path_buf(),
            temp_dir2.path().to_path_buf(),
        ])
        .expect("Should load from multiple paths");

    assert!(
        registry.get_command("plugin-path1:from-path1").is_some(),
        "Should find command from first path"
    );
    assert!(
        registry.get_command("plugin-path2:from-path2").is_some(),
        "Should find command from second path"
    );
}

// =============================================================================
// Test Group: Plugin Version Compatibility
// =============================================================================

/// Tests plugin manifest with version information.
#[test]
fn test_plugin_version_compatibility() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let manifest = r#"{
        "name": "versioned-plugin",
        "version": "2.1.0",
        "min_rct_version": "0.1.0"
    }"#;

    let plugin_dir = create_plugin(&temp_dir, "versioned-plugin", manifest);
    add_plugin_command(&plugin_dir, "ver-cmd", "Versioned command");

    let mut registry = PluginRegistry::new();
    registry
        .load_all(&[temp_dir.path().to_path_buf()])
        .expect("Should load versioned plugin");

    // Plugin should be loaded
    assert!(
        registry.get_command("versioned-plugin:ver-cmd").is_some(),
        "Should find versioned plugin command"
    );
}

/// Tests plugin with optional manifest fields.
#[test]
fn test_plugin_optional_manifest_fields() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let manifest = r#"{
        "name": "full-plugin",
        "version": "1.0.0",
        "description": "A fully documented plugin",
        "author": "Test Author"
    }"#;

    let plugin_dir = create_plugin(&temp_dir, "full-plugin", manifest);
    add_plugin_command(&plugin_dir, "full-cmd", "Full command");

    let mut registry = PluginRegistry::new();
    registry
        .load_all(&[temp_dir.path().to_path_buf()])
        .expect("Should load plugin with optional fields");

    assert!(
        registry.get_command("full-plugin:full-cmd").is_some(),
        "Should find full plugin command"
    );
}

// =============================================================================
// Test Group: Command Namespacing
// =============================================================================

/// Tests command namespacing with plugin:command format.
#[test]
fn test_plugin_command_namespacing() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let manifest = r#"{
        "name": "ns-plugin",
        "version": "1.0.0"
    }"#;

    let plugin_dir = create_plugin(&temp_dir, "ns-plugin", manifest);
    add_plugin_command(&plugin_dir, "my-command", "My command content");
    add_plugin_command(&plugin_dir, "another-cmd", "Another command content");

    let mut registry = PluginRegistry::new();
    registry
        .load_all(&[temp_dir.path().to_path_buf()])
        .expect("Should load plugins");

    // Full namespaced access
    let cmd1 = registry.get_command("ns-plugin:my-command");
    assert!(cmd1.is_some(), "Should find command with full namespace");
    assert_eq!(cmd1.unwrap().content, "My command content");

    let cmd2 = registry.get_command("ns-plugin:another-cmd");
    assert!(cmd2.is_some(), "Should find another command with namespace");
}

/// Tests short command access when name is unique.
#[test]
fn test_plugin_command_short_access() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let manifest = r#"{
        "name": "short-plugin",
        "version": "1.0.0"
    }"#;

    let plugin_dir = create_plugin(&temp_dir, "short-plugin", manifest);
    add_plugin_command(&plugin_dir, "unique-cmd", "Unique content");

    let mut registry = PluginRegistry::new();
    registry
        .load_all(&[temp_dir.path().to_path_buf()])
        .expect("Should load plugins");

    // Short access without namespace
    let cmd = registry.get_command("unique-cmd");
    assert!(cmd.is_some(), "Should find command with short name");
    assert_eq!(cmd.unwrap().content, "Unique content");
}

/// Tests loading commands from plugin with frontmatter.
#[test]
fn test_plugin_command_content() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let manifest = r#"{
        "name": "content-plugin",
        "version": "1.0.0"
    }"#;

    let plugin_dir = create_plugin(&temp_dir, "content-plugin", manifest);
    let command_content = r#"# My Command

This is the command template.

Use {{ arg }} to substitute.
"#;
    add_plugin_command(&plugin_dir, "template-cmd", command_content);

    let mut registry = PluginRegistry::new();
    registry
        .load_all(&[temp_dir.path().to_path_buf()])
        .expect("Should load plugins");

    let cmd = registry.get_command("content-plugin:template-cmd");
    assert!(cmd.is_some());
    let cmd = cmd.unwrap();
    assert!(cmd.content.contains("# My Command"));
    assert!(cmd.content.contains("{{ arg }}"));
}

// =============================================================================
// Test Group: Plugin Skills
// =============================================================================

/// Tests loading skills from a plugin.
#[test]
fn test_plugin_skills_loading() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let manifest = r#"{
        "name": "skills-plugin",
        "version": "1.0.0"
    }"#;

    let plugin_dir = create_plugin(&temp_dir, "skills-plugin", manifest);
    let skill_md = r#"---
name: plugin-skill
description: A skill from a plugin
---

Use this skill for plugin-related tasks.
"#;
    add_plugin_skill(&plugin_dir, "plugin-skill", skill_md);

    let mut registry = PluginRegistry::new();
    registry
        .load_all(&[temp_dir.path().to_path_buf()])
        .expect("Should load plugins");

    let skills: Vec<_> = registry.all_skills().collect();
    assert_eq!(skills.len(), 1, "Should have one skill from plugin");
    assert_eq!(skills[0].name, "plugin-skill");
    assert_eq!(skills[0].description, "A skill from a plugin");
    assert!(skills[0].instructions.contains("plugin-related tasks"));
}

/// Tests loading multiple skills from a plugin.
#[test]
fn test_plugin_skills_multiple() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let manifest = r#"{
        "name": "multi-skills-plugin",
        "version": "1.0.0"
    }"#;

    let plugin_dir = create_plugin(&temp_dir, "multi-skills-plugin", manifest);

    let skill1 = r#"---
name: skill-one
description: First skill
---
First skill instructions.
"#;

    let skill2 = r#"---
name: skill-two
description: Second skill
---
Second skill instructions.
"#;

    add_plugin_skill(&plugin_dir, "skill-one", skill1);
    add_plugin_skill(&plugin_dir, "skill-two", skill2);

    let mut registry = PluginRegistry::new();
    registry
        .load_all(&[temp_dir.path().to_path_buf()])
        .expect("Should load plugins");

    let skills: Vec<_> = registry.all_skills().collect();
    assert_eq!(skills.len(), 2, "Should have two skills from plugin");

    let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"skill-one"));
    assert!(names.contains(&"skill-two"));
}

// =============================================================================
// Test Group: Edge Cases
// =============================================================================

/// Tests PluginRegistry::new() creates empty registry.
#[test]
fn test_plugin_registry_new() {
    let registry = PluginRegistry::new();
    assert!(
        registry.get_command("any").is_none(),
        "New registry has no commands"
    );
    assert!(
        registry.all_skills().count() == 0,
        "New registry has no skills"
    );
}

/// Tests PluginRegistry::default() creates empty registry.
#[test]
fn test_plugin_registry_default() {
    let registry = PluginRegistry::default();
    assert!(
        registry.get_command("any").is_none(),
        "Default registry has no commands"
    );
}

/// Tests malformed plugin manifest is handled gracefully.
#[test]
fn test_plugin_malformed_manifest() {
    // Use separate directories to ensure isolation
    let valid_dir = TempDir::new().expect("Should create valid temp dir");
    let invalid_dir = TempDir::new().expect("Should create invalid temp dir");

    // Valid plugin - note: manifest name must match what we search for
    let valid_manifest = r#"{"name": "valid-plugin", "version": "1.0.0"}"#;
    let valid_plugin = create_plugin(&valid_dir, "valid-plugin-dir", valid_manifest);
    add_plugin_command(&valid_plugin, "valid-cmd", "Valid");

    // Invalid JSON manifest in separate directory
    let invalid_plugin = invalid_dir.path().join("invalid-plugin");
    let claude_dir = invalid_plugin.join(".claude-plugin");
    fs::create_dir_all(&claude_dir).expect("Should create dirs");
    fs::write(claude_dir.join("plugin.json"), "{ invalid json }").expect("Should write file");

    let mut registry = PluginRegistry::new();
    // Load from both paths - invalid should be skipped
    registry
        .load_all(&[
            valid_dir.path().to_path_buf(),
            invalid_dir.path().to_path_buf(),
        ])
        .expect("Should continue despite invalid plugin");

    // Valid plugin should be loaded (using manifest name, not directory name)
    assert!(
        registry.get_command("valid-plugin:valid-cmd").is_some(),
        "Should load valid plugin"
    );
}

/// Tests getting non-existent command returns None.
#[test]
fn test_plugin_get_nonexistent_command() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let manifest = r#"{
        "name": "test-plugin",
        "version": "1.0.0"
    }"#;

    let plugin_dir = create_plugin(&temp_dir, "test-plugin", manifest);
    add_plugin_command(&plugin_dir, "existing", "Exists");

    let mut registry = PluginRegistry::new();
    registry
        .load_all(&[temp_dir.path().to_path_buf()])
        .expect("Should load plugins");

    assert!(
        registry.get_command("nonexistent").is_none(),
        "Should return None for nonexistent command"
    );
    assert!(
        registry.get_command("test-plugin:nonexistent").is_none(),
        "Should return None for nonexistent namespaced command"
    );
}

/// Tests that files without .md extension are properly skipped.
#[test]
fn test_plugin_handles_no_extension() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let manifest = r#"{
        "name": "ext-plugin",
        "version": "1.0.0"
    }"#;

    let plugin_dir = create_plugin(&temp_dir, "ext-plugin", manifest);
    let commands_dir = plugin_dir.join("commands");
    fs::create_dir_all(&commands_dir).expect("Should create commands dir");

    // Create files with various problematic names
    fs::write(commands_dir.join("no-extension"), "No extension content")
        .expect("Should write file without extension");
    fs::write(commands_dir.join("valid.md"), "Valid command").expect("Should write valid .md file");
    fs::write(commands_dir.join(".txt"), "Just extension no stem")
        .expect("Should write dot-extension file");

    let mut registry = PluginRegistry::new();
    registry
        .load_all(&[temp_dir.path().to_path_buf()])
        .expect("Should load plugins without panicking");

    // Only the valid .md file should be loaded
    assert!(
        registry.get_command("ext-plugin:valid").is_some(),
        "Should load valid.md command"
    );
    assert!(
        registry.get_command("ext-plugin:no-extension").is_none(),
        "Should skip file without extension"
    );
    assert!(
        registry.get_command("ext-plugin:").is_none(),
        "Should not create command with empty name"
    );
}

/// Tests that dotfiles with .md extension are handled correctly.
#[test]
fn test_plugin_handles_dotfile() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let manifest = r#"{
        "name": "dot-plugin",
        "version": "1.0.0"
    }"#;

    let plugin_dir = create_plugin(&temp_dir, "dot-plugin", manifest);
    let commands_dir = plugin_dir.join("commands");
    fs::create_dir_all(&commands_dir).expect("Should create commands dir");

    // Create a dotfile with .md extension
    fs::write(commands_dir.join(".hidden.md"), "Hidden command content")
        .expect("Should write dotfile");
    fs::write(commands_dir.join("normal.md"), "Normal command").expect("Should write normal file");

    let mut registry = PluginRegistry::new();
    registry
        .load_all(&[temp_dir.path().to_path_buf()])
        .expect("Should load plugins without panicking");

    // Normal file should be loaded
    assert!(
        registry.get_command("dot-plugin:normal").is_some(),
        "Should load normal.md"
    );

    // Dotfile behavior - either loaded with dot prefix name or gracefully skipped
    // We verify the loader doesn't panic, the exact behavior can be defined
    let dotfile_cmd = registry.get_command("dot-plugin:.hidden");
    // Dotfiles with .md extension should be loaded (file_stem returns ".hidden")
    assert!(
        dotfile_cmd.is_some(),
        "Should handle dotfile with .md extension"
    );
}

// =============================================================================
// Graceful Degradation Tests (4.2.1)
// =============================================================================

/// Tests that plugin loading continues when WalkDir encounters permission errors.
///
/// This tests graceful degradation: if a subdirectory cannot be traversed
/// during plugin discovery (e.g., permission denied), loading should log
/// a warning and continue discovering other plugins.
#[cfg(unix)]
#[test]
fn test_plugin_continues_on_walkdir_permission_error() {
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = TempDir::new().expect("Should create temp dir");

    // Create a valid plugin in a readable directory
    let valid_manifest = r#"{"name": "valid-plugin", "version": "1.0.0"}"#;
    let valid_plugin = create_plugin(&temp_dir, "valid-plugin", valid_manifest);
    add_plugin_command(&valid_plugin, "valid-cmd", "Valid command content");

    // Create an unreadable directory that will cause WalkDir to fail
    let unreadable_dir = temp_dir.path().join("unreadable-subdir");
    fs::create_dir_all(&unreadable_dir).expect("Should create unreadable dir");

    // Put a plugin in the unreadable dir before making it unreadable
    let unreachable_plugin = unreadable_dir.join("unreachable-plugin");
    let claude_dir = unreachable_plugin.join(".claude-plugin");
    fs::create_dir_all(&claude_dir).expect("Should create claude dir");
    fs::write(
        claude_dir.join("plugin.json"),
        r#"{"name": "unreachable", "version": "1.0.0"}"#,
    )
    .expect("Should write manifest");

    // Remove read permission from the unreadable directory
    let mut perms = fs::metadata(&unreadable_dir)
        .expect("Should get metadata")
        .permissions();
    perms.set_mode(0o000);
    fs::set_permissions(&unreadable_dir, perms).expect("Should set permissions");

    // Ensure we restore permissions on cleanup
    struct RestorePerms {
        path: std::path::PathBuf,
    }
    impl Drop for RestorePerms {
        fn drop(&mut self) {
            let mut perms = fs::metadata(&self.path)
                .map(|m| m.permissions())
                .unwrap_or_else(|_| std::fs::Permissions::from_mode(0o755));
            perms.set_mode(0o755);
            let _ = fs::set_permissions(&self.path, perms);
        }
    }
    let _guard = RestorePerms {
        path: unreadable_dir.clone(),
    };

    let mut registry = PluginRegistry::new();

    // Load should succeed despite the unreadable subdirectory
    let result = registry.load_all(&[temp_dir.path().to_path_buf()]);
    assert!(
        result.is_ok(),
        "Plugin loading should succeed even with unreadable subdirs: {:?}",
        result.err()
    );

    // Valid plugin should still be discovered and loaded
    assert!(
        registry.get_command("valid-plugin:valid-cmd").is_some(),
        "Valid plugin should be loaded despite walkdir errors"
    );
}

/// Tests that plugin loading continues when a command file cannot be read.
///
/// This tests graceful degradation: if an individual command file in a plugin
/// cannot be read, the plugin should still load with other commands.
#[test]
fn test_plugin_continues_on_unreadable_command_file() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let manifest = r#"{"name": "partial-plugin", "version": "1.0.0"}"#;

    let plugin_dir = create_plugin(&temp_dir, "partial-plugin", manifest);
    let commands_dir = plugin_dir.join("commands");
    fs::create_dir_all(&commands_dir).expect("Should create commands dir");

    // Create a valid command
    fs::write(commands_dir.join("valid.md"), "Valid command content")
        .expect("Should write valid command");

    // Create a command with invalid UTF-8 content
    fs::write(commands_dir.join("invalid.md"), [0xFF, 0xFE, 0x00, 0x01])
        .expect("Should write invalid command");

    let mut registry = PluginRegistry::new();

    // Load should succeed despite the invalid command file
    let result = registry.load_all(&[temp_dir.path().to_path_buf()]);
    assert!(
        result.is_ok(),
        "Plugin loading should succeed even with unreadable command files: {:?}",
        result.err()
    );

    // Valid command should still be loaded
    assert!(
        registry.get_command("partial-plugin:valid").is_some(),
        "Valid command should be loaded despite invalid sibling"
    );
}

// =============================================================================
// Test Group: TOML Plugin Discovery (9.2.1)
// =============================================================================

use patina::plugins::registry::discover_plugins;

/// Helper to create a TOML-based plugin directory structure
fn create_toml_plugin(dir: &TempDir, plugin_name: &str, manifest_toml: &str) -> std::path::PathBuf {
    let plugin_dir = dir.path().join(plugin_name);
    fs::create_dir_all(&plugin_dir).expect("Should create plugin dir");
    fs::write(plugin_dir.join("rct-plugin.toml"), manifest_toml).expect("Should write manifest");
    plugin_dir
}

/// Tests discovering plugins with TOML manifests from a directory.
#[test]
fn test_discover_plugins() {
    let temp_dir = TempDir::new().expect("Should create temp dir");

    let manifest1 = r#"
name = "narsil"
version = "1.0.0"
description = "Code intelligence plugin"

[capabilities]
mcp = true

[mcp]
command = "narsil-mcp"
args = ["--repo", "."]
auto_start = true
"#;

    let manifest2 = r#"
name = "git-helper"
version = "0.2.0"
description = "Git workflow automation"

[capabilities]
commands = true
skills = true
"#;

    create_toml_plugin(&temp_dir, "narsil", manifest1);
    create_toml_plugin(&temp_dir, "git-helper", manifest2);

    let discovered = discover_plugins(temp_dir.path()).expect("Should discover plugins");

    assert_eq!(discovered.len(), 2, "Should discover both plugins");

    // Check first plugin (narsil)
    let narsil = discovered.iter().find(|p| p.name == "narsil");
    assert!(narsil.is_some(), "Should find narsil plugin");
    let narsil = narsil.unwrap();
    assert_eq!(narsil.version, "1.0.0");
    assert!(narsil.has_capability(patina::plugins::manifest::Capability::Mcp));

    // Check second plugin (git-helper)
    let git_helper = discovered.iter().find(|p| p.name == "git-helper");
    assert!(git_helper.is_some(), "Should find git-helper plugin");
    let git_helper = git_helper.unwrap();
    assert_eq!(git_helper.version, "0.2.0");
    assert!(git_helper.has_capability(patina::plugins::manifest::Capability::Commands));
    assert!(git_helper.has_capability(patina::plugins::manifest::Capability::Skills));
}

/// Tests that discover_plugins returns empty vec for empty directory.
#[test]
fn test_discover_plugins_empty_dir() {
    let temp_dir = TempDir::new().expect("Should create temp dir");

    let discovered = discover_plugins(temp_dir.path()).expect("Should handle empty dir");
    assert!(
        discovered.is_empty(),
        "Should return empty vec for empty dir"
    );
}

/// Tests that discover_plugins handles non-existent directory gracefully.
#[test]
fn test_discover_plugins_nonexistent_dir() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let nonexistent = temp_dir.path().join("nonexistent");

    let discovered = discover_plugins(&nonexistent).expect("Should handle nonexistent dir");
    assert!(
        discovered.is_empty(),
        "Should return empty vec for nonexistent dir"
    );
}

/// Tests that invalid TOML manifests are skipped during discovery.
#[test]
fn test_discover_plugins_skips_invalid() {
    let temp_dir = TempDir::new().expect("Should create temp dir");

    // Valid plugin
    let valid_manifest = r#"
name = "valid-plugin"
version = "1.0.0"
"#;
    create_toml_plugin(&temp_dir, "valid-plugin", valid_manifest);

    // Invalid plugin (missing required fields)
    let invalid_dir = temp_dir.path().join("invalid-plugin");
    fs::create_dir_all(&invalid_dir).expect("Should create invalid plugin dir");
    fs::write(invalid_dir.join("rct-plugin.toml"), "invalid = true")
        .expect("Should write invalid manifest");

    let discovered = discover_plugins(temp_dir.path()).expect("Should handle invalid plugins");

    assert_eq!(discovered.len(), 1, "Should only discover valid plugin");
    assert_eq!(discovered[0].name, "valid-plugin");
}

/// Tests that nested plugin directories are discovered.
#[test]
fn test_discover_plugins_nested() {
    let temp_dir = TempDir::new().expect("Should create temp dir");

    // Create nested structure like ~/.config/patina/plugins/category/plugin-name/
    let category_dir = temp_dir.path().join("ai-tools");
    fs::create_dir_all(&category_dir).expect("Should create category dir");

    let manifest = r#"
name = "nested-plugin"
version = "1.0.0"
"#;

    let plugin_dir = category_dir.join("nested-plugin");
    fs::create_dir_all(&plugin_dir).expect("Should create nested plugin dir");
    fs::write(plugin_dir.join("rct-plugin.toml"), manifest).expect("Should write manifest");

    let discovered = discover_plugins(temp_dir.path()).expect("Should discover nested plugins");

    assert_eq!(discovered.len(), 1, "Should discover nested plugin");
    assert_eq!(discovered[0].name, "nested-plugin");
}
