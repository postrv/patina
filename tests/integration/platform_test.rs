//! Platform-specific smoke tests (Phase 9.3).
//!
//! Validates terminal environment detection, clipboard strategy selection,
//! and multiplexer handling across different platforms.

use patina::terminal::{detect_terminal_environment, TerminalEnvironment};
use patina::tui::clipboard::{has_wl_copy, is_headless, is_wayland};
use std::env;

// ============================================================================
// Helpers
// ============================================================================

fn restore_env(key: &str, original: Option<String>) {
    if let Some(val) = original {
        env::set_var(key, val);
    } else {
        env::remove_var(key);
    }
}

// ============================================================================
// Windows-specific tests
// ============================================================================

#[cfg(target_os = "windows")]
#[test]
fn test_windows_terminal_detection() {
    // On Windows, there's no tmux/screen/SSH env vars by default
    let env = detect_terminal_environment();
    // Native is expected unless running under WSL or SSH
    assert!(
        matches!(
            env,
            TerminalEnvironment::Native | TerminalEnvironment::JetBrains
        ),
        "Windows should detect Native or JetBrains, got: {:?}",
        env
    );
}

#[cfg(target_os = "windows")]
#[test]
fn test_windows_path_handling() {
    // Verify path handling doesn't break on Windows backslashes
    let cwd = env::current_dir().expect("should get cwd");
    // On Windows, paths use backslashes but Rust's Path handles both
    assert!(
        cwd.is_absolute(),
        "Current directory should be an absolute path"
    );
}

// ============================================================================
// macOS-specific tests
// ============================================================================

#[cfg(target_os = "macos")]
#[test]
fn test_macos_clipboard_via_pbcopy() {
    // On macOS, clipboard should not be headless (pasteboard always available)
    assert!(!is_headless(), "macOS should never report as headless");
}

#[cfg(target_os = "macos")]
#[test]
fn test_macos_wayland_false() {
    // macOS does not use Wayland
    assert!(!is_wayland(), "macOS should not detect Wayland");
}

// ============================================================================
// Multiplexer simulation tests (cross-platform, using env var injection)
// ============================================================================

#[test]
fn test_tmux_env_triggers_degraded_mode() {
    let orig_tmux = env::var("TMUX").ok();

    env::set_var("TMUX", "/tmp/tmux-test/default,99999,0");

    let env_result = detect_terminal_environment();
    assert_eq!(env_result, TerminalEnvironment::Tmux);
    assert!(
        env_result.is_multiplexer(),
        "tmux should be identified as a multiplexer"
    );
    assert!(
        env_result.graphics_degraded(),
        "tmux should report degraded graphics"
    );

    restore_env("TMUX", orig_tmux);
}

#[test]
fn test_screen_env_triggers_degraded_mode() {
    let orig_tmux = env::var("TMUX").ok();
    let orig_sty = env::var("STY").ok();

    env::remove_var("TMUX");
    env::set_var("STY", "99999.pts-0.testhost");

    let env_result = detect_terminal_environment();
    assert_eq!(env_result, TerminalEnvironment::Screen);
    assert!(
        env_result.is_multiplexer(),
        "screen should be identified as a multiplexer"
    );
    assert!(
        env_result.graphics_degraded(),
        "screen should report degraded graphics"
    );

    restore_env("STY", orig_sty);
    restore_env("TMUX", orig_tmux);
}

#[test]
fn test_ssh_env_triggers_remote_mode() {
    let orig_tmux = env::var("TMUX").ok();
    let orig_sty = env::var("STY").ok();
    let orig_ssh_client = env::var("SSH_CLIENT").ok();

    env::remove_var("TMUX");
    env::remove_var("STY");
    env::set_var("SSH_CLIENT", "10.0.0.1 12345 22");

    let env_result = detect_terminal_environment();
    assert_eq!(env_result, TerminalEnvironment::SSH);
    assert!(env_result.is_remote(), "SSH should be identified as remote");
    assert!(
        !env_result.is_multiplexer(),
        "SSH alone should not be a multiplexer"
    );

    restore_env("SSH_CLIENT", orig_ssh_client);
    restore_env("STY", orig_sty);
    restore_env("TMUX", orig_tmux);
}

#[test]
fn test_ssh_tty_also_detects_ssh() {
    let orig_tmux = env::var("TMUX").ok();
    let orig_sty = env::var("STY").ok();
    let orig_ssh_client = env::var("SSH_CLIENT").ok();
    let orig_ssh_tty = env::var("SSH_TTY").ok();

    env::remove_var("TMUX");
    env::remove_var("STY");
    env::remove_var("SSH_CLIENT");
    env::set_var("SSH_TTY", "/dev/pts/0");

    let env_result = detect_terminal_environment();
    assert_eq!(env_result, TerminalEnvironment::SSH);

    restore_env("SSH_TTY", orig_ssh_tty);
    restore_env("SSH_CLIENT", orig_ssh_client);
    restore_env("STY", orig_sty);
    restore_env("TMUX", orig_tmux);
}

#[test]
fn test_wl_copy_detection_returns_bool() {
    // has_wl_copy() should return false on non-Wayland systems without panic
    let _result = has_wl_copy();
}

#[test]
fn test_native_when_clean_env() {
    let orig_tmux = env::var("TMUX").ok();
    let orig_sty = env::var("STY").ok();
    let orig_ssh_client = env::var("SSH_CLIENT").ok();
    let orig_ssh_tty = env::var("SSH_TTY").ok();
    let orig_emulator = env::var("TERMINAL_EMULATOR").ok();

    env::remove_var("TMUX");
    env::remove_var("STY");
    env::remove_var("SSH_CLIENT");
    env::remove_var("SSH_TTY");
    env::remove_var("TERMINAL_EMULATOR");

    let env_result = detect_terminal_environment();
    assert_eq!(env_result, TerminalEnvironment::Native);
    assert!(!env_result.is_multiplexer());
    assert!(!env_result.is_remote());
    assert!(!env_result.graphics_degraded());

    restore_env("TERMINAL_EMULATOR", orig_emulator);
    restore_env("SSH_TTY", orig_ssh_tty);
    restore_env("SSH_CLIENT", orig_ssh_client);
    restore_env("STY", orig_sty);
    restore_env("TMUX", orig_tmux);
}
