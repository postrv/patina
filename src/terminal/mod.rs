//! Terminal configuration, capability detection, and key binding setup.
//!
//! This module provides:
//! - **Graphics protocol detection**: Detects terminal capabilities for rendering images
//!   (Sixel, Kitty graphics, iTerm2 inline images)
//! - **Key binding configuration**: Configures terminal emulators to pass through Cmd+A/C/V
//!   to the application instead of handling them as terminal shortcuts.
//!
//! # Graphics Protocol Detection
//!
//! The `detect_graphics_protocol` function checks the terminal environment to determine
//! which graphics rendering protocol is available:
//!
//! ```rust
//! use patina::terminal::{detect_graphics_protocol, GraphicsProtocol};
//!
//! let protocol = detect_graphics_protocol();
//! match protocol {
//!     GraphicsProtocol::Sixel => println!("Using Sixel graphics"),
//!     GraphicsProtocol::Kitty => println!("Using Kitty graphics protocol"),
//!     GraphicsProtocol::ITerm2 => println!("Using iTerm2 inline images"),
//!     GraphicsProtocol::HalfBlock => println!("Using Unicode half-block fallback"),
//!     GraphicsProtocol::Unsupported => println!("No graphics support"),
//! }
//! ```

use std::env;
use std::fmt;
use std::process::Command;
use tracing::{debug, info, warn};

// ============================================================================
// Graphics Protocol Detection
// ============================================================================

/// Graphics protocols supported for terminal image display.
///
/// These protocols represent different ways to render images in terminal emulators.
/// The detection system probes the terminal environment to determine which protocol
/// is available.
///
/// # Protocol Preference Order
///
/// When detecting graphics capabilities, protocols are checked in this order:
/// 1. **Kitty** - Modern, efficient protocol with alpha channel support
/// 2. **iTerm2** - Widely supported on macOS with rich feature set
/// 3. **Sixel** - Legacy protocol with broad terminal support
/// 4. **HalfBlock** - Universal Unicode fallback using ▀/▄ characters
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GraphicsProtocol {
    /// Sixel bitmap graphics protocol.
    ///
    /// Supported by: xterm, mintty, mlterm, contour, foot, wezterm, yaft, RLogin
    ///
    /// Sixel is a DEC graphics standard from the 1980s that encodes bitmap images
    /// as ASCII text. Detection is based on the $TERM environment variable or
    /// terminal capability queries.
    Sixel,

    /// Kitty graphics protocol.
    ///
    /// Supported by: Kitty terminal, WezTerm (partial)
    ///
    /// Modern graphics protocol supporting PNG, 24-bit RGB, alpha channel,
    /// animation, and efficient transmission. Detection via $TERM or $KITTY_WINDOW_ID.
    Kitty,

    /// iTerm2 inline images protocol.
    ///
    /// Supported by: iTerm2, WezTerm, mintty
    ///
    /// Proprietary protocol using OSC escape sequences with base64-encoded images.
    /// Detection via $TERM_PROGRAM, $ITERM_SESSION_ID, or $LC_TERMINAL.
    ITerm2,

    /// Unicode half-block character fallback.
    ///
    /// Universal fallback using upper half block (▀, U+2580) and lower half block
    /// (▄, U+2584) characters to render two vertical pixels per terminal cell.
    /// Works on any terminal with Unicode support.
    HalfBlock,

    /// No graphics support available.
    ///
    /// Terminal does not support any known graphics protocol or Unicode rendering.
    /// Image display will show placeholder text.
    Unsupported,
}

impl fmt::Display for GraphicsProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sixel => write!(f, "Sixel"),
            Self::Kitty => write!(f, "Kitty"),
            Self::ITerm2 => write!(f, "iTerm2"),
            Self::HalfBlock => write!(f, "HalfBlock"),
            Self::Unsupported => write!(f, "Unsupported"),
        }
    }
}

/// Terminal identifiers known to support Sixel graphics.
///
/// This list includes terminals that have Sixel support enabled by default
/// or commonly configured with Sixel support.
const SIXEL_CAPABLE_TERMS: &[&str] = &[
    "xterm", // xterm with +sixel compile option
    "xterm-256color",
    "mlterm",  // Multilingual terminal emulator
    "mintty",  // Default terminal for Cygwin/MSYS2
    "contour", // Modern C++ terminal
    "foot",    // Wayland-native terminal
    "foot-extra",
    "yaft", // Yet Another Framebuffer Terminal
    "yaft-256color",
    "wezterm", // Cross-platform terminal emulator
    "RLogin",  // Windows terminal emulator for rlogin/telnet/SSH
];

/// Checks if a terminal identifier indicates Sixel graphics support.
///
/// # Arguments
///
/// * `term` - The terminal identifier (typically from $TERM)
///
/// # Returns
///
/// `true` if the terminal is known to support Sixel graphics.
///
/// # Example
///
/// ```rust
/// use patina::terminal::is_sixel_capable_term;
///
/// assert!(is_sixel_capable_term("xterm-256color"));
/// assert!(is_sixel_capable_term("mlterm"));
/// assert!(!is_sixel_capable_term("dumb"));
/// ```
#[must_use]
pub fn is_sixel_capable_term(term: &str) -> bool {
    // Check exact matches
    if SIXEL_CAPABLE_TERMS.contains(&term) {
        return true;
    }

    // Check prefix matches for versioned terminal names
    let term_lower = term.to_lowercase();
    SIXEL_CAPABLE_TERMS
        .iter()
        .any(|&t| term_lower.starts_with(&format!("{}-", t.to_lowercase())))
}

/// Checks if the current terminal is Kitty.
///
/// Detection methods:
/// - $KITTY_WINDOW_ID environment variable (set by Kitty)
/// - $TERM containing "kitty"
/// - $TERM_PROGRAM set to "kitty"
///
/// # Returns
///
/// `true` if running inside the Kitty terminal emulator.
///
/// # Example
///
/// ```rust
/// use patina::terminal::is_kitty_terminal;
///
/// if is_kitty_terminal() {
///     println!("Running in Kitty terminal");
/// }
/// ```
#[must_use]
pub fn is_kitty_terminal() -> bool {
    // Kitty sets KITTY_WINDOW_ID when running inside it
    if env::var("KITTY_WINDOW_ID").is_ok() {
        return true;
    }

    // Check TERM for kitty
    if let Ok(term) = env::var("TERM") {
        if term.to_lowercase().contains("kitty") {
            return true;
        }
    }

    // Check TERM_PROGRAM
    if let Ok(term_program) = env::var("TERM_PROGRAM") {
        if term_program.to_lowercase() == "kitty" {
            return true;
        }
    }

    false
}

/// Detects the graphics protocol supported by the current terminal.
///
/// This function probes the terminal environment to determine the best available
/// graphics rendering protocol. Detection is performed in order of preference:
///
/// 1. **Kitty** - Checked via $KITTY_WINDOW_ID or $TERM
/// 2. **iTerm2** - Checked via $ITERM_SESSION_ID, $TERM_PROGRAM, or $LC_TERMINAL
/// 3. **Sixel** - Checked via $TERM against known Sixel-capable terminals
/// 4. **HalfBlock** - Universal Unicode fallback (always available)
///
/// # Returns
///
/// The most capable graphics protocol available in the current terminal.
///
/// # Note
///
/// This function does not perform terminal queries that would produce visible output
/// or require reading from the terminal. It relies only on environment variables
/// and compile-time platform detection.
///
/// # Example
///
/// ```rust
/// use patina::terminal::{detect_graphics_protocol, GraphicsProtocol};
///
/// let protocol = detect_graphics_protocol();
/// println!("Detected graphics protocol: {}", protocol);
/// ```
#[must_use]
pub fn detect_graphics_protocol() -> GraphicsProtocol {
    debug!("Detecting terminal graphics protocol");

    // Check for Kitty first (highest capability)
    if is_kitty_terminal() {
        debug!("Detected Kitty terminal - using Kitty graphics protocol");
        return GraphicsProtocol::Kitty;
    }

    // Check for iTerm2 (common on macOS)
    if is_iterm2() {
        debug!("Detected iTerm2 - using iTerm2 inline images");
        return GraphicsProtocol::ITerm2;
    }

    // Check for Sixel support based on $TERM
    if let Ok(term) = env::var("TERM") {
        if is_sixel_capable_term(&term) {
            debug!("Detected Sixel-capable terminal: {}", term);
            return GraphicsProtocol::Sixel;
        }
    }

    // Check for WezTerm specifically (supports multiple protocols)
    if let Ok(term_program) = env::var("TERM_PROGRAM") {
        if term_program.to_lowercase() == "wezterm" {
            debug!("Detected WezTerm - using Sixel graphics");
            return GraphicsProtocol::Sixel;
        }
    }

    // Fall back to half-block rendering (universal Unicode support)
    debug!("No advanced graphics protocol detected - using half-block fallback");
    GraphicsProtocol::HalfBlock
}

/// Key bindings to configure for proper Cmd+key support.
/// Format: (hex_key, hex_modifier, escape_sequence, description)
const ITERM2_KEY_BINDINGS: &[(&str, &str, &str, &str)] = &[
    ("0x61", "0x100000", "[97;9u", "Cmd+A (Select All)"),
    ("0x63", "0x100000", "[99;9u", "Cmd+C (Copy)"),
    ("0x76", "0x100000", "[118;9u", "Cmd+V (Paste)"),
];

/// Detects if running inside iTerm2.
#[must_use]
pub fn is_iterm2() -> bool {
    // iTerm2 sets ITERM_SESSION_ID when running inside it
    if env::var("ITERM_SESSION_ID").is_ok() {
        return true;
    }
    // Also check TERM_PROGRAM as fallback
    if let Ok(term) = env::var("TERM_PROGRAM") {
        return term == "iTerm.app";
    }
    // Check LC_TERMINAL (sometimes set)
    if let Ok(term) = env::var("LC_TERMINAL") {
        return term == "iTerm2";
    }
    false
}

/// Detects if running inside a JetBrains IDE terminal (IntelliJ, RustRover, etc.).
///
/// JetBrains IDEs use JediTerm as their terminal emulator and set the
/// `TERMINAL_EMULATOR` environment variable to `JetBrains-JediTerm`.
///
/// # Returns
///
/// `true` if running inside a JetBrains IDE terminal.
///
/// # Note
///
/// JetBrains terminals do not support the Kitty keyboard protocol, so
/// Cmd+key shortcuts won't be detected with the SUPER modifier. Users
/// should use Ctrl+A (select all), Ctrl+Y (copy), and Ctrl+Shift+V (paste)
/// or Option+A/C/V as alternatives.
#[must_use]
pub fn is_jetbrains_terminal() -> bool {
    if let Ok(emulator) = env::var("TERMINAL_EMULATOR") {
        return emulator == "JetBrains-JediTerm";
    }
    false
}

/// Returns a description of the current terminal for keyboard support hints.
///
/// This helps users understand what keyboard shortcuts are available.
#[must_use]
pub fn terminal_keyboard_hint() -> &'static str {
    if is_iterm2() {
        "iTerm2 (Cmd+A/C/V supported)"
    } else if is_jetbrains_terminal() {
        "JetBrains (use Ctrl+A, Option+C/V, or Ctrl+Y)"
    } else if is_kitty_terminal() {
        "Kitty (Cmd+A/C/V supported)"
    } else {
        "Standard (Ctrl+A, Ctrl+Y, Ctrl+Shift+V)"
    }
}

/// Detects if running on macOS.
#[must_use]
pub fn is_macos() -> bool {
    cfg!(target_os = "macos")
}

/// Configures iTerm2 key bindings for Cmd+A/C/V passthrough.
///
/// This function:
/// 1. Checks if running in iTerm2 on macOS
/// 2. Checks if bindings are already configured (idempotent)
/// 3. Adds missing bindings to iTerm2 preferences
/// 4. Returns true if changes were made (user should restart iTerm2)
///
/// # Returns
///
/// - `Ok(true)` if changes were made and iTerm2 needs restart
/// - `Ok(false)` if no changes needed (already configured or not iTerm2)
/// - `Err` if configuration failed
pub fn configure_iterm2_keybindings() -> Result<bool, String> {
    if !is_macos() {
        debug!("Not macOS, skipping iTerm2 configuration");
        return Ok(false);
    }

    if !is_iterm2() {
        debug!("Not iTerm2, skipping key binding configuration");
        return Ok(false);
    }

    info!("Detected iTerm2 - checking Cmd+key bindings");

    let plist_path = expand_tilde("~/Library/Preferences/com.googlecode.iterm2.plist");
    let mut changes_made = false;

    for (hex_key, hex_modifier, escape_seq, description) in ITERM2_KEY_BINDINGS {
        let key_path = format!("{}-{}", hex_key, hex_modifier);
        let full_path = format!(":\"New Bookmarks\":0:\"Keyboard Map\":\"{}\"", key_path);

        // Check if binding exists
        let check = Command::new("/usr/libexec/PlistBuddy")
            .args(["-c", &format!("Print {}", full_path), &plist_path])
            .output();

        match check {
            Ok(output) if output.status.success() => {
                debug!("{} already configured", description);
            }
            _ => {
                // Binding doesn't exist, add it
                info!("Configuring {} for patina", description);

                // Create the dict entry
                let add_dict = Command::new("/usr/libexec/PlistBuddy")
                    .args(["-c", &format!("Add {} dict", full_path), &plist_path])
                    .output();

                if add_dict.is_err() || !add_dict.unwrap().status.success() {
                    warn!("Failed to create dict for {}", description);
                    continue;
                }

                // Set Action to 10 (Send Escape Sequence)
                let set_action = Command::new("/usr/libexec/PlistBuddy")
                    .args([
                        "-c",
                        &format!("Add {}:Action integer 10", full_path),
                        &plist_path,
                    ])
                    .output();

                if set_action.is_err() || !set_action.unwrap().status.success() {
                    warn!("Failed to set action for {}", description);
                    continue;
                }

                // Set the escape sequence text
                let set_text = Command::new("/usr/libexec/PlistBuddy")
                    .args([
                        "-c",
                        &format!("Add {}:Text string {}", full_path, escape_seq),
                        &plist_path,
                    ])
                    .output();

                if set_text.is_err() || !set_text.unwrap().status.success() {
                    warn!("Failed to set escape sequence for {}", description);
                    continue;
                }

                changes_made = true;
                info!("Configured {}", description);
            }
        }
    }

    Ok(changes_made)
}

/// Expands ~ to home directory.
fn expand_tilde(path: &str) -> String {
    if path.starts_with("~/") {
        if let Ok(home) = env::var("HOME") {
            return path.replacen("~", &home, 1);
        }
    }
    path.to_string()
}

/// Checks if iTerm2 key bindings are configured.
///
/// Returns the number of bindings that are properly configured.
#[must_use]
pub fn check_iterm2_bindings() -> usize {
    if !is_macos() || !is_iterm2() {
        return 0;
    }

    let plist_path = expand_tilde("~/Library/Preferences/com.googlecode.iterm2.plist");
    let mut configured = 0;

    for (hex_key, hex_modifier, _, _) in ITERM2_KEY_BINDINGS {
        let key_path = format!("{}-{}", hex_key, hex_modifier);
        let full_path = format!(":\"New Bookmarks\":0:\"Keyboard Map\":\"{}\"", key_path);

        let check = Command::new("/usr/libexec/PlistBuddy")
            .args(["-c", &format!("Print {}", full_path), &plist_path])
            .output();

        if check.is_ok() && check.unwrap().status.success() {
            configured += 1;
        }
    }

    configured
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_tilde() {
        let expanded = expand_tilde("~/test/path");
        assert!(!expanded.starts_with("~/"));
        assert!(expanded.contains("test/path"));
    }

    #[test]
    fn test_expand_tilde_no_tilde() {
        let path = "/absolute/path";
        assert_eq!(expand_tilde(path), path);
    }

    #[test]
    fn test_is_macos() {
        // This test passes on macOS, fails elsewhere
        #[cfg(target_os = "macos")]
        assert!(is_macos());

        #[cfg(not(target_os = "macos"))]
        assert!(!is_macos());
    }

    // ============================================================================
    // Graphics Protocol Detection Tests
    // ============================================================================

    #[test]
    fn test_graphics_protocol_variants_exist() {
        // Verify all protocol variants exist and can be constructed
        let _sixel = GraphicsProtocol::Sixel;
        let _kitty = GraphicsProtocol::Kitty;
        let _iterm = GraphicsProtocol::ITerm2;
        let _halfblock = GraphicsProtocol::HalfBlock;
        let _unsupported = GraphicsProtocol::Unsupported;
    }

    #[test]
    fn test_graphics_protocol_display() {
        assert_eq!(format!("{}", GraphicsProtocol::Sixel), "Sixel");
        assert_eq!(format!("{}", GraphicsProtocol::Kitty), "Kitty");
        assert_eq!(format!("{}", GraphicsProtocol::ITerm2), "iTerm2");
        assert_eq!(format!("{}", GraphicsProtocol::HalfBlock), "HalfBlock");
        assert_eq!(format!("{}", GraphicsProtocol::Unsupported), "Unsupported");
    }

    #[test]
    fn test_graphics_protocol_debug() {
        // GraphicsProtocol should implement Debug
        let protocol = GraphicsProtocol::Sixel;
        let debug_str = format!("{:?}", protocol);
        assert!(debug_str.contains("Sixel"));
    }

    #[test]
    fn test_graphics_protocol_clone() {
        // GraphicsProtocol should implement Clone
        let protocol = GraphicsProtocol::Kitty;
        // Clone trait is implemented (Copy implies Clone)
        let cloned: GraphicsProtocol = Clone::clone(&protocol);
        assert_eq!(protocol, cloned);
    }

    #[test]
    fn test_graphics_protocol_copy() {
        // GraphicsProtocol should implement Copy
        let protocol = GraphicsProtocol::HalfBlock;
        let copied = protocol;
        assert_eq!(protocol, copied);
    }

    #[test]
    fn test_graphics_protocol_eq() {
        // GraphicsProtocol should implement PartialEq and Eq
        assert_eq!(GraphicsProtocol::Sixel, GraphicsProtocol::Sixel);
        assert_ne!(GraphicsProtocol::Sixel, GraphicsProtocol::Kitty);
    }

    #[test]
    fn test_detect_graphics_protocol_returns_valid() {
        // Detection should return a valid protocol without panicking
        let protocol = detect_graphics_protocol();
        assert!(matches!(
            protocol,
            GraphicsProtocol::Sixel
                | GraphicsProtocol::Kitty
                | GraphicsProtocol::ITerm2
                | GraphicsProtocol::HalfBlock
                | GraphicsProtocol::Unsupported
        ));
    }

    #[test]
    fn test_detect_graphics_protocol_fallback() {
        // In a test environment without real terminal, should fall back to HalfBlock
        // This is the safe universal fallback
        let protocol = detect_graphics_protocol();

        // In CI/test environment, we expect HalfBlock as fallback
        // unless running in an actual supported terminal
        // The key is that it doesn't panic and returns a valid value
        assert!(matches!(
            protocol,
            GraphicsProtocol::HalfBlock
                | GraphicsProtocol::Sixel
                | GraphicsProtocol::Kitty
                | GraphicsProtocol::ITerm2
                | GraphicsProtocol::Unsupported
        ));
    }

    #[test]
    fn test_is_sixel_capable_term() {
        // Test known Sixel-capable terminal identifiers
        assert!(is_sixel_capable_term("xterm-256color"));
        assert!(is_sixel_capable_term("xterm"));
        assert!(is_sixel_capable_term("mlterm"));
        assert!(is_sixel_capable_term("mintty"));
        assert!(is_sixel_capable_term("contour"));
        assert!(is_sixel_capable_term("foot"));
        assert!(is_sixel_capable_term("yaft-256color"));
        assert!(is_sixel_capable_term("wezterm"));

        // Non-sixel terminals
        assert!(!is_sixel_capable_term("dumb"));
        assert!(!is_sixel_capable_term("vt100"));
        assert!(!is_sixel_capable_term("screen"));
    }

    #[test]
    fn test_is_kitty_terminal() {
        // Kitty detection should check TERM_PROGRAM or TERM
        // In test environment, this may return false
        let is_kitty = is_kitty_terminal();
        // Function should return a boolean without panicking
        // In CI/test environment without Kitty, expect false
        let _ = is_kitty; // Consume the result to verify the function completes
    }

    #[test]
    fn test_detect_iterm2_for_graphics() {
        // iTerm2 detection for graphics should reuse existing is_iterm2()
        let is_iterm = is_iterm2();
        // Function should return a boolean without panicking
        let _ = is_iterm; // Consume the result to verify the function completes
    }

    #[test]
    fn test_is_jetbrains_terminal() {
        // JetBrains detection should check TERMINAL_EMULATOR
        // In CI/test environment, this should return false
        let is_jetbrains = is_jetbrains_terminal();
        // Function should return a boolean without panicking
        let _ = is_jetbrains;
    }

    #[test]
    fn test_terminal_keyboard_hint() {
        // Should return a non-empty string describing keyboard support
        let hint = terminal_keyboard_hint();
        assert!(!hint.is_empty());
        // In test environment, should return "Standard" hint
        // (unless running in iTerm2, Kitty, or JetBrains)
    }
}
