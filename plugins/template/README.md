# Template Plugin

A minimal plugin template for Patina.

## Usage

1. Copy this directory to create your own plugin
2. Rename and update `rct-plugin.toml` with your plugin's details
3. Enable the capabilities you need (commands, skills, tools, hooks, mcp)
4. Add your implementation files
5. Install with: `patina plugin install ./your-plugin`

## Manifest Fields

| Field | Required | Description |
|-------|----------|-------------|
| `name` | Yes | Plugin name (lowercase, alphanumeric with hyphens) |
| `version` | Yes | Semantic version (e.g., "1.0.0") |
| `description` | No | Brief description of your plugin |
| `author` | No | Your name or organization |

## Capabilities

Enable capabilities in the `[capabilities]` section:

- `commands` - Provide slash commands
- `skills` - Provide skills for the agent
- `tools` - Provide tools for the agent
- `hooks` - Provide lifecycle hooks
- `mcp` - Provide an MCP server (requires `[mcp]` config)

## Example

```toml
name = "my-plugin"
version = "1.0.0"
description = "My awesome plugin"
author = "Your Name"

[capabilities]
tools = true
commands = true
```
