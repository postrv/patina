# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |

## Reporting a Vulnerability

**Please do not report security vulnerabilities through public GitHub issues.**

Instead, report security vulnerabilities by emailing the maintainers directly. You should receive a response within 48 hours. If you don't hear back, please follow up.

When reporting, please include:

- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Any suggested fixes (optional)

## Security Model

RCT implements multiple layers of security to protect users when Claude executes code on their behalf.

### Command Execution Security

The tool executor blocks dangerous shell commands by default:

**Blocked Patterns:**
- Destructive file operations: `rm -rf /`, `rm --no-preserve-root`
- Privilege escalation: `sudo`, `su -`, `doas`
- Dangerous permissions: `chmod 777`, `chmod u+s` (setuid)
- Disk/filesystem operations: `mkfs.*`, `dd if=* of=/dev/*`
- Remote code execution: `curl | bash`, `wget | sh`
- System disruption: `shutdown`, `reboot`, `halt`, `poweroff`
- History manipulation: `history -c`, `> ~/.bash_history`

### Path Traversal Protection

All file operations validate paths to prevent escaping the working directory:

- Absolute paths are rejected
- `..` path components are blocked
- Paths are canonicalized before access
- Symlinks are not followed

### Protected Directories

Write operations are blocked in system directories:

- `/etc`
- `/usr`
- `/bin`

### Timeout Enforcement

All commands have configurable timeouts (default: 5 minutes) to prevent:

- Resource exhaustion
- Hanging operations
- Denial of service

### Backup System

Files are automatically backed up before modification:

- Backups stored in `.rct_backups/`
- Timestamped filenames prevent overwrites
- Original content preserved for recovery

## Hooks Security

Hooks execute user-defined shell commands. Users should:

- Review hook configurations carefully
- Avoid storing sensitive data in hook commands
- Use exit code 2 to block dangerous operations

Hook context is passed via stdin as JSON, not command-line arguments, to prevent injection.

## API Key Security

- API keys are stored using the `secrecy` crate
- Keys are never logged or displayed
- Keys are redacted in debug output
- Use environment variables rather than command-line arguments

## MCP Server Security

When connecting to MCP servers:

- Only connect to trusted servers
- Review server permissions before enabling
- Use authentication headers for remote servers
- Monitor tool calls in audit logs

## Audit Logging

Enable audit logging for security monitoring:

```toml
[audit]
enabled = true
level = "all"
```

Audit logs capture:
- All tool invocations
- API calls
- Session lifecycle events

## Best Practices

### For Users

1. **Review permissions**: Understand what tools are available
2. **Enable audit logging**: Monitor operations for suspicious activity
3. **Use cost controls**: Set budget limits to prevent runaway costs
4. **Review hooks**: Audit hook configurations regularly
5. **Update regularly**: Keep RCT updated for security fixes

### For Developers

1. **Follow TDD**: Tests catch security regressions
2. **Run security scans**: Use `cargo audit` and narsil scans
3. **Validate all input**: Never trust external data
4. **Use safe defaults**: Secure by default, opt-in to risk
5. **Document security implications**: Note when operations have security impact

## Known Limitations

- Command blocking uses regex patterns which may not catch all variants
- Path validation depends on filesystem behavior
- Timeout enforcement requires cooperative process termination
- Audit logs are stored locally without integrity protection

## Security Updates

Security patches are released as point releases (e.g., 0.1.1, 0.1.2). We recommend:

- Subscribing to release notifications
- Updating promptly when security releases are announced
- Reviewing changelogs for security-related changes

## Acknowledgments

We appreciate responsible disclosure of security vulnerabilities. Contributors who report valid security issues will be acknowledged (with permission) in release notes.

---

*Last updated: 2026-01-30*
