# Security Model

This document describes the security model used by Patina for command execution, particularly for the bash tool.

## Overview

Patina provides a layered security approach to command execution:

1. **Dangerous Pattern Blocklist** - Always-on protection against known dangerous commands
2. **Command Normalization** - Detects escape-based bypass attempts
3. **Allowlist Mode** - Optional strict mode that only permits explicitly allowed commands
4. **Path Validation** - Prevents file operations outside the working directory

## Security Modes

### Blocklist Mode (Default)

In blocklist mode, commands are **allowed by default** unless they match a dangerous pattern.

**Pros:**
- Flexible - users can run most safe commands
- Good for general development workflows
- Minimal friction for legitimate use

**Cons:**
- Cannot protect against all possible dangerous commands
- New bypass techniques may emerge over time
- Pattern matching is inherently reactive

**When to use:** General development, trusted environments, when flexibility is important.

### Allowlist Mode

In allowlist mode, commands are **blocked by default** unless they explicitly match an allowed pattern.

**Pros:**
- Maximum security - only known-good commands allowed
- Protects against unknown dangerous commands
- Defense in depth against bypass attempts

**Cons:**
- More restrictive - may block legitimate commands
- Requires configuration for each allowed command pattern
- Can impact productivity if patterns are too narrow

**When to use:** High-security environments, CI/CD pipelines, when running untrusted code, regulated industries.

## Enabling Allowlist Mode

To enable allowlist mode, configure the `ToolExecutionPolicy`:

```rust
use rct::tools::{ToolExecutor, ToolExecutionPolicy};
use regex::Regex;

let policy = ToolExecutionPolicy {
    allowlist_mode: true,
    allowed_commands: vec![
        Regex::new(r"^cargo\s+(build|test|check|clippy|fmt)").unwrap(),
        Regex::new(r"^git\s+(status|diff|log|add|commit)").unwrap(),
        Regex::new(r"^echo\s+").unwrap(),
        Regex::new(r"^cat\s+").unwrap(),
        Regex::new(r"^ls\s*").unwrap(),
    ],
    ..Default::default()
};

let executor = ToolExecutor::new(working_dir).with_policy(policy);
```

**Important:** Even in allowlist mode, the dangerous pattern blocklist is still enforced. This means even if a command matches an allowlist pattern, it will still be blocked if it matches a dangerous pattern.

## Dangerous Patterns

The following categories of commands are blocked by default:

### Destructive File Operations
- `rm -rf /` and variants
- `rm --no-preserve-root`

### Privilege Escalation
- `sudo`
- `su -`, `su root`, bare `su`
- `doas`
- `pkexec`
- `runuser`

### Dangerous Permissions
- `chmod 777`
- `chmod -R 777`
- `chmod u+s` (setuid)

### Disk/Filesystem Destruction
- `mkfs.*`
- `dd` writing to `/dev/*`
- Direct writes to block devices

### Resource Exhaustion
- Fork bombs (`:(){:|:&};:`)

### Remote Code Execution
- `curl ... | sh`
- `wget ... | bash`
- Base64-encoded commands piped to shell

### System Disruption
- `shutdown`, `reboot`, `halt`, `poweroff`

### History Manipulation
- `history -c`
- Redirecting to `.bash_history`

### Code Injection via Eval
- `eval $var`
- `eval "..."` (quoted variable expansion)
- `eval $(...)` (command substitution)

### Command Substitution Bypass Attempts
- `$(which ...)` - finding then executing commands
- Backtick command substitution with dangerous commands
- `$(printf '\x...')` - hex-encoded commands

## Bypass Protections

### Command Normalization

Commands are normalized before pattern matching to detect escape-based bypasses:

| Input | Normalized | Reason |
|-------|-----------|--------|
| `r\m -rf /` | `rm -rf /` | Shell escape removed |
| `s\u\d\o echo test` | `sudo echo test` | Multiple escapes removed |

This protects against attackers who try to obfuscate dangerous commands using shell escape sequences.

### Pattern Matching on Both Forms

Both the original command and the normalized form are checked against dangerous patterns, ensuring bypass attempts are caught.

## Known Limitations

### Inherent Limitations of Pattern Matching

1. **Turing-complete shell**: Bash is Turing-complete, meaning arbitrary code execution is always possible with enough creativity
2. **Encoding bypasses**: While we detect common encoding (base64, hex), novel encoding schemes may bypass detection
3. **Indirect execution**: Commands like `python -c "os.system('...')"` can execute arbitrary code
4. **File-based execution**: Writing a script to a file then executing it bypasses command-line pattern matching

### Mitigation Recommendations

For maximum security:

1. **Use allowlist mode** with narrow patterns
2. **Sandbox execution** using containers, VMs, or seccomp-bpf
3. **Drop privileges** before executing commands
4. **Use AppArmor/SELinux** for mandatory access control
5. **Monitor and audit** command execution logs
6. **Network isolation** to prevent data exfiltration

### Not a Sandbox

This security model provides **defense in depth** but is NOT a sandbox. It should be used in conjunction with proper sandboxing technologies (containers, VMs, seccomp) for untrusted code execution.

## Security Best Practices

1. **Principle of least privilege**: Only grant the minimum necessary permissions
2. **Defense in depth**: Combine multiple security layers
3. **Regular updates**: Keep patterns updated as new bypass techniques emerge
4. **Audit logging**: Log all command executions for forensic analysis
5. **Testing**: Regularly test security controls with known bypass attempts

## Reporting Security Issues

If you discover a security vulnerability or bypass technique, please report it responsibly:

1. Do not publicly disclose until fixed
2. Email security concerns to the maintainers
3. Include a proof-of-concept if possible
4. Allow reasonable time for a fix before disclosure

## Changelog

- **2026-01-30**: Added enhanced command validation with:
  - Command normalization for escape bypass detection
  - Expanded privilege escalation patterns (su root, pkexec, runuser)
  - Command substitution detection (`$(...)`, backticks)
  - Encoded command detection (base64, hex via printf)
  - Allowlist mode for strict security environments
