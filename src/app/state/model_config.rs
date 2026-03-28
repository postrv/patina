/// Model configuration state extracted from AppState.
///
/// Groups fields related to model selection, effort level, thinking budget,
/// memory store, system prompt, and skill engine.
use crate::skills::SkillEngine;
use crate::types::config::EffortLevel;

pub struct ModelConfigState {
    /// Model name for cost tracking and API requests.
    pub(crate) current_model: String,

    /// Persistent memory store for cross-session context.
    pub(crate) memory_store: Option<crate::memory::store::MemoryStore>,

    /// Reasoning effort level for API requests.
    pub(crate) effort: EffortLevel,

    /// Optional explicit thinking budget that overrides effort level.
    pub(crate) thinking_budget: Option<u32>,

    /// System prompt text injected into API requests.
    pub(crate) system_prompt: Option<String>,

    /// Skill engine for context-aware skill matching and injection.
    pub(crate) skill_engine: Option<SkillEngine>,
}

impl ModelConfigState {
    /// Creates a new model config state with defaults.
    #[must_use]
    pub fn new() -> Self {
        Self {
            current_model: String::new(),
            memory_store: None,
            effort: EffortLevel::Auto,
            thinking_budget: None,
            system_prompt: None,
            skill_engine: None,
        }
    }

    /// Returns the current model name.
    #[must_use]
    pub fn current_model(&self) -> &str {
        &self.current_model
    }

    /// Sets the current model name.
    pub fn set_current_model(&mut self, model: String) {
        self.current_model = model;
    }

    /// Returns the memory store, if set.
    #[must_use]
    pub fn memory_store(&self) -> Option<&crate::memory::store::MemoryStore> {
        self.memory_store.as_ref()
    }

    /// Sets the memory store.
    pub fn set_memory_store(&mut self, store: crate::memory::store::MemoryStore) {
        self.memory_store = Some(store);
    }

    /// Returns the current effort level.
    #[must_use]
    pub fn effort(&self) -> EffortLevel {
        self.effort
    }

    /// Sets the effort level.
    pub fn set_effort(&mut self, effort: EffortLevel) {
        self.effort = effort;
    }

    /// Sets the thinking budget override.
    pub fn set_thinking_budget(&mut self, budget: Option<u32>) {
        self.thinking_budget = budget;
    }

    /// Returns the thinking budget override, if set.
    #[must_use]
    pub fn thinking_budget(&self) -> Option<u32> {
        self.thinking_budget
    }

    /// Sets the system prompt.
    pub fn set_system_prompt(&mut self, prompt: Option<String>) {
        self.system_prompt = prompt;
    }

    /// Returns the system prompt, if set.
    #[must_use]
    pub fn system_prompt(&self) -> Option<&str> {
        self.system_prompt.as_deref()
    }

    /// Returns the skill engine, if configured.
    #[must_use]
    pub fn skill_engine(&self) -> Option<&SkillEngine> {
        self.skill_engine.as_ref()
    }

    /// Sets the skill engine.
    pub fn set_skill_engine(&mut self, engine: SkillEngine) {
        self.skill_engine = Some(engine);
    }
}

impl Default for ModelConfigState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl ModelConfigState {
    /// Returns a mutable reference to the memory store, if set.
    pub fn memory_store_mut(&mut self) -> Option<&mut crate::memory::store::MemoryStore> {
        self.memory_store.as_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_config_defaults() {
        let state = ModelConfigState::new();
        assert!(state.current_model().is_empty());
        assert!(state.memory_store().is_none());
        assert_eq!(state.effort(), EffortLevel::Auto);
        assert!(state.thinking_budget().is_none());
        assert!(state.system_prompt().is_none());
        assert!(state.skill_engine().is_none());
    }

    #[test]
    fn test_set_current_model() {
        let mut state = ModelConfigState::new();
        state.set_current_model("claude-sonnet-4".to_string());
        assert_eq!(state.current_model(), "claude-sonnet-4");
    }

    #[test]
    fn test_effort_level() {
        let mut state = ModelConfigState::new();
        state.set_effort(EffortLevel::High);
        assert_eq!(state.effort(), EffortLevel::High);
    }

    #[test]
    fn test_thinking_budget() {
        let mut state = ModelConfigState::new();
        state.set_thinking_budget(Some(10_000));
        assert_eq!(state.thinking_budget(), Some(10_000));
        state.set_thinking_budget(None);
        assert!(state.thinking_budget().is_none());
    }

    #[test]
    fn test_system_prompt() {
        let mut state = ModelConfigState::new();
        state.set_system_prompt(Some("You are helpful.".to_string()));
        assert_eq!(state.system_prompt(), Some("You are helpful."));
        state.set_system_prompt(None);
        assert!(state.system_prompt().is_none());
    }

    #[test]
    fn test_skill_engine() {
        let mut state = ModelConfigState::new();
        assert!(state.skill_engine().is_none());
        state.set_skill_engine(SkillEngine::new());
        assert!(state.skill_engine().is_some());
        assert!(state.skill_engine().unwrap().all_skills().is_empty());
    }
}
