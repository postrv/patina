//! Slash command handler for the TUI application.
//!
//! This module provides the integration layer between user input and the
//! slash command system. It:
//! - Detects when user input is a slash command (starts with `/`)
//! - Dispatches to the appropriate command handler
//! - Returns results for display in the TUI
//!
//! # Example
//!
//! ```rust
//! use patina::app::commands::{SlashCommandHandler, CommandResult};
//! use std::path::PathBuf;
//!
//! let handler = SlashCommandHandler::new(PathBuf::from("."));
//!
//! match handler.handle("/help") {
//!     CommandResult::Executed(output) => println!("{}", output),
//!     CommandResult::NotACommand => println!("Not a slash command"),
//!     CommandResult::UnknownCommand(cmd) => println!("Unknown: {}", cmd),
//!     CommandResult::Error(e) => println!("Error: {}", e),
//! }
//! ```

use crate::commands::worktree::{parse_worktree_command, WorktreeCommand};
use crate::worktree::{WorktreeInfo, WorktreeManager};
use std::path::PathBuf;

/// Information about a loaded plugin for display purposes.
#[derive(Debug, Clone, Default)]
pub struct PluginInfo {
    /// The plugin name.
    pub name: String,
    /// The plugin version.
    pub version: String,
    /// Optional description of the plugin.
    pub description: Option<String>,
    /// List of command names provided by this plugin.
    pub commands: Vec<String>,
    /// List of skill names provided by this plugin.
    pub skills: Vec<String>,
}

/// Result of handling a slash command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandResult {
    /// The command was executed successfully with output.
    Executed(String),

    /// The input was not a slash command (doesn't start with `/`).
    NotACommand,

    /// The command was not recognized.
    UnknownCommand(String),

    /// An error occurred while executing the command.
    Error(String),
}

/// Handler for slash commands in the TUI.
///
/// Parses user input, identifies slash commands, and dispatches to the
/// appropriate handler.
pub struct SlashCommandHandler {
    /// Working directory for command execution.
    working_dir: PathBuf,
    /// Information about loaded plugins.
    plugins: Vec<PluginInfo>,
}

impl SlashCommandHandler {
    /// Creates a new slash command handler.
    #[must_use]
    pub fn new(working_dir: PathBuf) -> Self {
        Self {
            working_dir,
            plugins: Vec::new(),
        }
    }

    /// Adds plugin information to the handler.
    ///
    /// Use this to enable the `/plugins` command to display loaded plugins.
    #[must_use]
    pub fn with_plugins(mut self, plugins: Vec<PluginInfo>) -> Self {
        self.plugins = plugins;
        self
    }

    /// Handles user input, checking if it's a slash command.
    ///
    /// # Arguments
    ///
    /// * `input` - The raw user input string
    ///
    /// # Returns
    ///
    /// A `CommandResult` indicating the outcome:
    /// - `Executed(output)` - Command ran successfully
    /// - `NotACommand` - Input doesn't start with `/`
    /// - `UnknownCommand(name)` - Slash command not recognized
    /// - `Error(message)` - Command failed with error
    pub fn handle(&self, input: &str) -> CommandResult {
        let trimmed = input.trim();

        // Check if input is a slash command
        if !trimmed.starts_with('/') {
            return CommandResult::NotACommand;
        }

        // Parse the command name and arguments
        let without_slash = &trimmed[1..];
        let mut parts = without_slash.split_whitespace();

        let command_name = match parts.next() {
            Some(name) => name,
            None => return CommandResult::Error("Empty command".to_string()),
        };

        let args: String = parts.collect::<Vec<_>>().join(" ");

        // Dispatch to the appropriate handler
        match command_name {
            "worktree" => self.handle_worktree(&args),
            "help" => self.handle_help(if args.is_empty() { None } else { Some(&args) }),
            "plugins" => self.handle_plugins(),
            "terminal-setup" => self.handle_terminal_setup(),
            _ => CommandResult::UnknownCommand(command_name.to_string()),
        }
    }

    /// Handles the `/plugins` command.
    fn handle_plugins(&self) -> CommandResult {
        if self.plugins.is_empty() {
            return CommandResult::Executed("No plugins loaded.".to_string());
        }

        let mut output = String::from("Loaded Plugins:\n");

        for plugin in &self.plugins {
            output.push_str(&format!("\n  {} v{}", plugin.name, plugin.version));

            if let Some(desc) = &plugin.description {
                output.push_str(&format!("\n    {}", desc));
            }

            if !plugin.commands.is_empty() {
                let cmd_list = plugin.commands.join(", ");
                output.push_str(&format!(
                    "\n    Commands ({}): {}",
                    plugin.commands.len(),
                    cmd_list
                ));
            }

            if !plugin.skills.is_empty() {
                let skill_list = plugin.skills.join(", ");
                output.push_str(&format!(
                    "\n    Skills ({}): {}",
                    plugin.skills.len(),
                    skill_list
                ));
            }
        }

        CommandResult::Executed(output)
    }

    /// Formats a worktree entry for display.
    fn format_worktree(wt: &WorktreeInfo) -> String {
        let branch = if wt.branch.is_empty() {
            "detached"
        } else {
            &wt.branch
        };
        format!("  {} ({}) - {}", wt.name, branch, wt.path.display())
    }

    /// Formats a worktree entry with full status details.
    fn format_worktree_status(wt: &WorktreeInfo) -> String {
        let branch = if wt.branch.is_empty() {
            "detached"
        } else {
            &wt.branch
        };
        format!(
            "  {} ({})\n    Path: {}",
            wt.name,
            branch,
            wt.path.display()
        )
    }

    /// Handles the `/worktree` command.
    fn handle_worktree(&self, args: &str) -> CommandResult {
        let worktree_cmd = match parse_worktree_command(args) {
            Ok(cmd) => cmd,
            Err(e) => return CommandResult::Error(e.to_string()),
        };

        // Create worktree manager - handle potential failure
        let manager = match WorktreeManager::new(&self.working_dir) {
            Ok(m) => m,
            Err(e) => {
                return CommandResult::Error(format!(
                    "Failed to initialize worktree manager: {}",
                    e
                ))
            }
        };

        match worktree_cmd {
            WorktreeCommand::New { name } => match manager.create(&name) {
                Ok(info) => CommandResult::Executed(format!(
                    "Created worktree '{}' at {}",
                    name,
                    info.path.display()
                )),
                Err(e) => CommandResult::Error(format!("Failed to create worktree: {}", e)),
            },

            WorktreeCommand::List => match manager.list() {
                Ok(worktrees) if worktrees.is_empty() => {
                    CommandResult::Executed("No worktrees found.".to_string())
                }
                Ok(worktrees) => {
                    let output = worktrees
                        .iter()
                        .map(Self::format_worktree)
                        .collect::<Vec<_>>()
                        .join("\n");
                    CommandResult::Executed(format!("Worktrees:\n{}", output))
                }
                Err(e) => CommandResult::Error(format!("Failed to list worktrees: {}", e)),
            },

            WorktreeCommand::Switch { name } => {
                // Switch is not directly applicable in TUI context
                // Just report what would happen
                CommandResult::Executed(format!(
                    "To switch to worktree '{}', open a new terminal in that directory.",
                    name
                ))
            }

            WorktreeCommand::Remove { name } => match manager.remove(&name) {
                Ok(()) => CommandResult::Executed(format!("Removed worktree '{}'", name)),
                Err(e) => CommandResult::Error(format!("Failed to remove worktree: {}", e)),
            },

            WorktreeCommand::Clean => {
                // Clean prunable worktrees using git worktree prune
                match std::process::Command::new("git")
                    .args(["worktree", "prune"])
                    .current_dir(&self.working_dir)
                    .output()
                {
                    Ok(output) => {
                        if output.status.success() {
                            CommandResult::Executed("Pruned stale worktree entries.".to_string())
                        } else {
                            let stderr = String::from_utf8_lossy(&output.stderr);
                            CommandResult::Error(format!("Failed to prune worktrees: {}", stderr))
                        }
                    }
                    Err(e) => {
                        CommandResult::Error(format!("Failed to run git worktree prune: {}", e))
                    }
                }
            }

            WorktreeCommand::Status => match manager.list() {
                Ok(worktrees) if worktrees.is_empty() => {
                    CommandResult::Executed("No worktrees found.".to_string())
                }
                Ok(worktrees) => {
                    let output = worktrees
                        .iter()
                        .map(Self::format_worktree_status)
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    CommandResult::Executed(format!("Worktree Status:\n{}", output))
                }
                Err(e) => CommandResult::Error(format!("Failed to get worktree status: {}", e)),
            },
        }
    }

    /// Handles the `/help` command.
    fn handle_help(&self, command: Option<&str>) -> CommandResult {
        match command {
            None => {
                // General help listing all commands
                let help_text = r#"Available Commands:

  /worktree <subcommand>  - Manage git worktrees
    Subcommands: new, list, switch, remove, clean, status

  /plugins                - List loaded plugins

  /terminal-setup         - Configure terminal keyboard shortcuts

  /help [command]         - Show help for a command

Type /help <command> for detailed help on a specific command."#;
                CommandResult::Executed(help_text.to_string())
            }

            Some("worktree") => {
                let help_text = r#"/worktree - Manage git worktrees

Subcommands:
  new <name>     Create a new worktree with the given name
  list           List all worktrees in the repository
  switch <name>  Switch to an existing worktree
  remove <name>  Remove an existing worktree
  clean          Remove prunable worktrees (missing directories)
  status         Show status of all worktrees

Examples:
  /worktree new feature-123
  /worktree list
  /worktree remove feature-123"#;
                CommandResult::Executed(help_text.to_string())
            }

            Some("help") => {
                let help_text = r#"/help - Show help information

Usage:
  /help          Show list of all available commands
  /help <cmd>    Show detailed help for a specific command

Examples:
  /help
  /help worktree"#;
                CommandResult::Executed(help_text.to_string())
            }

            Some("plugins") => {
                let help_text = r#"/plugins - List loaded plugins

Usage:
  /plugins       Show all loaded plugins with details

Displays:
  - Plugin name and version
  - Plugin description (if available)
  - Commands provided by the plugin
  - Skills provided by the plugin

Plugins are loaded from ~/.config/patina/plugins/ at startup.
Use --no-plugins flag to disable plugin loading."#;
                CommandResult::Executed(help_text.to_string())
            }

            Some("terminal-setup") => {
                let help_text = r#"/terminal-setup - Configure terminal for optimal keyboard shortcuts

Usage:
  /terminal-setup    Auto-detect terminal and show configuration instructions

This command helps configure your terminal for Cmd+A/C/V (macOS) or
equivalent shortcuts. Configuration depends on your terminal:

iTerm2:
  Automatically configured on first run. Restart iTerm2 if prompted.

JetBrains (RustRover, IntelliJ, etc.):
  Requires manual configuration:
  1. Open Settings → Tools → Terminal
  2. Enable "Use Option as Meta key"
  3. Use Option+A/C/V for select all, copy, paste

Kitty, WezTerm, Ghostty:
  Cmd+A/C/V works natively (Kitty keyboard protocol).

Other terminals:
  Use Ctrl+A (select all), Ctrl+Y (copy), Ctrl+Shift+V (paste)."#;
                CommandResult::Executed(help_text.to_string())
            }

            Some(cmd) => CommandResult::UnknownCommand(cmd.to_string()),
        }
    }

    /// Handles the `/terminal-setup` command.
    ///
    /// Detects the current terminal and provides configuration instructions
    /// for enabling Cmd+A/C/V (macOS) or equivalent shortcuts.
    fn handle_terminal_setup(&self) -> CommandResult {
        use crate::terminal::{
            configure_iterm2_keybindings, is_iterm2, is_jetbrains_terminal, is_kitty_terminal,
            is_macos,
        };

        let mut output = String::from("🔧 Terminal Keyboard Configuration\n\n");

        if is_iterm2() {
            output.push_str("Terminal: iTerm2\n\n");
            match configure_iterm2_keybindings() {
                Ok(true) => {
                    output.push_str("✅ Configured iTerm2 key bindings!\n\n");
                    output.push_str("Please restart iTerm2 for changes to take effect.\n\n");
                    output.push_str("After restart, you can use:\n");
                    output.push_str("  • Cmd+A - Select all\n");
                    output.push_str("  • Cmd+C - Copy selection\n");
                    output.push_str("  • Cmd+V - Paste\n");
                }
                Ok(false) => {
                    output.push_str("✅ iTerm2 is already configured!\n\n");
                    output.push_str("You can use:\n");
                    output.push_str("  • Cmd+A - Select all\n");
                    output.push_str("  • Cmd+C - Copy selection\n");
                    output.push_str("  • Cmd+V - Paste\n");
                }
                Err(e) => {
                    output.push_str(&format!("⚠️  Failed to configure: {}\n\n", e));
                    output.push_str("Manual setup:\n");
                    output.push_str("  1. Open iTerm2 → Settings → Profiles → Keys\n");
                    output
                        .push_str("  2. Add key mappings for Cmd+A/C/V to send escape sequences\n");
                }
            }
        } else if is_jetbrains_terminal() {
            output.push_str("Terminal: JetBrains IDE (RustRover, IntelliJ, etc.)\n\n");
            output.push_str("⚠️  JetBrains terminals don't support Cmd+key passthrough.\n");
            output.push_str("The IDE intercepts Cmd+keys for its own shortcuts.\n\n");
            output.push_str("📋 To enable Option+A/C/V shortcuts:\n\n");
            output.push_str("  1. Open IDE Settings (Cmd+,)\n");
            output.push_str("  2. Navigate to: Tools → Terminal\n");
            output.push_str("  3. Enable: \"Use Option as Meta key\" ✓\n");
            output.push_str("  4. Click Apply, then OK\n\n");
            output.push_str("After configuration, you can use:\n");
            output.push_str("  • Option+A - Select all\n");
            output.push_str("  • Option+C - Copy selection\n");
            output.push_str("  • Option+V - Paste\n\n");
            output.push_str("Alternative shortcuts (always work):\n");
            output.push_str("  • Ctrl+A    - Select all\n");
            output.push_str("  • Ctrl+Y    - Copy selection (yank)\n");
            output.push_str("  • Ctrl+Shift+V - Paste\n");
        } else if is_kitty_terminal() {
            output.push_str("Terminal: Kitty\n\n");
            output.push_str("✅ Kitty supports the Kitty keyboard protocol natively!\n\n");
            output.push_str("You can use:\n");
            output.push_str("  • Cmd+A - Select all\n");
            output.push_str("  • Cmd+C - Copy selection\n");
            output.push_str("  • Cmd+V - Paste\n");
        } else if is_macos() {
            output.push_str("Terminal: macOS (unknown terminal)\n\n");
            output.push_str("Your terminal may not support Cmd+key detection.\n\n");
            output.push_str("Recommended terminals with Cmd+key support:\n");
            output.push_str("  • iTerm2 (configure with /terminal-setup)\n");
            output.push_str("  • Kitty (native support)\n");
            output.push_str("  • WezTerm (native support)\n");
            output.push_str("  • Ghostty (native support)\n\n");
            output.push_str("Universal shortcuts (always work):\n");
            output.push_str("  • Ctrl+A    - Select all\n");
            output.push_str("  • Ctrl+Y    - Copy selection (yank)\n");
            output.push_str("  • Ctrl+Shift+V - Paste\n");
        } else {
            output.push_str("Terminal: Linux/Windows\n\n");
            output.push_str("Standard keyboard shortcuts:\n");
            output.push_str("  • Ctrl+A       - Select all\n");
            output.push_str("  • Ctrl+Y       - Copy selection (yank)\n");
            output.push_str("  • Ctrl+Shift+V - Paste\n");
        }

        CommandResult::Executed(output)
    }

    /// Returns available command names for tab completion.
    #[must_use]
    pub fn available_commands(&self) -> Vec<&'static str> {
        vec!["worktree", "help", "plugins", "terminal-setup"]
    }

    /// Creates plugin info from a plugin registry.
    ///
    /// Extracts plugin metadata including name, version, description,
    /// commands, and skills for display by the `/plugins` command.
    #[must_use]
    pub fn build_plugin_info(registry: &crate::plugins::PluginRegistry) -> Vec<PluginInfo> {
        let plugin_names = registry.list_plugins();
        let all_commands = registry.list_commands();

        plugin_names
            .into_iter()
            .map(|name| {
                let manifest = registry.get_manifest(&name);

                // Collect commands for this plugin (format: "plugin:command")
                let prefix = format!("{}:", name);
                let commands: Vec<String> = all_commands
                    .iter()
                    .filter(|cmd| cmd.starts_with(&prefix))
                    .map(|cmd| cmd.strip_prefix(&prefix).unwrap_or(cmd).to_string())
                    .collect();

                // Get skills from manifest (future enhancement)
                let skills = Vec::new();

                PluginInfo {
                    name: name.clone(),
                    version: manifest
                        .map(|m| m.version.clone())
                        .unwrap_or_else(|| "unknown".to_string()),
                    description: manifest.and_then(|m| m.description.clone()),
                    commands,
                    skills,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_handler_in_temp() -> (SlashCommandHandler, TempDir) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let handler = SlashCommandHandler::new(temp_dir.path().to_path_buf());
        (handler, temp_dir)
    }

    // =========================================================================
    // Basic command detection tests
    // =========================================================================

    #[test]
    fn test_not_a_command_no_slash() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("hello world");
        assert_eq!(result, CommandResult::NotACommand);
    }

    #[test]
    fn test_not_a_command_empty() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("");
        assert_eq!(result, CommandResult::NotACommand);
    }

    #[test]
    fn test_not_a_command_whitespace_only() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("   ");
        assert_eq!(result, CommandResult::NotACommand);
    }

    #[test]
    fn test_empty_command_error() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/");
        assert_eq!(result, CommandResult::Error("Empty command".to_string()));
    }

    #[test]
    fn test_unknown_command() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/foobar");
        assert_eq!(result, CommandResult::UnknownCommand("foobar".to_string()));
    }

    // =========================================================================
    // Worktree command tests
    // =========================================================================

    #[test]
    fn test_handle_slash_command_worktree_new() {
        let (handler, _temp) = create_handler_in_temp();

        // Note: This will fail because temp_dir is not a git repo
        // The test documents the expected behavior - proper error handling
        let result = handler.handle("/worktree new my-feature");

        match result {
            CommandResult::Error(msg) => {
                // Expected: fails because not a git repo or can't create
                assert!(
                    msg.contains("Failed to create worktree")
                        || msg.contains("Failed to initialize")
                        || msg.contains("not a git repository"),
                    "Should report failure: {}",
                    msg
                );
            }
            CommandResult::Executed(_) => {
                // Would succeed in a real git repo
            }
            other => panic!("Unexpected result: {:?}", other),
        }
    }

    #[test]
    fn test_handle_slash_command_worktree_list() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/worktree list");

        // Should either list worktrees or report error (not a git repo)
        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("Worktrees:") || output.contains("No worktrees"),
                    "Should show worktree info: {}",
                    output
                );
            }
            CommandResult::Error(msg) => {
                // Expected in non-git directory
                assert!(
                    msg.contains("Failed to list")
                        || msg.contains("Failed to initialize")
                        || msg.contains("not a git repository"),
                    "Error: {}",
                    msg
                );
            }
            other => panic!("Unexpected result: {:?}", other),
        }
    }

    #[test]
    fn test_handle_slash_command_worktree_missing_arg() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/worktree new");

        match result {
            CommandResult::Error(msg) => {
                assert!(
                    msg.contains("missing") || msg.contains("argument"),
                    "Should report missing argument: {}",
                    msg
                );
            }
            other => panic!("Expected error for missing argument: {:?}", other),
        }
    }

    #[test]
    fn test_handle_slash_command_worktree_unknown_subcommand() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/worktree unknown");

        match result {
            CommandResult::Error(msg) => {
                assert!(
                    msg.contains("unknown") || msg.contains("Unknown"),
                    "Should report unknown subcommand: {}",
                    msg
                );
            }
            other => panic!("Expected error for unknown subcommand: {:?}", other),
        }
    }

    // =========================================================================
    // Help command tests
    // =========================================================================

    #[test]
    fn test_handle_slash_command_help() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/help");

        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("Available Commands"),
                    "Should list commands: {}",
                    output
                );
                assert!(output.contains("worktree"), "Should mention worktree");
                assert!(output.contains("help"), "Should mention help");
            }
            other => panic!("Expected help output: {:?}", other),
        }
    }

    #[test]
    fn test_handle_slash_command_help_worktree() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/help worktree");

        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("/worktree"),
                    "Should describe worktree: {}",
                    output
                );
                assert!(output.contains("new"), "Should list new subcommand");
                assert!(output.contains("list"), "Should list list subcommand");
            }
            other => panic!("Expected worktree help: {:?}", other),
        }
    }

    #[test]
    fn test_handle_slash_command_help_unknown() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/help unknown");

        assert_eq!(result, CommandResult::UnknownCommand("unknown".to_string()));
    }

    // =========================================================================
    // Edge cases and whitespace handling
    // =========================================================================

    #[test]
    fn test_command_with_extra_whitespace() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("  /help  ");

        match result {
            CommandResult::Executed(_) => {}
            other => panic!("Should handle whitespace: {:?}", other),
        }
    }

    #[test]
    fn test_available_commands() {
        let (handler, _temp) = create_handler_in_temp();

        let commands = handler.available_commands();

        assert!(commands.contains(&"worktree"));
        assert!(commands.contains(&"help"));
    }

    // =========================================================================
    // CommandResult equality tests
    // =========================================================================

    #[test]
    fn test_command_result_equality() {
        assert_eq!(CommandResult::NotACommand, CommandResult::NotACommand);
        assert_eq!(
            CommandResult::Executed("test".to_string()),
            CommandResult::Executed("test".to_string())
        );
        assert_ne!(
            CommandResult::Executed("a".to_string()),
            CommandResult::Executed("b".to_string())
        );
    }

    #[test]
    fn test_command_result_debug() {
        let result = CommandResult::Executed("output".to_string());
        let debug_str = format!("{:?}", result);
        assert!(debug_str.contains("Executed"));
    }

    // =========================================================================
    // Plugin command tests
    // =========================================================================

    #[test]
    fn test_handle_slash_command_plugins_no_plugins() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/plugins");

        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("No plugins loaded") || output.contains("no plugins"),
                    "Should indicate no plugins: {}",
                    output
                );
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_handle_slash_command_plugins_with_plugins() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let handler = SlashCommandHandler::new(temp_dir.path().to_path_buf());

        // Add mock plugin info
        let plugins = vec![
            PluginInfo {
                name: "test-plugin".to_string(),
                version: "1.0.0".to_string(),
                description: Some("A test plugin".to_string()),
                commands: vec!["test-cmd".to_string()],
                skills: vec!["test-skill".to_string()],
            },
            PluginInfo {
                name: "another-plugin".to_string(),
                version: "2.0.0".to_string(),
                description: None,
                commands: vec![],
                skills: vec![],
            },
        ];
        let handler = handler.with_plugins(plugins);

        let result = handler.handle("/plugins");

        match result {
            CommandResult::Executed(output) => {
                // Should list plugin names and versions
                assert!(output.contains("test-plugin"), "Should show plugin name");
                assert!(output.contains("1.0.0"), "Should show version");
                assert!(
                    output.contains("another-plugin"),
                    "Should show second plugin"
                );
                assert!(output.contains("2.0.0"), "Should show second version");
            }
            other => panic!("Expected plugin listing: {:?}", other),
        }
    }

    #[test]
    fn test_handle_slash_command_plugins_shows_commands_and_skills() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let handler = SlashCommandHandler::new(temp_dir.path().to_path_buf());

        let plugins = vec![PluginInfo {
            name: "my-plugin".to_string(),
            version: "1.0.0".to_string(),
            description: Some("Plugin with features".to_string()),
            commands: vec!["cmd1".to_string(), "cmd2".to_string()],
            skills: vec!["skill1".to_string()],
        }];
        let handler = handler.with_plugins(plugins);

        let result = handler.handle("/plugins");

        match result {
            CommandResult::Executed(output) => {
                // Should show commands and skills count or names
                assert!(
                    output.contains("cmd1") || output.contains("2 command"),
                    "Should show commands info: {}",
                    output
                );
                assert!(
                    output.contains("skill1") || output.contains("1 skill"),
                    "Should show skills info: {}",
                    output
                );
            }
            other => panic!("Expected plugin details: {:?}", other),
        }
    }

    #[test]
    fn test_help_includes_plugins_command() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/help");

        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("plugins"),
                    "Help should mention plugins command: {}",
                    output
                );
            }
            other => panic!("Expected help output: {:?}", other),
        }
    }

    #[test]
    fn test_help_plugins_shows_detailed_help() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/help plugins");

        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("/plugins"),
                    "Should describe plugins command"
                );
                assert!(
                    output.contains("loaded") || output.contains("list"),
                    "Should explain what it does: {}",
                    output
                );
            }
            other => panic!("Expected plugins help: {:?}", other),
        }
    }

    #[test]
    fn test_available_commands_includes_plugins() {
        let (handler, _temp) = create_handler_in_temp();

        let commands = handler.available_commands();

        assert!(
            commands.contains(&"plugins"),
            "Available commands should include 'plugins'"
        );
    }

    // =========================================================================
    // Terminal setup command tests
    // =========================================================================

    #[test]
    fn test_handle_terminal_setup_command() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/terminal-setup");

        match result {
            CommandResult::Executed(output) => {
                // Should contain terminal configuration info
                assert!(
                    output.contains("Terminal") || output.contains("terminal"),
                    "Should mention terminal: {}",
                    output
                );
                // Should contain keyboard shortcut info
                assert!(
                    output.contains("Ctrl") || output.contains("Cmd") || output.contains("Option"),
                    "Should mention keyboard shortcuts: {}",
                    output
                );
            }
            other => panic!("Expected executed result: {:?}", other),
        }
    }

    #[test]
    fn test_help_includes_terminal_setup() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/help");

        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("terminal-setup"),
                    "Help should mention terminal-setup command: {}",
                    output
                );
            }
            other => panic!("Expected help output: {:?}", other),
        }
    }

    #[test]
    fn test_help_terminal_setup_shows_detailed_help() {
        let (handler, _temp) = create_handler_in_temp();

        let result = handler.handle("/help terminal-setup");

        match result {
            CommandResult::Executed(output) => {
                assert!(
                    output.contains("/terminal-setup"),
                    "Should describe terminal-setup command"
                );
                assert!(
                    output.contains("JetBrains") || output.contains("iTerm"),
                    "Should mention supported terminals: {}",
                    output
                );
            }
            other => panic!("Expected terminal-setup help: {:?}", other),
        }
    }

    #[test]
    fn test_available_commands_includes_terminal_setup() {
        let (handler, _temp) = create_handler_in_temp();

        let commands = handler.available_commands();

        assert!(
            commands.contains(&"terminal-setup"),
            "Available commands should include 'terminal-setup'"
        );
    }
}
