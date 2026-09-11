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

use safec_ir::target::Target;

use clap::ValueEnum;

use crate::safety::SafetyLevel;

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
    /// The machine to compile for.
    ///
    /// Not the one this is running on, except by default. See ADR-0013 and
    /// `docs/architecture.md`'s "Output depends on the target, never on the
    /// host".
    pub target: Target,
    /// Whether `Unknown` analysis results are errors rather than warnings.
    ///
    /// The resolved answer, not the raw flag: `--safety strict` sets it too,
    /// because that level is defined as leaving nothing `Unknown`.
    pub deny_unknown: bool,
    /// When to colorize diagnostics.
    pub color: ColorMode,
}

/// An artifact the compiler can produce.
///
/// The variants are declared in pipeline order, from the first stage of the
/// frontend to the final build product, and their command line spellings are
/// pinned by a test.
///
/// Deliberately not `Ord`. Ordering invites a stage to ask "is what was
/// requested beyond me?", which is half a question: a pipeline grows at both
/// ends, and a comparison says nothing about a stage declared earlier. The
/// driver matches on the kind instead, so a variant added anywhere has to be
/// answered before the crate builds. `--emit` is part of the stable interface rather than a
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

impl EmitKind {
    /// How `--emit` spells this kind.
    ///
    /// Asked of the `ValueEnum` derive rather than answered by a table beside
    /// it, which is what that derive is here for: a second spelling of the same
    /// thing is a second thing to keep in step, and this one is read by a user
    /// in a diagnostic while the other decides what they may type.
    ///
    /// A `String` because the name is borrowed from a value clap builds on
    /// demand. A diagnostic allocates anyway.
    pub fn spelling(self) -> String {
        self.to_possible_value()
            .expect("every kind is a value `--emit` takes")
            .get_name()
            .to_owned()
    }

    /// Whether one artifact of this kind can be made from several inputs.
    ///
    /// A dump appends: `--emit tokens a.c b.c` reads as one listing because
    /// every line names its file. A module cannot, because appending two
    /// translation units is not a module with duplicates in it, so the driver
    /// refuses a run that asks for one. A program is made of several
    /// translation units by definition, which is what linking is, so it
    /// answers yes about what [`Self::Executable`] will do rather than about
    /// the nothing it does today.
    ///
    /// **Whoever makes `--emit executable` real owes this answer a second
    /// look.** It is yes because a program holds several inputs, and it is only
    /// true of an implementation that links them rather than building each one
    /// over the last.
    ///
    /// Asked of the kind, and exhaustively, rather than spelled as a list of
    /// the kinds that cannot: a second input is either *appended* to what the
    /// first produced or *replaces* it, so a kind added to this enum and
    /// forgotten here would silently throw away every input but the last.
    /// `E0004` is what stops that, and it is the reason this is a `match` and
    /// not a `matches!`.
    pub fn spans_inputs(self) -> bool {
        match self {
            Self::Tokens | Self::Ast | Self::SafetyIr | Self::Executable => true,
            Self::LlvmIr | Self::Object => false,
        }
    }
}

/// When to colorize output.
///
/// Deliberately not `clap::ColorChoice`: that type governs clap's own help
/// output, and letting it stand in here would put an argument parser in the
/// middle of the diagnostics layer.
///
/// Unlike [`SafetyLevel`], this stays an option. It is a presentation choice
/// the user made rather than a concept an analysis reasons about, so the
/// renderer reading it from here is the right direction.
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
