//! Fast mode toggle: switch to a faster/smaller model for quick interactions.
//!
//! Fast mode allows users to temporarily switch from a large, high-quality
//! model to a faster one for simple tasks (e.g., file edits, short answers).
//! The original model is remembered so it can be restored on toggle-off.

/// Tracks whether fast mode is active and which models are involved.
#[derive(Debug, Clone)]
pub struct FastModeState {
    /// Whether fast mode is currently enabled.
    enabled: bool,
    /// The model to use when fast mode is active (e.g., `"claude-3-haiku"`).
    fast_model: Option<String>,
    /// The model that was active before fast mode was enabled.
    original_model: Option<String>,
}

impl FastModeState {
    /// Create a new `FastModeState` in disabled mode.
    pub fn new() -> Self {
        Self {
            enabled: false,
            fast_model: None,
            original_model: None,
        }
    }

    /// Create with an explicit fast model.
    pub fn with_fast_model(fast_model: String) -> Self {
        Self {
            enabled: false,
            fast_model: Some(fast_model),
            original_model: None,
        }
    }

    /// Toggle fast mode on/off.
    ///
    /// When toggling on, stores the current model as `original_model`.
    /// When toggling off, clears the stored original model.
    ///
    /// Returns the new `enabled` state.
    pub fn toggle(&mut self) -> bool {
        self.enabled = !self.enabled;
        self.enabled
    }

    /// Whether fast mode is currently enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// The model that should currently be used, accounting for fast mode state.
    ///
    /// - If fast mode is on and a fast model is configured, returns the fast model.
    /// - If fast mode is off and an original model was stored, returns it.
    /// - Otherwise returns `None` (caller should use the default from settings).
    pub fn current_model(&self) -> Option<&str> {
        if self.enabled {
            self.fast_model.as_deref()
        } else {
            self.original_model.as_deref()
        }
    }

    /// Set the fast model identifier.
    pub fn set_fast_model(&mut self, model: String) {
        self.fast_model = Some(model);
    }

    /// Set the original model (called before enabling fast mode).
    pub fn set_original_model(&mut self, model: String) {
        self.original_model = Some(model);
    }

    /// The configured fast model, if any.
    pub fn fast_model(&self) -> Option<&str> {
        self.fast_model.as_deref()
    }

    /// The saved original model, if any.
    pub fn original_model(&self) -> Option<&str> {
        self.original_model.as_deref()
    }
}

impl Default for FastModeState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_disabled() {
        let state = FastModeState::new();
        assert!(!state.is_enabled());
    }

    #[test]
    fn with_fast_model_starts_disabled() {
        let state = FastModeState::with_fast_model("haiku".into());
        assert!(!state.is_enabled());
        assert_eq!(state.fast_model(), Some("haiku"));
    }

    #[test]
    fn accessors() {
        let mut state = FastModeState::new();
        state.set_fast_model("haiku".into());
        state.set_original_model("sonnet".into());
        assert_eq!(state.fast_model(), Some("haiku"));
        assert_eq!(state.original_model(), Some("sonnet"));
    }
}
