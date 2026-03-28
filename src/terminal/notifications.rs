//! Terminal notification support via OSC escape sequences.
//!
//! Detects the best notification protocol for the current terminal and emits
//! escape sequences to trigger native desktop notifications.
//!
//! # Supported Protocols
//!
//! | Protocol | Terminal | Escape |
//! |----------|----------|--------|
//! | `Osc9` | Most terminals (BEL) | `\x1b]9;{title}\x07` |
//! | `Osc777` | Kitty, urxvt | `\x1b]777;notify;{title};{body}\x07` |
//! | `Iterm2` | iTerm2 | `\x1b]9;{title}\n{body}\x07` |
//! | `Osc99` | Ghostty | `\x1b]99;i=1:d=0;{body}\x07` |
//!
//! # Examples
//!
//! ```rust
//! use patina::terminal::notifications::{NotificationManager, NotificationProtocol};
//!
//! let manager = NotificationManager::detect();
//! println!("Using protocol: {:?}", manager.protocol());
//! ```

use std::fmt;
use std::io::Write;

use tracing::debug;

use super::{is_iterm2, is_kitty_terminal};

/// Notification protocol supported by the terminal.
///
/// Each variant corresponds to a different OSC escape sequence format.
/// Detection chooses the most feature-rich protocol available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotificationProtocol {
    /// OSC 9 — universal BEL-based notification (title only).
    ///
    /// Works in most terminals. Format: `\x1b]9;{title}\x07`
    Osc9,

    /// OSC 777 — Kitty / urxvt notification with title and body.
    ///
    /// Format: `\x1b]777;notify;{title};{body}\x07`
    Osc777,

    /// iTerm2 notification with title and body.
    ///
    /// Format: `\x1b]9;{title}\n{body}\x07`
    Iterm2,

    /// OSC 99 — Ghostty notification with body.
    ///
    /// Format: `\x1b]99;i=1:d=0;{body}\x07`
    Osc99,

    /// No notification support — `notify()` is a no-op.
    None,
}

impl fmt::Display for NotificationProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Osc9 => write!(f, "OSC 9"),
            Self::Osc777 => write!(f, "OSC 777"),
            Self::Iterm2 => write!(f, "iTerm2"),
            Self::Osc99 => write!(f, "OSC 99"),
            Self::None => write!(f, "None"),
        }
    }
}

/// Manages terminal notifications using the detected protocol.
///
/// Created via [`detect`](NotificationManager::detect), which probes the
/// terminal environment to select the best available protocol. Notifications
/// are emitted as escape sequences written to stdout.
///
/// # Examples
///
/// ```rust,no_run
/// use patina::terminal::notifications::NotificationManager;
///
/// let manager = NotificationManager::detect();
/// manager.notify("Patina", "Task completed").unwrap();
/// ```
#[derive(Debug, Clone)]
pub struct NotificationManager {
    protocol: NotificationProtocol,
}

impl NotificationManager {
    /// Detects the best notification protocol for the current terminal.
    ///
    /// Detection order:
    /// 1. iTerm2 → [`NotificationProtocol::Iterm2`]
    /// 2. Kitty → [`NotificationProtocol::Osc777`]
    /// 3. Ghostty → [`NotificationProtocol::Osc99`]
    /// 4. Fallback → [`NotificationProtocol::Osc9`]
    #[must_use]
    pub fn detect() -> Self {
        let protocol = detect_notification_protocol();
        debug!("Detected notification protocol: {protocol}");
        Self { protocol }
    }

    /// Creates a notification manager with a specific protocol.
    ///
    /// Useful for testing or when the protocol is known in advance.
    #[must_use]
    pub fn with_protocol(protocol: NotificationProtocol) -> Self {
        Self { protocol }
    }

    /// Returns the detected notification protocol.
    #[must_use]
    pub fn protocol(&self) -> NotificationProtocol {
        self.protocol
    }

    /// Sends a desktop notification via the terminal's escape sequence protocol.
    ///
    /// # Arguments
    ///
    /// * `title` - The notification title (used by Osc9, Osc777, Iterm2)
    /// * `body` - The notification body text
    ///
    /// # Errors
    ///
    /// Returns an error if writing to stdout fails.
    pub fn notify(&self, title: &str, body: &str) -> anyhow::Result<()> {
        let sequence = self.build_sequence(title, body);
        if sequence.is_empty() {
            return Ok(());
        }

        let mut stdout = std::io::stdout().lock();
        stdout.write_all(sequence.as_bytes())?;
        stdout.flush()?;

        debug!(
            protocol = %self.protocol,
            title = %title,
            "Sent terminal notification"
        );

        Ok(())
    }

    /// Builds the escape sequence for the current protocol.
    ///
    /// Sanitizes `title` and `body` to strip control characters (`\x00`–`\x1f`,
    /// `\x7f`, `\x1b`) before interpolation, preventing escape sequence injection.
    ///
    /// Returns an empty string for [`NotificationProtocol::None`].
    #[must_use]
    fn build_sequence(&self, title: &str, body: &str) -> String {
        let title = sanitize_osc(title);
        let body = sanitize_osc(body);
        match self.protocol {
            NotificationProtocol::Osc9 => format!("\x1b]9;{title}\x07"),
            NotificationProtocol::Osc777 => {
                format!("\x1b]777;notify;{title};{body}\x07")
            }
            NotificationProtocol::Iterm2 => format!("\x1b]9;{title}\n{body}\x07"),
            NotificationProtocol::Osc99 => format!("\x1b]99;i=1:d=0;{body}\x07"),
            NotificationProtocol::None => String::new(),
        }
    }
}

/// Strips control characters from a string for safe inclusion in OSC escape sequences.
///
/// Removes all C0 control chars (`\x00`–`\x1f`), DEL (`\x7f`), and ESC (`\x1b`)
/// to prevent premature sequence termination or injection of arbitrary escape codes.
fn sanitize_osc(s: &str) -> String {
    s.chars().filter(|c| !c.is_ascii_control()).collect()
}

/// Detects the notification protocol for the current terminal.
///
/// Reuses the existing terminal detection functions from the parent module.
#[must_use]
fn detect_notification_protocol() -> NotificationProtocol {
    if is_iterm2() {
        return NotificationProtocol::Iterm2;
    }

    if is_kitty_terminal() {
        return NotificationProtocol::Osc777;
    }

    if is_ghostty() {
        return NotificationProtocol::Osc99;
    }

    // Universal fallback — BEL-based notification
    NotificationProtocol::Osc9
}

/// Checks if the terminal is Ghostty.
///
/// Detection via `$TERM_PROGRAM == "ghostty"`.
#[must_use]
fn is_ghostty() -> bool {
    std::env::var("TERM_PROGRAM")
        .map(|v| v.to_lowercase() == "ghostty")
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use serial_test::serial;

    use super::*;

    // =========================================================================
    // NotificationProtocol tests
    // =========================================================================

    #[test]
    fn test_protocol_display() {
        assert_eq!(format!("{}", NotificationProtocol::Osc9), "OSC 9");
        assert_eq!(format!("{}", NotificationProtocol::Osc777), "OSC 777");
        assert_eq!(format!("{}", NotificationProtocol::Iterm2), "iTerm2");
        assert_eq!(format!("{}", NotificationProtocol::Osc99), "OSC 99");
        assert_eq!(format!("{}", NotificationProtocol::None), "None");
    }

    #[test]
    fn test_protocol_eq() {
        assert_eq!(NotificationProtocol::Osc9, NotificationProtocol::Osc9);
        assert_ne!(NotificationProtocol::Osc9, NotificationProtocol::Osc777);
    }

    #[test]
    fn test_protocol_clone_copy() {
        let p = NotificationProtocol::Iterm2;
        let cloned = Clone::clone(&p);
        let copied = p;
        assert_eq!(p, cloned);
        assert_eq!(p, copied);
    }

    // =========================================================================
    // Escape sequence format tests
    // =========================================================================

    #[test]
    fn test_osc9_escape_sequence() {
        let mgr = NotificationManager::with_protocol(NotificationProtocol::Osc9);
        let seq = mgr.build_sequence("Patina", "Task done");
        assert_eq!(seq, "\x1b]9;Patina\x07");
    }

    #[test]
    fn test_osc777_escape_sequence() {
        let mgr = NotificationManager::with_protocol(NotificationProtocol::Osc777);
        let seq = mgr.build_sequence("Patina", "Task done");
        assert_eq!(seq, "\x1b]777;notify;Patina;Task done\x07");
    }

    #[test]
    fn test_iterm2_escape_sequence() {
        let mgr = NotificationManager::with_protocol(NotificationProtocol::Iterm2);
        let seq = mgr.build_sequence("Patina", "Task done");
        assert_eq!(seq, "\x1b]9;Patina\nTask done\x07");
    }

    #[test]
    fn test_osc99_escape_sequence() {
        let mgr = NotificationManager::with_protocol(NotificationProtocol::Osc99);
        let seq = mgr.build_sequence("Patina", "Task done");
        assert_eq!(seq, "\x1b]99;i=1:d=0;Task done\x07");
    }

    #[test]
    fn test_none_protocol_empty_sequence() {
        let mgr = NotificationManager::with_protocol(NotificationProtocol::None);
        let seq = mgr.build_sequence("Patina", "Task done");
        assert!(seq.is_empty());
    }

    #[test]
    fn test_none_protocol_notify_is_noop() {
        let mgr = NotificationManager::with_protocol(NotificationProtocol::None);
        let result = mgr.notify("Patina", "Task done");
        assert!(result.is_ok());
    }

    // =========================================================================
    // Detection tests (env-var controlled)
    // =========================================================================

    #[serial]
    #[test]
    fn test_detect_iterm2_protocol() {
        // Save and set env
        let saved = std::env::var("ITERM_SESSION_ID").ok();
        let saved_tp = std::env::var("TERM_PROGRAM").ok();
        let saved_kitty = std::env::var("KITTY_WINDOW_ID").ok();

        std::env::set_var("ITERM_SESSION_ID", "test-session");
        std::env::remove_var("KITTY_WINDOW_ID");

        let protocol = detect_notification_protocol();
        assert_eq!(
            protocol,
            NotificationProtocol::Iterm2,
            "Should detect iTerm2"
        );

        // Restore
        match saved {
            Some(v) => std::env::set_var("ITERM_SESSION_ID", v),
            None => std::env::remove_var("ITERM_SESSION_ID"),
        }
        match saved_tp {
            Some(v) => std::env::set_var("TERM_PROGRAM", v),
            None => std::env::remove_var("TERM_PROGRAM"),
        }
        match saved_kitty {
            Some(v) => std::env::set_var("KITTY_WINDOW_ID", v),
            None => std::env::remove_var("KITTY_WINDOW_ID"),
        }
    }

    #[serial]
    #[test]
    fn test_detect_kitty_protocol() {
        // Save and set env
        let saved_iterm = std::env::var("ITERM_SESSION_ID").ok();
        let saved_tp = std::env::var("TERM_PROGRAM").ok();
        let saved_kitty = std::env::var("KITTY_WINDOW_ID").ok();
        let saved_lc = std::env::var("LC_TERMINAL").ok();

        std::env::remove_var("ITERM_SESSION_ID");
        std::env::remove_var("LC_TERMINAL");
        std::env::set_var("TERM_PROGRAM", "other");
        std::env::set_var("KITTY_WINDOW_ID", "42");

        let protocol = detect_notification_protocol();
        assert_eq!(
            protocol,
            NotificationProtocol::Osc777,
            "Should detect Kitty via KITTY_WINDOW_ID"
        );

        // Restore
        match saved_iterm {
            Some(v) => std::env::set_var("ITERM_SESSION_ID", v),
            None => std::env::remove_var("ITERM_SESSION_ID"),
        }
        match saved_tp {
            Some(v) => std::env::set_var("TERM_PROGRAM", v),
            None => std::env::remove_var("TERM_PROGRAM"),
        }
        match saved_kitty {
            Some(v) => std::env::set_var("KITTY_WINDOW_ID", v),
            None => std::env::remove_var("KITTY_WINDOW_ID"),
        }
        match saved_lc {
            Some(v) => std::env::set_var("LC_TERMINAL", v),
            None => std::env::remove_var("LC_TERMINAL"),
        }
    }

    #[serial]
    #[test]
    fn test_detect_ghostty_protocol() {
        // Save and set env
        let saved_iterm = std::env::var("ITERM_SESSION_ID").ok();
        let saved_tp = std::env::var("TERM_PROGRAM").ok();
        let saved_kitty = std::env::var("KITTY_WINDOW_ID").ok();
        let saved_lc = std::env::var("LC_TERMINAL").ok();

        std::env::remove_var("ITERM_SESSION_ID");
        std::env::remove_var("KITTY_WINDOW_ID");
        std::env::remove_var("LC_TERMINAL");
        std::env::set_var("TERM_PROGRAM", "ghostty");

        let protocol = detect_notification_protocol();
        assert_eq!(
            protocol,
            NotificationProtocol::Osc99,
            "Should detect Ghostty"
        );

        // Restore
        match saved_iterm {
            Some(v) => std::env::set_var("ITERM_SESSION_ID", v),
            None => std::env::remove_var("ITERM_SESSION_ID"),
        }
        match saved_tp {
            Some(v) => std::env::set_var("TERM_PROGRAM", v),
            None => std::env::remove_var("TERM_PROGRAM"),
        }
        match saved_kitty {
            Some(v) => std::env::set_var("KITTY_WINDOW_ID", v),
            None => std::env::remove_var("KITTY_WINDOW_ID"),
        }
        match saved_lc {
            Some(v) => std::env::set_var("LC_TERMINAL", v),
            None => std::env::remove_var("LC_TERMINAL"),
        }
    }

    #[serial]
    #[test]
    fn test_detect_fallback_osc9() {
        // Save and clear all terminal-specific env vars
        let saved_iterm = std::env::var("ITERM_SESSION_ID").ok();
        let saved_tp = std::env::var("TERM_PROGRAM").ok();
        let saved_kitty = std::env::var("KITTY_WINDOW_ID").ok();
        let saved_term = std::env::var("TERM").ok();
        let saved_lc = std::env::var("LC_TERMINAL").ok();

        std::env::remove_var("ITERM_SESSION_ID");
        std::env::remove_var("KITTY_WINDOW_ID");
        std::env::remove_var("LC_TERMINAL");
        std::env::set_var("TERM_PROGRAM", "unknown-terminal");
        std::env::set_var("TERM", "xterm-256color");

        let protocol = detect_notification_protocol();
        assert_eq!(
            protocol,
            NotificationProtocol::Osc9,
            "Should fall back to OSC 9"
        );

        // Restore
        match saved_iterm {
            Some(v) => std::env::set_var("ITERM_SESSION_ID", v),
            None => std::env::remove_var("ITERM_SESSION_ID"),
        }
        match saved_tp {
            Some(v) => std::env::set_var("TERM_PROGRAM", v),
            None => std::env::remove_var("TERM_PROGRAM"),
        }
        match saved_kitty {
            Some(v) => std::env::set_var("KITTY_WINDOW_ID", v),
            None => std::env::remove_var("KITTY_WINDOW_ID"),
        }
        match saved_term {
            Some(v) => std::env::set_var("TERM", v),
            None => std::env::remove_var("TERM"),
        }
        match saved_lc {
            Some(v) => std::env::set_var("LC_TERMINAL", v),
            None => std::env::remove_var("LC_TERMINAL"),
        }
    }

    // =========================================================================
    // NotificationManager tests
    // =========================================================================

    #[test]
    fn test_manager_with_protocol() {
        let mgr = NotificationManager::with_protocol(NotificationProtocol::Osc777);
        assert_eq!(mgr.protocol(), NotificationProtocol::Osc777);
    }

    #[test]
    fn test_manager_detect_returns_valid() {
        let mgr = NotificationManager::detect();
        assert!(matches!(
            mgr.protocol(),
            NotificationProtocol::Osc9
                | NotificationProtocol::Osc777
                | NotificationProtocol::Iterm2
                | NotificationProtocol::Osc99
                | NotificationProtocol::None
        ));
    }

    #[test]
    fn test_is_ghostty_false_by_default() {
        // In test environment, TERM_PROGRAM is unlikely to be "ghostty"
        // But this verifies the function doesn't panic
        let _ = is_ghostty();
    }

    // =========================================================================
    // OSC sanitization tests
    // =========================================================================

    #[test]
    fn test_sanitize_osc_strips_bel() {
        assert_eq!(sanitize_osc("hello\x07world"), "helloworld");
    }

    #[test]
    fn test_sanitize_osc_strips_escape() {
        assert_eq!(sanitize_osc("hello\x1bworld"), "helloworld");
    }

    #[test]
    fn test_sanitize_osc_strips_all_control_chars() {
        let input = "a\x00b\x01c\x1bd\x07e\x0af";
        let sanitized = sanitize_osc(input);
        assert_eq!(sanitized, "abcdef");
    }

    #[test]
    fn test_sanitize_osc_preserves_normal_text() {
        let input = "Task complete: src/main.rs (3 files changed)";
        assert_eq!(sanitize_osc(input), input);
    }

    #[test]
    fn test_sanitize_osc_blocks_injection_attempt() {
        // Attempt to inject a fake notification via BEL + new OSC sequence
        let malicious = "evil\x07\x1b]777;notify;Spoofed;Fake\x07";
        let sanitized = sanitize_osc(malicious);
        assert!(!sanitized.contains('\x07'));
        assert!(!sanitized.contains('\x1b'));
        assert_eq!(sanitized, "evil]777;notify;Spoofed;Fake");
    }

    #[test]
    fn test_build_sequence_sanitizes_input() {
        let mgr = NotificationManager::with_protocol(NotificationProtocol::Osc777);
        let seq = mgr.build_sequence("Title\x07inject", "Body\x1b[0m");
        // Should not contain raw control chars from the input
        assert!(!seq.contains("\x07inject"));
        assert!(!seq.contains("\x1b[0m"));
        // But should still have the protocol-level control chars
        assert!(seq.starts_with("\x1b]777;"));
        assert!(seq.ends_with("\x07"));
    }
}
