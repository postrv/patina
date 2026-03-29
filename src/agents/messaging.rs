//! Queue-based inter-agent messaging system.
//!
//! Provides a [`MessageRouter`] that maintains per-agent message queues,
//! allowing running agents to communicate with each other during execution.
//! Messages are stored in FIFO order and drained on receive.
//!
//! # Example
//!
//! ```
//! use patina::agents::messaging::{MessageRouter, AgentMessage};
//!
//! let router = MessageRouter::new();
//! router.register_agent("agent-1");
//! router.register_agent("agent-2");
//!
//! router.send("agent-2", "agent-1", "Hello from agent 2").unwrap();
//!
//! assert!(router.has_messages("agent-1"));
//! let msgs = router.receive("agent-1");
//! assert_eq!(msgs.len(), 1);
//! assert_eq!(msgs[0].content, "Hello from agent 2");
//! ```

use anyhow::{bail, Result};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::SystemTime;

/// A message routed between agents via the [`MessageRouter`].
///
/// Each message records its sender, recipient, content, and the time it was created.
///
/// # Examples
///
/// ```
/// use patina::agents::messaging::AgentMessage;
/// use std::time::SystemTime;
///
/// let msg = AgentMessage {
///     from: "explorer".to_string(),
///     to: "writer".to_string(),
///     content: "Found 3 relevant files".to_string(),
///     timestamp: SystemTime::now(),
/// };
/// assert_eq!(msg.from, "explorer");
/// assert_eq!(msg.to, "writer");
/// ```
#[derive(Debug, Clone)]
pub struct AgentMessage {
    /// The ID or name of the sending agent.
    pub from: String,
    /// The ID or name of the receiving agent.
    pub to: String,
    /// The message content.
    pub content: String,
    /// When the message was created.
    pub timestamp: SystemTime,
}

/// Thread-safe queue-based router for inter-agent messaging.
///
/// Agents must be registered before they can send or receive messages.
/// Each registered agent gets an independent FIFO message queue.
/// Calling [`receive`](MessageRouter::receive) drains all pending messages
/// from the agent's queue.
///
/// # Thread Safety
///
/// All methods acquire an internal [`Mutex`] and are safe to call from
/// multiple threads or async tasks concurrently.
///
/// # Examples
///
/// ```
/// use patina::agents::messaging::MessageRouter;
///
/// let router = MessageRouter::new();
/// router.register_agent("a");
/// router.register_agent("b");
///
/// router.send("a", "b", "ping").unwrap();
/// let msgs = router.receive("b");
/// assert_eq!(msgs.len(), 1);
/// assert_eq!(msgs[0].content, "ping");
///
/// // Queue is drained after receive
/// assert!(!router.has_messages("b"));
/// ```
pub struct MessageRouter {
    /// Per-agent message queues, keyed by agent ID.
    queues: Mutex<HashMap<String, Vec<AgentMessage>>>,
}

impl MessageRouter {
    /// Creates a new empty message router with no registered agents.
    #[must_use]
    pub fn new() -> Self {
        Self {
            queues: Mutex::new(HashMap::new()),
        }
    }

    /// Registers an agent so it can send and receive messages.
    ///
    /// If the agent is already registered, this is a no-op (existing
    /// queued messages are preserved).
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn register_agent(&self, agent_id: &str) {
        let mut queues = self.queues.lock().unwrap_or_else(|e| e.into_inner());
        queues.entry(agent_id.to_string()).or_default();
    }

    /// Unregisters an agent, removing it and any pending messages.
    ///
    /// Returns `true` if the agent was registered and has been removed,
    /// `false` if the agent was not registered.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn unregister_agent(&self, agent_id: &str) -> bool {
        let mut queues = self.queues.lock().unwrap_or_else(|e| e.into_inner());
        queues.remove(agent_id).is_some()
    }

    /// Returns `true` if the given agent ID is registered.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    #[must_use]
    pub fn agent_exists(&self, agent_id: &str) -> bool {
        let queues = self.queues.lock().unwrap_or_else(|e| e.into_inner());
        queues.contains_key(agent_id)
    }

    /// Sends a message from one agent to another.
    ///
    /// Both the sender and receiver must be registered. The message is
    /// appended to the receiver's queue in FIFO order.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The sender (`from`) is not a registered agent
    /// - The receiver (`to`) is not a registered agent
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn send(&self, from: &str, to: &str, content: &str) -> Result<()> {
        let mut queues = self.queues.lock().unwrap_or_else(|e| e.into_inner());

        if !queues.contains_key(from) {
            bail!("Sender agent '{from}' is not registered");
        }
        if !queues.contains_key(to) {
            bail!("Target agent '{to}' is not registered");
        }

        let message = AgentMessage {
            from: from.to_string(),
            to: to.to_string(),
            content: content.to_string(),
            timestamp: SystemTime::now(),
        };

        queues
            .get_mut(to)
            .expect("target queue verified above")
            .push(message);

        Ok(())
    }

    /// Drains and returns all pending messages for the given agent.
    ///
    /// Messages are returned in FIFO order (oldest first). After this call
    /// the agent's queue is empty.
    ///
    /// Returns an empty `Vec` if the agent is not registered or has no
    /// pending messages.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    #[must_use]
    pub fn receive(&self, agent_id: &str) -> Vec<AgentMessage> {
        let mut queues = self.queues.lock().unwrap_or_else(|e| e.into_inner());
        queues
            .get_mut(agent_id)
            .map(std::mem::take)
            .unwrap_or_default()
    }

    /// Returns `true` if the agent has one or more pending messages.
    ///
    /// Returns `false` if the agent is not registered or has no messages.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    #[must_use]
    pub fn has_messages(&self, agent_id: &str) -> bool {
        let queues = self.queues.lock().unwrap_or_else(|e| e.into_inner());
        queues.get(agent_id).is_some_and(|q| !q.is_empty())
    }
}

impl Default for MessageRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for MessageRouter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let queues = self.queues.lock().unwrap_or_else(|e| e.into_inner());
        let agent_ids: Vec<&String> = queues.keys().collect();
        let total_messages: usize = queues.values().map(Vec::len).sum();
        f.debug_struct("MessageRouter")
            .field("registered_agents", &agent_ids.len())
            .field("agent_ids", &agent_ids)
            .field("total_pending_messages", &total_messages)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_send_and_receive_flow() {
        let router = MessageRouter::new();
        router.register_agent("alice");
        router.register_agent("bob");

        router.send("alice", "bob", "Hello Bob").unwrap();

        let messages = router.receive("bob");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].from, "alice");
        assert_eq!(messages[0].to, "bob");
        assert_eq!(messages[0].content, "Hello Bob");
    }

    #[test]
    fn test_message_ordering_fifo() {
        let router = MessageRouter::new();
        router.register_agent("sender");
        router.register_agent("receiver");

        router.send("sender", "receiver", "first").unwrap();
        router.send("sender", "receiver", "second").unwrap();
        router.send("sender", "receiver", "third").unwrap();

        let messages = router.receive("receiver");
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].content, "first");
        assert_eq!(messages[1].content, "second");
        assert_eq!(messages[2].content, "third");
    }

    #[test]
    fn test_send_to_nonexistent_agent_returns_error() {
        let router = MessageRouter::new();
        router.register_agent("sender");

        let result = router.send("sender", "ghost", "hello");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("ghost"),
            "Error should mention the missing agent: {err_msg}"
        );
        assert!(
            err_msg.contains("not registered"),
            "Error should say 'not registered': {err_msg}"
        );
    }

    #[test]
    fn test_send_from_nonexistent_agent_returns_error() {
        let router = MessageRouter::new();
        router.register_agent("receiver");

        let result = router.send("ghost", "receiver", "hello");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("ghost"),
            "Error should mention the missing sender: {err_msg}"
        );
    }

    #[test]
    fn test_drain_semantics_receive_clears_queue() {
        let router = MessageRouter::new();
        router.register_agent("a");
        router.register_agent("b");

        router.send("a", "b", "message 1").unwrap();
        router.send("a", "b", "message 2").unwrap();

        let first_batch = router.receive("b");
        assert_eq!(first_batch.len(), 2);

        // Queue should be empty after drain
        let second_batch = router.receive("b");
        assert!(second_batch.is_empty());
        assert!(!router.has_messages("b"));
    }

    #[test]
    fn test_register_unregister_lifecycle() {
        let router = MessageRouter::new();

        // Not registered yet
        assert!(!router.agent_exists("agent-1"));

        // Register
        router.register_agent("agent-1");
        assert!(router.agent_exists("agent-1"));

        // Send a message
        router.register_agent("agent-2");
        router.send("agent-2", "agent-1", "hello").unwrap();
        assert!(router.has_messages("agent-1"));

        // Unregister drops pending messages
        assert!(router.unregister_agent("agent-1"));
        assert!(!router.agent_exists("agent-1"));

        // Unregistering again returns false
        assert!(!router.unregister_agent("agent-1"));
    }

    #[test]
    fn test_register_is_idempotent() {
        let router = MessageRouter::new();
        router.register_agent("agent-1");
        router.register_agent("agent-2");

        router
            .send("agent-2", "agent-1", "before re-register")
            .unwrap();

        // Re-registering should preserve existing messages
        router.register_agent("agent-1");
        assert!(router.has_messages("agent-1"));

        let messages = router.receive("agent-1");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "before re-register");
    }

    #[test]
    fn test_has_messages_false_for_unregistered() {
        let router = MessageRouter::new();
        assert!(!router.has_messages("nobody"));
    }

    #[test]
    fn test_receive_empty_for_unregistered() {
        let router = MessageRouter::new();
        let messages = router.receive("nobody");
        assert!(messages.is_empty());
    }

    #[test]
    fn test_message_has_timestamp() {
        let before = SystemTime::now();
        let router = MessageRouter::new();
        router.register_agent("a");
        router.register_agent("b");
        router.send("a", "b", "timestamped").unwrap();
        let after = SystemTime::now();

        let messages = router.receive("b");
        assert_eq!(messages.len(), 1);

        let ts = messages[0].timestamp;
        assert!(ts >= before, "Timestamp should be >= test start");
        assert!(ts <= after, "Timestamp should be <= test end");
    }

    #[test]
    fn test_multiple_senders_to_one_receiver() {
        let router = MessageRouter::new();
        router.register_agent("a");
        router.register_agent("b");
        router.register_agent("c");

        router.send("a", "c", "from a").unwrap();
        router.send("b", "c", "from b").unwrap();

        let messages = router.receive("c");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].from, "a");
        assert_eq!(messages[1].from, "b");
    }

    #[test]
    fn test_debug_format() {
        let router = MessageRouter::new();
        router.register_agent("x");
        let debug_str = format!("{router:?}");
        assert!(debug_str.contains("MessageRouter"));
        assert!(debug_str.contains("registered_agents"));
        assert!(debug_str.contains("total_pending_messages"));
    }

    #[test]
    fn test_default_creates_empty_router() {
        let router = MessageRouter::default();
        assert!(!router.agent_exists("anything"));
    }

    #[tokio::test]
    async fn test_concurrent_access() {
        use std::sync::Arc;

        let router = Arc::new(MessageRouter::new());
        router.register_agent("receiver");

        // Spawn multiple tasks that send concurrently
        let mut handles = Vec::new();
        for i in 0..10 {
            let r = Arc::clone(&router);
            let sender_id = format!("sender-{i}");
            r.register_agent(&sender_id);
            let handle = tokio::spawn(async move {
                let msg = format!("message from {sender_id}");
                r.send(&sender_id, "receiver", &msg).unwrap();
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.await.unwrap();
        }

        let messages = router.receive("receiver");
        assert_eq!(messages.len(), 10, "All 10 messages should arrive");

        // Verify all senders are represented
        let mut senders: Vec<String> = messages.iter().map(|m| m.from.clone()).collect();
        senders.sort();
        for i in 0..10 {
            assert!(
                senders.contains(&format!("sender-{i}")),
                "Missing sender-{i}"
            );
        }
    }
}
