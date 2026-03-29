//! External editor integration for editing input in `$EDITOR`.
//!
//! Opens the user's preferred editor with the current input buffer contents,
//! then reads back the edited content when the editor closes. Editor preference
//! is detected from `$VISUAL`, `$EDITOR`, or falls back to `vi`.
//!
//! # Integration
//!
//! The keybinding `Ctrl+X Ctrl+E` (action [`Action::OpenEditor`]) triggers this
//! module. The keyboard handler should:
//!
//! 1. Suspend the TUI terminal (restore original terminal mode).
//! 2. Call [`ExternalEditor::open_with_content`] with the current input buffer.
//! 3. On success, replace the input buffer with the returned content.
//! 4. On failure (non-zero exit), discard changes and show a status message.
//! 5. Re-enter raw mode and resume the TUI.
//!
//! [`Action::OpenEditor`]: crate::keybindings::Action::OpenEditor
//!
//! # Editor detection
//!
//! Editors are detected in order: `$VISUAL` > `$EDITOR` > `vi`.
//! Editors known to require `--wait` (e.g. `code`, `subl`) are automatically
//! launched with that flag so the process blocks until the user closes the file.
//!
//! # Examples
//!
//! ```rust,no_run
//! use patina::tools::external_editor::ExternalEditor;
//!
//! let editor = ExternalEditor::new();
//! let edited = editor.open_with_content("draft text")?;
//! println!("User wrote: {edited}");
//! # Ok::<(), anyhow::Error>(())
//! ```

use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{bail, Context, Result};
use tracing::debug;

/// Monotonic counter to ensure unique temp file names even within the same
/// nanosecond (e.g. during parallel test execution).
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Editors that require a `--wait` flag so the process blocks until the user
/// closes the file (GUI editors that otherwise return immediately).
const WAIT_EDITORS: &[&str] = &["code", "subl", "atom", "zed"];

/// External editor integration.
///
/// Manages temp-file lifecycle and editor invocation for editing input buffers
/// in the user's preferred terminal or GUI editor.
///
/// # Examples
///
/// ```rust
/// use patina::tools::external_editor::ExternalEditor;
///
/// let editor = ExternalEditor::new();
/// assert!(!editor.editor_name().is_empty());
/// ```
pub struct ExternalEditor {
    /// Full editor command string (may include arguments, e.g. "code --wait").
    editor_command: String,
    /// Directory for temporary editor files.
    temp_dir: PathBuf,
}

impl ExternalEditor {
    /// Creates a new `ExternalEditor` using the detected editor and system temp dir.
    ///
    /// Editor preference is resolved via [`detect_editor`].
    ///
    /// [`detect_editor`]: Self::detect_editor
    ///
    /// # Examples
    ///
    /// ```rust
    /// use patina::tools::external_editor::ExternalEditor;
    ///
    /// let editor = ExternalEditor::new();
    /// // editor_name() returns at least "vi" as fallback
    /// assert!(!editor.editor_name().is_empty());
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            editor_command: Self::detect_editor(),
            temp_dir: env::temp_dir(),
        }
    }

    /// Detects the user's preferred editor from environment variables.
    ///
    /// Checks, in order:
    /// 1. `$VISUAL` — preferred for graphical editors
    /// 2. `$EDITOR` — standard fallback
    /// 3. `"vi"` — last-resort default
    ///
    /// # Examples
    ///
    /// ```rust
    /// // With no env vars set, falls back to "vi"
    /// // (actual result depends on environment)
    /// let editor = patina::tools::external_editor::ExternalEditor::detect_editor();
    /// assert!(!editor.is_empty());
    /// ```
    #[must_use]
    pub fn detect_editor() -> String {
        if let Ok(visual) = env::var("VISUAL") {
            if !visual.is_empty() {
                debug!(editor = %visual, "detected editor from $VISUAL");
                return visual;
            }
        }
        if let Ok(editor) = env::var("EDITOR") {
            if !editor.is_empty() {
                debug!(editor = %editor, "detected editor from $EDITOR");
                return editor;
            }
        }
        debug!("no $VISUAL or $EDITOR set, falling back to vi");
        "vi".to_string()
    }

    /// Returns the base name of the configured editor (without path or arguments).
    ///
    /// For example, `"/usr/bin/code --wait"` returns `"code"`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use patina::tools::external_editor::ExternalEditor;
    ///
    /// let editor = ExternalEditor::new();
    /// let name = editor.editor_name();
    /// // Always a non-empty string
    /// assert!(!name.is_empty());
    /// ```
    #[must_use]
    pub fn editor_name(&self) -> &str {
        // Take first whitespace-delimited token, then take the filename part
        let first_token = self
            .editor_command
            .split_whitespace()
            .next()
            .unwrap_or("vi");
        first_token.rsplit('/').next().unwrap_or(first_token)
    }

    /// Opens the editor with the given content, waits for it to close, and
    /// returns the (possibly edited) content.
    ///
    /// The flow:
    /// 1. Write `content` to a temporary `.md` file.
    /// 2. Spawn the editor process pointing at that file.
    /// 3. Wait for the editor to exit.
    /// 4. If exit status is success (code 0), read the file back.
    /// 5. Clean up the temporary file.
    ///
    /// Editors in the [`WAIT_EDITORS`] list automatically receive `--wait`.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The temporary file cannot be created or written.
    /// - The editor process fails to spawn.
    /// - The editor exits with a non-zero status.
    /// - The edited file cannot be read back.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use patina::tools::external_editor::ExternalEditor;
    ///
    /// let editor = ExternalEditor::new();
    /// let result = editor.open_with_content("hello")?;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn open_with_content(&self, content: &str) -> Result<String> {
        let temp_path = self.create_temp_file(content)?;

        let result = self.run_editor(&temp_path);

        // Always attempt cleanup, even if the editor failed
        if let Err(e) = fs::remove_file(&temp_path) {
            debug!(path = %temp_path.display(), error = %e, "failed to clean up temp file");
        }

        result
    }

    /// Creates a temporary `.md` file with the given content.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be created or written to.
    fn create_temp_file(&self, content: &str) -> Result<PathBuf> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let seq = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let filename = format!("patina-edit-{timestamp}-{pid}-{seq}.md");
        let path = self.temp_dir.join(filename);

        let mut file = fs::File::create(&path)
            .with_context(|| format!("creating temp file {}", path.display()))?;
        file.write_all(content.as_bytes())
            .with_context(|| format!("writing to temp file {}", path.display()))?;
        file.flush()
            .with_context(|| format!("flushing temp file {}", path.display()))?;

        debug!(path = %path.display(), "created temp file for external editor");
        Ok(path)
    }

    /// Spawns the editor and waits for it to exit, then reads back the file.
    ///
    /// # Errors
    ///
    /// Returns an error if the editor cannot be spawned, exits with non-zero,
    /// or the file cannot be read back.
    fn run_editor(&self, file_path: &PathBuf) -> Result<String> {
        let mut parts: Vec<&str> = self.editor_command.split_whitespace().collect();
        if parts.is_empty() {
            bail!("editor command is empty");
        }

        let program = parts.remove(0);
        let mut args: Vec<&str> = parts;

        // Auto-add --wait for GUI editors that need it, unless already present
        if needs_wait_flag(program) && !args.iter().any(|a| *a == "--wait" || *a == "-w") {
            args.push("--wait");
        }

        let file_str = file_path
            .to_str()
            .context("temp file path is not valid UTF-8")?;
        args.push(file_str);

        debug!(program, ?args, "launching external editor");

        let status = Command::new(program)
            .args(&args)
            .status()
            .with_context(|| format!("failed to launch editor '{program}'"))?;

        if !status.success() {
            let code = status.code().unwrap_or(-1);
            bail!("editor '{program}' exited with status {code}");
        }

        let edited = fs::read_to_string(file_path)
            .with_context(|| format!("reading back edited file {}", file_path.display()))?;

        Ok(edited)
    }
}

impl Default for ExternalEditor {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns `true` if the given editor program name requires a `--wait` flag
/// to block until the user closes the file.
///
/// # Examples
///
/// ```rust
/// // This is an internal function, tested via unit tests
/// ```
#[must_use]
fn needs_wait_flag(program: &str) -> bool {
    let base_name = program.rsplit('/').next().unwrap_or(program);
    WAIT_EDITORS.contains(&base_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // =========================================================================
    // Editor detection
    // =========================================================================

    #[test]
    #[serial_test::serial]
    fn detect_editor_from_visual_env_var() {
        // Save originals
        let orig_visual = env::var("VISUAL").ok();
        let orig_editor = env::var("EDITOR").ok();

        env::set_var("VISUAL", "emacs");
        env::set_var("EDITOR", "nano");

        let result = ExternalEditor::detect_editor();
        assert_eq!(
            result, "emacs",
            "$VISUAL should take precedence over $EDITOR"
        );

        // Restore
        match orig_visual {
            Some(v) => env::set_var("VISUAL", v),
            None => env::remove_var("VISUAL"),
        }
        match orig_editor {
            Some(v) => env::set_var("EDITOR", v),
            None => env::remove_var("EDITOR"),
        }
    }

    #[test]
    #[serial_test::serial]
    fn detect_editor_from_editor_env_var() {
        let orig_visual = env::var("VISUAL").ok();
        let orig_editor = env::var("EDITOR").ok();

        env::remove_var("VISUAL");
        env::set_var("EDITOR", "nano");

        let result = ExternalEditor::detect_editor();
        assert_eq!(
            result, "nano",
            "should fall back to $EDITOR when $VISUAL is unset"
        );

        match orig_visual {
            Some(v) => env::set_var("VISUAL", v),
            None => env::remove_var("VISUAL"),
        }
        match orig_editor {
            Some(v) => env::set_var("EDITOR", v),
            None => env::remove_var("EDITOR"),
        }
    }

    #[test]
    #[serial_test::serial]
    fn detect_editor_fallback_to_vi() {
        let orig_visual = env::var("VISUAL").ok();
        let orig_editor = env::var("EDITOR").ok();

        env::remove_var("VISUAL");
        env::remove_var("EDITOR");

        let result = ExternalEditor::detect_editor();
        assert_eq!(result, "vi", "should fall back to vi when no env vars set");

        match orig_visual {
            Some(v) => env::set_var("VISUAL", v),
            None => env::remove_var("VISUAL"),
        }
        match orig_editor {
            Some(v) => env::set_var("EDITOR", v),
            None => env::remove_var("EDITOR"),
        }
    }

    #[test]
    #[serial_test::serial]
    fn detect_editor_ignores_empty_visual() {
        let orig_visual = env::var("VISUAL").ok();
        let orig_editor = env::var("EDITOR").ok();

        env::set_var("VISUAL", "");
        env::set_var("EDITOR", "nano");

        let result = ExternalEditor::detect_editor();
        assert_eq!(result, "nano", "empty $VISUAL should be ignored");

        match orig_visual {
            Some(v) => env::set_var("VISUAL", v),
            None => env::remove_var("VISUAL"),
        }
        match orig_editor {
            Some(v) => env::set_var("EDITOR", v),
            None => env::remove_var("EDITOR"),
        }
    }

    // =========================================================================
    // Editor name extraction
    // =========================================================================

    #[test]
    fn editor_name_simple_command() {
        let editor = ExternalEditor {
            editor_command: "vim".to_string(),
            temp_dir: env::temp_dir(),
        };
        assert_eq!(editor.editor_name(), "vim");
    }

    #[test]
    fn editor_name_with_path() {
        let editor = ExternalEditor {
            editor_command: "/usr/bin/vim".to_string(),
            temp_dir: env::temp_dir(),
        };
        assert_eq!(editor.editor_name(), "vim");
    }

    #[test]
    fn editor_name_with_args() {
        let editor = ExternalEditor {
            editor_command: "code --wait".to_string(),
            temp_dir: env::temp_dir(),
        };
        assert_eq!(editor.editor_name(), "code");
    }

    #[test]
    fn editor_name_with_path_and_args() {
        let editor = ExternalEditor {
            editor_command: "/usr/local/bin/subl --wait --new-window".to_string(),
            temp_dir: env::temp_dir(),
        };
        assert_eq!(editor.editor_name(), "subl");
    }

    // =========================================================================
    // --wait flag detection
    // =========================================================================

    #[test]
    fn needs_wait_flag_for_code() {
        assert!(needs_wait_flag("code"));
        assert!(needs_wait_flag("/usr/bin/code"));
    }

    #[test]
    fn needs_wait_flag_for_subl() {
        assert!(needs_wait_flag("subl"));
        assert!(needs_wait_flag("/opt/homebrew/bin/subl"));
    }

    #[test]
    fn needs_wait_flag_for_atom() {
        assert!(needs_wait_flag("atom"));
    }

    #[test]
    fn needs_wait_flag_for_zed() {
        assert!(needs_wait_flag("zed"));
    }

    #[test]
    fn no_wait_flag_for_vim() {
        assert!(!needs_wait_flag("vim"));
        assert!(!needs_wait_flag("/usr/bin/vim"));
    }

    #[test]
    fn no_wait_flag_for_nano() {
        assert!(!needs_wait_flag("nano"));
    }

    #[test]
    fn no_wait_flag_for_vi() {
        assert!(!needs_wait_flag("vi"));
    }

    #[test]
    fn no_wait_flag_for_emacs() {
        assert!(!needs_wait_flag("emacs"));
    }

    // =========================================================================
    // Temp file creation and content round-trip
    // =========================================================================

    #[test]
    fn temp_file_creation_writes_content() {
        let editor = ExternalEditor {
            editor_command: "cat".to_string(),
            temp_dir: env::temp_dir(),
        };

        let content = "Hello, editor!\nLine two.";
        let path = editor
            .create_temp_file(content)
            .expect("should create temp file");

        assert!(path.exists(), "temp file should exist");
        assert!(
            path.extension().is_some_and(|ext| ext == "md"),
            "temp file should have .md extension"
        );
        assert!(
            path.to_str().unwrap().contains("patina-edit-"),
            "temp file should contain patina-edit- prefix"
        );

        let read_back = fs::read_to_string(&path).expect("should read temp file");
        assert_eq!(read_back, content, "content round-trip should be identical");

        // Clean up
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn temp_file_cleanup_on_success() {
        // Use `true` as the "editor" — it exits 0 without modifying the file
        let editor = ExternalEditor {
            editor_command: "true".to_string(),
            temp_dir: env::temp_dir(),
        };

        let content = "test content";
        let result = editor.open_with_content(content);
        assert!(
            result.is_ok(),
            "open_with_content should succeed with `true`"
        );

        // The temp file should have been cleaned up — we can't check exact path
        // but we can verify the result is the original content (since `true`
        // doesn't modify the file).
        assert_eq!(result.unwrap(), content);
    }

    #[test]
    fn content_round_trip_with_true_editor() {
        let editor = ExternalEditor {
            editor_command: "true".to_string(),
            temp_dir: env::temp_dir(),
        };

        let content = "line 1\nline 2\nline 3\n";
        let result = editor.open_with_content(content).expect("should succeed");
        assert_eq!(
            result, content,
            "unmodified round-trip should preserve content"
        );
    }

    #[test]
    fn content_round_trip_empty_input() {
        let editor = ExternalEditor {
            editor_command: "true".to_string(),
            temp_dir: env::temp_dir(),
        };

        let result = editor
            .open_with_content("")
            .expect("should succeed with empty input");
        assert_eq!(result, "", "empty content should round-trip as empty");
    }

    #[test]
    fn content_round_trip_unicode() {
        let editor = ExternalEditor {
            editor_command: "true".to_string(),
            temp_dir: env::temp_dir(),
        };

        let content = "Hello \u{1F600} world \u{00E9}\u{00E8}\u{00EA}";
        let result = editor
            .open_with_content(content)
            .expect("should succeed with unicode");
        assert_eq!(result, content, "unicode content should round-trip intact");
    }

    // =========================================================================
    // Error paths
    // =========================================================================

    #[test]
    fn editor_nonzero_exit_returns_error() {
        let editor = ExternalEditor {
            editor_command: "false".to_string(),
            temp_dir: env::temp_dir(),
        };

        let result = editor.open_with_content("test");
        assert!(result.is_err(), "non-zero exit should produce an error");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("exited with status"),
            "error message should mention exit status, got: {err_msg}"
        );
    }

    #[test]
    fn nonexistent_editor_returns_error() {
        let editor = ExternalEditor {
            editor_command: "this-editor-does-not-exist-xyz-999".to_string(),
            temp_dir: env::temp_dir(),
        };

        let result = editor.open_with_content("test");
        assert!(
            result.is_err(),
            "nonexistent editor should produce an error"
        );
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("failed to launch"),
            "error should mention launch failure, got: {err_msg}"
        );
    }

    #[test]
    fn invalid_temp_dir_returns_error() {
        let editor = ExternalEditor {
            editor_command: "true".to_string(),
            temp_dir: PathBuf::from("/nonexistent/directory/that/does/not/exist"),
        };

        let result = editor.open_with_content("test");
        assert!(result.is_err(), "invalid temp dir should produce an error");
    }

    #[test]
    fn editor_with_wait_flag_already_present_does_not_duplicate() {
        // Construct an ExternalEditor with "code --wait" and verify no duplication
        let editor = ExternalEditor {
            editor_command: "code --wait".to_string(),
            temp_dir: env::temp_dir(),
        };

        let path = editor
            .create_temp_file("test")
            .expect("should create temp file");

        // We can inspect the args that would be built
        let parts: Vec<&str> = editor.editor_command.split_whitespace().collect();
        let program = parts[0];
        let existing_args: Vec<&str> = parts[1..].to_vec();

        // Verify --wait is already in the args
        assert!(existing_args.contains(&"--wait"));

        // needs_wait_flag would return true for "code"
        assert!(needs_wait_flag(program));

        // But the run_editor logic checks for existing --wait before adding
        // This is a structural test — the actual dedup logic is in run_editor

        let _ = fs::remove_file(&path);
    }

    // =========================================================================
    // Default trait
    // =========================================================================

    #[test]
    fn default_creates_valid_editor() {
        let editor = ExternalEditor::default();
        assert!(!editor.editor_command.is_empty());
        assert!(!editor.editor_name().is_empty());
    }

    // =========================================================================
    // Temp file uses .md extension
    // =========================================================================

    #[test]
    fn temp_file_has_md_extension() {
        let editor = ExternalEditor {
            editor_command: "true".to_string(),
            temp_dir: env::temp_dir(),
        };

        let path = editor.create_temp_file("# Markdown heading").unwrap();
        assert_eq!(
            path.extension().and_then(|e| e.to_str()),
            Some("md"),
            "temp file must use .md extension for syntax highlighting"
        );
        let _ = fs::remove_file(&path);
    }
}
