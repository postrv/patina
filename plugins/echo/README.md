# Echo Plugin

A simple echo tool that demonstrates basic tool registration in Patina.

## Description

This plugin provides an `echo` tool that returns whatever message is sent to it.
It serves as a minimal example of how to create a tool-providing plugin.

## Capabilities

- **tools**: Provides the `echo` tool

## Usage

```bash
# Install the plugin
patina plugin install ./plugins/echo

# The echo tool will be available in conversations
```

## Tool: echo

**Input**: A text message
**Output**: The same text message, echoed back

This is useful for testing tool execution and plugin integration.
