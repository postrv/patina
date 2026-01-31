//! Rendering performance benchmarks
//!
//! These benchmarks measure the performance of critical rendering paths:
//! - Full redraw with many messages
//! - Streaming token append
//! - Input character echo
//!
//! Performance targets:
//! - `full_redraw_100_messages`: <1ms
//! - `streaming_token_append`: <100μs
//! - `input_character_echo`: <10μs

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use patina::app::state::AppState;
use patina::tui::render;
use patina::types::message::{Message, Role};
use std::path::PathBuf;

/// Creates an AppState populated with the specified number of messages.
fn create_state_with_messages(message_count: usize) -> AppState {
    let mut state = AppState::new(PathBuf::from("/tmp"));

    for i in 0..message_count {
        let role = if i % 2 == 0 {
            Role::User
        } else {
            Role::Assistant
        };

        let content = format!(
            "This is message number {} with some content that spans multiple words \
             to simulate realistic message lengths. The message contains various \
             information that would be typical in a conversation with Claude.",
            i
        );

        state.add_message(Message { role, content });
    }

    state
}

/// Benchmark: Full redraw with 100 messages
/// Target: <1ms
fn bench_full_redraw_100_messages(c: &mut Criterion) {
    let state = create_state_with_messages(100);
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).expect("Failed to create terminal");

    c.bench_function("full_redraw_100_messages", |b| {
        b.iter(|| {
            terminal
                .draw(|frame| {
                    render(frame, black_box(&state));
                })
                .expect("Failed to draw");
        });
    });
}

/// Benchmark: Streaming token append (just the data operation)
/// Target: <100μs
///
/// Measures the performance of appending a single token during streaming.
/// This measures just the data operation, not the render.
fn bench_streaming_token_append(c: &mut Criterion) {
    c.bench_function("streaming_token_append", |b| {
        b.iter_batched(
            || {
                // Setup: Create state with some messages and a streaming response
                let mut state = create_state_with_messages(10);
                // Simulate streaming state by setting current_response
                state.current_response = Some(String::with_capacity(4096));
                state
            },
            |mut state| {
                // Append a token (the core operation during streaming)
                if let Some(ref mut response) = state.current_response {
                    response.push_str(black_box("token "));
                }
                black_box(&state);
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

/// Benchmark: Full streaming cycle (append + render)
/// Target: <500μs
///
/// Measures the complete streaming cycle: append token and re-render.
/// This is what actually happens during response generation.
fn bench_streaming_cycle(c: &mut Criterion) {
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).expect("Failed to create terminal");

    c.bench_function("streaming_cycle", |b| {
        b.iter_batched(
            || {
                // Setup: Create state with some messages and a streaming response
                let mut state = create_state_with_messages(10);
                state.current_response = Some(String::with_capacity(4096));
                state
            },
            |mut state| {
                // Append a token and render
                if let Some(ref mut response) = state.current_response {
                    response.push_str(black_box("token "));
                }
                terminal
                    .draw(|frame| {
                        render(frame, &state);
                    })
                    .expect("Failed to draw");
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

/// Benchmark: Input character echo
/// Target: <10μs
///
/// Measures the performance of inserting a character and updating the cursor,
/// which happens on every keypress.
fn bench_input_character_echo(c: &mut Criterion) {
    c.bench_function("input_character_echo", |b| {
        b.iter_batched(
            || {
                // Setup: Create state with some existing input
                let mut state = AppState::new(PathBuf::from("/tmp"));
                state.input = "Hello, this is some existing input text".to_string();
                state
            },
            |mut state| {
                // Insert a character at the current position
                state.insert_char(black_box('x'));
                black_box(&state);
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

/// Benchmark: Input with cursor movement
/// Target: <10μs
///
/// Measures cursor movement performance.
fn bench_cursor_movement(c: &mut Criterion) {
    c.bench_function("cursor_movement", |b| {
        b.iter_batched(
            || {
                let mut state = AppState::new(PathBuf::from("/tmp"));
                state.input = "Hello, this is some text for cursor movement testing".to_string();
                // Move cursor to middle
                for _ in 0..25 {
                    state.cursor_right();
                }
                state
            },
            |mut state| {
                state.cursor_left();
                state.cursor_right();
                state.cursor_home();
                state.cursor_end();
                black_box(&state);
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

/// Benchmark: Scroll operations
/// Target: <1μs
fn bench_scroll_operations(c: &mut Criterion) {
    c.bench_function("scroll_operations", |b| {
        b.iter_batched(
            || {
                let mut state = create_state_with_messages(50);
                state.scroll_up(100);
                state
            },
            |mut state| {
                state.scroll_down(black_box(10));
                state.scroll_up(black_box(5));
                black_box(&state);
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

/// Benchmark: Large message rendering
/// Target: <5ms
///
/// Tests rendering performance with messages containing very long content.
fn bench_large_message_rendering(c: &mut Criterion) {
    let mut state = AppState::new(PathBuf::from("/tmp"));

    // Create messages with very long content (simulating code blocks, etc.)
    for i in 0..20 {
        let role = if i % 2 == 0 {
            Role::User
        } else {
            Role::Assistant
        };

        // Each message has ~5000 characters
        let content = (0..100)
            .map(|j| {
                format!(
                    "Line {}: This is a long line of text that simulates code or documentation.\n",
                    j
                )
            })
            .collect::<String>();

        state.add_message(Message { role, content });
    }

    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).expect("Failed to create terminal");

    c.bench_function("large_message_rendering", |b| {
        b.iter(|| {
            terminal
                .draw(|frame| {
                    render(frame, black_box(&state));
                })
                .expect("Failed to draw");
        });
    });
}

criterion_group!(
    benches,
    bench_full_redraw_100_messages,
    bench_streaming_token_append,
    bench_streaming_cycle,
    bench_input_character_echo,
    bench_cursor_movement,
    bench_scroll_operations,
    bench_large_message_rendering,
);

criterion_main!(benches);
