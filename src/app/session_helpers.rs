//! Session persistence helpers.
//!
//! Handles loading a previous session for `--resume` and auto-saving
//! the current session on quit or after significant events.

use anyhow::{Context, Result};
use tracing::{debug, info, warn};

use crate::session::{default_sessions_dir, SessionManager};
use crate::types::config::ResumeMode;

use super::state::AppState;
use super::Config;

/// Loads session state based on the resume mode.
pub(crate) async fn load_session_state(config: &Config) -> Result<AppState> {
    let sessions_dir = default_sessions_dir()?;
    let manager = SessionManager::new(sessions_dir);

    let session_id = match &config.resume_mode {
        ResumeMode::None => unreachable!("load_session_state called with ResumeMode::None"),
        ResumeMode::Last => {
            let (id, metadata) = manager
                .find_latest()
                .await?
                .context("No sessions found to resume")?;
            info!(
                session_id = %id,
                message_count = metadata.message_count,
                "Resuming most recent session"
            );
            id
        }
        ResumeMode::SessionId(id) => {
            info!(session_id = %id, "Resuming session by ID");
            id.clone()
        }
    };

    let session = manager
        .load(&session_id)
        .await
        .context(format!("Failed to load session '{}'", session_id))?;

    // Create AppState from the loaded session
    let mut state = AppState::with_performance_config(
        session.working_dir().clone(),
        config.skip_permissions,
        &config.performance,
        config.plugins_enabled,
        config.subagents_enabled,
    );
    state.restore_from_session(&session);

    Ok(state)
}

/// Auto-saves the current session.
///
/// Creates a new session or updates an existing one. Errors are logged
/// but do not interrupt the application flow.
pub(crate) async fn auto_save_session(state: &mut AppState, session_manager: &SessionManager) {
    let session = state.to_session();

    let result = if let Some(existing_id) = state.session_tracking().id() {
        // Update existing session
        session_manager
            .update(existing_id, &session)
            .await
            .map(|()| existing_id.to_string())
    } else {
        // Create new session
        session_manager.save(&session).await
    };

    match result {
        Ok(id) => {
            if state.session_tracking().id().is_none() {
                debug!(session_id = %id, "Created new session");
                state.session_tracking_mut().set_id(id);
            } else {
                debug!(session_id = %id, "Updated session");
            }
        }
        Err(e) => {
            warn!(error = %e, "Failed to auto-save session");
        }
    }
}
