//! Spinner component — animated loading indicator with random verb messages.
//!
//! Matches CCB's `Spinner.tsx` + `spinnerVerbs.ts`: displays an animated
//! braille spinner with a randomly-selected verb from a pool of 188 verbs,
//! plus a shimmer highlight effect sliding across the verb text.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

// ── Spinner animation frames ──────────────────────────────────────────

/// Braille-based spinner frames for smooth animation.
const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

// ── CCB SPINNER_VERBS (188 verbs from constants/spinnerVerbs.ts) ──────

/// Spinner verb pool — one is randomly selected each time the spinner starts.
/// Exact copy of CCB's `SPINNER_VERBS` array.
pub const SPINNER_VERBS: &[&str] = &[
    "Accomplishing",
    "Actioning",
    "Actualizing",
    "Architecting",
    "Baking",
    "Beaming",
    "Beboppin'",
    "Befuddling",
    "Billowing",
    "Blanching",
    "Bloviating",
    "Boogieing",
    "Boondoggling",
    "Booping",
    "Bootstrapping",
    "Brewing",
    "Bunning",
    "Burrowing",
    "Calculating",
    "Canoodling",
    "Caramelizing",
    "Cascading",
    "Catapulting",
    "Cerebrating",
    "Channeling",
    "Channelling",
    "Choreographing",
    "Churning",
    "Clauding",
    "Coalescing",
    "Cogitating",
    "Combobulating",
    "Composing",
    "Computing",
    "Concocting",
    "Considering",
    "Contemplating",
    "Cooking",
    "Crafting",
    "Creating",
    "Crunching",
    "Crystallizing",
    "Cultivating",
    "Deciphering",
    "Deliberating",
    "Determining",
    "Dilly-dallying",
    "Discombobulating",
    "Doing",
    "Doodling",
    "Drizzling",
    "Ebbing",
    "Effecting",
    "Elucidating",
    "Embellishing",
    "Enchanting",
    "Envisioning",
    "Evaporating",
    "Fermenting",
    "Fiddle-faddling",
    "Finagling",
    "Flambéing",
    "Flibbertigibbeting",
    "Flowing",
    "Flummoxing",
    "Fluttering",
    "Forging",
    "Forming",
    "Frolicking",
    "Frosting",
    "Gallivanting",
    "Galloping",
    "Garnishing",
    "Generating",
    "Gesticulating",
    "Germinating",
    "Gitifying",
    "Grooving",
    "Gusting",
    "Harmonizing",
    "Hashing",
    "Hatching",
    "Herding",
    "Honking",
    "Hullaballooing",
    "Hyperspacing",
    "Ideating",
    "Imagining",
    "Improvising",
    "Incubating",
    "Inferring",
    "Infusing",
    "Ionizing",
    "Jitterbugging",
    "Julienning",
    "Kneading",
    "Leavening",
    "Levitating",
    "Lollygagging",
    "Manifesting",
    "Marinating",
    "Meandering",
    "Metamorphosing",
    "Misting",
    "Moonwalking",
    "Moseying",
    "Mulling",
    "Mustering",
    "Musing",
    "Nebulizing",
    "Nesting",
    "Newspapering",
    "Noodling",
    "Nucleating",
    "Orbiting",
    "Orchestrating",
    "Osmosing",
    "Perambulating",
    "Percolating",
    "Perusing",
    "Philosophising",
    "Photosynthesizing",
    "Pollinating",
    "Pondering",
    "Pontificating",
    "Pouncing",
    "Precipitating",
    "Prestidigitating",
    "Processing",
    "Proofing",
    "Propagating",
    "Puttering",
    "Puzzling",
    "Quantumizing",
    "Razzle-dazzling",
    "Razzmatazzing",
    "Recombobulating",
    "Reticulating",
    "Roosting",
    "Ruminating",
    "Sautéing",
    "Scampering",
    "Schlepping",
    "Scurrying",
    "Seasoning",
    "Shenaniganing",
    "Shimmying",
    "Simmering",
    "Skedaddling",
    "Sketching",
    "Slithering",
    "Smooshing",
    "Sock-hopping",
    "Spelunking",
    "Spinning",
    "Sprouting",
    "Stewing",
    "Sublimating",
    "Swirling",
    "Swooping",
    "Symbioting",
    "Synthesizing",
    "Tempering",
    "Thinking",
    "Thundering",
    "Tinkering",
    "Tomfoolering",
    "Topsy-turvying",
    "Transfiguring",
    "Transmuting",
    "Twisting",
    "Undulating",
    "Unfurling",
    "Unravelling",
    "Vibing",
    "Waddling",
    "Wandering",
    "Warping",
    "Whatchamacalliting",
    "Whirlpooling",
    "Whirring",
    "Whisking",
    "Wibbling",
    "Working",
    "Wrangling",
    "Zesting",
    "Zigzagging",
];

// ── Shimmer animation ─────────────────────────────────────────────────

/// Compute the shimmer highlight position given tick count and text width.
///
/// Returns the index of the character that should be highlighted.
/// Returns a value outside `[0, width)` when the shimmer is off-screen.
fn shimmer_index(tick: usize, width: usize, speed: usize) -> i32 {
    if width == 0 || speed == 0 {
        return -100;
    }
    let cycle_len = width + 20;
    let pos = (tick / speed) % cycle_len;
    i32::try_from(pos).unwrap_or(0) - 10
}

// ── Random verb selection ─────────────────────────────────────────────

/// Pick a random verb from `SPINNER_VERBS`.
///
/// Uses a simple hash of the current time as seed — not cryptographic,
/// just needs to look varied across spinner starts.
fn random_verb() -> &'static str {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let idx = (seed as usize) % SPINNER_VERBS.len();
    SPINNER_VERBS[idx]
}

// ── Spinner struct ────────────────────────────────────────────────────

/// Loading spinner with a random verb message and shimmer animation.
pub struct Spinner {
    /// Current animation frame index (wraps around).
    frame: usize,
    /// Total tick count for shimmer animation.
    tick: usize,
    /// The randomly-selected verb (e.g. "Cogitating").
    verb: String,
    /// Optional override message (replaces verb when set).
    override_message: Option<String>,
    /// Whether the spinner is actively animating.
    active: bool,
    /// Shimmer color (the highlight sliding across text).
    shimmer_color: Color,
}

impl Spinner {
    /// Create a new spinner (inactive by default).
    #[must_use]
    pub fn new() -> Self {
        Self {
            frame: 0,
            tick: 0,
            verb: random_verb().to_string(),
            override_message: None,
            active: false,
            shimmer_color: Color::White,
        }
    }

    /// Start the spinner, picking a new random verb.
    pub fn start_with_random_verb(&mut self) {
        self.verb = random_verb().to_string();
        self.override_message = None;
        self.active = true;
        self.frame = 0;
        self.tick = 0;
    }

    /// Start the spinner with a specific message (overrides verb).
    pub fn start(&mut self, message: impl Into<String>) {
        self.override_message = Some(message.into());
        self.active = true;
        self.frame = 0;
        self.tick = 0;
    }

    /// Stop the spinner.
    pub fn stop(&mut self) {
        self.active = false;
    }

    /// Advance to the next animation frame. Call on each Tick event.
    pub fn tick(&mut self) {
        if self.active {
            self.frame = (self.frame + 1) % FRAMES.len();
            self.tick += 1;
        }
    }

    /// Whether the spinner is currently active.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.active
    }

    /// Current display message (verb + "…" or override message).
    #[must_use]
    pub fn message(&self) -> String {
        if let Some(ref msg) = self.override_message {
            msg.clone()
        } else {
            format!("{}…", self.verb)
        }
    }

    /// The raw verb (without ellipsis).
    #[must_use]
    pub fn verb(&self) -> &str {
        &self.verb
    }

    /// Update the override message without restarting.
    pub fn set_message(&mut self, message: impl Into<String>) {
        self.override_message = Some(message.into());
    }

    /// Clear the override message (reverts to verb display).
    pub fn clear_override(&mut self) {
        self.override_message = None;
    }

    /// Set the shimmer highlight color.
    pub fn set_shimmer_color(&mut self, color: Color) {
        self.shimmer_color = color;
    }
}

impl Default for Spinner {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for &Spinner {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.active || area.width < 3 || area.height == 0 {
            return;
        }

        let frame_char = FRAMES[self.frame];
        let msg = self.message();
        let msg_chars: Vec<char> = msg.chars().collect();
        let glimmer_pos = shimmer_index(self.tick, msg_chars.len(), 4);

        // Build styled spans for the message with shimmer effect
        let mut spans = vec![
            Span::styled(frame_char, Style::default().fg(Color::Cyan)),
            Span::raw(" "),
        ];

        // Render each character of the message with shimmer
        for (i, ch) in msg_chars.iter().enumerate() {
            let dist = (i32::try_from(i).unwrap_or(0) - glimmer_pos).abs();
            let style = if dist == 0 {
                // Shimmer highlight: bright + bold
                Style::default()
                    .fg(self.shimmer_color)
                    .add_modifier(Modifier::BOLD)
            } else if dist <= 2 {
                // Near shimmer: slightly brighter
                Style::default().fg(Color::Gray)
            } else {
                // Normal: dim
                Style::default().fg(Color::DarkGray)
            };
            spans.push(Span::styled(ch.to_string(), style));
        }

        let line = Line::from(spans);
        let line_area = Rect { height: 1, ..area };
        Widget::render(line, line_area, buf);
    }
}

// ── Settings integration ──────────────────────────────────────────────

/// Spinner verb configuration from settings.json.
#[derive(Debug, Clone)]
pub struct SpinnerVerbConfig {
    /// Custom verbs to add or replace.
    pub verbs: Vec<String>,
    /// Mode: "replace" replaces defaults, "append" adds to defaults.
    pub mode: SpinnerVerbMode,
}

/// How custom verbs interact with the default verb list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpinnerVerbMode {
    /// Replace all default verbs with custom ones.
    Replace,
    /// Append custom verbs to the default list.
    Append,
}

/// Get the effective verb list, considering settings overrides.
pub fn effective_verbs(config: Option<&SpinnerVerbConfig>) -> Vec<&str> {
    let Some(config) = config else {
        return SPINNER_VERBS.to_vec();
    };

    match config.mode {
        SpinnerVerbMode::Replace => {
            if config.verbs.is_empty() {
                SPINNER_VERBS.to_vec()
            } else {
                config.verbs.iter().map(String::as_str).collect()
            }
        }
        SpinnerVerbMode::Append => {
            let mut all: Vec<&str> = SPINNER_VERBS.to_vec();
            all.extend(config.verbs.iter().map(String::as_str));
            all
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spinner_verbs_count() {
        assert_eq!(SPINNER_VERBS.len(), 187);
    }

    #[test]
    fn spinner_verbs_all_non_empty() {
        for verb in SPINNER_VERBS {
            assert!(!verb.is_empty());
        }
    }

    #[test]
    fn spinner_starts_inactive() {
        let spinner = Spinner::new();
        assert!(!spinner.is_active());
    }

    #[test]
    fn spinner_start_with_random_verb() {
        let mut spinner = Spinner::new();
        spinner.start_with_random_verb();
        assert!(spinner.is_active());
        assert!(!spinner.verb().is_empty());
        // Verb should be from SPINNER_VERBS
        assert!(SPINNER_VERBS.contains(&spinner.verb()));
    }

    #[test]
    fn spinner_start_with_message() {
        let mut spinner = Spinner::new();
        spinner.start("Loading...");
        assert!(spinner.is_active());
        assert_eq!(spinner.message(), "Loading...");
    }

    #[test]
    fn spinner_message_includes_ellipsis() {
        let mut spinner = Spinner::new();
        spinner.start_with_random_verb();
        let msg = spinner.message();
        assert!(msg.ends_with('…'), "expected ellipsis, got: {msg}");
    }

    #[test]
    fn spinner_stop() {
        let mut spinner = Spinner::new();
        spinner.start_with_random_verb();
        spinner.stop();
        assert!(!spinner.is_active());
    }

    #[test]
    fn spinner_tick_advances() {
        let mut spinner = Spinner::new();
        spinner.start_with_random_verb();
        assert_eq!(spinner.frame, 0);
        spinner.tick();
        assert_eq!(spinner.frame, 1);
    }

    #[test]
    fn spinner_tick_wraps_around() {
        let mut spinner = Spinner::new();
        spinner.start_with_random_verb();
        for _ in 0..FRAMES.len() {
            spinner.tick();
        }
        assert_eq!(spinner.frame, 0);
    }

    #[test]
    fn spinner_tick_inactive_noop() {
        let mut spinner = Spinner::new();
        spinner.tick();
        assert_eq!(spinner.frame, 0);
        assert_eq!(spinner.tick, 0);
    }

    #[test]
    fn spinner_renders_when_active() {
        let mut spinner = Spinner::new();
        spinner.start("Testing");
        let area = Rect::new(0, 0, 40, 1);
        let mut buf = Buffer::empty(area);
        Widget::render(&spinner, area, &mut buf);
        let content: String = (0..area.width)
            .map(|x| buf.cell((x, 0)).unwrap().symbol().to_string())
            .collect();
        assert!(content.contains("Testing"));
    }

    #[test]
    fn spinner_does_not_render_inactive() {
        let spinner = Spinner::new();
        let area = Rect::new(0, 0, 40, 1);
        let mut buf = Buffer::empty(area);
        Widget::render(&spinner, area, &mut buf);
        let content: String = (0..area.width)
            .map(|x| buf.cell((x, 0)).unwrap().symbol().to_string())
            .collect();
        assert_eq!(content.trim(), "");
    }

    #[test]
    fn shimmer_index_cycles() {
        let idx1 = shimmer_index(0, 10, 4);
        let idx2 = shimmer_index(4, 10, 4);
        assert_ne!(idx1, idx2);
    }

    #[test]
    fn effective_verbs_default() {
        let verbs = effective_verbs(None);
        assert_eq!(verbs.len(), 187);
    }

    #[test]
    fn effective_verbs_replace() {
        let config = SpinnerVerbConfig {
            verbs: vec!["Custom1".into(), "Custom2".into()],
            mode: SpinnerVerbMode::Replace,
        };
        let verbs = effective_verbs(Some(&config));
        assert_eq!(verbs.len(), 2);
        assert_eq!(verbs[0], "Custom1");
    }

    #[test]
    fn effective_verbs_append() {
        let config = SpinnerVerbConfig {
            verbs: vec!["Extra".into()],
            mode: SpinnerVerbMode::Append,
        };
        let verbs = effective_verbs(Some(&config));
        assert_eq!(verbs.len(), 188);
    }

    #[test]
    fn effective_verbs_replace_empty_falls_back() {
        let config = SpinnerVerbConfig {
            verbs: vec![],
            mode: SpinnerVerbMode::Replace,
        };
        let verbs = effective_verbs(Some(&config));
        assert_eq!(verbs.len(), 187);
    }

    #[test]
    fn spinner_default() {
        let spinner = Spinner::default();
        assert!(!spinner.is_active());
    }

    #[test]
    fn spinner_clear_override() {
        let mut spinner = Spinner::new();
        spinner.start("Override");
        assert_eq!(spinner.message(), "Override");
        spinner.clear_override();
        // Should fall back to verb + ellipsis
        assert!(spinner.message().ends_with('…'));
    }
}
