use crate::session::Checkpoint;

/// Session tracking state extracted from AppState.
///
/// Groups the session ID, name, dirty flag, and checkpoints for auto-save
/// and session branching functionality.
#[derive(Debug, Clone, Default)]
pub struct SessionTracking {
    /// Current session ID, assigned on first save or restore.
    id: Option<String>,
    /// Optional custom session name set by `/rename`.
    name: Option<String>,
    /// Whether the session needs to be saved.
    dirty: bool,
    /// Checkpoints for session branching and rewind.
    checkpoints: Vec<Checkpoint>,
}

impl SessionTracking {
    /// Creates a new `SessionTracking` with no session ID and clean state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the current session ID, if one has been assigned.
    #[must_use]
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    /// Sets the session ID.
    pub fn set_id(&mut self, id: String) {
        self.id = Some(id);
    }

    /// Marks the session as needing to be saved.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Returns `true` and clears the dirty flag if the session needs saving.
    ///
    /// This is an atomic check-and-clear to prevent double saves.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// Returns whether the session is dirty without clearing.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Returns the custom session name, if one has been set.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Sets or clears the custom session name.
    pub fn set_name(&mut self, name: Option<String>) {
        self.name = name;
        self.dirty = true;
    }

    /// Returns the checkpoints for this session.
    #[must_use]
    pub fn checkpoints(&self) -> &[Checkpoint] {
        &self.checkpoints
    }

    /// Adds a checkpoint, skipping if one already exists at the same message index.
    ///
    /// # Arguments
    ///
    /// * `checkpoint` - The checkpoint to add.
    pub fn add_checkpoint(&mut self, checkpoint: Checkpoint) {
        let idx = checkpoint.message_index();
        if self.checkpoints.iter().any(|c| c.message_index() == idx) {
            return;
        }
        self.checkpoints.push(checkpoint);
        self.dirty = true;
    }

    /// Replaces all checkpoints (used when restoring from a session).
    pub fn set_checkpoints(&mut self, checkpoints: Vec<Checkpoint>) {
        self.checkpoints = checkpoints;
    }
}
