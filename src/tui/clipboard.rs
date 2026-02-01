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

/// Copies text to the system clipboard.
///
/// Tries multiple methods in order:
/// 1. Native clipboard (arboard) - works on desktop
/// 2. OSC 52 escape sequence - works in supported terminals
///
/// # Arguments
///
/// * `text` - The text to copy to the clipboard
///
/// # Errors
///
/// Returns an error if all clipboard methods fail.
pub fn copy_to_clipboard(text: &str) -> Result<()> {
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
}
