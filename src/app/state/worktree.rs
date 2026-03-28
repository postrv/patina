/// Worktree status bar state extracted from AppState.
///
/// Groups all fields related to the git worktree status display:
/// branch name, modified file count, and ahead/behind indicators.
/// Self-tracks render dirty state via an internal flag.
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
    /// Whether any mutation has occurred since last `mark_clean()`.
    dirty: bool,
}

impl WorktreeStatus {
    /// Creates a new `WorktreeStatus` with all fields at their defaults.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns whether any mutation has occurred since last `mark_clean()`.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Clears the dirty flag after rendering.
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// Sets the current branch name.
    pub fn set_branch(&mut self, branch: String) {
        self.branch = Some(branch);
        self.dirty = true;
    }

    /// Returns the current branch name, if set.
    #[must_use]
    pub fn branch(&self) -> Option<&str> {
        self.branch.as_deref()
    }

    /// Sets the number of modified files.
    pub fn set_modified(&mut self, count: usize) {
        self.modified = count;
        self.dirty = true;
    }

    /// Returns the number of modified files.
    #[must_use]
    pub fn modified(&self) -> usize {
        self.modified
    }

    /// Sets the number of commits ahead of upstream.
    pub fn set_ahead(&mut self, count: usize) {
        self.ahead = count;
        self.dirty = true;
    }

    /// Returns the number of commits ahead of upstream.
    #[must_use]
    pub fn ahead(&self) -> usize {
        self.ahead
    }

    /// Sets the number of commits behind upstream.
    pub fn set_behind(&mut self, count: usize) {
        self.behind = count;
        self.dirty = true;
    }

    /// Returns the number of commits behind upstream.
    #[must_use]
    pub fn behind(&self) -> usize {
        self.behind
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worktree_dirty_on_set_branch() {
        let mut ws = WorktreeStatus::new();
        assert!(!ws.is_dirty());
        ws.set_branch("main".to_string());
        assert!(ws.is_dirty());
    }

    #[test]
    fn test_worktree_dirty_on_set_modified() {
        let mut ws = WorktreeStatus::new();
        ws.set_modified(5);
        assert!(ws.is_dirty());
    }

    #[test]
    fn test_worktree_dirty_on_set_ahead_behind() {
        let mut ws = WorktreeStatus::new();
        ws.set_ahead(2);
        assert!(ws.is_dirty());
        ws.mark_clean();
        assert!(!ws.is_dirty());
        ws.set_behind(1);
        assert!(ws.is_dirty());
    }

    #[test]
    fn test_worktree_clean_after_mark_clean() {
        let mut ws = WorktreeStatus::new();
        ws.set_branch("feature".to_string());
        ws.set_modified(3);
        assert!(ws.is_dirty());
        ws.mark_clean();
        assert!(!ws.is_dirty());
    }
}
