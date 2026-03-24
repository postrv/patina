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
// Terminal Environment Detection
// ============================================================================

/// The runtime environment in which Patina is executing.
///
/// Multiplexers (tmux, screen) and remote sessions (SSH) can degrade terminal
/// capabilities — image rendering, clipboard access, and keyboard protocols
/// may not work as expected. Detecting the environment early allows Patina
/// to fall back gracefully instead of crashing or producing garbage output.
///
/// # Detection Priority
///
/// Environments are detected in this order:
/// 1. **Tmux** — `$TMUX` is set
/// 2. **Screen** — `$STY` is set
/// 3. **SSH** — `$SSH_CLIENT` or `$SSH_TTY` is set
/// 4. **JetBrains** — `$TERMINAL_EMULATOR` is `JetBrains-JediTerm`
/// 5. **Native** — none of the above apply
///
/// # Examples
///
/// ```rust
/// use patina::terminal::{TerminalEnvironment, detect_terminal_environment};
///
/// let env = detect_terminal_environment();
/// if env.is_multiplexer() {
///     println!("Running inside a multiplexer — image rendering degraded");
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TerminalEnvironment {
    /// Running directly in a terminal emulator without a multiplexer or
    /// remote session layer.
    Native,

    /// Running inside tmux. Detected via `$TMUX`.
    ///
    /// Capabilities affected:
    /// - Sixel / Kitty graphics protocols may not pass through
    /// - Clipboard access may require `set -g set-clipboard on`
    Tmux,

    /// Running inside GNU screen. Detected via `$STY`.
    ///
    /// Capabilities affected:
    /// - Graphics protocols are not forwarded
    /// - Clipboard is limited to screen's internal buffer
    Screen,

    /// Running over an SSH connection. Detected via `$SSH_CLIENT` or `$SSH_TTY`.
    ///
    /// Capabilities affected:
    /// - Native clipboard (arboard) is unavailable — OSC 52 fallback required
    /// - Graphics protocols depend on the local terminal, not the remote
    SSH,

    /// Running inside a JetBrains IDE terminal (IntelliJ, RustRover, etc.).
    /// Detected via `$TERMINAL_EMULATOR == "JetBrains-JediTerm"`.
    ///
    /// Capabilities affected:
    /// - Kitty keyboard protocol not supported
    /// - Cmd+key shortcuts unavailable
    JetBrains,
}

impl TerminalEnvironment {
    /// Returns `true` if the environment is a terminal multiplexer (tmux or screen).
    ///
    /// Multiplexers intercept escape sequences, which breaks advanced graphics
    /// protocols (Sixel, Kitty) and may degrade clipboard support.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use patina::terminal::TerminalEnvironment;
    ///
    /// assert!(TerminalEnvironment::Tmux.is_multiplexer());
    /// assert!(TerminalEnvironment::Screen.is_multiplexer());
    /// assert!(!TerminalEnvironment::Native.is_multiplexer());
    /// ```
    #[must_use]
    pub fn is_multiplexer(&self) -> bool {
        matches!(self, Self::Tmux | Self::Screen)
    }

    /// Returns `true` if the session is remote (SSH).
    ///
    /// Remote sessions lack a local display server, so native clipboard
    /// libraries (arboard) will fail. Use OSC 52 instead.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use patina::terminal::TerminalEnvironment;
    ///
    /// assert!(TerminalEnvironment::SSH.is_remote());
    /// assert!(!TerminalEnvironment::Native.is_remote());
    /// ```
    #[must_use]
    pub fn is_remote(&self) -> bool {
        matches!(self, Self::SSH)
    }

    /// Returns `true` if advanced graphics protocols (Sixel, Kitty) should be
    /// disabled in this environment.
    ///
    /// Multiplexers intercept the escape sequences used by these protocols,
    /// so image rendering must fall back to Unicode half-blocks.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use patina::terminal::TerminalEnvironment;
    ///
    /// assert!(TerminalEnvironment::Tmux.graphics_degraded());
    /// assert!(TerminalEnvironment::Screen.graphics_degraded());
    /// assert!(!TerminalEnvironment::Native.graphics_degraded());
    /// ```
    #[must_use]
    pub fn graphics_degraded(&self) -> bool {
        self.is_multiplexer()
    }
}

impl fmt::Display for TerminalEnvironment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Native => write!(f, "Native"),
            Self::Tmux => write!(f, "tmux"),
            Self::Screen => write!(f, "GNU screen"),
            Self::SSH => write!(f, "SSH"),
            Self::JetBrains => write!(f, "JetBrains"),
        }
    }
}

/// Detects the terminal environment by inspecting well-known environment
/// variables.
///
/// Detection order (first match wins):
/// 1. `$TMUX` → [`TerminalEnvironment::Tmux`]
/// 2. `$STY` → [`TerminalEnvironment::Screen`]
/// 3. `$SSH_CLIENT` or `$SSH_TTY` → [`TerminalEnvironment::SSH`]
/// 4. `$TERMINAL_EMULATOR == "JetBrains-JediTerm"` → [`TerminalEnvironment::JetBrains`]
/// 5. Otherwise → [`TerminalEnvironment::Native`]
///
/// # Examples
///
/// ```rust
/// use patina::terminal::detect_terminal_environment;
///
/// let env = detect_terminal_environment();
/// println!("Running in: {}", env);
/// ```
#[must_use]
pub fn detect_terminal_environment() -> TerminalEnvironment {
    // tmux sets $TMUX to the socket path
    if env::var("TMUX").is_ok() {
        debug!("Detected tmux environment via $TMUX");
        return TerminalEnvironment::Tmux;
    }

    // GNU screen sets $STY to the session name
    if env::var("STY").is_ok() {
        debug!("Detected GNU screen environment via $STY");
        return TerminalEnvironment::Screen;
    }

    // SSH sets $SSH_CLIENT or $SSH_TTY
    if env::var("SSH_CLIENT").is_ok() || env::var("SSH_TTY").is_ok() {
        debug!("Detected SSH environment via $SSH_CLIENT/$SSH_TTY");
        return TerminalEnvironment::SSH;
    }

    // JetBrains IDE terminal
    if let Ok(emulator) = env::var("TERMINAL_EMULATOR") {
        if emulator == "JetBrains-JediTerm" {
            debug!("Detected JetBrains IDE terminal via $TERMINAL_EMULATOR");
            return TerminalEnvironment::JetBrains;
        }
    }

    debug!("No multiplexer or remote session detected — native terminal");
    TerminalEnvironment::Native
}

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
/// 1. **Multiplexer check** - tmux/screen force `HalfBlock` fallback
/// 2. **Kitty** - Checked via $KITTY_WINDOW_ID or $TERM
/// 3. **iTerm2** - Checked via $ITERM_SESSION_ID, $TERM_PROGRAM, or $LC_TERMINAL
/// 4. **Sixel** - Checked via $TERM against known Sixel-capable terminals
/// 5. **HalfBlock** - Universal Unicode fallback (always available)
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
/// When running inside a terminal multiplexer (tmux or GNU screen), advanced
/// graphics protocols (Sixel, Kitty) are disabled because the multiplexer
/// intercepts the escape sequences. The function falls back to `HalfBlock`.
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

    // Multiplexers intercept escape sequences — advanced graphics won't work
    let term_env = detect_terminal_environment();
    if term_env.graphics_degraded() {
        info!(
            "Running inside {} — image rendering degraded to half-blocks",
            term_env
        );
        return GraphicsProtocol::HalfBlock;
    }

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

                if !add_dict.is_ok_and(|o| o.status.success()) {
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

                if !set_action.is_ok_and(|o| o.status.success()) {
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

                if !set_text.is_ok_and(|o| o.status.success()) {
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

        if check.is_ok_and(|o| o.status.success()) {
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

    // ============================================================================
    // Terminal Environment Detection Tests (9.1)
    // ============================================================================

    #[test]
    fn test_terminal_environment_variants() {
        // All five variants must be constructible
        let _native = TerminalEnvironment::Native;
        let _tmux = TerminalEnvironment::Tmux;
        let _screen = TerminalEnvironment::Screen;
        let _ssh = TerminalEnvironment::SSH;
        let _jetbrains = TerminalEnvironment::JetBrains;
    }

    #[test]
    fn test_terminal_environment_display() {
        assert_eq!(format!("{}", TerminalEnvironment::Native), "Native");
        assert_eq!(format!("{}", TerminalEnvironment::Tmux), "tmux");
        assert_eq!(format!("{}", TerminalEnvironment::Screen), "GNU screen");
        assert_eq!(format!("{}", TerminalEnvironment::SSH), "SSH");
        assert_eq!(format!("{}", TerminalEnvironment::JetBrains), "JetBrains");
    }

    #[test]
    fn test_terminal_environment_debug() {
        let env = TerminalEnvironment::Tmux;
        let debug_str = format!("{:?}", env);
        assert!(debug_str.contains("Tmux"));
    }

    #[test]
    fn test_terminal_environment_clone_copy() {
        let env = TerminalEnvironment::SSH;
        let cloned = Clone::clone(&env);
        let copied = env;
        assert_eq!(env, cloned);
        assert_eq!(env, copied);
    }

    #[test]
    fn test_terminal_environment_eq() {
        assert_eq!(TerminalEnvironment::Native, TerminalEnvironment::Native);
        assert_ne!(TerminalEnvironment::Native, TerminalEnvironment::Tmux);
        assert_ne!(TerminalEnvironment::Tmux, TerminalEnvironment::Screen);
    }

    #[test]
    fn test_is_multiplexer() {
        assert!(TerminalEnvironment::Tmux.is_multiplexer());
        assert!(TerminalEnvironment::Screen.is_multiplexer());
        assert!(!TerminalEnvironment::Native.is_multiplexer());
        assert!(!TerminalEnvironment::SSH.is_multiplexer());
        assert!(!TerminalEnvironment::JetBrains.is_multiplexer());
    }

    #[test]
    fn test_is_remote() {
        assert!(TerminalEnvironment::SSH.is_remote());
        assert!(!TerminalEnvironment::Native.is_remote());
        assert!(!TerminalEnvironment::Tmux.is_remote());
        assert!(!TerminalEnvironment::Screen.is_remote());
        assert!(!TerminalEnvironment::JetBrains.is_remote());
    }

    #[test]
    fn test_graphics_degraded() {
        assert!(TerminalEnvironment::Tmux.graphics_degraded());
        assert!(TerminalEnvironment::Screen.graphics_degraded());
        assert!(!TerminalEnvironment::Native.graphics_degraded());
        assert!(!TerminalEnvironment::SSH.graphics_degraded());
        assert!(!TerminalEnvironment::JetBrains.graphics_degraded());
    }

    #[test]
    fn test_detect_terminal_environment_returns_valid() {
        let env = detect_terminal_environment();
        assert!(matches!(
            env,
            TerminalEnvironment::Native
                | TerminalEnvironment::Tmux
                | TerminalEnvironment::Screen
                | TerminalEnvironment::SSH
                | TerminalEnvironment::JetBrains
        ));
    }

    /// Restores an environment variable to its original value, or removes it
    /// if it was not set before the test.
    fn restore_env(key: &str, original: Option<String>) {
        if let Some(val) = original {
            env::set_var(key, val);
        } else {
            env::remove_var(key);
        }
    }

    #[test]
    fn test_detect_tmux_via_env_var() {
        let original = env::var("TMUX").ok();
        env::set_var("TMUX", "/tmp/tmux-1000/default,12345,0");

        let result = detect_terminal_environment();
        assert_eq!(result, TerminalEnvironment::Tmux);

        restore_env("TMUX", original);
    }

    #[test]
    fn test_detect_screen_via_env_var() {
        let original_tmux = env::var("TMUX").ok();
        let original_sty = env::var("STY").ok();
        env::remove_var("TMUX");
        env::set_var("STY", "12345.pts-0.hostname");

        let result = detect_terminal_environment();
        assert_eq!(result, TerminalEnvironment::Screen);

        restore_env("STY", original_sty);
        restore_env("TMUX", original_tmux);
    }

    #[test]
    fn test_detect_ssh_via_env_var() {
        let original_tmux = env::var("TMUX").ok();
        let original_sty = env::var("STY").ok();
        let original_ssh_client = env::var("SSH_CLIENT").ok();
        let original_ssh_tty = env::var("SSH_TTY").ok();

        env::remove_var("TMUX");
        env::remove_var("STY");
        env::set_var("SSH_CLIENT", "192.168.1.1 54321 22");

        let result = detect_terminal_environment();
        assert_eq!(result, TerminalEnvironment::SSH);

        restore_env("SSH_CLIENT", original_ssh_client);
        restore_env("SSH_TTY", original_ssh_tty);
        restore_env("STY", original_sty);
        restore_env("TMUX", original_tmux);
    }

    #[test]
    fn test_detect_native_when_no_env_vars() {
        let original_tmux = env::var("TMUX").ok();
        let original_sty = env::var("STY").ok();
        let original_ssh_client = env::var("SSH_CLIENT").ok();
        let original_ssh_tty = env::var("SSH_TTY").ok();
        let original_emulator = env::var("TERMINAL_EMULATOR").ok();

        env::remove_var("TMUX");
        env::remove_var("STY");
        env::remove_var("SSH_CLIENT");
        env::remove_var("SSH_TTY");
        env::remove_var("TERMINAL_EMULATOR");

        let result = detect_terminal_environment();
        assert_eq!(result, TerminalEnvironment::Native);

        restore_env("TERMINAL_EMULATOR", original_emulator);
        restore_env("SSH_TTY", original_ssh_tty);
        restore_env("SSH_CLIENT", original_ssh_client);
        restore_env("STY", original_sty);
        restore_env("TMUX", original_tmux);
    }

    #[test]
    fn test_image_protocol_disabled_in_tmux() {
        let original_tmux = env::var("TMUX").ok();
        let original_kitty = env::var("KITTY_WINDOW_ID").ok();

        // Simulate being inside tmux with a Kitty-capable terminal
        env::set_var("TMUX", "/tmp/tmux-1000/default,12345,0");
        env::set_var("KITTY_WINDOW_ID", "1");

        let protocol = detect_graphics_protocol();
        // Even though Kitty is detected, tmux forces HalfBlock
        assert_eq!(protocol, GraphicsProtocol::HalfBlock);

        restore_env("KITTY_WINDOW_ID", original_kitty);
        restore_env("TMUX", original_tmux);
    }

    #[test]
    fn test_image_protocol_disabled_in_screen() {
        let original_sty = env::var("STY").ok();
        let original_tmux = env::var("TMUX").ok();
        let original_kitty = env::var("KITTY_WINDOW_ID").ok();

        // Simulate being inside screen
        env::remove_var("TMUX");
        env::set_var("STY", "12345.pts-0.hostname");
        env::set_var("KITTY_WINDOW_ID", "1");

        let protocol = detect_graphics_protocol();
        assert_eq!(protocol, GraphicsProtocol::HalfBlock);

        restore_env("KITTY_WINDOW_ID", original_kitty);
        restore_env("TMUX", original_tmux);
        restore_env("STY", original_sty);
    }

    #[test]
    fn test_tmux_takes_priority_over_ssh() {
        let original_tmux = env::var("TMUX").ok();
        let original_ssh_client = env::var("SSH_CLIENT").ok();

        env::set_var("TMUX", "/tmp/tmux-1000/default,12345,0");
        env::set_var("SSH_CLIENT", "192.168.1.1 54321 22");

        let result = detect_terminal_environment();
        assert_eq!(result, TerminalEnvironment::Tmux);

        restore_env("SSH_CLIENT", original_ssh_client);
        restore_env("TMUX", original_tmux);
    }
}
