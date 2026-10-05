//! The driver: everything between a parsed command line and an exit code.
//!
//! The phases each know how to do one thing. What is decided here is the order
//! they happen in: which files are read, when diagnostics are rendered, and
//! what the process reports. Keeping that in one place is what lets a phase be
//! written without an opinion about the ones around it.
//!
//! It also holds what `--emit tokens` and `--emit ast` write. An artifact is a
//! rendering of what a phase produced rather than a thing the phase owns, so
//! it sits with the driver that decides when to write one. `--emit safety-ir`
//! is the exception and is in `safec_ir::print`, along with the line shape all
//! three share: what an IR line says is a fact about the IR, and ADR-0011 put
//! the IR in a crate this one depends on.
//!
//! [`compile`] does the work and hands back what it found, so that a caller can
//! read the diagnostics rather than scrape them out of a stream. [`run_compiler`] is the
//! half that reports and decides the outcome.
//!
//! **One concern per file.** This file is the pipeline. `driver/words.rs` holds
//! what a safety finding reads as, `backend.rs` running `clang` and what its
//! failures read as, and `dumps.rs` what `--emit tokens`, `--emit ast` and
//! `--emit hatches` write. The tests are in `tests.rs`.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::ast::Ast;
use crate::diagnostics::render::Renderer;
use crate::diagnostics::{Diagnostic, DiagnosticSink, Policy, Remedy};
use crate::lexer::lex;
use crate::lowering::lower;
use crate::options::{EmitKind, Options};
use crate::parser::parse;
use crate::safety::SafetyLevel;
use crate::sema::{Resolution, resolve};
use crate::token::Token;
use crate::types::{Types, check};
use safec_ir::analysis::Conclusion;
use safec_ir::ir::{FuncId, TranslationUnit};
use safec_ir::memory::{self};
use safec_ir::nullability::{self};
use safec_ir::print::dump_ir;
use safec_ir::source::{FileId, FileName, SourceMap};
use safec_ir::target::Integer;

/// Everything one run of the compiler produced.
///
/// Not `Compilation`, which both reference compilers already use and for the
/// opposite thing: clang builds one before running anything, as the list of
/// jobs it is about to perform, and rustc uses it for `{Stop, Continue}`. This
/// is the far end of a run, and the other name is worth leaving free for the
/// job list that `--emit object` and `--emit executable` will eventually want.
#[derive(Debug)]
pub struct Compiled {
    /// Every file that was read.
    pub sources: SourceMap,
    /// Everything the compiler had to say about them.
    pub diagnostics: DiagnosticSink,
    /// What `--emit` asked for, if the pipeline reaches that far.
    ///
    /// `None` means the run was refused before it read anything: too many
    /// inputs for a kind that takes one, or a destination that is one of the
    /// inputs. Both are facts about the invocation, and the diagnostics say so.
    /// It does not mean nothing was compiled: a run that reported a lexical
    /// error still emits the tokens, because they are still what was asked for
    /// and they are still worth reading.
    ///
    /// `--emit ast` answers differently, and `compile` says why: an input the
    /// scan reported on is not parsed, so it contributes no tree. A run of one
    /// such file produces `Some(empty)`.
    ///
    /// Bytes rather than text, because `--emit object` is not text. Every other
    /// kind is UTF-8 this compiler wrote and a caller reading one back can say
    /// so; an object is what `clang` handed over and has no encoding at all.
    ///
    /// Bytes carry no file mode, and one artifact needs one. The mode is put on
    /// where [`run_compiler`] writes, which is the only place that knows there
    /// is a file at all.
    pub artifact: Option<Vec<u8>>,
}

/// What a run amounted to.
///
/// Separate from [`ExitCode`], which is the process's answer rather than the
/// compiler's: it can be built but never read back, so a caller that is not
/// `main` can do nothing with one. This says what happened, and `main` decides
/// what to exit with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing reported an error.
    Succeeded,
    /// Something did, and the diagnostics say what.
    Failed,
}

impl From<Outcome> for ExitCode {
    fn from(outcome: Outcome) -> Self {
        match outcome {
            Outcome::Succeeded => ExitCode::SUCCESS,
            Outcome::Failed => ExitCode::FAILURE,
        }
    }
}

/// Run the compiler over `options`, rendering nothing.
///
/// Split from [`run_compiler`] so that a test can inspect what was reported
/// instead of reading it back out of a stream.
///
/// **Nothing here writes to a path the user named.** This function reads files
/// and spawns children that answer on a pipe; [`run_compiler`] is the only
/// thing that writes, which is what makes one rule about what a failed run
/// leaves behind enough. `--emit executable` is where that is tempting to
/// break, because a linker wants to write its own output, and breaking it means
/// moving that rule rather than losing it.
///
/// **The gate for per-input work is on the input, not on the run.**
/// [`DiagnosticSink::has_errors`] answers whether anything in the whole run
/// failed, so gating a phase on it would let a typo in `a.c` decide that `b.c`
/// is never looked at, and a user with two broken files would fix them one run
/// at a time. The loop below takes an error count either side of each input's
/// own work instead, and the run-level answer is for what genuinely spans the
/// run: the outcome, and linking, which `finish` does once the loop is done.
pub fn compile(options: &Options) -> Compiled {
    // The one place a sink is built from the options, so that no part of the
    // compiler can invent its own policy. See ADR-0001.
    let mut diagnostics = DiagnosticSink::with_policy(Policy::from(options));
    let mut sources = SourceMap::new();

    if options.inputs.is_empty() {
        // Not reachable through the command line, which requires an input, but
        // `Options` is built directly by everything that is not the parser.
        // Such a run would otherwise be told only that the pipeline is missing,
        // which is true and beside the point.
        diagnostics.report(Diagnostic::error("no input files"));
    }

    // Once per run and before any input is read, because a check that did not
    // run has nowhere to say so from: the safety gate is inside `lowered`, which
    // two artifacts never reach and four levels have nothing behind. See
    // ADR-0035.
    if let Some(diagnostic) = undelivered(options) {
        diagnostics.report(diagnostic);
    }

    // A module is one translation unit, and this compiler makes one artifact
    // per run. Appending a second unit to the first is not a module with
    // duplicates in it: `int f(int);` in one input and `int f(int x) { ... }`
    // in another gives a `declare` beside a `define` of one name, and LLVM
    // refuses to parse that. Ordinary, correct C, so the answer is to say no
    // rather than to write something nothing can read.
    //
    // No code, because this is about the invocation rather than about a
    // program. `--emit object` is where one artifact per input arrives, and
    // where this stops being a refusal.
    //
    // The question is asked of the kind rather than answered by a list here,
    // so that a kind added to `EmitKind` has to say which half it is in before
    // the workspace builds. See `EmitKind::spans_inputs`.
    if !options.emit.spans_inputs() && options.inputs.len() > 1 {
        diagnostics.report(
            Diagnostic::error(format!(
                "`--emit {}` takes one input at a time, and this run was given {}",
                options.emit.spelling(),
                options.inputs.len()
            ))
            .with_note(
                "a module is one translation unit, and two appended into one is not a module",
            )
            .with_note("run it once per input"),
        );
        return Compiled {
            sources,
            diagnostics,
            artifact: None,
        };
    }

    // Two of the kinds have a destination when nobody named one, and this
    // compiler does not look at an extension to decide what an input is. So
    // `safec --emit object x.o` derives the name it was handed, and
    // `safec --emit executable a.exe` is handed the name it would have chosen,
    // and writing either would destroy the source. Said before anything is
    // read, because the answer does not depend on what the file turns out to
    // contain.
    //
    // `-o` naming an input is the same collision said out loud and is refused
    // the same way: a user who meant it can still say so with a different name.
    //
    // Two spellings of one path are two paths here, as they are for a repeated
    // input above, so `-o ./x.o` on `x.o` is not caught. Deciding when two
    // paths name one file belongs to the source map, and `#include` is what
    // will make it worth deciding.
    if let Some(path) = destination(options) {
        if options.inputs.contains(&path) {
            // Where the name came from, because the two answers need different
            // advice: a user who wrote `-o` is not helped by being told to
            // write `-o`, and a program is not named after its input at all.
            let came_from = if options.output.is_some() {
                "`-o` named it, and it is one of the inputs".to_owned()
            } else {
                format!(
                    "`--emit {}` is named `{}` when `-o` does not say otherwise",
                    options.emit.spelling(),
                    path.display()
                )
            };

            diagnostics.report(
                Diagnostic::error(format!(
                    "`--emit {}` would write over its own input `{}`",
                    options.emit.spelling(),
                    path.display()
                ))
                .with_note(came_from)
                .with_note("give it a name that is not an input"),
            );
            return Compiled {
                sources,
                diagnostics,
                artifact: None,
            };
        }
    }

    let mut seen = HashSet::new();
    let mut loaded = Vec::new();
    for input in &options.inputs {
        // The same path twice is one translation unit, not two: reading it
        // twice would report everything in it twice. `cc` answers differently,
        // compiling the file twice so that the link fails on a duplicate
        // symbol, which is a divergence taken on purpose: saying everything a
        // user has to read twice is the worse of the two answers for something
        // whose output is diagnostics.
        //
        // `Path` compares by component rather than by spelling, so `dir/./a.c`
        // is caught along with an identical spelling. `a.c` and `./a.c` are
        // still two, and so are `main.c` and `MAIN.C` on a filesystem that says
        // otherwise. Deciding when two paths name one file belongs to the
        // source map, and `#include` is what will make it worth deciding.
        if !seen.insert(input.as_path()) {
            continue;
        }

        // Every input is attempted. Reporting the first bad path and stopping
        // would make a user with three of them run the compiler three times.
        match sources.load(input) {
            Ok(file) => loaded.push(file),
            Err(error) => diagnostics.report(load_failure(input, &error)),
        }
    }

    // What the pipeline can produce, decided once, for every kind there is.
    //
    // A `match` rather than a comparison on `EmitKind`'s pipeline order. A
    // comparison answers "beyond the last stage that exists" and never "before
    // the first one", and a pipeline grows at both ends: C runs the
    // preprocessor before the lexer, so `--emit preprocessed` belongs above
    // `Tokens` in that enum, and a `>` gate let such a run exit successfully
    // having produced nothing and said nothing.
    //
    // Exhaustive, so a kind added anywhere is a compile error until somebody
    // says what it produces. Every kind produces something, which is why this
    // answers an `Emitted` rather than an `Option<Emitted>`: there is no arm
    // left for a kind the pipeline cannot reach, so a new one has to be built
    // rather than reported. `Compiled::artifact` is still an option, because a
    // run can be refused above before it reaches here.
    let mut artifact = match options.emit {
        EmitKind::Tokens => Emitted::Tokens(String::new()),
        EmitKind::Ast => Emitted::Ast(String::new()),
        EmitKind::SafetyIr => Emitted::SafetyIr(String::new()),
        EmitKind::Hatches => Emitted::Hatches(String::new()),
        EmitKind::LlvmIr => Emitted::LlvmIr(String::new()),
        EmitKind::Object => Emitted::Object(Vec::new()),
        EmitKind::Executable => Emitted::Program {
            modules: Vec::new(),
            bytes: Vec::new(),
        },
    };
    for &file in &loaded {
        // `file_owned` rather than `file`: the scan holds its text for longer
        // than a statement, and adds to the map while doing so once `#include`
        // lands. See ADR-0005.
        let source = sources.file_owned(file);

        // Taken before the scan rather than before the parse, so that it
        // answers "was this input read whole" rather than "did it parse". A
        // directive is the case that separates the two: the lexer reports it,
        // says in its own note that the line is not compiled, and hands the
        // parser a token it skips without complaint, so a gate placed after
        // the scan is open on a file whose declarations were dropped. Every
        // macro name in it is then a name nothing declares.
        let read_whole = diagnostics.error_count();
        let tokens = lex(file, &source, &mut diagnostics);

        // Every input appends to one artifact, and every line names its file,
        // so `--emit tokens a.c b.c` reads as one dump rather than needing two
        // destinations.
        match &mut artifact {
            Emitted::Tokens(out) => dump_tokens(&source, &tokens, out),
            Emitted::Ast(out) => {
                let Some(analysed) = analysed(
                    &sources,
                    file,
                    &tokens,
                    read_whole,
                    options.target.int(),
                    &mut diagnostics,
                ) else {
                    continue;
                };
                dump_ast(&sources, &analysed.ast, out);
            }
            Emitted::SafetyIr(out) => {
                let Some((unit, _)) = lowered(
                    &sources,
                    file,
                    &tokens,
                    read_whole,
                    options,
                    &mut diagnostics,
                ) else {
                    continue;
                };
                dump_ir(&sources, &unit, out);
            }
            Emitted::Hatches(out) => {
                let Some((unit, hatched)) = lowered(
                    &sources,
                    file,
                    &tokens,
                    read_whole,
                    options,
                    &mut diagnostics,
                ) else {
                    continue;
                };
                dump_hatches(&sources, &unit, &hatched, out);
            }
            Emitted::LlvmIr(out) => {
                let Some((unit, _)) = lowered(
                    &sources,
                    file,
                    &tokens,
                    read_whole,
                    options,
                    &mut diagnostics,
                ) else {
                    continue;
                };
                out.push_str(&module(&sources, &unit, options.target, &mut diagnostics));
            }
            Emitted::Object(out) => {
                let Some((unit, _)) = lowered(
                    &sources,
                    file,
                    &tokens,
                    read_whole,
                    options,
                    &mut diagnostics,
                ) else {
                    continue;
                };
                // Nothing reaches `clang` from a run that read nothing, because
                // this arm is what builds the module and an input that could
                // not be read never gets here. So the artifact stays empty and
                // `run_compiler` declines to write an empty one over a path the
                // user gave. That is the same rule the header's placement buys
                // `--emit llvm-ir`, and it is worth saying because the failure
                // it avoids is quieter here: `clang` answers a valid, useless
                // object for a module with no functions in it.
                let module = module(&sources, &unit, options.target, &mut diagnostics);
                if said_something(&diagnostics, read_whole) {
                    continue;
                }

                match assembled(&module, options.target) {
                    Ok(object) => *out = object,
                    Err(why) => diagnostics.report(clang_failure(options.emit, "an object", &why)),
                }
            }
            Emitted::Program { modules, .. } => {
                let Some((unit, _)) = lowered(
                    &sources,
                    file,
                    &tokens,
                    read_whole,
                    options,
                    &mut diagnostics,
                ) else {
                    continue;
                };
                // **Nothing is linked here.** A program is one artifact out
                // of every input, so the only thing this arm can do about one
                // of them is keep it, and `finish` is where they become a
                // program.
                //
                // No gate on what this input reported, unlike the arm above:
                // `finish` declines to link a run that reported anything at
                // all, so a module kept from an input that failed is a module
                // nothing reads. A gate here would be a second answer to a
                // question already answered, and the arm above needs its own
                // because nothing downstream of it asks again.
                let text = module(&sources, &unit, options.target, &mut diagnostics);

                modules.push(Module {
                    named: stem(sources.file(file).name()),
                    text,
                });
            }
        }
    }

    finish(&mut artifact, options, &mut diagnostics);

    // Where the artifact goes is `run_compiler`'s and not this function's: what
    // an artifact *is* does not depend on where it is written, and a test that
    // wants to read one back should not have to give it a path to get it.
    //
    Compiled {
        sources,
        diagnostics,
        artifact: Some(artifact.into_bytes()),
    }
}

/// The artifact being built, and which one it is.
///
/// The gate that decides whether to build anything runs once, before the
/// inputs, and the dump runs once per input, so the two are in different
/// places. Naming the artifact keeps them one decision rather than two matches
/// on `EmitKind` that have to agree. The two-copy version of this has already
/// cost something here: a gate written as two
/// comparisons left `--emit preprocessed` answered by neither, and the compiler
/// exited zero having written nothing to either stream.
enum Emitted {
    /// What `--emit tokens` asked for.
    Tokens(String),
    /// What `--emit ast` asked for.
    Ast(String),
    /// What `--emit safety-ir` asked for.
    SafetyIr(String),
    /// What `--emit hatches` asked for.
    Hatches(String),
    /// What `--emit llvm-ir` asked for.
    LlvmIr(String),
    /// What `--emit object` asked for.
    ///
    /// The one kind that is not text, and the reason [`Compiled::artifact`] is
    /// bytes. `clang` made these and this compiler only carries them.
    Object(Vec<u8>),
    /// What `--emit executable` asked for, and what it is being made of.
    ///
    /// Bytes for the same reason as an object, and the one artifact that needs
    /// a mode as well: see [`EmitKind::is_a_program`] and where
    /// [`run_compiler`] writes.
    ///
    /// **The only kind that is not finished per input**, so it is the only one
    /// that carries work in progress. The modules live here rather than in a
    /// local beside the artifact for the reason this enum exists at all: a
    /// second place that only one kind fills is a second thing that has to
    /// agree with the first, and `match options.emit` cannot see a local.
    Program {
        modules: Vec<Module>,
        bytes: Vec<u8>,
    },
}

impl Emitted {
    fn into_bytes(self) -> Vec<u8> {
        match self {
            Self::Tokens(text)
            | Self::Ast(text)
            | Self::SafetyIr(text)
            | Self::Hatches(text)
            | Self::LlvmIr(text) => text.into_bytes(),
            Self::Object(bytes) | Self::Program { bytes, .. } => bytes,
        }
    }
}

/// One input's tree, and what the stages after the parser could say about it.
struct Analysed {
    ast: Ast,
    /// Absent where the parse reported something, since a resolution drawn
    /// from a tree the parser gave up on is a claim about a program nobody
    /// wrote.
    typed: Option<(Resolution, Types)>,
}

/// Read one input as far as the frontend goes, or nothing where the scan
/// reported about it.
///
/// The two gates live here rather than in each `--emit` arm, because they are
/// one decision about one input. The `Emitted` enum below is the same shape
/// for the same reason, and its doc comment carries what one decision spelled
/// in two places cost this compiler.
///
/// The first gate is this input read whole. `has_errors` answers for the whole
/// run, so a count taken either side of the work on one file is what says
/// whether that file came through it. A directive is the case that separates
/// "read whole" from "parsed": the lexer reports it, says in its own note that
/// the line is not compiled, and hands the parser a token it skips without
/// complaint, so a gate placed after the scan is open on a file whose
/// declarations were dropped.
///
/// The second is the parse. What the parser says about a token stream with a
/// hole in it is a second diagnostic about the first one's problem, and the
/// resolution and the types are what would draw one.
fn analysed(
    sources: &SourceMap,
    file: FileId,
    tokens: &[Token],
    read_whole: usize,
    int_range: Integer,
    diagnostics: &mut DiagnosticSink,
) -> Option<Analysed> {
    if diagnostics.error_count() != read_whole {
        return None;
    }

    let mut ast = parse(file, tokens, diagnostics);
    if diagnostics.error_count() != read_whole {
        return Some(Analysed { ast, typed: None });
    }

    let resolution = resolve(sources, &ast, diagnostics);
    let types = check(sources, &mut ast, &resolution, int_range, diagnostics);

    Some(Analysed {
        ast,
        typed: Some((resolution, types)),
    })
}

/// One input's IR, or nothing where the frontend could not get that far.
///
/// Shared by the two `--emit` kinds that read the IR rather than duplicated in
/// each, for the reason [`Emitted`] gives for existing at all: two copies of a
/// gate are two things that have to agree, and when they stop, a case is
/// answered by neither.
fn lowered(
    sources: &SourceMap,
    file: FileId,
    tokens: &[Token],
    read_whole: usize,
    options: &Options,
    diagnostics: &mut DiagnosticSink,
) -> Option<(TranslationUnit, Vec<Hatched>)> {
    let analysed = analysed(
        sources,
        file,
        tokens,
        read_whole,
        options.target.int(),
        diagnostics,
    )?;
    // Nothing to lower from a tree whose names and types are not known: the IR
    // would be built out of what the frontend could not work out, and the
    // lowering says so about each piece rather than saying it once here.
    let (resolution, types) = analysed.typed.as_ref()?;

    let unit = lower(
        sources,
        &analysed.ast,
        resolution,
        types,
        options.target,
        diagnostics,
    );

    // The one place a safety check runs, because this is the one place the IR
    // is built: every artifact that has one reaches it through here, so there
    // is no arm for a check to be left out of.
    //
    // A comparison rather than a match, which on an ordered enum usually
    // answers one side of the question, and is right here for the reason
    // `SafetyLevel`
    // gives in its own doc comment: the levels are cumulative and ordered on
    // purpose, so a level above this one runs this check too, and there is
    // nothing below `Off`. `EmitKind` is the other case and deliberately has no
    // `Ord` at all.
    let mut hatched = Vec::new();
    if options.safety >= SafetyLevel::Memory {
        for finding in memory::findings(sources, &unit) {
            if let Some(diagnostic) = memory_finding(&finding) {
                let concluded = Concluded {
                    function: finding.function,
                    conclusion: finding.conclusion,
                    diagnostic,
                };
                route(&unit, concluded, &mut hatched, diagnostics);
            }
        }
        // A second analysis rather than a second question for the first: one
        // walk over one lattice is what makes the double free and the use after
        // free one check, and whether a pointer can be null is answered by a
        // lattice that shares nothing with theirs.
        for finding in nullability::findings(&unit) {
            if let Some(diagnostic) = nullability_finding(&finding) {
                let concluded = Concluded {
                    function: finding.function,
                    conclusion: finding.conclusion,
                    diagnostic,
                };
                route(&unit, concluded, &mut hatched, diagnostics);
            }
        }
    }

    Some((unit, hatched))
}

/// What a check concluded, as a diagnostic, and the function it was concluded
/// in.
///
/// One type for both checks, so that [`route`] is one rule rather than a copy
/// in each loop, since two copies of one rule drift apart.
#[derive(Clone)]
struct Concluded {
    function: FuncId,
    conclusion: Conclusion,
    diagnostic: Diagnostic,
}

/// A conclusion drawn inside a hatch, kept for `--emit hatches`.
type Hatched = Concluded;

/// Report a conclusion, or keep it for the listing where it was drawn inside a
/// hatch and could not be proved.
///
/// **What a hatch changes is what a conclusion is about, not what it
/// concluded.** Every conclusion inside a hatch is kept for the listing,
/// whatever it was. One that could not be proved is not reported as well,
/// because it is a statement about the hatch and not about the program. One
/// that was proved is reported as it would be anywhere: a hatch is for what
/// cannot be proved, and a program proved undefined on some execution is not
/// that. See ADR-0038.
///
/// Every conclusion written out rather than a comparison, so that a fourth one
/// is answered for here by `error[E0004]`.
fn route(
    unit: &TranslationUnit,
    concluded: Concluded,
    hatched: &mut Vec<Hatched>,
    diagnostics: &mut DiagnosticSink,
) {
    let in_a_hatch = unit.function(concluded.function).hatch();
    let reported = match concluded.conclusion {
        Conclusion::Unknown => !in_a_hatch,
        Conclusion::Safe | Conclusion::Unsafe => true,
    };

    if in_a_hatch {
        hatched.push(concluded.clone());
    }
    if reported {
        diagnostics.report(concluded.diagnostic);
    }
}

/// What a run asked for above what it can deliver, as what a user reads.
///
/// `None` where the run delivers what it asked for, which is every run that
/// named no level: [`Cli::into_options`] resolves an unnamed one to what the
/// artifact can carry, so asked and delivered agree by construction and a user
/// who configured nothing is told nothing.
///
/// [`Cli::into_options`]: crate::cli::Cli::into_options
///
/// **Built with [`Diagnostic::concluded`] rather than [`Diagnostic::error`]**,
/// because a check that never ran established nothing, and that is what an
/// unproven conclusion means. The severity is then the sink's, taken once from
/// the policy, so `--allow-unknown` governs this the way it governs every other
/// unproven result rather than through a second reading of the policy here. See
/// ADR-0035, and ADR-0001 for why that place is the sink.
///
/// **No code and no label.** `docs/diagnostics.md` gives a code to a class of
/// program a reader can search for, and this is a fact about the invocation:
/// there is no program to point at, and a run refused here may never read one.
///
/// **Every string here names a command-line flag, and that is an assumption
/// rather than a rule.** It holds while the command line is the only front door,
/// which `docs/diagnostics.md` licenses for a fact about the invocation. The
/// Clang adapter builds an [`Options`] with no command line behind it:
/// `EmitKind::default_safety` is why it cannot reach this by defaulting, but a
/// caller that sets the pair itself is told to change flags it does not have.
/// Whoever writes that adapter decides what it reads instead, and there is one
/// caller today, so nothing is abstracted for it here.
///
/// **Two axes, twice over.** The gate is one of them and what it says is the
/// other, and the second was got wrong here first.
///
/// Mutation: compare `options.safety` against `SafetyLevel::IMPLEMENTED` instead
/// of against [`Options::delivered`], which drops the artifact half of the gate.
/// `an_artifact_that_stops_before_the_ir_delivers_no_checks` fails and
/// `a_level_with_no_checks_behind_it_is_not_delivered` stays green.
///
/// Mutation: choose the note and the remedy with a single `if
/// options.emit.reaches_the_ir()`, which is how this shipped to review. Then a
/// run whose level *and* artifact both fall short says one of the two and its
/// remedy sends the reader to `--emit safety-ir`, which is refused again for the
/// other reason. `both_reasons_a_level_can_go_undelivered_are_said_at_once`
/// fails and the two single-cause cases stay green.
///
/// Both are the shape of the worst defect this project has had,
/// and the second is what guarding against it in one place and not the next
/// costs: ADR-0034 makes a remedy the change that would make the program
/// compile, so a remedy answering one of two causes is a promise the run breaks.
fn undelivered(options: &Options) -> Option<Diagnostic> {
    let asked = options.safety;
    let delivered = options.delivered();
    if delivered == asked {
        return None;
    }

    // **Both causes can hold at once, so both are said.** The artifact stopping
    // before the IR and the level having nothing behind it are independent, and
    // `--emit ast --safety strict` is both. An `if/else` here said whichever was
    // written first, which is the two-axis mistake this function's own doc
    // comment warns about, one level over from the gate that avoids it.
    let stops_short = !options.emit.reaches_the_ir();
    let unimplemented = asked > SafetyLevel::IMPLEMENTED;
    let mut notes = Vec::new();
    if stops_short {
        notes.push(format!(
            "`--emit {}` stops before the Safety IR every check reads",
            options.emit.spelling()
        ));
    }
    if unimplemented {
        notes.push(format!(
            "`{}` is the highest level with checks behind it",
            SafetyLevel::IMPLEMENTED.spelling()
        ));
    }
    debug_assert!(
        !notes.is_empty(),
        "a run delivers less than it asked for only through one of these two"
    );

    // **The remedy is worked out from what this run delivers, not from whichever
    // cause was written first.** ADR-0034 makes a remedy the change that would
    // make the program compile, so a remedy answering one of two causes is a
    // promise this run breaks: `--emit ast --safety strict` used to be told to
    // ask for `--emit safety-ir`, which is refused again for the other reason.
    let mut ways = vec![format!("ask for `--safety {}`", delivered.spelling())];
    if stops_short {
        // Raising the artifact is the other way, and the level has to come with
        // it: an artifact that reaches the IR still cannot deliver a level
        // nothing implements.
        ways.push(if unimplemented {
            format!(
                "`--emit safety-ir --safety {}`",
                SafetyLevel::IMPLEMENTED.spelling()
            )
        } else {
            "`--emit safety-ir`".to_owned()
        });
    }
    // Not offered to a run that already gave it, which is advice to do what has
    // been done, and not offered beside the level that is defined as leaving
    // nothing unknown, where `Cli::check` refuses the pair. Following a remedy
    // into an argument conflict is the same broken promise as following one into
    // this diagnostic again.
    if !options.allow_unknown && asked < SafetyLevel::Strict {
        ways.push("`--allow-unknown` while a program is being migrated".to_owned());
    }

    let mut diagnostic = Diagnostic::concluded(
        Conclusion::Unknown,
        format!(
            "`--safety {}` asks for more than this run delivers",
            asked.spelling()
        ),
        Remedy::new(ways.join(", or ")),
    )?;
    for note in notes {
        diagnostic = diagnostic.with_note(note);
    }
    Some(diagnostic)
}

/// Where an artifact is written, which is not always what `-o` said.
///
/// A program is named after nothing: `a.out`, or `a.exe` where a program
/// carries an extension, which is what `cc` and `clang` do and which
/// `Target::program_name` answers for the machine the run named rather than for
/// the host. Not the input's stem, which is `rustc`'s answer: a script that
/// calls `safec` where it meant `cc` should find the file it was expecting.
///
/// An object is the other kind with a default. `cc` and `rustc` both write
/// `<stem>.o` into the *current* directory whatever path the input came from,
/// measured rather than recalled: `clang -c sub/deep.c` leaves `./deep.o` and
/// `rustc --emit obj sub/r.rs` leaves `./r.o`, and `clang` writes `.o` on this
/// host rather than `.obj`. A user typing `safec --emit object a.c` means the
/// same thing by it, and the alternative is a stream, which for an object is
/// bytes into a terminal.
///
/// The stem rather than `Path::with_extension`, which keeps the directory the
/// input came from and would leave `sub/deep.o` beside `sub/deep.c`. Both spell
/// the name the same way, `a.tar.c` included; where they differ is where the
/// file lands, and `cc` and `rustc` both land it here.
///
/// Every other kind answers what `-o` said, so `Options` keeps meaning what was
/// asked for and the default lives in one place.
fn destination(options: &Options) -> Option<PathBuf> {
    if let Some(path) = &options.output {
        return Some(path.clone());
    }
    // Exhaustive, and not two `matches!`, because a gate spelled as two
    // conditions can leave a case answered by neither: a
    // kind added and forgotten in a list is answered by no arm and goes to the
    // stream, which for a kind with a default name is silent and wrong.
    match options.emit {
        EmitKind::Tokens
        | EmitKind::Ast
        | EmitKind::SafetyIr
        | EmitKind::Hatches
        | EmitKind::LlvmIr => None,
        EmitKind::Object => {
            let mut named = options.inputs.first()?.file_stem()?.to_os_string();
            named.push(".o");
            Some(PathBuf::from(named))
        }
        EmitKind::Executable => Some(PathBuf::from(options.target.program_name())),
    }
}

/// Everything that is about the run rather than about one input.
///
/// The loop in [`compile`] is gated on each input, for the reason that function
/// gives: a typo in `a.c` must not decide that `b.c` is never looked at. This is
/// the other half, and it is gated on the run, because a program made of the
/// inputs that happened to compile is not the program that was asked for. That
/// distinction is why the two halves are separate at all, and `compile`'s own
/// comment named linking as the thing that would want the second one.
///
/// Only a program is made here. Every other kind finishes an artifact per input
/// and has nothing left to do, which is what the early return says.
///
/// **Nothing here can point at source.** It holds modules, which are backend
/// text, and no source map, so every diagnostic it makes is a sentence without
/// a span. That is enough for what a linker says, and not enough for anything
/// that wants to blame one of the inputs: whoever needs that keeps the
/// translation unit rather than its text.
fn finish(artifact: &mut Emitted, options: &Options, diagnostics: &mut DiagnosticSink) {
    let Emitted::Program { modules, bytes } = artifact else {
        return;
    };

    // Nothing is linked for a run that reported anything, and there is no
    // second condition: a run with no modules is a run whose inputs all
    // reported, or one that was given none, and both of those have said so
    // already. Spawning here would answer a linker's words about an empty
    // program on top of the reason the user already has.
    //
    // **This is the gate for a program, and the only one.** The per-input arm
    // keeps whatever it built, because a module from an input that reported is
    // one nothing here will read.
    if diagnostics.has_errors() {
        return;
    }

    match linked(modules, options.target) {
        Ok(program) => *bytes = program,
        Err(why) => diagnostics.report(link_failure(options, &why)),
    }
}

/// What to call a file made out of this one.
///
/// The stem, because the extension belongs to what the file held rather than to
/// what is being made out of it, and the name rather than the path, because a
/// name is what a linker quotes back. A file with no name of its own on disk
/// keeps the one it was given, angle brackets and all, since nothing but a
/// test builds one.
fn stem(name: &FileName) -> String {
    match name {
        FileName::Real(path) => path
            .file_stem()
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
            .into_owned(),
        FileName::Virtual(named) => named.clone(),
    }
}

/// Whether this input's own work reported anything.
///
/// **Nothing goes to a tool from a unit that is not what the source said.** A
/// type the lowering cannot hold and a shape the backend cannot write both
/// leave a module with a function missing from it, which still assembles and
/// still links against nothing, so the run ends with the frontend's diagnostic
/// followed by a linker's exit code. The second one names another program and a
/// number, and there is nothing a user can do with it.
///
/// `before` is the error count taken either side of this input's own work, the
/// same one the parse is gated on. A run-level count would let a typo in `a.c`
/// decide that `b.c` is never compiled, which is what `compile`'s own comment
/// says not to do.
fn said_something(diagnostics: &DiagnosticSink, before: usize) -> bool {
    diagnostics.error_count() != before
}

/// Why a file could not be turned into source.
///
/// [`SourceMap::load`] reads and decodes in one step, so one error covers both,
/// and the two are worth telling apart. Saying a file cannot be read when it
/// reads perfectly and only the decoding failed points the user at paths and
/// permissions, and a C file with a Latin-1 or Shift-JIS byte in a comment is
/// ordinary in the code this compiler exists to accept.
fn load_failure(path: &Path, error: &io::Error) -> Diagnostic {
    if error.kind() == io::ErrorKind::InvalidData {
        Diagnostic::error(format!("`{}` is not valid UTF-8", path.display()))
            .with_note("safec reads source files as UTF-8")
            .with_note(error.to_string())
    } else {
        Diagnostic::error(format!("cannot read `{}`", path.display())).with_note(error.to_string())
    }
}

/// Run the compiler and write what it found to `report`.
///
/// Named the way rustc names its entry point rather than `run`, which the
/// Safety IR interpreter will want: once a program can be executed in process,
/// "run" means two different things and only one of them returns the outcome of
/// a compilation.
///
/// The outcome follows `cc`: a run that reported an error failed. Two belongs
/// to the argument parser, which uses it for an invocation it could not
/// understand, so nothing here returns it.
///
/// `report` is a parameter rather than `io::stderr()` so that a caller can read
/// what a run said without spawning a process. Colour is still resolved against
/// stderr, because that is where the binary sends this; a caller writing
/// somewhere else should ask for `Never` or `Always` rather than `Auto`.
///
/// `artifact` is the other stream, and it is separate because the two are
/// different kinds of thing. Diagnostics are what the compiler says; an
/// artifact is what it was asked to make. A caller reading one must not be
/// handed the other, which is why the binary sends them to stderr and stdout.
///
/// The report goes first, and it is the stream that this is about. If the
/// artifact is being piped into something that stops reading, the write fails,
/// and a user who loses the diagnostics as well learns nothing about why.
///
/// A path is the exception, and is written before the report rather than after
/// it: a path that cannot be written is itself a diagnostic, and after the
/// report there is no sink left to say it into.
///
/// # Errors
///
/// If the diagnostics could not be written. There is nothing left to report
/// that on, so the caller has only the outcome to say it with.
pub fn run_compiler(
    options: &Options,
    report: &mut impl io::Write,
    artifact: &mut impl io::Write,
) -> io::Result<Outcome> {
    let mut compiled = compile(options);

    // One `match` rather than two `if`s, so that the four combinations are
    // answered here and not by whichever condition happened to be written
    // first. That has cost something here: a gate
    // spelled as two comparisons left a case answered by neither, and the
    // compiler exited zero having produced nothing.
    //
    // A path is answered here, before the diagnostics are rendered, because a
    // path that cannot be written is one of them. The stream is answered after
    // the rendering instead, which is why this yields what to write rather than
    // writing it: see the ordering paragraph on this function.
    let written = destination(options);
    let to_stream = match (&written, &compiled.artifact) {
        // `fs::write` creates, truncates and writes in one call, so one failure
        // covers all three and the note says which it was.
        //
        // Nothing is written when a failed run produced an empty artifact. That
        // is not the same as producing nothing: `--emit tokens` answers
        // `Some("")` for an input it could not open, and writing it would
        // truncate whatever the path already held, leaving exactly the
        // zero-byte file newer than every source that the arm below exists to
        // avoid. An empty artifact from a run that reported nothing is a real
        // answer to an empty program, and is written.
        //
        // A failed run leaves no build product either, empty or not. A module
        // missing a function the backend refused still assembles, so without
        // this the run exits 1 having left a program with a function deleted
        // from it, newer than the source, for a build system to read as
        // finished. Which kinds are that sort of artifact is
        // `EmitKind::survives_an_error`.
        (Some(path), Some(emitted)) => {
            let unfinished = compiled.diagnostics.has_errors()
                && (emitted.is_empty() || !options.emit.survives_an_error());
            if !unfinished {
                // Only a file this run wrote is this run's to take away, and
                // what decides that is whether the open succeeded. `fs::write`
                // is `File::create` followed by `write_all` and answers one
                // `Err` for both, so asking the question means writing the two
                // out: the open is where a read-only file refuses, with nothing
                // of ours on disk and what is there still the user's, and after
                // the open the file has been truncated to nothing, so whatever
                // is there is this run's wreckage whoever created it.
                //
                // Asking beforehand whether the path existed answers the wrong
                // one of those. A write that fails partway through a file that
                // was already there is then left behind, which is a zero-byte
                // artifact newer than every source: exactly the thing the rule
                // above exists to avoid, arrived at through the other door.
                let written = fs::File::create(path)
                    .map_err(|error| (false, error))
                    .and_then(|mut file| {
                        // A program that was written and could not be made
                        // runnable is a build product too: the run is about to
                        // fail and there is a file on disk, newer than the
                        // source, that a build system reads as finished.
                        file.write_all(emitted)
                            .and_then(|()| make_runnable(path, options.emit.is_a_program()))
                            .map_err(|error| (true, error))
                    });

                if let Err((ours, error)) = written {
                    // Best effort, because whatever stopped the write can stop
                    // this too, and the diagnostic is the same either way.
                    //
                    // Nothing guards the taking away. A test can refuse the
                    // open, and one does; it cannot make a write or a
                    // `set_permissions` fail on a file this process has just
                    // created and owns, which would take `unsafe` or a second
                    // user. #181 records that rather than leaving the silence to
                    // be read as coverage.
                    if ours {
                        let _ = fs::remove_file(path);
                    }
                    compiled.diagnostics.report(write_failure(path, &error));
                }
            }
            None
        }
        // A run that produced nothing leaves no file, rather than an empty one
        // that a build system would read as newer than the source it came from.
        // The diagnostic saying so is `compile`'s and has already been made.
        (Some(_), None) => None,
        (None, emitted) => emitted.as_deref(),
    };

    Renderer::new(options.color).render_all(&compiled.sources, &compiled.diagnostics, report)?;

    if let Some(emitted) = to_stream {
        artifact.write_all(emitted)?;
    }

    Ok(if compiled.diagnostics.has_errors() {
        Outcome::Failed
    } else {
        Outcome::Succeeded
    })
}

/// Make the file the machine will run, where what was written is a program.
///
/// A verb, because this does something rather than answering something:
/// [`EmitKind::is_a_program`] beside it is the question.
///
/// An artifact is bytes and a mode is not one of them, so the one kind that has
/// to be runnable says so and this is where it is said. `fs::write` creates a
/// file the way `File::create` does, which on Unix is `0o666` before the
/// process's `umask`, so a program written by it is one nobody can run.
///
/// **An execute bit wherever there is a read bit**, taken from the mode the
/// file was created with rather than written down: `0o644` becomes `0o755` and
/// a user whose `umask` is `0o077` gets `0o700` rather than a program their
/// whole machine can run. Writing `0o755` here would decide that for them.
///
/// Windows has no such bit, which is why this is the one `cfg` in the driver:
/// a file there is runnable for being a file, and the name is what decides.
fn make_runnable(path: &Path, program: bool) -> io::Result<()> {
    if !program {
        return Ok(());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        let mut mode = fs::metadata(path)?.permissions();
        let read = mode.mode() & 0o444;
        mode.set_mode(mode.mode() | (read >> 2));
        fs::set_permissions(path, mode)?;
    }
    #[cfg(not(unix))]
    let _ = path;

    Ok(())
}

/// Why an artifact could not be written where it was asked for.
///
/// The shape [`load_failure`] has for the other end of the same question, and
/// carrying no code for the same reason: this is the driver saying something
/// about a path rather than about a program, and `docs/diagnostics.md`'s topics
/// are about programs.
fn write_failure(path: &Path, error: &io::Error) -> Diagnostic {
    Diagnostic::error(format!("cannot write `{}`", path.display())).with_note(error.to_string())
}

#[cfg(test)]
mod tests;
mod words;
use words::{memory_finding, nullability_finding};
mod backend;
use backend::{Module, assembled, clang_failure, link_failure, linked, module};
mod dumps;
use dumps::{dump_ast, dump_hatches, dump_tokens};
