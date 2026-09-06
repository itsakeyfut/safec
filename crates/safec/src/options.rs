//! The options the compiler runs on.
//!
//! [`crate::cli::Cli`] is only the command line spelling of these. Nothing past
//! the CLI boundary should depend on that type: the planned Clang adapter will
//! build `Options` directly, with no argument parser involved.
//!
//! `clap` still appears here for the `ValueEnum` derives, which give the value
//! enums their command line spellings. That is a trait implementation rather
//! than an API surface, so no clap type crosses this boundary.

use std::path::PathBuf;

use clap::ValueEnum;

/// Everything the compiler needs to know about one invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// The C source files to compile.
    pub inputs: Vec<PathBuf>,
    /// Where to write the result, when the user asked for a specific path.
    pub output: Option<PathBuf>,
    /// How much of the safety model to enforce.
    pub safety: SafetyLevel,
    /// The artifact to produce.
    pub emit: EmitKind,
    /// Whether `Unknown` analysis results are errors rather than warnings.
    ///
    /// The resolved answer, not the raw flag: `--safety strict` sets it too,
    /// because that level is defined as leaving nothing `Unknown`.
    pub deny_unknown: bool,
    /// When to colorize diagnostics.
    pub color: ColorMode,
}

/// How much of the safety model to enforce.
///
/// Levels are cumulative: each one enables every check below it. The variants
/// are declared in increasing order so that `Ord` can be derived, letting an
/// analysis pass ask `level >= SafetyLevel::Lifetime` instead of matching on
/// every variant it applies to.
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

/// An artifact the compiler can produce.
///
/// The variants are declared in pipeline order, from the first stage of the
/// frontend to the final build product, and their command line spellings are
/// pinned by a test. `--emit` is part of the stable interface rather than a
/// debug convenience, so a variant is renamed only deliberately.
///
/// The name follows rustc's `--emit`, which takes a *set* of artifacts. Only
/// one can be asked for today, but the option is expected to widen to a list
/// rather than to change meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum EmitKind {
    /// The token stream produced by the lexer.
    Tokens,
    /// The abstract syntax tree produced by the parser.
    Ast,
    /// The Safety IR, the central abstraction the analyses operate on.
    SafetyIr,
    /// LLVM IR, in textual form.
    LlvmIr,
    /// An object file.
    Object,
    /// A linked native executable.
    ///
    /// Spelled out rather than abbreviated, because the artifact only carries
    /// an `.exe` extension on Windows. `exe` stays accepted as an alias.
    #[value(alias = "exe")]
    Executable,
}

/// When to colorize output.
///
/// Deliberately not `clap::ColorChoice`: that type governs clap's own help
/// output, and letting it stand in here would put an argument parser in the
/// middle of the diagnostics layer. This moves to the diagnostics module once
/// one exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum ColorMode {
    /// Colorize when the stream is a terminal.
    Auto,
    /// Always colorize.
    Always,
    /// Never colorize.
    Never,
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
