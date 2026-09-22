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
    /// Whether a result the analysis could not prove is a warning rather than
    /// an error.
    ///
    /// The request, not the resolved answer. Every value of it is a legitimate
    /// thing to ask for, and what it means at the level this run asked for is
    /// decided in one place, [`crate::diagnostics::Policy::new`]. See ADR-0004
    /// for why that place is not here, and ADR-0033 for why the default is to
    /// deny.
    pub allow_unknown: bool,
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
    /// refuses a run that asks for one.
    ///
    /// A program is made of several translation units by definition, which is
    /// what linking is, and this answers yes because that is what the driver
    /// does: every input's module is kept and one link is run over all of them.
    /// It answered no while one was, which is the rule following the
    /// implementation rather than the intention, because a rule that promises
    /// what the code does not do is a run that silently builds the last input
    /// and throws the rest away.
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

    /// Whether a file of this kind has to be runnable by the machine it was
    /// made for.
    ///
    /// The one kind that needs a mode as well as bytes. An artifact is bytes
    /// here, and a mode is not one of them, so the run that writes a program
    /// has to say that it is one.
    ///
    /// Exhaustive for the reason [`Self::spans_inputs`] gives.
    pub fn is_a_program(self) -> bool {
        match self {
            Self::Tokens | Self::Ast | Self::SafetyIr | Self::LlvmIr | Self::Object => false,
            Self::Executable => true,
        }
    }

    /// Whether an artifact of this kind is worth writing when the run reported
    /// an error.
    ///
    /// A dump of what could be read is still true about what could be read: a
    /// user redirecting `--emit safety-ir` over three inputs, one of which is
    /// missing, wants the two that are there. A build product is not, because
    /// nothing downstream reads a diagnostic. An object whose module lost a
    /// function the backend refused still assembles, and left on disk it is a
    /// program with a function deleted from it, newer than the source it came
    /// from.
    ///
    /// **LLVM IR is one of those**, and the sentence above is true of it word
    /// for word: `clang out.ll -o prog` compiles the file, and a run whose
    /// backend refused a function writes one with that function reduced to a
    /// declaration. It answered `true` here until #144, where the missing-input
    /// reason was measured against it and does not reach: [`Self::spans_inputs`]
    /// answers `false` for this kind, so a run over several inputs is refused
    /// before one of them can be missing.
    ///
    /// **Only the kinds above and `LlvmIr` reach this.** `Object` and
    /// `Executable` are on the false side and the write rule never consults
    /// them: the object arm of `compile` gives up before it assembles anything
    /// a run that reported, and `finish` returns before it links, so both
    /// artifacts are already empty and the write rule's other half is what
    /// stops the file. `LlvmIr` is the first member of this side that the rule
    /// decides anything about, because its module is built whatever happened.
    /// Measured, by moving each arm across on its own and watching the suite:
    /// only `Tokens`, `LlvmIr` and, once #144 added a case for it, `SafetyIr`
    /// change an answer.
    ///
    /// **`Ast` is held by nothing**, and says the same as `SafetyIr` for the
    /// same reason. One case holds the rule and a second differing in one word
    /// would be a test of the corpus rather than of this. What is left for that
    /// arm is `error[E0004]`, which makes somebody answer for a kind and, as
    /// RK-015 in the review knowledge bank puts it, nothing more than that.
    ///
    /// **Asked of the file and not of the stream.** `--emit llvm-ir` with no
    /// `-o` still writes its module to the artifact stream on a run that
    /// failed, and it is the only build product that can reach one, because the
    /// other two derive a filename when nobody names a path. What this rule is
    /// about is a file a build system reads as finished and newer than its
    /// source, and a compiler writing to a stream is not making one; a `>` that
    /// turns it into a file is the caller's. What that keeps is a way to read
    /// the IR of a program this compiler refused, and for one of the two ways
    /// it can refuse the stream is the **only** way: `--safety off` gives a
    /// proved-unsafe program its file back and does nothing at all for a
    /// backend refusal, because `SC0801` is not a safety check. Measured:
    /// `--safety off --emit llvm-ir -o out.ll` over a double free exits 0 and
    /// writes, and over `p = p + 1` exits 1 and writes nothing. So the
    /// exception is load-bearing for whoever is debugging the backend rather
    /// than a convenience.
    ///
    /// **That is a line drawn, not a case closed.** `safec --emit llvm-ir x.c >
    /// out.ll` leaves the same file `-o` was just stopped from leaving, and a
    /// `Makefile` writes its rule that way; unless `.DELETE_ON_ERROR` is set,
    /// `make` keeps the target and reads it as up to date next time. So "the
    /// redirect is the caller's" is true about who typed it and not about who
    /// is harmed, and it sits against `Severity::Error`'s own doc comment,
    /// which says the compilation cannot produce an artifact without
    /// qualifying it. Closing it is two lines in `run_compiler`'s
    /// `(None, emitted)` arm and wants a way to say "the stream on purpose"
    /// first, which is a flag with no user today.
    /// `a_double_free_is_found_on_a_backend_run` pins the current answer, so
    /// whoever closes it fails a named test rather than passing quietly.
    ///
    /// Exhaustive for the reason [`Self::spans_inputs`] gives.
    pub fn survives_an_error(self) -> bool {
        match self {
            Self::Tokens | Self::Ast | Self::SafetyIr => true,
            Self::LlvmIr | Self::Object | Self::Executable => false,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A kind that survives an error is a kind that spans inputs.
    ///
    /// **The reason made checkable.** The only reason on record for writing
    /// an artifact from a run that failed is that it is a partial read over
    /// several inputs, which [`EmitKind::survives_an_error`] gives and which
    /// needs the kind to take several. A kind that cannot is a module or an
    /// object, because that is what [`EmitKind::spans_inputs`] answers `false`
    /// for, and those are the build products.
    ///
    /// Driven by `value_variants` rather than by a list, so that it covers a
    /// kind nobody has written yet. `error[E0004]` makes somebody answer both
    /// questions for a new kind and nothing makes them answer rightly:
    /// measured, a kind added and answered the way a textual artifact invites
    /// leaves the whole suite green while `--emit <it> -o out` writes a file
    /// on a run that proved the program unsafe, which is #144 again.
    ///
    /// **One direction only.** A build product that does span inputs, a
    /// dependency file being the candidate, could answer `true` here and this
    /// would not object. That direction leaves a file on disk and this one
    /// costs a missing dump, and the first is what #144 was.
    ///
    /// Mutation: answer `true` from `survives_an_error` for `LlvmIr`, which
    /// does not span. This fails, naming the kind.
    #[test]
    fn a_kind_that_survives_an_error_is_a_kind_that_spans_inputs() {
        for kind in EmitKind::value_variants() {
            assert!(
                !kind.survives_an_error() || kind.spans_inputs(),
                "{kind:?} is written when a run fails and cannot take the several inputs that is the only reason on record for writing one"
            );
        }
    }
}
