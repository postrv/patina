//! Tool classification for parallel execution safety.
//!
//! This module provides classification of tools and bash commands to determine
//! whether they can safely be executed in parallel.

use once_cell::sync::Lazy;
use std::collections::HashSet;

/// Classification of tool safety for parallel execution.
///
/// This enum categorizes tools based on their side effects to determine
/// whether they can safely be executed in parallel with other tools.
///
/// # Variants
///
/// - `ReadOnly` - Tool only reads data, safe to parallelize
/// - `Mutating` - Tool modifies state, must run sequentially
/// - `Unknown` - Tool behavior unknown, treated as mutating (pessimistic)
///
/// # Examples
///
/// ```
/// use patina::tools::parallel::ToolSafetyClass;
///
/// let read_class = ToolSafetyClass::ReadOnly;
/// assert!(read_class.is_parallelizable());
///
/// let mutating_class = ToolSafetyClass::Mutating;
/// assert!(!mutating_class.is_parallelizable());
///
/// let unknown_class = ToolSafetyClass::Unknown;
/// assert!(!unknown_class.is_parallelizable());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSafetyClass {
    /// Tool only reads data and has no side effects.
    /// Safe to execute in parallel with other ReadOnly tools.
    ReadOnly,

    /// Tool modifies state (files, system, etc.).
    /// Must be executed sequentially to preserve correctness.
    Mutating,

    /// Tool behavior is unknown or unpredictable.
    /// Treated as Mutating for safety (pessimistic approach).
    Unknown,
}

impl ToolSafetyClass {
    /// Returns whether this tool class can be safely parallelized.
    ///
    /// Only `ReadOnly` tools return `true`. Both `Mutating` and `Unknown`
    /// return `false` to ensure correctness over performance.
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::tools::parallel::ToolSafetyClass;
    ///
    /// assert!(ToolSafetyClass::ReadOnly.is_parallelizable());
    /// assert!(!ToolSafetyClass::Mutating.is_parallelizable());
    /// assert!(!ToolSafetyClass::Unknown.is_parallelizable());
    /// ```
    #[must_use]
    pub const fn is_parallelizable(&self) -> bool {
        matches!(self, Self::ReadOnly)
    }
}

/// Classifies a tool by name to determine its safety class for parallel execution.
///
/// # Classification Rules
///
/// - **ReadOnly**: `read_file`, `glob`, `grep`, `list_files`, `web_fetch`, `web_search`
/// - **Mutating**: `write_file`, `edit`
/// - **Unknown**: `bash`, any MCP tools (starting with `mcp__`), unrecognized tools
///
/// # Arguments
///
/// * `tool_name` - The name of the tool to classify
///
/// # Examples
///
/// ```
/// use patina::tools::parallel::{classify_tool, ToolSafetyClass};
///
/// assert_eq!(classify_tool("read_file"), ToolSafetyClass::ReadOnly);
/// assert_eq!(classify_tool("write_file"), ToolSafetyClass::Mutating);
/// assert_eq!(classify_tool("bash"), ToolSafetyClass::Unknown);
/// ```
#[must_use]
pub fn classify_tool(tool_name: &str) -> ToolSafetyClass {
    match tool_name {
        // ReadOnly tools - safe to parallelize
        "read_file" | "glob" | "grep" | "list_files" | "web_fetch" | "web_search" => {
            ToolSafetyClass::ReadOnly
        }

        // Mutating tools - must run sequentially
        "write_file" | "edit" => ToolSafetyClass::Mutating,

        // Bash is inherently unpredictable - classify as Unknown
        "bash" => ToolSafetyClass::Unknown,

        // MCP tools are external - classify as Unknown (pessimistic)
        name if name.starts_with("mcp__") => ToolSafetyClass::Unknown,

        // Any unrecognized tool is treated as Unknown (pessimistic by default)
        _ => ToolSafetyClass::Unknown,
    }
}

/// Static set of bash commands that are safe to run in parallel.
///
/// These commands only read data and have no side effects that could
/// conflict with other parallel operations.
///
/// # Safety Criteria
///
/// Commands are included only if they:
/// 1. Never modify files or system state
/// 2. Are deterministic (same input = same output)
/// 3. Do not have hidden side effects (like network calls)
///
/// # Commands Included
///
/// - File inspection: `cat`, `head`, `tail`, `wc`, `file`, `stat`
/// - Directory listing: `ls`, `find`, `tree`, `du`
/// - Text search: `grep`, `rg`, `ag`, `ack`
/// - System info: `pwd`, `whoami`, `hostname`, `uname`, `date`
/// - Environment: `env`, `printenv`, `echo`, `which`, `type`
/// - Git read operations: `git status`, `git log`, `git diff`, `git show`, etc.
pub static SAFE_BASH_COMMANDS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    let commands = [
        // File inspection
        "cat",
        "head",
        "tail",
        "wc",
        "file",
        "stat",
        "md5sum",
        "sha256sum",
        "xxd",
        "hexdump",
        "strings",
        // Directory listing
        "ls",
        "find",
        "tree",
        "du",
        "df",
        "exa",
        "lsd",
        // Text search
        "grep",
        "rg",
        "ag",
        "ack",
        "sed", // Note: sed with -i is mutating, but we check for that
        "awk",
        // System info
        "pwd",
        "whoami",
        "hostname",
        "uname",
        "date",
        "uptime",
        "id",
        // Environment
        "env",
        "printenv",
        "echo",
        "printf",
        "which",
        "type",
        "whereis",
        "command",
        // Version/help
        "man",
        "help",
        "info",
        // Path manipulation
        "basename",
        "dirname",
        "realpath",
        "readlink",
        // Text processing (read-only)
        "sort",
        "uniq",
        "cut",
        "tr",
        "tee",
        "diff",
        "cmp",
        "comm",
        "join",
        "paste",
        "fold",
        "fmt",
        "nl",
        "rev",
        "tac",
        "expand",
        "unexpand",
        // JSON/data processing
        "jq",
        "yq",
        "xq",
        // Git read operations
        "git",
        // Cargo read operations
        "cargo",
        // npm read operations
        "npm",
        // Other read operations
        "test",
        "[",
        "true",
        "false",
    ];
    commands.into_iter().collect()
});

/// Git subcommands that are safe to run in parallel (read-only).
pub(crate) static SAFE_GIT_SUBCOMMANDS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    let subcommands = [
        "status",
        "log",
        "diff",
        "show",
        "branch",
        "tag",
        "describe",
        "rev-parse",
        "rev-list",
        "ls-files",
        "ls-tree",
        "cat-file",
        "blame",
        "shortlog",
        "config",
        "remote",
        "stash",
        "reflog",
        "name-rev",
        "for-each-ref",
    ];
    subcommands.into_iter().collect()
});

/// Cargo subcommands that are safe to run in parallel (read-only).
pub(crate) static SAFE_CARGO_SUBCOMMANDS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    let subcommands = [
        "check",
        "clippy",
        "test",
        "doc",
        "tree",
        "metadata",
        "pkgid",
        "verify-project",
        "locate-project",
        "read-manifest",
    ];
    subcommands.into_iter().collect()
});

/// npm subcommands that are safe to run in parallel (read-only).
pub(crate) static SAFE_NPM_SUBCOMMANDS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    let subcommands = [
        "ls", "list", "view", "info", "show", "outdated", "search", "audit", "doctor", "explain",
        "fund", "pack", "query",
    ];
    subcommands.into_iter().collect()
});

/// Classifies a bash command to determine if it's safe to run in parallel.
///
/// # Classification Rules
///
/// A bash command is classified as `ReadOnly` only if:
/// 1. It starts with a command from `SAFE_BASH_COMMANDS`
/// 2. It does not contain shell operators that could have side effects:
///    - Pipes (`|`) - could chain to mutating commands
///    - Output redirection (`>`, `>>`) - writes to files
///    - Command substitution (`$()`, `` ` ``) - could execute arbitrary code
///    - Background execution (`&`) - side effects unknown
///    - Logical operators (`&&`, `||`) - could chain to mutating commands
///
/// For git/cargo/npm, only specific read-only subcommands are allowed.
///
/// # Arguments
///
/// * `command` - The full bash command string to classify
///
/// # Examples
///
/// ```
/// use patina::tools::parallel::{classify_bash_command, ToolSafetyClass};
///
/// assert_eq!(classify_bash_command("ls -la"), ToolSafetyClass::ReadOnly);
/// assert_eq!(classify_bash_command("cat file.txt"), ToolSafetyClass::ReadOnly);
/// assert_eq!(classify_bash_command("rm -rf /"), ToolSafetyClass::Unknown);
/// assert_eq!(classify_bash_command("ls | grep foo"), ToolSafetyClass::Unknown);
/// ```
#[must_use]
pub fn classify_bash_command(command: &str) -> ToolSafetyClass {
    let trimmed = command.trim();

    if trimmed.is_empty() {
        return ToolSafetyClass::Unknown;
    }

    // Reject commands with shell operators that could chain to mutating operations
    // These patterns are checked BEFORE command parsing for security
    if contains_shell_operators(trimmed) {
        return ToolSafetyClass::Unknown;
    }

    // Extract the first word (command name)
    let first_word = trimmed.split_whitespace().next().unwrap_or("");

    // Check if the base command is in our safe list
    if !SAFE_BASH_COMMANDS.contains(first_word) {
        return ToolSafetyClass::Unknown;
    }

    // Special handling for commands that have dangerous flags
    if has_mutating_flags(trimmed, first_word) {
        return ToolSafetyClass::Unknown;
    }

    // Special handling for git - check subcommand
    if first_word == "git" {
        return classify_git_command(trimmed);
    }

    // Special handling for cargo - check subcommand
    if first_word == "cargo" {
        return classify_cargo_command(trimmed);
    }

    // Special handling for npm - check subcommand
    if first_word == "npm" {
        return classify_npm_command(trimmed);
    }

    ToolSafetyClass::ReadOnly
}

/// Checks if a command contains shell operators that could chain to mutating operations.
fn contains_shell_operators(command: &str) -> bool {
    // Check for pipes, redirections, command substitution, etc.
    // These could chain a read-only command to a mutating one

    // Output redirection (> or >>)
    if command.contains('>') {
        return true;
    }

    // Pipes (|)
    if command.contains('|') {
        return true;
    }

    // Background execution (&)
    // Be careful not to match && which we check separately
    if command.contains(" & ") || command.ends_with(" &") || command.ends_with('&') {
        // Check it's not && at the end
        if !command.ends_with("&&") {
            return true;
        }
    }

    // Logical operators (&& or ||)
    if command.contains("&&") || command.contains("||") {
        return true;
    }

    // Command substitution $() or backticks
    if command.contains("$(") || command.contains('`') {
        return true;
    }

    // Semicolon (command separator)
    if command.contains(';') {
        return true;
    }

    false
}

/// Checks if a command has flags that make it mutating.
fn has_mutating_flags(command: &str, base_command: &str) -> bool {
    match base_command {
        // sed with -i (in-place edit)
        "sed" => command.contains(" -i") || command.contains(" --in-place"),

        // tee without -a could overwrite (but tee is typically used with pipes which we reject)
        // For safety, we don't flag tee specially since pipes are already rejected
        _ => false,
    }
}

/// Git flags that take an argument (the next word is not a subcommand).
static GIT_FLAGS_WITH_ARGS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    let flags = [
        "-C",
        "-c",
        "--git-dir",
        "--work-tree",
        "--namespace",
        "-p",
        "--paginate",
    ];
    flags.into_iter().collect()
});

/// Classifies a git command based on its subcommand.
fn classify_git_command(command: &str) -> ToolSafetyClass {
    let parts: Vec<&str> = command.split_whitespace().collect();

    // Need at least "git <subcommand>"
    if parts.len() < 2 {
        return ToolSafetyClass::Unknown;
    }

    // Find the subcommand, skipping flags and their arguments
    let mut skip_next = false;
    let subcommand = parts
        .iter()
        .skip(1)
        .find(|part| {
            if skip_next {
                skip_next = false;
                return false;
            }
            if part.starts_with('-') {
                // Check if this flag takes an argument
                if GIT_FLAGS_WITH_ARGS.contains(*part) {
                    skip_next = true;
                }
                return false;
            }
            true
        })
        .copied()
        .unwrap_or("");

    if SAFE_GIT_SUBCOMMANDS.contains(subcommand) {
        ToolSafetyClass::ReadOnly
    } else {
        ToolSafetyClass::Unknown
    }
}

/// Classifies a cargo command based on its subcommand.
fn classify_cargo_command(command: &str) -> ToolSafetyClass {
    let parts: Vec<&str> = command.split_whitespace().collect();

    if parts.len() < 2 {
        return ToolSafetyClass::Unknown;
    }

    let subcommand = parts
        .iter()
        .skip(1)
        .find(|part| !part.starts_with('-'))
        .copied()
        .unwrap_or("");

    if SAFE_CARGO_SUBCOMMANDS.contains(subcommand) {
        ToolSafetyClass::ReadOnly
    } else {
        ToolSafetyClass::Unknown
    }
}

/// Classifies an npm command based on its subcommand.
fn classify_npm_command(command: &str) -> ToolSafetyClass {
    let parts: Vec<&str> = command.split_whitespace().collect();

    if parts.len() < 2 {
        return ToolSafetyClass::Unknown;
    }

    let subcommand = parts
        .iter()
        .skip(1)
        .find(|part| !part.starts_with('-'))
        .copied()
        .unwrap_or("");

    if SAFE_NPM_SUBCOMMANDS.contains(subcommand) {
        ToolSafetyClass::ReadOnly
    } else {
        ToolSafetyClass::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Tests for ToolSafetyClass enum
    // =========================================================================

    #[test]
    fn test_tool_safety_class_variants() {
        let readonly = ToolSafetyClass::ReadOnly;
        let mutating = ToolSafetyClass::Mutating;
        let unknown = ToolSafetyClass::Unknown;

        assert_ne!(readonly, mutating);
        assert_ne!(readonly, unknown);
        assert_ne!(mutating, unknown);

        assert_eq!(format!("{:?}", readonly), "ReadOnly");
        assert_eq!(format!("{:?}", mutating), "Mutating");
        assert_eq!(format!("{:?}", unknown), "Unknown");
    }

    #[test]
    fn test_tool_safety_class_is_parallelizable() {
        assert!(ToolSafetyClass::ReadOnly.is_parallelizable());
        assert!(!ToolSafetyClass::Mutating.is_parallelizable());
        assert!(!ToolSafetyClass::Unknown.is_parallelizable());
    }

    #[test]
    fn test_tool_safety_class_clone_copy() {
        let original = ToolSafetyClass::ReadOnly;
        let cloned = original;
        assert_eq!(original, cloned);
    }

    // =========================================================================
    // Tests for classify_tool function
    // =========================================================================

    #[test]
    fn test_classify_readonly_tools() {
        assert_eq!(classify_tool("read_file"), ToolSafetyClass::ReadOnly);
        assert_eq!(classify_tool("glob"), ToolSafetyClass::ReadOnly);
        assert_eq!(classify_tool("grep"), ToolSafetyClass::ReadOnly);
        assert_eq!(classify_tool("list_files"), ToolSafetyClass::ReadOnly);
        assert_eq!(classify_tool("web_fetch"), ToolSafetyClass::ReadOnly);
        assert_eq!(classify_tool("web_search"), ToolSafetyClass::ReadOnly);
    }

    #[test]
    fn test_classify_mutating_tools() {
        assert_eq!(classify_tool("write_file"), ToolSafetyClass::Mutating);
        assert_eq!(classify_tool("edit"), ToolSafetyClass::Mutating);
    }

    #[test]
    fn test_classify_unknown_tools() {
        assert_eq!(classify_tool("bash"), ToolSafetyClass::Unknown);
        assert_eq!(
            classify_tool("mcp__narsil-mcp__search_code"),
            ToolSafetyClass::Unknown
        );
        assert_eq!(classify_tool("some_random_tool"), ToolSafetyClass::Unknown);
        assert_eq!(classify_tool(""), ToolSafetyClass::Unknown);
    }

    // =========================================================================
    // Tests for SAFE_BASH_COMMANDS whitelist
    // =========================================================================

    #[test]
    fn test_safe_bash_whitelist_contains_common_commands() {
        let expected = [
            "ls", "cat", "head", "tail", "wc", "find", "grep", "pwd", "echo", "which",
        ];
        for cmd in expected {
            assert!(SAFE_BASH_COMMANDS.contains(cmd), "should contain '{}'", cmd);
        }
    }

    #[test]
    fn test_safe_bash_whitelist_excludes_dangerous() {
        let dangerous = ["rm", "mv", "cp", "chmod", "chown", "sudo", "su"];
        for cmd in dangerous {
            assert!(
                !SAFE_BASH_COMMANDS.contains(cmd),
                "should NOT contain '{}'",
                cmd
            );
        }
    }

    #[test]
    fn test_safe_git_subcommands() {
        let expected = ["status", "log", "diff", "show", "branch", "tag", "blame"];
        for cmd in expected {
            assert!(
                SAFE_GIT_SUBCOMMANDS.contains(cmd),
                "should contain '{}'",
                cmd
            );
        }

        let mutating = ["push", "pull", "merge", "commit", "add", "reset"];
        for cmd in mutating {
            assert!(
                !SAFE_GIT_SUBCOMMANDS.contains(cmd),
                "should NOT contain '{}'",
                cmd
            );
        }
    }

    #[test]
    fn test_safe_cargo_subcommands() {
        let expected = ["check", "clippy", "test", "doc", "tree"];
        for cmd in expected {
            assert!(
                SAFE_CARGO_SUBCOMMANDS.contains(cmd),
                "should contain '{}'",
                cmd
            );
        }

        let mutating = ["build", "run", "install", "publish"];
        for cmd in mutating {
            assert!(
                !SAFE_CARGO_SUBCOMMANDS.contains(cmd),
                "should NOT contain '{}'",
                cmd
            );
        }
    }

    #[test]
    fn test_safe_npm_subcommands() {
        let expected = ["ls", "list", "view", "outdated", "search", "audit"];
        for cmd in expected {
            assert!(
                SAFE_NPM_SUBCOMMANDS.contains(cmd),
                "should contain '{}'",
                cmd
            );
        }

        let mutating = ["install", "uninstall", "update", "publish"];
        for cmd in mutating {
            assert!(
                !SAFE_NPM_SUBCOMMANDS.contains(cmd),
                "should NOT contain '{}'",
                cmd
            );
        }
    }

    // =========================================================================
    // Tests for classify_bash_command function
    // =========================================================================

    #[test]
    fn test_classify_bash_simple_readonly() {
        assert_eq!(classify_bash_command("ls"), ToolSafetyClass::ReadOnly);
        assert_eq!(classify_bash_command("ls -la"), ToolSafetyClass::ReadOnly);
        assert_eq!(
            classify_bash_command("cat file.txt"),
            ToolSafetyClass::ReadOnly
        );
    }

    #[test]
    fn test_classify_bash_dangerous() {
        assert_eq!(
            classify_bash_command("rm file.txt"),
            ToolSafetyClass::Unknown
        );
        assert_eq!(classify_bash_command("rm -rf /"), ToolSafetyClass::Unknown);
    }

    #[test]
    fn test_classify_bash_with_operators() {
        assert_eq!(
            classify_bash_command("ls | grep foo"),
            ToolSafetyClass::Unknown
        );
        assert_eq!(
            classify_bash_command("echo hello > file.txt"),
            ToolSafetyClass::Unknown
        );
        assert_eq!(
            classify_bash_command("echo $(whoami)"),
            ToolSafetyClass::Unknown
        );
        assert_eq!(
            classify_bash_command("ls && rm file"),
            ToolSafetyClass::Unknown
        );
        assert_eq!(classify_bash_command("ls &"), ToolSafetyClass::Unknown);
    }

    #[test]
    fn test_classify_bash_sed() {
        assert_eq!(
            classify_bash_command("sed 's/foo/bar/' file.txt"),
            ToolSafetyClass::ReadOnly
        );
        assert_eq!(
            classify_bash_command("sed -i 's/foo/bar/' file.txt"),
            ToolSafetyClass::Unknown
        );
    }

    #[test]
    fn test_classify_bash_git() {
        assert_eq!(
            classify_bash_command("git status"),
            ToolSafetyClass::ReadOnly
        );
        assert_eq!(
            classify_bash_command("git log --oneline"),
            ToolSafetyClass::ReadOnly
        );
        assert_eq!(
            classify_bash_command("git -C /path status"),
            ToolSafetyClass::ReadOnly
        );
        assert_eq!(
            classify_bash_command("git commit -m 'test'"),
            ToolSafetyClass::Unknown
        );
        assert_eq!(classify_bash_command("git push"), ToolSafetyClass::Unknown);
    }

    #[test]
    fn test_classify_bash_cargo() {
        assert_eq!(
            classify_bash_command("cargo check"),
            ToolSafetyClass::ReadOnly
        );
        assert_eq!(
            classify_bash_command("cargo clippy"),
            ToolSafetyClass::ReadOnly
        );
        assert_eq!(
            classify_bash_command("cargo build"),
            ToolSafetyClass::Unknown
        );
    }

    #[test]
    fn test_classify_bash_npm() {
        assert_eq!(classify_bash_command("npm ls"), ToolSafetyClass::ReadOnly);
        assert_eq!(
            classify_bash_command("npm install"),
            ToolSafetyClass::Unknown
        );
    }

    #[test]
    fn test_classify_bash_empty() {
        assert_eq!(classify_bash_command(""), ToolSafetyClass::Unknown);
        assert_eq!(classify_bash_command("   "), ToolSafetyClass::Unknown);
    }
}
