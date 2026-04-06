//! TUI layout — splits the terminal into distinct areas.
//!
//! Matches Claude Code's visual structure:
//! ```text
//! [crab art]  Crab Code v0.1.0            ← header (4 lines: 3 art/info + 1 separator)
//!             claude-sonnet-4-6
//!             C:\path\to\project
//! ────────────────────────────────────────
//! [conversation content]                   ← content (flexible)
//! [spinner / status]                       ← status (1 line)
//! ────────────────────────────────────────
//! ❯ input text                             ← input (variable height)
//! ────────────────────────────────────────
//! ? for shortcuts                          ← bottom_bar (1 line)
//! ```

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Default sidebar width in columns.
pub const DEFAULT_SIDEBAR_WIDTH: u16 = 24;

/// Named areas of the TUI layout.
pub struct AppLayout {
    /// Header area (3 lines art/info + 1 line separator = 4 lines total).
    pub header: Rect,
    /// Optional sidebar (session list). `None` when sidebar is hidden.
    pub sidebar: Option<Rect>,
    /// Main content area (conversation messages, tool output).
    pub content: Rect,
    /// Spinner / status line.
    pub status: Rect,
    /// Separator line above input (`───`).
    pub separator_top: Rect,
    /// Text input area (no border, just `❯` prompt).
    pub input: Rect,
    /// Separator line below input (`───`).
    pub separator_bottom: Rect,
    /// Bottom status bar (`? for shortcuts`).
    pub bottom_bar: Rect,
}

impl AppLayout {
    /// Compute the layout for the given terminal area.
    #[must_use]
    pub fn compute(area: Rect, input_height: u16) -> Self {
        Self::compute_with_sidebar(area, input_height, false, DEFAULT_SIDEBAR_WIDTH)
    }

    /// Compute layout with optional sidebar panel.
    #[must_use]
    pub fn compute_with_sidebar(
        area: Rect,
        input_height: u16,
        show_sidebar: bool,
        sidebar_width: u16,
    ) -> Self {
        // Fixed overhead: header(4) + status(1) + sep_top(1) + sep_bottom(1) + bottom_bar(1) = 8
        let input_h = input_height.max(1).min(area.height.saturating_sub(8));

        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4),       // header (3 art/info + 1 separator)
                Constraint::Min(1),          // content
                Constraint::Length(1),       // status
                Constraint::Length(1),       // separator above input
                Constraint::Length(input_h), // input
                Constraint::Length(1),       // separator below input
                Constraint::Length(1),       // bottom bar
            ])
            .split(area);

        let (sidebar, content) = if show_sidebar && area.width > sidebar_width + 20 {
            let horizontal = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(sidebar_width), Constraint::Min(1)])
                .split(vertical[1]);
            (Some(horizontal[0]), horizontal[1])
        } else {
            (None, vertical[1])
        };

        Self {
            header: vertical[0],
            sidebar,
            content,
            status: vertical[2],
            separator_top: vertical[3],
            input: vertical[4],
            separator_bottom: vertical[5],
            bottom_bar: vertical[6],
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

        assert_eq!(layout.header.height, 4);
        assert_eq!(layout.status.height, 1);
        assert_eq!(layout.separator_top.height, 1);
        assert_eq!(layout.input.height, 3);
        assert_eq!(layout.separator_bottom.height, 1);
        assert_eq!(layout.bottom_bar.height, 1);
        // content: 40 - 4 - 1 - 1 - 3 - 1 - 1 = 29
        assert_eq!(layout.content.height, 29);
        assert!(layout.sidebar.is_none());
    }

    #[test]
    fn layout_full_width() {
        let area = Rect::new(0, 0, 80, 24);
        let layout = AppLayout::compute(area, 1);

        assert_eq!(layout.header.width, 80);
        assert_eq!(layout.content.width, 80);
        assert_eq!(layout.status.width, 80);
        assert_eq!(layout.input.width, 80);
        assert_eq!(layout.bottom_bar.width, 80);
    }

    #[test]
    fn layout_input_height_clamped() {
        let area = Rect::new(0, 0, 80, 15);
        let layout = AppLayout::compute(area, 100);
        // Fixed overhead = 8, input = min(100, 15-8) = 7
        // But ratatui reserves Min(1) for content, so actual input ≤ 6
        assert!(layout.input.height >= 1);
        assert!(layout.content.height >= 1);
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

        assert_eq!(layout.header.y, 0);
        assert_eq!(layout.content.y, layout.header.y + layout.header.height);
        assert_eq!(layout.status.y, layout.content.y + layout.content.height);
        assert_eq!(layout.separator_top.y, layout.status.y + layout.status.height);
        assert_eq!(layout.input.y, layout.separator_top.y + layout.separator_top.height);
        assert_eq!(layout.separator_bottom.y, layout.input.y + layout.input.height);
        assert_eq!(
            layout.bottom_bar.y,
            layout.separator_bottom.y + layout.separator_bottom.height
        );
    }

    #[test]
    fn layout_total_height_matches_area() {
        let area = Rect::new(0, 0, 100, 50);
        let layout = AppLayout::compute(area, 4);

        let total = layout.header.height
            + layout.content.height
            + layout.status.height
            + layout.separator_top.height
            + layout.input.height
            + layout.separator_bottom.height
            + layout.bottom_bar.height;
        assert_eq!(total, area.height);
    }

    #[test]
    fn layout_small_terminal() {
        let area = Rect::new(0, 0, 40, 12);
        let layout = AppLayout::compute(area, 1);
        // 4 + content + 1 + 1 + 1 + 1 + 1 = 12 => content = 3
        assert_eq!(layout.content.height, 3);
        assert_eq!(layout.input.height, 1);
    }

    #[test]
    fn layout_with_sidebar() {
        let area = Rect::new(0, 0, 120, 40);
        let layout = AppLayout::compute_with_sidebar(area, 3, true, 24);

        assert!(layout.sidebar.is_some());
        let sidebar = layout.sidebar.unwrap();
        assert_eq!(sidebar.width, 24);
        assert_eq!(sidebar.width + layout.content.width, 120);
        assert_eq!(sidebar.height, layout.content.height);
    }

    #[test]
    fn layout_sidebar_hidden_when_requested() {
        let area = Rect::new(0, 0, 120, 40);
        let layout = AppLayout::compute_with_sidebar(area, 3, false, 24);
        assert!(layout.sidebar.is_none());
        assert_eq!(layout.content.width, 120);
    }

    #[test]
    fn layout_sidebar_hidden_on_narrow_terminal() {
        let area = Rect::new(0, 0, 40, 24);
        let layout = AppLayout::compute_with_sidebar(area, 1, true, 24);
        assert!(layout.sidebar.is_none());
        assert_eq!(layout.content.width, 40);
    }

    #[test]
    fn layout_sidebar_y_matches_content() {
        let area = Rect::new(0, 0, 100, 30);
        let layout = AppLayout::compute_with_sidebar(area, 2, true, 24);

        let sidebar = layout.sidebar.unwrap();
        assert_eq!(sidebar.y, layout.content.y);
        assert_eq!(sidebar.height, layout.content.height);
    }

    #[test]
    fn layout_default_sidebar_width() {
        assert_eq!(DEFAULT_SIDEBAR_WIDTH, 24);
    }
}
