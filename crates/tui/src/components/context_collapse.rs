//! UI widget for collapsing/expanding sections of conversation context.
//!
//! Long tool outputs and system messages can clutter the conversation view.
//! This widget renders a collapsible section with a toggle indicator that
//! the user can expand or collapse to manage visual noise.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};

// ── Types ─────────────────────────────────────────────────────────────

/// A single collapsible section within a conversation.
#[derive(Debug, Clone)]
pub struct CollapsibleSection {
    /// Label shown in the section header (e.g. "Tool output: Bash").
    pub label: String,
    /// Full content (shown when expanded).
    pub content: String,
    /// Whether this section is currently collapsed.
    pub collapsed: bool,
}

impl CollapsibleSection {
    /// Create a new section, expanded by default.
    pub fn new(label: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            content: content.into(),
            collapsed: false,
        }
    }

    /// Toggle the collapsed state.
    pub fn toggle(&mut self) {
        self.collapsed = !self.collapsed;
    }

    /// The indicator character for the current state.
    fn indicator(&self) -> &'static str {
        if self.collapsed { "+" } else { "-" }
    }
}

// ── Widget ────────────────────────────────────────────────────────────

/// A widget that renders a list of collapsible context sections.
///
/// Each section shows a clickable header and, when expanded, its full
/// content. Collapsed sections show only the header with a `[+]` indicator.
pub struct ContextCollapse {
    /// The sections to render.
    sections: Vec<CollapsibleSection>,
    /// Index of the currently focused section (for keyboard navigation).
    focused: usize,
}

impl ContextCollapse {
    /// Create a new context collapse widget with the given sections.
    #[must_use]
    pub fn new(sections: Vec<CollapsibleSection>) -> Self {
        Self {
            sections,
            focused: 0,
        }
    }

    /// Toggle the currently focused section.
    pub fn toggle_focused(&mut self) {
        if let Some(section) = self.sections.get_mut(self.focused) {
            section.toggle();
        }
    }

    /// Move focus to the next section.
    pub fn focus_next(&mut self) {
        if !self.sections.is_empty() {
            self.focused = (self.focused + 1) % self.sections.len();
        }
    }

    /// Move focus to the previous section.
    pub fn focus_prev(&mut self) {
        if !self.sections.is_empty() {
            self.focused = self
                .focused
                .checked_sub(1)
                .unwrap_or(self.sections.len() - 1);
        }
    }

    /// The currently focused section index.
    #[must_use]
    pub const fn focused(&self) -> usize {
        self.focused
    }

    /// Add a new section at the end.
    pub fn push_section(&mut self, section: CollapsibleSection) {
        self.sections.push(section);
    }

    /// The number of sections.
    #[must_use]
    pub fn section_count(&self) -> usize {
        self.sections.len()
    }

    /// Get a reference to the sections.
    #[must_use]
    pub fn sections(&self) -> &[CollapsibleSection] {
        &self.sections
    }
}

impl Widget for &ContextCollapse {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width < 10 {
            return;
        }

        let mut y = area.y;

        for (i, section) in self.sections.iter().enumerate() {
            if y >= area.y + area.height {
                break;
            }

            // Header line
            let is_focused = i == self.focused;
            let header_style = if is_focused {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let header = Line::from(vec![
                Span::styled(format!("[{}] ", section.indicator()), header_style),
                Span::styled(&section.label, header_style),
            ]);

            let header_area = Rect::new(area.x, y, area.width, 1);
            Widget::render(header, header_area, buf);
            y += 1;

            // Content (only when expanded)
            if !section.collapsed && y < area.y + area.height {
                let remaining_height = (area.y + area.height).saturating_sub(y);
                let content_area = Rect::new(
                    area.x + 2,
                    y,
                    area.width.saturating_sub(2),
                    remaining_height,
                );

                let content = Paragraph::new(section.content.as_str())
                    .style(Style::default().fg(Color::Gray))
                    .block(Block::default().borders(Borders::NONE))
                    .wrap(Wrap { trim: true });

                Widget::render(content, content_area, buf);
                // Estimate lines used (rough: content length / width + 1)
                let lines_used = if area.width > 2 {
                    (section.content.len() as u16 / (area.width - 2)).min(remaining_height) + 1
                } else {
                    1
                };
                y += lines_used;
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapsible_section_new() {
        let section = CollapsibleSection::new("Header", "Body content");
        assert_eq!(section.label, "Header");
        assert!(!section.collapsed);
        assert_eq!(section.indicator(), "-");
    }

    #[test]
    fn collapsible_section_toggle() {
        let mut section = CollapsibleSection::new("Header", "Body");
        assert!(!section.collapsed);
        section.toggle();
        assert!(section.collapsed);
        assert_eq!(section.indicator(), "+");
        section.toggle();
        assert!(!section.collapsed);
    }

    #[test]
    fn context_collapse_navigation() {
        let sections = vec![
            CollapsibleSection::new("A", "content a"),
            CollapsibleSection::new("B", "content b"),
            CollapsibleSection::new("C", "content c"),
        ];
        let mut widget = ContextCollapse::new(sections);
        assert_eq!(widget.focused(), 0);
        assert_eq!(widget.section_count(), 3);

        widget.focus_next();
        assert_eq!(widget.focused(), 1);

        widget.focus_next();
        assert_eq!(widget.focused(), 2);

        // Wraps around
        widget.focus_next();
        assert_eq!(widget.focused(), 0);

        // Wraps backwards
        widget.focus_prev();
        assert_eq!(widget.focused(), 2);
    }

    #[test]
    fn toggle_focused_section() {
        let sections = vec![
            CollapsibleSection::new("A", "content a"),
            CollapsibleSection::new("B", "content b"),
        ];
        let mut widget = ContextCollapse::new(sections);

        assert!(!widget.sections[0].collapsed);
        widget.toggle_focused();
        assert!(widget.sections[0].collapsed);
    }

    #[test]
    fn empty_sections() {
        let widget = ContextCollapse::new(vec![]);
        assert_eq!(widget.section_count(), 0);
        assert_eq!(widget.focused(), 0);
    }

    #[test]
    fn renders_without_panic() {
        let sections = vec![CollapsibleSection::new("Section 1", "Some content here")];
        let widget = ContextCollapse::new(sections);
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        Widget::render(&widget, area, &mut buf);
    }

    #[test]
    fn tiny_area_does_not_panic() {
        let sections = vec![CollapsibleSection::new("A", "content")];
        let widget = ContextCollapse::new(sections);
        let area = Rect::new(0, 0, 5, 1);
        let mut buf = Buffer::empty(area);
        Widget::render(&widget, area, &mut buf);
    }
}
