//! Seed-based PRNG sprite generation for the buddy system.
//!
//! Generates a deterministic buddy appearance (species, eyes, hat) from a
//! session identifier hash so that each session sees the same companion.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Visual species of the buddy sprite.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Species {
    /// A small crab (the project mascot).
    Crab,
    /// An octopus.
    Octopus,
    /// A robot.
    Robot,
    /// A fox.
    Fox,
}

/// Eye style for the buddy sprite.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eyes {
    /// Normal round eyes: `(o o)`.
    Round,
    /// Happy squinting eyes: `(^ ^)`.
    Happy,
    /// Star-shaped eyes: `(* *)`.
    Star,
    /// Winking eye: `(- o)`.
    Wink,
}

/// Hat or accessory worn by the buddy sprite.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hat {
    /// No hat.
    None,
    /// Top hat: `___`.
    TopHat,
    /// Baseball cap: `>`.
    Cap,
    /// Party hat: `^`.
    Party,
}

/// Complete sprite descriptor.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sprite {
    /// The buddy species.
    pub species: Species,
    /// The eye style.
    pub eyes: Eyes,
    /// The hat or accessory.
    pub hat: Hat,
}

const SPECIES_TABLE: [Species; 4] = [Species::Crab, Species::Octopus, Species::Robot, Species::Fox];
const EYES_TABLE: [Eyes; 4] = [Eyes::Round, Eyes::Happy, Eyes::Star, Eyes::Wink];
const HAT_TABLE: [Hat; 4] = [Hat::None, Hat::TopHat, Hat::Cap, Hat::Party];

/// Generate a deterministic [`Sprite`] from a session identifier string.
///
/// The identifier is hashed with the standard library hasher and the
/// resulting bits are used to select species, eyes, and hat.
#[allow(dead_code)]
pub fn generate_sprite(session_id: &str) -> Sprite {
    let mut hasher = DefaultHasher::new();
    session_id.hash(&mut hasher);
    let hash = hasher.finish();

    let species_idx = (hash & 0x03) as usize;
    let eyes_idx = ((hash >> 2) & 0x03) as usize;
    let hat_idx = ((hash >> 4) & 0x03) as usize;

    Sprite {
        species: SPECIES_TABLE[species_idx],
        eyes: EYES_TABLE[eyes_idx],
        hat: HAT_TABLE[hat_idx],
    }
}

/// Return ASCII art lines for the given sprite.
///
/// The returned vector contains one string per line of the sprite.
#[allow(dead_code)]
pub fn render_ascii(sprite: &Sprite) -> Vec<String> {
    let hat_line = match sprite.hat {
        Hat::None => String::new(),
        Hat::TopHat => " ___\n |___|".into(),
        Hat::Cap => "  >---".into(),
        Hat::Party => "   ^".into(),
    };

    let eyes = match sprite.eyes {
        Eyes::Round => "(o o)",
        Eyes::Happy => "(^ ^)",
        Eyes::Star => "(* *)",
        Eyes::Wink => "(- o)",
    };

    let body = match sprite.species {
        Species::Crab => format!("  ╱▔╲{eyes}╱▔╲\n  ╲_╱ ███ ╲_╱\n    ╱╱   ╲╲"),
        Species::Octopus => format!("  {eyes}\n /||||\\"),
        Species::Robot => format!(" [{eyes}]\n |____|"),
        Species::Fox => format!(" /\\{eyes}/\\\n   (  )"),
    };

    let mut lines = Vec::new();
    if !hat_line.is_empty() {
        lines.push(hat_line);
    }
    lines.push(body);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_session_id_produces_same_sprite() {
        let a = generate_sprite("session-abc-123");
        let b = generate_sprite("session-abc-123");
        assert_eq!(a, b);
    }

    #[test]
    fn different_session_ids_may_differ() {
        let a = generate_sprite("session-1");
        let b = generate_sprite("session-2");
        // Not guaranteed to differ but statistically likely; at minimum the
        // function should not panic.
        let _ = (a, b);
    }

    #[test]
    fn render_ascii_produces_non_empty_output() {
        let sprite = generate_sprite("test-session");
        let lines = render_ascii(&sprite);
        assert!(!lines.is_empty());
        // At least the body line should be non-empty
        assert!(lines.iter().any(|l| !l.is_empty()));
    }

    #[test]
    fn all_species_render() {
        for species in &SPECIES_TABLE {
            let sprite = Sprite {
                species: *species,
                eyes: Eyes::Round,
                hat: Hat::None,
            };
            let lines = render_ascii(&sprite);
            assert!(!lines.is_empty());
        }
    }

    #[test]
    fn all_hats_render() {
        for hat in &HAT_TABLE {
            let sprite = Sprite {
                species: Species::Crab,
                eyes: Eyes::Happy,
                hat: *hat,
            };
            let lines = render_ascii(&sprite);
            assert!(!lines.is_empty());
        }
    }
}
