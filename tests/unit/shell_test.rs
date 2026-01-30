//! Unit tests for cross-platform shell abstraction.
//!
//! These tests verify the `ShellConfig` type correctly detects the platform
//! and provides appropriate shell configuration.

use rct::shell::{execute_shell_command, ShellConfig, ShellOutput};

// ============================================================================
// ShellConfig Tests
// ============================================================================

#[test]
#[cfg(unix)]
fn test_shell_config_returns_sh_on_unix() {
    let config = ShellConfig::default();

    assert_eq!(config.command, "sh");
    assert_eq!(config.args, vec!["-c"]);
    assert_eq!(config.exit_success, 0);
}

#[test]
#[cfg(windows)]
fn test_shell_config_returns_cmd_on_windows() {
    let config = ShellConfig::default();

    assert_eq!(config.command, "cmd.exe");
    assert_eq!(config.args, vec!["/C"]);
    assert_eq!(config.exit_success, 0);
}

#[test]
fn test_shell_config_command_builds_correctly() {
    let config = ShellConfig::default();

    // Verify the config can be used to build a command
    let mut cmd = std::process::Command::new(&config.command);
    for arg in &config.args {
        cmd.arg(arg);
    }
    cmd.arg("echo test");

    // The command should be buildable (not testing execution here)
    assert!(!config.command.is_empty());
    assert!(!config.args.is_empty());
}

// ============================================================================
// ShellOutput Tests
// ============================================================================

#[test]
fn test_shell_output_success() {
    let output = ShellOutput {
        exit_code: 0,
        stdout: "hello".to_string(),
        stderr: String::new(),
    };

    assert!(output.success());
    assert_eq!(output.stdout, "hello");
}

#[test]
fn test_shell_output_failure() {
    let output = ShellOutput {
        exit_code: 1,
        stdout: String::new(),
        stderr: "error".to_string(),
    };

    assert!(!output.success());
    assert_eq!(output.stderr, "error");
}

#[test]
fn test_shell_output_custom_exit_code() {
    let output = ShellOutput {
        exit_code: 42,
        stdout: String::new(),
        stderr: String::new(),
    };

    assert!(!output.success());
    assert_eq!(output.exit_code, 42);
}

// ============================================================================
// Shell Execution Tests
// ============================================================================

#[tokio::test]
async fn test_execute_shell_command_echo() {
    let result = execute_shell_command("echo hello", None).await;
    let output = result.expect("echo should succeed");

    assert!(output.success());
    assert!(output.stdout.contains("hello"));
}

#[tokio::test]
async fn test_execute_shell_command_exit_code() {
    let result = execute_shell_command("exit 42", None).await;
    let output = result.expect("exit should not error");

    assert_eq!(output.exit_code, 42);
    assert!(!output.success());
}

#[tokio::test]
async fn test_execute_shell_command_stderr() {
    // Use a command that writes to stderr
    #[cfg(unix)]
    let cmd = "echo error >&2";
    #[cfg(windows)]
    let cmd = "echo error 1>&2";

    let result = execute_shell_command(cmd, None).await;
    let output = result.expect("stderr write should succeed");

    assert!(output.stderr.contains("error"));
}

#[tokio::test]
async fn test_execute_shell_command_with_stdin() {
    #[cfg(unix)]
    let cmd = "cat";
    #[cfg(windows)]
    let cmd = "more";

    let result = execute_shell_command(cmd, Some("hello from stdin")).await;
    let output = result.expect("stdin piping should work");

    assert!(output.stdout.contains("hello from stdin"));
}
