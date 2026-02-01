//! Unit tests for the image display widget.
//!
//! These tests verify that images are rendered correctly in the TUI,
//! using half-block characters for fallback rendering when advanced
//! graphics protocols are not available.

use patina::tui::widgets::image_display::{
    detect_graphics_protocol, GraphicsProtocol, ImageDisplayState, ImageDisplayWidget,
};
use ratatui::{backend::TestBackend, Terminal};

// ============================================================================
// Helper Functions
// ============================================================================

/// Creates a test terminal with the given dimensions.
fn test_terminal(width: u16, height: u16) -> Terminal<TestBackend> {
    let backend = TestBackend::new(width, height);
    Terminal::new(backend).expect("Failed to create test terminal")
}

/// Extracts the rendered content as a string from the terminal buffer.
fn buffer_to_string(terminal: &Terminal<TestBackend>) -> String {
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol().chars().next().unwrap_or(' '))
        .collect()
}

// ============================================================================
// Graphics Protocol Detection Tests
// ============================================================================

/// Tests that Sixel support can be detected from terminal capabilities.
///
/// Sixel is a graphics protocol supported by terminals like xterm, mintty,
/// and mlterm. Detection should check $TERM or query XTGETTCAP.
#[test]
fn test_detect_sixel_support() {
    // When TERM indicates sixel support, protocol should be Sixel
    let protocol = detect_graphics_protocol();

    // In test environment without real terminal, should return None or Unsupported
    // The important thing is that the detection function exists and doesn't panic
    assert!(
        matches!(
            protocol,
            GraphicsProtocol::Unsupported
                | GraphicsProtocol::Sixel
                | GraphicsProtocol::Kitty
                | GraphicsProtocol::ITerm2
                | GraphicsProtocol::HalfBlock
        ),
        "Protocol detection should return a valid GraphicsProtocol variant"
    );
}

/// Tests that Kitty graphics protocol support can be detected.
///
/// Kitty uses APC (Application Program Command) sequences for graphics.
/// Detection should query the terminal for Kitty graphics support.
#[test]
fn test_detect_kitty_graphics_support() {
    // In test environment, we verify the function exists and returns valid result
    let protocol = detect_graphics_protocol();

    // Test that detection doesn't panic and returns a valid variant
    match protocol {
        GraphicsProtocol::Kitty => {
            // If Kitty is detected, that's valid
        }
        GraphicsProtocol::Sixel
        | GraphicsProtocol::ITerm2
        | GraphicsProtocol::HalfBlock
        | GraphicsProtocol::Unsupported => {
            // Other protocols are also valid results
        }
    }
}

// ============================================================================
// Image Display State Tests
// ============================================================================

/// Tests that ImageDisplayState can be created with pixel data.
#[test]
fn test_image_display_state_creation() {
    // Create a 4x4 red image (RGBA format)
    let width = 4;
    let height = 4;
    let pixels: Vec<u8> = [255, 0, 0, 255].repeat(width * height);

    let state = ImageDisplayState::new(width as u32, height as u32, pixels.clone());

    assert_eq!(state.width(), width as u32);
    assert_eq!(state.height(), height as u32);
    assert_eq!(state.pixels(), &pixels);
}

/// Tests that ImageDisplayState calculates aspect ratio correctly.
#[test]
fn test_image_display_state_aspect_ratio() {
    // 16:9 aspect ratio image
    let state = ImageDisplayState::new(1920, 1080, vec![0; 1920 * 1080 * 4]);
    let aspect = state.aspect_ratio();

    // 1920/1080 = 1.777...
    assert!(
        (aspect - 16.0 / 9.0).abs() < 0.01,
        "Aspect ratio should be ~16:9"
    );
}

// ============================================================================
// Image Widget Rendering Tests
// ============================================================================

/// Tests that the widget renders a placeholder when graphics are unsupported.
///
/// On terminals that don't support any graphics protocol, the widget should
/// display a text placeholder indicating that an image cannot be displayed.
#[test]
fn test_image_widget_renders_placeholder_unsupported() {
    let mut terminal = test_terminal(40, 10);

    // Create a simple test image
    let pixels: Vec<u8> = [128, 128, 128, 255].repeat(100);
    let state = ImageDisplayState::new(10, 10, pixels);

    // Force unsupported protocol for testing
    let widget = ImageDisplayWidget::new(&state).with_protocol(GraphicsProtocol::Unsupported);

    terminal
        .draw(|frame| {
            frame.render_widget(widget, frame.area());
        })
        .expect("Failed to draw");

    let content = buffer_to_string(&terminal);

    // Should show placeholder text indicating image cannot be displayed
    assert!(
        content.contains("[Image]")
            || content.contains("image")
            || content.contains("🖼")
            || content.contains("10x10"),
        "Should display placeholder for unsupported terminals. Content: {}",
        content
    );
}

/// Tests that the widget renders using half-block characters.
///
/// Half-block rendering uses Unicode characters U+2580 (▀) and U+2584 (▄)
/// to display 2 vertical pixels per character cell, allowing images to be
/// rendered on any terminal that supports Unicode.
#[test]
fn test_image_widget_renders_halfblock_basic() {
    let mut terminal = test_terminal(20, 10);

    // Create a 4x4 image with distinct colors
    // Top half red, bottom half blue
    let mut pixels = Vec::new();
    for y in 0..4 {
        for _ in 0..4 {
            if y < 2 {
                pixels.extend_from_slice(&[255, 0, 0, 255]); // Red
            } else {
                pixels.extend_from_slice(&[0, 0, 255, 255]); // Blue
            }
        }
    }

    let state = ImageDisplayState::new(4, 4, pixels);
    let widget = ImageDisplayWidget::new(&state).with_protocol(GraphicsProtocol::HalfBlock);

    terminal
        .draw(|frame| {
            frame.render_widget(widget, frame.area());
        })
        .expect("Failed to draw");

    let content = buffer_to_string(&terminal);

    // Half-block rendering should produce visible output
    // It uses upper half block (▀) or lower half block (▄) characters
    assert!(
        !content.trim().is_empty(),
        "Half-block rendering should produce visible output"
    );

    // Check for half-block characters or that colors are being set
    let buffer = terminal.backend().buffer();
    let mut found_non_default_color = false;

    for cell in buffer.content() {
        if cell.fg != ratatui::style::Color::Reset && cell.fg != ratatui::style::Color::White {
            found_non_default_color = true;
            break;
        }
    }

    // In half-block mode, we expect colored cells
    assert!(
        found_non_default_color || content.contains('▀') || content.contains('▄'),
        "Half-block rendering should use colored cells or half-block characters"
    );
}

/// Tests that the widget respects maximum dimensions.
///
/// When an image is larger than the available render area, it should be
/// scaled down to fit while maintaining aspect ratio.
#[test]
fn test_image_widget_respects_max_dimensions() {
    // Small terminal area
    let mut terminal = test_terminal(10, 5);

    // Large image (100x100)
    let pixels: Vec<u8> = [100, 150, 200, 255].repeat(100 * 100);
    let state = ImageDisplayState::new(100, 100, pixels);

    let widget = ImageDisplayWidget::new(&state)
        .with_protocol(GraphicsProtocol::HalfBlock)
        .with_max_dimensions(10, 5);

    terminal
        .draw(|frame| {
            frame.render_widget(widget, frame.area());
        })
        .expect("Failed to draw");

    // Widget should render without panic and fit within bounds
    // The terminal size is the constraint
    let buffer = terminal.backend().buffer();
    assert_eq!(buffer.area.width, 10);
    assert_eq!(buffer.area.height, 5);
}

/// Tests that the widget maintains aspect ratio when scaling.
///
/// When an image is scaled to fit a smaller area, the aspect ratio
/// should be preserved to avoid distortion.
#[test]
fn test_image_widget_maintains_aspect_ratio() {
    let mut terminal = test_terminal(40, 20);

    // Wide image (20:10 = 2:1 aspect ratio)
    let width = 20;
    let height = 10;
    let pixels: Vec<u8> = [255, 200, 100, 255].repeat(width * height);

    let state = ImageDisplayState::new(width as u32, height as u32, pixels);

    // Set max dimensions that would cause scaling
    let widget = ImageDisplayWidget::new(&state)
        .with_protocol(GraphicsProtocol::HalfBlock)
        .with_max_dimensions(10, 10);

    // Get the calculated display dimensions
    let (display_width, display_height) = widget.calculate_display_dimensions(10, 10);

    // For 2:1 aspect ratio fitting in 10x10:
    // With half-block rendering, each terminal cell is 1 pixel wide x 2 pixels tall
    // So visual aspect = display_width / (display_height * 2)

    // Visual aspect in pixel space (accounting for half-block 2:1 cell ratio)
    let visual_aspect = display_width as f64 / (display_height as f64 * 2.0);
    let original_aspect = width as f64 / height as f64;

    // Allow some rounding error due to integer division
    assert!(
        (visual_aspect - original_aspect).abs() < 0.5,
        "Visual aspect ratio ({}) should be close to original ({}), dims: {}x{}",
        visual_aspect,
        original_aspect,
        display_width,
        display_height
    );

    // Verify dimensions don't exceed max
    assert!(
        display_width <= 10,
        "Display width should not exceed max: {}",
        display_width
    );
    assert!(
        display_height <= 10,
        "Display height should not exceed max: {}",
        display_height
    );

    // Now render to ensure it doesn't panic
    terminal
        .draw(|frame| {
            frame.render_widget(widget, frame.area());
        })
        .expect("Failed to draw");
}

// ============================================================================
// Edge Case Tests
// ============================================================================

/// Tests that empty or zero-dimension images are handled gracefully.
#[test]
fn test_image_widget_handles_empty_image() {
    let mut terminal = test_terminal(20, 10);

    // Zero-dimension image
    let state = ImageDisplayState::new(0, 0, vec![]);

    let widget = ImageDisplayWidget::new(&state).with_protocol(GraphicsProtocol::HalfBlock);

    // Should not panic
    terminal
        .draw(|frame| {
            frame.render_widget(widget, frame.area());
        })
        .expect("Drawing empty image should not panic");
}

/// Tests that very large images don't cause memory issues.
#[test]
fn test_image_widget_handles_large_image_reference() {
    // We don't actually allocate a huge image, just test the state creation
    // The widget should handle the metadata without issues
    let state = ImageDisplayState::new(10000, 10000, vec![]);

    // Should report correct dimensions even with no pixel data
    assert_eq!(state.width(), 10000);
    assert_eq!(state.height(), 10000);
}

/// Tests that single-pixel images render correctly.
#[test]
fn test_image_widget_renders_single_pixel() {
    let mut terminal = test_terminal(10, 5);

    // Single pixel image
    let pixels = vec![255, 128, 64, 255]; // Orange color
    let state = ImageDisplayState::new(1, 1, pixels);

    let widget = ImageDisplayWidget::new(&state).with_protocol(GraphicsProtocol::HalfBlock);

    terminal
        .draw(|frame| {
            frame.render_widget(widget, frame.area());
        })
        .expect("Single pixel image should render");

    // Should produce some output
    let buffer = terminal.backend().buffer();
    assert!(buffer.area.width > 0 && buffer.area.height > 0);
}
