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

/// Source of a completion candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionSource {
    /// Built-in slash command.
    Builtin,
    /// Command provided by a plugin.
    Plugin(String),
    /// MCP tool (future).
    McpTool(String),
    /// User-defined command (future).
    User,
}

impl CompletionSource {
    /// Returns a short display indicator for the source.
    ///
    /// Used in the completion menu to show where a command comes from.
    #[must_use]
    pub fn indicator(&self) -> &str {
        match self {
            Self::Builtin => "",
            Self::Plugin(_) => "[P]",
            Self::McpTool(_) => "[M]",
            Self::User => "[U]",
        }
    }
}

/// A single completion candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionEntry {
    /// The command name (without leading `/`).
    pub name: String,
    /// A short description shown alongside the name.
    pub description: String,
    /// Where this candidate comes from.
    pub source: CompletionSource,
}

impl CompletionEntry {
    /// Creates a new completion entry.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        source: CompletionSource,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            source,
        }
    }
}

/// Scores a candidate against a filter string.
///
/// Returns a score from 0 to 100:
/// - 100: exact name match
/// - 80: name starts with filter (prefix match)
/// - 60: filter found as substring in name (fuzzy name)
/// - 40: filter found as substring in description (fuzzy description)
/// - 0: no match
///
/// All comparisons are case-insensitive. An empty filter matches everything
/// with score 100.
#[must_use]
pub fn score_candidate(entry: &CompletionEntry, filter: &str) -> u8 {
    if filter.is_empty() {
        return 100;
    }

    let filter_lower = filter.to_lowercase();
    let name_lower = entry.name.to_lowercase();
    let desc_lower = entry.description.to_lowercase();

    if name_lower == filter_lower {
        100
    } else if name_lower.starts_with(&filter_lower) {
        80
    } else if name_lower.contains(&filter_lower) {
        60
    } else if desc_lower.contains(&filter_lower) {
        40
    } else {
        0
    }
}

/// A scored candidate for internal sorting.
#[derive(Debug, Clone)]
struct ScoredEntry {
    entry: CompletionEntry,
    score: u8,
}

/// Manages the completion popup lifecycle, filtering, and selection.
///
/// Created when the user types `/` at position 0, destroyed when they dismiss
/// or accept a candidate.
#[derive(Debug, Clone)]
pub struct CompletionState {
    /// All candidates from all providers.
    candidates: Vec<CompletionEntry>,
    /// Currently visible (filtered + sorted) entries.
    filtered: Vec<CompletionEntry>,
    /// Index into `filtered` for the highlighted entry.
    selected: usize,
    /// Current filter text (everything after `/`).
    filter: String,
}

impl CompletionState {
    /// Creates a new completion state showing all candidates.
    #[must_use]
    pub fn new(candidates: Vec<CompletionEntry>) -> Self {
        let filtered = candidates.clone();
        Self {
            candidates,
            filtered,
            selected: 0,
            filter: String::new(),
        }
    }

    /// Updates the filter and recomputes the filtered list.
    ///
    /// Resets selection to 0.
    pub fn set_filter(&mut self, filter: &str) {
        self.filter = filter.to_string();
        self.refilter();
        self.selected = 0;
    }

    /// Returns the current filter string.
    #[must_use]
    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Returns the filtered and sorted entries.
    #[must_use]
    pub fn filtered(&self) -> &[CompletionEntry] {
        &self.filtered
    }

    /// Returns the index of the currently selected entry.
    #[must_use]
    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// Returns the currently selected entry, if any.
    #[must_use]
    pub fn selected_entry(&self) -> Option<&CompletionEntry> {
        self.filtered.get(self.selected)
    }

    /// Moves selection down, wrapping to the top.
    pub fn select_next(&mut self) {
        if !self.filtered.is_empty() {
            self.selected = (self.selected + 1) % self.filtered.len();
        }
    }

    /// Moves selection up, wrapping to the bottom.
    pub fn select_previous(&mut self) {
        if !self.filtered.is_empty() {
            if self.selected == 0 {
                self.selected = self.filtered.len() - 1;
            } else {
                self.selected -= 1;
            }
        }
    }

    /// Accepts the currently selected entry.
    ///
    /// Returns the command name (without `/`) if there is a selection,
    /// or `None` if the filtered list is empty.
    #[must_use]
    pub fn accept(&self) -> Option<String> {
        self.selected_entry().map(|e| e.name.clone())
    }

    /// Recomputes the filtered list from candidates using current filter.
    fn refilter(&mut self) {
        let mut scored: Vec<ScoredEntry> = self
            .candidates
            .iter()
            .filter_map(|entry| {
                let score = score_candidate(entry, &self.filter);
                if score > 0 {
                    Some(ScoredEntry {
                        entry: entry.clone(),
                        score,
                    })
                } else {
                    None
                }
            })
            .collect();

        // Sort by score descending, then by name ascending for stable ordering.
        scored.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.entry.name.cmp(&b.entry.name))
        });

        self.filtered = scored.into_iter().map(|s| s.entry).collect();
    }
}

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
            CompletionEntry::new("help", "Show help information", CompletionSource::Builtin),
            CompletionEntry::new("mcp", "Show MCP server status", CompletionSource::Builtin),
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

        assert_eq!(builtin.indicator(), "");
        assert_eq!(plugin.indicator(), "[P]");
        assert_eq!(mcp.indicator(), "[M]");
        assert_eq!(user.indicator(), "[U]");
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
    fn builtin_provider_returns_seven_commands() {
        let provider = BuiltinCommandProvider;
        let candidates = provider.candidates();
        assert_eq!(candidates.len(), 7);
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
        assert!(names.contains(&"help"));
        assert!(names.contains(&"mcp"));
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
        assert_eq!(all.len(), 8); // 7 builtin + 1 mcp
    }
}
