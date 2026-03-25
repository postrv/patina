//! MCP command security validation.
//!
//! Validates that MCP server commands are safe to execute, preventing:
//! - Execution of dangerous commands (rm, sudo, etc.)
//! - Path traversal attacks (../../../bin/rm)
//! - Relative path exploitation (./malicious_script)
//! - Shell injection via arguments
//!
//! # Example
//!
//! ```
//! use patina::mcp::security::validate_mcp_command;
//!
//! // Absolute path to a legitimate server is allowed
//! assert!(validate_mcp_command("/usr/bin/mcp-server", &[]).is_ok());
//!
//! // Dangerous commands are blocked
//! assert!(validate_mcp_command("rm", &[]).is_err());
//! ```

use crate::error::{RctError, RctResult};
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashMap;
use std::path::Path;

/// Commands that are ALWAYS blocked, even with absolute paths (Unix).
///
/// These commands have no legitimate use as MCP servers and could cause
/// system damage or privilege escalation.
#[cfg(unix)]
static ALWAYS_BLOCKED: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        // Destructive file operations
        Regex::new(r"^rm$").unwrap(),
        Regex::new(r"^rmdir$").unwrap(),
        Regex::new(r"^shred$").unwrap(),
        // Privilege escalation
        Regex::new(r"^sudo$").unwrap(),
        Regex::new(r"^su$").unwrap(),
        Regex::new(r"^doas$").unwrap(),
        Regex::new(r"^pkexec$").unwrap(),
        Regex::new(r"^runuser$").unwrap(),
        // Disk operations
        Regex::new(r"^dd$").unwrap(),
        Regex::new(r"^mkfs").unwrap(),
        Regex::new(r"^fdisk$").unwrap(),
        // System control
        Regex::new(r"^shutdown$").unwrap(),
        Regex::new(r"^reboot$").unwrap(),
        Regex::new(r"^halt$").unwrap(),
        Regex::new(r"^poweroff$").unwrap(),
        // Network tools that could exfiltrate data
        Regex::new(r"^nc$").unwrap(),
        Regex::new(r"^netcat$").unwrap(),
        Regex::new(r"^ncat$").unwrap(),
    ]
});

/// Commands that are ALWAYS blocked, even with absolute paths (Windows).
///
/// These commands have no legitimate use as MCP servers and could cause
/// system damage or privilege escalation.
#[cfg(windows)]
static ALWAYS_BLOCKED: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        // Registry manipulation (case-insensitive for Windows)
        Regex::new(r"(?i)^reg\.exe$").unwrap(),
        Regex::new(r"(?i)^reg$").unwrap(),
        // System control
        Regex::new(r"(?i)^shutdown\.exe$").unwrap(),
        Regex::new(r"(?i)^shutdown$").unwrap(),
        // Disk formatting
        Regex::new(r"(?i)^format\.com$").unwrap(),
        Regex::new(r"(?i)^format$").unwrap(),
        // Destructive file operations when used as MCP server
        Regex::new(r"(?i)^del\.exe$").unwrap(),
        Regex::new(r"(?i)^rmdir\.exe$").unwrap(),
        Regex::new(r"(?i)^rd\.exe$").unwrap(),
    ]
});

/// Commands that require an absolute path to be used (Unix).
///
/// These are interpreters that could be legitimate MCP server hosts
/// when specified with an absolute path, showing clear intent.
/// Without an absolute path, they could be PATH-hijacked.
#[cfg(unix)]
static REQUIRE_ABSOLUTE_PATH: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        // Shell interpreters
        Regex::new(r"^(ba)?sh$").unwrap(),
        Regex::new(r"^zsh$").unwrap(),
        Regex::new(r"^fish$").unwrap(),
        Regex::new(r"^csh$").unwrap(),
        Regex::new(r"^tcsh$").unwrap(),
        Regex::new(r"^ksh$").unwrap(),
        Regex::new(r"^dash$").unwrap(),
        // Script interpreters
        Regex::new(r"^python[0-9.]*$").unwrap(),
        Regex::new(r"^perl$").unwrap(),
        Regex::new(r"^ruby$").unwrap(),
        Regex::new(r"^node$").unwrap(),
        Regex::new(r"^php$").unwrap(),
    ]
});

/// Commands that require an absolute path to be used (Windows).
///
/// These are interpreters that could be legitimate MCP server hosts
/// when specified with an absolute path, showing clear intent.
/// Without an absolute path, they could be PATH-hijacked.
#[cfg(windows)]
static REQUIRE_ABSOLUTE_PATH: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        // Windows shell interpreters (case-insensitive)
        Regex::new(r"(?i)^cmd\.exe$").unwrap(),
        Regex::new(r"(?i)^cmd$").unwrap(),
        Regex::new(r"(?i)^powershell\.exe$").unwrap(),
        Regex::new(r"(?i)^powershell$").unwrap(),
        Regex::new(r"(?i)^pwsh\.exe$").unwrap(),
        Regex::new(r"(?i)^pwsh$").unwrap(),
        // Script interpreters (also on Windows)
        Regex::new(r"(?i)^python[0-9.]*\.exe$").unwrap(),
        Regex::new(r"(?i)^python[0-9.]*$").unwrap(),
        Regex::new(r"(?i)^perl\.exe$").unwrap(),
        Regex::new(r"(?i)^perl$").unwrap(),
        Regex::new(r"(?i)^ruby\.exe$").unwrap(),
        Regex::new(r"(?i)^ruby$").unwrap(),
        Regex::new(r"(?i)^node\.exe$").unwrap(),
        Regex::new(r"(?i)^node$").unwrap(),
        Regex::new(r"(?i)^php\.exe$").unwrap(),
        Regex::new(r"(?i)^php$").unwrap(),
    ]
});

/// Dangerous argument patterns that indicate shell injection attempts (Unix).
#[cfg(unix)]
static DANGEROUS_ARG_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        // Shell command chaining
        Regex::new(r";\s*rm\s").unwrap(),
        Regex::new(r";\s*sudo\s").unwrap(),
        Regex::new(r"\|\s*sh").unwrap(),
        Regex::new(r"\|\s*bash").unwrap(),
        // Command substitution
        Regex::new(r"\$\(").unwrap(),
        Regex::new(r"`").unwrap(),
        // Dangerous redirects
        Regex::new(r">\s*/dev/").unwrap(),
        Regex::new(r">\s*/etc/").unwrap(),
    ]
});

/// Dangerous argument patterns that indicate shell injection attempts (Windows).
///
/// These patterns detect:
/// - PowerShell encoded commands (-EncodedCommand, -enc, -e)
/// - Invoke-Expression (iex) for arbitrary code execution
/// - Destructive commands (del /s, format, rd /s)
/// - Registry manipulation (reg delete, reg add)
#[cfg(windows)]
static DANGEROUS_ARG_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        // PowerShell encoded commands (base64 bypass)
        Regex::new(r"(?i)-e(nc(odedcommand)?)?(\s|$)").unwrap(),
        // PowerShell Invoke-Expression (arbitrary code execution)
        Regex::new(r"(?i)\biex\s*\(").unwrap(),
        Regex::new(r"(?i)\binvoke-expression\b").unwrap(),
        // Destructive file operations
        Regex::new(r"(?i)\bdel\s+/[sq]").unwrap(),
        Regex::new(r"(?i)\bdel\s+.*/[sq]").unwrap(),
        Regex::new(r"(?i)\brd\s+/[sq]").unwrap(),
        Regex::new(r"(?i)\brmdir\s+/[sq]").unwrap(),
        // Disk formatting
        Regex::new(r"(?i)\bformat\s+[a-z]:").unwrap(),
        // Registry manipulation
        Regex::new(r"(?i)\breg\s+(delete|add)\b").unwrap(),
        // Command chaining with dangerous commands
        Regex::new(r"(?i)&\s*del\s").unwrap(),
        Regex::new(r"(?i)&\s*format\s").unwrap(),
    ]
});

/// Checks if a path is absolute on the current platform.
///
/// # Unix
/// - Paths starting with `/` are absolute
///
/// # Windows
/// - Paths starting with drive letter (e.g., `C:\`) are absolute
/// - UNC paths (e.g., `\\server\share`) are absolute
fn is_absolute_path(path: &str) -> bool {
    // Unix: starts with /
    if path.starts_with('/') {
        return true;
    }

    // Windows: drive letter (e.g., C:\ or C:/)
    // Check for pattern like "X:" where X is a letter, followed by \ or /
    let bytes = path.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let second = bytes[1];
        if first.is_ascii_alphabetic() && second == b':' {
            // It's a drive letter path (absolute on Windows)
            return true;
        }
    }

    // Windows: UNC path (\\server\share)
    if path.starts_with(r"\\") {
        return true;
    }

    false
}

/// Validates that an MCP command is safe to execute.
///
/// # Security Checks
///
/// 1. **Path traversal**: Commands with `..` are rejected.
///
/// 2. **Relative paths**: Commands starting with `./` or `../` are rejected.
///
/// 3. **Always blocked commands**: Commands like `rm`, `sudo`, `dd` are blocked
///    even with absolute paths, as they have no legitimate use as MCP servers.
///
/// 4. **Interpreter commands**: Shell and script interpreters (`bash`, `python`)
///    are allowed ONLY when specified with an absolute path (e.g., `/bin/bash`).
///    This ensures the user explicitly chooses which binary to run.
///
/// 5. **Argument validation**: Arguments are checked for shell injection
///    patterns like `; rm -rf /` or `$(malicious)`.
///
/// # Platform Support
///
/// - **Unix**: Recognizes `/path/to/command` as absolute
/// - **Windows**: Recognizes `C:\path\to\command` and `\\server\share\command` as absolute
///
/// # Errors
///
/// Returns `RctError::McpValidation` if validation fails. The error is
/// security-related and can be checked via `is_security_related()`.
pub fn validate_mcp_command(command: &str, args: &[String]) -> RctResult<()> {
    // Check for path traversal (works for both Unix and Windows separators)
    if command.contains("..") {
        return Err(RctError::mcp_validation(
            "path traversal not allowed in MCP command",
        ));
    }

    // Check for relative paths
    // Unix: starts with ./
    // Windows: starts with .\ or ./
    if command.starts_with("./") || command.starts_with(r".\") {
        return Err(RctError::mcp_validation(
            "relative paths not allowed for MCP servers; use absolute paths",
        ));
    }

    // Determine if this is an absolute path
    // Unix: starts with /
    // Windows: starts with drive letter (C:\) or UNC path (\\server)
    let is_absolute = is_absolute_path(command);

    // Get the command basename for pattern matching
    let command_name = Path::new(command)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(command);

    // Check against always-blocked commands (even with absolute paths)
    for pattern in ALWAYS_BLOCKED.iter() {
        if pattern.is_match(command_name) {
            return Err(RctError::mcp_validation(format!(
                "security policy blocked '{}': command not allowed as MCP server",
                command_name
            )));
        }
    }

    // Check against commands that require absolute paths
    // These are interpreters that are legitimate when explicitly specified
    for pattern in REQUIRE_ABSOLUTE_PATH.iter() {
        if pattern.is_match(command_name) && !is_absolute {
            return Err(RctError::mcp_validation(format!(
                "'{}' requires an absolute path (e.g., /bin/{}) to prevent PATH hijacking",
                command_name, command_name
            )));
        }
    }

    // Check if this is a known interpreter (shell or script interpreter)
    // For interpreters, we skip argument validation because the script content
    // inherently contains shell constructs - that's the intended behavior.
    // The key protection is requiring an absolute path.
    let is_interpreter = REQUIRE_ABSOLUTE_PATH
        .iter()
        .any(|pattern| pattern.is_match(command_name));

    // Only check arguments for shell injection patterns on non-interpreters
    // For interpreters like /bin/bash, the script content IS the intended execution
    if !is_interpreter {
        for arg in args {
            for pattern in DANGEROUS_ARG_PATTERNS.iter() {
                if pattern.is_match(arg) {
                    return Err(RctError::mcp_validation(
                        "security policy blocked: potential shell injection in argument",
                    ));
                }
            }
        }
    }

    Ok(())
}

/// Environment variable names that are forbidden in MCP server processes.
///
/// These variables can be used to inject shared libraries into child processes,
/// enabling arbitrary code execution. They must never be passed to MCP servers.
static DANGEROUS_ENV_VARS: Lazy<Vec<&str>> = Lazy::new(|| {
    vec![
        "LD_PRELOAD",
        "LD_LIBRARY_PATH",
        "DYLD_INSERT_LIBRARIES",
        "DYLD_FALLBACK_LIBRARY_PATH",
        "DYLD_FRAMEWORK_PATH",
    ]
});

/// Validates that MCP environment variables do not contain dangerous entries.
///
/// Rejects environment variables that could be used to inject shared libraries
/// into the child process (e.g., `LD_PRELOAD`, `DYLD_INSERT_LIBRARIES`).
///
/// # Errors
///
/// Returns `RctError::McpValidation` if any dangerous environment variable is present.
///
/// # Examples
///
/// ```
/// use patina::mcp::security::validate_mcp_env;
/// use std::collections::HashMap;
///
/// // Normal variables are allowed
/// let mut env = HashMap::new();
/// env.insert("PATH".to_string(), "/usr/bin".to_string());
/// assert!(validate_mcp_env(&env).is_ok());
///
/// // LD_PRELOAD is rejected
/// let mut env = HashMap::new();
/// env.insert("LD_PRELOAD".to_string(), "/tmp/evil.so".to_string());
/// assert!(validate_mcp_env(&env).is_err());
/// ```
pub fn validate_mcp_env(env: &HashMap<String, String>) -> RctResult<()> {
    for key in env.keys() {
        let upper = key.to_uppercase();
        for &dangerous in DANGEROUS_ENV_VARS.iter() {
            if upper == dangerous {
                return Err(RctError::mcp_validation(format!(
                    "environment variable '{key}' is not allowed for MCP servers \
                     (library injection risk)"
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // =============================================================================
    // is_absolute_path tests - Cross-platform path detection
    // =============================================================================

    #[test]
    fn test_unix_absolute_path() {
        assert!(is_absolute_path("/bin/bash"));
        assert!(is_absolute_path("/usr/bin/python"));
        assert!(is_absolute_path("/"));
    }

    #[test]
    fn test_unix_relative_path() {
        assert!(!is_absolute_path("./script.sh"));
        assert!(!is_absolute_path("../bin/program"));
        assert!(!is_absolute_path("program"));
        assert!(!is_absolute_path("bin/program"));
    }

    #[test]
    fn test_windows_drive_letter_path() {
        assert!(is_absolute_path(r"C:\Windows\System32\cmd.exe"));
        assert!(is_absolute_path(r"D:\Program Files\app.exe"));
        assert!(is_absolute_path("C:/Windows/System32/cmd.exe"));
        assert!(is_absolute_path("c:"));
    }

    #[test]
    fn test_windows_unc_path() {
        assert!(is_absolute_path(r"\\server\share\file.exe"));
        assert!(is_absolute_path(r"\\192.168.1.1\share"));
    }

    #[test]
    fn test_windows_relative_path() {
        assert!(!is_absolute_path(r".\script.bat"));
        assert!(!is_absolute_path(r"..\bin\program.exe"));
        assert!(!is_absolute_path("program.exe"));
        assert!(!is_absolute_path(r"bin\program.exe"));
    }

    // =============================================================================
    // validate_mcp_command tests - Security validation
    // =============================================================================

    #[test]
    fn test_validate_blocks_rm() {
        let result = validate_mcp_command("rm", &[]);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string().to_lowercase();
        assert!(err.contains("blocked"));
    }

    #[test]
    fn test_validate_allows_absolute_bash() {
        #[cfg(unix)]
        {
            let result = validate_mcp_command("/bin/bash", &[]);
            assert!(result.is_ok());
        }
    }

    #[test]
    fn test_validate_rejects_path_traversal() {
        let result = validate_mcp_command("../../../bin/rm", &[]);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string().to_lowercase();
        assert!(err.contains("traversal"));
    }

    #[test]
    fn test_validate_blocks_shell_injection() {
        #[cfg(unix)]
        {
            let result = validate_mcp_command("/usr/bin/server", &["; rm -rf /".to_string()]);
            assert!(result.is_err());
        }
    }

    #[test]
    fn test_blocks_relative_unix_path() {
        let result = validate_mcp_command("./malicious", &[]);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string().to_lowercase();
        assert!(err.contains("relative"));
    }

    #[test]
    fn test_blocks_relative_windows_path() {
        let result = validate_mcp_command(r".\malicious.exe", &[]);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string().to_lowercase();
        assert!(err.contains("relative"));
    }

    #[test]
    fn test_allows_unix_absolute_path() {
        #[cfg(unix)]
        {
            let result = validate_mcp_command("/usr/bin/legitimate-mcp-server", &[]);
            assert!(result.is_ok());
        }
    }

    #[test]
    fn test_allows_windows_absolute_path() {
        #[cfg(windows)]
        {
            let result = validate_mcp_command(r"C:\Program Files\mcp-server.exe", &[]);
            assert!(result.is_ok());
        }
    }

    // =============================================================================
    // validate_mcp_env tests - Environment variable validation
    // =============================================================================

    #[test]
    fn test_validate_env_rejects_ld_preload() {
        let mut env = HashMap::new();
        env.insert("LD_PRELOAD".to_string(), "/tmp/evil.so".to_string());
        let result = validate_mcp_env(&env);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string().to_lowercase();
        assert!(err.contains("ld_preload"));
        assert!(err.contains("injection"));
    }

    #[test]
    fn test_validate_env_rejects_dyld_insert() {
        let mut env = HashMap::new();
        env.insert(
            "DYLD_INSERT_LIBRARIES".to_string(),
            "/tmp/evil.dylib".to_string(),
        );
        let result = validate_mcp_env(&env);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string().to_lowercase();
        assert!(err.contains("dyld_insert_libraries"));
        assert!(err.contains("injection"));
    }

    #[test]
    fn test_validate_env_allows_normal_vars() {
        let mut env = HashMap::new();
        env.insert("PATH".to_string(), "/usr/bin:/bin".to_string());
        env.insert("HOME".to_string(), "/home/user".to_string());
        env.insert("API_KEY".to_string(), "sk-test-123".to_string());
        let result = validate_mcp_env(&env);
        assert!(result.is_ok());
    }
}
