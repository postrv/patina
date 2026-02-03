//! Security policy for tool execution.
//!
//! This module provides security-related types and functions including:
//! - Dangerous command pattern detection
//! - Protected path configuration
//! - Command normalization for bypass detection
//! - Security policy configuration

use once_cell::sync::Lazy;
use regex::Regex;
use std::path::PathBuf;
use std::time::Duration;

/// Static collection of dangerous command patterns for Unix systems.
///
/// These patterns are compiled once on first access, ensuring:
/// - No runtime panics from invalid regex (patterns validated at initialization)
/// - No repeated compilation cost when creating new `ToolExecutionPolicy` instances
/// - Consistent pattern set across all policy instances
#[cfg(unix)]
pub(crate) static DANGEROUS_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        // Destructive file operations
        Regex::new(r"rm\s+-rf\s+/").expect("invalid regex: rm -rf"),
        Regex::new(r"rm\s+-fr\s+/").expect("invalid regex: rm -fr"),
        Regex::new(r"rm\s+--no-preserve-root").expect("invalid regex: rm --no-preserve-root"),
        // Privilege escalation - comprehensive patterns
        Regex::new(r"sudo\s+").expect("invalid regex: sudo"),
        Regex::new(r"\bsu\s+-").expect("invalid regex: su -"),
        Regex::new(r"\bsu\s+root\b").expect("invalid regex: su root"),
        Regex::new(r"\bsu\s*$").expect("invalid regex: bare su"),
        Regex::new(r"doas\s+").expect("invalid regex: doas"),
        Regex::new(r"\bpkexec\b").expect("invalid regex: pkexec"),
        Regex::new(r"\brunuser\b").expect("invalid regex: runuser"),
        // Dangerous permissions
        Regex::new(r"chmod\s+777").expect("invalid regex: chmod 777"),
        Regex::new(r"chmod\s+-R\s+777").expect("invalid regex: chmod -R 777"),
        Regex::new(r"chmod\s+u\+s").expect("invalid regex: chmod setuid"),
        // Disk/filesystem destruction
        Regex::new(r"mkfs\.").expect("invalid regex: mkfs"),
        Regex::new(r"dd\s+if=.+of=/dev/").expect("invalid regex: dd to device"),
        Regex::new(r">\s*/dev/sd[a-z]").expect("invalid regex: redirect to sd"),
        Regex::new(r">\s*/dev/nvme").expect("invalid regex: redirect to nvme"),
        // Fork bombs and resource exhaustion
        Regex::new(r":\(\)\s*\{\s*:\|:&\s*\}\s*;").expect("invalid regex: fork bomb"),
        // Remote code execution patterns
        Regex::new(r"curl\s+.+\|\s*(ba)?sh").expect("invalid regex: curl pipe sh"),
        Regex::new(r"wget\s+.+\|\s*(ba)?sh").expect("invalid regex: wget pipe sh"),
        Regex::new(r"curl\s+.+\|\s*sudo").expect("invalid regex: curl pipe sudo"),
        Regex::new(r"wget\s+.+\|\s*sudo").expect("invalid regex: wget pipe sudo"),
        // System disruption
        Regex::new(r"\bshutdown\b").expect("invalid regex: shutdown"),
        Regex::new(r"\breboot\b").expect("invalid regex: reboot"),
        Regex::new(r"\bhalt\b").expect("invalid regex: halt"),
        Regex::new(r"\bpoweroff\b").expect("invalid regex: poweroff"),
        // History manipulation (hiding tracks)
        Regex::new(r"history\s+-c").expect("invalid regex: history clear"),
        Regex::new(r">\s*~/\.bash_history").expect("invalid regex: bash_history redirect"),
        // Dangerous eval patterns
        Regex::new(r"\beval\s+\$").expect("invalid regex: eval var"),
        Regex::new(r#"\beval\s+["'$]"#).expect("invalid regex: eval string"),
        // Command substitution patterns
        Regex::new(r"\$\(\s*which\s+").expect("invalid regex: which substitution"),
        Regex::new(r"`\s*which\s+").expect("invalid regex: which backtick"),
        Regex::new(r"\$\(\s*printf\s+").expect("invalid regex: printf substitution"),
        // Encoded command execution patterns
        Regex::new(r"base64\s+(-d|--decode).*\|\s*(ba)?sh").expect("invalid regex: base64 decode"),
        Regex::new(r"\|\s*base64\s+(-d|--decode).*\|\s*(ba)?sh")
            .expect("invalid regex: piped base64"),
        Regex::new(r#"printf\s+["']\\x[0-9a-fA-F]"#).expect("invalid regex: printf hex"),
    ]
});

/// Static collection of dangerous command patterns for Windows systems.
///
/// These patterns detect dangerous Windows commands including:
/// - Recursive file deletion (del /s, rd /s)
/// - Disk formatting (format)
/// - Privilege escalation (runas)
/// - PowerShell code injection (encoded commands, Invoke-Expression)
/// - Registry manipulation (reg add/delete)
#[cfg(windows)]
pub(crate) static DANGEROUS_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        // Destructive file operations (case-insensitive for Windows)
        Regex::new(r"(?i)\bdel\s+/[sq]").expect("invalid regex: del /s or /q"),
        Regex::new(r"(?i)\bdel\s+.*/[sq]").expect("invalid regex: del with path"),
        Regex::new(r"(?i)\brd\s+/[sq]").expect("invalid regex: rd /s or /q"),
        Regex::new(r"(?i)\brmdir\s+/[sq]").expect("invalid regex: rmdir /s or /q"),
        // Disk formatting
        Regex::new(r"(?i)\bformat\s+[a-z]:").expect("invalid regex: format drive"),
        // Privilege escalation
        Regex::new(r"(?i)\brunas\s+/user").expect("invalid regex: runas"),
        // PowerShell dangers - encoded commands bypass security scanning
        Regex::new(r"(?i)powershell.*\s+-e\s").expect("invalid regex: powershell -e"),
        Regex::new(r"(?i)powershell.*\s+-enc\s").expect("invalid regex: powershell -enc"),
        Regex::new(r"(?i)powershell.*\s+-encodedcommand\s")
            .expect("invalid regex: powershell -encodedcommand"),
        // PowerShell Invoke-Expression - executes arbitrary code
        Regex::new(r"(?i)\biex\s*\(").expect("invalid regex: iex()"),
        Regex::new(r"(?i)\binvoke-expression\b").expect("invalid regex: Invoke-Expression"),
        // Registry manipulation
        Regex::new(r"(?i)\breg\s+delete\b").expect("invalid regex: reg delete"),
        Regex::new(r"(?i)\breg\s+add\b").expect("invalid regex: reg add"),
        // System disruption (shared with Unix but case-insensitive on Windows)
        Regex::new(r"(?i)\bshutdown\b").expect("invalid regex: shutdown"),
        // Remote code execution patterns (Windows versions)
        Regex::new(r"(?i)curl\s+.+\|\s*powershell").expect("invalid regex: curl pipe powershell"),
        Regex::new(r"(?i)invoke-webrequest.*\|\s*iex").expect("invalid regex: IWR pipe iex"),
        // Certutil abuse for downloading/decoding (common attack vector)
        Regex::new(r"(?i)certutil\s+-urlcache").expect("invalid regex: certutil download"),
        Regex::new(r"(?i)certutil\s+-decode").expect("invalid regex: certutil decode"),
    ]
});

/// Security policy for tool execution.
///
/// # Security Modes
///
/// The policy supports two security modes:
///
/// - **Blocklist mode** (default): Commands are allowed unless they match a dangerous pattern.
///   Good for general-purpose use where flexibility is needed.
///
/// - **Allowlist mode**: Commands are blocked unless they match an allowed pattern.
///   More restrictive, suitable for high-security environments. Enable by setting
///   `allowlist_mode = true` and providing patterns in `allowed_commands`.
///
/// In both modes, dangerous patterns are always checked and will block matching commands.
pub struct ToolExecutionPolicy {
    /// Patterns that match dangerous commands (always blocked).
    pub dangerous_patterns: Vec<Regex>,
    /// Paths that are protected from write operations.
    pub protected_paths: Vec<PathBuf>,
    /// Maximum allowed file size for write operations.
    pub max_file_size: usize,
    /// Maximum allowed output size for bash commands (P0-3: prevents memory issues).
    ///
    /// When command output exceeds this limit, it will be truncated with a notice.
    /// Default is 1MB.
    pub max_output_size: usize,
    /// Timeout for command execution.
    pub command_timeout: Duration,
    /// Enable allowlist mode (default: false).
    ///
    /// When enabled, only commands matching `allowed_commands` will be permitted.
    /// Dangerous patterns are still enforced on top of the allowlist.
    pub allowlist_mode: bool,
    /// Patterns for commands that are allowed in allowlist mode.
    ///
    /// Only used when `allowlist_mode` is true.
    pub allowed_commands: Vec<Regex>,
}

impl Default for ToolExecutionPolicy {
    fn default() -> Self {
        Self {
            dangerous_patterns: DANGEROUS_PATTERNS.clone(),
            protected_paths: default_protected_paths(),
            max_file_size: 10 * 1024 * 1024,
            max_output_size: 1024 * 1024, // 1MB default for bash output
            command_timeout: Duration::from_secs(300),
            allowlist_mode: false,
            allowed_commands: vec![],
        }
    }
}

/// Returns platform-specific protected paths.
#[cfg(unix)]
fn default_protected_paths() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/etc"),
        PathBuf::from("/usr"),
        PathBuf::from("/bin"),
    ]
}

/// Returns platform-specific protected paths for Windows.
#[cfg(windows)]
fn default_protected_paths() -> Vec<PathBuf> {
    vec![
        PathBuf::from(r"C:\Windows"),
        PathBuf::from(r"C:\Program Files"),
        PathBuf::from(r"C:\Program Files (x86)"),
    ]
}

/// Normalizes a command string by removing shell escape characters.
///
/// This helps detect bypass attempts where characters are escaped to avoid
/// pattern matching (e.g., `r\m` becoming `rm` after shell processing).
pub fn normalize_command(cmd: &str) -> String {
    let mut result = String::with_capacity(cmd.len());
    let mut chars = cmd.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            // Skip the backslash and include the next character literally
            // unless it's a special escape sequence we want to preserve
            if let Some(&next) = chars.peek() {
                match next {
                    // Preserve common escape sequences that don't affect command names
                    'n' | 't' | 'r' | '0' | 'x' => {
                        result.push(c);
                        result.push(chars.next().unwrap());
                    }
                    // For letters, the backslash is often used to bypass filters
                    // e.g., r\m -> rm, so we skip the backslash
                    'a'..='z' | 'A'..='Z' => {
                        result.push(chars.next().unwrap());
                    }
                    // For other characters, preserve both
                    _ => {
                        result.push(c);
                        result.push(chars.next().unwrap());
                    }
                }
            } else {
                result.push(c);
            }
        } else {
            result.push(c);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_command_basic() {
        assert_eq!(normalize_command("ls"), "ls");
        assert_eq!(normalize_command("echo hello"), "echo hello");
    }

    #[test]
    fn test_normalize_command_escape_bypass() {
        // Should detect escape-based bypass attempts
        assert_eq!(normalize_command(r"r\m -rf /"), "rm -rf /");
        assert_eq!(normalize_command(r"su\do command"), "sudo command");
    }

    #[test]
    fn test_normalize_command_preserve_special() {
        // Should preserve actual escape sequences
        assert_eq!(normalize_command(r"echo \n"), r"echo \n");
        assert_eq!(normalize_command(r"echo \t"), r"echo \t");
    }

    #[test]
    fn test_default_policy() {
        let policy = ToolExecutionPolicy::default();
        assert!(!policy.dangerous_patterns.is_empty());
        assert!(!policy.protected_paths.is_empty());
        assert_eq!(policy.max_file_size, 10 * 1024 * 1024);
        assert_eq!(policy.max_output_size, 1024 * 1024);
        assert_eq!(policy.command_timeout, Duration::from_secs(300));
        assert!(!policy.allowlist_mode);
        assert!(policy.allowed_commands.is_empty());
    }

    #[test]
    fn test_dangerous_patterns_block_sudo() {
        let policy = ToolExecutionPolicy::default();
        let cmd = "sudo rm -rf /";
        assert!(policy.dangerous_patterns.iter().any(|p| p.is_match(cmd)));
    }

    #[test]
    fn test_dangerous_patterns_block_rm_rf() {
        let policy = ToolExecutionPolicy::default();
        let cmd = "rm -rf /";
        assert!(policy.dangerous_patterns.iter().any(|p| p.is_match(cmd)));
    }

    #[test]
    fn test_dangerous_patterns_allow_safe() {
        let policy = ToolExecutionPolicy::default();
        let cmd = "ls -la";
        assert!(!policy.dangerous_patterns.iter().any(|p| p.is_match(cmd)));
    }
}
