//! Transcript search with regex support and match navigation.
//!
//! Provides full-text search across conversation entries using Rust's `regex`
//! crate. Matches can be navigated forward and backward with wrapping, and the
//! current match position is exposed for "3/15"-style status display.
//!
//! # Example
//!
//! ```
//! use patina::tui::search::SearchState;
//!
//! let entries = vec!["Hello world", "hello rust", "goodbye"];
//! let mut state = SearchState::new();
//! state.set_query("hello", &entries);
//! assert_eq!(state.match_count(), 2);
//!
//! let m = state.next_match().unwrap();
//! assert_eq!(m.entry_index, 0);
//! ```

use regex::Regex;

/// A single search hit inside a conversation entry.
///
/// Identifies the entry, the line within that entry, and the byte span of the
/// match so callers can highlight the exact range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchMatch {
    /// Index of the conversation entry containing the match.
    pub entry_index: usize,
    /// Zero-based line offset within the entry text.
    pub line_offset: usize,
    /// Byte offset where the match starts within the entry text.
    pub byte_start: usize,
    /// Byte offset where the match ends (exclusive) within the entry text.
    pub byte_end: usize,
}

/// State machine for transcript search with regex support.
///
/// Holds the compiled query, the match list, and a cursor for navigating
/// between matches. Invalid regex patterns are captured in `regex_error`
/// rather than propagated as panics.
///
/// # Example
///
/// ```
/// use patina::tui::search::SearchState;
///
/// let entries = vec!["line one", "line two"];
/// let mut state = SearchState::new();
/// state.set_query("one", &entries);
/// assert_eq!(state.match_count(), 1);
/// assert_eq!(state.current_position(), Some((1, 1)));
/// ```
#[derive(Debug, Clone)]
pub struct SearchState {
    /// The raw query string entered by the user.
    query: String,
    /// All matches found for the current query.
    matches: Vec<SearchMatch>,
    /// Index into `matches` for the currently focused match.
    current_match: usize,
    /// Whether the search bar is currently visible / active.
    active: bool,
    /// Human-readable error when the query is not valid regex.
    regex_error: Option<String>,
}

impl Default for SearchState {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchState {
    /// Creates an empty, inactive search state.
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::tui::search::SearchState;
    ///
    /// let state = SearchState::new();
    /// assert!(!state.is_active());
    /// assert_eq!(state.match_count(), 0);
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            query: String::new(),
            matches: Vec::new(),
            current_match: 0,
            active: false,
            regex_error: None,
        }
    }

    /// Sets the search query, compiling it as a regex and scanning `entries`.
    ///
    /// If the pattern is not valid regex the error is stored in
    /// [`regex_error`](Self::regex_error) and the match list is cleared.
    /// An empty query clears all state without error.
    ///
    /// # Arguments
    ///
    /// * `query` - The regex pattern to search for.
    /// * `entries` - Conversation entry texts to search through.
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::tui::search::SearchState;
    ///
    /// let entries = vec!["foo bar", "baz"];
    /// let mut state = SearchState::new();
    /// state.set_query("ba[rz]", &entries);
    /// assert_eq!(state.match_count(), 2);
    /// ```
    pub fn set_query<S: AsRef<str>>(&mut self, query: &str, entries: &[S]) {
        self.query = query.to_string();
        self.regex_error = None;
        self.current_match = 0;

        if query.is_empty() {
            self.matches.clear();
            return;
        }

        match search_entries(entries, query) {
            Ok(matches) => self.matches = matches,
            Err(e) => {
                self.regex_error = Some(e);
                self.matches.clear();
            }
        }
    }

    /// Advances to the next match, wrapping around at the end.
    ///
    /// Returns `None` if there are no matches.
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::tui::search::SearchState;
    ///
    /// let entries = vec!["a a"];
    /// let mut state = SearchState::new();
    /// state.set_query("a", &entries);
    /// assert!(state.next_match().is_some());
    /// ```
    pub fn next_match(&mut self) -> Option<&SearchMatch> {
        if self.matches.is_empty() {
            return None;
        }
        let idx = self.current_match;
        self.current_match = (idx + 1) % self.matches.len();
        self.matches.get(idx)
    }

    /// Moves to the previous match, wrapping around at the beginning.
    ///
    /// Returns `None` if there are no matches.
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::tui::search::SearchState;
    ///
    /// let entries = vec!["a a"];
    /// let mut state = SearchState::new();
    /// state.set_query("a", &entries);
    /// assert!(state.prev_match().is_some());
    /// ```
    pub fn prev_match(&mut self) -> Option<&SearchMatch> {
        if self.matches.is_empty() {
            return None;
        }
        if self.current_match == 0 {
            self.current_match = self.matches.len() - 1;
        } else {
            self.current_match -= 1;
        }
        self.matches.get(self.current_match)
    }

    /// Returns the total number of matches for the current query.
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::tui::search::SearchState;
    ///
    /// let state = SearchState::new();
    /// assert_eq!(state.match_count(), 0);
    /// ```
    #[must_use]
    pub fn match_count(&self) -> usize {
        self.matches.len()
    }

    /// Returns the 1-based current position and total match count.
    ///
    /// Suitable for rendering a "3/15" indicator. Returns `None` when there
    /// are no matches.
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::tui::search::SearchState;
    ///
    /// let entries = vec!["x x x"];
    /// let mut state = SearchState::new();
    /// state.set_query("x", &entries);
    /// assert_eq!(state.current_position(), Some((1, 3)));
    /// ```
    #[must_use]
    pub fn current_position(&self) -> Option<(usize, usize)> {
        if self.matches.is_empty() {
            None
        } else {
            Some((self.current_match + 1, self.matches.len()))
        }
    }

    /// Clears the search query, matches, and error state.
    ///
    /// Does **not** deactivate the search bar; call [`set_active`](Self::set_active)
    /// for that.
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::tui::search::SearchState;
    ///
    /// let entries = vec!["hello"];
    /// let mut state = SearchState::new();
    /// state.set_query("hello", &entries);
    /// assert_eq!(state.match_count(), 1);
    ///
    /// state.clear();
    /// assert_eq!(state.match_count(), 0);
    /// assert!(state.query().is_empty());
    /// ```
    pub fn clear(&mut self) {
        self.query.clear();
        self.matches.clear();
        self.current_match = 0;
        self.regex_error = None;
    }

    /// Returns `true` when the search bar is visible / active.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Sets whether the search bar is visible / active.
    pub fn set_active(&mut self, active: bool) {
        self.active = active;
    }

    /// Returns the current query string.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Returns the regex compilation error, if any.
    #[must_use]
    pub fn regex_error(&self) -> Option<&str> {
        self.regex_error.as_deref()
    }

    /// Returns all matches for the current query.
    #[must_use]
    pub fn matches(&self) -> &[SearchMatch] {
        &self.matches
    }

    /// Returns the currently focused match, if any.
    #[must_use]
    pub fn current(&self) -> Option<&SearchMatch> {
        self.matches.get(self.current_match)
    }
}

/// Searches conversation entry texts for a regex pattern.
///
/// Each entry is scanned line-by-line, and every non-overlapping match
/// produces a [`SearchMatch`] recording its entry index, line offset, and
/// byte span within the full entry text.
///
/// # Arguments
///
/// * `entries` - Slice of entry texts to search.
/// * `pattern` - A regex pattern string.
///
/// # Returns
///
/// A `Vec<SearchMatch>` on success, or a human-readable error string if the
/// pattern fails to compile.
///
/// # Errors
///
/// Returns an error string when `pattern` is not a valid regex.
///
/// # Examples
///
/// ```
/// use patina::tui::search::search_entries;
///
/// let entries = vec!["hello world", "goodbye world"];
/// let matches = search_entries(&entries, "world").unwrap();
/// assert_eq!(matches.len(), 2);
/// ```
pub fn search_entries<S: AsRef<str>>(
    entries: &[S],
    pattern: &str,
) -> Result<Vec<SearchMatch>, String> {
    let regex = Regex::new(pattern).map_err(|e| format!("Invalid regex: {e}"))?;

    let mut results = Vec::new();

    for (entry_index, entry) in entries.iter().enumerate() {
        let text = entry.as_ref();
        let mut line_start_byte = 0;

        for (line_offset, line) in text.lines().enumerate() {
            for mat in regex.find_iter(line) {
                results.push(SearchMatch {
                    entry_index,
                    line_offset,
                    byte_start: line_start_byte + mat.start(),
                    byte_end: line_start_byte + mat.end(),
                });
            }
            // +1 for the newline character (or 0 if this is the last line
            // without a trailing newline — but `lines()` handles that).
            line_start_byte += line.len() + 1;
        }
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // search_entries tests
    // =========================================================================

    #[test]
    fn test_basic_string_search() {
        let entries = vec!["hello world", "goodbye world"];
        let matches = search_entries(&entries, "world").unwrap();
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].entry_index, 0);
        assert_eq!(matches[0].byte_start, 6);
        assert_eq!(matches[0].byte_end, 11);
        assert_eq!(matches[1].entry_index, 1);
    }

    #[test]
    fn test_regex_search() {
        let entries = vec!["foo123bar", "baz456qux"];
        let matches = search_entries(&entries, r"\d+").unwrap();
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].byte_start, 3);
        assert_eq!(matches[0].byte_end, 6);
        assert_eq!(matches[1].byte_start, 3);
        assert_eq!(matches[1].byte_end, 6);
    }

    #[test]
    fn test_invalid_regex_returns_error() {
        let entries: Vec<&str> = vec!["test"];
        let result = search_entries(&entries, "[invalid");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Invalid regex"));
    }

    #[test]
    fn test_empty_pattern_matches_everything() {
        // An empty regex matches every position; verify we don't panic.
        // The regex crate matches empty patterns at every byte boundary.
        let entries = vec!["ab"];
        let matches = search_entries(&entries, "").unwrap();
        // Empty pattern matches at positions 0, 1, 2 (after each char and at end)
        assert!(!matches.is_empty());
    }

    #[test]
    fn test_no_matches() {
        let entries = vec!["hello world"];
        let matches = search_entries(&entries, "xyz").unwrap();
        assert_eq!(matches.len(), 0);
    }

    #[test]
    fn test_search_across_multiple_entries() {
        let entries = vec!["first entry\nsecond line", "third entry", "fourth\nfifth"];
        let matches = search_entries(&entries, "entry").unwrap();
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].entry_index, 0);
        assert_eq!(matches[0].line_offset, 0);
        assert_eq!(matches[1].entry_index, 1);
        assert_eq!(matches[1].line_offset, 0);
    }

    #[test]
    fn test_multiline_entry_line_offsets() {
        let entries = vec!["line0\nline1\nline2"];
        let matches = search_entries(&entries, "line").unwrap();
        assert_eq!(matches.len(), 3);
        assert_eq!(matches[0].line_offset, 0);
        assert_eq!(matches[1].line_offset, 1);
        assert_eq!(matches[2].line_offset, 2);
    }

    #[test]
    fn test_byte_offsets_in_multiline_entry() {
        let entries = vec!["ab\ncd"];
        let matches = search_entries(&entries, "cd").unwrap();
        assert_eq!(matches.len(), 1);
        // "ab\n" = 3 bytes, then "cd" starts at byte 3
        assert_eq!(matches[0].byte_start, 3);
        assert_eq!(matches[0].byte_end, 5);
    }

    #[test]
    fn test_multiple_matches_per_line() {
        let entries = vec!["aaa"];
        let matches = search_entries(&entries, "a").unwrap();
        assert_eq!(matches.len(), 3);
    }

    // =========================================================================
    // SearchState tests
    // =========================================================================

    #[test]
    fn test_new_state_is_empty() {
        let state = SearchState::new();
        assert!(!state.is_active());
        assert_eq!(state.match_count(), 0);
        assert!(state.query().is_empty());
        assert!(state.regex_error().is_none());
        assert!(state.current_position().is_none());
        assert!(state.current().is_none());
    }

    #[test]
    fn test_default_is_same_as_new() {
        let state = SearchState::default();
        assert!(!state.is_active());
        assert_eq!(state.match_count(), 0);
    }

    #[test]
    fn test_set_query_basic() {
        let entries = vec!["hello world", "hello rust"];
        let mut state = SearchState::new();
        state.set_query("hello", &entries);
        assert_eq!(state.match_count(), 2);
        assert_eq!(state.query(), "hello");
        assert!(state.regex_error().is_none());
    }

    #[test]
    fn test_set_query_with_invalid_regex() {
        let entries = vec!["test"];
        let mut state = SearchState::new();
        state.set_query("[bad", &entries);
        assert_eq!(state.match_count(), 0);
        assert!(state.regex_error().is_some());
        assert!(state.regex_error().unwrap().contains("Invalid regex"));
    }

    #[test]
    fn test_set_query_empty_clears() {
        let entries = vec!["hello"];
        let mut state = SearchState::new();
        state.set_query("hello", &entries);
        assert_eq!(state.match_count(), 1);

        state.set_query("", &entries);
        assert_eq!(state.match_count(), 0);
        assert!(state.regex_error().is_none());
    }

    #[test]
    fn test_next_match_cycles() {
        let entries = vec!["a b a"];
        let mut state = SearchState::new();
        state.set_query("a", &entries);
        assert_eq!(state.match_count(), 2);

        // First call: returns match at index 0, advances cursor to 1
        let m0 = state.next_match().unwrap().clone();
        assert_eq!(m0.byte_start, 0);

        // Second call: returns match at index 1, advances cursor to 0 (wrap)
        let m1 = state.next_match().unwrap().clone();
        assert_eq!(m1.byte_start, 4);

        // Third call: wraps back to index 0
        let m2 = state.next_match().unwrap().clone();
        assert_eq!(m2.byte_start, 0);
    }

    #[test]
    fn test_prev_match_cycles() {
        let entries = vec!["a b a"];
        let mut state = SearchState::new();
        state.set_query("a", &entries);
        assert_eq!(state.match_count(), 2);

        // First prev_match from position 0: wraps to last (index 1)
        let m0 = state.prev_match().unwrap().clone();
        assert_eq!(m0.byte_start, 4);

        // Second prev_match: goes to index 0
        let m1 = state.prev_match().unwrap().clone();
        assert_eq!(m1.byte_start, 0);

        // Third prev_match: wraps back to index 1
        let m2 = state.prev_match().unwrap().clone();
        assert_eq!(m2.byte_start, 4);
    }

    #[test]
    fn test_next_match_with_no_matches() {
        let entries = vec!["hello"];
        let mut state = SearchState::new();
        state.set_query("xyz", &entries);
        assert!(state.next_match().is_none());
    }

    #[test]
    fn test_prev_match_with_no_matches() {
        let entries = vec!["hello"];
        let mut state = SearchState::new();
        state.set_query("xyz", &entries);
        assert!(state.prev_match().is_none());
    }

    #[test]
    fn test_match_count() {
        let entries = vec!["aaa"];
        let mut state = SearchState::new();
        state.set_query("a", &entries);
        assert_eq!(state.match_count(), 3);
    }

    #[test]
    fn test_current_position_with_matches() {
        let entries = vec!["a b a"];
        let mut state = SearchState::new();
        state.set_query("a", &entries);

        // Initial position: 1/2
        assert_eq!(state.current_position(), Some((1, 2)));

        // After next_match: cursor advances to 1, so position is 2/2
        state.next_match();
        assert_eq!(state.current_position(), Some((2, 2)));

        // After another next_match: wraps to 0, position is 1/2
        state.next_match();
        assert_eq!(state.current_position(), Some((1, 2)));
    }

    #[test]
    fn test_current_position_no_matches() {
        let state = SearchState::new();
        assert_eq!(state.current_position(), None);
    }

    #[test]
    fn test_clear_resets_state() {
        let entries = vec!["hello world"];
        let mut state = SearchState::new();
        state.set_active(true);
        state.set_query("hello", &entries);
        assert_eq!(state.match_count(), 1);
        assert_eq!(state.query(), "hello");

        state.clear();

        assert_eq!(state.match_count(), 0);
        assert!(state.query().is_empty());
        assert!(state.regex_error().is_none());
        assert!(state.current_position().is_none());
        // clear() does not deactivate
        assert!(state.is_active());
    }

    #[test]
    fn test_set_active() {
        let mut state = SearchState::new();
        assert!(!state.is_active());

        state.set_active(true);
        assert!(state.is_active());

        state.set_active(false);
        assert!(!state.is_active());
    }

    #[test]
    fn test_matches_accessor() {
        let entries = vec!["hello"];
        let mut state = SearchState::new();
        state.set_query("hello", &entries);
        assert_eq!(state.matches().len(), 1);
        assert_eq!(state.matches()[0].entry_index, 0);
    }

    #[test]
    fn test_current_accessor() {
        let entries = vec!["hello world"];
        let mut state = SearchState::new();
        state.set_query("hello", &entries);
        let current = state.current().unwrap();
        assert_eq!(current.entry_index, 0);
        assert_eq!(current.byte_start, 0);
        assert_eq!(current.byte_end, 5);
    }

    #[test]
    fn test_case_sensitive_by_default() {
        let entries = vec!["Hello hello"];
        let mut state = SearchState::new();
        state.set_query("Hello", &entries);
        assert_eq!(state.match_count(), 1);
    }

    #[test]
    fn test_case_insensitive_via_regex_flag() {
        let entries = vec!["Hello hello"];
        let mut state = SearchState::new();
        state.set_query("(?i)hello", &entries);
        assert_eq!(state.match_count(), 2);
    }

    #[test]
    fn test_regex_special_characters() {
        let entries = vec!["price is $10.00"];
        let mut state = SearchState::new();
        state.set_query(r"\$\d+\.\d+", &entries);
        assert_eq!(state.match_count(), 1);
        let m = state.current().unwrap();
        assert_eq!(m.byte_start, 9);
        assert_eq!(m.byte_end, 15);
    }

    #[test]
    fn test_search_empty_entries() {
        let entries: Vec<&str> = vec![];
        let mut state = SearchState::new();
        state.set_query("hello", &entries);
        assert_eq!(state.match_count(), 0);
    }

    #[test]
    fn test_search_entries_with_empty_strings() {
        let entries = vec!["", "hello", ""];
        let mut state = SearchState::new();
        state.set_query("hello", &entries);
        assert_eq!(state.match_count(), 1);
        assert_eq!(state.matches()[0].entry_index, 1);
    }

    #[test]
    fn test_set_query_replaces_previous() {
        let entries = vec!["hello world"];
        let mut state = SearchState::new();
        state.set_query("hello", &entries);
        assert_eq!(state.match_count(), 1);

        state.set_query("world", &entries);
        assert_eq!(state.match_count(), 1);
        assert_eq!(state.matches()[0].byte_start, 6);
    }

    #[test]
    fn test_set_query_resets_current_match() {
        let entries = vec!["a a a"];
        let mut state = SearchState::new();
        state.set_query("a", &entries);
        state.next_match(); // advance cursor
        state.next_match();

        // New query should reset cursor
        state.set_query("a", &entries);
        assert_eq!(state.current_position(), Some((1, 3)));
    }
}
