//! Integration tests for plugin host API.

mod common;

use common::TestContext;
use rct::plugins::PluginRegistry;
use std::fs;

// ============================================================================
// 6.5.1.1 Plugin Lifecycle Tests
// ============================================================================

#[test]
fn test_plugin_load_from_valid_directory() {
    let ctx = TestContext::new();

    // Create a valid plugin structure
    create_test_plugin(&ctx, "test-plugin", "1.0.0");

    let mut registry = PluginRegistry::new();
    registry.load_all(&[ctx.path().to_path_buf()]).unwrap();

    // Plugin should be loaded
    assert!(registry.has_plugin("test-plugin"));
}

#[test]
fn test_plugin_load_multiple_plugins() {
    let ctx = TestContext::new();

    create_test_plugin(&ctx, "plugin-one", "1.0.0");
    create_test_plugin(&ctx, "plugin-two", "2.0.0");

    let mut registry = PluginRegistry::new();
    registry.load_all(&[ctx.path().to_path_buf()]).unwrap();

    assert!(registry.has_plugin("plugin-one"));
    assert!(registry.has_plugin("plugin-two"));
    assert_eq!(registry.plugin_count(), 2);
}

#[test]
fn test_plugin_load_with_commands() {
    let ctx = TestContext::new();

    let plugin_dir = create_test_plugin(&ctx, "cmd-plugin", "1.0.0");

    // Add a command
    let cmd_dir = plugin_dir.join("commands");
    fs::create_dir_all(&cmd_dir).unwrap();
    fs::write(cmd_dir.join("greet.md"), "# Greet Command\n\nSay hello!").unwrap();

    let mut registry = PluginRegistry::new();
    registry.load_all(&[ctx.path().to_path_buf()]).unwrap();

    let cmd = registry.get_command("cmd-plugin:greet");
    assert!(cmd.is_some(), "Command should be loadable with full name");

    let cmd = registry.get_command("greet");
    assert!(cmd.is_some(), "Command should be loadable with short name");
}

#[test]
fn test_plugin_load_with_skills() {
    let ctx = TestContext::new();

    let plugin_dir = create_test_plugin(&ctx, "skill-plugin", "1.0.0");

    // Add a skill
    let skill_dir = plugin_dir.join("skills/my-skill");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        r#"---
name: my-skill
description: A test skill
---
# Instructions
Do something useful.
"#,
    )
    .unwrap();

    let mut registry = PluginRegistry::new();
    registry.load_all(&[ctx.path().to_path_buf()]).unwrap();

    let skills: Vec<_> = registry.all_skills().collect();
    assert!(!skills.is_empty(), "Should have loaded the skill");
    assert_eq!(skills[0].name, "my-skill");
}

#[test]
fn test_plugin_unload() {
    let ctx = TestContext::new();

    create_test_plugin(&ctx, "unload-test", "1.0.0");

    let mut registry = PluginRegistry::new();
    registry.load_all(&[ctx.path().to_path_buf()]).unwrap();

    assert!(registry.has_plugin("unload-test"));

    // Unload the plugin
    let unloaded = registry.unload_plugin("unload-test");
    assert!(unloaded, "Should return true when unloading existing plugin");
    assert!(!registry.has_plugin("unload-test"));
}

#[test]
fn test_plugin_unload_nonexistent() {
    let mut registry = PluginRegistry::new();

    let unloaded = registry.unload_plugin("nonexistent");
    assert!(
        !unloaded,
        "Should return false when unloading nonexistent plugin"
    );
}

#[test]
fn test_plugin_unload_removes_commands() {
    let ctx = TestContext::new();

    let plugin_dir = create_test_plugin(&ctx, "cmd-unload", "1.0.0");

    let cmd_dir = plugin_dir.join("commands");
    fs::create_dir_all(&cmd_dir).unwrap();
    fs::write(cmd_dir.join("test-cmd.md"), "# Test").unwrap();

    let mut registry = PluginRegistry::new();
    registry.load_all(&[ctx.path().to_path_buf()]).unwrap();

    assert!(registry.get_command("test-cmd").is_some());

    registry.unload_plugin("cmd-unload");

    assert!(
        registry.get_command("test-cmd").is_none(),
        "Commands should be removed when plugin is unloaded"
    );
}

#[test]
fn test_plugin_reload() {
    let ctx = TestContext::new();

    let plugin_dir = create_test_plugin(&ctx, "reload-test", "1.0.0");

    let mut registry = PluginRegistry::new();
    registry.load_all(&[ctx.path().to_path_buf()]).unwrap();

    // Add a command after initial load
    let cmd_dir = plugin_dir.join("commands");
    fs::create_dir_all(&cmd_dir).unwrap();
    fs::write(cmd_dir.join("new-cmd.md"), "# New Command").unwrap();

    // Reload the plugin
    let reloaded = registry.reload_plugin("reload-test", &plugin_dir);
    assert!(reloaded.is_ok(), "Reload should succeed");

    // New command should be available
    assert!(
        registry.get_command("new-cmd").is_some(),
        "Reloaded plugin should have new command"
    );
}

// ============================================================================
// 6.5.1.1 Plugin Isolation Tests
// ============================================================================

#[test]
fn test_plugin_isolation_separate_namespaces() {
    let ctx = TestContext::new();

    // Create two plugins with commands of the same name
    let plugin1_dir = create_test_plugin(&ctx, "plugin-a", "1.0.0");
    let plugin2_dir = create_test_plugin(&ctx, "plugin-b", "1.0.0");

    let cmd_dir1 = plugin1_dir.join("commands");
    let cmd_dir2 = plugin2_dir.join("commands");
    fs::create_dir_all(&cmd_dir1).unwrap();
    fs::create_dir_all(&cmd_dir2).unwrap();

    fs::write(cmd_dir1.join("shared.md"), "# Plugin A Version").unwrap();
    fs::write(cmd_dir2.join("shared.md"), "# Plugin B Version").unwrap();

    let mut registry = PluginRegistry::new();
    registry.load_all(&[ctx.path().to_path_buf()]).unwrap();

    // Both should be accessible with full namespace
    let cmd_a = registry.get_command("plugin-a:shared");
    let cmd_b = registry.get_command("plugin-b:shared");

    assert!(cmd_a.is_some(), "Plugin A command should be accessible");
    assert!(cmd_b.is_some(), "Plugin B command should be accessible");
    assert_ne!(
        cmd_a.unwrap().content,
        cmd_b.unwrap().content,
        "Commands from different plugins should have different content"
    );
}

#[test]
fn test_plugin_isolation_manifest_info() {
    let ctx = TestContext::new();

    create_test_plugin(&ctx, "manifest-test", "2.5.0");

    let mut registry = PluginRegistry::new();
    registry.load_all(&[ctx.path().to_path_buf()]).unwrap();

    let manifest = registry.get_manifest("manifest-test");
    assert!(manifest.is_some());

    let manifest = manifest.unwrap();
    assert_eq!(manifest.name, "manifest-test");
    assert_eq!(manifest.version, "2.5.0");
}

#[test]
fn test_plugin_isolation_path_tracking() {
    let ctx = TestContext::new();

    create_test_plugin(&ctx, "path-test", "1.0.0");

    let mut registry = PluginRegistry::new();
    registry.load_all(&[ctx.path().to_path_buf()]).unwrap();

    let path = registry.get_plugin_path("path-test");
    assert!(path.is_some());
    assert!(path.unwrap().exists());
}

#[test]
fn test_plugin_list_all() {
    let ctx = TestContext::new();

    create_test_plugin(&ctx, "list-a", "1.0.0");
    create_test_plugin(&ctx, "list-b", "1.0.0");
    create_test_plugin(&ctx, "list-c", "1.0.0");

    let mut registry = PluginRegistry::new();
    registry.load_all(&[ctx.path().to_path_buf()]).unwrap();

    let names = registry.list_plugins();
    assert_eq!(names.len(), 3);
    assert!(names.contains(&"list-a".to_string()));
    assert!(names.contains(&"list-b".to_string()));
    assert!(names.contains(&"list-c".to_string()));
}

// ============================================================================
// 6.5.1.2 Tool Routing Tests
// ============================================================================

#[test]
fn test_tool_routing_by_full_name() {
    let ctx = TestContext::new();

    let plugin_dir = create_test_plugin(&ctx, "tool-plugin", "1.0.0");

    let cmd_dir = plugin_dir.join("commands");
    fs::create_dir_all(&cmd_dir).unwrap();
    fs::write(cmd_dir.join("my-tool.md"), "# My Tool\n\nThis is a tool.").unwrap();

    let mut registry = PluginRegistry::new();
    registry.load_all(&[ctx.path().to_path_buf()]).unwrap();

    // Should find with full namespaced name
    let tool = registry.get_command("tool-plugin:my-tool");
    assert!(tool.is_some(), "Tool should be found with full namespace");
    assert!(tool.unwrap().content.contains("My Tool"));
}

#[test]
fn test_tool_routing_by_short_name() {
    let ctx = TestContext::new();

    let plugin_dir = create_test_plugin(&ctx, "short-plugin", "1.0.0");

    let cmd_dir = plugin_dir.join("commands");
    fs::create_dir_all(&cmd_dir).unwrap();
    fs::write(cmd_dir.join("short-tool.md"), "# Short Tool").unwrap();

    let mut registry = PluginRegistry::new();
    registry.load_all(&[ctx.path().to_path_buf()]).unwrap();

    // Should find with short name (when unambiguous)
    let tool = registry.get_command("short-tool");
    assert!(tool.is_some(), "Tool should be found with short name");
}

#[test]
fn test_tool_routing_ambiguous_short_name() {
    let ctx = TestContext::new();

    let plugin1_dir = create_test_plugin(&ctx, "plugin-x", "1.0.0");
    let plugin2_dir = create_test_plugin(&ctx, "plugin-y", "1.0.0");

    let cmd_dir1 = plugin1_dir.join("commands");
    let cmd_dir2 = plugin2_dir.join("commands");
    fs::create_dir_all(&cmd_dir1).unwrap();
    fs::create_dir_all(&cmd_dir2).unwrap();

    fs::write(cmd_dir1.join("common.md"), "# From X").unwrap();
    fs::write(cmd_dir2.join("common.md"), "# From Y").unwrap();

    let mut registry = PluginRegistry::new();
    registry.load_all(&[ctx.path().to_path_buf()]).unwrap();

    // Short name should still resolve (to first found)
    let tool = registry.get_command("common");
    assert!(tool.is_some(), "Ambiguous short name should still resolve");

    // Full names should be distinct
    let tool_x = registry.get_command("plugin-x:common");
    let tool_y = registry.get_command("plugin-y:common");
    assert!(tool_x.is_some());
    assert!(tool_y.is_some());
    assert_ne!(tool_x.unwrap().content, tool_y.unwrap().content);
}

#[test]
fn test_tool_routing_nonexistent() {
    let ctx = TestContext::new();

    create_test_plugin(&ctx, "empty-plugin", "1.0.0");

    let mut registry = PluginRegistry::new();
    registry.load_all(&[ctx.path().to_path_buf()]).unwrap();

    let tool = registry.get_command("nonexistent-tool");
    assert!(tool.is_none(), "Nonexistent tool should return None");
}

#[test]
fn test_tool_routing_list_all_commands() {
    let ctx = TestContext::new();

    let plugin_dir = create_test_plugin(&ctx, "multi-cmd", "1.0.0");

    let cmd_dir = plugin_dir.join("commands");
    fs::create_dir_all(&cmd_dir).unwrap();
    fs::write(cmd_dir.join("cmd-a.md"), "# A").unwrap();
    fs::write(cmd_dir.join("cmd-b.md"), "# B").unwrap();
    fs::write(cmd_dir.join("cmd-c.md"), "# C").unwrap();

    let mut registry = PluginRegistry::new();
    registry.load_all(&[ctx.path().to_path_buf()]).unwrap();

    let commands = registry.list_commands();
    assert_eq!(commands.len(), 3);
    assert!(commands.contains(&"multi-cmd:cmd-a".to_string()));
    assert!(commands.contains(&"multi-cmd:cmd-b".to_string()));
    assert!(commands.contains(&"multi-cmd:cmd-c".to_string()));
}

#[test]
fn test_tool_routing_get_plugin_for_command() {
    let ctx = TestContext::new();

    let plugin_dir = create_test_plugin(&ctx, "owner-plugin", "1.0.0");

    let cmd_dir = plugin_dir.join("commands");
    fs::create_dir_all(&cmd_dir).unwrap();
    fs::write(cmd_dir.join("owned-cmd.md"), "# Owned").unwrap();

    let mut registry = PluginRegistry::new();
    registry.load_all(&[ctx.path().to_path_buf()]).unwrap();

    let plugin_name = registry.get_command_plugin("owner-plugin:owned-cmd");
    assert_eq!(plugin_name, Some("owner-plugin".to_string()));

    let plugin_name = registry.get_command_plugin("owned-cmd");
    assert_eq!(plugin_name, Some("owner-plugin".to_string()));

    let plugin_name = registry.get_command_plugin("nonexistent");
    assert_eq!(plugin_name, None);
}

#[test]
fn test_tool_routing_command_count() {
    let ctx = TestContext::new();

    let plugin1_dir = create_test_plugin(&ctx, "count-a", "1.0.0");
    let plugin2_dir = create_test_plugin(&ctx, "count-b", "1.0.0");

    let cmd_dir1 = plugin1_dir.join("commands");
    let cmd_dir2 = plugin2_dir.join("commands");
    fs::create_dir_all(&cmd_dir1).unwrap();
    fs::create_dir_all(&cmd_dir2).unwrap();

    fs::write(cmd_dir1.join("x.md"), "# X").unwrap();
    fs::write(cmd_dir1.join("y.md"), "# Y").unwrap();
    fs::write(cmd_dir2.join("z.md"), "# Z").unwrap();

    let mut registry = PluginRegistry::new();
    registry.load_all(&[ctx.path().to_path_buf()]).unwrap();

    assert_eq!(registry.command_count(), 3);
}

// ============================================================================
// Helper functions
// ============================================================================

fn create_test_plugin(ctx: &TestContext, name: &str, version: &str) -> std::path::PathBuf {
    let plugin_dir = ctx.path().join(name);
    let manifest_dir = plugin_dir.join(".claude-plugin");
    fs::create_dir_all(&manifest_dir).unwrap();

    let manifest = serde_json::json!({
        "name": name,
        "version": version,
        "description": format!("Test plugin: {}", name)
    });

    fs::write(
        manifest_dir.join("plugin.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    plugin_dir
}
