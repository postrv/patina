/// Session tracking state extracted from AppState.
///
/// Groups the session ID and dirty flag for auto-save functionality.
#[derive(Debug, Clone, Default)]
pub struct SessionTracking {
    /// Current session ID, assigned on first save or restore.
    id: Option<String>,
    /// Whether the session needs to be saved.
    dirty: bool,
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
}
