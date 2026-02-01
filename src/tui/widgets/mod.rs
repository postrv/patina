//! TUI widgets for Patina.
//!
//! This module contains custom ratatui widgets for the Patina terminal UI.

pub mod permission_prompt;
pub mod tool_block;
pub mod worktree_picker;

pub use permission_prompt::{
    handle_key_input as handle_permission_key, PermissionPromptState, PermissionPromptWidget,
    SelectedOption as PermissionSelectedOption,
};
pub use tool_block::{ToolBlockState, ToolBlockWidget};
pub use worktree_picker::{WorktreePickerState, WorktreePickerWidget};
