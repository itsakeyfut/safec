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

impl Options {
    /// The level this run can be held to, which is never above the one it asked
    /// for.
    ///
    /// Two independent things lower it and the answer is the lowest of the
    /// three: [`SafetyLevel::IMPLEMENTED`] is the highest level with checks behind
    /// it, and an artifact that does not reach the IR runs no check at all
    /// whatever the level was. Both are the same sentence to whoever reads the
    /// report, which is why this is one function rather than two conditions at
    /// the site that reports. Answering two axes with two conditions is the
    /// shape `CLAUDE.md` calls the worst defect this project has had, and
    /// `driver.rs::undelivered` shipped to review with exactly that in its note
    /// and its remedy while this function was avoiding it in the gate. See
    /// ADR-0035.
    ///
    /// `min` for the level and a `match` inside
    /// [`EmitKind::reaches_the_ir`] for the kind, because the levels are
    /// cumulative and ordered on purpose while [`EmitKind`] deliberately is not.
    pub fn delivered(&self) -> SafetyLevel {
        if self.emit.reaches_the_ir() {
            self.safety.min(SafetyLevel::IMPLEMENTED)
        } else {
            SafetyLevel::Off
        }
    }
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

    /// Whether making this artifact builds the Safety IR, which is what every
    /// safety check reads.
    ///
    /// The kinds that answer `false` stop in the frontend, so no check runs for
    /// them however high a level the run asked for. That is a fact about the
    /// artifact rather than about the program, and ADR-0035 is what a run says
    /// about it.
    ///
    /// Exhaustive for the reason [`Self::spans_inputs`] gives, and this enum has
    /// no `Ord` so the question cannot be spelled as a comparison instead. A
    /// comparison would answer "past the last stage that reads the IR" and never
    /// "before the first one", and the pipeline grows at both ends.
    pub fn reaches_the_ir(self) -> bool {
        match self {
            Self::Tokens | Self::Ast => false,
            Self::SafetyIr | Self::LlvmIr | Self::Object | Self::Executable => true,
        }
    }

    /// The safety level a run defaults to when it asks for this artifact.
    ///
    /// The default is the highest level the artifact can carry rather than one
    /// constant: a kind that stops before the IR runs no check, so a constant
    /// `memory` was false for two of the six and turned a `--safety` nobody typed
    /// into a claim nothing answered. See ADR-0035.
    ///
    /// **Here rather than inside `Cli::into_options`, so that a caller which
    /// never saw a command line can ask.** This module's own doc comment says the
    /// Clang adapter will build an [`Options`] directly, and ADR-0004 put the
    /// `--allow-unknown` resolution on `Policy::new` for that reason rather than
    /// in the CLI. A resolution reachable only through clap is one such a caller
    /// answers differently, or not at all, and then gets a report naming flags it
    /// has no command line to have typed.
    ///
    /// `Memory` spelled out rather than [`SafetyLevel::IMPLEMENTED`], which is
    /// equal to it today and is not the same thing. `IMPLEMENTED` rises when a
    /// level lands, and a default that rose with it would reject programs that
    /// built the day before, which is what `docs/safety-model.md` opens by
    /// refusing: the ladder is for migrating existing C, not for moving under
    /// it. Raising
    /// this is a decision about the way in, not a consequence of implementing a
    /// check.
    pub fn default_safety(self) -> SafetyLevel {
        if self.reaches_the_ir() {
            SafetyLevel::Memory
        } else {
            SafetyLevel::Off
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

    /// Which artifacts are made by building the Safety IR.
    ///
    /// Written out rather than asked of the `match`, because a test that asks
    /// the implementation what it says holds for whatever it says, which is
    /// RK-001 in the review knowledge bank. This table is the definition.
    ///
    /// The length check covers the kind nobody has written yet: adding one
    /// without answering for it here fails by name rather than passing quietly.
    ///
    /// Mutation: answer `true` for `Ast`. This fails naming the kind, and the
    /// corpus case `an_artifact_that_stops_before_the_ir_delivers_no_checks`
    /// fails with it.
    #[test]
    fn every_emit_kind_says_whether_it_reaches_the_ir() {
        const ROWS: [(EmitKind, bool); 6] = [
            (EmitKind::Tokens, false),
            (EmitKind::Ast, false),
            (EmitKind::SafetyIr, true),
            (EmitKind::LlvmIr, true),
            (EmitKind::Object, true),
            (EmitKind::Executable, true),
        ];

        assert_eq!(
            ROWS.len(),
            EmitKind::value_variants().len(),
            "an emit kind was added and this table did not answer for it"
        );

        for (kind, reaches) in ROWS {
            assert_eq!(kind.reaches_the_ir(), reaches, "{kind:?}");
        }
    }

    /// A kind that is a program reaches the IR.
    ///
    /// **The reason made checkable.** A program is compiled code and this
    /// compiler has no route from source to code that goes round the Safety IR,
    /// so the table above is not free to answer `false` here for a kind that
    /// answers `true` to [`EmitKind::is_a_program`], whatever that kind turns
    /// out to be.
    ///
    /// Driven by the roster rather than by a list, which is what covers the kind
    /// nobody has written: `error[E0004]` makes somebody add an arm and nothing
    /// whatsoever makes the arm they add correct, which is RK-015.
    ///
    /// Mutation: answer `false` from `reaches_the_ir` for `Executable`. This
    /// fails, naming the kind.
    #[test]
    fn a_kind_that_is_a_program_reaches_the_ir() {
        for kind in EmitKind::value_variants() {
            assert!(
                !kind.is_a_program() || kind.reaches_the_ir(),
                "{kind:?} is a program, and there is no route to one that goes round the IR"
            );
        }
    }

    /// What a run can be held to, for every level against both answers to
    /// whether its artifact reaches the IR.
    ///
    /// The rows are written out rather than computed from
    /// [`SafetyLevel::IMPLEMENTED`], for the reason
    /// `every_emit_kind_says_whether_it_reaches_the_ir` gives. The day
    /// `IMPLEMENTED` moves, this table is what has to be edited to say so, which
    /// is the point of it: that edit is how the change that lands the next level
    /// finds out it owes one here.
    ///
    /// Mutation: drop the `min` and answer `self.safety`. Every row above
    /// `memory` in the reaching column fails.
    ///
    /// Mutation: answer `self.safety.min(SafetyLevel::IMPLEMENTED)` in both arms,
    /// which drops the artifact question. Every row above `off` in the stopping
    /// column fails.
    #[test]
    fn a_run_is_delivered_no_more_than_the_levels_with_checks_behind_it() {
        // The level asked for, what a run delivers when its artifact reaches the
        // IR, and what it delivers when the artifact stops before it.
        const ROWS: [(SafetyLevel, SafetyLevel, SafetyLevel); 6] = [
            (SafetyLevel::Off, SafetyLevel::Off, SafetyLevel::Off),
            (SafetyLevel::Memory, SafetyLevel::Memory, SafetyLevel::Off),
            (SafetyLevel::Lifetime, SafetyLevel::Memory, SafetyLevel::Off),
            (
                SafetyLevel::Ownership,
                SafetyLevel::Memory,
                SafetyLevel::Off,
            ),
            (SafetyLevel::Thread, SafetyLevel::Memory, SafetyLevel::Off),
            (SafetyLevel::Strict, SafetyLevel::Memory, SafetyLevel::Off),
        ];

        assert_eq!(
            ROWS.len(),
            SafetyLevel::value_variants().len(),
            "a safety level was added and this table did not answer for it"
        );

        for (asked, reaching, stopping) in ROWS {
            assert_eq!(
                options(asked, EmitKind::SafetyIr).delivered(),
                reaching,
                "{asked:?}, artifact reaches the IR"
            );
            assert_eq!(
                options(asked, EmitKind::Ast).delivered(),
                stopping,
                "{asked:?}, artifact stops before the IR"
            );
        }
    }

    /// One invocation, for the two fields [`Options::delivered`] reads.
    ///
    /// The rest is whatever builds: nothing here looks at an input, a target or
    /// a colour, and naming a triple would make the answer look as though it
    /// depended on one.
    fn options(safety: SafetyLevel, emit: EmitKind) -> Options {
        Options {
            inputs: Vec::new(),
            output: None,
            safety,
            emit,
            target: Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
            allow_unknown: false,
            color: ColorMode::Never,
        }
    }
}
