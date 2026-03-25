use super::*;

impl AppState {
    /// Sets the compression orchestrator.
    pub fn set_compression_orchestrator(&mut self, orchestrator: Arc<CompressionOrchestrator>) {
        self.compression.set_compression_orchestrator(orchestrator);
    }

    /// Returns a reference to the compression orchestrator if available.
    #[must_use]
    pub fn compression_orchestrator(&self) -> Option<&Arc<CompressionOrchestrator>> {
        self.compression.compression_orchestrator()
    }

    /// Returns true if the compression orchestrator supports CCG.
    #[must_use]
    pub fn has_ccg_support(&self) -> bool {
        self.compression.has_ccg_support()
    }

    /// Injects CCG context by fetching the default context (manifest + architecture).
    ///
    /// This method fetches context from narsil-mcp via the compression orchestrator.
    /// The context is cached based on the repository hash, so subsequent calls with
    /// the same hash return cached results without making MCP calls.
    ///
    /// # Arguments
    ///
    /// * `client` - An active MCP client connected to narsil-mcp
    /// * `repo_hash` - Current repository hash for cache validation
    ///
    /// # Returns
    ///
    /// - `Ok(Some(content))` - CCG context was fetched successfully
    /// - `Ok(None)` - No orchestrator available or context injection disabled
    ///
    /// # Example
    ///
    /// ```ignore
    /// if let Some(context) = state.inject_ccg_context(&client, "abc123").await? {
    ///     // Prepend context to system message
    ///     system_prompt = format!("{}\n\n{}", context, system_prompt);
    /// }
    /// ```
    pub async fn inject_ccg_context(
        &mut self,
        client: &McpConnection,
        repo_hash: &str,
    ) -> anyhow::Result<Option<String>> {
        // Check if orchestrator is available
        let orchestrator = match &self.compression.compression_orchestrator {
            Some(orch) => orch.clone(),
            None => return Ok(None),
        };

        // Check if hash has changed (for cache invalidation tracking)
        let hash_changed = self
            .compression
            .last_ccg_hash
            .as_ref()
            .is_none_or(|h| h != repo_hash);

        if hash_changed {
            tracing::debug!(
                old_hash = ?self.compression.last_ccg_hash,
                new_hash = %repo_hash,
                "Repository hash changed, may fetch fresh CCG context"
            );
        }

        // Fetch context via orchestrator
        let result = orchestrator
            .get_default_context_async(client, repo_hash)
            .await;

        // Update cached hash
        self.compression.last_ccg_hash = Some(repo_hash.to_string());

        // Return content if non-empty, also cache it
        let content = result.content();
        if content.is_empty() {
            self.compression.cached_ccg_context = None;
            Ok(None)
        } else {
            tracing::info!(
                tokens = result.tokens_approx(),
                source = ?result.source(),
                "CCG context fetched and cached"
            );
            let content_string = content.to_string();
            self.compression.cached_ccg_context = Some(content_string.clone());
            Ok(Some(content_string))
        }
    }

    /// Returns the last CCG hash used for context injection.
    #[must_use]
    pub fn last_ccg_hash(&self) -> Option<&str> {
        self.compression.last_ccg_hash()
    }

    /// Sets the last CCG hash.
    pub fn set_last_ccg_hash(&mut self, hash: String) {
        self.compression.set_last_ccg_hash(hash);
    }

    /// Returns whether there is cached CCG context.
    #[must_use]
    pub fn has_cached_ccg_context(&self) -> bool {
        self.compression.has_cached_ccg_context()
    }

    /// Takes the cached CCG context.
    pub fn take_cached_ccg_context(&mut self) -> Option<String> {
        self.compression.take_cached_ccg_context()
    }

    /// Returns context for injection if auto-context is enabled.
    #[must_use]
    pub fn context_for_injection(&self) -> Option<&str> {
        self.compression.context_for_injection()
    }

    /// Returns whether a narsil MCP client is available.
    #[must_use]
    pub fn has_narsil_client(&self) -> bool {
        self.compression.has_narsil_client()
    }

    /// Sets the narsil MCP connection.
    pub fn set_narsil_client(&mut self, client: McpConnection) {
        self.compression.set_narsil_client(client);
    }

    /// Returns the maximum token budget for auto-injected context.
    #[must_use]
    pub fn context_token_budget(&self) -> usize {
        self.compression.context_token_budget()
    }

    /// Sets the maximum token budget for auto-injected context.
    pub fn set_context_token_budget(&mut self, budget: usize) {
        self.compression.set_context_token_budget(budget);
    }

    /// Returns the number of tokens injected in the most recent context injection.
    #[must_use]
    pub fn context_tokens_injected(&self) -> usize {
        self.compression.context_tokens_injected()
    }

    /// Sets the number of tokens injected.
    pub fn set_context_tokens_injected(&mut self, tokens: usize) {
        self.compression.set_context_tokens_injected(tokens);
    }

    /// Refreshes the cached CCG context by calling `build_context()`.
    ///
    /// This method lazily connects a narsil MCP client, fetches context
    /// from the compression orchestrator using the full 3-layer approach
    /// (manifest + architecture + symbols), and caches the result for
    /// injection into the next API message.
    ///
    /// Returns `None` if auto-context is disabled, no orchestrator is
    /// configured, or the narsil MCP client fails to connect.
    ///
    /// # Returns
    ///
    /// The context string if successfully fetched, `None` otherwise.
    pub async fn refresh_build_context(&mut self) -> Option<String> {
        if !self.compression.auto_context_enabled {
            return None;
        }

        let orchestrator = match &self.compression.compression_orchestrator {
            Some(orch) => orch.clone(),
            None => return None,
        };

        // Skip re-fetch if the git hash is unchanged and we already have cached context.
        // This prevents redundant MCP calls when the codebase hasn't changed.
        let repo_hash =
            get_git_head_hash(&self.working_dir).unwrap_or_else(|| "unknown".to_string());
        let hash_changed = self.compression.last_ccg_hash.as_ref() != Some(&repo_hash);

        if !hash_changed && self.compression.cached_ccg_context.is_some() {
            tracing::debug!(
                hash = %repo_hash,
                tokens = self.compression.context_tokens_injected,
                "CCG hash unchanged, reusing cached context"
            );
            return self.compression.cached_ccg_context.clone();
        }

        // Lazily initialize narsil connection
        if self.compression.narsil_client.is_none() {
            let working_dir = self.working_dir.to_string_lossy().to_string();
            let timeout = std::time::Duration::from_secs(30);
            match McpConnection::connect_stdio(
                "narsil-mcp",
                "narsil-mcp",
                &["--repos".to_string(), working_dir],
                &std::collections::HashMap::new(),
                timeout,
            )
            .await
            {
                Ok(conn) => {
                    tracing::info!("Narsil MCP connection established for context injection");
                    self.compression.narsil_client = Some(conn);
                }
                Err(e) => {
                    tracing::warn!("Failed to connect narsil-mcp for context: {}", e);
                    return None;
                }
            }
        }

        let client = self.compression.narsil_client.as_ref()?;

        let result = orchestrator
            .build_context(
                client,
                &repo_hash,
                &[], // active_files: empty = project-wide symbols
                self.compression.context_token_budget,
            )
            .await;

        // Update the hash to track what version of the codebase this context is for
        self.compression.last_ccg_hash = Some(repo_hash.clone());

        let content = result.result().content().to_string();
        if content.is_empty() {
            self.compression.cached_ccg_context = None;
            self.compression.context_tokens_injected = 0;
            tracing::debug!(hash = %repo_hash, "Context fetch returned empty result");
            None
        } else {
            tracing::info!(
                tokens = result.total_tokens(),
                manifest = result.manifest_tokens(),
                architecture = result.architecture_tokens(),
                symbols = result.symbol_tokens(),
                hash = %repo_hash,
                cache_status = "miss",
                "Build context refreshed for injection"
            );
            self.compression.context_tokens_injected = result.total_tokens();
            self.compression.cached_ccg_context = Some(content.clone());
            Some(content)
        }
    }

    /// Formats context suggestions into a string suitable for prepending to a message.
    ///
    /// Returns an empty string if suggestions is empty.
    ///
    /// # Arguments
    ///
    /// * `suggestions` - Context suggestions to format
    ///
    /// # Returns
    ///
    /// A formatted string containing all context suggestions.
    #[must_use]
    pub fn format_context_suggestions(suggestions: &[ContextSuggestion]) -> String {
        if suggestions.is_empty() {
            return String::new();
        }

        let mut parts = Vec::new();
        for suggestion in suggestions {
            parts.push(format!(
                "[Context: {}]\n{}",
                suggestion.description, suggestion.content
            ));
        }
        parts.join("\n\n")
    }
}
