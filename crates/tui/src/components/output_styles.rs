//! Configurable output formatting styles for different content types.
//!
//! Provides a centralized style registry for rendering different kinds of
//! content (code, errors, warnings, tool results, etc.) in the TUI.

use ratatui::style::{Color, Modifier, Style};

/// Classification of content being rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContentType {
    /// Source code or code blocks.
    Code,
    /// Error messages.
    Error,
    /// Warning messages.
    Warning,
    /// Informational messages.
    Info,
    /// Tool execution results.
    ToolResult,
    /// System messages (status, internal).
    SystemMessage,
    /// User input text.
    UserInput,
    /// Assistant response text.
    AssistantResponse,
    /// Diff additions.
    DiffAdd,
    /// Diff deletions.
    DiffRemove,
    /// Muted / secondary text.
    Muted,
}

/// Configurable output formatting styles.
///
/// Each content type maps to a ratatui `Style`. The default configuration
/// uses a dark-theme palette suitable for most terminals.
pub struct OutputStyles {
    code: Style,
    error: Style,
    warning: Style,
    info: Style,
    tool_result: Style,
    system_message: Style,
    user_input: Style,
    assistant_response: Style,
    diff_add: Style,
    diff_remove: Style,
    muted: Style,
}

impl OutputStyles {
    /// Create the default style configuration.
    #[must_use]
    pub fn default_styles() -> Self {
        Self {
            code: Style::default().fg(Color::White),
            error: Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            warning: Style::default().fg(Color::Yellow),
            info: Style::default().fg(Color::Cyan),
            tool_result: Style::default().fg(Color::Gray),
            system_message: Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
            user_input: Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
            assistant_response: Style::default().fg(Color::White),
            diff_add: Style::default().fg(Color::Green),
            diff_remove: Style::default().fg(Color::Red),
            muted: Style::default().fg(Color::DarkGray),
        }
    }

    /// Get the style for a given content type.
    #[must_use]
    pub fn style_for(&self, content_type: ContentType) -> Style {
        match content_type {
            ContentType::Code => self.code,
            ContentType::Error => self.error,
            ContentType::Warning => self.warning,
            ContentType::Info => self.info,
            ContentType::ToolResult => self.tool_result,
            ContentType::SystemMessage => self.system_message,
            ContentType::UserInput => self.user_input,
            ContentType::AssistantResponse => self.assistant_response,
            ContentType::DiffAdd => self.diff_add,
            ContentType::DiffRemove => self.diff_remove,
            ContentType::Muted => self.muted,
        }
    }

    /// Override the style for a specific content type.
    pub fn set_style(&mut self, content_type: ContentType, style: Style) {
        match content_type {
            ContentType::Code => self.code = style,
            ContentType::Error => self.error = style,
            ContentType::Warning => self.warning = style,
            ContentType::Info => self.info = style,
            ContentType::ToolResult => self.tool_result = style,
            ContentType::SystemMessage => self.system_message = style,
            ContentType::UserInput => self.user_input = style,
            ContentType::AssistantResponse => self.assistant_response = style,
            ContentType::DiffAdd => self.diff_add = style,
            ContentType::DiffRemove => self.diff_remove = style,
            ContentType::Muted => self.muted = style,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_styles_returns_valid_styles() {
        let styles = OutputStyles::default_styles();
        // Each content type should return a non-default style
        let error_style = styles.style_for(ContentType::Error);
        assert_eq!(error_style.fg, Some(Color::Red));
    }

    #[test]
    fn all_content_types_have_styles() {
        let styles = OutputStyles::default_styles();
        let types = [
            ContentType::Code,
            ContentType::Error,
            ContentType::Warning,
            ContentType::Info,
            ContentType::ToolResult,
            ContentType::SystemMessage,
            ContentType::UserInput,
            ContentType::AssistantResponse,
            ContentType::DiffAdd,
            ContentType::DiffRemove,
            ContentType::Muted,
        ];
        for ct in types {
            let _ = styles.style_for(ct);
        }
    }

    #[test]
    fn set_style_overrides() {
        let mut styles = OutputStyles::default_styles();
        let custom = Style::default().fg(Color::Magenta);
        styles.set_style(ContentType::Code, custom);
        assert_eq!(styles.style_for(ContentType::Code).fg, Some(Color::Magenta));
    }

    #[test]
    fn content_type_equality() {
        assert_eq!(ContentType::Code, ContentType::Code);
        assert_ne!(ContentType::Code, ContentType::Error);
    }
}
