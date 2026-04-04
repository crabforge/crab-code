use ratatui::style::{Color, Modifier, Style};

/// Color theme for the TUI.
///
/// Defines all semantic colors used across the UI. Components reference
/// theme fields instead of hard-coding colors, making it easy to switch
/// between dark and light themes.
#[derive(Debug, Clone)]
pub struct Theme {
    // ─── General ───
    /// Default foreground.
    pub fg: Color,
    /// Default background.
    pub bg: Color,
    /// Muted/secondary text.
    pub muted: Color,

    // ─── Markdown ───
    /// Heading text color.
    pub heading: Color,
    /// Bold style modifier (combined with current fg).
    pub bold: Modifier,
    /// Italic style modifier.
    pub italic: Modifier,
    /// Inline code foreground.
    pub inline_code_fg: Color,
    /// Inline code background.
    pub inline_code_bg: Color,
    /// Link text color.
    pub link: Color,
    /// List bullet/number color.
    pub list_marker: Color,
    /// Block quote bar color.
    pub blockquote: Color,

    // ─── Diff ───
    /// Added line foreground.
    pub diff_add_fg: Color,
    /// Added line background.
    pub diff_add_bg: Color,
    /// Removed line foreground.
    pub diff_remove_fg: Color,
    /// Removed line background.
    pub diff_remove_bg: Color,
    /// Diff hunk header color.
    pub diff_hunk: Color,

    // ─── Syntax (fallback for non-syntect rendering) ───
    /// Keyword color.
    pub syntax_keyword: Color,
    /// String literal color.
    pub syntax_string: Color,
    /// Comment color.
    pub syntax_comment: Color,
    /// Function name color.
    pub syntax_function: Color,
    /// Type/class name color.
    pub syntax_type: Color,
    /// Number literal color.
    pub syntax_number: Color,

    // ─── UI chrome ───
    /// Status bar / border color.
    pub border: Color,
    /// Error text color.
    pub error: Color,
    /// Warning text color.
    pub warning: Color,
    /// Success text color.
    pub success: Color,
}

impl Theme {
    /// Default dark theme (terminal-friendly 256-color palette).
    #[must_use]
    pub fn dark() -> Self {
        Self {
            fg: Color::White,
            bg: Color::Reset,
            muted: Color::DarkGray,

            heading: Color::Cyan,
            bold: Modifier::BOLD,
            italic: Modifier::ITALIC,
            inline_code_fg: Color::Yellow,
            inline_code_bg: Color::Reset,
            link: Color::Blue,
            list_marker: Color::DarkGray,
            blockquote: Color::DarkGray,

            diff_add_fg: Color::Green,
            diff_add_bg: Color::Reset,
            diff_remove_fg: Color::Red,
            diff_remove_bg: Color::Reset,
            diff_hunk: Color::Cyan,

            syntax_keyword: Color::Magenta,
            syntax_string: Color::Green,
            syntax_comment: Color::DarkGray,
            syntax_function: Color::Yellow,
            syntax_type: Color::Cyan,
            syntax_number: Color::LightRed,

            border: Color::DarkGray,
            error: Color::Red,
            warning: Color::Yellow,
            success: Color::Green,
        }
    }

    /// Light theme for terminals with light backgrounds.
    #[must_use]
    pub fn light() -> Self {
        Self {
            fg: Color::Black,
            bg: Color::Reset,
            muted: Color::Gray,

            heading: Color::DarkGray,
            bold: Modifier::BOLD,
            italic: Modifier::ITALIC,
            inline_code_fg: Color::Rgb(139, 0, 0),
            inline_code_bg: Color::Rgb(240, 240, 240),
            link: Color::Blue,
            list_marker: Color::Gray,
            blockquote: Color::Gray,

            diff_add_fg: Color::Rgb(0, 100, 0),
            diff_add_bg: Color::Rgb(230, 255, 230),
            diff_remove_fg: Color::Rgb(139, 0, 0),
            diff_remove_bg: Color::Rgb(255, 230, 230),
            diff_hunk: Color::Blue,

            syntax_keyword: Color::Rgb(128, 0, 128),
            syntax_string: Color::Rgb(0, 128, 0),
            syntax_comment: Color::Gray,
            syntax_function: Color::Rgb(0, 0, 139),
            syntax_type: Color::Rgb(0, 128, 128),
            syntax_number: Color::Rgb(255, 69, 0),

            border: Color::Gray,
            error: Color::Red,
            warning: Color::Rgb(204, 120, 0),
            success: Color::Rgb(0, 128, 0),
        }
    }

    /// Helper: create a ratatui `Style` from foreground color.
    #[must_use]
    pub fn style_fg(&self, fg: Color) -> Style {
        Style::default().fg(fg)
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_theme_defaults() {
        let theme = Theme::dark();
        assert_eq!(theme.fg, Color::White);
        assert_eq!(theme.diff_add_fg, Color::Green);
        assert_eq!(theme.diff_remove_fg, Color::Red);
    }

    #[test]
    fn light_theme_differs_from_dark() {
        let dark = Theme::dark();
        let light = Theme::light();
        assert_ne!(dark.fg, light.fg);
    }

    #[test]
    fn default_is_dark() {
        let def = Theme::default();
        assert_eq!(def.fg, Color::White);
    }

    #[test]
    fn style_fg_helper() {
        let theme = Theme::dark();
        let style = theme.style_fg(Color::Red);
        assert_eq!(style.fg, Some(Color::Red));
    }
}
