//! Unified application event types for the event-driven architecture.
//!
//! `AppEvent` is the single event type that all event sources (crossterm input,
//! API streaming, background tasks, timers) produce. The event loop receives
//! `AppEvent` values and dispatches them to handlers.
//!
//! This module is a pure addition — no existing code is modified.

use crossterm::event::{KeyEvent, MouseEvent};

use crate::agents::AgentEvent;
use crate::api::StreamEvent;
use crate::continuous::ContinuousEvent;
use crate::permissions::PermissionResponse;
use crate::types::ToolResultBlock;

/// A unified application event produced by all event sources.
///
/// The event loop merges crossterm terminal events, API streaming events,
/// background tool results, and timer ticks into this single enum.
/// Each variant maps to a specific handler in the `EventDispatcher`.
///
/// # Examples
///
/// ```rust,ignore
/// use patina::app::events::AppEvent;
/// use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
///
/// let event = AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
/// assert!(event.is_input());
/// assert!(!event.is_background());
/// ```
#[derive(Debug)]
pub enum AppEvent {
    /// A keyboard event from crossterm.
    Key(KeyEvent),

    /// A mouse event from crossterm.
    Mouse(MouseEvent),

    /// The terminal was resized.
    Resize {
        /// New terminal width in columns.
        width: u16,
        /// New terminal height in rows.
        height: u16,
    },

    /// An API streaming chunk was received.
    ApiChunk(StreamEvent),

    /// A tool execution completed with its result.
    ToolResult {
        /// The tool use ID correlating this result to its request.
        tool_id: String,
        /// The tool execution result.
        result: ToolResultBlock,
    },

    /// A periodic tick for animations (throbber, progress indicators).
    Tick,

    /// A request to quit the application.
    Quit,

    /// A user response to a permission prompt.
    PermissionResponse(PermissionResponse),

    /// An event from a background agent (progress, completion, conflict).
    Agent(AgentEvent),

    /// An event from the continuous coding loop (progress, gates, stagnation).
    Continuous(ContinuousEvent),
}

impl AppEvent {
    /// Returns `true` if this is a user input event (key, mouse, or resize).
    #[must_use]
    pub fn is_input(&self) -> bool {
        matches!(self, Self::Key(_) | Self::Mouse(_) | Self::Resize { .. })
    }

    /// Returns `true` if this is a background event (API chunk, tool result, or agent event).
    #[must_use]
    pub fn is_background(&self) -> bool {
        matches!(
            self,
            Self::ApiChunk(_) | Self::ToolResult { .. } | Self::Agent(_) | Self::Continuous(_)
        )
    }

    /// Returns `true` if this event signals application termination.
    #[must_use]
    pub fn is_quit(&self) -> bool {
        matches!(self, Self::Quit)
    }
}

impl std::fmt::Display for AppEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Key(key) => write!(f, "Key({:?}+{:?})", key.code, key.modifiers),
            Self::Mouse(mouse) => write!(f, "Mouse({:?})", mouse.kind),
            Self::Resize { width, height } => write!(f, "Resize({width}x{height})"),
            Self::ApiChunk(event) => write!(f, "ApiChunk({event:?})"),
            Self::ToolResult { tool_id, .. } => write!(f, "ToolResult({tool_id})"),
            Self::Tick => write!(f, "Tick"),
            Self::Quit => write!(f, "Quit"),
            Self::PermissionResponse(resp) => write!(f, "PermissionResponse({resp:?})"),
            Self::Agent(agent_event) => write!(f, "Agent({agent_event:?})"),
            Self::Continuous(event) => write!(f, "Continuous({})", event.event_type()),
        }
    }
}

impl From<crossterm::event::Event> for AppEvent {
    fn from(event: crossterm::event::Event) -> Self {
        match event {
            crossterm::event::Event::Key(key) => Self::Key(key),
            crossterm::event::Event::Mouse(mouse) => Self::Mouse(mouse),
            crossterm::event::Event::Resize(width, height) => Self::Resize { width, height },
            // FocusGained/FocusLost and Paste are mapped to Tick (no-op) since we
            // don't handle them but the enum must be exhaustive.
            _ => Self::Tick,
        }
    }
}

impl From<StreamEvent> for AppEvent {
    fn from(event: StreamEvent) -> Self {
        Self::ApiChunk(event)
    }
}

impl From<PermissionResponse> for AppEvent {
    fn from(response: PermissionResponse) -> Self {
        Self::PermissionResponse(response)
    }
}

impl From<AgentEvent> for AppEvent {
    fn from(event: AgentEvent) -> Self {
        Self::Agent(event)
    }
}

impl From<ContinuousEvent> for AppEvent {
    fn from(event: ContinuousEvent) -> Self {
        Self::Continuous(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::AgentProgress;
    use crate::continuous::ContinuousEvent;
    use crossterm::event::{
        KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseButton, MouseEventKind,
    };

    // =========================================================================
    // Construction tests
    // =========================================================================

    #[test]
    fn key_event_from_crossterm() {
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let event = AppEvent::Key(key);
        assert!(matches!(event, AppEvent::Key(_)));
    }

    #[test]
    fn mouse_event_from_crossterm() {
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        let event = AppEvent::Mouse(mouse);
        assert!(matches!(event, AppEvent::Mouse(_)));
    }

    #[test]
    fn resize_event_stores_dimensions() {
        let event = AppEvent::Resize {
            width: 120,
            height: 40,
        };
        match event {
            AppEvent::Resize { width, height } => {
                assert_eq!(width, 120);
                assert_eq!(height, 40);
            }
            _ => panic!("Expected Resize variant"),
        }
    }

    #[test]
    fn api_chunk_wraps_stream_event() {
        let stream = StreamEvent::ContentDelta("hello".to_string());
        let event = AppEvent::ApiChunk(stream);
        match event {
            AppEvent::ApiChunk(StreamEvent::ContentDelta(text)) => {
                assert_eq!(text, "hello");
            }
            _ => panic!("Expected ApiChunk(ContentDelta)"),
        }
    }

    #[test]
    fn tool_result_stores_id_and_block() {
        let result = ToolResultBlock {
            tool_use_id: "toolu_123".to_string(),
            content: "output".to_string(),
            is_error: false,
        };
        let event = AppEvent::ToolResult {
            tool_id: "toolu_123".to_string(),
            result,
        };
        match event {
            AppEvent::ToolResult { tool_id, result } => {
                assert_eq!(tool_id, "toolu_123");
                assert_eq!(result.content, "output");
                assert!(!result.is_error);
            }
            _ => panic!("Expected ToolResult"),
        }
    }

    #[test]
    fn tick_event_construction() {
        let event = AppEvent::Tick;
        assert!(matches!(event, AppEvent::Tick));
    }

    #[test]
    fn quit_event_construction() {
        let event = AppEvent::Quit;
        assert!(matches!(event, AppEvent::Quit));
    }

    #[test]
    fn permission_response_construction() {
        let event = AppEvent::PermissionResponse(PermissionResponse::AllowOnce);
        assert!(matches!(
            event,
            AppEvent::PermissionResponse(PermissionResponse::AllowOnce)
        ));
    }

    // =========================================================================
    // Classification tests (is_input, is_background, is_quit)
    // =========================================================================

    #[test]
    fn key_is_input() {
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        assert!(event.is_input());
        assert!(!event.is_background());
        assert!(!event.is_quit());
    }

    #[test]
    fn mouse_is_input() {
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        let event = AppEvent::Mouse(mouse);
        assert!(event.is_input());
        assert!(!event.is_background());
    }

    #[test]
    fn resize_is_input() {
        let event = AppEvent::Resize {
            width: 80,
            height: 24,
        };
        assert!(event.is_input());
        assert!(!event.is_background());
    }

    #[test]
    fn api_chunk_is_background() {
        let event = AppEvent::ApiChunk(StreamEvent::ContentDelta("x".to_string()));
        assert!(event.is_background());
        assert!(!event.is_input());
    }

    #[test]
    fn tool_result_is_background() {
        let event = AppEvent::ToolResult {
            tool_id: "id".to_string(),
            result: ToolResultBlock {
                tool_use_id: "id".to_string(),
                content: "ok".to_string(),
                is_error: false,
            },
        };
        assert!(event.is_background());
        assert!(!event.is_input());
    }

    #[test]
    fn tick_is_neither_input_nor_background() {
        let event = AppEvent::Tick;
        assert!(!event.is_input());
        assert!(!event.is_background());
        assert!(!event.is_quit());
    }

    #[test]
    fn quit_is_quit() {
        let event = AppEvent::Quit;
        assert!(event.is_quit());
        assert!(!event.is_input());
        assert!(!event.is_background());
    }

    #[test]
    fn permission_response_is_neither_input_nor_background() {
        let event = AppEvent::PermissionResponse(PermissionResponse::Deny);
        assert!(!event.is_input());
        assert!(!event.is_background());
        assert!(!event.is_quit());
    }

    // =========================================================================
    // Display tests
    // =========================================================================

    #[test]
    fn display_key_event() {
        let event = AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let s = format!("{event}");
        assert!(s.contains("Key("));
        assert!(s.contains("Enter"));
    }

    #[test]
    fn display_resize_event() {
        let event = AppEvent::Resize {
            width: 120,
            height: 40,
        };
        assert_eq!(format!("{event}"), "Resize(120x40)");
    }

    #[test]
    fn display_tick() {
        assert_eq!(format!("{}", AppEvent::Tick), "Tick");
    }

    #[test]
    fn display_quit() {
        assert_eq!(format!("{}", AppEvent::Quit), "Quit");
    }

    #[test]
    fn display_tool_result() {
        let event = AppEvent::ToolResult {
            tool_id: "toolu_abc".to_string(),
            result: ToolResultBlock {
                tool_use_id: "toolu_abc".to_string(),
                content: "ok".to_string(),
                is_error: false,
            },
        };
        let s = format!("{event}");
        assert!(s.contains("ToolResult(toolu_abc)"));
    }

    #[test]
    fn display_permission_response() {
        let event = AppEvent::PermissionResponse(PermissionResponse::AllowAlways);
        let s = format!("{event}");
        assert!(s.contains("PermissionResponse"));
    }

    // =========================================================================
    // From conversion tests
    // =========================================================================

    #[test]
    fn from_crossterm_key_event() {
        let crossterm_event =
            crossterm::event::Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
        let app_event = AppEvent::from(crossterm_event);
        match app_event {
            AppEvent::Key(key) => {
                assert_eq!(key.code, KeyCode::Char('x'));
                assert_eq!(key.modifiers, KeyModifiers::CONTROL);
            }
            _ => panic!("Expected Key variant"),
        }
    }

    #[test]
    fn from_crossterm_mouse_event() {
        let mouse = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 5,
            row: 10,
            modifiers: KeyModifiers::NONE,
        };
        let crossterm_event = crossterm::event::Event::Mouse(mouse);
        let app_event = AppEvent::from(crossterm_event);
        assert!(matches!(app_event, AppEvent::Mouse(_)));
    }

    #[test]
    fn from_crossterm_resize_event() {
        let crossterm_event = crossterm::event::Event::Resize(200, 50);
        let app_event = AppEvent::from(crossterm_event);
        match app_event {
            AppEvent::Resize { width, height } => {
                assert_eq!(width, 200);
                assert_eq!(height, 50);
            }
            _ => panic!("Expected Resize variant"),
        }
    }

    #[test]
    fn from_crossterm_focus_maps_to_tick() {
        let crossterm_event = crossterm::event::Event::FocusGained;
        let app_event = AppEvent::from(crossterm_event);
        assert!(matches!(app_event, AppEvent::Tick));
    }

    #[test]
    fn from_stream_event() {
        let stream = StreamEvent::Error("test error".to_string());
        let app_event = AppEvent::from(stream);
        match app_event {
            AppEvent::ApiChunk(StreamEvent::Error(msg)) => {
                assert_eq!(msg, "test error");
            }
            _ => panic!("Expected ApiChunk(Error)"),
        }
    }

    #[test]
    fn from_permission_response() {
        let resp = PermissionResponse::Deny;
        let app_event = AppEvent::from(resp);
        assert!(matches!(
            app_event,
            AppEvent::PermissionResponse(PermissionResponse::Deny)
        ));
    }

    #[test]
    fn from_agent_event() {
        let agent_evt = AgentEvent::Progress {
            agent_id: "a1".to_string(),
            agent_name: "explorer".to_string(),
            progress: AgentProgress::IterationStarted {
                iteration: 1,
                max: 10,
            },
        };
        let app_event = AppEvent::from(agent_evt);
        assert!(matches!(
            app_event,
            AppEvent::Agent(AgentEvent::Progress { .. })
        ));
    }

    // =========================================================================
    // Agent event construction and classification
    // =========================================================================

    #[test]
    fn agent_event_construction() {
        let event = AppEvent::Agent(AgentEvent::Progress {
            agent_id: "a1".to_string(),
            agent_name: "explorer".to_string(),
            progress: AgentProgress::Completed {
                output: "done".to_string(),
                iterations_used: 3,
            },
        });
        assert!(matches!(event, AppEvent::Agent(_)));
    }

    #[test]
    fn agent_event_is_background() {
        let event = AppEvent::Agent(AgentEvent::Progress {
            agent_id: "a1".to_string(),
            agent_name: "explorer".to_string(),
            progress: AgentProgress::IterationStarted {
                iteration: 1,
                max: 5,
            },
        });
        assert!(event.is_background());
        assert!(!event.is_input());
        assert!(!event.is_quit());
    }

    #[test]
    fn agent_conflict_event_is_background() {
        use crate::agents::ConflictReport;
        let event = AppEvent::Agent(AgentEvent::ConflictDetected {
            report: ConflictReport::empty(),
        });
        assert!(event.is_background());
    }

    #[test]
    fn display_agent_event() {
        let event = AppEvent::Agent(AgentEvent::Progress {
            agent_id: "a1".to_string(),
            agent_name: "explorer".to_string(),
            progress: AgentProgress::IterationStarted {
                iteration: 2,
                max: 10,
            },
        });
        let s = format!("{event}");
        assert!(s.contains("Agent("));
    }

    // =========================================================================
    // Continuous event construction and classification
    // =========================================================================

    #[test]
    fn continuous_event_construction() {
        let event = AppEvent::Continuous(ContinuousEvent::IterationStart { iteration: 1 });
        assert!(matches!(event, AppEvent::Continuous(_)));
    }

    #[test]
    fn continuous_event_is_background() {
        let event = AppEvent::Continuous(ContinuousEvent::IterationStart { iteration: 1 });
        assert!(event.is_background());
        assert!(!event.is_input());
        assert!(!event.is_quit());
    }

    #[test]
    fn continuous_stagnation_event_is_background() {
        let event = AppEvent::Continuous(ContinuousEvent::StagnationDetected {
            iterations_without_progress: 5,
            threshold: 3,
        });
        assert!(event.is_background());
    }

    #[test]
    fn display_continuous_event() {
        let event = AppEvent::Continuous(ContinuousEvent::IterationStart { iteration: 2 });
        let s = format!("{event}");
        assert!(s.contains("Continuous("));
        assert!(s.contains("iteration_start"));
    }

    #[test]
    fn from_continuous_event() {
        let cont_evt = ContinuousEvent::QualityGateCheck {
            gate: "tests".to_string(),
        };
        let app_event = AppEvent::from(cont_evt);
        assert!(matches!(
            app_event,
            AppEvent::Continuous(ContinuousEvent::QualityGateCheck { .. })
        ));
    }

    // =========================================================================
    // Send + Sync assertions
    // =========================================================================

    #[test]
    fn app_event_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<AppEvent>();
    }

    #[test]
    fn app_event_is_sync() {
        fn assert_sync<T: Sync>() {}
        assert_sync::<AppEvent>();
    }

    // =========================================================================
    // Pattern matching exhaustiveness (ensures all variants are covered)
    // =========================================================================

    #[test]
    fn all_variants_matchable() {
        let events: Vec<AppEvent> = vec![
            AppEvent::Key(KeyEvent {
                code: KeyCode::Char('a'),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }),
            AppEvent::Mouse(MouseEvent {
                kind: MouseEventKind::Moved,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            }),
            AppEvent::Resize {
                width: 80,
                height: 24,
            },
            AppEvent::ApiChunk(StreamEvent::MessageStop),
            AppEvent::ToolResult {
                tool_id: "t".to_string(),
                result: ToolResultBlock {
                    tool_use_id: "t".to_string(),
                    content: String::new(),
                    is_error: false,
                },
            },
            AppEvent::Tick,
            AppEvent::Quit,
            AppEvent::PermissionResponse(PermissionResponse::AllowOnce),
            AppEvent::Agent(AgentEvent::Progress {
                agent_id: "test".to_string(),
                agent_name: "test-agent".to_string(),
                progress: AgentProgress::IterationStarted {
                    iteration: 1,
                    max: 10,
                },
            }),
            AppEvent::Continuous(ContinuousEvent::IterationStart { iteration: 1 }),
        ];

        // Every variant should match exactly one arm
        for event in events {
            let matched = match event {
                AppEvent::Key(_) => "key",
                AppEvent::Mouse(_) => "mouse",
                AppEvent::Resize { .. } => "resize",
                AppEvent::ApiChunk(_) => "api_chunk",
                AppEvent::ToolResult { .. } => "tool_result",
                AppEvent::Tick => "tick",
                AppEvent::Quit => "quit",
                AppEvent::PermissionResponse(_) => "permission",
                AppEvent::Agent(_) => "agent",
                AppEvent::Continuous(_) => "continuous",
            };
            assert!(!matched.is_empty());
        }
    }
}
