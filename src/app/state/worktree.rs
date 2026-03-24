/// Worktree status bar state extracted from AppState.
///
/// Groups all fields related to the git worktree status display:
/// branch name, modified file count, and ahead/behind indicators.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorktreeStatus {
    /// Current branch name, if known.
    branch: Option<String>,
    /// Number of modified (dirty) files.
    modified: usize,
    /// Number of commits ahead of upstream.
    ahead: usize,
    /// Number of commits behind upstream.
    behind: usize,
}

impl WorktreeStatus {
    /// Creates a new `WorktreeStatus` with all fields at their defaults.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the current branch name.
    pub fn set_branch(&mut self, branch: String) {
        self.branch = Some(branch);
    }

    /// Returns the current branch name, if set.
    #[must_use]
    pub fn branch(&self) -> Option<&str> {
        self.branch.as_deref()
    }

    /// Sets the number of modified files.
    pub fn set_modified(&mut self, count: usize) {
        self.modified = count;
    }

    /// Returns the number of modified files.
    #[must_use]
    pub fn modified(&self) -> usize {
        self.modified
    }

    /// Sets the number of commits ahead of upstream.
    pub fn set_ahead(&mut self, count: usize) {
        self.ahead = count;
    }

    /// Returns the number of commits ahead of upstream.
    #[must_use]
    pub fn ahead(&self) -> usize {
        self.ahead
    }

    /// Sets the number of commits behind upstream.
    pub fn set_behind(&mut self, count: usize) {
        self.behind = count;
    }

    /// Returns the number of commits behind upstream.
    #[must_use]
    pub fn behind(&self) -> usize {
        self.behind
    }
}
