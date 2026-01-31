# Patina Plugin API

This document describes the stable API for developing Patina plugins.

## Overview

Patina plugins extend functionality through:
- **Commands**: Slash commands invoked by users (e.g., `/my-plugin:greet`)
- **Tools**: Agent tools that the AI can use
- **Skills**: Context-aware instructions for specific tasks

## Plugin Structure

A plugin is a directory with the following structure:

```
my-plugin/
  .claude-plugin/
    plugin.json      # Plugin manifest (required)
  commands/
    my-command.md    # Slash command definitions
  skills/
    my-skill/
      SKILL.md       # Skill definition
  hooks/
    hooks.json       # Hook definitions
```

## Plugin Manifest

The `plugin.json` file is required and defines plugin metadata:

```json
{
  "name": "my-plugin",
  "version": "1.0.0",
  "description": "A description of what this plugin does",
  "author": "Your Name",
  "min_rct_version": "0.1.0"
}
```

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `name` | Yes | Unique plugin identifier (lowercase, hyphens allowed) |
| `version` | Yes | Semantic version string |
| `description` | No | Brief description |
| `author` | No | Plugin author |
| `min_rct_version` | No | Minimum Patina version required |

## Commands

Commands are markdown files in the `commands/` directory.

### Example Command

`commands/greet.md`:
```markdown
# Greet Command

Say hello to the user.

## Usage

/my-plugin:greet [name]

## Arguments

- `name` (optional): Name to greet. Defaults to "World".
```

Commands can be invoked as:
- `/my-plugin:greet` - Full namespaced name
- `/greet` - Short name (if unambiguous)

## Skills

Skills provide context-aware instructions. Each skill is a directory with a `SKILL.md` file.

### Skill Format

`skills/debugging/SKILL.md`:
```markdown
---
name: debugging
description: Help debug code issues
keywords:
  - debug
  - error
  - bug
file_patterns:
  - "*.rs"
  - "*.py"
---

# Debugging Instructions

When debugging code:
1. First, understand the error message
2. Check the stack trace
3. Add logging to isolate the issue
...
```

### Frontmatter Fields

| Field | Required | Description |
|-------|----------|-------------|
| `name` | Yes | Skill identifier |
| `description` | Yes | Brief description |
| `keywords` | No | Keywords for matching |
| `file_patterns` | No | File globs for automatic activation |

## Host API (Programmatic Plugins)

For advanced plugins, implement the Rust traits directly:

### RctPlugin Trait

```rust
use patina::plugins::host::{RctPlugin, PluginInfo};

impl RctPlugin for MyPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            name: "my-plugin".to_string(),
            version: "1.0.0".to_string(),
            description: Some("My plugin".to_string()),
        }
    }

    fn on_load(&mut self) -> anyhow::Result<()> {
        // Initialize resources
        Ok(())
    }

    fn on_unload(&mut self) -> anyhow::Result<()> {
        // Cleanup resources
        Ok(())
    }
}
```

### CommandProvider Trait

```rust
use patina::plugins::host::{CommandProvider, PluginCommand};

impl CommandProvider for MyPlugin {
    fn commands(&self) -> Vec<PluginCommand> {
        vec![PluginCommand {
            name: "greet".to_string(),
            description: "Say hello".to_string(),
            documentation: "# Greet\n\nSay hello to the user.".to_string(),
        }]
    }

    fn execute(&self, name: &str, args: &str) -> anyhow::Result<String> {
        match name {
            "greet" => Ok(format!("Hello, {}!", args)),
            _ => anyhow::bail!("Unknown command: {}", name),
        }
    }
}
```

### ToolProvider Trait

```rust
use patina::plugins::host::{ToolProvider, PluginTool};

impl ToolProvider for MyPlugin {
    fn tools(&self) -> Vec<PluginTool> {
        vec![PluginTool {
            name: "calculate".to_string(),
            description: "Perform calculations".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "expression": {
                        "type": "string",
                        "description": "Math expression to evaluate"
                    }
                },
                "required": ["expression"]
            }),
        }]
    }

    fn execute_tool(&self, name: &str, input: serde_json::Value)
        -> anyhow::Result<serde_json::Value>
    {
        match name {
            "calculate" => {
                let expr = input["expression"].as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing expression"))?;
                // Evaluate expression...
                Ok(serde_json::json!({ "result": 42 }))
            }
            _ => anyhow::bail!("Unknown tool: {}", name),
        }
    }
}
```

## Plugin Discovery

Patina searches for plugins in these locations (in order):
1. `.claude/plugins/` in the current project
2. `~/.config/claude/plugins/` for user plugins
3. `/usr/share/claude/plugins/` for system plugins

## Namespacing

All plugin resources are namespaced to avoid conflicts:
- Commands: `plugin-name:command-name`
- Tools: `plugin-name:tool-name`
- Skills: `plugin-name:skill-name`

Short names (without namespace) work when unambiguous.

## Version Compatibility

- Plugins specify `min_rct_version` in their manifest
- Patina checks compatibility before loading
- Incompatible plugins are skipped with a warning

## Security

Plugins run with the same permissions as Patina:
- Can read/write files in the working directory
- Can execute commands (subject to tool restrictions)
- Cannot bypass security policies

## Best Practices

1. **Use meaningful names**: Choose clear, descriptive plugin names
2. **Document commands**: Include usage examples in command markdown
3. **Handle errors gracefully**: Return helpful error messages
4. **Test your plugin**: Verify all commands and tools work correctly
5. **Version properly**: Follow semantic versioning
