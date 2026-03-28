//! MCP server coordination methods for AppState.
//!
//! Extracted from state/mod.rs to reduce file size.

use super::*;

impl AppState {
    /// Sets the MCP server manager.
    pub fn set_mcp_manager(&mut self, manager: crate::mcp::manager::McpManager) {
        self.mcp_manager = Some(std::sync::Arc::new(manager));
    }

    /// Returns a reference to the MCP manager, if set.
    #[must_use]
    pub fn mcp_manager(&self) -> Option<&crate::mcp::manager::McpManager> {
        self.mcp_manager.as_deref()
    }

    /// Returns a mutable reference to the MCP manager, if set.
    ///
    /// Only succeeds when there is exactly one `Arc` reference (i.e. no
    /// spawned tasks are still holding a clone).  Used at shutdown.
    pub fn mcp_manager_mut(&mut self) -> Option<&mut crate::mcp::manager::McpManager> {
        self.mcp_manager.as_mut().and_then(std::sync::Arc::get_mut)
    }

    /// Returns all tool definitions: built-in defaults plus MCP server tools.
    ///
    /// This is the unified tool list sent to the Anthropic API.
    #[must_use]
    pub fn all_tool_definitions(&self) -> Vec<crate::api::tools::ToolDefinition> {
        let mut tools = default_tools();
        if let Some(manager) = &self.mcp_manager {
            tools.extend(manager.tool_definitions());
        }
        tools
    }
}

#[cfg(test)]
impl AppState {
    /// Returns `true` if an MCP manager is configured.
    #[must_use]
    pub fn has_mcp_manager(&self) -> bool {
        self.mcp_manager.is_some()
    }
}
