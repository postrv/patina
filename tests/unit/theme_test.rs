//! Unit tests for Patina theme module.
//!
//! These tests verify that theme colors are correctly defined and visually distinct.

use patina::tui::theme::PatinaTheme;
use ratatui::style::{Color, Modifier};

// ============================================================================
// Color Distinctiveness Tests
// ============================================================================

/// Tests that key colors are visually distinct from each other.
/// This prevents accidental use of similar colors for different roles.
#[test]
fn test_theme_colors_are_distinct() {
    // Background colors should be distinct
    assert_ne!(
        PatinaTheme::BG_PRIMARY,
        PatinaTheme::BG_SECONDARY,
        "Primary and secondary backgrounds should be distinct"
    );
    assert_ne!(
        PatinaTheme::BG_SECONDARY,
        PatinaTheme::BG_HIGHLIGHT,
        "Secondary and highlight backgrounds should be distinct"
    );
    assert_ne!(
        PatinaTheme::BG_PRIMARY,
        PatinaTheme::BG_CODE,
        "Primary background and code background should be distinct"
    );

    // Accent colors should be distinct
    assert_ne!(
        PatinaTheme::VERDIGRIS,
        PatinaTheme::BRONZE,
        "Verdigris and bronze should be distinct"
    );
    assert_ne!(
        PatinaTheme::VERDIGRIS_BRIGHT,
        PatinaTheme::COPPER_BRIGHT,
        "Bright verdigris and copper should be distinct"
    );

    // Role-based colors should be distinct
    assert_ne!(
        PatinaTheme::USER_TEXT,
        PatinaTheme::ASSISTANT_TEXT,
        "User and assistant text colors should be distinct"
    );
    assert_ne!(
        PatinaTheme::USER_LABEL,
        PatinaTheme::ASSISTANT_LABEL,
        "User and assistant label colors should be distinct"
    );
}

/// Tests that semantic colors are distinct.
#[test]
fn test_semantic_colors_are_distinct() {
    assert_ne!(
        PatinaTheme::SUCCESS,
        PatinaTheme::WARNING,
        "Success and warning should be distinct"
    );
    assert_ne!(
        PatinaTheme::WARNING,
        PatinaTheme::ERROR,
        "Warning and error should be distinct"
    );
    assert_ne!(
        PatinaTheme::SUCCESS,
        PatinaTheme::ERROR,
        "Success and error should be distinct"
    );
}

// ============================================================================
// Style Tests
// ============================================================================

/// Tests that user_message() style returns correct foreground color.
#[test]
fn test_theme_styles_return_correct_colors() {
    let user_style = PatinaTheme::user_message();
    assert_eq!(
        user_style.fg,
        Some(PatinaTheme::USER_TEXT),
        "User message style should use USER_TEXT color"
    );

    let assistant_style = PatinaTheme::assistant_message();
    assert_eq!(
        assistant_style.fg,
        Some(PatinaTheme::ASSISTANT_TEXT),
        "Assistant message style should use ASSISTANT_TEXT color"
    );

    let error_style = PatinaTheme::error();
    assert_eq!(
        error_style.fg,
        Some(PatinaTheme::ERROR),
        "Error style should use ERROR color"
    );

    let warning_style = PatinaTheme::warning();
    assert_eq!(
        warning_style.fg,
        Some(PatinaTheme::WARNING),
        "Warning style should use WARNING color"
    );

    let success_style = PatinaTheme::success();
    assert_eq!(
        success_style.fg,
        Some(PatinaTheme::SUCCESS),
        "Success style should use SUCCESS color"
    );
}

/// Tests that label styles include bold modifier.
#[test]
fn test_label_styles_are_bold() {
    let user_label = PatinaTheme::user_label();
    assert!(
        user_label.add_modifier.contains(Modifier::BOLD),
        "User label should be bold"
    );

    let assistant_label = PatinaTheme::assistant_label();
    assert!(
        assistant_label.add_modifier.contains(Modifier::BOLD),
        "Assistant label should be bold"
    );

    let tool_header = PatinaTheme::tool_header();
    assert!(
        tool_header.add_modifier.contains(Modifier::BOLD),
        "Tool header should be bold"
    );

    let title_style = PatinaTheme::title();
    assert!(
        title_style.add_modifier.contains(Modifier::BOLD),
        "Title should be bold"
    );

    let prompt_style = PatinaTheme::prompt();
    assert!(
        prompt_style.add_modifier.contains(Modifier::BOLD),
        "Prompt should be bold"
    );
}

/// Tests that code block style has both foreground and background.
#[test]
fn test_code_block_has_background() {
    let code_style = PatinaTheme::code_block();
    assert!(
        code_style.fg.is_some(),
        "Code block should have foreground color"
    );
    assert!(
        code_style.bg.is_some(),
        "Code block should have background color"
    );
    assert_eq!(
        code_style.bg,
        Some(PatinaTheme::BG_CODE),
        "Code block should use BG_CODE background"
    );
}

/// Tests that status bar style has both foreground and background.
#[test]
fn test_status_bar_has_background() {
    let status_style = PatinaTheme::status_bar();
    assert!(
        status_style.fg.is_some(),
        "Status bar should have foreground color"
    );
    assert!(
        status_style.bg.is_some(),
        "Status bar should have background color"
    );
    assert_eq!(
        status_style.bg,
        Some(PatinaTheme::STATUS_BG),
        "Status bar should use STATUS_BG background"
    );
}

// ============================================================================
// Color Value Tests
// ============================================================================

/// Tests that background colors are dark (low RGB values).
/// This ensures the theme remains readable with light text.
#[test]
fn test_background_colors_are_dark() {
    // Primary background: #0d1f22
    if let Color::Rgb(r, g, b) = PatinaTheme::BG_PRIMARY {
        assert!(
            r < 50 && g < 50 && b < 50,
            "BG_PRIMARY should be dark (r={}, g={}, b={})",
            r,
            g,
            b
        );
    } else {
        panic!("BG_PRIMARY should be an RGB color");
    }

    // Secondary background: #142d32
    if let Color::Rgb(r, g, b) = PatinaTheme::BG_SECONDARY {
        assert!(
            r < 60 && g < 60 && b < 60,
            "BG_SECONDARY should be dark (r={}, g={}, b={})",
            r,
            g,
            b
        );
    } else {
        panic!("BG_SECONDARY should be an RGB color");
    }
}

/// Tests that accent colors have sufficient brightness for visibility.
#[test]
fn test_accent_colors_are_bright_enough() {
    // Verdigris bright: #7ec8b8 (126, 200, 184)
    if let Color::Rgb(r, g, b) = PatinaTheme::VERDIGRIS_BRIGHT {
        assert!(
            r > 100 || g > 100 || b > 100,
            "VERDIGRIS_BRIGHT should have at least one channel > 100"
        );
    } else {
        panic!("VERDIGRIS_BRIGHT should be an RGB color");
    }

    // Copper bright: #d4a574 (212, 165, 116)
    if let Color::Rgb(r, g, b) = PatinaTheme::COPPER_BRIGHT {
        assert!(
            r > 100 || g > 100 || b > 100,
            "COPPER_BRIGHT should have at least one channel > 100"
        );
    } else {
        panic!("COPPER_BRIGHT should be an RGB color");
    }
}

/// Tests that the specific hex color values match the specification.
#[test]
fn test_specified_hex_colors() {
    // Background #0d1f22
    assert_eq!(
        PatinaTheme::BG_PRIMARY,
        Color::Rgb(13, 31, 34),
        "BG_PRIMARY should be #0d1f22"
    );

    // User #d4a574
    assert_eq!(
        PatinaTheme::USER_TEXT,
        Color::Rgb(212, 165, 116),
        "USER_TEXT should be #d4a574"
    );

    // Assistant #7ec8b8
    assert_eq!(
        PatinaTheme::ASSISTANT_TEXT,
        Color::Rgb(126, 200, 184),
        "ASSISTANT_TEXT should be #7ec8b8"
    );

    // Tool #c9956c
    assert_eq!(
        PatinaTheme::TOOL_HEADER,
        Color::Rgb(201, 149, 108),
        "TOOL_HEADER should be #c9956c"
    );
}

// ============================================================================
// Border Style Tests
// ============================================================================

/// Tests that border styles are correctly configured.
#[test]
fn test_border_styles() {
    let normal_border = PatinaTheme::border();
    assert_eq!(
        normal_border.fg,
        Some(PatinaTheme::BORDER),
        "Normal border should use BORDER color"
    );

    let focused_border = PatinaTheme::border_focused();
    assert_eq!(
        focused_border.fg,
        Some(PatinaTheme::BORDER_FOCUSED),
        "Focused border should use BORDER_FOCUSED color"
    );

    // Focused border should be more prominent than normal
    assert_ne!(
        PatinaTheme::BORDER,
        PatinaTheme::BORDER_FOCUSED,
        "Border and focused border should be different"
    );
}

/// Tests streaming indicator style has blink modifier.
#[test]
fn test_streaming_indicator_blinks() {
    let streaming_style = PatinaTheme::streaming();
    assert!(
        streaming_style.add_modifier.contains(Modifier::SLOW_BLINK),
        "Streaming indicator should have slow blink modifier"
    );
}
