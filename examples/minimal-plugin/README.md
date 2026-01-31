# Minimal Plugin Example

This is a minimal Patina plugin demonstrating the standard plugin structure.

## Structure

```
minimal-plugin/
  .claude-plugin/
    plugin.json       # Plugin manifest
  commands/
    hello.md          # /hello command
    echo.md           # /echo command
  skills/
    greeting-skill/
      SKILL.md        # Greeting skill definition
```

## Installation

Copy this directory to one of:
- `.claude/plugins/` in your project
- `~/.config/claude/plugins/` for user-wide access

## Usage

After installation, the commands are available:

```
/minimal-plugin:hello Alice
/hello World

/minimal-plugin:echo Hello!
/echo Testing
```

## Extending

To add your own commands:
1. Create a `.md` file in `commands/`
2. Follow the format in `hello.md` or `echo.md`

To add skills:
1. Create a directory in `skills/`
2. Add a `SKILL.md` with YAML frontmatter and instructions
