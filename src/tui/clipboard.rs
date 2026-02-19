//! Clipboard utilities for terminal applications.
//!
//! Provides multiple clipboard backends:
//! 1. Native clipboard via arboard (works on desktop)
//! 2. OSC 52 escape sequences (works in iTerm2, kitty, tmux, SSH, etc.)
//!
//! OSC 52 is particularly useful because:
//! - Works over SSH connections
//! - Works inside tmux/screen
//! - Supported by iTerm2, kitty, WezTerm, alacritty, and many others
//!
//! # Example
//!
//! ```ignore
//! use patina::tui::clipboard::copy_to_clipboard;
//!
//! copy_to_clipboard("Hello, world!")?;
//! ```

use anyhow::Result;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::io::{self, Write};
use std::process::Command;

/// Returns `true` if no display server is available (headless system, SSH, etc.).
///
/// A headless environment has neither `$DISPLAY` (X11) nor `$WAYLAND_DISPLAY`
/// (Wayland). In this case, native clipboard libraries like arboard will fail
/// because they need a display connection.
///
/// On macOS, the display server is always available (AppKit, no env var needed),
/// so this function returns `false` on macOS regardless of environment variables.
///
/// # Examples
///
/// ```rust
/// use patina::tui::clipboard::is_headless;
///
/// if is_headless() {
///     println!("No display server — clipboard will use OSC 52");
/// }
/// ```
#[must_use]
pub fn is_headless() -> bool {
    // macOS always has a display server (pasteboard service), even over SSH
    if cfg!(target_os = "macos") {
        return false;
    }

    let has_x11 = std::env::var("DISPLAY").is_ok();
    let has_wayland = std::env::var("WAYLAND_DISPLAY").is_ok();
    !has_x11 && !has_wayland
}

/// Returns `true` if running under a Wayland display server.
///
/// Checks for `$WAYLAND_DISPLAY` which is set by Wayland compositors.
///
/// # Examples
///
/// ```rust
/// use patina::tui::clipboard::is_wayland;
///
/// if is_wayland() {
///     println!("Wayland detected");
/// }
/// ```
#[must_use]
pub fn is_wayland() -> bool {
    std::env::var("WAYLAND_DISPLAY").is_ok()
}

/// Returns `true` if `wl-copy` is available on `$PATH`.
///
/// Wayland environments need `wl-copy` / `wl-paste` from the `wl-clipboard`
/// package for reliable clipboard access. If these tools are missing, we
/// fall back to OSC 52.
///
/// # Examples
///
/// ```rust
/// use patina::tui::clipboard::has_wl_copy;
///
/// if has_wl_copy() {
///     println!("wl-copy available for Wayland clipboard");
/// }
/// ```
#[must_use]
pub fn has_wl_copy() -> bool {
    Command::new("which")
        .arg("wl-copy")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Reads text from the system clipboard.
///
/// Uses the native clipboard via arboard. On headless systems, this will
/// likely fail — callers should handle the error and fall back to other
/// input methods.
///
/// # Errors
///
/// Returns an error if clipboard access fails.
pub fn paste_from_clipboard() -> Result<String> {
    let mut clipboard = arboard::Clipboard::new()?;
    let text = clipboard.get_text()?;
    tracing::debug!(len = text.len(), "Read from clipboard");
    Ok(text)
}

/// Copies text to the system clipboard.
///
/// Strategy selection:
/// 1. **Headless** (no `$DISPLAY` / `$WAYLAND_DISPLAY`) — skip arboard,
///    go directly to OSC 52
/// 2. **Wayland without `wl-copy`** — skip arboard (it may hang),
///    fall back to OSC 52
/// 3. **All other environments** — try arboard first, then OSC 52
///
/// # Arguments
///
/// * `text` - The text to copy to the clipboard
///
/// # Errors
///
/// Returns an error if all clipboard methods fail.
pub fn copy_to_clipboard(text: &str) -> Result<()> {
    // Headless: arboard will fail immediately, skip straight to OSC 52
    if is_headless() {
        tracing::debug!("Headless environment detected — using OSC 52 directly");
        copy_via_osc52(text)?;
        return Ok(());
    }

    // Wayland without wl-copy: arboard may hang or fail
    if is_wayland() && !has_wl_copy() {
        tracing::debug!(
            "Wayland without wl-copy detected — falling back to OSC 52"
        );
        copy_via_osc52(text)?;
        return Ok(());
    }

    // Try native clipboard first
    if let Ok(()) = copy_via_arboard(text) {
        tracing::debug!("Copied to clipboard via arboard");
        return Ok(());
    }

    // Fall back to OSC 52
    copy_via_osc52(text)?;
    tracing::debug!("Copied to clipboard via OSC 52");
    Ok(())
}

/// Copies text using the arboard crate (native clipboard).
fn copy_via_arboard(text: &str) -> Result<()> {
    let mut clipboard = arboard::Clipboard::new()?;
    clipboard.set_text(text)?;
    Ok(())
}

/// Copies text using OSC 52 escape sequence.
///
/// OSC 52 format: `ESC ] 52 ; c ; <base64-data> BEL`
///
/// This tells the terminal to put the decoded text into the clipboard.
/// Supported by: iTerm2, kitty, WezTerm, alacritty, foot, tmux, and many others.
///
/// Note: iTerm2 requires "Applications in terminal may access clipboard" to be
/// enabled in Preferences > General > Selection.
fn copy_via_osc52(text: &str) -> Result<()> {
    let encoded = STANDARD.encode(text);
    let sequence = format!("\x1b]52;c;{}\x07", encoded);

    // Write directly to stdout (terminal)
    let mut stdout = io::stdout();
    stdout.write_all(sequence.as_bytes())?;
    stdout.flush()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_osc52_encoding() {
        let text = "Hello, world!";
        let encoded = STANDARD.encode(text);
        assert_eq!(encoded, "SGVsbG8sIHdvcmxkIQ==");
    }

    #[test]
    fn test_osc52_sequence_format() {
        let text = "test";
        let encoded = STANDARD.encode(text);
        let sequence = format!("\x1b]52;c;{}\x07", encoded);
        assert!(sequence.starts_with("\x1b]52;c;"));
        assert!(sequence.ends_with("\x07"));
    }

    // ============================================================================
    // Environment-aware clipboard tests (9.2)
    // ============================================================================

    fn restore_env(key: &str, original: Option<String>) {
        if let Some(val) = original {
            env::set_var(key, val);
        } else {
            env::remove_var(key);
        }
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn test_detect_headless_no_display() {
        let orig_display = env::var("DISPLAY").ok();
        let orig_wayland = env::var("WAYLAND_DISPLAY").ok();

        env::remove_var("DISPLAY");
        env::remove_var("WAYLAND_DISPLAY");

        assert!(is_headless());

        restore_env("DISPLAY", orig_display);
        restore_env("WAYLAND_DISPLAY", orig_wayland);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn test_not_headless_with_x11_display() {
        let orig_display = env::var("DISPLAY").ok();
        let orig_wayland = env::var("WAYLAND_DISPLAY").ok();

        env::set_var("DISPLAY", ":0");
        env::remove_var("WAYLAND_DISPLAY");

        assert!(!is_headless());

        restore_env("DISPLAY", orig_display);
        restore_env("WAYLAND_DISPLAY", orig_wayland);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_never_headless() {
        // macOS always has a display server (pasteboard) even over SSH
        assert!(!is_headless());
    }

    #[test]
    fn test_detect_wayland_environment() {
        let orig = env::var("WAYLAND_DISPLAY").ok();

        env::set_var("WAYLAND_DISPLAY", "wayland-0");
        assert!(is_wayland());

        env::remove_var("WAYLAND_DISPLAY");
        assert!(!is_wayland());

        restore_env("WAYLAND_DISPLAY", orig);
    }

    #[test]
    fn test_has_wl_copy_returns_bool() {
        // Just verify it returns a bool without panicking.
        // On macOS/non-Wayland systems this should be false.
        let result = has_wl_copy();
        let _ = result;
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn test_headless_skips_arboard_uses_osc52() {
        let orig_display = env::var("DISPLAY").ok();
        let orig_wayland = env::var("WAYLAND_DISPLAY").ok();

        env::remove_var("DISPLAY");
        env::remove_var("WAYLAND_DISPLAY");

        // Verify the environment is detected as headless
        assert!(is_headless());

        // copy_to_clipboard in headless should go to OSC 52 path
        // We can't easily test the actual clipboard write in CI,
        // but we verify the detection logic is correct
        restore_env("DISPLAY", orig_display);
        restore_env("WAYLAND_DISPLAY", orig_wayland);
    }

    #[test]
    fn test_wayland_checks_wl_copy_availability() {
        let orig_wayland = env::var("WAYLAND_DISPLAY").ok();

        env::set_var("WAYLAND_DISPLAY", "wayland-0");
        assert!(is_wayland());
        // has_wl_copy() should return a definitive answer
        let _available = has_wl_copy();

        restore_env("WAYLAND_DISPLAY", orig_wayland);
    }
}
