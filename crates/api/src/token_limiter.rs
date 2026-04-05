//! Dynamic token limiting and rate limit tracking.
//!
//! `TokenLimiter` manages per-request token budgets that adapt based on
//! conversation length and tool usage. `RateLimitTracker` parses API
//! response headers to track tokens-per-minute and requests-per-minute.

use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// TokenBudget (output of adaptive_limit)
// ---------------------------------------------------------------------------

/// Computed token budget for a single request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenBudget {
    /// Maximum input tokens allowed.
    pub max_input: u32,
    /// Maximum output tokens to request.
    pub max_output: u32,
    /// Tokens reserved for tool definitions and results.
    pub tool_reserved: u32,
    /// Effective tokens available for conversation history.
    pub history_available: u32,
}

impl TokenBudget {
    /// Total input budget (history + tool reserved).
    #[must_use]
    pub fn total_input(&self) -> u32 {
        self.history_available + self.tool_reserved
    }
}

// ---------------------------------------------------------------------------
// TokenLimiter
// ---------------------------------------------------------------------------

/// Manages token budgets that adapt to conversation state.
#[derive(Debug, Clone)]
pub struct TokenLimiter {
    /// Hard cap on input tokens.
    pub max_input_tokens: u32,
    /// Hard cap on output tokens.
    pub max_output_tokens: u32,
    /// Base tokens reserved per tool (multiplied by tool count).
    pub tokens_per_tool: u32,
    /// Minimum output tokens to always request.
    min_output: u32,
    /// Context window of the model.
    context_window: u32,
}

impl TokenLimiter {
    /// Create a limiter for the given context window.
    #[must_use]
    pub fn new(context_window: u32, max_output_tokens: u32) -> Self {
        Self {
            max_input_tokens: context_window.saturating_sub(max_output_tokens),
            max_output_tokens,
            tokens_per_tool: 500,
            min_output: 256,
            context_window,
        }
    }

    /// Set tokens reserved per tool definition.
    #[must_use]
    pub fn with_tokens_per_tool(mut self, tokens: u32) -> Self {
        self.tokens_per_tool = tokens;
        self
    }

    /// Set minimum output tokens.
    #[must_use]
    pub fn with_min_output(mut self, min: u32) -> Self {
        self.min_output = min;
        self
    }

    /// Compute an adaptive token budget based on conversation state.
    ///
    /// As conversations grow longer, the output budget shrinks to leave room
    /// for history. Tool reservations scale with the number of active tools.
    #[must_use]
    pub fn adaptive_limit(&self, conversation_tokens: u32, tool_count: u32) -> TokenBudget {
        let tool_reserved = tool_count * self.tokens_per_tool;
        let used = conversation_tokens + tool_reserved;

        // Available for output: context_window - used, capped at max_output.
        let output_room = self.context_window.saturating_sub(used);
        let max_output = output_room.min(self.max_output_tokens).max(self.min_output);

        // Available for history: context_window - output - tools.
        let history_available = self
            .context_window
            .saturating_sub(max_output + tool_reserved);

        TokenBudget {
            max_input: self.max_input_tokens,
            max_output,
            tool_reserved,
            history_available,
        }
    }
}

// ---------------------------------------------------------------------------
// RateLimitInfo — parsed from response headers
// ---------------------------------------------------------------------------

/// Parsed rate limit information from API response headers.
#[derive(Debug, Clone)]
pub struct RateLimitInfo {
    /// Requests remaining in the current window.
    pub requests_remaining: Option<u32>,
    /// Tokens remaining in the current window.
    pub tokens_remaining: Option<u32>,
    /// Requests limit per minute.
    pub requests_limit: Option<u32>,
    /// Tokens limit per minute.
    pub tokens_limit: Option<u32>,
    /// When the rate limit resets.
    pub reset_at: Option<Instant>,
}

impl RateLimitInfo {
    /// Parse rate limit info from a header map.
    ///
    /// Looks for standard `x-ratelimit-*` headers used by Anthropic and `OpenAI`.
    #[must_use]
    pub fn from_headers(headers: &[(String, String)]) -> Self {
        let mut info = Self {
            requests_remaining: None,
            tokens_remaining: None,
            requests_limit: None,
            tokens_limit: None,
            reset_at: None,
        };

        for (key, value) in headers {
            let key_lower = key.to_lowercase();
            match key_lower.as_str() {
                "x-ratelimit-remaining-requests" => {
                    info.requests_remaining = value.parse().ok();
                }
                "x-ratelimit-remaining-tokens" => {
                    info.tokens_remaining = value.parse().ok();
                }
                "x-ratelimit-limit-requests" => {
                    info.requests_limit = value.parse().ok();
                }
                "x-ratelimit-limit-tokens" => {
                    info.tokens_limit = value.parse().ok();
                }
                "x-ratelimit-reset-requests" | "x-ratelimit-reset-tokens" => {
                    // Parse duration strings like "1s", "100ms", "1m0s".
                    if let Some(dur) = parse_reset_duration(value) {
                        info.reset_at = Some(Instant::now() + dur);
                    }
                }
                _ => {}
            }
        }

        info
    }

    /// Whether we are near the rate limit (less than 10% remaining).
    #[must_use]
    pub fn is_near_limit(&self) -> bool {
        if let (Some(remaining), Some(limit)) = (self.requests_remaining, self.requests_limit)
            && limit > 0 && remaining * 10 < limit
        {
            return true;
        }
        if let (Some(remaining), Some(limit)) = (self.tokens_remaining, self.tokens_limit)
            && limit > 0 && remaining * 10 < limit
        {
            return true;
        }
        false
    }
}

/// Parse a duration string like "1s", "500ms", "1m30s" into a `Duration`.
fn parse_reset_duration(s: &str) -> Option<Duration> {
    let s = s.trim();

    // Try milliseconds: "500ms"
    if let Some(ms_str) = s.strip_suffix("ms") {
        return ms_str.parse::<u64>().ok().map(Duration::from_millis);
    }

    // Try seconds: "5s" or "5.5s"
    if let Some(sec_str) = s.strip_suffix('s') {
        // Might have minutes prefix: "1m30s" → won't have 's' suffix directly
        // Simple case: just seconds.
        if !sec_str.contains('m') {
            return sec_str.parse::<f64>().ok().map(Duration::from_secs_f64);
        }
    }

    // Try "XmYs" format.
    if let Some(m_pos) = s.find('m') {
        let mins: u64 = s[..m_pos].parse().ok()?;
        let rest = &s[m_pos + 1..];
        let secs: u64 = if rest.is_empty() {
            0
        } else {
            rest.strip_suffix('s')?.parse().ok()?
        };
        return Some(Duration::from_secs(mins * 60 + secs));
    }

    // Try plain number as seconds.
    s.parse::<f64>().ok().map(Duration::from_secs_f64)
}

// ---------------------------------------------------------------------------
// RateLimitTracker
// ---------------------------------------------------------------------------

/// Tracks rate limit state across requests.
#[derive(Debug)]
pub struct RateLimitTracker {
    /// Most recent rate limit info.
    latest: Option<RateLimitInfo>,
    /// Number of requests made in this tracking window.
    request_count: u32,
    /// Total tokens consumed in this tracking window.
    tokens_consumed: u64,
    /// When tracking started.
    window_start: Instant,
}

impl Default for RateLimitTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimitTracker {
    #[must_use]
    pub fn new() -> Self {
        Self {
            latest: None,
            request_count: 0,
            tokens_consumed: 0,
            window_start: Instant::now(),
        }
    }

    /// Update with new rate limit info from a response.
    pub fn update(&mut self, info: RateLimitInfo, tokens_used: u64) {
        self.latest = Some(info);
        self.request_count += 1;
        self.tokens_consumed += tokens_used;
    }

    /// Whether we should back off before the next request.
    #[must_use]
    pub fn should_back_off(&self) -> bool {
        self.latest
            .as_ref()
            .is_some_and(RateLimitInfo::is_near_limit)
    }

    /// Suggested wait duration if we should back off.
    #[must_use]
    pub fn back_off_duration(&self) -> Option<Duration> {
        let info = self.latest.as_ref()?;
        if !info.is_near_limit() {
            return None;
        }
        info.reset_at.map(|reset| {
            reset
                .checked_duration_since(Instant::now())
                .unwrap_or(Duration::from_secs(1))
        })
    }

    /// Number of requests in current window.
    #[must_use]
    pub fn request_count(&self) -> u32 {
        self.request_count
    }

    /// Total tokens consumed in current window.
    #[must_use]
    pub fn tokens_consumed(&self) -> u64 {
        self.tokens_consumed
    }

    /// Reset the tracking window.
    pub fn reset(&mut self) {
        self.latest = None;
        self.request_count = 0;
        self.tokens_consumed = 0;
        self.window_start = Instant::now();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- TokenLimiter --

    #[test]
    fn limiter_basic_budget() {
        let limiter = TokenLimiter::new(200_000, 16_000);
        let budget = limiter.adaptive_limit(0, 0);
        assert_eq!(budget.max_output, 16_000);
        assert_eq!(budget.tool_reserved, 0);
    }

    #[test]
    fn limiter_output_shrinks_with_long_conversation() {
        let limiter = TokenLimiter::new(200_000, 16_000);
        let budget = limiter.adaptive_limit(190_000, 0);
        // Only 10k tokens left, so output is capped at 10k.
        assert_eq!(budget.max_output, 10_000);
    }

    #[test]
    fn limiter_min_output_enforced() {
        let limiter = TokenLimiter::new(200_000, 16_000).with_min_output(512);
        let budget = limiter.adaptive_limit(200_000, 0);
        assert_eq!(budget.max_output, 512);
    }

    #[test]
    fn limiter_tool_reservation() {
        let limiter = TokenLimiter::new(200_000, 16_000).with_tokens_per_tool(1000);
        let budget = limiter.adaptive_limit(0, 10);
        assert_eq!(budget.tool_reserved, 10_000);
        // Output should still be full since plenty of room.
        assert_eq!(budget.max_output, 16_000);
    }

    #[test]
    fn limiter_total_input() {
        let limiter = TokenLimiter::new(200_000, 16_000).with_tokens_per_tool(500);
        let budget = limiter.adaptive_limit(50_000, 5);
        assert_eq!(budget.tool_reserved, 2500);
        assert!(budget.total_input() > 0);
    }

    // -- RateLimitInfo --

    #[test]
    fn rate_limit_info_from_headers() {
        let headers = vec![
            (
                "x-ratelimit-remaining-requests".to_string(),
                "50".to_string(),
            ),
            (
                "x-ratelimit-remaining-tokens".to_string(),
                "10000".to_string(),
            ),
            ("x-ratelimit-limit-requests".to_string(), "100".to_string()),
            ("x-ratelimit-limit-tokens".to_string(), "100000".to_string()),
        ];
        let info = RateLimitInfo::from_headers(&headers);
        assert_eq!(info.requests_remaining, Some(50));
        assert_eq!(info.tokens_remaining, Some(10000));
        assert_eq!(info.requests_limit, Some(100));
        assert_eq!(info.tokens_limit, Some(100000));
    }

    #[test]
    fn rate_limit_info_case_insensitive() {
        let headers = vec![(
            "X-RateLimit-Remaining-Requests".to_string(),
            "42".to_string(),
        )];
        let info = RateLimitInfo::from_headers(&headers);
        assert_eq!(info.requests_remaining, Some(42));
    }

    #[test]
    fn rate_limit_near_limit() {
        let headers = vec![
            (
                "x-ratelimit-remaining-requests".to_string(),
                "5".to_string(),
            ),
            ("x-ratelimit-limit-requests".to_string(), "1000".to_string()),
        ];
        let info = RateLimitInfo::from_headers(&headers);
        assert!(info.is_near_limit()); // 5/1000 < 10%
    }

    #[test]
    fn rate_limit_not_near_limit() {
        let headers = vec![
            (
                "x-ratelimit-remaining-requests".to_string(),
                "500".to_string(),
            ),
            ("x-ratelimit-limit-requests".to_string(), "1000".to_string()),
        ];
        let info = RateLimitInfo::from_headers(&headers);
        assert!(!info.is_near_limit()); // 500/1000 = 50%
    }

    #[test]
    fn rate_limit_empty_headers() {
        let info = RateLimitInfo::from_headers(&[]);
        assert!(info.requests_remaining.is_none());
        assert!(!info.is_near_limit());
    }

    // -- parse_reset_duration --

    #[test]
    fn parse_duration_seconds() {
        assert_eq!(parse_reset_duration("5s"), Some(Duration::from_secs(5)));
    }

    #[test]
    fn parse_duration_milliseconds() {
        assert_eq!(
            parse_reset_duration("500ms"),
            Some(Duration::from_millis(500))
        );
    }

    #[test]
    fn parse_duration_minutes_seconds() {
        assert_eq!(parse_reset_duration("1m30s"), Some(Duration::from_secs(90)));
    }

    #[test]
    fn parse_duration_plain_number() {
        assert_eq!(parse_reset_duration("10"), Some(Duration::from_secs(10)));
    }

    // -- RateLimitTracker --

    #[test]
    fn tracker_initially_no_backoff() {
        let tracker = RateLimitTracker::new();
        assert!(!tracker.should_back_off());
        assert_eq!(tracker.request_count(), 0);
        assert_eq!(tracker.tokens_consumed(), 0);
    }

    #[test]
    fn tracker_update_counts() {
        let mut tracker = RateLimitTracker::new();
        let info = RateLimitInfo {
            requests_remaining: Some(100),
            tokens_remaining: Some(50000),
            requests_limit: Some(1000),
            tokens_limit: Some(100000),
            reset_at: None,
        };
        tracker.update(info, 500);
        assert_eq!(tracker.request_count(), 1);
        assert_eq!(tracker.tokens_consumed(), 500);
    }

    #[test]
    fn tracker_back_off_when_near_limit() {
        let mut tracker = RateLimitTracker::new();
        let info = RateLimitInfo {
            requests_remaining: Some(2),
            tokens_remaining: Some(100),
            requests_limit: Some(1000),
            tokens_limit: Some(100000),
            reset_at: Some(Instant::now() + Duration::from_secs(5)),
        };
        tracker.update(info, 100);
        assert!(tracker.should_back_off());
        assert!(tracker.back_off_duration().is_some());
    }

    #[test]
    fn tracker_reset() {
        let mut tracker = RateLimitTracker::new();
        let info = RateLimitInfo {
            requests_remaining: Some(1),
            tokens_remaining: Some(1),
            requests_limit: Some(100),
            tokens_limit: Some(100),
            reset_at: None,
        };
        tracker.update(info, 1000);
        tracker.reset();
        assert!(!tracker.should_back_off());
        assert_eq!(tracker.request_count(), 0);
        assert_eq!(tracker.tokens_consumed(), 0);
    }
}
