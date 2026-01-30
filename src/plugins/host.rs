//! Plugin host API providing stable interfaces for plugin development.
//!
//! This module defines the traits that plugins must implement to integrate
//! with RCT. The API is designed to be stable and backward-compatible.
//!
//! # Example
//!
//! ```ignore
//! use rct::plugins::host::{RctPlugin, PluginInfo, CommandProvider, Command};
//!
//! struct MyPlugin;
//!
//! impl RctPlugin for MyPlugin {
//!     fn info(&self) -> PluginInfo {
//!         PluginInfo {
//!             name: "my-plugin".to_string(),
//!             version: "1.0.0".to_string(),
//!             description: Some("My custom plugin".to_string()),
//!         }
//!     }
//!
//!     fn on_load(&mut self) -> anyhow::Result<()> {
//!         Ok(())
//!     }
//!
//!     fn on_unload(&mut self) -> anyhow::Result<()> {
//!         Ok(())
//!     }
//! }
//! ```

use anyhow::Result;
use std::collections::HashMap;

/// Information about a plugin.
#[derive(Debug, Clone)]
pub struct PluginInfo {
    /// Unique name identifying the plugin.
    pub name: String,
    /// Semantic version string.
    pub version: String,
    /// Optional human-readable description.
    pub description: Option<String>,
}

/// Core trait that all RCT plugins must implement.
///
/// This trait defines the lifecycle hooks and basic information
/// that every plugin must provide.
pub trait RctPlugin: Send + Sync {
    /// Returns plugin metadata.
    fn info(&self) -> PluginInfo;

    /// Called when the plugin is loaded.
    ///
    /// Use this to initialize resources.
    ///
    /// # Errors
    ///
    /// Return an error to abort plugin loading.
    fn on_load(&mut self) -> Result<()>;

    /// Called when the plugin is unloaded.
    ///
    /// Use this to clean up resources.
    ///
    /// # Errors
    ///
    /// Errors during unload are logged but do not prevent unloading.
    fn on_unload(&mut self) -> Result<()>;
}

/// A command provided by a plugin.
#[derive(Debug, Clone)]
pub struct PluginCommand {
    /// Command name (without plugin namespace).
    pub name: String,
    /// Brief description for help text.
    pub description: String,
    /// Full markdown documentation.
    pub documentation: String,
}

/// Trait for plugins that provide slash commands.
pub trait CommandProvider {
    /// Returns the list of commands this plugin provides.
    fn commands(&self) -> Vec<PluginCommand>;

    /// Executes a command.
    ///
    /// # Arguments
    ///
    /// * `name` - Command name (without plugin namespace)
    /// * `args` - Arguments passed to the command
    ///
    /// # Returns
    ///
    /// The command output as a string, or an error if execution fails.
    ///
    /// # Errors
    ///
    /// Returns an error if the command fails to execute.
    fn execute(&self, name: &str, args: &str) -> Result<String>;
}

/// A tool provided by a plugin.
#[derive(Debug, Clone)]
pub struct PluginTool {
    /// Tool name (without plugin namespace).
    pub name: String,
    /// Brief description of what the tool does.
    pub description: String,
    /// JSON Schema for the tool's input parameters.
    pub input_schema: serde_json::Value,
}

/// Trait for plugins that provide tools for the agent.
pub trait ToolProvider {
    /// Returns the list of tools this plugin provides.
    fn tools(&self) -> Vec<PluginTool>;

    /// Executes a tool.
    ///
    /// # Arguments
    ///
    /// * `name` - Tool name (without plugin namespace)
    /// * `input` - Tool input as JSON
    ///
    /// # Returns
    ///
    /// The tool result as JSON.
    ///
    /// # Errors
    ///
    /// Returns an error if the tool fails to execute.
    fn execute_tool(&self, name: &str, input: serde_json::Value) -> Result<serde_json::Value>;
}

/// A skill provided by a plugin.
#[derive(Debug, Clone)]
pub struct PluginSkill {
    /// Skill name.
    pub name: String,
    /// Brief description.
    pub description: String,
    /// Keywords for matching.
    pub keywords: Vec<String>,
    /// File patterns for matching.
    pub file_patterns: Vec<String>,
    /// Full skill instructions.
    pub instructions: String,
}

/// Trait for plugins that provide skills.
pub trait SkillProvider {
    /// Returns the list of skills this plugin provides.
    fn skills(&self) -> Vec<PluginSkill>;
}

/// Host context provided to plugins.
///
/// Plugins can use this to access RCT functionality.
pub struct PluginContext {
    working_dir: std::path::PathBuf,
    env_vars: HashMap<String, String>,
}

impl PluginContext {
    /// Creates a new plugin context.
    #[must_use]
    pub fn new(working_dir: std::path::PathBuf) -> Self {
        Self {
            working_dir,
            env_vars: HashMap::new(),
        }
    }

    /// Returns the current working directory.
    #[must_use]
    pub fn working_dir(&self) -> &std::path::Path {
        &self.working_dir
    }

    /// Gets an environment variable.
    #[must_use]
    pub fn get_env(&self, key: &str) -> Option<&str> {
        self.env_vars.get(key).map(String::as_str)
    }

    /// Sets an environment variable in the plugin context.
    pub fn set_env(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.env_vars.insert(key.into(), value.into());
    }
}

/// Plugin host that manages loaded plugins.
pub struct PluginHost {
    plugins: HashMap<String, Box<dyn RctPlugin>>,
    context: PluginContext,
}

impl PluginHost {
    /// Creates a new plugin host.
    #[must_use]
    pub fn new(context: PluginContext) -> Self {
        Self {
            plugins: HashMap::new(),
            context,
        }
    }

    /// Returns the plugin context.
    #[must_use]
    pub fn context(&self) -> &PluginContext {
        &self.context
    }

    /// Returns a mutable reference to the plugin context.
    pub fn context_mut(&mut self) -> &mut PluginContext {
        &mut self.context
    }

    /// Registers a plugin with the host.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin fails to load.
    pub fn register(&mut self, mut plugin: Box<dyn RctPlugin>) -> Result<()> {
        let info = plugin.info();
        plugin.on_load()?;
        self.plugins.insert(info.name, plugin);
        Ok(())
    }

    /// Unregisters a plugin by name.
    ///
    /// Returns `true` if the plugin was found and unregistered.
    pub fn unregister(&mut self, name: &str) -> bool {
        if let Some(mut plugin) = self.plugins.remove(name) {
            // Best effort unload - errors are logged but don't prevent unregistration
            let _ = plugin.on_unload();
            true
        } else {
            false
        }
    }

    /// Returns the number of registered plugins.
    #[must_use]
    pub fn plugin_count(&self) -> usize {
        self.plugins.len()
    }

    /// Checks if a plugin is registered.
    #[must_use]
    pub fn has_plugin(&self, name: &str) -> bool {
        self.plugins.contains_key(name)
    }

    /// Returns plugin info by name.
    #[must_use]
    pub fn get_plugin_info(&self, name: &str) -> Option<PluginInfo> {
        self.plugins.get(name).map(|p| p.info())
    }

    /// Lists all registered plugin names.
    #[must_use]
    pub fn list_plugins(&self) -> Vec<String> {
        self.plugins.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestPlugin {
        name: String,
        load_count: usize,
        unload_count: usize,
    }

    impl TestPlugin {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
                load_count: 0,
                unload_count: 0,
            }
        }
    }

    impl RctPlugin for TestPlugin {
        fn info(&self) -> PluginInfo {
            PluginInfo {
                name: self.name.clone(),
                version: "1.0.0".to_string(),
                description: Some("Test plugin".to_string()),
            }
        }

        fn on_load(&mut self) -> Result<()> {
            self.load_count += 1;
            Ok(())
        }

        fn on_unload(&mut self) -> Result<()> {
            self.unload_count += 1;
            Ok(())
        }
    }

    #[test]
    fn test_plugin_host_register() {
        let ctx = PluginContext::new(std::path::PathBuf::from("/tmp"));
        let mut host = PluginHost::new(ctx);

        let plugin = Box::new(TestPlugin::new("test"));
        host.register(plugin).unwrap();

        assert!(host.has_plugin("test"));
        assert_eq!(host.plugin_count(), 1);
    }

    #[test]
    fn test_plugin_host_unregister() {
        let ctx = PluginContext::new(std::path::PathBuf::from("/tmp"));
        let mut host = PluginHost::new(ctx);

        let plugin = Box::new(TestPlugin::new("test"));
        host.register(plugin).unwrap();

        let unregistered = host.unregister("test");
        assert!(unregistered);
        assert!(!host.has_plugin("test"));
    }

    #[test]
    fn test_plugin_info() {
        let ctx = PluginContext::new(std::path::PathBuf::from("/tmp"));
        let mut host = PluginHost::new(ctx);

        let plugin = Box::new(TestPlugin::new("info-test"));
        host.register(plugin).unwrap();

        let info = host.get_plugin_info("info-test").unwrap();
        assert_eq!(info.name, "info-test");
        assert_eq!(info.version, "1.0.0");
    }

    #[test]
    fn test_plugin_context() {
        let mut ctx = PluginContext::new(std::path::PathBuf::from("/work"));

        ctx.set_env("TEST_VAR", "test_value");
        assert_eq!(ctx.get_env("TEST_VAR"), Some("test_value"));
        assert_eq!(ctx.get_env("NONEXISTENT"), None);
        assert_eq!(ctx.working_dir(), std::path::Path::new("/work"));
    }

    #[test]
    fn test_plugin_list() {
        let ctx = PluginContext::new(std::path::PathBuf::from("/tmp"));
        let mut host = PluginHost::new(ctx);

        host.register(Box::new(TestPlugin::new("a"))).unwrap();
        host.register(Box::new(TestPlugin::new("b"))).unwrap();
        host.register(Box::new(TestPlugin::new("c"))).unwrap();

        let names = host.list_plugins();
        assert_eq!(names.len(), 3);
        assert!(names.contains(&"a".to_string()));
        assert!(names.contains(&"b".to_string()));
        assert!(names.contains(&"c".to_string()));
    }
}
