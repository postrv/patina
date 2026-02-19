//! Continuous coding progress widget for displaying loop status in the TUI.
//!
//! This widget shows the current state of a continuous coding session, including:
//! - Current iteration and total completed
//! - Quality gate results (pass/fail)
//! - Stagnation warnings
//! - Human checkpoint notifications
//!
//! # Example
//!
//! ```rust,ignore
//! use patina::tui::widgets::continuous_progress::ContinuousProgressWidget;
//! use patina::app::state::{ContinuousLoopStatus, GateResult};
//!
//! let widget = ContinuousProgressWidget::new(
//!     &ContinuousLoopStatus::Running { iteration: 3 },
//!     5,
//!     &[GateResult { gate: "tests".into(), passed: true, message: None }],
//! );
//! // frame.render_widget(widget, area);
//! ```

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

use crate::app::state::{ContinuousLoopStatus, GateResult};
use crate::tui::theme::PatinaTheme;

/// Widget for displaying continuous coding session progress.
pub struct ContinuousProgressWidget<'a> {
    /// Current loop status.
    status: &'a ContinuousLoopStatus,
    /// Total completed iterations.
    iterations_completed: u32,
    /// Quality gate results from the current iteration.
    gate_results: &'a [GateResult],
    /// Gate currently being checked.
    checking_gate: Option<&'a str>,
    /// Last iteration duration in milliseconds.
    last_duration_ms: Option<u64>,
}

impl<'a> ContinuousProgressWidget<'a> {
    /// Creates a new continuous progress widget.
    ///
    /// # Arguments
    ///
    /// * `status` - Current loop status
    /// * `iterations_completed` - Total completed iterations
    /// * `gate_results` - Quality gate results from the current iteration
    #[must_use]
    pub fn new(
        status: &'a ContinuousLoopStatus,
        iterations_completed: u32,
        gate_results: &'a [GateResult],
    ) -> Self {
        Self {
            status,
            iterations_completed,
            gate_results,
            checking_gate: None,
            last_duration_ms: None,
        }
    }

    /// Sets the gate currently being checked.
    #[must_use]
    pub fn with_checking_gate(mut self, gate: Option<&'a str>) -> Self {
        self.checking_gate = gate;
        self
    }

    /// Sets the last iteration duration.
    #[must_use]
    pub fn with_last_duration(mut self, duration_ms: Option<u64>) -> Self {
        self.last_duration_ms = duration_ms;
        self
    }

    /// Renders the status line showing the current loop state.
    #[must_use]
    pub fn render_status_line(&self) -> Line<'static> {
        match self.status {
            ContinuousLoopStatus::Inactive => Line::from(vec![
                Span::styled(" ○ ".to_string(), Style::default().fg(PatinaTheme::MUTED)),
                Span::styled(
                    "Inactive".to_string(),
                    Style::default().fg(PatinaTheme::MUTED),
                ),
            ]),
            ContinuousLoopStatus::Running { iteration } => Line::from(vec![
                Span::styled(" ● ".to_string(), Style::default().fg(PatinaTheme::SUCCESS)),
                Span::styled(
                    format!("Running iteration {iteration}"),
                    Style::default().fg(PatinaTheme::SUCCESS),
                ),
            ]),
            ContinuousLoopStatus::Stagnated {
                iterations_without_progress,
                threshold,
            } => Line::from(vec![
                Span::styled(" ⚠ ".to_string(), Style::default().fg(PatinaTheme::WARNING)),
                Span::styled(
                    format!(
                        "Stagnated ({iterations_without_progress}/{threshold} idle iterations)"
                    ),
                    Style::default().fg(PatinaTheme::WARNING),
                ),
            ]),
            ContinuousLoopStatus::HumanRequired { reason } => Line::from(vec![
                Span::styled(" ✋ ".to_string(), Style::default().fg(PatinaTheme::ERROR)),
                Span::styled(
                    format!("Human required: {reason}"),
                    Style::default().fg(PatinaTheme::ERROR),
                ),
            ]),
        }
    }

    /// Renders the iteration summary line.
    #[must_use]
    pub fn render_iteration_line(&self) -> Line<'static> {
        let label_style = Style::default().fg(PatinaTheme::MUTED);
        let value_style = Style::default().fg(PatinaTheme::VERDIGRIS_BRIGHT);

        let mut spans = vec![
            Span::styled(" Completed: ".to_string(), label_style),
            Span::styled(self.iterations_completed.to_string(), value_style),
        ];

        if let Some(duration_ms) = self.last_duration_ms {
            let secs = duration_ms / 1000;
            spans.push(Span::styled(" | Last: ".to_string(), label_style));
            spans.push(Span::styled(format!("{secs}s"), value_style));
        }

        Line::from(spans)
    }

    /// Renders quality gate results as a line of pass/fail indicators.
    #[must_use]
    pub fn render_gate_results(&self) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        for result in self.gate_results {
            let (icon, style) = if result.passed {
                ("✓", Style::default().fg(PatinaTheme::SUCCESS))
            } else {
                ("✗", Style::default().fg(PatinaTheme::ERROR))
            };

            let mut spans = vec![
                Span::styled(format!(" {icon} "), style),
                Span::styled(result.gate.clone(), style),
            ];

            if let Some(ref msg) = result.message {
                spans.push(Span::styled(
                    format!(" ({msg})"),
                    Style::default().fg(PatinaTheme::MUTED),
                ));
            }

            lines.push(Line::from(spans));
        }

        if let Some(gate) = self.checking_gate {
            lines.push(Line::from(vec![
                Span::styled(" ◐ ".to_string(), Style::default().fg(PatinaTheme::WARNING)),
                Span::styled(
                    format!("{gate}..."),
                    Style::default().fg(PatinaTheme::WARNING),
                ),
            ]));
        }

        lines
    }
}

impl Widget for ContinuousProgressWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 3 || area.height < 1 {
            return;
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(PatinaTheme::BORDER))
            .title(Span::styled(
                " Continuous Loop ",
                Style::default().fg(PatinaTheme::BRONZE),
            ))
            .style(Style::default().bg(PatinaTheme::BG_SECONDARY));

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 1 {
            return;
        }

        let gate_lines = self.render_gate_results();
        let gate_count = gate_lines.len();

        // Status + iteration + gate results
        let row_count = 2 + gate_count;

        let constraints: Vec<Constraint> = (0..row_count)
            .map(|_| Constraint::Length(1))
            .chain(std::iter::once(Constraint::Min(0)))
            .collect();

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        // Status line
        if !layout.is_empty() {
            let status_line = self.render_status_line();
            Paragraph::new(status_line).render(layout[0], buf);
        }

        // Iteration line
        if layout.len() > 1 {
            let iter_line = self.render_iteration_line();
            Paragraph::new(iter_line).render(layout[1], buf);
        }

        // Gate results
        for (i, gate_line) in gate_lines.into_iter().enumerate() {
            let idx = 2 + i;
            if idx < layout.len() {
                Paragraph::new(gate_line).render(layout[idx], buf);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    fn test_terminal(width: u16, height: u16) -> Terminal<TestBackend> {
        let backend = TestBackend::new(width, height);
        Terminal::new(backend).expect("Failed to create test terminal")
    }

    // =========================================================================
    // Status line rendering
    // =========================================================================

    #[test]
    fn status_line_inactive_shows_idle_indicator() {
        let status = ContinuousLoopStatus::Inactive;
        let widget = ContinuousProgressWidget::new(&status, 0, &[]);
        let line = widget.render_status_line();
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("Inactive"), "Should show Inactive status");
        assert!(text.contains('○'), "Should show idle circle indicator");
    }

    #[test]
    fn status_line_running_shows_iteration() {
        let status = ContinuousLoopStatus::Running { iteration: 5 };
        let widget = ContinuousProgressWidget::new(&status, 3, &[]);
        let line = widget.render_status_line();
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            text.contains("Running iteration 5"),
            "Should show current iteration"
        );
        assert!(text.contains('●'), "Should show running indicator");
    }

    #[test]
    fn status_line_stagnated_shows_warning() {
        let status = ContinuousLoopStatus::Stagnated {
            iterations_without_progress: 5,
            threshold: 3,
        };
        let widget = ContinuousProgressWidget::new(&status, 10, &[]);
        let line = widget.render_status_line();
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("Stagnated"), "Should show Stagnated");
        assert!(text.contains("5/3"), "Should show iterations/threshold");
    }

    #[test]
    fn status_line_human_required_shows_reason() {
        let status = ContinuousLoopStatus::HumanRequired {
            reason: "Build failures".to_string(),
        };
        let widget = ContinuousProgressWidget::new(&status, 5, &[]);
        let line = widget.render_status_line();
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            text.contains("Human required"),
            "Should show Human required"
        );
        assert!(text.contains("Build failures"), "Should show the reason");
    }

    // =========================================================================
    // Iteration line rendering
    // =========================================================================

    #[test]
    fn iteration_line_shows_completed_count() {
        let status = ContinuousLoopStatus::Running { iteration: 4 };
        let widget = ContinuousProgressWidget::new(&status, 3, &[]);
        let line = widget.render_iteration_line();
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("Completed: 3"), "Should show completed count");
    }

    #[test]
    fn iteration_line_shows_duration_when_set() {
        let status = ContinuousLoopStatus::Running { iteration: 2 };
        let widget =
            ContinuousProgressWidget::new(&status, 1, &[]).with_last_duration(Some(45_000));
        let line = widget.render_iteration_line();
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            text.contains("Last: 45s"),
            "Should show duration in seconds"
        );
    }

    #[test]
    fn iteration_line_no_duration_when_none() {
        let status = ContinuousLoopStatus::Running { iteration: 1 };
        let widget = ContinuousProgressWidget::new(&status, 0, &[]);
        let line = widget.render_iteration_line();
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(!text.contains("Last:"), "Should not show duration");
    }

    // =========================================================================
    // Gate results rendering
    // =========================================================================

    #[test]
    fn gate_results_empty_returns_empty() {
        let status = ContinuousLoopStatus::Inactive;
        let widget = ContinuousProgressWidget::new(&status, 0, &[]);
        let lines = widget.render_gate_results();
        assert!(lines.is_empty(), "No gates should produce no lines");
    }

    #[test]
    fn gate_results_passed_shows_checkmark() {
        let status = ContinuousLoopStatus::Running { iteration: 1 };
        let results = vec![GateResult {
            gate: "tests".to_string(),
            passed: true,
            message: None,
        }];
        let widget = ContinuousProgressWidget::new(&status, 0, &results);
        let lines = widget.render_gate_results();
        assert_eq!(lines.len(), 1);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains('✓'), "Passed gate should show checkmark");
        assert!(text.contains("tests"), "Should show gate name");
    }

    #[test]
    fn gate_results_failed_shows_x() {
        let status = ContinuousLoopStatus::Running { iteration: 1 };
        let results = vec![GateResult {
            gate: "clippy".to_string(),
            passed: false,
            message: Some("3 warnings".to_string()),
        }];
        let widget = ContinuousProgressWidget::new(&status, 0, &results);
        let lines = widget.render_gate_results();
        assert_eq!(lines.len(), 1);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains('✗'), "Failed gate should show X");
        assert!(text.contains("clippy"), "Should show gate name");
        assert!(text.contains("3 warnings"), "Should show failure message");
    }

    #[test]
    fn gate_results_multiple_gates() {
        let status = ContinuousLoopStatus::Running { iteration: 1 };
        let results = vec![
            GateResult {
                gate: "fmt".to_string(),
                passed: true,
                message: None,
            },
            GateResult {
                gate: "clippy".to_string(),
                passed: true,
                message: Some("0 warnings".to_string()),
            },
            GateResult {
                gate: "tests".to_string(),
                passed: false,
                message: Some("2 failed".to_string()),
            },
        ];
        let widget = ContinuousProgressWidget::new(&status, 0, &results);
        let lines = widget.render_gate_results();
        assert_eq!(lines.len(), 3, "Should have one line per gate");
    }

    #[test]
    fn gate_results_checking_gate_shows_spinner() {
        let status = ContinuousLoopStatus::Running { iteration: 1 };
        let widget =
            ContinuousProgressWidget::new(&status, 0, &[]).with_checking_gate(Some("tests"));
        let lines = widget.render_gate_results();
        assert_eq!(lines.len(), 1);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("tests..."), "Should show gate being checked");
        assert!(text.contains('◐'), "Should show spinner");
    }

    // =========================================================================
    // Widget rendering (full render)
    // =========================================================================

    #[test]
    fn widget_renders_in_terminal() {
        let mut terminal = test_terminal(60, 12);
        let status = ContinuousLoopStatus::Running { iteration: 3 };
        let results = vec![
            GateResult {
                gate: "fmt".to_string(),
                passed: true,
                message: None,
            },
            GateResult {
                gate: "tests".to_string(),
                passed: false,
                message: Some("1 failed".to_string()),
            },
        ];

        terminal
            .draw(|frame| {
                let widget = ContinuousProgressWidget::new(&status, 2, &results)
                    .with_last_duration(Some(12_000));
                frame.render_widget(widget, frame.area());
            })
            .expect("Failed to draw");

        let buffer = terminal.backend().buffer();
        let content: String = buffer
            .content()
            .iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();

        assert!(
            content.contains("Continuous Loop"),
            "Should have widget title"
        );
        assert!(
            content.contains("Running iteration 3"),
            "Should show running status"
        );
    }

    #[test]
    fn widget_handles_tiny_area() {
        let mut terminal = test_terminal(2, 1);
        let status = ContinuousLoopStatus::Inactive;

        // Should not panic
        terminal
            .draw(|frame| {
                let widget = ContinuousProgressWidget::new(&status, 0, &[]);
                frame.render_widget(widget, frame.area());
            })
            .expect("Failed to draw");
    }

    #[test]
    fn widget_shows_stagnation_warning() {
        let mut terminal = test_terminal(70, 10);
        let status = ContinuousLoopStatus::Stagnated {
            iterations_without_progress: 5,
            threshold: 3,
        };

        terminal
            .draw(|frame| {
                let widget = ContinuousProgressWidget::new(&status, 8, &[]);
                frame.render_widget(widget, frame.area());
            })
            .expect("Failed to draw");

        let buffer = terminal.backend().buffer();
        let content: String = buffer
            .content()
            .iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();

        assert!(content.contains("Stagnated"), "Should show stagnation");
    }

    // =========================================================================
    // Builder pattern
    // =========================================================================

    #[test]
    fn with_checking_gate_sets_gate() {
        let status = ContinuousLoopStatus::Inactive;
        let widget =
            ContinuousProgressWidget::new(&status, 0, &[]).with_checking_gate(Some("tests"));
        assert_eq!(widget.checking_gate, Some("tests"));
    }

    #[test]
    fn with_last_duration_sets_duration() {
        let status = ContinuousLoopStatus::Inactive;
        let widget = ContinuousProgressWidget::new(&status, 0, &[]).with_last_duration(Some(5000));
        assert_eq!(widget.last_duration_ms, Some(5000));
    }
}
