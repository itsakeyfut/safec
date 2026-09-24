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
/// is decided by what it concluded and by the policy the run resolved to, not by
/// the level: a result the analysis proved is an error at every level that runs
/// the check that proved it. What the level does decide is whether any check
/// runs at all, which is why [`crate::diagnostics::Policy`] reads it: see
/// ADR-0033.
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
    /// The highest level whose checks exist.
    ///
    /// One name rather than the comparison spelled out wherever the question is
    /// asked. The driver runs the memory and nullability checks at
    /// [`Self::Memory`]; every level above it selects checks that do not exist,
    /// and what a run is told about that is ADR-0035.
    ///
    /// **A fact about the compiler, where `Options::delivered` is a fact about
    /// one run.** The two were one word until a review pointed out that the
    /// wrong one is a type-correct shortcut at every call site, which is the
    /// substitution ADR-0035's headline mutation exists to catch.
    ///
    /// **Moving this is the last step of landing a level, not the only one.**
    /// The checks have to exist, a gate in `driver::lowered` has to run them,
    /// `options.rs`'s table has to answer for the new row, and three corpus
    /// expectations have to be re-blessed. Moving it alone leaves the compiler
    /// silent about a level nothing checks, and here silence *means* the level
    /// was delivered, which is the bottom row of `CLAUDE.md`'s list.
    /// `driver.rs::the_implemented_level_runs_a_check_the_level_below_it_does_not`
    /// is what refuses that: the four expectations that also change read as
    /// housekeeping, and that one reads as the check that is missing.
    pub const IMPLEMENTED: Self = Self::Memory;

    /// How `--safety` spells this level.
    ///
    /// Asked of the `ValueEnum` derive rather than answered by a table beside
    /// it, for the reason [`crate::options::EmitKind::spelling`] gives: a second
    /// spelling of the same thing is a second thing to keep in step, and this
    /// one is read by a user in a diagnostic while the other decides what they
    /// may type.
    pub fn spelling(self) -> String {
        self.to_possible_value()
            .expect("every level is a value `--safety` takes")
            .get_name()
            .to_owned()
    }

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

    /// Every level's spelling, as the command line takes it.
    ///
    /// **Two of the six reach [`SafetyLevel::spelling`] from nowhere else.** Its
    /// only caller is the undelivered report, and no corpus case asks for
    /// `ownership` or `thread`, so those two were never spelled by anything.
    /// Measured: special-casing `Ownership` inside `spelling` to answer
    /// `wrong-spelling` left the entire workspace green, which put a diagnostic
    /// naming a flag the user never typed one edit away.
    ///
    /// Written out rather than compared against `to_possible_value`, which is
    /// what `spelling` is implemented in terms of. That comparison holds for
    /// whatever the derive happens to say and is RK-001's shape; this table is a
    /// definition of what a user may type.
    ///
    /// Mutation: answer any other string for any one variant. This fails, naming
    /// it, and nothing else in the workspace does.
    #[test]
    fn every_safety_level_is_spelled_the_way_the_command_line_takes_it() {
        const ROWS: [(SafetyLevel, &str); 6] = [
            (SafetyLevel::Off, "off"),
            (SafetyLevel::Memory, "memory"),
            (SafetyLevel::Lifetime, "lifetime"),
            (SafetyLevel::Ownership, "ownership"),
            (SafetyLevel::Thread, "thread"),
            (SafetyLevel::Strict, "strict"),
        ];

        assert_eq!(
            ROWS.len(),
            SafetyLevel::value_variants().len(),
            "a safety level was added and this table did not answer for it"
        );

        for (level, spelling) in ROWS {
            assert_eq!(level.spelling(), spelling, "{level:?}");
        }
    }
}
