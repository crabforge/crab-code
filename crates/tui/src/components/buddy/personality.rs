//! Personality traits for the buddy companion.
//!
//! Each buddy has a primary personality derived from its sprite hash.
//! The personality influences the tone of buddy messages.

use std::fmt;

use super::sprite::Sprite;

/// Primary personality trait of a buddy.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Personality {
    /// Asks follow-up questions; notices patterns.
    Curious,
    /// Offers suggestions; celebrates progress.
    Helpful,
    /// Uses light humor; keeps morale high.
    Playful,
}

impl Personality {
    /// Derive a personality from a [`Sprite`].
    ///
    /// The derivation is deterministic: the same sprite always produces the
    /// same personality.
    #[allow(dead_code)]
    pub fn from_sprite(sprite: &Sprite) -> Self {
        // Use a simple discriminator combining the species and eyes ordinals.
        let ordinal = sprite.species as usize + sprite.eyes as usize;
        match ordinal % 3 {
            0 => Self::Curious,
            1 => Self::Helpful,
            _ => Self::Playful,
        }
    }

    /// A short greeting appropriate for this personality.
    #[allow(dead_code)]
    pub fn greeting(self) -> &'static str {
        match self {
            Self::Curious => "Hmm, what are we working on today?",
            Self::Helpful => "Ready to help! What do you need?",
            Self::Playful => "Let's build something awesome!",
        }
    }

    /// A short encouragement message.
    #[allow(dead_code)]
    pub fn encouragement(self) -> &'static str {
        match self {
            Self::Curious => "Interesting approach -- let's see where it goes.",
            Self::Helpful => "You're making great progress!",
            Self::Playful => "Nice one! Keep it rolling!",
        }
    }
}

impl fmt::Display for Personality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Curious => write!(f, "curious"),
            Self::Helpful => write!(f, "helpful"),
            Self::Playful => write!(f, "playful"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::buddy::sprite::{Eyes, Hat, Species, generate_sprite};

    #[test]
    fn personality_from_sprite_is_deterministic() {
        let sprite = generate_sprite("stable-session");
        let a = Personality::from_sprite(&sprite);
        let b = Personality::from_sprite(&sprite);
        assert_eq!(a, b);
    }

    #[test]
    fn all_personalities_have_greetings() {
        for p in [Personality::Curious, Personality::Helpful, Personality::Playful] {
            assert!(!p.greeting().is_empty());
            assert!(!p.encouragement().is_empty());
        }
    }

    #[test]
    fn display_impl() {
        assert_eq!(Personality::Curious.to_string(), "curious");
        assert_eq!(Personality::Helpful.to_string(), "helpful");
        assert_eq!(Personality::Playful.to_string(), "playful");
    }

    #[test]
    fn all_three_personalities_reachable() {
        // Construct sprites that exercise each branch of the modulo.
        let sprites = [
            Sprite { species: Species::Crab, eyes: Eyes::Round, hat: Hat::None },     // 0+0 = 0 mod 3 = 0
            Sprite { species: Species::Crab, eyes: Eyes::Happy, hat: Hat::None },     // 0+1 = 1 mod 3 = 1
            Sprite { species: Species::Crab, eyes: Eyes::Star, hat: Hat::None },      // 0+2 = 2 mod 3 = 2
        ];
        let personalities: Vec<_> = sprites.iter().map(Personality::from_sprite).collect();
        assert!(personalities.contains(&Personality::Curious));
        assert!(personalities.contains(&Personality::Helpful));
        assert!(personalities.contains(&Personality::Playful));
    }
}
