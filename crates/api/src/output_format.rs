//! Streaming output formatting and Markdown rendering pipeline.
//!
//! `OutputFormatter` provides a unified trait for transforming streamed text
//! tokens into formatted output. `MarkdownStreamProcessor` handles incremental
//! Markdown rendering (code blocks, lists, headings, emphasis). `CodeBlockDetector`
//! tracks fenced code block boundaries and extracts language annotations.

use std::fmt;

// ---------------------------------------------------------------------------
// OutputFormatter trait
// ---------------------------------------------------------------------------

/// Unified output formatting interface.
///
/// Implementations receive text tokens one at a time via `push_token` and
/// produce formatted output fragments. `flush` drains any buffered state.
pub trait OutputFormatter: Send + Sync {
    /// Formatter name (for diagnostics).
    fn name(&self) -> &str;

    /// Push a single text token, returning zero or more formatted fragments.
    fn push_token(&mut self, token: &str) -> Vec<OutputFragment>;

    /// Flush any remaining buffered content.
    fn flush(&mut self) -> Vec<OutputFragment>;

    /// Reset internal state for a new message.
    fn reset(&mut self);
}

/// A fragment of formatted output produced by an `OutputFormatter`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputFragment {
    pub text: String,
    pub style: FragmentStyle,
}

/// Visual style hint for an output fragment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FragmentStyle {
    /// Plain text, no special formatting.
    Plain,
    /// Inside a fenced code block.
    Code,
    /// Code block delimiter (opening or closing fence).
    CodeFence,
    /// Language annotation on a code fence.
    Language,
    /// Heading (level 1–6).
    Heading(u8),
    /// List item bullet / number.
    ListMarker,
    /// Bold text.
    Bold,
    /// Italic text.
    Italic,
    /// Inline code span.
    InlineCode,
}

impl fmt::Display for FragmentStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plain => write!(f, "plain"),
            Self::Code => write!(f, "code"),
            Self::CodeFence => write!(f, "code_fence"),
            Self::Language => write!(f, "language"),
            Self::Heading(lvl) => write!(f, "h{lvl}"),
            Self::ListMarker => write!(f, "list_marker"),
            Self::Bold => write!(f, "bold"),
            Self::Italic => write!(f, "italic"),
            Self::InlineCode => write!(f, "inline_code"),
        }
    }
}

impl OutputFragment {
    #[must_use]
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: FragmentStyle::Plain,
        }
    }

    #[must_use]
    pub fn styled(text: impl Into<String>, style: FragmentStyle) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }
}

// ---------------------------------------------------------------------------
// CodeBlockDetector
// ---------------------------------------------------------------------------

/// Tracks fenced code block boundaries in a stream of text tokens.
///
/// Detects opening/closing triple-backtick fences and extracts the optional
/// language annotation from the opening fence.
#[derive(Debug, Clone)]
pub struct CodeBlockDetector {
    /// Whether we are currently inside a code block.
    in_block: bool,
    /// Buffer for accumulating a potential fence line.
    line_buffer: String,
    /// Language of the current code block (if any).
    language: Option<String>,
    /// Number of backticks in the opening fence (to match closing).
    fence_width: usize,
}

impl Default for CodeBlockDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeBlockDetector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            in_block: false,
            line_buffer: String::new(),
            language: None,
            fence_width: 0,
        }
    }

    /// Whether we are currently inside a fenced code block.
    #[must_use]
    pub fn is_in_block(&self) -> bool {
        self.in_block
    }

    /// Language annotation of the current code block, if any.
    #[must_use]
    pub fn language(&self) -> Option<&str> {
        self.language.as_deref()
    }

    /// Feed a token and return any detected events.
    pub fn feed(&mut self, token: &str) -> Vec<CodeBlockEvent> {
        let mut events = Vec::new();
        self.line_buffer.push_str(token);

        // Process complete lines in the buffer.
        while let Some(newline_pos) = self.line_buffer.find('\n') {
            let line: String = self.line_buffer[..newline_pos].to_string();
            self.line_buffer = self.line_buffer[newline_pos + 1..].to_string();
            events.extend(self.process_line(&line));
        }

        events
    }

    /// Flush remaining buffer (end of stream).
    pub fn finish(&mut self) -> Vec<CodeBlockEvent> {
        if self.line_buffer.is_empty() {
            return Vec::new();
        }
        let line = std::mem::take(&mut self.line_buffer);
        self.process_line(&line)
    }

    fn process_line(&mut self, line: &str) -> Vec<CodeBlockEvent> {
        let trimmed = line.trim();
        let backtick_count = trimmed.chars().take_while(|&c| c == '`').count();

        if self.in_block {
            // Look for closing fence: at least fence_width backticks, nothing else.
            if backtick_count >= self.fence_width && trimmed.chars().all(|c| c == '`') {
                self.in_block = false;
                let lang = self.language.take();
                self.fence_width = 0;
                return vec![CodeBlockEvent::Close { language: lang }];
            }
        } else {
            // Look for opening fence: at least 3 backticks.
            if backtick_count >= 3 {
                self.in_block = true;
                self.fence_width = backtick_count;
                let lang_part = trimmed[backtick_count..].trim();
                self.language = if lang_part.is_empty() {
                    None
                } else {
                    Some(lang_part.to_string())
                };
                return vec![CodeBlockEvent::Open {
                    language: self.language.clone(),
                }];
            }
        }

        Vec::new()
    }

    /// Reset state for a new message.
    pub fn reset(&mut self) {
        self.in_block = false;
        self.line_buffer.clear();
        self.language = None;
        self.fence_width = 0;
    }
}

/// Events emitted by `CodeBlockDetector`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeBlockEvent {
    /// A code block opened, optionally with a language annotation.
    Open { language: Option<String> },
    /// A code block closed.
    Close { language: Option<String> },
}

// ---------------------------------------------------------------------------
// MarkdownStreamProcessor
// ---------------------------------------------------------------------------

/// Streaming Markdown renderer that processes tokens incrementally.
///
/// Handles: fenced code blocks, headings, unordered/ordered lists,
/// bold, italic, and inline code. Produces styled `OutputFragment`s.
#[derive(Debug)]
pub struct MarkdownStreamProcessor {
    code_detector: CodeBlockDetector,
    /// Buffer for the current line (to detect headings, list items, etc.).
    line_buffer: String,
    /// Whether we are at the start of a new line.
    at_line_start: bool,
}

impl Default for MarkdownStreamProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl MarkdownStreamProcessor {
    #[must_use]
    pub fn new() -> Self {
        Self {
            code_detector: CodeBlockDetector::new(),
            line_buffer: String::new(),
            at_line_start: true,
        }
    }

    /// Process buffered line content into styled fragments.
    fn emit_line(&self, line: &str) -> Vec<OutputFragment> {
        if self.code_detector.is_in_block() {
            // Inside code block — emit as Code style, no inline parsing.
            return vec![OutputFragment::styled(
                line.to_string(),
                FragmentStyle::Code,
            )];
        }

        let trimmed = line.trim_start();

        // Check for heading: # ... ######
        if let Some(heading) = Self::parse_heading(trimmed) {
            return heading;
        }

        // Check for unordered list item: - or *
        if let Some(list_item) = Self::parse_unordered_list(trimmed) {
            return list_item;
        }

        // Check for ordered list item: 1. 2. etc.
        if let Some(list_item) = Self::parse_ordered_list(trimmed) {
            return list_item;
        }

        // Inline formatting pass.
        Self::parse_inline(line)
    }

    fn parse_heading(line: &str) -> Option<Vec<OutputFragment>> {
        let hash_count = line.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&hash_count) {
            let rest = &line[hash_count..];
            if rest.is_empty() || rest.starts_with(' ') {
                let text = rest.trim_start();
                #[allow(clippy::cast_possible_truncation)] // bounded to 1..=6 above
                let level = hash_count as u8;
                let mut frags = vec![OutputFragment::styled(
                    "#".repeat(hash_count),
                    FragmentStyle::Heading(level),
                )];
                if !text.is_empty() {
                    frags.push(OutputFragment::styled(
                        format!(" {text}"),
                        FragmentStyle::Heading(level),
                    ));
                }
                return Some(frags);
            }
        }
        None
    }

    fn parse_unordered_list(line: &str) -> Option<Vec<OutputFragment>> {
        if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            let marker = &line[..2];
            let mut frags = vec![OutputFragment::styled(marker, FragmentStyle::ListMarker)];
            frags.extend(Self::parse_inline(rest));
            return Some(frags);
        }
        None
    }

    fn parse_ordered_list(line: &str) -> Option<Vec<OutputFragment>> {
        let digit_count = line.chars().take_while(char::is_ascii_digit).count();
        if digit_count > 0 && line[digit_count..].starts_with(". ") {
            let marker = &line[..digit_count + 2];
            let rest = &line[digit_count + 2..];
            let mut frags = vec![OutputFragment::styled(marker, FragmentStyle::ListMarker)];
            frags.extend(Self::parse_inline(rest));
            return Some(frags);
        }
        None
    }

    /// Parse inline formatting: **bold**, *italic*, `code`.
    fn parse_inline(text: &str) -> Vec<OutputFragment> {
        let mut frags = Vec::new();
        let mut chars = text.char_indices().peekable();
        let mut plain_start = 0;

        while let Some(&(i, ch)) = chars.peek() {
            match ch {
                '`' => {
                    // Inline code.
                    if i > plain_start {
                        frags.push(OutputFragment::plain(&text[plain_start..i]));
                    }
                    chars.next();
                    let code_start = i + 1;
                    let mut code_end = None;
                    while let Some(&(j, c2)) = chars.peek() {
                        chars.next();
                        if c2 == '`' {
                            code_end = Some(j);
                            break;
                        }
                    }
                    if let Some(end) = code_end {
                        frags.push(OutputFragment::styled(
                            &text[code_start..end],
                            FragmentStyle::InlineCode,
                        ));
                        plain_start = end + 1;
                    } else {
                        // No closing backtick — treat as plain.
                        frags.push(OutputFragment::plain(&text[i..]));
                        return frags;
                    }
                }
                '*' => {
                    // Bold (**) or italic (*).
                    if i > plain_start {
                        frags.push(OutputFragment::plain(&text[plain_start..i]));
                    }
                    chars.next();

                    // Check for **bold**
                    if chars.peek().is_some_and(|&(_, c)| c == '*') {
                        chars.next();
                        let bold_start = i + 2;
                        let mut bold_end = None;
                        while let Some(&(j, c2)) = chars.peek() {
                            chars.next();
                            if c2 == '*' && chars.peek().is_some_and(|&(_, c3)| c3 == '*') {
                                chars.next();
                                bold_end = Some(j);
                                break;
                            }
                        }
                        if let Some(end) = bold_end {
                            frags.push(OutputFragment::styled(
                                &text[bold_start..end],
                                FragmentStyle::Bold,
                            ));
                            plain_start = end + 2;
                        } else {
                            frags.push(OutputFragment::plain(&text[i..]));
                            return frags;
                        }
                    } else {
                        // *italic*
                        let italic_start = i + 1;
                        let mut italic_end = None;
                        while let Some(&(j, c2)) = chars.peek() {
                            chars.next();
                            if c2 == '*' {
                                italic_end = Some(j);
                                break;
                            }
                        }
                        if let Some(end) = italic_end {
                            frags.push(OutputFragment::styled(
                                &text[italic_start..end],
                                FragmentStyle::Italic,
                            ));
                            plain_start = end + 1;
                        } else {
                            frags.push(OutputFragment::plain(&text[i..]));
                            return frags;
                        }
                    }
                }
                _ => {
                    chars.next();
                }
            }
        }

        if plain_start < text.len() {
            frags.push(OutputFragment::plain(&text[plain_start..]));
        }

        frags
    }
}

impl OutputFormatter for MarkdownStreamProcessor {
    fn name(&self) -> &'static str {
        "markdown"
    }

    fn push_token(&mut self, token: &str) -> Vec<OutputFragment> {
        let mut fragments = Vec::new();

        for ch in token.chars() {
            if ch == '\n' {
                // Process the completed line.
                let events = self.code_detector.feed(&format!("{}\n", self.line_buffer));

                if events.is_empty() {
                    let line = std::mem::take(&mut self.line_buffer);
                    fragments.extend(self.emit_line(&line));
                } else {
                    for event in &events {
                        match event {
                            CodeBlockEvent::Open { language } => {
                                let mut fence_text = "```".to_string();
                                if let Some(lang) = language {
                                    fence_text.push_str(lang);
                                }
                                fragments.push(OutputFragment::styled(
                                    fence_text,
                                    FragmentStyle::CodeFence,
                                ));
                                if let Some(lang) = language {
                                    fragments.push(OutputFragment::styled(
                                        lang.clone(),
                                        FragmentStyle::Language,
                                    ));
                                }
                            }
                            CodeBlockEvent::Close { .. } => {
                                fragments
                                    .push(OutputFragment::styled("```", FragmentStyle::CodeFence));
                            }
                        }
                    }
                }

                fragments.push(OutputFragment::plain("\n"));
                self.line_buffer.clear();
                self.at_line_start = true;
            } else {
                self.line_buffer.push(ch);
                self.at_line_start = false;
            }
        }

        fragments
    }

    fn flush(&mut self) -> Vec<OutputFragment> {
        let mut fragments = Vec::new();

        // Flush code block detector.
        let events = self.code_detector.finish();
        if !events.is_empty() {
            for event in &events {
                match event {
                    CodeBlockEvent::Open { language } => {
                        let mut fence_text = "```".to_string();
                        if let Some(lang) = language {
                            fence_text.push_str(lang);
                        }
                        fragments
                            .push(OutputFragment::styled(fence_text, FragmentStyle::CodeFence));
                    }
                    CodeBlockEvent::Close { .. } => {
                        fragments.push(OutputFragment::styled("```", FragmentStyle::CodeFence));
                    }
                }
            }
        }

        // Flush remaining line buffer.
        if !self.line_buffer.is_empty() {
            let line = std::mem::take(&mut self.line_buffer);
            fragments.extend(self.emit_line(&line));
        }

        fragments
    }

    fn reset(&mut self) {
        self.code_detector.reset();
        self.line_buffer.clear();
        self.at_line_start = true;
    }
}

// ---------------------------------------------------------------------------
// PlainTextFormatter — passthrough (no formatting)
// ---------------------------------------------------------------------------

/// Passthrough formatter that emits all tokens as plain text.
#[derive(Debug, Default)]
pub struct PlainTextFormatter;

impl OutputFormatter for PlainTextFormatter {
    fn name(&self) -> &'static str {
        "plain"
    }

    fn push_token(&mut self, token: &str) -> Vec<OutputFragment> {
        if token.is_empty() {
            Vec::new()
        } else {
            vec![OutputFragment::plain(token)]
        }
    }

    fn flush(&mut self) -> Vec<OutputFragment> {
        Vec::new()
    }

    fn reset(&mut self) {}
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- OutputFragment --

    #[test]
    fn fragment_plain_constructor() {
        let f = OutputFragment::plain("hello");
        assert_eq!(f.text, "hello");
        assert_eq!(f.style, FragmentStyle::Plain);
    }

    #[test]
    fn fragment_styled_constructor() {
        let f = OutputFragment::styled("code", FragmentStyle::Code);
        assert_eq!(f.text, "code");
        assert_eq!(f.style, FragmentStyle::Code);
    }

    #[test]
    fn fragment_style_display() {
        assert_eq!(FragmentStyle::Plain.to_string(), "plain");
        assert_eq!(FragmentStyle::Heading(2).to_string(), "h2");
        assert_eq!(FragmentStyle::InlineCode.to_string(), "inline_code");
    }

    // -- CodeBlockDetector --

    #[test]
    fn code_block_detector_open_close() {
        let mut det = CodeBlockDetector::new();
        assert!(!det.is_in_block());

        let events = det.feed("```rust\n");
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            CodeBlockEvent::Open {
                language: Some("rust".to_string())
            }
        );
        assert!(det.is_in_block());
        assert_eq!(det.language(), Some("rust"));

        let events = det.feed("let x = 1;\n");
        assert!(events.is_empty());

        let events = det.feed("```\n");
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            CodeBlockEvent::Close {
                language: Some("rust".to_string())
            }
        );
        assert!(!det.is_in_block());
    }

    #[test]
    fn code_block_no_language() {
        let mut det = CodeBlockDetector::new();
        let events = det.feed("```\n");
        assert_eq!(events[0], CodeBlockEvent::Open { language: None });
        assert!(det.is_in_block());
        assert_eq!(det.language(), None);
    }

    #[test]
    fn code_block_four_backtick_fence() {
        let mut det = CodeBlockDetector::new();
        let events = det.feed("````python\n");
        assert_eq!(
            events[0],
            CodeBlockEvent::Open {
                language: Some("python".to_string())
            }
        );
        // Must close with at least 4 backticks.
        let events = det.feed("```\n");
        assert!(
            events.is_empty(),
            "3 backticks should not close a 4-backtick fence"
        );

        let events = det.feed("````\n");
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], CodeBlockEvent::Close { .. }));
    }

    #[test]
    fn code_block_reset() {
        let mut det = CodeBlockDetector::new();
        det.feed("```\n");
        assert!(det.is_in_block());
        det.reset();
        assert!(!det.is_in_block());
        assert_eq!(det.language(), None);
    }

    #[test]
    fn code_block_finish_flushes_buffer() {
        let mut det = CodeBlockDetector::new();
        // Feed without newline.
        det.feed("```rust");
        assert!(!det.is_in_block()); // Not yet — no newline processed.
        let events = det.finish();
        assert_eq!(events.len(), 1);
        assert!(det.is_in_block());
    }

    // -- MarkdownStreamProcessor --

    #[test]
    fn markdown_plain_text() {
        let mut proc = MarkdownStreamProcessor::new();
        let frags = proc.push_token("hello world\n");
        let texts: Vec<_> = frags.iter().map(|f| (f.text.as_str(), f.style)).collect();
        assert_eq!(texts[0], ("hello world", FragmentStyle::Plain));
    }

    #[test]
    fn markdown_heading() {
        let mut proc = MarkdownStreamProcessor::new();
        let frags = proc.push_token("## Title\n");
        let styles: Vec<_> = frags.iter().map(|f| f.style).collect();
        assert!(styles.contains(&FragmentStyle::Heading(2)));
    }

    #[test]
    fn markdown_heading_levels() {
        for level in 1..=6 {
            let mut proc = MarkdownStreamProcessor::new();
            let hashes = "#".repeat(level);
            let frags = proc.push_token(&format!("{hashes} Level {level}\n"));
            let has_heading = frags
                .iter()
                .any(|f| f.style == FragmentStyle::Heading(level as u8));
            assert!(has_heading, "Level {level} heading not detected");
        }
    }

    #[test]
    fn markdown_unordered_list() {
        let mut proc = MarkdownStreamProcessor::new();
        let frags = proc.push_token("- item one\n");
        assert!(frags.iter().any(|f| f.style == FragmentStyle::ListMarker));
        assert!(frags.iter().any(|f| f.text.contains("item one")));
    }

    #[test]
    fn markdown_ordered_list() {
        let mut proc = MarkdownStreamProcessor::new();
        let frags = proc.push_token("1. first\n");
        assert!(frags.iter().any(|f| f.style == FragmentStyle::ListMarker));
        assert!(frags.iter().any(|f| f.text.contains("first")));
    }

    #[test]
    fn markdown_bold() {
        let mut proc = MarkdownStreamProcessor::new();
        let frags = proc.push_token("this is **bold** text\n");
        assert!(
            frags
                .iter()
                .any(|f| f.style == FragmentStyle::Bold && f.text == "bold")
        );
    }

    #[test]
    fn markdown_italic() {
        let mut proc = MarkdownStreamProcessor::new();
        let frags = proc.push_token("this is *italic* text\n");
        assert!(
            frags
                .iter()
                .any(|f| f.style == FragmentStyle::Italic && f.text == "italic")
        );
    }

    #[test]
    fn markdown_inline_code() {
        let mut proc = MarkdownStreamProcessor::new();
        let frags = proc.push_token("run `cargo test` now\n");
        assert!(
            frags
                .iter()
                .any(|f| f.style == FragmentStyle::InlineCode && f.text == "cargo test")
        );
    }

    #[test]
    fn markdown_code_block_content_not_inline_parsed() {
        let mut proc = MarkdownStreamProcessor::new();

        // Open code block.
        let frags = proc.push_token("```rust\n");
        assert!(frags.iter().any(|f| f.style == FragmentStyle::CodeFence));

        // Code inside — should be Code style, not parsed for **bold**.
        let frags = proc.push_token("let **x** = 1;\n");
        assert!(frags.iter().any(|f| f.style == FragmentStyle::Code));
        assert!(!frags.iter().any(|f| f.style == FragmentStyle::Bold));

        // Close code block.
        let frags = proc.push_token("```\n");
        assert!(frags.iter().any(|f| f.style == FragmentStyle::CodeFence));
    }

    #[test]
    fn markdown_incremental_tokens() {
        let mut proc = MarkdownStreamProcessor::new();

        // Feed tokens one character at a time.
        let text = "## Hi\n";
        let mut all_frags = Vec::new();
        for ch in text.chars() {
            all_frags.extend(proc.push_token(&ch.to_string()));
        }
        assert!(
            all_frags
                .iter()
                .any(|f| f.style == FragmentStyle::Heading(2))
        );
    }

    #[test]
    fn markdown_flush_remaining() {
        let mut proc = MarkdownStreamProcessor::new();
        // Push without newline.
        let frags = proc.push_token("partial");
        assert!(frags.is_empty()); // Buffered, no newline yet.

        let frags = proc.flush();
        assert!(frags.iter().any(|f| f.text == "partial"));
    }

    #[test]
    fn markdown_reset() {
        let mut proc = MarkdownStreamProcessor::new();
        proc.push_token("```\n");
        proc.reset();
        // After reset, should not be in code block.
        let frags = proc.push_token("normal text\n");
        assert!(!frags.iter().any(|f| f.style == FragmentStyle::Code));
    }

    #[test]
    fn markdown_multiline_code_block() {
        let mut proc = MarkdownStreamProcessor::new();

        let input = "```python\nprint('hello')\nprint('world')\n```\n";
        let frags = proc.push_token(input);

        let code_frags: Vec<_> = frags
            .iter()
            .filter(|f| f.style == FragmentStyle::Code)
            .collect();
        assert_eq!(code_frags.len(), 2); // Two lines of code.
        assert_eq!(code_frags[0].text, "print('hello')");
        assert_eq!(code_frags[1].text, "print('world')");
    }

    // -- PlainTextFormatter --

    #[test]
    fn plain_text_passthrough() {
        let mut fmt = PlainTextFormatter;
        let frags = fmt.push_token("**bold** `code`");
        assert_eq!(frags.len(), 1);
        assert_eq!(frags[0].style, FragmentStyle::Plain);
        assert_eq!(frags[0].text, "**bold** `code`");
    }

    #[test]
    fn plain_text_empty_token() {
        let mut fmt = PlainTextFormatter;
        let frags = fmt.push_token("");
        assert!(frags.is_empty());
    }

    #[test]
    fn plain_text_name() {
        let fmt = PlainTextFormatter;
        assert_eq!(fmt.name(), "plain");
    }

    // -- Trait object usage --

    #[test]
    fn formatter_as_trait_object() {
        let mut formatters: Vec<Box<dyn OutputFormatter>> = vec![
            Box::new(MarkdownStreamProcessor::new()),
            Box::new(PlainTextFormatter),
        ];

        for f in &mut formatters {
            let frags = f.push_token("test\n");
            assert!(!frags.is_empty());
            f.reset();
        }
    }
}
