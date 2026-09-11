//! What a safety check answers with.
//!
//! `docs/safety-model.md` makes the model three-valued and draws the line this
//! module is one side of: "a check names its conclusion and never reads the
//! policy". A conclusion is the naming. What each one costs a build is decided
//! once, elsewhere, from the policy the user asked for, which is ADR-0001.
//!
//! Here rather than beside the diagnostics because a check cannot see a
//! diagnostic: this crate is what an analysis is written against and nothing in
//! the workspace points at `safec`. `safec_llvm::emit::Refusal` has the same
//! shape for the same reason, and `safec` turns one of those into a diagnostic
//! too.

/// What a check concluded about one thing it looked at.
///
/// The three answers `docs/safety-model.md` gives, and the whole of what a
/// check says: not a severity, not a message, and not whether the build should
/// fail. What each means for what a user reads is `Diagnostic::concluded`'s in
/// `safec`, which is the one place that answers it.
///
/// **Three rather than two.** A check that has proved something safe could say
/// nothing instead, and then "safe is not reported" would be a rule about every
/// caller rather than a fact about this type. An analysis whose lattice answers
/// all three has somewhere to put the third, and the thing that turns a
/// conclusion into a report answers `None` for this one, so reporting a safe
/// result is not something to remember not to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Conclusion {
    /// Proved safe. There is nothing to say.
    Safe,
    /// Proved unsafe. A fact about the program, like a syntax error and unlike
    /// a suspicion.
    Unsafe,
    /// Neither proved safe nor proved wrong.
    ///
    /// The unknown of the three-valued model, and the thing an annotation
    /// exists to remove. Whether it fails a build is the policy's to say, which
    /// is why a check that reaches this does not need to know what
    /// `--deny-unknown` is.
    Unknown,
}
