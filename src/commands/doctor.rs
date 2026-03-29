//! Diagnostic command for environment health checks.
//!
//! This module implements the `/doctor` command that diagnoses the user's
//! environment and reports issues with actionable fix suggestions.
//!
//! # Checks Performed
//!
//! - API key configuration
//! - Git availability and configuration
//! - GitHub CLI (`gh`) installation and authentication
//! - Terminal feature support (colors, unicode)
//! - Narsil-MCP availability
//! - Configured MCP server reachability
//! - Configuration file validity
//! - Permission configuration validity
//! - Disk space availability
//! - Rust toolchain version
//!
//! # Example
//!
//! ```
//! use patina::commands::doctor::{run_all_checks, format_report};
//!
//! let checks = run_all_checks();
//! let report = format_report(&checks);
//! println!("{report}");
//! ```

use std::path::Path;
use std::process::Command;

/// Status of a diagnostic check.
///
/// Ordered by severity: `Pass` < `Warn` < `Fail`.
///
/// # Examples
///
/// ```
/// use patina::commands::doctor::CheckStatus;
///
/// assert!(CheckStatus::Fail > CheckStatus::Warn);
/// assert!(CheckStatus::Warn > CheckStatus::Pass);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CheckStatus {
    /// Check passed successfully.
    Pass,
    /// Check passed with a non-critical warning.
    Warn,
    /// Check failed — action required.
    Fail,
}

impl CheckStatus {
    /// Returns the display indicator for this status.
    ///
    /// - Pass: `"\u{2713}"` (check mark)
    /// - Warn: `"\u{26a0}"` (warning sign)
    /// - Fail: `"\u{2717}"` (cross mark)
    #[must_use]
    pub fn indicator(&self) -> &'static str {
        match self {
            CheckStatus::Pass => "\u{2713}",
            CheckStatus::Warn => "\u{26a0}",
            CheckStatus::Fail => "\u{2717}",
        }
    }
}

/// Result of a single diagnostic check.
///
/// Contains the check name, its pass/warn/fail status, a human-readable
/// message, and an optional fix suggestion for failures.
///
/// # Examples
///
/// ```
/// use patina::commands::doctor::{DiagnosticCheck, CheckStatus};
///
/// let check = DiagnosticCheck {
///     name: "API Key".to_string(),
///     status: CheckStatus::Pass,
///     message: "ANTHROPIC_API_KEY is set".to_string(),
///     fix: None,
/// };
/// assert_eq!(check.status, CheckStatus::Pass);
/// ```
#[derive(Debug, Clone)]
pub struct DiagnosticCheck {
    /// Name of this diagnostic check.
    pub name: String,
    /// Pass, warn, or fail status.
    pub status: CheckStatus,
    /// Human-readable description of the check result.
    pub message: String,
    /// Optional fix suggestion (typically present for failures).
    pub fix: Option<String>,
}

/// Checks whether the `ANTHROPIC_API_KEY` environment variable is set.
///
/// Does not reveal the key value — only reports whether it is present
/// and non-empty.
///
/// # Examples
///
/// ```
/// use patina::commands::doctor::check_api_key;
///
/// let result = check_api_key();
/// assert_eq!(result.name, "API Key");
/// ```
#[must_use]
pub fn check_api_key() -> DiagnosticCheck {
    check_api_key_with_env(std::env::var("ANTHROPIC_API_KEY").ok())
}

/// Testable inner implementation of [`check_api_key`].
///
/// Accepts the environment variable value as an `Option<String>` so that
/// tests can inject values without modifying the process environment.
#[must_use]
pub fn check_api_key_with_env(value: Option<String>) -> DiagnosticCheck {
    match value {
        Some(ref v) if !v.trim().is_empty() => DiagnosticCheck {
            name: "API Key".to_string(),
            status: CheckStatus::Pass,
            message: "ANTHROPIC_API_KEY is set".to_string(),
            fix: None,
        },
        Some(_) => DiagnosticCheck {
            name: "API Key".to_string(),
            status: CheckStatus::Fail,
            message: "ANTHROPIC_API_KEY is set but empty".to_string(),
            fix: Some("Set your API key: export ANTHROPIC_API_KEY=\"sk-ant-...\"".to_string()),
        },
        None => DiagnosticCheck {
            name: "API Key".to_string(),
            status: CheckStatus::Fail,
            message: "ANTHROPIC_API_KEY is not set".to_string(),
            fix: Some("Set your API key: export ANTHROPIC_API_KEY=\"sk-ant-...\"".to_string()),
        },
    }
}

/// Checks whether `git` is available and has `user.name` / `user.email` configured.
///
/// # Examples
///
/// ```
/// use patina::commands::doctor::check_git;
///
/// let result = check_git();
/// assert_eq!(result.name, "Git");
/// ```
#[must_use]
pub fn check_git() -> DiagnosticCheck {
    let git_available = Command::new("git")
        .arg("--version")
        .output()
        .ok()
        .is_some_and(|o| o.status.success());

    if !git_available {
        return DiagnosticCheck {
            name: "Git".to_string(),
            status: CheckStatus::Fail,
            message: "git is not installed or not in PATH".to_string(),
            fix: Some("Install git: https://git-scm.com/downloads".to_string()),
        };
    }

    let user_name = Command::new("git")
        .args(["config", "user.name"])
        .output()
        .ok()
        .is_some_and(|o| o.status.success() && !o.stdout.is_empty());

    let user_email = Command::new("git")
        .args(["config", "user.email"])
        .output()
        .ok()
        .is_some_and(|o| o.status.success() && !o.stdout.is_empty());

    if user_name && user_email {
        DiagnosticCheck {
            name: "Git".to_string(),
            status: CheckStatus::Pass,
            message: "git is installed and configured".to_string(),
            fix: None,
        }
    } else {
        DiagnosticCheck {
            name: "Git".to_string(),
            status: CheckStatus::Warn,
            message: "git is installed but user.name or user.email is not configured".to_string(),
            fix: Some(
                "Run: git config --global user.name \"Your Name\" && git config --global user.email \"you@example.com\""
                    .to_string(),
            ),
        }
    }
}

/// Checks whether the GitHub CLI (`gh`) is installed and authenticated.
///
/// # Examples
///
/// ```
/// use patina::commands::doctor::check_gh_cli;
///
/// let result = check_gh_cli();
/// assert_eq!(result.name, "GitHub CLI");
/// ```
#[must_use]
pub fn check_gh_cli() -> DiagnosticCheck {
    let gh_available = Command::new("gh")
        .arg("--version")
        .output()
        .ok()
        .is_some_and(|o| o.status.success());

    if !gh_available {
        return DiagnosticCheck {
            name: "GitHub CLI".to_string(),
            status: CheckStatus::Warn,
            message: "gh CLI is not installed".to_string(),
            fix: Some("Install gh: https://cli.github.com/".to_string()),
        };
    }

    let auth_ok = Command::new("gh")
        .args(["auth", "status"])
        .output()
        .ok()
        .is_some_and(|o| o.status.success());

    if auth_ok {
        DiagnosticCheck {
            name: "GitHub CLI".to_string(),
            status: CheckStatus::Pass,
            message: "gh CLI is installed and authenticated".to_string(),
            fix: None,
        }
    } else {
        DiagnosticCheck {
            name: "GitHub CLI".to_string(),
            status: CheckStatus::Warn,
            message: "gh CLI is installed but not authenticated".to_string(),
            fix: Some("Run: gh auth login".to_string()),
        }
    }
}

/// Checks whether the terminal supports colors and unicode.
///
/// Inspects the `TERM` and `COLORTERM` environment variables and
/// verifies that unicode rendering is likely supported.
///
/// # Examples
///
/// ```
/// use patina::commands::doctor::check_terminal;
///
/// let result = check_terminal();
/// assert_eq!(result.name, "Terminal");
/// ```
#[must_use]
pub fn check_terminal() -> DiagnosticCheck {
    check_terminal_with_env(std::env::var("TERM").ok(), std::env::var("COLORTERM").ok())
}

/// Testable inner implementation of [`check_terminal`].
///
/// Accepts environment variable values so tests can exercise all branches
/// without modifying the process environment.
#[must_use]
pub fn check_terminal_with_env(term: Option<String>, colorterm: Option<String>) -> DiagnosticCheck {
    let has_color = colorterm.is_some()
        || term
            .as_deref()
            .is_some_and(|t| t.contains("256color") || t.contains("truecolor"));

    let has_term = term.as_deref().is_some_and(|t| t != "dumb");

    if has_color && has_term {
        DiagnosticCheck {
            name: "Terminal".to_string(),
            status: CheckStatus::Pass,
            message: "Terminal supports colors and unicode".to_string(),
            fix: None,
        }
    } else if has_term {
        DiagnosticCheck {
            name: "Terminal".to_string(),
            status: CheckStatus::Warn,
            message: "Terminal may not support full color output".to_string(),
            fix: Some("Set COLORTERM=truecolor or use a modern terminal emulator".to_string()),
        }
    } else {
        DiagnosticCheck {
            name: "Terminal".to_string(),
            status: CheckStatus::Fail,
            message: "Terminal is set to 'dumb' or TERM is not set".to_string(),
            fix: Some("Use a terminal that sets the TERM environment variable".to_string()),
        }
    }
}

/// Checks whether narsil-mcp is available.
///
/// Looks for `narsil-mcp` in `PATH` by attempting to run it.
///
/// # Examples
///
/// ```
/// use patina::commands::doctor::check_narsil;
///
/// let result = check_narsil();
/// assert_eq!(result.name, "Narsil MCP");
/// ```
#[must_use]
pub fn check_narsil() -> DiagnosticCheck {
    let available = Command::new("narsil-mcp")
        .arg("--version")
        .output()
        .ok()
        .is_some_and(|o| o.status.success());

    if available {
        DiagnosticCheck {
            name: "Narsil MCP".to_string(),
            status: CheckStatus::Pass,
            message: "narsil-mcp is available".to_string(),
            fix: None,
        }
    } else {
        DiagnosticCheck {
            name: "Narsil MCP".to_string(),
            status: CheckStatus::Warn,
            message: "narsil-mcp is not available (code intelligence disabled)".to_string(),
            fix: Some("Install narsil-mcp for code intelligence and security scanning".to_string()),
        }
    }
}

/// Checks whether configured MCP servers appear reachable.
///
/// Loads the MCP configuration from `working_dir` and verifies that
/// stdio-based servers have their commands available in `PATH`, and
/// SSE-based servers have valid URLs.
///
/// # Examples
///
/// ```
/// use patina::commands::doctor::check_mcp_servers;
/// use std::path::Path;
///
/// let result = check_mcp_servers(Path::new("."));
/// assert_eq!(result.name, "MCP Servers");
/// ```
#[must_use]
pub fn check_mcp_servers(working_dir: &Path) -> DiagnosticCheck {
    let config_result = crate::mcp::config::load_mcp_config(working_dir);

    let servers = match config_result {
        Ok(result) => result.servers,
        Err(e) => {
            return DiagnosticCheck {
                name: "MCP Servers".to_string(),
                status: CheckStatus::Fail,
                message: format!("Failed to load MCP config: {e}"),
                fix: Some("Check .mcp.json and ~/.claude.json for syntax errors".to_string()),
            };
        }
    };

    if servers.is_empty() {
        return DiagnosticCheck {
            name: "MCP Servers".to_string(),
            status: CheckStatus::Pass,
            message: "No MCP servers configured".to_string(),
            fix: None,
        };
    }

    let mut unreachable = Vec::new();

    for (name, entry) in &servers {
        if entry.is_sse() {
            // SSE servers: just verify URL is non-empty
            if entry.url.as_deref().unwrap_or("").is_empty() {
                unreachable.push(name.clone());
            }
        } else {
            // Stdio servers: verify command exists by attempting to locate it
            let cmd_ok = Command::new("which")
                .arg(&entry.command)
                .output()
                .ok()
                .is_some_and(|o| o.status.success());
            if !cmd_ok {
                unreachable.push(name.clone());
            }
        }
    }

    if unreachable.is_empty() {
        DiagnosticCheck {
            name: "MCP Servers".to_string(),
            status: CheckStatus::Pass,
            message: format!(
                "All {} configured MCP server(s) appear reachable",
                servers.len()
            ),
            fix: None,
        }
    } else {
        DiagnosticCheck {
            name: "MCP Servers".to_string(),
            status: CheckStatus::Warn,
            message: format!(
                "{} of {} MCP server(s) unreachable: {}",
                unreachable.len(),
                servers.len(),
                unreachable.join(", ")
            ),
            fix: Some("Verify server commands are installed and in PATH".to_string()),
        }
    }
}

/// Checks whether the main configuration file is valid.
///
/// Verifies that `~/.config/patina/config.toml` (or platform equivalent)
/// exists and is parseable. A missing file is acceptable (defaults are used).
///
/// # Examples
///
/// ```
/// use patina::commands::doctor::check_config;
///
/// let result = check_config();
/// assert_eq!(result.name, "Config");
/// ```
#[must_use]
pub fn check_config() -> DiagnosticCheck {
    let config_dir = directories::ProjectDirs::from("com", "patina", "patina");

    let Some(dirs) = config_dir else {
        return DiagnosticCheck {
            name: "Config".to_string(),
            status: CheckStatus::Warn,
            message: "Could not determine config directory".to_string(),
            fix: None,
        };
    };

    let config_path = dirs.config_dir().join("config.toml");

    if !config_path.exists() {
        return DiagnosticCheck {
            name: "Config".to_string(),
            status: CheckStatus::Pass,
            message: "No config file found (using defaults)".to_string(),
            fix: None,
        };
    }

    match std::fs::read_to_string(&config_path) {
        Ok(content) => {
            // Verify it is valid TOML
            if content.parse::<toml::Table>().is_ok() {
                DiagnosticCheck {
                    name: "Config".to_string(),
                    status: CheckStatus::Pass,
                    message: format!("Config file is valid: {}", config_path.display()),
                    fix: None,
                }
            } else {
                DiagnosticCheck {
                    name: "Config".to_string(),
                    status: CheckStatus::Fail,
                    message: format!("Config file has invalid TOML: {}", config_path.display()),
                    fix: Some(format!(
                        "Fix or delete the config file: {}",
                        config_path.display()
                    )),
                }
            }
        }
        Err(e) => DiagnosticCheck {
            name: "Config".to_string(),
            status: CheckStatus::Fail,
            message: format!("Cannot read config file: {e}"),
            fix: Some(format!("Check file permissions: {}", config_path.display())),
        },
    }
}

/// Checks whether the permission configuration file is valid.
///
/// Verifies that `permissions.toml` exists and is parseable. A missing
/// file is acceptable (no custom permissions applied).
///
/// # Examples
///
/// ```
/// use patina::commands::doctor::check_permissions;
///
/// let result = check_permissions();
/// assert_eq!(result.name, "Permissions");
/// ```
#[must_use]
pub fn check_permissions() -> DiagnosticCheck {
    let config_dir = directories::ProjectDirs::from("com", "patina", "patina");

    let Some(dirs) = config_dir else {
        return DiagnosticCheck {
            name: "Permissions".to_string(),
            status: CheckStatus::Warn,
            message: "Could not determine config directory".to_string(),
            fix: None,
        };
    };

    let perm_path = dirs.config_dir().join("permissions.toml");

    if !perm_path.exists() {
        return DiagnosticCheck {
            name: "Permissions".to_string(),
            status: CheckStatus::Pass,
            message: "No permissions file (using defaults)".to_string(),
            fix: None,
        };
    }

    match std::fs::read_to_string(&perm_path) {
        Ok(content) => {
            if toml::from_str::<crate::permissions::PermissionConfig>(&content).is_ok() {
                DiagnosticCheck {
                    name: "Permissions".to_string(),
                    status: CheckStatus::Pass,
                    message: format!("Permissions file is valid: {}", perm_path.display()),
                    fix: None,
                }
            } else {
                DiagnosticCheck {
                    name: "Permissions".to_string(),
                    status: CheckStatus::Fail,
                    message: format!(
                        "Permissions file has invalid format: {}",
                        perm_path.display()
                    ),
                    fix: Some(format!(
                        "Fix or delete the permissions file: {}",
                        perm_path.display()
                    )),
                }
            }
        }
        Err(e) => DiagnosticCheck {
            name: "Permissions".to_string(),
            status: CheckStatus::Fail,
            message: format!("Cannot read permissions file: {e}"),
            fix: Some(format!("Check file permissions: {}", perm_path.display())),
        },
    }
}

/// Checks whether sufficient disk space is available for session storage.
///
/// Sessions are stored under the user's data directory. This check warns
/// if less than 100 MB of free space remains.
///
/// # Examples
///
/// ```
/// use patina::commands::doctor::check_disk_space;
///
/// let result = check_disk_space();
/// assert_eq!(result.name, "Disk Space");
/// ```
#[must_use]
pub fn check_disk_space() -> DiagnosticCheck {
    check_disk_space_with_bytes(get_available_disk_bytes())
}

/// Returns available bytes on the filesystem containing the user's home directory.
///
/// Uses `df` on Unix-like systems and `PowerShell` on Windows.
/// Returns `None` if the information cannot be determined.
fn get_available_disk_bytes() -> Option<u64> {
    let home = directories::BaseDirs::new()?;
    let path = home.home_dir();

    #[cfg(unix)]
    {
        // Use `df -k <path>` and parse available column
        let output = Command::new("df")
            .args(["-k", &path.to_string_lossy()])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        // Second line, 4th column is available KB
        let line = text.lines().nth(1)?;
        let avail_kb: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
        Some(avail_kb * 1024)
    }

    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// Testable inner implementation of [`check_disk_space`].
///
/// Accepts optional available bytes so tests can simulate low/high disk space.
#[must_use]
pub fn check_disk_space_with_bytes(available: Option<u64>) -> DiagnosticCheck {
    /// Minimum recommended free space for sessions (100 MB).
    const MIN_SPACE_BYTES: u64 = 100 * 1024 * 1024;

    match available {
        Some(bytes) if bytes >= MIN_SPACE_BYTES => {
            let mb = bytes / (1024 * 1024);
            DiagnosticCheck {
                name: "Disk Space".to_string(),
                status: CheckStatus::Pass,
                message: format!("{mb} MB available for session storage"),
                fix: None,
            }
        }
        Some(bytes) => {
            let mb = bytes / (1024 * 1024);
            DiagnosticCheck {
                name: "Disk Space".to_string(),
                status: CheckStatus::Warn,
                message: format!("Only {mb} MB available for session storage"),
                fix: Some("Free up disk space to avoid session write failures".to_string()),
            }
        }
        None => DiagnosticCheck {
            name: "Disk Space".to_string(),
            status: CheckStatus::Warn,
            message: "Could not determine available disk space".to_string(),
            fix: None,
        },
    }
}

/// Checks whether the Rust toolchain is installed and reports its version.
///
/// This is informational — Rust is only needed for building from source.
///
/// # Examples
///
/// ```
/// use patina::commands::doctor::check_rust_version;
///
/// let result = check_rust_version();
/// assert_eq!(result.name, "Rust Toolchain");
/// ```
#[must_use]
pub fn check_rust_version() -> DiagnosticCheck {
    let output = Command::new("rustc").arg("--version").output().ok();

    match output {
        Some(o) if o.status.success() => {
            let version = String::from_utf8_lossy(&o.stdout).trim().to_string();
            DiagnosticCheck {
                name: "Rust Toolchain".to_string(),
                status: CheckStatus::Pass,
                message: version,
                fix: None,
            }
        }
        _ => DiagnosticCheck {
            name: "Rust Toolchain".to_string(),
            status: CheckStatus::Warn,
            message: "Rust toolchain not found (only needed for building from source)".to_string(),
            fix: Some("Install Rust: https://rustup.rs/".to_string()),
        },
    }
}

/// Runs all diagnostic checks and returns the results.
///
/// Checks are run in a deterministic order. The working directory
/// used for MCP server checks defaults to the current directory.
///
/// # Examples
///
/// ```
/// use patina::commands::doctor::run_all_checks;
///
/// let checks = run_all_checks();
/// assert!(!checks.is_empty());
/// ```
#[must_use]
pub fn run_all_checks() -> Vec<DiagnosticCheck> {
    let working_dir = std::env::current_dir().unwrap_or_default();
    run_all_checks_in(working_dir.as_path())
}

/// Runs all diagnostic checks using the specified working directory.
///
/// # Arguments
///
/// * `working_dir` - Directory to use for MCP and project-relative checks.
///
/// # Examples
///
/// ```
/// use patina::commands::doctor::run_all_checks_in;
/// use std::path::Path;
///
/// let checks = run_all_checks_in(Path::new("."));
/// assert!(checks.len() >= 10);
/// ```
#[must_use]
pub fn run_all_checks_in(working_dir: &Path) -> Vec<DiagnosticCheck> {
    vec![
        check_api_key(),
        check_git(),
        check_gh_cli(),
        check_terminal(),
        check_narsil(),
        check_mcp_servers(working_dir),
        check_config(),
        check_permissions(),
        check_disk_space(),
        check_rust_version(),
    ]
}

/// Formats a diagnostic report from the given checks.
///
/// Produces a human-readable report with status indicators:
/// - `\u{2713}` for pass
/// - `\u{26a0}` for warn
/// - `\u{2717}` for fail
///
/// Failures include their fix suggestion indented below the check line.
/// The report ends with a summary line: "X passed, Y warnings, Z failures".
///
/// # Arguments
///
/// * `checks` - Slice of diagnostic check results to format.
///
/// # Examples
///
/// ```
/// use patina::commands::doctor::{DiagnosticCheck, CheckStatus, format_report};
///
/// let checks = vec![
///     DiagnosticCheck {
///         name: "Test".to_string(),
///         status: CheckStatus::Pass,
///         message: "all good".to_string(),
///         fix: None,
///     },
/// ];
/// let report = format_report(&checks);
/// assert!(report.contains("\u{2713}"));
/// assert!(report.contains("1 passed"));
/// ```
#[must_use]
pub fn format_report(checks: &[DiagnosticCheck]) -> String {
    let mut lines = Vec::new();
    lines.push("Patina Doctor\n".to_string());

    let mut passed = 0u32;
    let mut warnings = 0u32;
    let mut failures = 0u32;

    for check in checks {
        match check.status {
            CheckStatus::Pass => passed += 1,
            CheckStatus::Warn => warnings += 1,
            CheckStatus::Fail => failures += 1,
        }

        lines.push(format!(
            "  {} {}: {}",
            check.status.indicator(),
            check.name,
            check.message,
        ));

        if let Some(ref fix) = check.fix {
            if check.status == CheckStatus::Fail || check.status == CheckStatus::Warn {
                lines.push(format!("    Fix: {fix}"));
            }
        }
    }

    lines.push(String::new());
    lines.push(format!(
        "{passed} passed, {warnings} warnings, {failures} failures"
    ));

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // CheckStatus ordering
    // =========================================================================

    #[test]
    fn test_check_status_ordering_fail_is_greatest() {
        assert!(CheckStatus::Fail > CheckStatus::Warn);
        assert!(CheckStatus::Fail > CheckStatus::Pass);
    }

    #[test]
    fn test_check_status_ordering_warn_greater_than_pass() {
        assert!(CheckStatus::Warn > CheckStatus::Pass);
    }

    #[test]
    fn test_check_status_ordering_pass_is_least() {
        assert!(CheckStatus::Pass < CheckStatus::Warn);
        assert!(CheckStatus::Pass < CheckStatus::Fail);
    }

    #[test]
    fn test_check_status_equality() {
        assert_eq!(CheckStatus::Pass, CheckStatus::Pass);
        assert_eq!(CheckStatus::Warn, CheckStatus::Warn);
        assert_eq!(CheckStatus::Fail, CheckStatus::Fail);
    }

    // =========================================================================
    // CheckStatus::indicator
    // =========================================================================

    #[test]
    fn test_indicator_pass() {
        assert_eq!(CheckStatus::Pass.indicator(), "\u{2713}");
    }

    #[test]
    fn test_indicator_warn() {
        assert_eq!(CheckStatus::Warn.indicator(), "\u{26a0}");
    }

    #[test]
    fn test_indicator_fail() {
        assert_eq!(CheckStatus::Fail.indicator(), "\u{2717}");
    }

    // =========================================================================
    // check_api_key
    // =========================================================================

    #[test]
    fn test_check_api_key_present() {
        let check = check_api_key_with_env(Some("sk-ant-test-key".to_string()));
        assert_eq!(check.status, CheckStatus::Pass);
        assert_eq!(check.name, "API Key");
        assert!(check.message.contains("is set"));
        assert!(check.fix.is_none());
    }

    #[test]
    fn test_check_api_key_missing() {
        let check = check_api_key_with_env(None);
        assert_eq!(check.status, CheckStatus::Fail);
        assert!(check.message.contains("not set"));
        assert!(check.fix.is_some());
    }

    #[test]
    fn test_check_api_key_empty_string() {
        let check = check_api_key_with_env(Some(String::new()));
        assert_eq!(check.status, CheckStatus::Fail);
        assert!(check.message.contains("empty"));
        assert!(check.fix.is_some());
    }

    #[test]
    fn test_check_api_key_whitespace_only() {
        let check = check_api_key_with_env(Some("   ".to_string()));
        assert_eq!(check.status, CheckStatus::Fail);
        assert!(check.message.contains("empty"));
    }

    #[test]
    fn test_check_api_key_does_not_reveal_value() {
        let check = check_api_key_with_env(Some("sk-ant-super-secret".to_string()));
        assert!(!check.message.contains("sk-ant-super-secret"));
        assert!(
            check.fix.is_none()
                || !check
                    .fix
                    .as_deref()
                    .unwrap()
                    .contains("sk-ant-super-secret")
        );
    }

    // =========================================================================
    // check_terminal
    // =========================================================================

    #[test]
    fn test_check_terminal_full_color() {
        let check = check_terminal_with_env(
            Some("xterm-256color".to_string()),
            Some("truecolor".to_string()),
        );
        assert_eq!(check.status, CheckStatus::Pass);
        assert_eq!(check.name, "Terminal");
    }

    #[test]
    fn test_check_terminal_256color_no_colorterm() {
        let check = check_terminal_with_env(Some("xterm-256color".to_string()), None);
        assert_eq!(check.status, CheckStatus::Pass);
    }

    #[test]
    fn test_check_terminal_basic_term_no_color() {
        let check = check_terminal_with_env(Some("xterm".to_string()), None);
        assert_eq!(check.status, CheckStatus::Warn);
        assert!(check.fix.is_some());
    }

    #[test]
    fn test_check_terminal_dumb() {
        let check = check_terminal_with_env(Some("dumb".to_string()), None);
        assert_eq!(check.status, CheckStatus::Fail);
    }

    #[test]
    fn test_check_terminal_no_term() {
        let check = check_terminal_with_env(None, None);
        assert_eq!(check.status, CheckStatus::Fail);
        assert!(check.message.contains("not set"));
    }

    #[test]
    fn test_check_terminal_colorterm_overrides_basic_term() {
        let check =
            check_terminal_with_env(Some("xterm".to_string()), Some("truecolor".to_string()));
        assert_eq!(check.status, CheckStatus::Pass);
    }

    // =========================================================================
    // check_disk_space
    // =========================================================================

    #[test]
    fn test_check_disk_space_plenty() {
        let check = check_disk_space_with_bytes(Some(500 * 1024 * 1024));
        assert_eq!(check.status, CheckStatus::Pass);
        assert_eq!(check.name, "Disk Space");
        assert!(check.message.contains("500 MB"));
    }

    #[test]
    fn test_check_disk_space_low() {
        let check = check_disk_space_with_bytes(Some(50 * 1024 * 1024));
        assert_eq!(check.status, CheckStatus::Warn);
        assert!(check.message.contains("50 MB"));
        assert!(check.fix.is_some());
    }

    #[test]
    fn test_check_disk_space_exactly_threshold() {
        let check = check_disk_space_with_bytes(Some(100 * 1024 * 1024));
        assert_eq!(check.status, CheckStatus::Pass);
    }

    #[test]
    fn test_check_disk_space_just_below_threshold() {
        let check = check_disk_space_with_bytes(Some(100 * 1024 * 1024 - 1));
        assert_eq!(check.status, CheckStatus::Warn);
    }

    #[test]
    fn test_check_disk_space_unknown() {
        let check = check_disk_space_with_bytes(None);
        assert_eq!(check.status, CheckStatus::Warn);
        assert!(check.message.contains("Could not determine"));
    }

    // =========================================================================
    // format_report
    // =========================================================================

    #[test]
    fn test_format_report_all_pass() {
        let checks = vec![
            DiagnosticCheck {
                name: "Alpha".to_string(),
                status: CheckStatus::Pass,
                message: "all good".to_string(),
                fix: None,
            },
            DiagnosticCheck {
                name: "Beta".to_string(),
                status: CheckStatus::Pass,
                message: "also good".to_string(),
                fix: None,
            },
        ];

        let report = format_report(&checks);
        assert!(report.contains("2 passed, 0 warnings, 0 failures"));
        assert!(report.contains("\u{2713} Alpha"));
        assert!(report.contains("\u{2713} Beta"));
    }

    #[test]
    fn test_format_report_with_failures() {
        let checks = vec![
            DiagnosticCheck {
                name: "Good".to_string(),
                status: CheckStatus::Pass,
                message: "fine".to_string(),
                fix: None,
            },
            DiagnosticCheck {
                name: "Bad".to_string(),
                status: CheckStatus::Fail,
                message: "broken".to_string(),
                fix: Some("fix it".to_string()),
            },
        ];

        let report = format_report(&checks);
        assert!(report.contains("1 passed, 0 warnings, 1 failures"));
        assert!(report.contains("\u{2717} Bad: broken"));
        assert!(report.contains("Fix: fix it"));
    }

    #[test]
    fn test_format_report_mixed_statuses() {
        let checks = vec![
            DiagnosticCheck {
                name: "A".to_string(),
                status: CheckStatus::Pass,
                message: "ok".to_string(),
                fix: None,
            },
            DiagnosticCheck {
                name: "B".to_string(),
                status: CheckStatus::Warn,
                message: "meh".to_string(),
                fix: Some("try this".to_string()),
            },
            DiagnosticCheck {
                name: "C".to_string(),
                status: CheckStatus::Fail,
                message: "bad".to_string(),
                fix: Some("do that".to_string()),
            },
        ];

        let report = format_report(&checks);
        assert!(report.contains("1 passed, 1 warnings, 1 failures"));
        assert!(report.contains("\u{2713} A: ok"));
        assert!(report.contains("\u{26a0} B: meh"));
        assert!(report.contains("Fix: try this"));
        assert!(report.contains("\u{2717} C: bad"));
        assert!(report.contains("Fix: do that"));
    }

    #[test]
    fn test_format_report_empty() {
        let report = format_report(&[]);
        assert!(report.contains("0 passed, 0 warnings, 0 failures"));
    }

    #[test]
    fn test_format_report_fix_shown_for_failures_only() {
        let checks = vec![
            DiagnosticCheck {
                name: "Pass".to_string(),
                status: CheckStatus::Pass,
                message: "ok".to_string(),
                fix: Some("should not appear".to_string()),
            },
            DiagnosticCheck {
                name: "Fail".to_string(),
                status: CheckStatus::Fail,
                message: "broken".to_string(),
                fix: Some("should appear".to_string()),
            },
        ];

        let report = format_report(&checks);
        assert!(!report.contains("should not appear"));
        assert!(report.contains("should appear"));
    }

    #[test]
    fn test_format_report_fix_shown_for_warnings() {
        let checks = vec![DiagnosticCheck {
            name: "Warn".to_string(),
            status: CheckStatus::Warn,
            message: "warning".to_string(),
            fix: Some("warning fix".to_string()),
        }];

        let report = format_report(&checks);
        assert!(report.contains("Fix: warning fix"));
    }

    #[test]
    fn test_format_report_header() {
        let report = format_report(&[]);
        assert!(report.starts_with("Patina Doctor"));
    }

    #[test]
    fn test_format_report_failure_without_fix() {
        let checks = vec![DiagnosticCheck {
            name: "Broken".to_string(),
            status: CheckStatus::Fail,
            message: "something went wrong".to_string(),
            fix: None,
        }];

        let report = format_report(&checks);
        assert!(report.contains("\u{2717} Broken: something went wrong"));
        assert!(!report.contains("    Fix:"));
    }

    // =========================================================================
    // run_all_checks
    // =========================================================================

    #[test]
    fn test_run_all_checks_returns_all_ten() {
        let checks = run_all_checks_in(Path::new("."));
        assert_eq!(checks.len(), 10);
    }

    #[test]
    fn test_run_all_checks_names_are_unique() {
        let checks = run_all_checks_in(Path::new("."));
        let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
        let mut deduped = names.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(names.len(), deduped.len(), "Duplicate check names found");
    }

    #[test]
    fn test_run_all_checks_expected_names() {
        let checks = run_all_checks_in(Path::new("."));
        let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"API Key"));
        assert!(names.contains(&"Git"));
        assert!(names.contains(&"GitHub CLI"));
        assert!(names.contains(&"Terminal"));
        assert!(names.contains(&"Narsil MCP"));
        assert!(names.contains(&"MCP Servers"));
        assert!(names.contains(&"Config"));
        assert!(names.contains(&"Permissions"));
        assert!(names.contains(&"Disk Space"));
        assert!(names.contains(&"Rust Toolchain"));
    }

    // =========================================================================
    // DiagnosticCheck struct
    // =========================================================================

    #[test]
    fn test_diagnostic_check_debug() {
        let check = DiagnosticCheck {
            name: "Test".to_string(),
            status: CheckStatus::Pass,
            message: "ok".to_string(),
            fix: None,
        };
        let debug = format!("{check:?}");
        assert!(debug.contains("Test"));
        assert!(debug.contains("Pass"));
    }

    #[test]
    fn test_diagnostic_check_clone() {
        let check = DiagnosticCheck {
            name: "Test".to_string(),
            status: CheckStatus::Fail,
            message: "bad".to_string(),
            fix: Some("fix".to_string()),
        };
        let cloned = check.clone();
        assert_eq!(cloned.name, check.name);
        assert_eq!(cloned.status, check.status);
        assert_eq!(cloned.message, check.message);
        assert_eq!(cloned.fix, check.fix);
    }
}
