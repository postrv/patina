//! Slash command auto-completion engine.
//!
//! Provides real-time filtering and scoring of completion candidates as the user
//! types a `/` command. Candidates come from multiple sources: built-in commands,
//! plugin commands, and (future) MCP tools.
//!
//! # Architecture
//!
//! - [`CompletionEntry`] — a single candidate with name, description, and source
//! - [`CompletionSource`] — discriminates built-in vs plugin vs MCP vs user
//! - [`CompletionState`] — manages the filtered list, selection index, and lifecycle
//! - [`CompletionProvider`] — trait for pluggable candidate sources
//! - [`score_candidate`] — scores a candidate against a filter string
//!
//! The data types ([`CompletionEntry`], [`CompletionSource`], [`CompletionState`],
//! [`score_candidate`]) are canonically defined in [`crate::types::ui_state`] and
//! re-exported here for backward compatibility. Provider traits and implementations
//! remain in this module since they depend on `app/`-level types.
//!
//! # Example
//!
//! ```rust,ignore
//! use patina::app::completion::{CompletionState, CompletionEntry, CompletionSource};
//!
//! let candidates = vec![
//!     CompletionEntry::new("help", "Show help information", CompletionSource::Builtin),
//!     CompletionEntry::new("plugins", "List loaded plugins", CompletionSource::Builtin),
//! ];
//! let mut state = CompletionState::new(candidates);
//! state.set_filter("he");
//! assert_eq!(state.filtered().len(), 1);
//! ```

// Re-export data types from their canonical location in types/ui_state.
pub use crate::types::ui_state::{
    score_candidate, CompletionEntry, CompletionSource, CompletionState,
};

/// Trait for pluggable completion candidate sources.
pub trait CompletionProvider: Send + Sync {
    /// Returns all candidates from this source.
    fn candidates(&self) -> Vec<CompletionEntry>;
}

/// Provides built-in slash commands as completion candidates.
pub struct BuiltinCommandProvider;

impl CompletionProvider for BuiltinCommandProvider {
    fn candidates(&self) -> Vec<CompletionEntry> {
        vec![
            CompletionEntry::new(
                "agent",
                "Manage background agents",
                CompletionSource::Builtin,
            ),
            CompletionEntry::new(
                "continuous",
                "Start/stop continuous coding loop",
                CompletionSource::Builtin,
            ),
            CompletionEntry::new(
                "context",
                "Show context window usage",
                CompletionSource::Builtin,
            ),
            CompletionEntry::new(
                "cost",
                "Show session cost and token usage",
                CompletionSource::Builtin,
            ),
            CompletionEntry::new(
                "experiment",
                "Manage isolated experiments",
                CompletionSource::Builtin,
            ),
            CompletionEntry::new(
                "export",
                "Export conversation (markdown/json)",
                CompletionSource::Builtin,
            ),
            CompletionEntry::new(
                "fork",
                "Fork session into a new branch",
                CompletionSource::Builtin,
            ),
            CompletionEntry::new("help", "Show help information", CompletionSource::Builtin),
            CompletionEntry::new("mcp", "Show MCP server status", CompletionSource::Builtin),
            CompletionEntry::new(
                "memory",
                "Manage persistent memory",
                CompletionSource::Builtin,
            ),
            CompletionEntry::new("plugins", "List loaded plugins", CompletionSource::Builtin),
            CompletionEntry::new(
                "terminal-setup",
                "Configure terminal settings",
                CompletionSource::Builtin,
            ),
            CompletionEntry::new(
                "worktree",
                "Manage git worktree agents",
                CompletionSource::Builtin,
            ),
        ]
    }
}

/// Provides plugin commands as completion candidates.
pub struct PluginCommandProvider {
    commands: Vec<(String, String)>,
}

impl PluginCommandProvider {
    /// Creates a provider from a plugin registry.
    #[must_use]
    pub fn from_registry(registry: &crate::plugins::PluginRegistry) -> Self {
        let command_names = registry.list_commands();
        let commands: Vec<(String, String)> = command_names
            .into_iter()
            .map(|name| {
                let plugin = registry
                    .get_command_plugin(&name)
                    .unwrap_or_else(|| "unknown".to_string());
                (name, plugin)
            })
            .collect();
        Self { commands }
    }

    /// Creates an empty provider (no plugins loaded).
    #[must_use]
    pub fn empty() -> Self {
        Self {
            commands: Vec::new(),
        }
    }
}

impl CompletionProvider for PluginCommandProvider {
    fn candidates(&self) -> Vec<CompletionEntry> {
        self.commands
            .iter()
            .map(|(name, plugin)| {
                CompletionEntry::new(
                    name.clone(),
                    format!("Plugin command ({plugin})"),
                    CompletionSource::Plugin(plugin.clone()),
                )
            })
            .collect()
    }
}

/// Provides MCP tools as completion candidates (future).
pub struct McpToolProvider {
    tools: Vec<(String, String)>,
}

impl McpToolProvider {
    /// Creates an empty provider (MCP not yet wired).
    #[must_use]
    pub fn empty() -> Self {
        Self { tools: Vec::new() }
    }

    /// Creates a provider from a list of tool names and server names.
    #[must_use]
    pub fn new(tools: Vec<(String, String)>) -> Self {
        Self { tools }
    }
}

impl CompletionProvider for McpToolProvider {
    fn candidates(&self) -> Vec<CompletionEntry> {
        self.tools
            .iter()
            .map(|(name, server)| {
                CompletionEntry::new(
                    name.clone(),
                    format!("MCP tool ({server})"),
                    CompletionSource::McpTool(server.clone()),
                )
            })
            .collect()
    }
}

/// Collects candidates from all providers.
#[must_use]
pub fn collect_candidates(providers: &[&dyn CompletionProvider]) -> Vec<CompletionEntry> {
    providers.iter().flat_map(|p| p.candidates()).collect()
}

/// Distinguishes between slash-command and file-mention completion modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionMode {
    /// Slash command completion (triggered by `/` at start of input).
    SlashCommand,
    /// File mention completion (triggered by `@` at start of word).
    FileMention,
}

/// Directories filtered out of file completion results.
const IGNORED_DIRS: &[&str] = &[".git", "target", "node_modules"];

/// Maximum number of file completion results returned.
const MAX_FILE_RESULTS: usize = 20;

/// Provides file path completion candidates for `@`-mention autocomplete.
pub struct FileCompletionProvider {
    working_dir: std::path::PathBuf,
}

impl FileCompletionProvider {
    /// Creates a new file completion provider rooted at `working_dir`.
    #[must_use]
    pub fn new(working_dir: std::path::PathBuf) -> Self {
        Self { working_dir }
    }

    /// Searches for files matching `partial_path` relative to the working directory.
    #[must_use]
    pub fn search(&self, partial_path: &str) -> Vec<CompletionEntry> {
        let mut results = Vec::new();
        let (dir_prefix, name_prefix) = if let Some(last_slash) = partial_path.rfind('/') {
            (
                partial_path[..=last_slash].to_string(),
                partial_path[last_slash + 1..].to_string(),
            )
        } else {
            (String::new(), partial_path.to_string())
        };
        let search_dir = if dir_prefix.is_empty() {
            self.working_dir.clone()
        } else {
            self.working_dir.join(&dir_prefix)
        };
        let entries = match std::fs::read_dir(&search_dir) {
            Ok(entries) => entries,
            Err(_) => return results,
        };
        let name_prefix_lower = name_prefix.to_lowercase();
        let mut matched: Vec<(String, bool)> = Vec::new();
        for entry_result in entries {
            let entry = match entry_result {
                Ok(e) => e,
                Err(_) => continue,
            };
            let file_name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry.path().is_dir();
            if is_dir && IGNORED_DIRS.contains(&file_name.as_str()) {
                continue;
            }
            if file_name.starts_with('.') {
                continue;
            }
            if file_name.to_lowercase().starts_with(&name_prefix_lower) {
                let relative_path = if dir_prefix.is_empty() {
                    file_name.clone()
                } else {
                    format!("{dir_prefix}{file_name}")
                };
                matched.push((relative_path, is_dir));
            }
        }
        matched.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        for (path, is_dir) in matched.into_iter().take(MAX_FILE_RESULTS) {
            let display_name = if is_dir {
                format!("{path}/")
            } else {
                path.clone()
            };
            let description = if is_dir { "Directory" } else { "File" };
            results.push(CompletionEntry::new(
                display_name,
                description,
                CompletionSource::File,
            ));
        }
        results
    }
}

/// Detects whether the character at the given position triggers file-mention completion.
#[must_use]
pub fn is_file_mention_trigger(text: &str, cursor_pos: usize) -> bool {
    if cursor_pos == 0 || cursor_pos > text.chars().count() {
        return false;
    }
    let chars: Vec<char> = text.chars().collect();
    if chars[cursor_pos - 1] != '@' {
        return false;
    }
    cursor_pos == 1 || chars.get(cursor_pos - 2).is_none_or(|c| c.is_whitespace())
}

/// Extracts the partial path after the most recent `@` trigger.
#[must_use]
pub fn extract_mention_filter(text: &str, cursor_pos: usize) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    if cursor_pos > chars.len() {
        return None;
    }
    let mut at_pos = None;
    for i in (0..cursor_pos).rev() {
        if chars[i] == '@' {
            if i == 0 || chars[i - 1].is_whitespace() {
                at_pos = Some(i);
                break;
            }
            return None;
        }
        if chars[i].is_whitespace() {
            return None;
        }
    }
    at_pos.map(|pos| chars[pos + 1..cursor_pos].iter().collect())
}

/// Resolves `@path` mentions in a user message by reading file contents.
#[must_use]
pub fn resolve_mentions(input: &str, working_dir: &std::path::Path) -> (String, String) {
    let mut context = String::new();
    let mention_paths = extract_at_paths(input);
    for path_str in &mention_paths {
        let full_path = working_dir.join(path_str);
        match std::fs::read_to_string(&full_path) {
            Ok(file_content) => {
                context.push_str(&format!(
                    "<file_context path=\"{path_str}\">\n{file_content}\n</file_context>\n\n"
                ));
            }
            Err(e) => {
                tracing::warn!("Failed to read mentioned file {}: {}", path_str, e);
            }
        }
    }
    (context, input.to_string())
}

fn extract_at_paths(text: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' && (i == 0 || chars[i - 1].is_whitespace()) {
            let start = i + 1;
            let mut end = start;
            while end < chars.len() && !chars[end].is_whitespace() {
                end += 1;
            }
            if end > start {
                paths.push(chars[start..end].iter().collect());
            }
            i = end;
        } else {
            i += 1;
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- CompletionEntry and CompletionSource tests (8.1.1) ---

    #[test]
    fn completion_entry_construction() {
        let entry = CompletionEntry::new("help", "Show help", CompletionSource::Builtin);
        assert_eq!(entry.name, "help");
        assert_eq!(entry.description, "Show help");
        assert_eq!(entry.source, CompletionSource::Builtin);
    }

    #[test]
    fn completion_source_all_variants() {
        let builtin = CompletionSource::Builtin;
        let plugin = CompletionSource::Plugin("my-plugin".to_string());
        let mcp = CompletionSource::McpTool("server".to_string());
        let user = CompletionSource::User;
        let file = CompletionSource::File;

        assert_eq!(builtin.indicator(), "");
        assert_eq!(plugin.indicator(), "[P]");
        assert_eq!(mcp.indicator(), "[M]");
        assert_eq!(user.indicator(), "[U]");
        assert_eq!(file.indicator(), "[@]");
    }

    #[test]
    fn completion_entry_clone_and_eq() {
        let entry = CompletionEntry::new("help", "Show help", CompletionSource::Builtin);
        let cloned = entry.clone();
        assert_eq!(entry, cloned);
    }

    #[test]
    fn completion_entry_debug() {
        let entry = CompletionEntry::new("help", "Show help", CompletionSource::Builtin);
        let debug = format!("{entry:?}");
        assert!(debug.contains("help"));
    }

    // --- score_candidate tests (8.1.2) ---

    #[test]
    fn score_exact_match() {
        let entry = CompletionEntry::new("help", "Show help", CompletionSource::Builtin);
        assert_eq!(score_candidate(&entry, "help"), 100);
    }

    #[test]
    fn score_prefix_match() {
        let entry = CompletionEntry::new("help", "Show help", CompletionSource::Builtin);
        assert_eq!(score_candidate(&entry, "he"), 80);
    }

    #[test]
    fn score_fuzzy_name() {
        let entry = CompletionEntry::new(
            "terminal-setup",
            "Configure terminal",
            CompletionSource::Builtin,
        );
        assert_eq!(score_candidate(&entry, "setup"), 60);
    }

    #[test]
    fn score_fuzzy_description() {
        let entry = CompletionEntry::new("help", "Show documentation", CompletionSource::Builtin);
        assert_eq!(score_candidate(&entry, "doc"), 40);
    }

    #[test]
    fn score_no_match() {
        let entry = CompletionEntry::new("help", "Show help", CompletionSource::Builtin);
        assert_eq!(score_candidate(&entry, "xyz"), 0);
    }

    #[test]
    fn score_case_insensitive() {
        let entry = CompletionEntry::new("Help", "Show HELP", CompletionSource::Builtin);
        assert_eq!(score_candidate(&entry, "HELP"), 100);
        assert_eq!(score_candidate(&entry, "he"), 80);
    }

    #[test]
    fn score_empty_filter_matches_all() {
        let entry = CompletionEntry::new("anything", "whatever", CompletionSource::Builtin);
        assert_eq!(score_candidate(&entry, ""), 100);
    }

    // --- CompletionState tests (8.1.3) ---

    fn sample_candidates() -> Vec<CompletionEntry> {
        vec![
            CompletionEntry::new("agent", "Manage agents", CompletionSource::Builtin),
            CompletionEntry::new("continuous", "Continuous loop", CompletionSource::Builtin),
            CompletionEntry::new("help", "Show help", CompletionSource::Builtin),
            CompletionEntry::new("mcp", "Show MCP server status", CompletionSource::Builtin),
            CompletionEntry::new("plugins", "List plugins", CompletionSource::Builtin),
            CompletionEntry::new(
                "terminal-setup",
                "Terminal config",
                CompletionSource::Builtin,
            ),
            CompletionEntry::new("worktree", "Git worktrees", CompletionSource::Builtin),
        ]
    }

    #[test]
    fn state_new_shows_all_candidates() {
        let state = CompletionState::new(sample_candidates());
        assert_eq!(state.filtered().len(), 7);
        assert_eq!(state.selected_index(), 0);
    }

    #[test]
    fn state_set_filter_narrows_results() {
        let mut state = CompletionState::new(sample_candidates());
        state.set_filter("con");
        // "continuous" matches by prefix, "terminal-setup" matches "con" in "Terminal config"
        assert_eq!(state.filtered().len(), 2);
        assert_eq!(state.filtered()[0].name, "continuous"); // prefix=80 > desc=40
    }

    #[test]
    fn state_filter_sorted_by_score() {
        let candidates = vec![
            CompletionEntry::new("abc-help", "unrelated", CompletionSource::Builtin),
            CompletionEntry::new("help", "Show help", CompletionSource::Builtin),
            CompletionEntry::new("helper", "Helper tool", CompletionSource::Builtin),
        ];
        let mut state = CompletionState::new(candidates);
        state.set_filter("help");
        assert_eq!(state.filtered()[0].name, "help"); // exact = 100
        assert_eq!(state.filtered()[1].name, "helper"); // prefix = 80
        assert_eq!(state.filtered()[2].name, "abc-help"); // substring = 60
    }

    #[test]
    fn state_select_next_wraps() {
        let mut state = CompletionState::new(sample_candidates());
        assert_eq!(state.selected_index(), 0);
        for _ in 0..7 {
            state.select_next();
        }
        assert_eq!(state.selected_index(), 0); // wrapped back
    }

    #[test]
    fn state_select_previous_wraps() {
        let mut state = CompletionState::new(sample_candidates());
        state.select_previous(); // 0 -> 6
        assert_eq!(state.selected_index(), 6);
    }

    #[test]
    fn state_selected_entry() {
        let state = CompletionState::new(sample_candidates());
        let entry = state.selected_entry().unwrap();
        assert_eq!(entry.name, "agent");
    }

    #[test]
    fn state_accept_returns_name() {
        let state = CompletionState::new(sample_candidates());
        assert_eq!(state.accept(), Some("agent".to_string()));
    }

    #[test]
    fn state_accept_empty_returns_none() {
        let mut state = CompletionState::new(sample_candidates());
        state.set_filter("nonexistent_garbage");
        assert!(state.accept().is_none());
    }

    #[test]
    fn state_filter_resets_selection() {
        let mut state = CompletionState::new(sample_candidates());
        state.select_next();
        state.select_next();
        assert_eq!(state.selected_index(), 2);
        state.set_filter("a");
        assert_eq!(state.selected_index(), 0);
    }

    #[test]
    fn state_select_on_empty_is_noop() {
        let mut state = CompletionState::new(Vec::new());
        state.select_next();
        assert_eq!(state.selected_index(), 0);
        state.select_previous();
        assert_eq!(state.selected_index(), 0);
    }

    // --- BuiltinCommandProvider tests (8.2.1) ---

    #[test]
    fn builtin_provider_returns_all_commands() {
        let provider = BuiltinCommandProvider;
        let candidates = provider.candidates();
        assert_eq!(candidates.len(), 13);
        for entry in &candidates {
            assert_eq!(entry.source, CompletionSource::Builtin);
            assert!(!entry.description.is_empty());
        }
    }

    #[test]
    fn builtin_provider_contains_all_commands() {
        let provider = BuiltinCommandProvider;
        let candidates = provider.candidates();
        let names: Vec<&str> = candidates.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"agent"));
        assert!(names.contains(&"continuous"));
        assert!(names.contains(&"context"));
        assert!(names.contains(&"cost"));
        assert!(names.contains(&"experiment"));
        assert!(names.contains(&"export"));
        assert!(names.contains(&"fork"));
        assert!(names.contains(&"help"));
        assert!(names.contains(&"mcp"));
        assert!(names.contains(&"memory"));
        assert!(names.contains(&"plugins"));
        assert!(names.contains(&"terminal-setup"));
        assert!(names.contains(&"worktree"));
    }

    // --- PluginCommandProvider tests (8.2.2) ---

    #[test]
    fn plugin_provider_empty_returns_empty() {
        let provider = PluginCommandProvider::empty();
        assert!(provider.candidates().is_empty());
    }

    #[test]
    fn plugin_provider_returns_plugin_source() {
        let provider = PluginCommandProvider {
            commands: vec![("my-plugin:greet".to_string(), "my-plugin".to_string())],
        };
        let candidates = provider.candidates();
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].source,
            CompletionSource::Plugin("my-plugin".to_string())
        );
    }

    // --- McpToolProvider tests (8.2.3) ---

    #[test]
    fn mcp_provider_empty_returns_empty() {
        let provider = McpToolProvider::empty();
        assert!(provider.candidates().is_empty());
    }

    #[test]
    fn mcp_provider_returns_mcp_source() {
        let provider = McpToolProvider::new(vec![("read_file".to_string(), "narsil".to_string())]);
        let candidates = provider.candidates();
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].source,
            CompletionSource::McpTool("narsil".to_string())
        );
    }

    // --- collect_candidates tests ---

    #[test]
    fn collect_candidates_merges_sources() {
        let builtin = BuiltinCommandProvider;
        let mcp = McpToolProvider::new(vec![("tool1".to_string(), "srv".to_string())]);
        let all = collect_candidates(&[&builtin, &mcp]);
        assert_eq!(all.len(), 14); // 13 builtin + 1 mcp
    }

    #[test]
    fn file_provider_search_finds_files() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.path().join("lib.rs"), "").unwrap();
        let provider = FileCompletionProvider::new(dir.path().to_path_buf());
        let results = provider.search("");
        assert!(results.len() >= 2);
    }

    #[test]
    fn file_provider_filters_git_directory() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::create_dir_all(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "").unwrap();
        let provider = FileCompletionProvider::new(dir.path().to_path_buf());
        let results = provider.search("");
        let names: Vec<&str> = results.iter().map(|e| e.name.as_str()).collect();
        assert!(!names.contains(&".git/"));
        assert!(!names.contains(&"target/"));
        assert!(names.contains(&"Cargo.toml"));
    }

    #[test]
    fn file_provider_handles_nested_paths() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "").unwrap();
        let provider = FileCompletionProvider::new(dir.path().to_path_buf());
        let results = provider.search("src/");
        assert!(results.iter().any(|e| e.name == "src/main.rs"));
    }

    #[test]
    fn at_trigger_at_start() {
        assert!(is_file_mention_trigger("@", 1));
    }

    #[test]
    fn at_trigger_after_space() {
        assert!(is_file_mention_trigger("hello @", 7));
    }

    #[test]
    fn at_trigger_not_mid_word() {
        assert!(!is_file_mention_trigger("hello@world", 6));
    }

    #[test]
    fn completion_mode_variants() {
        assert_ne!(CompletionMode::SlashCommand, CompletionMode::FileMention);
    }

    #[test]
    fn extract_mention_filter_simple() {
        assert_eq!(extract_mention_filter("@src", 4), Some("src".to_string()));
    }

    #[test]
    fn extract_mention_filter_no_at() {
        assert!(extract_mention_filter("no at", 5).is_none());
    }

    #[test]
    fn resolve_mentions_parses_at_path() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("test.txt"), "file content").unwrap();
        let (ctx, text) = resolve_mentions("check @test.txt please", dir.path());
        assert!(ctx.contains("file content"));
        assert_eq!(text, "check @test.txt please");
    }

    #[test]
    fn resolve_mentions_no_mentions() {
        let dir = tempfile::TempDir::new().unwrap();
        let (ctx, _) = resolve_mentions("no mentions", dir.path());
        assert!(ctx.is_empty());
    }

    #[test]
    fn resolve_mentions_missing_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let (ctx, _) = resolve_mentions("@nonexistent.txt", dir.path());
        assert!(ctx.is_empty());
    }

    #[test]
    fn file_entry_has_file_source() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("test.rs"), "").unwrap();
        let provider = FileCompletionProvider::new(dir.path().to_path_buf());
        let results = provider.search("test");
        assert!(!results.is_empty());
        assert_eq!(results[0].source, CompletionSource::File);
    }
}
