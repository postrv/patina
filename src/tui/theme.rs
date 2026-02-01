//! Patina color theme — bronze and verdigris aesthetic
//!
//! Color palette derived from the Patina logo:
//! - Deep teal background
//! - Verdigris (teal/cyan) for assistant content
//! - Bronze/copper for user content and accents
//!
//! # Usage
//!
//! ```rust,ignore
//! use patina::tui::theme::PatinaTheme;
//! use ratatui::style::Style;
//!
//! let style = Style::default()
//!     .fg(PatinaTheme::VERDIGRIS)
//!     .bg(PatinaTheme::BG_PRIMARY);
//! ```

use ratatui::style::{Color, Modifier, Style};

/// Patina color theme constants and pre-built styles.
///
/// This struct provides a cohesive color palette for the Patina terminal UI,
/// inspired by oxidized copper (patina/verdigris) aesthetic.
///
/// # Color Categories
///
/// - **Background colors**: Dark teals for various UI surfaces
/// - **Verdigris (teal/cyan)**: Primary accent for assistant content
/// - **Bronze/copper**: Secondary accent for user content
/// - **Semantic colors**: Success, warning, error states
/// - **Role-based colors**: User vs assistant message styling
/// - **UI element colors**: Borders, prompts, scrollbars
pub struct PatinaTheme;

impl PatinaTheme {
    // =========================================================================
    // Background Colors
    // =========================================================================

    /// Deep dark teal — main background.
    /// Hex: `#0d1f22`
    pub const BG_PRIMARY: Color = Color::Rgb(13, 31, 34);

    /// Slightly lighter teal — panels, code blocks.
    /// Hex: `#142d32`
    pub const BG_SECONDARY: Color = Color::Rgb(20, 45, 50);

    /// Highlight/selection background.
    /// Hex: `#1e3c41`
    pub const BG_HIGHLIGHT: Color = Color::Rgb(30, 60, 65);

    /// Code block background — slightly warmer.
    /// Hex: `#193237`
    pub const BG_CODE: Color = Color::Rgb(25, 50, 55);

    // =========================================================================
    // Verdigris (Teal/Cyan) — Primary accent for assistant
    // =========================================================================

    /// Bright verdigris — emphasis, headings.
    /// Hex: `#7ec8b8`
    pub const VERDIGRIS_BRIGHT: Color = Color::Rgb(126, 200, 184);

    /// Standard verdigris — body text.
    /// Hex: `#5fb3a1`
    pub const VERDIGRIS: Color = Color::Rgb(95, 179, 161);

    /// Muted verdigris — secondary text, timestamps.
    /// Hex: `#468c7d`
    pub const VERDIGRIS_MUTED: Color = Color::Rgb(70, 140, 125);

    /// Dark verdigris — borders, separators.
    /// Hex: `#2d645a`
    pub const VERDIGRIS_DARK: Color = Color::Rgb(45, 100, 90);

    // =========================================================================
    // Bronze/Copper — Secondary accent for user
    // =========================================================================

    /// Bright copper — highlights, active elements.
    /// Hex: `#d4a574`
    pub const COPPER_BRIGHT: Color = Color::Rgb(212, 165, 116);

    /// Standard bronze — headings, labels.
    /// Hex: `#c9956c`
    pub const BRONZE: Color = Color::Rgb(201, 149, 108);

    /// Muted bronze — timestamps, secondary.
    /// Hex: `#a07855`
    pub const BRONZE_MUTED: Color = Color::Rgb(160, 120, 85);

    /// Dark bronze — subtle accents.
    /// Hex: `#785a41`
    pub const BRONZE_DARK: Color = Color::Rgb(120, 90, 65);

    // =========================================================================
    // Semantic Colors
    // =========================================================================

    /// Success — bright verdigris.
    pub const SUCCESS: Color = Self::VERDIGRIS_BRIGHT;

    /// Warning — warm amber.
    /// Hex: `#e6b464`
    pub const WARNING: Color = Color::Rgb(230, 180, 100);

    /// Error — muted red (not harsh).
    /// Hex: `#c86464`
    pub const ERROR: Color = Color::Rgb(200, 100, 100);

    /// Muted/disabled text.
    /// Hex: `#647873`
    pub const MUTED: Color = Color::Rgb(100, 120, 115);

    // =========================================================================
    // Diff Colors (subtle backgrounds, not foregrounds)
    // =========================================================================

    /// Diff addition background — subtle dark green.
    /// Hex: `#122319`
    pub const DIFF_ADDITION_BG: Color = Color::Rgb(18, 35, 25);

    /// Diff deletion background — subtle dark red.
    /// Hex: `#281416`
    pub const DIFF_DELETION_BG: Color = Color::Rgb(40, 20, 22);

    /// Diff hunk header background — subtle bronze tint.
    /// Hex: `#1e1814`
    pub const DIFF_HUNK_BG: Color = Color::Rgb(30, 24, 20);

    // Legacy foreground colors (kept for compatibility)
    /// Diff addition — muted green (distinct from verdigris).
    /// Hex: `#5fb87a`
    pub const DIFF_ADDITION: Color = Color::Rgb(95, 184, 122);

    /// Diff deletion — muted red (same as ERROR for consistency).
    pub const DIFF_DELETION: Color = Self::ERROR;

    /// Diff hunk header — muted bronze.
    pub const DIFF_HUNK: Color = Self::BRONZE_MUTED;

    // =========================================================================
    // Role-based Colors
    // =========================================================================

    /// User message text color.
    pub const USER_TEXT: Color = Self::COPPER_BRIGHT;

    /// User label color.
    pub const USER_LABEL: Color = Self::BRONZE;

    /// Assistant message text color.
    pub const ASSISTANT_TEXT: Color = Self::VERDIGRIS_BRIGHT;

    /// Assistant label color.
    pub const ASSISTANT_LABEL: Color = Self::VERDIGRIS;

    /// System message text color.
    pub const SYSTEM_TEXT: Color = Self::MUTED;

    // =========================================================================
    // UI Element Colors
    // =========================================================================

    /// Border — normal state.
    pub const BORDER: Color = Self::VERDIGRIS_DARK;

    /// Border — focused/active state.
    pub const BORDER_FOCUSED: Color = Self::VERDIGRIS;

    /// Input prompt character (›).
    pub const PROMPT: Color = Self::BRONZE;

    /// Cursor color.
    pub const CURSOR: Color = Self::COPPER_BRIGHT;

    /// Scrollbar track.
    pub const SCROLLBAR_TRACK: Color = Self::BG_SECONDARY;

    /// Scrollbar thumb.
    pub const SCROLLBAR_THUMB: Color = Self::VERDIGRIS_MUTED;

    /// Tool execution header.
    pub const TOOL_HEADER: Color = Self::BRONZE;

    /// Tool execution content.
    pub const TOOL_CONTENT: Color = Self::VERDIGRIS;

    /// Status bar background.
    pub const STATUS_BG: Color = Self::BG_SECONDARY;

    /// Status bar text.
    pub const STATUS_TEXT: Color = Self::VERDIGRIS_MUTED;

    // =========================================================================
    // Pre-built Styles
    // =========================================================================

    /// Style for user messages.
    #[must_use]
    pub fn user_message() -> Style {
        Style::default().fg(Self::USER_TEXT)
    }

    /// Style for user label ("You").
    #[must_use]
    pub fn user_label() -> Style {
        Style::default()
            .fg(Self::USER_LABEL)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for assistant messages.
    #[must_use]
    pub fn assistant_message() -> Style {
        Style::default().fg(Self::ASSISTANT_TEXT)
    }

    /// Style for assistant label ("Patina").
    #[must_use]
    pub fn assistant_label() -> Style {
        Style::default()
            .fg(Self::ASSISTANT_LABEL)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for code blocks.
    #[must_use]
    pub fn code_block() -> Style {
        Style::default().fg(Self::VERDIGRIS).bg(Self::BG_CODE)
    }

    /// Style for inline code.
    #[must_use]
    pub fn code_inline() -> Style {
        Style::default()
            .fg(Self::VERDIGRIS_BRIGHT)
            .bg(Self::BG_CODE)
    }

    /// Style for tool execution headers.
    #[must_use]
    pub fn tool_header() -> Style {
        Style::default()
            .fg(Self::TOOL_HEADER)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for borders (normal).
    #[must_use]
    pub fn border() -> Style {
        Style::default().fg(Self::BORDER)
    }

    /// Style for borders (focused).
    #[must_use]
    pub fn border_focused() -> Style {
        Style::default().fg(Self::BORDER_FOCUSED)
    }

    /// Style for the title ("Patina").
    #[must_use]
    pub fn title() -> Style {
        Style::default()
            .fg(Self::BRONZE)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for timestamps.
    #[must_use]
    pub fn timestamp() -> Style {
        Style::default().fg(Self::MUTED)
    }

    /// Style for success messages.
    #[must_use]
    pub fn success() -> Style {
        Style::default().fg(Self::SUCCESS)
    }

    /// Style for warning messages.
    #[must_use]
    pub fn warning() -> Style {
        Style::default().fg(Self::WARNING)
    }

    /// Style for error messages.
    #[must_use]
    pub fn error() -> Style {
        Style::default().fg(Self::ERROR)
    }

    /// Style for the input prompt.
    #[must_use]
    pub fn prompt() -> Style {
        Style::default()
            .fg(Self::PROMPT)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for status bar.
    #[must_use]
    pub fn status_bar() -> Style {
        Style::default().fg(Self::STATUS_TEXT).bg(Self::STATUS_BG)
    }

    /// Style for streaming indicator.
    #[must_use]
    pub fn streaming() -> Style {
        Style::default()
            .fg(Self::VERDIGRIS_BRIGHT)
            .add_modifier(Modifier::SLOW_BLINK)
    }

    /// Style for diff additions (lines starting with +).
    /// Uses subtle green background with default text color.
    #[must_use]
    pub fn diff_addition() -> Style {
        Style::default()
            .fg(Self::USER_TEXT)
            .bg(Self::DIFF_ADDITION_BG)
    }

    /// Style for diff deletions (lines starting with -).
    /// Uses subtle red background with default text color.
    #[must_use]
    pub fn diff_deletion() -> Style {
        Style::default()
            .fg(Self::USER_TEXT)
            .bg(Self::DIFF_DELETION_BG)
    }

    /// Style for diff hunk headers (lines starting with @@).
    /// Uses subtle bronze background with muted text.
    #[must_use]
    pub fn diff_hunk() -> Style {
        Style::default()
            .fg(Self::BRONZE_MUTED)
            .bg(Self::DIFF_HUNK_BG)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_are_distinct() {
        // Ensure key colors are visually distinct
        assert_ne!(PatinaTheme::BG_PRIMARY, PatinaTheme::BG_SECONDARY);
        assert_ne!(PatinaTheme::VERDIGRIS, PatinaTheme::BRONZE);
        assert_ne!(PatinaTheme::USER_TEXT, PatinaTheme::ASSISTANT_TEXT);
    }

    #[test]
    fn styles_have_correct_foreground() {
        let user_style = PatinaTheme::user_message();
        assert_eq!(user_style.fg, Some(PatinaTheme::USER_TEXT));

        let assistant_style = PatinaTheme::assistant_message();
        assert_eq!(assistant_style.fg, Some(PatinaTheme::ASSISTANT_TEXT));
    }

    #[test]
    fn background_colors_are_dark() {
        // All backgrounds should have low RGB values (dark theme)
        if let Color::Rgb(r, g, b) = PatinaTheme::BG_PRIMARY {
            assert!(r < 50 && g < 50 && b < 50);
        }
    }

    #[test]
    fn diff_backgrounds_are_distinct() {
        assert_ne!(PatinaTheme::DIFF_ADDITION_BG, PatinaTheme::DIFF_DELETION_BG);
        assert_ne!(PatinaTheme::DIFF_ADDITION_BG, PatinaTheme::BG_PRIMARY);
        assert_ne!(PatinaTheme::DIFF_DELETION_BG, PatinaTheme::BG_PRIMARY);
    }

    #[test]
    fn diff_styles_use_background_highlighting() {
        // Diff styles should use background colors for subtle highlighting
        assert_eq!(
            PatinaTheme::diff_addition().bg,
            Some(PatinaTheme::DIFF_ADDITION_BG)
        );
        assert_eq!(
            PatinaTheme::diff_deletion().bg,
            Some(PatinaTheme::DIFF_DELETION_BG)
        );
        assert_eq!(PatinaTheme::diff_hunk().bg, Some(PatinaTheme::DIFF_HUNK_BG));
        // Text should remain readable (user text color for additions/deletions)
        assert_eq!(
            PatinaTheme::diff_addition().fg,
            Some(PatinaTheme::USER_TEXT)
        );
        assert_eq!(
            PatinaTheme::diff_deletion().fg,
            Some(PatinaTheme::USER_TEXT)
        );
    }
}
