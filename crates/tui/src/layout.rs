//! TUI layout — splits the terminal into distinct areas.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Named areas of the TUI layout.
pub struct AppLayout {
    /// Top bar (title, model name, token count).
    pub top_bar: Rect,
    /// Main content area (conversation messages, tool output).
    pub content: Rect,
    /// Spinner / status line between content and input.
    pub status: Rect,
    /// Text input area at the bottom.
    pub input: Rect,
    /// Bottom status bar (mode, cost, shortcuts).
    pub bottom_bar: Rect,
}

impl AppLayout {
    /// Compute the layout for the given terminal area.
    ///
    /// Layout (top to bottom):
    /// - Top bar: 1 line
    /// - Content: fills remaining space
    /// - Status line: 1 line (spinner / progress)
    /// - Input: `input_height` lines (minimum 1)
    /// - Bottom bar: 1 line
    #[must_use]
    pub fn compute(area: Rect, input_height: u16) -> Self {
        let input_h = input_height.max(1).min(area.height.saturating_sub(4));

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),       // top bar
                Constraint::Min(1),          // content
                Constraint::Length(1),       // status
                Constraint::Length(input_h), // input
                Constraint::Length(1),       // bottom bar
            ])
            .split(area);

        Self {
            top_bar: chunks[0],
            content: chunks[1],
            status: chunks[2],
            input: chunks[3],
            bottom_bar: chunks[4],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_basic_dimensions() {
        let area = Rect::new(0, 0, 120, 40);
        let layout = AppLayout::compute(area, 3);

        assert_eq!(layout.top_bar.height, 1);
        assert_eq!(layout.status.height, 1);
        assert_eq!(layout.input.height, 3);
        assert_eq!(layout.bottom_bar.height, 1);
        // content gets the rest: 40 - 1 - 1 - 3 - 1 = 34
        assert_eq!(layout.content.height, 34);
    }

    #[test]
    fn layout_full_width() {
        let area = Rect::new(0, 0, 80, 24);
        let layout = AppLayout::compute(area, 1);

        assert_eq!(layout.top_bar.width, 80);
        assert_eq!(layout.content.width, 80);
        assert_eq!(layout.status.width, 80);
        assert_eq!(layout.input.width, 80);
        assert_eq!(layout.bottom_bar.width, 80);
    }

    #[test]
    fn layout_input_height_clamped() {
        let area = Rect::new(0, 0, 80, 10);
        // Request 100 lines of input — should be clamped
        let layout = AppLayout::compute(area, 100);
        // max input = 10 - 4 = 6
        assert_eq!(layout.input.height, 6);
    }

    #[test]
    fn layout_minimum_input_height() {
        let area = Rect::new(0, 0, 80, 24);
        let layout = AppLayout::compute(area, 0);
        assert_eq!(layout.input.height, 1);
    }

    #[test]
    fn layout_y_positions_are_contiguous() {
        let area = Rect::new(0, 0, 80, 30);
        let layout = AppLayout::compute(area, 2);

        assert_eq!(layout.top_bar.y, 0);
        assert_eq!(layout.content.y, layout.top_bar.y + layout.top_bar.height);
        assert_eq!(layout.status.y, layout.content.y + layout.content.height);
        assert_eq!(layout.input.y, layout.status.y + layout.status.height);
        assert_eq!(layout.bottom_bar.y, layout.input.y + layout.input.height);
    }

    #[test]
    fn layout_total_height_matches_area() {
        let area = Rect::new(0, 0, 100, 50);
        let layout = AppLayout::compute(area, 4);

        let total = layout.top_bar.height
            + layout.content.height
            + layout.status.height
            + layout.input.height
            + layout.bottom_bar.height;
        assert_eq!(total, area.height);
    }

    #[test]
    fn layout_small_terminal() {
        let area = Rect::new(0, 0, 40, 5);
        let layout = AppLayout::compute(area, 1);
        // 1 + content + 1 + 1 + 1 = 5 => content = 1
        assert_eq!(layout.content.height, 1);
        assert_eq!(layout.input.height, 1);
    }
}
