//! Buddy notification widget — shows messages from the buddy companion.
//!
//! Renders a small speech-bubble overlay next to (or below) the buddy
//! sprite. Used to surface personality-driven tips and encouragement.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use std::time::{Duration, Instant};

/// Default duration for a buddy notification before it auto-dismisses.
const DEFAULT_DURATION: Duration = Duration::from_secs(5);

/// A notification message from the buddy companion.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct BuddyNotification {
    /// Message text.
    message: String,
    /// When the notification was created.
    created_at: Instant,
    /// Auto-dismiss duration.
    duration: Duration,
}

#[allow(dead_code)]
impl BuddyNotification {
    /// Create a new buddy notification with the default duration.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            created_at: Instant::now(),
            duration: DEFAULT_DURATION,
        }
    }

    /// Create a notification with a custom duration.
    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    /// Whether this notification has expired.
    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() >= self.duration
    }

    /// The message text.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Time remaining before expiry, or `None` if already expired.
    pub fn time_remaining(&self) -> Option<Duration> {
        self.duration.checked_sub(self.created_at.elapsed())
    }
}

/// Renders a [`BuddyNotification`] as a speech-bubble line.
#[allow(dead_code)]
pub struct BuddyNotificationWidget<'a> {
    notification: &'a BuddyNotification,
}

#[allow(dead_code)]
impl<'a> BuddyNotificationWidget<'a> {
    /// Create a new widget for the given notification.
    pub fn new(notification: &'a BuddyNotification) -> Self {
        Self { notification }
    }
}

impl Widget for BuddyNotificationWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 5 || area.height == 0 {
            return;
        }

        if self.notification.is_expired() {
            return;
        }

        // Truncate message to fit the area
        let max_len = (area.width as usize).saturating_sub(4); // "< " + " >"
        let msg = if self.notification.message.len() > max_len {
            format!("{}...", &self.notification.message[..max_len.saturating_sub(3)])
        } else {
            self.notification.message.clone()
        };

        let line = Line::from(vec![
            Span::styled(
                "< ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(msg, Style::default().fg(Color::White)),
            Span::styled(
                " >",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);

        let line_area = Rect { height: 1, ..area };
        Widget::render(line, line_area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_message_accessor() {
        let n = BuddyNotification::new("Hello from your buddy!");
        assert_eq!(n.message(), "Hello from your buddy!");
        assert!(!n.is_expired());
    }

    #[test]
    fn notification_zero_duration_expires_immediately() {
        let n = BuddyNotification::new("poof").with_duration(Duration::ZERO);
        assert!(n.is_expired());
        assert!(n.time_remaining().is_none());
    }

    #[test]
    fn notification_with_long_duration_not_expired() {
        let n = BuddyNotification::new("staying").with_duration(Duration::from_secs(60));
        assert!(!n.is_expired());
        assert!(n.time_remaining().is_some());
    }

    #[test]
    fn widget_renders_message() {
        let n = BuddyNotification::new("Test message");
        let widget = BuddyNotificationWidget::new(&n);

        let area = Rect::new(0, 0, 40, 1);
        let mut buf = Buffer::empty(area);
        Widget::render(widget, area, &mut buf);

        let content: String = (0..area.width)
            .map(|x| buf.cell((x, 0)).unwrap().symbol().to_string())
            .collect();
        assert!(content.contains("Test message"));
    }

    #[test]
    fn widget_does_not_render_expired() {
        let n = BuddyNotification::new("gone").with_duration(Duration::ZERO);
        let widget = BuddyNotificationWidget::new(&n);

        let area = Rect::new(0, 0, 40, 1);
        let mut buf = Buffer::empty(area);
        Widget::render(widget, area, &mut buf);

        let content: String = (0..area.width)
            .map(|x| buf.cell((x, 0)).unwrap().symbol().to_string())
            .collect();
        assert_eq!(content.trim(), "");
    }

    #[test]
    fn widget_handles_tiny_area() {
        let n = BuddyNotification::new("hello");
        let widget = BuddyNotificationWidget::new(&n);

        let area = Rect::new(0, 0, 3, 1);
        let mut buf = Buffer::empty(area);
        Widget::render(widget, area, &mut buf);
        // Should not panic
    }
}
