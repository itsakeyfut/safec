//! The safety model: how much of it `safec` is asked to enforce.
//!
//! Separate from [`crate::options`] on purpose. A safety level is something an
//! analysis compares itself against, not something the user asked for. The
//! command line selects one, but the meaning belongs here, and keeping it here
//! is what lets [`crate::diagnostics`] say which check a report came from
//! without depending on what the user typed.
//!
//! `clap` appears for the `ValueEnum` derive, which gives the levels their
//! command line spellings. That is a trait implementation rather than an API
//! surface, so no clap type crosses this boundary. If the safety model ever has
//! to build without an argument parser, the derive is what to replace.

use clap::ValueEnum;

/// How much of the safety model to enforce.
///
/// Levels are cumulative: each one enables every check below it. The variants
/// are declared in increasing order so that `Ord` can be derived, letting an
/// analysis pass ask `level >= SafetyLevel::Lifetime` instead of matching on
/// every variant it applies to.
///
/// A level decides which checks run and nothing else. How loudly a check speaks
/// is decided by what it concluded and by `--deny-unknown`, not by the level:
/// a result the analysis proved is an error at every level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum SafetyLevel {
    /// Level 0: ordinary C, no safety analysis.
    Off,
    /// Level 1: memory checks, such as use-after-free and double free.
    Memory,
    /// Level 2: adds lifetime and escape analysis.
    Lifetime,
    /// Level 3: adds ownership and move checking.
    Ownership,
    /// Level 4: adds thread safety checking.
    Thread,
    /// Level 5: the fully checked subset. Nothing may be left `Unknown`.
    Strict,
}

impl SafetyLevel {
    /// The numeric level, as used in the design documents.
    ///
    /// Diagnostics use this to tell the user which level a check belongs to.
    pub fn number(self) -> u8 {
        match self {
            Self::Off => 0,
            Self::Memory => 1,
            Self::Lifetime => 2,
            Self::Ownership => 3,
            Self::Thread => 4,
            Self::Strict => 5,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spelled out rather than looped over `value_variants()`. That iterates in
    /// declaration order, which is the same order the derived `Ord` compares
    /// on, so a loop would hold for any declaration order at all and pin
    /// nothing.
    #[test]
    fn safety_levels_are_ordered_by_strictness() {
        assert!(SafetyLevel::Off < SafetyLevel::Memory);
        assert!(SafetyLevel::Memory < SafetyLevel::Lifetime);
        assert!(SafetyLevel::Lifetime < SafetyLevel::Ownership);
        assert!(SafetyLevel::Ownership < SafetyLevel::Thread);
        assert!(SafetyLevel::Thread < SafetyLevel::Strict);
    }

    #[test]
    fn safety_level_numbers_match_the_design_documents() {
        assert_eq!(SafetyLevel::Off.number(), 0);
        assert_eq!(SafetyLevel::Memory.number(), 1);
        assert_eq!(SafetyLevel::Lifetime.number(), 2);
        assert_eq!(SafetyLevel::Ownership.number(), 3);
        assert_eq!(SafetyLevel::Thread.number(), 4);
        assert_eq!(SafetyLevel::Strict.number(), 5);
    }

    /// `Ord` comes from the declaration order and `number` is written out by
    /// hand; this keeps the two from drifting apart when a level is added.
    #[test]
    fn safety_level_numbers_follow_the_declaration_order() {
        for (position, level) in SafetyLevel::value_variants().iter().enumerate() {
            assert_eq!(
                level.number(),
                position as u8,
                "{level:?} is out of step with its position"
            );
        }
    }
}
