//! Platform-specific smoke tests (Phase 9.3).
//!
//! Validates terminal environment detection, clipboard strategy selection,
//! and multiplexer handling across different platforms.

use patina::terminal::{detect_terminal_environment, TerminalEnvironment};
use patina::tui::clipboard::{has_wl_copy, is_headless, is_wayland};
use std::env;
use std::sync::Mutex;

/// Guards env-var-mutating tests to prevent race conditions.
/// `env::set_var` and `env::remove_var` are not thread-safe.
static ENV_MUTEX: Mutex<()> = Mutex::new(());

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
    let _guard = ENV_MUTEX.lock().unwrap();
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
    let _guard = ENV_MUTEX.lock().unwrap();
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
    let _guard = ENV_MUTEX.lock().unwrap();
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
    let _guard = ENV_MUTEX.lock().unwrap();
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

// ============================================================================
// Cross-platform clipboard utility tests
// ============================================================================

#[test]
fn test_is_headless_returns_bool_on_all_platforms() {
    // is_headless() must work without panicking on every platform.
    // On macOS it's always false; on Linux it depends on $DISPLAY / $WAYLAND_DISPLAY.
    let result = is_headless();
    if cfg!(target_os = "macos") {
        assert!(!result, "macOS should never report as headless");
    }
    // On Linux CI (headless), this is expected to be true unless a display is set.
    // Just verify it returns without panic — the value is environment-dependent.
}

#[test]
fn test_is_wayland_returns_bool_on_all_platforms() {
    // is_wayland() should return false unless $WAYLAND_DISPLAY is set.
    let result = is_wayland();
    if cfg!(target_os = "macos") || cfg!(target_os = "windows") {
        assert!(!result, "Wayland should not be detected on macOS/Windows");
    }
    // On Linux, the result depends on whether a Wayland session is running.
}

#[test]
fn test_wl_copy_detection_returns_bool() {
    // has_wl_copy() should return false on non-Wayland systems without panic
    let _result = has_wl_copy();
}

// ============================================================================
// Environment detection edge cases
// ============================================================================

#[test]
fn test_native_when_clean_env() {
    let _guard = ENV_MUTEX.lock().unwrap();
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
