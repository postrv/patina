//! Image display widget for rendering images in the terminal.
//!
//! This widget supports multiple graphics protocols:
//! - **Half-block**: Universal fallback using Unicode half-block characters (▀/▄)
//! - **Sixel**: Bitmap graphics protocol supported by xterm, mintty, mlterm
//! - **Kitty**: Kitty terminal graphics protocol
//! - **iTerm2**: iTerm2 inline images protocol
//!
//! # Example
//!
//! ```rust,ignore
//! use patina::tui::widgets::image_display::{ImageDisplayState, ImageDisplayWidget, GraphicsProtocol};
//!
//! // Create state from RGBA pixel data
//! let pixels: Vec<u8> = vec![255, 0, 0, 255].repeat(100); // 10x10 red image
//! let state = ImageDisplayState::new(10, 10, pixels);
//!
//! // Create widget with half-block rendering
//! let widget = ImageDisplayWidget::new(&state)
//!     .with_protocol(GraphicsProtocol::HalfBlock);
//!
//! // Render in a frame
//! // frame.render_widget(widget, area);
//! ```

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

// Re-export GraphicsProtocol and detect_graphics_protocol from the terminal module
// for backward compatibility with existing code that imports from this module.
pub use crate::terminal::{detect_graphics_protocol, GraphicsProtocol};

/// State for the image display widget.
///
/// Holds the pixel data and dimensions of an image to be rendered.
#[derive(Debug, Clone)]
pub struct ImageDisplayState {
    /// Width of the image in pixels.
    width: u32,

    /// Height of the image in pixels.
    height: u32,

    /// RGBA pixel data (4 bytes per pixel).
    pixels: Vec<u8>,
}

impl ImageDisplayState {
    /// Creates a new image display state.
    ///
    /// # Arguments
    ///
    /// * `width` - Width of the image in pixels
    /// * `height` - Height of the image in pixels
    /// * `pixels` - RGBA pixel data (4 bytes per pixel)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use patina::tui::widgets::image_display::ImageDisplayState;
    ///
    /// // Create a 10x10 red image
    /// let pixels: Vec<u8> = vec![255, 0, 0, 255].repeat(100);
    /// let state = ImageDisplayState::new(10, 10, pixels);
    /// ```
    #[must_use]
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        Self {
            width,
            height,
            pixels,
        }
    }

    /// Returns the width of the image in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Returns the height of the image in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Returns a reference to the RGBA pixel data.
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Returns the aspect ratio (width / height).
    ///
    /// Returns 1.0 for zero-height images to avoid division by zero.
    #[must_use]
    pub fn aspect_ratio(&self) -> f64 {
        if self.height == 0 {
            1.0
        } else {
            self.width as f64 / self.height as f64
        }
    }
}

/// Widget for rendering an image in the terminal.
pub struct ImageDisplayWidget<'a> {
    /// The image state to render.
    state: &'a ImageDisplayState,

    /// Graphics protocol to use for rendering.
    protocol: GraphicsProtocol,

    /// Maximum width in terminal columns.
    max_width: Option<u16>,

    /// Maximum height in terminal rows.
    max_height: Option<u16>,
}

impl<'a> ImageDisplayWidget<'a> {
    /// Creates a new image display widget.
    ///
    /// # Arguments
    ///
    /// * `state` - The image state containing pixel data
    #[must_use]
    pub fn new(state: &'a ImageDisplayState) -> Self {
        Self {
            state,
            protocol: detect_graphics_protocol(),
            max_width: None,
            max_height: None,
        }
    }

    /// Sets the graphics protocol to use for rendering.
    ///
    /// # Arguments
    ///
    /// * `protocol` - The graphics protocol to use
    #[must_use]
    pub fn with_protocol(mut self, protocol: GraphicsProtocol) -> Self {
        self.protocol = protocol;
        self
    }

    /// Sets the maximum dimensions for rendering.
    ///
    /// # Arguments
    ///
    /// * `max_width` - Maximum width in terminal columns
    /// * `max_height` - Maximum height in terminal rows
    #[must_use]
    pub fn with_max_dimensions(mut self, max_width: u16, max_height: u16) -> Self {
        self.max_width = Some(max_width);
        self.max_height = Some(max_height);
        self
    }

    /// Calculates the display dimensions that fit within constraints while
    /// maintaining aspect ratio.
    ///
    /// # Arguments
    ///
    /// * `available_width` - Available width in terminal columns
    /// * `available_height` - Available height in terminal rows
    ///
    /// # Returns
    ///
    /// A tuple of (width, height) in terminal units.
    #[must_use]
    pub fn calculate_display_dimensions(
        &self,
        available_width: u16,
        available_height: u16,
    ) -> (u16, u16) {
        let max_w = self
            .max_width
            .unwrap_or(available_width)
            .min(available_width);
        let max_h = self
            .max_height
            .unwrap_or(available_height)
            .min(available_height);

        if self.state.width == 0 || self.state.height == 0 {
            return (0, 0);
        }

        // For half-block rendering, each terminal row represents 2 pixel rows
        let pixel_rows_per_cell = 2u32;

        // Calculate scaling factor to fit within bounds
        let scale_w = max_w as f64 / self.state.width as f64;
        let scale_h = (max_h as f64 * pixel_rows_per_cell as f64) / self.state.height as f64;

        let scale = scale_w.min(scale_h);

        let display_width = ((self.state.width as f64 * scale).round() as u16).max(1);
        let display_height =
            ((self.state.height as f64 * scale / pixel_rows_per_cell as f64).round() as u16).max(1);

        (display_width.min(max_w), display_height.min(max_h))
    }

    /// Renders a placeholder for unsupported terminals.
    fn render_placeholder(&self, area: Rect, buf: &mut Buffer) {
        let text = format!("[Image {}x{}]", self.state.width, self.state.height);

        let paragraph = Paragraph::new(Line::from(vec![Span::styled(
            text,
            Style::default().fg(Color::Gray),
        )]));

        paragraph.render(area, buf);
    }

    /// Renders the image using half-block characters.
    ///
    /// Uses Unicode half-block characters (▀ upper, ▄ lower) to display
    /// two vertical pixels per terminal cell.
    fn render_halfblock(&self, area: Rect, buf: &mut Buffer) {
        if self.state.width == 0 || self.state.height == 0 || self.state.pixels.is_empty() {
            return;
        }

        let (display_width, display_height) =
            self.calculate_display_dimensions(area.width, area.height);

        // Scale factors for sampling
        let x_scale = self.state.width as f64 / display_width as f64;
        let y_scale = self.state.height as f64 / (display_height as f64 * 2.0);

        for row in 0..display_height.min(area.height) {
            for col in 0..display_width.min(area.width) {
                // Sample top pixel
                let top_y = ((row as f64 * 2.0) * y_scale) as u32;
                let top_x = (col as f64 * x_scale) as u32;

                // Sample bottom pixel
                let bot_y = ((row as f64 * 2.0 + 1.0) * y_scale) as u32;
                let bot_x = top_x;

                let top_color = self.sample_pixel(top_x, top_y);
                let bot_color = self.sample_pixel(bot_x, bot_y);

                // Use upper half block (▀) with fg=top, bg=bottom
                let cell = buf.cell_mut((area.x + col, area.y + row));
                if let Some(cell) = cell {
                    cell.set_symbol("▀").set_fg(top_color).set_bg(bot_color);
                }
            }
        }
    }

    /// Samples a pixel at the given coordinates, returning a terminal color.
    fn sample_pixel(&self, x: u32, y: u32) -> Color {
        let x = x.min(self.state.width.saturating_sub(1));
        let y = y.min(self.state.height.saturating_sub(1));

        let idx = ((y * self.state.width + x) * 4) as usize;

        if idx + 3 < self.state.pixels.len() {
            let r = self.state.pixels[idx];
            let g = self.state.pixels[idx + 1];
            let b = self.state.pixels[idx + 2];
            Color::Rgb(r, g, b)
        } else {
            Color::Reset
        }
    }
}

impl Widget for ImageDisplayWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        match self.protocol {
            GraphicsProtocol::Unsupported => {
                self.render_placeholder(area, buf);
            }
            GraphicsProtocol::HalfBlock => {
                self.render_halfblock(area, buf);
            }
            GraphicsProtocol::Sixel | GraphicsProtocol::Kitty | GraphicsProtocol::ITerm2 => {
                // TODO: Implement advanced graphics protocols
                // For now, fall back to half-block
                self.render_halfblock(area, buf);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_creation() {
        let pixels = vec![255, 0, 0, 255, 0, 255, 0, 255];
        let state = ImageDisplayState::new(2, 1, pixels.clone());

        assert_eq!(state.width(), 2);
        assert_eq!(state.height(), 1);
        assert_eq!(state.pixels(), &pixels);
    }

    #[test]
    fn test_aspect_ratio() {
        let state = ImageDisplayState::new(16, 9, vec![]);
        assert!((state.aspect_ratio() - 16.0 / 9.0).abs() < 0.001);
    }

    #[test]
    fn test_aspect_ratio_zero_height() {
        let state = ImageDisplayState::new(10, 0, vec![]);
        assert_eq!(state.aspect_ratio(), 1.0);
    }

    #[test]
    fn test_graphics_protocol_variants() {
        // Verify all variants exist
        let _sixel = GraphicsProtocol::Sixel;
        let _kitty = GraphicsProtocol::Kitty;
        let _iterm = GraphicsProtocol::ITerm2;
        let _halfblock = GraphicsProtocol::HalfBlock;
        let _unsupported = GraphicsProtocol::Unsupported;
    }

    #[test]
    fn test_detect_graphics_protocol_returns_valid() {
        let protocol = detect_graphics_protocol();
        // Should return a valid protocol
        assert!(matches!(
            protocol,
            GraphicsProtocol::HalfBlock
                | GraphicsProtocol::Sixel
                | GraphicsProtocol::Kitty
                | GraphicsProtocol::ITerm2
                | GraphicsProtocol::Unsupported
        ));
    }

    #[test]
    fn test_calculate_display_dimensions_preserves_aspect() {
        let state = ImageDisplayState::new(100, 50, vec![]);
        let widget = ImageDisplayWidget::new(&state);

        let (w, h) = widget.calculate_display_dimensions(20, 20);

        // 100:50 = 2:1 aspect ratio
        // Should fit in 20x20 while maintaining aspect
        assert!(w <= 20);
        assert!(h <= 20);

        // With half-block rendering, each terminal cell is 1 pixel wide x 2 pixels tall
        // So the visual aspect ratio = w / (h * 2) in pixel space
        // Original aspect = 100/50 = 2.0
        // Visual aspect should be approximately 2.0
        let visual_aspect = w as f64 / (h as f64 * 2.0);
        let original_aspect = 100.0 / 50.0;
        assert!(
            (visual_aspect - original_aspect).abs() < 0.5,
            "Visual aspect ({}) should be near original ({}), terminal dims: {}x{}",
            visual_aspect,
            original_aspect,
            w,
            h
        );
    }

    #[test]
    fn test_sample_pixel_bounds() {
        let pixels = vec![255, 128, 64, 255]; // Single pixel
        let state = ImageDisplayState::new(1, 1, pixels);
        let widget = ImageDisplayWidget::new(&state);

        // Sampling within bounds
        let color = widget.sample_pixel(0, 0);
        assert_eq!(color, Color::Rgb(255, 128, 64));

        // Sampling out of bounds should clamp
        let clamped = widget.sample_pixel(100, 100);
        assert_eq!(clamped, Color::Rgb(255, 128, 64));
    }
}
