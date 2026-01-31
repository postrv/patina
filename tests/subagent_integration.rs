//! Integration tests for subagent orchestration.

use patina::agents::{SubagentConfig, SubagentOrchestrator};

// ============================================================================
// 6.1.1 Subagent Spawn Tests
// ============================================================================

/// Creates a test subagent configuration.
fn test_config() -> SubagentConfig {
    SubagentConfig {
        name: "test-agent".to_string(),
        description: "A test subagent".to_string(),
        system_prompt: "You are a test agent.".to_string(),
        allowed_tools: vec!["read".to_string(), "grep".to_string()],
        max_turns: 5,
    }
}

#[test]
fn test_subagent_spawn_returns_unique_id() {
    let mut orchestrator = SubagentOrchestrator::new();
    let config = test_config();

    let id1 = orchestrator.spawn(config.clone());
    let id2 = orchestrator.spawn(config);

    assert_ne!(id1, id2, "Each spawned subagent should have a unique ID");
}

#[test]
fn test_subagent_spawn_initial_status_is_pending() {
    let mut orchestrator = SubagentOrchestrator::new();
    let config = test_config();

    let id = orchestrator.spawn(config);

    assert_eq!(
        orchestrator.get_status(id),
        Some("pending"),
        "Newly spawned subagent should have 'pending' status"
    );
}

#[test]
fn test_subagent_spawn_stores_config() {
    let mut orchestrator = SubagentOrchestrator::new();
    let config = SubagentConfig {
        name: "explorer".to_string(),
        description: "Explores codebases".to_string(),
        system_prompt: "You explore code.".to_string(),
        allowed_tools: vec!["glob".to_string(), "read".to_string()],
        max_turns: 10,
    };

    let id = orchestrator.spawn(config);

    // Verify the agent was stored and can be queried
    assert!(
        orchestrator.get_status(id).is_some(),
        "Spawned agent should be retrievable"
    );
}

#[test]
fn test_subagent_get_status_nonexistent() {
    let orchestrator = SubagentOrchestrator::new();
    let fake_id = uuid::Uuid::new_v4();

    assert_eq!(
        orchestrator.get_status(fake_id),
        None,
        "Nonexistent agent ID should return None"
    );
}

#[tokio::test]
async fn test_subagent_run_changes_status() {
    let mut orchestrator = SubagentOrchestrator::new();
    let config = test_config();

    let id = orchestrator.spawn(config);
    assert_eq!(orchestrator.get_status(id), Some("pending"));

    let result = orchestrator.run(id).await;
    assert!(result.is_ok(), "Run should succeed");
    assert_eq!(
        orchestrator.get_status(id),
        Some("completed"),
        "Status should be 'completed' after run"
    );
}

#[tokio::test]
async fn test_subagent_run_returns_result() {
    let mut orchestrator = SubagentOrchestrator::new();
    let config = SubagentConfig {
        name: "result-test".to_string(),
        description: "Tests result handling".to_string(),
        system_prompt: "Test prompt".to_string(),
        allowed_tools: vec![],
        max_turns: 3,
    };

    let id = orchestrator.spawn(config);
    let result = orchestrator.run(id).await.expect("Run should succeed");

    assert_eq!(result.id, id);
    assert_eq!(result.name, "result-test");
    assert!(result.success);
}

#[tokio::test]
async fn test_subagent_run_nonexistent_fails() {
    let mut orchestrator = SubagentOrchestrator::new();
    let fake_id = uuid::Uuid::new_v4();

    let result = orchestrator.run(fake_id).await;
    assert!(result.is_err(), "Running nonexistent agent should fail");
}

// ============================================================================
// 6.1.1 Subagent Config Tests
// ============================================================================

#[test]
fn test_subagent_config_max_concurrent_default() {
    let orchestrator = SubagentOrchestrator::new();

    // Default max_concurrent should be reasonable (test via active_count behavior)
    assert_eq!(
        orchestrator.active_count(),
        0,
        "New orchestrator should have 0 active agents"
    );
}

#[test]
fn test_subagent_config_max_concurrent_custom() {
    let orchestrator = SubagentOrchestrator::new().with_max_concurrent(8);

    // Verify the orchestrator was configured (indirectly via builder pattern succeeding)
    assert_eq!(orchestrator.active_count(), 0);
}

#[test]
fn test_subagent_active_count_tracks_running() {
    let mut orchestrator = SubagentOrchestrator::new();

    // Initially zero
    assert_eq!(orchestrator.active_count(), 0);

    // Pending agents don't count as active
    let _id = orchestrator.spawn(test_config());
    assert_eq!(
        orchestrator.active_count(),
        0,
        "Pending agents are not active"
    );
}

#[test]
fn test_subagent_mark_failed() {
    let mut orchestrator = SubagentOrchestrator::new();
    let config = test_config();

    let id = orchestrator.spawn(config);
    assert_eq!(orchestrator.get_status(id), Some("pending"));

    let marked = orchestrator.mark_failed(id);
    assert!(marked, "Should return true when marking existing agent");
    assert_eq!(orchestrator.get_status(id), Some("failed"));
}

#[test]
fn test_subagent_mark_failed_nonexistent() {
    let mut orchestrator = SubagentOrchestrator::new();
    let fake_id = uuid::Uuid::new_v4();

    let marked = orchestrator.mark_failed(fake_id);
    assert!(
        !marked,
        "Should return false when marking nonexistent agent"
    );
}

#[test]
fn test_subagent_config_clone() {
    let config = SubagentConfig {
        name: "cloneable".to_string(),
        description: "Test cloning".to_string(),
        system_prompt: "Clone me".to_string(),
        allowed_tools: vec!["tool1".to_string(), "tool2".to_string()],
        max_turns: 7,
    };

    let cloned = config.clone();

    assert_eq!(cloned.name, "cloneable");
    assert_eq!(cloned.description, "Test cloning");
    assert_eq!(cloned.system_prompt, "Clone me");
    assert_eq!(cloned.allowed_tools, vec!["tool1", "tool2"]);
    assert_eq!(cloned.max_turns, 7);
}

#[test]
fn test_subagent_result_debug() {
    use patina::agents::SubagentResult;

    let result = SubagentResult {
        id: uuid::Uuid::nil(),
        name: "debug-test".to_string(),
        output: "test output".to_string(),
        success: true,
    };

    let debug = format!("{:?}", result);
    assert!(debug.contains("debug-test"));
    assert!(debug.contains("test output"));
}

#[test]
fn test_subagent_orchestrator_default() {
    let orchestrator = SubagentOrchestrator::default();
    assert_eq!(orchestrator.active_count(), 0);
}

// ============================================================================
// 6.1.2 Subagent Isolation Tests
// ============================================================================

#[test]
fn test_subagent_context_isolation_separate_configs() {
    let mut orchestrator = SubagentOrchestrator::new();

    let config1 = SubagentConfig {
        name: "agent-1".to_string(),
        description: "First agent".to_string(),
        system_prompt: "You are agent 1.".to_string(),
        allowed_tools: vec!["read".to_string()],
        max_turns: 5,
    };

    let config2 = SubagentConfig {
        name: "agent-2".to_string(),
        description: "Second agent".to_string(),
        system_prompt: "You are agent 2.".to_string(),
        allowed_tools: vec!["write".to_string()],
        max_turns: 10,
    };

    let id1 = orchestrator.spawn(config1);
    let id2 = orchestrator.spawn(config2);

    // Each agent should have its own config retrievable
    let retrieved1 = orchestrator.get_config(id1);
    let retrieved2 = orchestrator.get_config(id2);

    assert!(retrieved1.is_some(), "Agent 1 config should be retrievable");
    assert!(retrieved2.is_some(), "Agent 2 config should be retrievable");

    let cfg1 = retrieved1.unwrap();
    let cfg2 = retrieved2.unwrap();

    assert_eq!(cfg1.name, "agent-1");
    assert_eq!(cfg2.name, "agent-2");
    assert_ne!(
        cfg1.system_prompt, cfg2.system_prompt,
        "Prompts should be different"
    );
}

#[test]
fn test_subagent_context_isolation_independent_state() {
    let mut orchestrator = SubagentOrchestrator::new();

    let config1 = test_config();
    let config2 = test_config();

    let id1 = orchestrator.spawn(config1);
    let id2 = orchestrator.spawn(config2);

    // Mark one as failed, other should remain pending
    orchestrator.mark_failed(id1);

    assert_eq!(orchestrator.get_status(id1), Some("failed"));
    assert_eq!(
        orchestrator.get_status(id2),
        Some("pending"),
        "Agent 2 should remain pending when Agent 1 is marked failed"
    );
}

#[tokio::test]
async fn test_subagent_context_isolation_independent_execution() {
    let mut orchestrator = SubagentOrchestrator::new();

    let config1 = SubagentConfig {
        name: "exec-agent-1".to_string(),
        description: "Execution test 1".to_string(),
        system_prompt: "Test 1".to_string(),
        allowed_tools: vec![],
        max_turns: 3,
    };

    let config2 = SubagentConfig {
        name: "exec-agent-2".to_string(),
        description: "Execution test 2".to_string(),
        system_prompt: "Test 2".to_string(),
        allowed_tools: vec![],
        max_turns: 3,
    };

    let id1 = orchestrator.spawn(config1);
    let id2 = orchestrator.spawn(config2);

    // Run only agent 1
    let result1 = orchestrator.run(id1).await.expect("Run should succeed");

    assert_eq!(result1.name, "exec-agent-1");
    assert_eq!(orchestrator.get_status(id1), Some("completed"));
    assert_eq!(
        orchestrator.get_status(id2),
        Some("pending"),
        "Agent 2 should still be pending"
    );
}

#[test]
fn test_subagent_tool_restrictions_allowed_list() {
    let mut orchestrator = SubagentOrchestrator::new();

    let config = SubagentConfig {
        name: "restricted-agent".to_string(),
        description: "Agent with tool restrictions".to_string(),
        system_prompt: "You have limited tools.".to_string(),
        allowed_tools: vec!["read".to_string(), "grep".to_string()],
        max_turns: 5,
    };

    let id = orchestrator.spawn(config);

    assert!(
        orchestrator.is_tool_allowed(id, "read"),
        "read should be allowed"
    );
    assert!(
        orchestrator.is_tool_allowed(id, "grep"),
        "grep should be allowed"
    );
    assert!(
        !orchestrator.is_tool_allowed(id, "write"),
        "write should NOT be allowed"
    );
    assert!(
        !orchestrator.is_tool_allowed(id, "bash"),
        "bash should NOT be allowed"
    );
}

#[test]
fn test_subagent_tool_restrictions_empty_list() {
    let mut orchestrator = SubagentOrchestrator::new();

    let config = SubagentConfig {
        name: "no-tools-agent".to_string(),
        description: "Agent with no tools".to_string(),
        system_prompt: "You have no tools.".to_string(),
        allowed_tools: vec![],
        max_turns: 3,
    };

    let id = orchestrator.spawn(config);

    assert!(
        !orchestrator.is_tool_allowed(id, "read"),
        "No tools should be allowed"
    );
    assert!(
        !orchestrator.is_tool_allowed(id, "write"),
        "No tools should be allowed"
    );
}

#[test]
fn test_subagent_tool_restrictions_nonexistent_agent() {
    let orchestrator = SubagentOrchestrator::new();
    let fake_id = uuid::Uuid::new_v4();

    // Nonexistent agent should not have any tools allowed
    assert!(
        !orchestrator.is_tool_allowed(fake_id, "read"),
        "Nonexistent agent should not have tools allowed"
    );
}

#[test]
fn test_subagent_tool_restrictions_case_sensitive() {
    let mut orchestrator = SubagentOrchestrator::new();

    let config = SubagentConfig {
        name: "case-test".to_string(),
        description: "Case sensitivity test".to_string(),
        system_prompt: "Test".to_string(),
        allowed_tools: vec!["Read".to_string()],
        max_turns: 3,
    };

    let id = orchestrator.spawn(config);

    assert!(
        orchestrator.is_tool_allowed(id, "Read"),
        "Exact case should match"
    );
    assert!(
        !orchestrator.is_tool_allowed(id, "read"),
        "Different case should not match"
    );
}

#[test]
fn test_subagent_get_allowed_tools() {
    let mut orchestrator = SubagentOrchestrator::new();

    let config = SubagentConfig {
        name: "tools-list-test".to_string(),
        description: "Test allowed tools list".to_string(),
        system_prompt: "Test".to_string(),
        allowed_tools: vec!["read".to_string(), "write".to_string(), "glob".to_string()],
        max_turns: 5,
    };

    let id = orchestrator.spawn(config);

    let tools = orchestrator.get_allowed_tools(id);
    assert!(tools.is_some(), "Should return tools for valid agent");

    let tools = tools.unwrap();
    assert_eq!(tools.len(), 3);
    assert!(tools.contains(&"read".to_string()));
    assert!(tools.contains(&"write".to_string()));
    assert!(tools.contains(&"glob".to_string()));
}

#[test]
fn test_subagent_get_allowed_tools_nonexistent() {
    let orchestrator = SubagentOrchestrator::new();
    let fake_id = uuid::Uuid::new_v4();

    let tools = orchestrator.get_allowed_tools(fake_id);
    assert!(tools.is_none(), "Should return None for nonexistent agent");
}

// ============================================================================
// 6.1.3 Subagent Concurrency Tests
// ============================================================================

#[tokio::test]
async fn test_parallel_subagent_execution() {
    let mut orchestrator = SubagentOrchestrator::new();

    let configs: Vec<_> = (0..3)
        .map(|i| SubagentConfig {
            name: format!("parallel-agent-{}", i),
            description: format!("Parallel test agent {}", i),
            system_prompt: "Test".to_string(),
            allowed_tools: vec![],
            max_turns: 3,
        })
        .collect();

    let ids: Vec<_> = configs.into_iter().map(|c| orchestrator.spawn(c)).collect();

    // All should start as pending
    for id in &ids {
        assert_eq!(orchestrator.get_status(*id), Some("pending"));
    }

    // Run all in sequence (actual parallel would require async spawning)
    for id in &ids {
        let result = orchestrator.run(*id).await;
        assert!(result.is_ok(), "Each run should succeed");
    }

    // All should be completed
    for id in &ids {
        assert_eq!(orchestrator.get_status(*id), Some("completed"));
    }
}

#[tokio::test]
async fn test_parallel_subagent_independent_results() {
    let mut orchestrator = SubagentOrchestrator::new();

    let config1 = SubagentConfig {
        name: "result-1".to_string(),
        description: "First".to_string(),
        system_prompt: "Test".to_string(),
        allowed_tools: vec!["read".to_string()],
        max_turns: 5,
    };

    let config2 = SubagentConfig {
        name: "result-2".to_string(),
        description: "Second".to_string(),
        system_prompt: "Test".to_string(),
        allowed_tools: vec!["write".to_string()],
        max_turns: 10,
    };

    let id1 = orchestrator.spawn(config1);
    let id2 = orchestrator.spawn(config2);

    let result1 = orchestrator.run(id1).await.expect("Run 1 should succeed");
    let result2 = orchestrator.run(id2).await.expect("Run 2 should succeed");

    // Results should have correct agent names
    assert_eq!(result1.name, "result-1");
    assert_eq!(result2.name, "result-2");

    // IDs should match
    assert_eq!(result1.id, id1);
    assert_eq!(result2.id, id2);
}

#[test]
fn test_subagent_max_turns_config() {
    let mut orchestrator = SubagentOrchestrator::new();

    let config = SubagentConfig {
        name: "max-turns-test".to_string(),
        description: "Test max turns".to_string(),
        system_prompt: "Test".to_string(),
        allowed_tools: vec![],
        max_turns: 15,
    };

    let id = orchestrator.spawn(config);

    let retrieved = orchestrator.get_config(id).expect("Config should exist");
    assert_eq!(
        retrieved.max_turns, 15,
        "max_turns should be preserved in config"
    );
}

#[test]
fn test_subagent_max_turns_different_values() {
    let mut orchestrator = SubagentOrchestrator::new();

    let configs = vec![
        SubagentConfig {
            name: "short-agent".to_string(),
            description: "Short".to_string(),
            system_prompt: "Test".to_string(),
            allowed_tools: vec![],
            max_turns: 3,
        },
        SubagentConfig {
            name: "medium-agent".to_string(),
            description: "Medium".to_string(),
            system_prompt: "Test".to_string(),
            allowed_tools: vec![],
            max_turns: 10,
        },
        SubagentConfig {
            name: "long-agent".to_string(),
            description: "Long".to_string(),
            system_prompt: "Test".to_string(),
            allowed_tools: vec![],
            max_turns: 50,
        },
    ];

    let ids: Vec<_> = configs.into_iter().map(|c| orchestrator.spawn(c)).collect();

    assert_eq!(orchestrator.get_config(ids[0]).unwrap().max_turns, 3);
    assert_eq!(orchestrator.get_config(ids[1]).unwrap().max_turns, 10);
    assert_eq!(orchestrator.get_config(ids[2]).unwrap().max_turns, 50);
}

#[test]
fn test_subagent_max_concurrent_limit() {
    let orchestrator = SubagentOrchestrator::new().with_max_concurrent(2);

    // Verify orchestrator was configured (max_concurrent getter needed)
    assert_eq!(orchestrator.max_concurrent(), 2);
}

#[test]
fn test_subagent_max_concurrent_default_value() {
    let orchestrator = SubagentOrchestrator::new();

    // Default should be 4
    assert_eq!(orchestrator.max_concurrent(), 4);
}

#[test]
fn test_subagent_can_spawn_returns_correct_value() {
    let mut orchestrator = SubagentOrchestrator::new().with_max_concurrent(2);

    // Initially, should be able to spawn
    assert!(
        orchestrator.can_spawn(),
        "Should be able to spawn when under limit"
    );

    // After spawning agents (they're pending, not running)
    let _id1 = orchestrator.spawn(test_config());
    let _id2 = orchestrator.spawn(test_config());

    // Pending agents don't count against concurrent limit
    assert!(
        orchestrator.can_spawn(),
        "Pending agents don't count against limit"
    );
}

#[tokio::test]
async fn test_subagent_running_counts_against_concurrent() {
    let mut orchestrator = SubagentOrchestrator::new().with_max_concurrent(2);

    let config1 = test_config();
    let config2 = test_config();

    let id1 = orchestrator.spawn(config1);
    let id2 = orchestrator.spawn(config2);

    // Initially both pending
    assert_eq!(orchestrator.active_count(), 0);

    // After running, they complete immediately in our simple impl
    // So active_count goes 0 -> 1 -> 0
    let _ = orchestrator.run(id1).await;
    let _ = orchestrator.run(id2).await;

    // After completion, active count returns to 0
    assert_eq!(orchestrator.active_count(), 0);
}

#[test]
fn test_subagent_list_all_agents() {
    let mut orchestrator = SubagentOrchestrator::new();

    let ids: Vec<_> = (0..5)
        .map(|i| {
            orchestrator.spawn(SubagentConfig {
                name: format!("agent-{}", i),
                description: format!("Agent {}", i),
                system_prompt: "Test".to_string(),
                allowed_tools: vec![],
                max_turns: 3,
            })
        })
        .collect();

    let all_ids = orchestrator.list_agents();
    assert_eq!(all_ids.len(), 5, "Should have 5 agents");

    // All spawned IDs should be in the list
    for id in &ids {
        assert!(all_ids.contains(id), "All spawned IDs should be listed");
    }
}

#[test]
fn test_subagent_list_all_agents_empty() {
    let orchestrator = SubagentOrchestrator::new();

    let all_ids = orchestrator.list_agents();
    assert!(all_ids.is_empty(), "New orchestrator should have no agents");
}

#[test]
fn test_subagent_remove_completed() {
    let mut orchestrator = SubagentOrchestrator::new();

    let id = orchestrator.spawn(test_config());
    assert!(orchestrator.get_status(id).is_some());

    // Remove the agent
    let removed = orchestrator.remove_agent(id);
    assert!(removed, "Should return true when removing existing agent");

    // Agent should no longer exist
    assert!(
        orchestrator.get_status(id).is_none(),
        "Removed agent should not be retrievable"
    );
}

#[test]
fn test_subagent_remove_nonexistent() {
    let mut orchestrator = SubagentOrchestrator::new();
    let fake_id = uuid::Uuid::new_v4();

    let removed = orchestrator.remove_agent(fake_id);
    assert!(
        !removed,
        "Should return false when removing nonexistent agent"
    );
}
