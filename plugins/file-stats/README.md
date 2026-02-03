# File Stats Plugin

A plugin that provides file statistics information.

## Description

This plugin demonstrates filesystem access from a plugin. It provides a tool
that returns metadata about files such as size, modification time, and permissions.

## Capabilities

- **tools**: Provides the `file_stats` tool

## Usage

```bash
# Install the plugin
patina plugin install ./plugins/file-stats

# The file_stats tool will be available in conversations
```

## Tool: file_stats

**Input**: A file path
**Output**: File metadata including:
- Size (bytes)
- Last modified time
- Created time (if available)
- File type (file, directory, symlink)
- Permissions (Unix mode)

## Security Note

This plugin demonstrates how filesystem permissions work in the plugin system.
File access is subject to the same sandbox restrictions as built-in tools.
