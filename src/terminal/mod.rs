//! Terminal configuration and key binding setup.
//!
//! Automatically configures terminal emulators to pass through Cmd+A/C/V
//! to the application instead of handling them as terminal shortcuts.

use std::env;
use std::process::Command;
use tracing::{debug, info, warn};

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
}
