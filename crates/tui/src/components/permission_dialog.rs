//! Dedicated permission prompt dialog with risk-level display.
//!
//! A more detailed permission dialog than the base `dialog.rs`, supporting
//! four risk levels and four response choices including "always deny".

use crossterm::event::KeyCode;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};

/// Risk level for a tool execution request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskLevel {
    /// No side effects (read-only tools).
    Low,
    /// Moderate side effects (file writes, network reads).
    Medium,
    /// Significant side effects (shell commands, file deletion).
    High,
    /// Potentially irreversible (rm -rf, git push --force).
    Critical,
}

impl RiskLevel {
    /// Color associated with this risk level.
    #[must_use]
    pub fn color(self) -> Color {
        match self {
            Self::Low => Color::Green,
            Self::Medium => Color::Yellow,
            Self::High => Color::Red,
            Self::Critical => Color::LightRed,
        }
    }

    /// Human-readable label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

/// User response to a permission prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionChoice {
    /// Allow this single execution.
    Allow,
    /// Deny this single execution.
    Deny,
    /// Always allow this tool (for this session).
    AlwaysAllow,
    /// Always deny this tool (for this session).
    AlwaysDeny,
}

/// Dedicated permission dialog with full risk-level information.
pub struct PermissionDialog {
    /// Name of the tool requesting permission.
    pub tool_name: String,
    /// Description of what the tool will do.
    pub description: String,
    /// Risk level of the operation.
    pub risk_level: RiskLevel,
    /// Request identifier for tracking.
    pub request_id: String,
    selected: usize,
    options: Vec<(&'static str, PermissionChoice)>,
}

impl PermissionDialog {
    /// Create a new permission dialog.
    pub fn new(
        tool_name: impl Into<String>,
        description: impl Into<String>,
        risk_level: RiskLevel,
        request_id: impl Into<String>,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            description: description.into(),
            risk_level,
            request_id: request_id.into(),
            selected: 0,
            options: vec![
                ("Allow", PermissionChoice::Allow),
                ("Deny", PermissionChoice::Deny),
                ("Always Allow", PermissionChoice::AlwaysAllow),
                ("Always Deny", PermissionChoice::AlwaysDeny),
            ],
        }
    }

    /// Handle a key event. Returns `Some(choice)` when the user confirms.
    pub fn handle_key(&mut self, code: KeyCode) -> Option<PermissionChoice> {
        match code {
            KeyCode::Left | KeyCode::Char('h') => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
                None
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if self.selected < self.options.len() - 1 {
                    self.selected += 1;
                }
                None
            }
            KeyCode::Enter | KeyCode::Char(' ') => Some(self.options[self.selected].1),
            KeyCode::Char('y' | 'Y') => Some(PermissionChoice::Allow),
            KeyCode::Char('n' | 'N') | KeyCode::Esc => Some(PermissionChoice::Deny),
            _ => None,
        }
    }

    /// Currently selected option index.
    #[must_use]
    pub const fn selected(&self) -> usize {
        self.selected
    }

    /// Compute the centered dialog area within the given terminal area.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn dialog_area(terminal: Rect) -> Rect {
        let width = 64.min(terminal.width.saturating_sub(4));
        let height = 12.min(terminal.height.saturating_sub(2));
        let x = (terminal.width.saturating_sub(width)) / 2;
        let y = (terminal.height.saturating_sub(height)) / 2;
        Rect::new(x, y, width, height)
    }
}

impl Widget for &PermissionDialog {
    #[allow(clippy::cast_possible_truncation)]
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height < 6 || area.width < 24 {
            return;
        }

        Widget::render(Clear, area, buf);

        let border_color = self.risk_level.color();
        let block = Block::default()
            .title(" Permission Required ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color));

        let inner = block.inner(area);
        Widget::render(block, area, buf);

        if inner.height < 4 || inner.width < 10 {
            return;
        }

        let chunks = Layout::vertical([
            Constraint::Length(1), // tool name + risk
            Constraint::Length(1), // spacer
            Constraint::Min(1),    // description
            Constraint::Length(1), // spacer
            Constraint::Length(1), // buttons
        ])
        .split(inner);

        // Tool name + risk badge
        let risk_style = Style::default()
            .fg(self.risk_level.color())
            .add_modifier(Modifier::BOLD);
        let tool_line = Line::from(vec![
            Span::styled("Tool: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                &self.tool_name,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled("[", Style::default().fg(Color::DarkGray)),
            Span::styled(self.risk_level.label(), risk_style),
            Span::styled("]", Style::default().fg(Color::DarkGray)),
        ]);
        Widget::render(tool_line, chunks[0], buf);

        // Description
        let desc = Paragraph::new(self.description.as_str())
            .style(Style::default().fg(Color::Gray))
            .wrap(Wrap { trim: true });
        Widget::render(desc, chunks[2], buf);

        // Buttons
        let button_spans: Vec<Span> = self
            .options
            .iter()
            .enumerate()
            .flat_map(|(i, (label, _))| {
                let style = if i == self.selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                let mut spans = vec![Span::styled(format!(" {label} "), style)];
                if i + 1 < self.options.len() {
                    spans.push(Span::raw("  "));
                }
                spans
            })
            .collect();

        let buttons = Paragraph::new(Line::from(button_spans)).alignment(Alignment::Center);
        Widget::render(buttons, chunks[4], buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialog() -> PermissionDialog {
        PermissionDialog::new("bash", "rm -rf /tmp/cache", RiskLevel::Critical, "req_1")
    }

    #[test]
    fn new_dialog_defaults() {
        let d = dialog();
        assert_eq!(d.tool_name, "bash");
        assert_eq!(d.risk_level, RiskLevel::Critical);
        assert_eq!(d.selected(), 0);
    }

    #[test]
    fn navigate_options() {
        let mut d = dialog();
        d.handle_key(KeyCode::Right);
        assert_eq!(d.selected(), 1);
        d.handle_key(KeyCode::Right);
        assert_eq!(d.selected(), 2);
        d.handle_key(KeyCode::Right);
        assert_eq!(d.selected(), 3);
        // Clamp at end
        d.handle_key(KeyCode::Right);
        assert_eq!(d.selected(), 3);
        d.handle_key(KeyCode::Left);
        assert_eq!(d.selected(), 2);
    }

    #[test]
    fn enter_confirms() {
        let mut d = dialog();
        assert_eq!(d.handle_key(KeyCode::Enter), Some(PermissionChoice::Allow));
    }

    #[test]
    fn shortcut_keys() {
        let mut d = dialog();
        assert_eq!(
            d.handle_key(KeyCode::Char('y')),
            Some(PermissionChoice::Allow)
        );
        assert_eq!(
            d.handle_key(KeyCode::Char('n')),
            Some(PermissionChoice::Deny)
        );
        assert_eq!(d.handle_key(KeyCode::Esc), Some(PermissionChoice::Deny));
    }

    #[test]
    fn risk_levels() {
        assert_eq!(RiskLevel::Low.label(), "low");
        assert_eq!(RiskLevel::Medium.label(), "medium");
        assert_eq!(RiskLevel::High.label(), "high");
        assert_eq!(RiskLevel::Critical.label(), "critical");
        assert_eq!(RiskLevel::Low.color(), Color::Green);
        assert_eq!(RiskLevel::Critical.color(), Color::LightRed);
    }

    #[test]
    fn dialog_area_centered() {
        let terminal = Rect::new(0, 0, 80, 24);
        let area = PermissionDialog::dialog_area(terminal);
        assert!(area.x > 0);
        assert!(area.y > 0);
        assert!(area.width <= 64);
        assert!(area.height <= 12);
    }

    #[test]
    fn renders_without_panic() {
        let d = dialog();
        let area = Rect::new(0, 0, 64, 12);
        let mut buf = Buffer::empty(area);
        Widget::render(&d, area, &mut buf);
    }

    #[test]
    fn tiny_area_does_not_panic() {
        let d = dialog();
        let area = Rect::new(0, 0, 10, 3);
        let mut buf = Buffer::empty(area);
        Widget::render(&d, area, &mut buf);
    }
}
