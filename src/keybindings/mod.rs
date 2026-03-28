//! Keybinding customization system.
//!
//! Provides a configurable keybinding manager that supports single key presses
//! and multi-key chords (e.g., Ctrl+X Ctrl+K). Keybindings can be loaded from
//! a JSON configuration file, with user bindings overriding defaults.
//!
//! # JSON Configuration Format
//!
//! ```json
//! {
//!   "bindings": {
//!     "ctrl+c": "quit",
//!     "ctrl+d": "quit",
//!     "enter": "submit",
//!     "shift+enter": "new_line",
//!     "ctrl+x ctrl+k": "kill_background_agents"
//!   }
//! }
//! ```
//!
//! # Examples
//!
//! ```rust
//! use patina::keybindings::{KeybindingManager, KeyPress, KeyResolution, Action};
//! use crossterm::event::{KeyCode, KeyModifiers};
//!
//! let mut mgr = KeybindingManager::with_defaults();
//! let key = KeyPress::from_crossterm(KeyCode::Char('c'), KeyModifiers::CONTROL);
//! assert_eq!(mgr.resolve(key), KeyResolution::Action(Action::Quit));
//! ```

use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyModifiers};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// A single key press with optional modifiers.
///
/// Wraps a crossterm `KeyCode` and `KeyModifiers` into a hashable,
/// comparable struct suitable for use as a binding key.
///
/// # Examples
///
/// ```rust
/// use patina::keybindings::KeyPress;
/// use crossterm::event::{KeyCode, KeyModifiers};
///
/// let kp = KeyPress::from_crossterm(KeyCode::Char('c'), KeyModifiers::CONTROL);
/// assert_eq!(kp.code, KeyCode::Char('c'));
/// assert_eq!(kp.modifiers, KeyModifiers::CONTROL);
/// ```
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct KeyPress {
    /// The key code for this press.
    pub code: KeyCode,
    /// The modifier keys held during this press.
    pub modifiers: KeyModifiers,
}

impl KeyPress {
    /// Creates a `KeyPress` from crossterm key event components.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use patina::keybindings::KeyPress;
    /// use crossterm::event::{KeyCode, KeyModifiers};
    ///
    /// let kp = KeyPress::from_crossterm(KeyCode::Enter, KeyModifiers::NONE);
    /// assert_eq!(kp.code, KeyCode::Enter);
    /// ```
    #[must_use]
    pub fn from_crossterm(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self { code, modifiers }
    }
}

/// A sequence of key presses forming a chord (e.g., Ctrl+X Ctrl+K).
///
/// A chord with a single element is equivalent to a single key binding.
/// Multi-element chords require pressing keys in sequence.
///
/// # Examples
///
/// ```rust
/// use patina::keybindings::{KeyChord, KeyPress};
/// use crossterm::event::{KeyCode, KeyModifiers};
///
/// let chord = KeyChord(vec![
///     KeyPress::from_crossterm(KeyCode::Char('x'), KeyModifiers::CONTROL),
///     KeyPress::from_crossterm(KeyCode::Char('k'), KeyModifiers::CONTROL),
/// ]);
/// assert_eq!(chord.0.len(), 2);
/// ```
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct KeyChord(pub Vec<KeyPress>);

/// Actions that keybindings can trigger.
///
/// Each variant corresponds to a distinct application action. The `Custom`
/// variant allows user-defined actions for plugin or extension use.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Quit the application.
    Quit,
    /// Submit the current input.
    Submit,
    /// Insert a new line in the input.
    NewLine,
    /// Scroll the viewport up.
    ScrollUp,
    /// Scroll the viewport down.
    ScrollDown,
    /// Scroll to the top of the content.
    ScrollToTop,
    /// Scroll to the bottom of the content.
    ScrollToBottom,
    /// Page up in the viewport.
    PageUp,
    /// Page down in the viewport.
    PageDown,
    /// Select all content.
    SelectAll,
    /// Copy selection to clipboard.
    Copy,
    /// Paste from clipboard.
    Paste,
    /// Cancel the current operation.
    CancelOperation,
    /// Focus the content area.
    FocusContent,
    /// Focus the input area.
    FocusInput,
    /// Toggle the help display.
    ToggleHelp,
    /// Kill all background agents.
    KillBackgroundAgents,
    /// Open an external editor.
    OpenEditor,
    /// A user-defined custom action.
    Custom(String),
}

/// Result of resolving a key press against bindings.
///
/// Returned by [`KeybindingManager::resolve`] to indicate whether a key press
/// completed a binding, is a prefix of a chord, or is unbound.
#[derive(Debug, PartialEq)]
pub enum KeyResolution {
    /// Key matched a complete binding.
    Action(Action),
    /// Key matched a prefix of a chord, waiting for more keys.
    Partial,
    /// Key does not match any binding.
    Unbound,
}

/// Manages keybindings with chord support.
///
/// Maintains a map of key chords to actions and a buffer for tracking
/// in-progress chord sequences. Supports loading custom bindings from
/// a JSON configuration file that override defaults.
///
/// # Examples
///
/// ```rust
/// use patina::keybindings::{KeybindingManager, KeyPress, KeyResolution, Action};
/// use crossterm::event::{KeyCode, KeyModifiers};
///
/// let mut mgr = KeybindingManager::with_defaults();
/// let key = KeyPress::from_crossterm(KeyCode::Char('c'), KeyModifiers::CONTROL);
/// assert_eq!(mgr.resolve(key), KeyResolution::Action(Action::Quit));
/// ```
pub struct KeybindingManager {
    bindings: HashMap<KeyChord, Action>,
    chord_buffer: Vec<KeyPress>,
}

impl KeybindingManager {
    /// Creates a manager with default keybindings matching the hardcoded behavior.
    ///
    /// The default bindings cover all currently hardcoded keys in the keyboard handler:
    /// - Ctrl+C, Ctrl+D -> Quit
    /// - Enter -> Submit
    /// - Ctrl+Up, PageUp, Ctrl+K -> ScrollUp
    /// - Ctrl+Down, PageDown, Ctrl+J -> ScrollDown
    /// - Home, Ctrl+G -> ScrollToTop
    /// - End -> ScrollToBottom
    /// - Ctrl+A -> SelectAll
    /// - Ctrl+Shift+C, Super+C, Alt+C -> Copy
    /// - Ctrl+Y -> Copy (yank)
    /// - Ctrl+Shift+V, Super+V, Alt+V -> Paste
    /// - Esc -> CancelOperation
    ///
    /// # Examples
    ///
    /// ```rust
    /// use patina::keybindings::KeybindingManager;
    ///
    /// let mgr = KeybindingManager::with_defaults();
    /// assert!(mgr.binding_count() > 0);
    /// ```
    #[must_use]
    pub fn with_defaults() -> Self {
        let mut bindings = HashMap::new();

        // Exit commands
        bindings.insert(
            KeyChord(vec![KeyPress::from_crossterm(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL,
            )]),
            Action::Quit,
        );
        bindings.insert(
            KeyChord(vec![KeyPress::from_crossterm(
                KeyCode::Char('d'),
                KeyModifiers::CONTROL,
            )]),
            Action::Quit,
        );

        // Submit input
        bindings.insert(
            KeyChord(vec![KeyPress::from_crossterm(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )]),
            Action::Submit,
        );

        // New line
        bindings.insert(
            KeyChord(vec![KeyPress::from_crossterm(
                KeyCode::Enter,
                KeyModifiers::SHIFT,
            )]),
            Action::NewLine,
        );

        // Scroll up: Ctrl+Up, PageUp, Ctrl+K
        bindings.insert(
            KeyChord(vec![KeyPress::from_crossterm(
                KeyCode::Up,
                KeyModifiers::CONTROL,
            )]),
            Action::ScrollUp,
        );
        bindings.insert(
            KeyChord(vec![KeyPress::from_crossterm(
                KeyCode::PageUp,
                KeyModifiers::NONE,
            )]),
            Action::PageUp,
        );
        bindings.insert(
            KeyChord(vec![KeyPress::from_crossterm(
                KeyCode::Char('k'),
                KeyModifiers::CONTROL,
            )]),
            Action::ScrollUp,
        );

        // Scroll down: Ctrl+Down, PageDown, Ctrl+J
        bindings.insert(
            KeyChord(vec![KeyPress::from_crossterm(
                KeyCode::Down,
                KeyModifiers::CONTROL,
            )]),
            Action::ScrollDown,
        );
        bindings.insert(
            KeyChord(vec![KeyPress::from_crossterm(
                KeyCode::PageDown,
                KeyModifiers::NONE,
            )]),
            Action::PageDown,
        );
        bindings.insert(
            KeyChord(vec![KeyPress::from_crossterm(
                KeyCode::Char('j'),
                KeyModifiers::CONTROL,
            )]),
            Action::ScrollDown,
        );

        // Scroll to top: Home, Ctrl+G
        bindings.insert(
            KeyChord(vec![KeyPress::from_crossterm(
                KeyCode::Home,
                KeyModifiers::NONE,
            )]),
            Action::ScrollToTop,
        );
        bindings.insert(
            KeyChord(vec![KeyPress::from_crossterm(
                KeyCode::Char('g'),
                KeyModifiers::CONTROL,
            )]),
            Action::ScrollToTop,
        );

        // Scroll to bottom: End
        bindings.insert(
            KeyChord(vec![KeyPress::from_crossterm(
                KeyCode::End,
                KeyModifiers::NONE,
            )]),
            Action::ScrollToBottom,
        );

        // Select all: Ctrl+A, Super+A, Alt+A, Ctrl+Shift+A
        bindings.insert(
            KeyChord(vec![KeyPress::from_crossterm(
                KeyCode::Char('a'),
                KeyModifiers::CONTROL,
            )]),
            Action::SelectAll,
        );
        bindings.insert(
            KeyChord(vec![KeyPress::from_crossterm(
                KeyCode::Char('a'),
                KeyModifiers::SUPER,
            )]),
            Action::SelectAll,
        );
        bindings.insert(
            KeyChord(vec![KeyPress::from_crossterm(
                KeyCode::Char('a'),
                KeyModifiers::ALT,
            )]),
            Action::SelectAll,
        );
        bindings.insert(
            KeyChord(vec![KeyPress::from_crossterm(
                KeyCode::Char('A'),
                KeyModifiers::from_bits_truncate(
                    KeyModifiers::CONTROL.bits() | KeyModifiers::SHIFT.bits(),
                ),
            )]),
            Action::SelectAll,
        );

        // Copy: Ctrl+Shift+C, Super+C, Alt+C
        bindings.insert(
            KeyChord(vec![KeyPress::from_crossterm(
                KeyCode::Char('C'),
                KeyModifiers::from_bits_truncate(
                    KeyModifiers::CONTROL.bits() | KeyModifiers::SHIFT.bits(),
                ),
            )]),
            Action::Copy,
        );
        bindings.insert(
            KeyChord(vec![KeyPress::from_crossterm(
                KeyCode::Char('c'),
                KeyModifiers::SUPER,
            )]),
            Action::Copy,
        );
        bindings.insert(
            KeyChord(vec![KeyPress::from_crossterm(
                KeyCode::Char('c'),
                KeyModifiers::ALT,
            )]),
            Action::Copy,
        );

        // Copy alternative: Ctrl+Y (yank)
        bindings.insert(
            KeyChord(vec![KeyPress::from_crossterm(
                KeyCode::Char('y'),
                KeyModifiers::CONTROL,
            )]),
            Action::Copy,
        );
        bindings.insert(
            KeyChord(vec![KeyPress::from_crossterm(
                KeyCode::Char('Y'),
                KeyModifiers::CONTROL,
            )]),
            Action::Copy,
        );

        // Paste: Ctrl+Shift+V, Super+V, Alt+V
        bindings.insert(
            KeyChord(vec![KeyPress::from_crossterm(
                KeyCode::Char('V'),
                KeyModifiers::from_bits_truncate(
                    KeyModifiers::CONTROL.bits() | KeyModifiers::SHIFT.bits(),
                ),
            )]),
            Action::Paste,
        );
        bindings.insert(
            KeyChord(vec![KeyPress::from_crossterm(
                KeyCode::Char('v'),
                KeyModifiers::SUPER,
            )]),
            Action::Paste,
        );
        bindings.insert(
            KeyChord(vec![KeyPress::from_crossterm(
                KeyCode::Char('v'),
                KeyModifiers::ALT,
            )]),
            Action::Paste,
        );

        // Cancel / clear selection: Esc
        bindings.insert(
            KeyChord(vec![KeyPress::from_crossterm(
                KeyCode::Esc,
                KeyModifiers::NONE,
            )]),
            Action::CancelOperation,
        );

        Self {
            bindings,
            chord_buffer: Vec::new(),
        }
    }

    /// Loads keybindings from a JSON file, merging with defaults.
    ///
    /// User bindings in the file override default bindings for the same key.
    /// Bindings not specified in the file retain their default values.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or contains invalid JSON.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use patina::keybindings::KeybindingManager;
    /// use std::path::Path;
    ///
    /// let mgr = KeybindingManager::load_from_file(Path::new("keybindings.json"))?;
    /// ```
    pub fn load_from_file(path: &Path) -> Result<Self> {
        let content =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let config: KeybindingConfig =
            serde_json::from_str(&content).context("parsing keybinding config")?;

        let mut mgr = Self::with_defaults();

        for (key_str, action_str) in &config.bindings {
            let chord = parse_chord_string(key_str)?;
            let action: Action =
                serde_json::from_value(serde_json::Value::String(action_str.clone()))
                    .with_context(|| format!("parsing action '{action_str}'"))?;
            mgr.bindings.insert(chord, action);
        }

        Ok(mgr)
    }

    /// Resolves a key press against the current bindings.
    ///
    /// Appends the key to the chord buffer, then checks:
    /// 1. If the buffer matches a complete binding, returns [`KeyResolution::Action`]
    ///    and clears the buffer.
    /// 2. If the buffer is a prefix of any binding, returns [`KeyResolution::Partial`].
    /// 3. Otherwise, clears the buffer and returns [`KeyResolution::Unbound`].
    ///
    /// # Examples
    ///
    /// ```rust
    /// use patina::keybindings::{KeybindingManager, KeyPress, KeyResolution, Action};
    /// use crossterm::event::{KeyCode, KeyModifiers};
    ///
    /// let mut mgr = KeybindingManager::with_defaults();
    /// let key = KeyPress::from_crossterm(KeyCode::Char('c'), KeyModifiers::CONTROL);
    /// assert_eq!(mgr.resolve(key), KeyResolution::Action(Action::Quit));
    /// ```
    pub fn resolve(&mut self, key: KeyPress) -> KeyResolution {
        self.chord_buffer.push(key);

        let current_chord = KeyChord(self.chord_buffer.clone());

        // Check for exact match
        if let Some(action) = self.bindings.get(&current_chord) {
            let action = action.clone();
            self.chord_buffer.clear();
            return KeyResolution::Action(action);
        }

        // Check for prefix match
        let is_prefix = self.bindings.keys().any(|chord| {
            chord.0.len() > self.chord_buffer.len()
                && chord.0[..self.chord_buffer.len()] == self.chord_buffer[..]
        });

        if is_prefix {
            return KeyResolution::Partial;
        }

        // No match
        self.chord_buffer.clear();
        KeyResolution::Unbound
    }

    /// Clears any pending chord state.
    ///
    /// Call this when a timeout or mode change should reset the chord buffer.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use patina::keybindings::{KeybindingManager, KeyPress};
    /// use crossterm::event::{KeyCode, KeyModifiers};
    ///
    /// let mut mgr = KeybindingManager::with_defaults();
    /// mgr.clear_chord();
    /// ```
    pub fn clear_chord(&mut self) {
        self.chord_buffer.clear();
    }

    /// Returns the number of registered bindings.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use patina::keybindings::KeybindingManager;
    ///
    /// let mgr = KeybindingManager::with_defaults();
    /// assert!(mgr.binding_count() > 0);
    /// ```
    #[must_use]
    pub fn binding_count(&self) -> usize {
        self.bindings.len()
    }

    /// Returns `true` if the chord buffer is currently empty.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use patina::keybindings::KeybindingManager;
    ///
    /// let mgr = KeybindingManager::with_defaults();
    /// assert!(mgr.chord_buffer_empty());
    /// ```
    #[must_use]
    pub fn chord_buffer_empty(&self) -> bool {
        self.chord_buffer.is_empty()
    }
}

/// JSON configuration format for keybindings.
#[derive(Debug, Deserialize)]
struct KeybindingConfig {
    bindings: HashMap<String, String>,
}

/// Parses a key string like "ctrl+c" into a `KeyPress`.
///
/// Supports modifiers: `ctrl`, `shift`, `alt`/`option`, `super`/`cmd`.
/// The key name follows the last `+` separator.
///
/// # Errors
///
/// Returns an error if the key string contains an unknown modifier or key name.
///
/// # Examples
///
/// ```rust
/// use patina::keybindings::parse_key_string;
/// use crossterm::event::{KeyCode, KeyModifiers};
///
/// let kp = parse_key_string("ctrl+c").unwrap();
/// assert_eq!(kp.code, KeyCode::Char('c'));
/// assert_eq!(kp.modifiers, KeyModifiers::CONTROL);
/// ```
pub fn parse_key_string(s: &str) -> Result<KeyPress> {
    let parts: Vec<&str> = s.split('+').collect();
    let mut modifiers = KeyModifiers::NONE;

    // All parts except the last are modifiers
    for &part in &parts[..parts.len() - 1] {
        match part.to_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= KeyModifiers::CONTROL,
            "shift" => modifiers |= KeyModifiers::SHIFT,
            "alt" | "option" => modifiers |= KeyModifiers::ALT,
            "super" | "cmd" | "command" => modifiers |= KeyModifiers::SUPER,
            other => anyhow::bail!("unknown modifier: '{other}'"),
        }
    }

    // The last part is the key name
    let key_name = parts
        .last()
        .ok_or_else(|| anyhow::anyhow!("empty key string"))?;

    let code = parse_key_code(key_name)?;

    Ok(KeyPress { code, modifiers })
}

/// Parses a chord string like "ctrl+x ctrl+k" into a `KeyChord`.
///
/// Each space-separated segment is parsed as a single key press.
///
/// # Errors
///
/// Returns an error if any segment contains an unknown modifier or key name.
///
/// # Examples
///
/// ```rust
/// use patina::keybindings::parse_chord_string;
///
/// let chord = parse_chord_string("ctrl+x ctrl+k").unwrap();
/// assert_eq!(chord.0.len(), 2);
/// ```
pub fn parse_chord_string(s: &str) -> Result<KeyChord> {
    let keys: Result<Vec<KeyPress>> = s.split_whitespace().map(parse_key_string).collect();
    let keys = keys?;
    if keys.is_empty() {
        anyhow::bail!("empty chord string");
    }
    Ok(KeyChord(keys))
}

/// Parses a key name string into a `KeyCode`.
///
/// # Errors
///
/// Returns an error if the key name is not recognized.
fn parse_key_code(name: &str) -> Result<KeyCode> {
    match name.to_lowercase().as_str() {
        "enter" | "return" => Ok(KeyCode::Enter),
        "esc" | "escape" => Ok(KeyCode::Esc),
        "backspace" => Ok(KeyCode::Backspace),
        "tab" => Ok(KeyCode::Tab),
        "space" | " " => Ok(KeyCode::Char(' ')),
        "up" => Ok(KeyCode::Up),
        "down" => Ok(KeyCode::Down),
        "left" => Ok(KeyCode::Left),
        "right" => Ok(KeyCode::Right),
        "home" => Ok(KeyCode::Home),
        "end" => Ok(KeyCode::End),
        "pageup" => Ok(KeyCode::PageUp),
        "pagedown" => Ok(KeyCode::PageDown),
        "delete" | "del" => Ok(KeyCode::Delete),
        "insert" | "ins" => Ok(KeyCode::Insert),
        s if s.len() == 1 => {
            let c = s.chars().next().expect("single-char string has a char");
            Ok(KeyCode::Char(c))
        }
        other => anyhow::bail!("unknown key: '{other}'"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};
    use std::io::Write;
    use tempfile::NamedTempFile;

    // =========================================================================
    // KeyPress::from_crossterm
    // =========================================================================

    #[test]
    fn keypress_from_crossterm_ctrl_c() {
        let kp = KeyPress::from_crossterm(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(kp.code, KeyCode::Char('c'));
        assert_eq!(kp.modifiers, KeyModifiers::CONTROL);
    }

    #[test]
    fn keypress_from_crossterm_enter() {
        let kp = KeyPress::from_crossterm(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(kp.code, KeyCode::Enter);
        assert_eq!(kp.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn keypress_from_crossterm_shift_enter() {
        let kp = KeyPress::from_crossterm(KeyCode::Enter, KeyModifiers::SHIFT);
        assert_eq!(kp.code, KeyCode::Enter);
        assert_eq!(kp.modifiers, KeyModifiers::SHIFT);
    }

    // =========================================================================
    // parse_key_string
    // =========================================================================

    #[test]
    fn parse_key_string_ctrl_c() {
        let kp = parse_key_string("ctrl+c").unwrap();
        assert_eq!(kp.code, KeyCode::Char('c'));
        assert_eq!(kp.modifiers, KeyModifiers::CONTROL);
    }

    #[test]
    fn parse_key_string_shift_enter() {
        let kp = parse_key_string("shift+enter").unwrap();
        assert_eq!(kp.code, KeyCode::Enter);
        assert_eq!(kp.modifiers, KeyModifiers::SHIFT);
    }

    #[test]
    fn parse_key_string_ctrl_shift_c() {
        let kp = parse_key_string("ctrl+shift+c").unwrap();
        assert_eq!(kp.code, KeyCode::Char('c'));
        assert_eq!(kp.modifiers, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    }

    #[test]
    fn parse_key_string_enter() {
        let kp = parse_key_string("enter").unwrap();
        assert_eq!(kp.code, KeyCode::Enter);
        assert_eq!(kp.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn parse_key_string_esc() {
        let kp = parse_key_string("esc").unwrap();
        assert_eq!(kp.code, KeyCode::Esc);
        assert_eq!(kp.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn parse_key_string_super_a() {
        let kp = parse_key_string("super+a").unwrap();
        assert_eq!(kp.code, KeyCode::Char('a'));
        assert_eq!(kp.modifiers, KeyModifiers::SUPER);
    }

    #[test]
    fn parse_key_string_alt_v() {
        let kp = parse_key_string("alt+v").unwrap();
        assert_eq!(kp.code, KeyCode::Char('v'));
        assert_eq!(kp.modifiers, KeyModifiers::ALT);
    }

    #[test]
    fn parse_key_string_unknown_modifier_errors() {
        assert!(parse_key_string("hyper+c").is_err());
    }

    #[test]
    fn parse_key_string_unknown_key_errors() {
        assert!(parse_key_string("ctrl+unknownkey").is_err());
    }

    // =========================================================================
    // parse_chord_string
    // =========================================================================

    #[test]
    fn parse_chord_string_single_key() {
        let chord = parse_chord_string("ctrl+c").unwrap();
        assert_eq!(chord.0.len(), 1);
        assert_eq!(chord.0[0].code, KeyCode::Char('c'));
        assert_eq!(chord.0[0].modifiers, KeyModifiers::CONTROL);
    }

    #[test]
    fn parse_chord_string_two_keys() {
        let chord = parse_chord_string("ctrl+x ctrl+k").unwrap();
        assert_eq!(chord.0.len(), 2);
        assert_eq!(chord.0[0].code, KeyCode::Char('x'));
        assert_eq!(chord.0[0].modifiers, KeyModifiers::CONTROL);
        assert_eq!(chord.0[1].code, KeyCode::Char('k'));
        assert_eq!(chord.0[1].modifiers, KeyModifiers::CONTROL);
    }

    #[test]
    fn parse_chord_string_empty_errors() {
        assert!(parse_chord_string("").is_err());
    }

    // =========================================================================
    // Default keybindings include all hardcoded keys
    // =========================================================================

    #[test]
    fn defaults_include_ctrl_c_quit() {
        let mut mgr = KeybindingManager::with_defaults();
        let key = KeyPress::from_crossterm(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(mgr.resolve(key), KeyResolution::Action(Action::Quit));
    }

    #[test]
    fn defaults_include_ctrl_d_quit() {
        let mut mgr = KeybindingManager::with_defaults();
        let key = KeyPress::from_crossterm(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(mgr.resolve(key), KeyResolution::Action(Action::Quit));
    }

    #[test]
    fn defaults_include_enter_submit() {
        let mut mgr = KeybindingManager::with_defaults();
        let key = KeyPress::from_crossterm(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(mgr.resolve(key), KeyResolution::Action(Action::Submit));
    }

    #[test]
    fn defaults_include_ctrl_up_scroll_up() {
        let mut mgr = KeybindingManager::with_defaults();
        let key = KeyPress::from_crossterm(KeyCode::Up, KeyModifiers::CONTROL);
        assert_eq!(mgr.resolve(key), KeyResolution::Action(Action::ScrollUp));
    }

    #[test]
    fn defaults_include_page_up() {
        let mut mgr = KeybindingManager::with_defaults();
        let key = KeyPress::from_crossterm(KeyCode::PageUp, KeyModifiers::NONE);
        assert_eq!(mgr.resolve(key), KeyResolution::Action(Action::PageUp));
    }

    #[test]
    fn defaults_include_ctrl_k_scroll_up() {
        let mut mgr = KeybindingManager::with_defaults();
        let key = KeyPress::from_crossterm(KeyCode::Char('k'), KeyModifiers::CONTROL);
        assert_eq!(mgr.resolve(key), KeyResolution::Action(Action::ScrollUp));
    }

    #[test]
    fn defaults_include_ctrl_down_scroll_down() {
        let mut mgr = KeybindingManager::with_defaults();
        let key = KeyPress::from_crossterm(KeyCode::Down, KeyModifiers::CONTROL);
        assert_eq!(mgr.resolve(key), KeyResolution::Action(Action::ScrollDown));
    }

    #[test]
    fn defaults_include_page_down() {
        let mut mgr = KeybindingManager::with_defaults();
        let key = KeyPress::from_crossterm(KeyCode::PageDown, KeyModifiers::NONE);
        assert_eq!(mgr.resolve(key), KeyResolution::Action(Action::PageDown));
    }

    #[test]
    fn defaults_include_ctrl_j_scroll_down() {
        let mut mgr = KeybindingManager::with_defaults();
        let key = KeyPress::from_crossterm(KeyCode::Char('j'), KeyModifiers::CONTROL);
        assert_eq!(mgr.resolve(key), KeyResolution::Action(Action::ScrollDown));
    }

    #[test]
    fn defaults_include_home_scroll_to_top() {
        let mut mgr = KeybindingManager::with_defaults();
        let key = KeyPress::from_crossterm(KeyCode::Home, KeyModifiers::NONE);
        assert_eq!(mgr.resolve(key), KeyResolution::Action(Action::ScrollToTop));
    }

    #[test]
    fn defaults_include_ctrl_g_scroll_to_top() {
        let mut mgr = KeybindingManager::with_defaults();
        let key = KeyPress::from_crossterm(KeyCode::Char('g'), KeyModifiers::CONTROL);
        assert_eq!(mgr.resolve(key), KeyResolution::Action(Action::ScrollToTop));
    }

    #[test]
    fn defaults_include_end_scroll_to_bottom() {
        let mut mgr = KeybindingManager::with_defaults();
        let key = KeyPress::from_crossterm(KeyCode::End, KeyModifiers::NONE);
        assert_eq!(
            mgr.resolve(key),
            KeyResolution::Action(Action::ScrollToBottom)
        );
    }

    #[test]
    fn defaults_include_ctrl_a_select_all() {
        let mut mgr = KeybindingManager::with_defaults();
        let key = KeyPress::from_crossterm(KeyCode::Char('a'), KeyModifiers::CONTROL);
        assert_eq!(mgr.resolve(key), KeyResolution::Action(Action::SelectAll));
    }

    #[test]
    fn defaults_include_super_a_select_all() {
        let mut mgr = KeybindingManager::with_defaults();
        let key = KeyPress::from_crossterm(KeyCode::Char('a'), KeyModifiers::SUPER);
        assert_eq!(mgr.resolve(key), KeyResolution::Action(Action::SelectAll));
    }

    #[test]
    fn defaults_include_alt_a_select_all() {
        let mut mgr = KeybindingManager::with_defaults();
        let key = KeyPress::from_crossterm(KeyCode::Char('a'), KeyModifiers::ALT);
        assert_eq!(mgr.resolve(key), KeyResolution::Action(Action::SelectAll));
    }

    #[test]
    fn defaults_include_ctrl_shift_c_copy() {
        let mut mgr = KeybindingManager::with_defaults();
        let key = KeyPress::from_crossterm(
            KeyCode::Char('C'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert_eq!(mgr.resolve(key), KeyResolution::Action(Action::Copy));
    }

    #[test]
    fn defaults_include_super_c_copy() {
        let mut mgr = KeybindingManager::with_defaults();
        let key = KeyPress::from_crossterm(KeyCode::Char('c'), KeyModifiers::SUPER);
        assert_eq!(mgr.resolve(key), KeyResolution::Action(Action::Copy));
    }

    #[test]
    fn defaults_include_ctrl_y_copy() {
        let mut mgr = KeybindingManager::with_defaults();
        let key = KeyPress::from_crossterm(KeyCode::Char('y'), KeyModifiers::CONTROL);
        assert_eq!(mgr.resolve(key), KeyResolution::Action(Action::Copy));
    }

    #[test]
    fn defaults_include_ctrl_shift_v_paste() {
        let mut mgr = KeybindingManager::with_defaults();
        let key = KeyPress::from_crossterm(
            KeyCode::Char('V'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert_eq!(mgr.resolve(key), KeyResolution::Action(Action::Paste));
    }

    #[test]
    fn defaults_include_super_v_paste() {
        let mut mgr = KeybindingManager::with_defaults();
        let key = KeyPress::from_crossterm(KeyCode::Char('v'), KeyModifiers::SUPER);
        assert_eq!(mgr.resolve(key), KeyResolution::Action(Action::Paste));
    }

    #[test]
    fn defaults_include_esc_cancel() {
        let mut mgr = KeybindingManager::with_defaults();
        let key = KeyPress::from_crossterm(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(
            mgr.resolve(key),
            KeyResolution::Action(Action::CancelOperation)
        );
    }

    // =========================================================================
    // resolve: single key match returns Action
    // =========================================================================

    #[test]
    fn resolve_single_key_returns_action() {
        let mut mgr = KeybindingManager::with_defaults();
        let key = KeyPress::from_crossterm(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(mgr.resolve(key), KeyResolution::Action(Action::Quit));
    }

    // =========================================================================
    // resolve: unbound key returns Unbound
    // =========================================================================

    #[test]
    fn resolve_unbound_key_returns_unbound() {
        let mut mgr = KeybindingManager::with_defaults();
        let key = KeyPress::from_crossterm(KeyCode::F(12), KeyModifiers::NONE);
        assert_eq!(mgr.resolve(key), KeyResolution::Unbound);
    }

    // =========================================================================
    // resolve: chord sequence
    // =========================================================================

    #[test]
    fn resolve_chord_first_key_returns_partial_second_returns_action() {
        let mut mgr = KeybindingManager::with_defaults();

        // Add a chord binding: Ctrl+X Ctrl+K -> KillBackgroundAgents
        mgr.bindings.insert(
            KeyChord(vec![
                KeyPress::from_crossterm(KeyCode::Char('x'), KeyModifiers::CONTROL),
                KeyPress::from_crossterm(KeyCode::Char('k'), KeyModifiers::CONTROL),
            ]),
            Action::KillBackgroundAgents,
        );
        // Remove the single-key binding for Ctrl+K that would conflict
        mgr.bindings.remove(&KeyChord(vec![KeyPress::from_crossterm(
            KeyCode::Char('k'),
            KeyModifiers::CONTROL,
        )]));

        let key1 = KeyPress::from_crossterm(KeyCode::Char('x'), KeyModifiers::CONTROL);
        assert_eq!(mgr.resolve(key1), KeyResolution::Partial);

        let key2 = KeyPress::from_crossterm(KeyCode::Char('k'), KeyModifiers::CONTROL);
        assert_eq!(
            mgr.resolve(key2),
            KeyResolution::Action(Action::KillBackgroundAgents)
        );
    }

    #[test]
    fn resolve_chord_wrong_second_key_returns_unbound() {
        let mut mgr = KeybindingManager::with_defaults();

        // Add a chord binding: Ctrl+X Ctrl+K -> KillBackgroundAgents
        mgr.bindings.insert(
            KeyChord(vec![
                KeyPress::from_crossterm(KeyCode::Char('x'), KeyModifiers::CONTROL),
                KeyPress::from_crossterm(KeyCode::Char('k'), KeyModifiers::CONTROL),
            ]),
            Action::KillBackgroundAgents,
        );

        let key1 = KeyPress::from_crossterm(KeyCode::Char('x'), KeyModifiers::CONTROL);
        assert_eq!(mgr.resolve(key1), KeyResolution::Partial);

        let key2 = KeyPress::from_crossterm(KeyCode::F(12), KeyModifiers::NONE);
        assert_eq!(mgr.resolve(key2), KeyResolution::Unbound);
    }

    // =========================================================================
    // clear_chord resets buffer
    // =========================================================================

    #[test]
    fn clear_chord_resets_buffer() {
        let mut mgr = KeybindingManager::with_defaults();

        // Add a chord binding so first key returns Partial
        mgr.bindings.insert(
            KeyChord(vec![
                KeyPress::from_crossterm(KeyCode::Char('x'), KeyModifiers::CONTROL),
                KeyPress::from_crossterm(KeyCode::Char('k'), KeyModifiers::CONTROL),
            ]),
            Action::KillBackgroundAgents,
        );

        let key1 = KeyPress::from_crossterm(KeyCode::Char('x'), KeyModifiers::CONTROL);
        assert_eq!(mgr.resolve(key1), KeyResolution::Partial);
        assert!(!mgr.chord_buffer_empty());

        mgr.clear_chord();
        assert!(mgr.chord_buffer_empty());
    }

    // =========================================================================
    // load_from_file: user bindings override defaults
    // =========================================================================

    #[test]
    fn load_from_file_overrides_defaults() {
        let config = r#"{
            "bindings": {
                "ctrl+c": "cancel_operation",
                "ctrl+x ctrl+k": "kill_background_agents"
            }
        }"#;

        let mut tmpfile = NamedTempFile::new().unwrap();
        tmpfile.write_all(config.as_bytes()).unwrap();
        tmpfile.flush().unwrap();

        let mgr = KeybindingManager::load_from_file(tmpfile.path()).unwrap();

        // Ctrl+C should now be CancelOperation instead of Quit
        let mut mgr = mgr;
        let key = KeyPress::from_crossterm(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(
            mgr.resolve(key),
            KeyResolution::Action(Action::CancelOperation)
        );
    }

    #[test]
    fn load_from_file_preserves_non_overridden_defaults() {
        let config = r#"{
            "bindings": {
                "ctrl+c": "cancel_operation"
            }
        }"#;

        let mut tmpfile = NamedTempFile::new().unwrap();
        tmpfile.write_all(config.as_bytes()).unwrap();
        tmpfile.flush().unwrap();

        let mut mgr = KeybindingManager::load_from_file(tmpfile.path()).unwrap();

        // Ctrl+D should still be Quit (not overridden)
        let key = KeyPress::from_crossterm(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(mgr.resolve(key), KeyResolution::Action(Action::Quit));
    }

    // =========================================================================
    // JSON deserialization of Action
    // =========================================================================

    #[test]
    fn action_deserialize_quit() {
        let action: Action = serde_json::from_str("\"quit\"").unwrap();
        assert_eq!(action, Action::Quit);
    }

    #[test]
    fn action_deserialize_submit() {
        let action: Action = serde_json::from_str("\"submit\"").unwrap();
        assert_eq!(action, Action::Submit);
    }

    #[test]
    fn action_deserialize_scroll_up() {
        let action: Action = serde_json::from_str("\"scroll_up\"").unwrap();
        assert_eq!(action, Action::ScrollUp);
    }

    #[test]
    fn action_deserialize_kill_background_agents() {
        let action: Action = serde_json::from_str("\"kill_background_agents\"").unwrap();
        assert_eq!(action, Action::KillBackgroundAgents);
    }

    #[test]
    fn action_deserialize_custom() {
        let action: Action = serde_json::from_str("{\"custom\": \"my_action\"}").unwrap();
        assert_eq!(action, Action::Custom("my_action".to_string()));
    }

    // =========================================================================
    // JSON config deserialization
    // =========================================================================

    #[test]
    fn json_config_deserializes() {
        let json = r#"{
            "bindings": {
                "ctrl+c": "quit",
                "ctrl+d": "quit",
                "enter": "submit",
                "shift+enter": "new_line"
            }
        }"#;

        let config: KeybindingConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.bindings.len(), 4);
        assert_eq!(config.bindings.get("ctrl+c").unwrap(), "quit");
    }

    // =========================================================================
    // load_from_file: invalid file returns error
    // =========================================================================

    #[test]
    fn load_from_file_nonexistent_returns_error() {
        let result = KeybindingManager::load_from_file(Path::new("/nonexistent/path.json"));
        assert!(result.is_err());
    }

    #[test]
    fn load_from_file_invalid_json_returns_error() {
        let mut tmpfile = NamedTempFile::new().unwrap();
        tmpfile.write_all(b"not valid json").unwrap();
        tmpfile.flush().unwrap();

        let result = KeybindingManager::load_from_file(tmpfile.path());
        assert!(result.is_err());
    }

    // =========================================================================
    // binding_count
    // =========================================================================

    #[test]
    fn binding_count_is_nonzero_for_defaults() {
        let mgr = KeybindingManager::with_defaults();
        assert!(
            mgr.binding_count() > 10,
            "defaults should have many bindings, got {}",
            mgr.binding_count()
        );
    }

    // =========================================================================
    // Chord buffer state
    // =========================================================================

    #[test]
    fn chord_buffer_starts_empty() {
        let mgr = KeybindingManager::with_defaults();
        assert!(mgr.chord_buffer_empty());
    }

    #[test]
    fn resolve_action_clears_chord_buffer() {
        let mut mgr = KeybindingManager::with_defaults();
        let key = KeyPress::from_crossterm(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let _ = mgr.resolve(key);
        assert!(mgr.chord_buffer_empty());
    }

    #[test]
    fn resolve_unbound_clears_chord_buffer() {
        let mut mgr = KeybindingManager::with_defaults();
        let key = KeyPress::from_crossterm(KeyCode::F(12), KeyModifiers::NONE);
        let _ = mgr.resolve(key);
        assert!(mgr.chord_buffer_empty());
    }

    // =========================================================================
    // Load from file with chord bindings
    // =========================================================================

    #[test]
    fn load_from_file_chord_binding_works() {
        let config = r#"{
            "bindings": {
                "ctrl+x ctrl+k": "kill_background_agents"
            }
        }"#;

        let mut tmpfile = NamedTempFile::new().unwrap();
        tmpfile.write_all(config.as_bytes()).unwrap();
        tmpfile.flush().unwrap();

        let mut mgr = KeybindingManager::load_from_file(tmpfile.path()).unwrap();

        let key1 = KeyPress::from_crossterm(KeyCode::Char('x'), KeyModifiers::CONTROL);
        assert_eq!(mgr.resolve(key1), KeyResolution::Partial);

        let key2 = KeyPress::from_crossterm(KeyCode::Char('k'), KeyModifiers::CONTROL);
        assert_eq!(
            mgr.resolve(key2),
            KeyResolution::Action(Action::KillBackgroundAgents)
        );
    }
}
