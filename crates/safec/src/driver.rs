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

use std::collections::HashSet;
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::SystemTime;

use crate::ast::{
    Ast, Declaration, Expr, ExprId, InitDeclarator, Item, Parameters, Stmt, StmtId, Type, TypeId,
    spell_type,
};
use crate::cli::HOST_TRIPLE;
use crate::diagnostics::render::Renderer;
use crate::diagnostics::{Code, Diagnostic, DiagnosticSink, Label, Policy, Remedy};
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
use safec_ir::memory::{self, Kind, Unproven};
use safec_ir::nullability::{self, Asked};
use safec_ir::print::{dump_ir, dump_node, quoted, shown};
use safec_ir::source::{FileId, FileName, SourceFile, SourceMap, Span};
use safec_ir::target::{Integer, Target};
use safec_llvm::emit::Refusal;

// Code generation takes `SC08xx`, which `docs/diagnostics.md` allocates. One
// code for every shape of a refusal, for the reason `types.rs` gives for
// `MISMATCH`: what differs between them is the message, and a reader filtering
// on the code wants "the backend could not write this" rather than a list of
// the ways that can happen.
//
// One of the two diagnostics in this file that carry one. The rest are the
// driver saying something about a run or about the machine it is on, and this
// is the backend saying something about a program, which is the line
// `docs/diagnostics.md` draws. No count here: that document carries one, with
// the command that settles it, and two places counting the same thing is one
// place too many.
const BACKEND: Code = Code::new("SC0801");

/// A value freed where it may already have been freed.
///
/// `SC04xx` is the memory axis, reserved by `docs/diagnostics.md` before
/// anything could emit from it, and this is the first. It is built here rather
/// than where the check is because `safec_ir` cannot see a `Diagnostic` at all,
/// which is ADR-0011, and `BACKEND` above is the same arrangement for the same
/// reason.
const DOUBLE_FREE: Code = Code::new("SC0401");

/// A value read or written through a pointer after it was freed.
///
/// A code of its own rather than `DOUBLE_FREE`'s, because a reader filtering on
/// one wants the two apart: a double free is a mistake about ownership and this
/// is a mistake about lifetime, and the programs that produce them are different
/// programs. `docs/diagnostics.md`'s rule is that a code names a class of
/// program to search for, and these are two classes.
const USE_AFTER_FREE: Code = Code::new("SC0402");

/// A value read or written through a pointer that may be null.
///
/// A third code rather than a severity on one of the two above, because
/// `docs/diagnostics.md` makes a code a class of program to search for and this
/// is a third class: the other two are about a pointer that pointed somewhere
/// once, and this is about one that may never have.
const NULL_DEREFERENCE: Code = Code::new("SC0403");

/// A value freed through a pointer that is not the start of an allocation.
///
/// A code of its own rather than `DOUBLE_FREE`'s, because the fix is different:
/// a double free is a mistake about ownership, a use after free one about
/// lifetime, and this is a mistake about which value was handed to `free`. It
/// is also not `NULL_DEREFERENCE`'s, which is about a pointer that may point
/// nowhere, where this one points somewhere real that `free` does not take.
/// See ADR-0036.
const INTERIOR_FREE: Code = Code::new("SC0404");

/// A pointer that may be null passed to a parameter declared `_Nonnull`.
///
/// Not `NULL_DEREFERENCE`'s, though the question is the same one, because the
/// fix is at a different place: that one is fixed where the pointer is read,
/// and this one where it is handed over, or at the promise the parameter made.
/// See ADR-0037.
const NULL_ARGUMENT: Code = Code::new("SC0405");

/// A pointer to an allocation that was freed, handed back by a `return`.
///
/// Not `USE_AFTER_FREE`'s, because that code is a dereference and this is a
/// read with none, and because the fix is at the return or at the free rather
/// than at a read. The caller that would have dereferenced it believes it is
/// live and cannot see why it is not. See ADR-0041.
const RETURN_AFTER_FREE: Code = Code::new("SC0406");

/// A pointer to an allocation that was freed, handed to a call.
///
/// Not `USE_AFTER_FREE`'s, for `RETURN_AFTER_FREE`'s reasons: this is a read
/// with no dereference, and the fix is at the call or at the free. The callee
/// believes its parameter is live and cannot see why it is not. See ADR-0042.
const ARGUMENT_AFTER_FREE: Code = Code::new("SC0407");

/// What to change where this check stopped following a pointer.
///
/// **It names no cause, and that is the whole of its design.**
/// `Unproven::Lost` has five producers, which its own doc comment lists, and a
/// `memory::Finding` does not say which of them answered. A remedy naming one
/// would be right for some and false for the others: a conservative
/// over-approximation given a confident word. So this says only what is true
/// of all five, and tells the reader the one thing that matters here, which is
/// not to go hunting for a defect.
///
/// Shared by the three codes that can lose a pointer, because the fact is the
/// same fact. #213 carries the reason into the `Finding` and replaces this
/// with five.
const LOST_REMEDY: &str = "nothing here says the program is wrong: this check could no longer \
                           say which allocation this pointer holds";

/// What to change where C has not ordered the free against the use.
///
/// The only remedy here about something C decided rather than about what the
/// program meant, which is why `Unsequenced` is a row of its own rather than
/// sharing one with `Disagreement`. See ADR-0022, and the note attached below
/// for what the check did and did not find.
const UNSEQUENCED_REMEDY: &str = "put the free and this in separate statements, so that C orders \
                                  one before the other";

/// What to change where this check could not tell whether the free reached.
///
/// **The free is conditional because this check does not know there is one.**
/// `Unproven::Disagreement` has two causes: paths that disagree about a free,
/// where one exists, and a call this check cannot read that was handed the
/// pointer, where there may be none at all. Nothing in a `Finding` separates
/// them, and its `freed` span does not: `memory.rs` carries a free span only
/// where it also answers `Unproven::Unsequenced`, so a `Disagreement` always
/// arrives with `None` and the report carries no `freed here` label.
///
/// What stood here said `free it on every path or on none` unconditionally. On
/// a program with no `free` in it that is an instruction to add one, and adding
/// it turns the warning into a proved use after free: the corpus case
/// `an_unsequenced_use_before_an_opaque_call_is_reported` allocates, calls two
/// functions, frees nothing, and carried exactly that advice at exit 0. A
/// remedy is a claim in the way a label is, and that one claimed a free
/// this check never found.
///
/// `if it is freed` is what makes one sentence true of both causes while
/// keeping the action a reader of the first can take. Naming no free at all
/// would be safe the way `LOST_REMEDY` is safe, and would cost the reader whose
/// paths really do disagree the one thing they could have done about it.
const DISAGREEMENT_REMEDY: &str = "if it is freed, free it on every path or on none, and keep it \
                                   out of a call this check cannot read in between";

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

/// Every hatch in `unit`, and under each what the checks concluded inside it.
///
/// One line per hatch, at its name, and one line per conclusion, at its caret,
/// in the order a reader meets them in the file. Each conclusion says its
/// code, what was concluded, and its message. A hatch nothing was concluded
/// in is listed all the same, which is what makes this the count of them: at
/// `--safety off` no check runs and every hatch is listed with nothing under
/// it.
///
/// Every line begins with [`dump_node`], so a file's name is escaped the way it
/// is in every other artifact, and the function's name is the file's text and
/// is written with `{:?}`, because source text is content.
fn dump_hatches(
    sources: &SourceMap,
    unit: &TranslationUnit,
    hatched: &[Hatched],
    out: &mut String,
) {
    for id in unit.functions() {
        let function = unit.function(id);
        if !function.hatch() {
            continue;
        }

        dump_node(sources, "Hatch", function.name, 0, out);
        writeln!(out, " {:?}", quoted(sources, function.name))
            .expect("writing to a string cannot fail");

        let mut inside: Vec<&Hatched> = hatched
            .iter()
            .filter(|concluded| concluded.function == id)
            .collect();
        inside.sort_by_key(|concluded| {
            let at = caret(&concluded.diagnostic);
            (at.file().index(), at.start())
        });

        for concluded in inside {
            dump_node(sources, "Conclusion", caret(&concluded.diagnostic), 1, out);
            let code = concluded
                .diagnostic
                .code()
                .map_or(String::new(), |code| code.as_str().to_owned());
            // In `docs/safety-model.md`'s three words, so that the one a
            // reader does not see here yet reads alike when it arrives.
            let said = match concluded.conclusion {
                Conclusion::Safe => "safe",
                Conclusion::Unsafe => "unsafe",
                Conclusion::Unknown => "unknown",
            };
            writeln!(
                out,
                " {:?} {:?} {:?}",
                code,
                said,
                concluded.diagnostic.message()
            )
            .expect("writing to a string cannot fail");
        }
    }
}

/// Where a check's diagnostic puts its caret.
///
/// Every one `memory_finding` and `nullability_finding` build has a primary
/// label, because a conclusion is about a place in the program.
fn caret(diagnostic: &Diagnostic) -> Span {
    diagnostic
        .primary_label()
        .expect("a check's diagnostic points at what it concluded about")
        .span()
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

/// What a memory check concluded, as what a user reads.
///
/// The check names a conclusion and never reads the policy, so this does not
/// either: `Diagnostic::concluded` is the one place a conclusion becomes a
/// severity and `DiagnosticSink` is the one place the policy is applied.
/// ADR-0001 is why there is one of each.
///
/// The value is not named. `docs/safety-model.md` writes "use of freed value
/// `p`" and the IR holds no `p`: a local is a type and an index, which is #136.
/// The caret goes on the call or the use, so the quoted line above it shows
/// `free(p)` or `*p` and the reader finds the name in their own text.
fn memory_finding(finding: &memory::Finding) -> Option<Diagnostic> {
    // The label says only as much as the conclusion does, and says it in words
    // nothing else here uses. "freed again" asserts there was a first time,
    // which an unproven result does not know, and reusing the word the other
    // diagnostic spends on the *earlier, legitimate* free would have a reader
    // who learned that pair take the suspect for the safe one.
    //
    // **A reason names the analysis where the analysis is all there is.**
    // `Unproven::Lost` is this check having stopped following the
    // pointer, so nothing about the *program* was established and there is
    // nothing about the program to say. Giving those the words below put
    // `may free it again here` on a file with one `free` in it, and
    // `used here, perhaps after the free` on one with none. The other two
    // reasons each have a free behind them: a site the paths disagree about
    // was freed on one of them, and a site an opaque call was handed may have
    // been freed by it.
    //
    // The reason is not read beside `Unsafe`, where there is nothing
    // unproven, and each `Unknown` row names every reason rather than taking
    // `_`, so that a fourth cannot fall into a row written before it existed.
    // An arm that means two things at once reports the second as the first,
    // and this is that read forwards. Its limit is that `E0004` makes somebody
    // write an arm and does not make the arm right.
    // **A remedy is a claim like a label is**, so each of
    // these is written against what its row established and not against what
    // its words suggest. Two rows are worth saying out loud. `Lost` names no
    // cause because `Unproven::Lost` has five producers and a `Finding` does
    // not say which answered, so anything more specific would be right for some
    // of them and false for the rest; #213 carries the reason and replaces it
    // with five. `Disagreement` has two causes of its own, paths that disagree
    // and a call this check cannot read, so its remedy names what would let
    // this check conclude rather than which of the two happened.
    //
    // `Unsequenced` is split out of the group it shares its words with, and
    // only because its remedy differs: C decided that one, and what to change
    // is the statement boundary rather than the free. The message and the label
    // are written twice as a result, which is the same trade this function
    // already makes by naming every reason rather than taking `_`.
    let (code, message, label, remedy) = match (finding.kind, finding.conclusion, finding.unproven)
    {
        (Kind::DoubleFree, Conclusion::Unsafe, _) => (
            DOUBLE_FREE,
            "this frees a value that was freed already",
            "freed again here",
            "remove one of the two frees, or take this one off the path that reaches the first",
        ),
        (Kind::DoubleFree, Conclusion::Unknown, Some(Unproven::Lost)) => (
            DOUBLE_FREE,
            "this frees a pointer this check stopped following",
            "this check cannot say what this points at",
            LOST_REMEDY,
        ),
        // **`Unsequenced` is split out below and not here**, which is the one
        // place the two kinds are shaped differently. No corpus case reaches a
        // double free that is unproven for that reason, and two written to try
        // were reported as proved double frees instead:
        // `(free(p), 0) + (free(p), 0)` and `h((free(p), 0), (free(p), 0))`.
        // That is what was measured, rather than a claim that none exists. It
        // reads as the shape of the question: ADR-0023 carries a *read*
        // forwards to the free it is unordered against, and a second free is
        // the same defect whichever of the two runs first, so an open order
        // takes nothing away from it.
        //
        // A row here would therefore be one nothing can break, and the group
        // keeps the remedy it had. `DISAGREEMENT_REMEDY` asks for the free
        // conditionally, so it stays true of a double free that arrives this
        // way if one ever does.
        // `Unproven::Offset` is here and in the `UseAfterFree` row below
        // although neither kind can carry it: `memory::report::interior` is the
        // only producer and it builds only an `InteriorFree`. Named rather than
        // taken by `_`, for the reason above.
        (
            Kind::DoubleFree,
            Conclusion::Unknown,
            Some(Unproven::Disagreement | Unproven::Unsequenced | Unproven::Offset) | None,
        ) => (
            DOUBLE_FREE,
            "this may free a value that was freed already",
            "may free it again here",
            DISAGREEMENT_REMEDY,
        ),
        (Kind::UseAfterFree, Conclusion::Unsafe, _) => (
            USE_AFTER_FREE,
            "this uses a value after it was freed",
            "used here",
            "move the free after this use, or do not free here",
        ),
        (Kind::UseAfterFree, Conclusion::Unknown, Some(Unproven::Lost)) => (
            USE_AFTER_FREE,
            "this uses a pointer this check stopped following",
            "this check cannot say what this points at",
            LOST_REMEDY,
        ),
        (Kind::UseAfterFree, Conclusion::Unknown, Some(Unproven::Unsequenced)) => (
            USE_AFTER_FREE,
            "this may use a value after it was freed",
            "used here, perhaps after the free",
            UNSEQUENCED_REMEDY,
        ),
        (
            Kind::UseAfterFree,
            Conclusion::Unknown,
            Some(Unproven::Disagreement | Unproven::Offset) | None,
        ) => (
            USE_AFTER_FREE,
            "this may use a value after it was freed",
            "used here, perhaps after the free",
            DISAGREEMENT_REMEDY,
        ),
        (Kind::ReturnAfterFree, Conclusion::Unsafe, _) => (
            RETURN_AFTER_FREE,
            "this returns a pointer to an allocation that was freed",
            "returned here",
            "return a pointer that is still allocated, or do not free this one before returning it",
        ),
        (Kind::ReturnAfterFree, Conclusion::Unknown, Some(Unproven::Lost)) => (
            RETURN_AFTER_FREE,
            "this returns a pointer this check stopped following",
            "this check cannot say what this points at",
            LOST_REMEDY,
        ),
        // `Unsequenced` cannot arrive, because `memory::report::verdict`
        // answers that a `return` is ordered after every free in its
        // expression, and `Offset` cannot because only
        // `memory::report::interior` builds it. Named rather than taken by `_`,
        // for the reason the double-free rows give.
        (
            Kind::ReturnAfterFree,
            Conclusion::Unknown,
            Some(Unproven::Disagreement | Unproven::Unsequenced | Unproven::Offset) | None,
        ) => (
            RETURN_AFTER_FREE,
            "this may return a pointer to an allocation that was freed",
            "returned here, perhaps after the free",
            DISAGREEMENT_REMEDY,
        ),
        (Kind::ArgumentAfterFree, Conclusion::Unsafe, _) => (
            ARGUMENT_AFTER_FREE,
            "this passes a pointer to an allocation that was freed",
            "passed here",
            "pass a pointer that is still allocated, or do not free this one before passing it",
        ),
        (Kind::ArgumentAfterFree, Conclusion::Unknown, Some(Unproven::Lost)) => (
            ARGUMENT_AFTER_FREE,
            "this passes a pointer this check stopped following",
            "this check cannot say what this points at",
            LOST_REMEDY,
        ),
        (Kind::ArgumentAfterFree, Conclusion::Unknown, Some(Unproven::Unsequenced)) => (
            ARGUMENT_AFTER_FREE,
            "this may pass a pointer to an allocation that was freed",
            "passed here, perhaps after the free",
            UNSEQUENCED_REMEDY,
        ),
        // `Offset` cannot arrive, because only `memory::report::interior`
        // builds it. Named rather than taken by `_`, for the reason the
        // double-free rows give.
        (
            Kind::ArgumentAfterFree,
            Conclusion::Unknown,
            Some(Unproven::Disagreement | Unproven::Offset) | None,
        ) => (
            ARGUMENT_AFTER_FREE,
            "this may pass a pointer to an allocation that was freed",
            "passed here, perhaps after the free",
            DISAGREEMENT_REMEDY,
        ),
        // **Words of its own, under the same code**: what is passed is live,
        // and `a pointer to an allocation that was freed` would be false about
        // `use2(&a)`. The fix is the same, at the call or at the free. Only
        // `Disagreement` arrives, because `memory::known::Known::handed_below`
        // always answers `Reached::Partial` beside a site freed or unproven;
        // every other reason is named rather than taken by `_`, for the reason
        // the double-free rows give. The label says `perhaps` and the remedy
        // has `DISAGREEMENT_REMEDY`'s two halves because the site may be
        // unproven rather than freed: a free on one path, or a call this check
        // cannot read. See ADR-0042.
        (
            Kind::FreedBehindArgument,
            Conclusion::Unsafe | Conclusion::Unknown,
            Some(
                Unproven::Disagreement | Unproven::Lost | Unproven::Unsequenced | Unproven::Offset,
            )
            | None,
        ) => (
            ARGUMENT_AFTER_FREE,
            "this may pass a pointer to where a freed pointer is stored",
            "passed here, perhaps holding a freed pointer behind it",
            "set a pointer stored there to null once it is freed, free it on every path or on \
             none, and keep it out of a call this check cannot read before this one",
        ),
        (Kind::InteriorFree, Conclusion::Unsafe, _) => (
            INTERIOR_FREE,
            "this frees a pointer that is not the start of an allocation",
            "not the start of the allocation",
            "free the pointer the allocation was made with, and keep the offset in a variable \
             of its own",
        ),
        // **The remedy is conditional because the offset may really be
        // zero.** What this check could not evaluate is the offset, not the
        // program, and an instruction that holds only where the offset is not
        // zero has to say so: a remedy is a claim like a label is, which is
        // `DISAGREEMENT_REMEDY`'s own reason.
        //
        // Every reason named, although only `Offset` arrives: the other three
        // come out of `verdict`, which `memory::report::interior` does not
        // call.
        (
            Kind::InteriorFree,
            Conclusion::Unknown,
            Some(
                Unproven::Offset | Unproven::Lost | Unproven::Disagreement | Unproven::Unsequenced,
            )
            | None,
        ) => (
            INTERIOR_FREE,
            "this may free a pointer that is not the start of an allocation",
            "this offset may not be zero",
            "free the pointer the allocation was made with if this offset can be non-zero",
        ),
        // Neither check answers this and `Diagnostic::concluded` gives `None`
        // for it, so none of the four is read. Written out rather than `_` so
        // that a fourth conclusion has to be answered for here.
        (_, Conclusion::Safe, _) => (DOUBLE_FREE, "nothing", "nothing", "nothing"),
    };

    let mut diagnostic = Diagnostic::concluded(finding.conclusion, message, Remedy::new(remedy))?
        .with_code(code)
        .with_safety_level(SafetyLevel::Memory)
        .with_label(Label::primary(finding.at, label));

    // These two mean the same thing under either conclusion and under either
    // check, so they share their words where the primary does not. Each is
    // attached only where the check knows it: what makes a finding `Unknown` is
    // that the paths or the sites reaching it disagree, and there is then no
    // single place to point at.
    if let Some(freed) = finding.freed {
        diagnostic = diagnostic.with_label(Label::secondary(freed, "freed here"));
    }
    // **Why this one is unproven, because it is not the usual why.** Every
    // other `Unknown` here is the check having lost something; this is the
    // check having found nothing that orders the free first. Without the note
    // a reader sees two carets and a warning and has no way to tell which of
    // the two it is, and neither is something more of the same analysis would
    // fix. See ADR-0022.
    //
    // **It says what this check found and not what C decided**, because the
    // two are not the same: the order really is open in `*p + (free(p), 0)`,
    // and in `x = (free(p), *p)` C17 6.5.17 p2 settles it while this check
    // still records nothing, which is #178. A note claiming C left those
    // unsequenced would be false about the second.
    //
    // 6.5.2.2 p10 rather than 6.5 p3, which the first draft of this cited: p3
    // leaves subexpressions *unsequenced*, and unsequenced evaluations may
    // interleave. What makes a call one order or the other is p10's
    // indeterminate sequencing, and a free is a call.
    if finding.unproven == Some(Unproven::Unsequenced) {
        diagnostic = diagnostic.with_note(
            "C17 6.5.2.2 p10 leaves a call indeterminately sequenced with the rest of its \
             expression, and this check found nothing here that orders the free before this",
        );
    }
    if let Some(made) = finding.made {
        diagnostic = diagnostic.with_label(Label::secondary(made, "allocated here"));
    }

    Some(diagnostic)
}

/// What the nullability check concluded, as what a user reads.
///
/// The check names a conclusion and never reads the policy, so this does not
/// either, for `memory_finding`'s reason and ADR-0001's.
///
/// **Two rows per question.** The memory check's words differ by which of its
/// two questions was asked and by why an answer was unproven; there are two
/// questions here, a dereference and an argument to a `_Nonnull` parameter, and
/// one reason, which is that nothing established what the pointer holds. A
/// reason enum with one variant is a field nobody reads.
///
/// The value is not named, for `memory_finding`'s reason: the IR holds no `p`,
/// which is #136, and the caret's quoted line shows the reader their own text.
fn nullability_finding(finding: &nullability::Finding) -> Option<Diagnostic> {
    // The unproven row does not say "may be null" of the *pointer* and then
    // blame the dereference: what this check failed to establish is that the
    // pointer is not null, and a reader who is told the pointer may be null is
    // being told something was worked out about it. Nothing was.
    //
    // Every conclusion written out rather than `_`, so that a fourth has to be
    // answered for here. `Safe` is unreachable through `Nullness::concluded`
    // and `Diagnostic::concluded` gives `None` for it either way.
    // The unproven row asks for a test rather than for a value, because what
    // this check failed to establish is that the pointer is not null and a test
    // is what would establish it. Telling a reader to give it a value there
    // would assert that it has none, which is the row above's sentence and not
    // this one's, and a remedy is a claim like a label is.
    //
    // The argument rows follow the same rule, and their unsafe remedy offers
    // the promise as the other way out, because a parameter declared
    // `_Nonnull` that a caller has a reason to pass null to is a promise that
    // was wrong rather than a call that was.
    let (code, message, label, remedy) = match (finding.asked, finding.conclusion) {
        (Asked::Dereference, Conclusion::Unsafe) => (
            NULL_DEREFERENCE,
            "this dereferences a null pointer",
            "this is null when it is read through",
            "give this pointer a value before reading through it, or do not read through it here",
        ),
        (Asked::Dereference, Conclusion::Unknown) => (
            NULL_DEREFERENCE,
            "this may dereference a null pointer",
            "this check cannot say this is not null",
            "test this pointer against null before reading through it",
        ),
        (Asked::Argument { promise: _ }, Conclusion::Unsafe) => (
            NULL_ARGUMENT,
            "this passes a null pointer to a parameter declared `_Nonnull`",
            "this is null when it is passed",
            "pass a pointer to an object here, or remove `_Nonnull` from the parameter",
        ),
        (Asked::Argument { promise: _ }, Conclusion::Unknown) => (
            NULL_ARGUMENT,
            "this may pass a null pointer to a parameter declared `_Nonnull`",
            "this check cannot say this is not null",
            "test this pointer against null before passing it",
        ),
        (Asked::Dereference | Asked::Argument { promise: _ }, Conclusion::Safe) => {
            (NULL_DEREFERENCE, "nothing", "nothing", "nothing")
        }
    };

    let mut diagnostic = Diagnostic::concluded(finding.conclusion, message, Remedy::new(remedy))?
        .with_code(code)
        .with_safety_level(SafetyLevel::Memory)
        .with_label(Label::primary(finding.at, label));
    if let Asked::Argument { promise } = finding.asked {
        diagnostic = diagnostic.with_label(Label::secondary(
            promise,
            "the parameter is declared `_Nonnull` here",
        ));
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

/// The module one unit becomes, with whatever the backend could not write
/// already reported.
///
/// Shared by the two `--emit` kinds that reach the backend rather than written
/// out in each, for the reason [`Emitted`] gives for existing: two copies of a
/// rule are two things that have to agree.
///
/// The header goes in here rather than where the artifact is made, so that a
/// run which read none of its inputs leaves nothing rather than a module with
/// no functions in it. `run_compiler` reads emptiness as "made nothing" and
/// declines to write it over a path the user gave; a header alone would pass
/// that test and destroy the file. Once per artifact, because both kinds refuse
/// a second input and a module may carry one `target triple`.
fn module(
    sources: &SourceMap,
    unit: &TranslationUnit,
    target: Target,
    diagnostics: &mut DiagnosticSink,
) -> String {
    let mut module = safec_llvm::emit::header(target);

    // The backend answers what it could not write rather than reporting it,
    // because it cannot see a `Diagnostic`: ADR-0011 put those in this crate.
    // Every function it could write is in `module` already, and the ones it
    // could not are declarations.
    for refusal in safec_llvm::emit::functions(sources, unit, &mut module) {
        diagnostics.report(backend_failure(&refusal));
    }

    module
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

/// Why `clang` could not make what was asked of it.
///
/// Four answers rather than two, because a program on the path is not always
/// the program its name says. Only [`Self::Refused`] is `clang` speaking about
/// a module; the other three are the machine speaking about `clang`, and
/// wording them as a refusal blames a user's program for their installation.
/// That has been done here once, to a `clang` that never ran.
#[derive(Debug)]
enum Unmade {
    /// There is no `clang` to run.
    Absent,
    /// There is something by that name and it could not be started, and this is
    /// what the operating system said about it.
    Unrunnable(String),
    /// It ran and refused, or could not be spoken to, and this is what it said.
    Refused(String),
    /// It ran, said nothing was wrong, and made nothing.
    Silent,
}

/// Run `clang` over a module and hand back what it said.
///
/// Spawning a tool rather than linking one is ADR-0015, which also says what it
/// costs: this is the only thing here that needs a program at run time, and the
/// two `--emit` kinds that reach it do not work on a machine without one.
///
/// The two callers differ in what they ask for and in what they read back, and
/// share everything about what a tool on the path might do instead of the job,
/// which is why this is one function. The arguments carry the job; `-x ir` and
/// the target are here because both jobs take modules for a named machine.
///
/// `-Wno-override-module` because the module names its own triple and `clang`'s
/// own is more specific, so it warns about agreeing.
///
/// **`-x ir` is sticky**: it says what every file after it is, so a job naming
/// something that is not a module would have that read as one too. Both jobs
/// hand over modules, and a third that does not would move the flag into the
/// jobs rather than add a file to one. The failure is loud, which is why this
/// is a sentence rather than a shape.
///
/// **A module goes in on standard input or does not.** One does, for a compile,
/// because the object comes back on the other pipe and nothing needs a name. A
/// link names files instead, because there can be several and only one of them
/// could be a stream; there is then nothing to write, and the child is given a
/// standard input that is already at its end rather than a pipe nobody writes
/// to.
///
/// **When there is one, it is written on a thread.** Both pipes are open at
/// once, and a module larger than the pipe buffer would otherwise deadlock
/// against a `clang` that has begun answering before it has finished reading.
/// About 64 KiB on this host, which an ordinary `.c` file reaches; the same
/// hazard was measured in `tests/llvm.rs` and is why that one does not `expect`
/// its write.
fn clang(job: &[&OsStr], stdin: Option<&str>, target: Target) -> Result<Output, Unmade> {
    let mut spawned = Command::new("clang")
        .args(["-x", "ir", "-Wno-override-module"])
        .arg(format!("--target={}", target.triple()))
        .args(job)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        // A spawn that failed for any other reason is a third answer and not a
        // refusal: `clang` never ran, so it has said nothing about the module,
        // and wording it as though it had blames the program for a directory
        // named `clang` on the path, or a binary this machine cannot start.
        .map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => Unmade::Absent,
            _ => Unmade::Unrunnable(error.to_string()),
        })?;

    let feeding = stdin.map(|module| {
        let mut input = spawned.stdin.take().expect("the pipe was asked for");
        let written = module.to_owned();
        thread::spawn(move || input.write_all(written.as_bytes()))
    });

    let finished = spawned
        .wait_with_output()
        .map_err(|error| Unmade::Refused(error.to_string()))?;

    // After the output, because a `clang` that gave up early breaks the pipe
    // and its own words are the better answer. A write that failed for any
    // other reason surfaces as a `clang` that got an incomplete module and
    // said so.
    if let Some(feeding) = feeding {
        let _ = feeding.join();
    }

    Ok(finished)
}

/// What `clang` said when it refused, as far as this job can hear it.
///
/// **Which stream carries the words depends on the job.** For a compile they
/// are on standard error, because standard output is the object. For a link
/// they can be on either: `link.exe` writes its own diagnosis to standard
/// output and `clang`'s driver writes the summary to standard error, so on
/// Windows the line that says *why* is the one a compile would have to throw
/// away. Measured on this host: a module with no `main` answers
/// `LINK : fatal error LNK1561` on standard output and
/// `clang: error: linker command failed with exit code 1561` on standard error.
///
/// Another tool's text, in whatever encoding that tool writes: `from_utf8_lossy`
/// rather than a failure, because a mangled sentence is worth more to a user
/// than none. `docs/architecture.md` records this as a divergence from "do not
/// let the host into a diagnostic".
fn refused(streams: &[&[u8]]) -> Unmade {
    let said: Vec<String> = streams
        .iter()
        // A carriage return in the middle is the tool's line ending rather than
        // something it meant to say, and the renderer escapes what it does not
        // recognise, so leaving one in puts a literal escape in a note.
        // Trimming the ends is not enough once a tool writes more than one
        // line, which is what a linker over several modules does.
        .map(|stream| String::from_utf8_lossy(stream).replace("\r\n", "\n"))
        .map(|stream| stream.trim().to_owned())
        .filter(|stream| !stream.is_empty())
        .collect();

    Unmade::Refused(said.join("\n"))
}

/// One module as an object for the machine it names.
///
/// The object comes back on the second pipe, so nothing here needs a temporary
/// file and no temporary name reaches the object: `clang` records what it was
/// given, and the same module assembled twice is the same bytes.
///
/// An exit status is not evidence that an object exists. A `clang` on the path
/// is not always LLVM's: a compiler cache or a distributing wrapper is routinely
/// installed under that name, and one that is misconfigured answers nothing and
/// exits successfully. Taking that as an object writes zero bytes over whatever
/// `-o` named and exits zero, which is the one thing `compile` says a compiler
/// must never do.
fn assembled(module: &str, target: Target) -> Result<Vec<u8>, Unmade> {
    let finished = clang(
        &[
            OsStr::new("-c"),
            OsStr::new("-o"),
            OsStr::new("-"),
            OsStr::new("-"),
        ],
        Some(module),
        target,
    )?;

    if !finished.status.success() {
        // Standard output is the object here, not words, whatever is on it.
        return Err(refused(&[&finished.stderr]));
    }
    if finished.stdout.is_empty() {
        return Err(Unmade::Silent);
    }
    Ok(finished.stdout)
}

/// Several modules as one program for the machine it names.
///
/// One spawn and not one per module: `clang -x ir` with no `-c` reads every
/// module it is given and answers a linked program, so there is no object in
/// between and nothing to keep one in. Measured rather than assumed, and it is
/// why this does not go through [`assembled`].
///
/// **The modules go to a directory of this run's own, and so does the
/// program.** Only one of them could have been a stream, and a linker cannot
/// write to one either: `-o -` makes a file called `-` and exits successfully,
/// measured. Reading the program back rather than pointing `clang` at what the
/// user asked for is what keeps `compile` from writing to a path the user
/// named, which is the invariant the write rule in [`run_compiler`] rests on.
/// The directory's own name reaches neither, measured: the same modules linked
/// from two differently named directories answer the same bytes. That is not
/// the same as two runs agreeing, and they do not: `link.exe` stamps the time
/// into a program, so two runs a second apart differ in four bytes. What is
/// reproducible is what this compiler decides, which is why the claim is about
/// the directory rather than about the program.
///
/// **One command line names every module, and a command line has a ceiling.**
/// About six hundred inputs with short names on this host, where the limit is
/// 32767 characters; fewer with the paths a real project has. Past it the spawn
/// fails and the run says so, honestly but in the wrong voice: the fault is
/// this compiler's command line rather than the user's installation, and the
/// message is the one about `clang`. A response file is what answers it, and
/// nothing reaches the ceiling while an input is one file a person wrote.
///
/// It does reach a *diagnostic*, because a linker quotes the path it was told
/// to write as well as the files it was given: a duplicate symbol answers
/// `<scratch>\program : fatal error LNK1169`. That is another tool's text and
/// `docs/architecture.md` records passing it through as a divergence; the only
/// way to keep the path out of it is to hand `clang` what the user asked for,
/// which is the invariant above.
fn linked(modules: &[Module], target: Target) -> Result<Vec<u8>, Unlinked> {
    let scratch = Scratch::new().map_err(|error| Unlinked::Nowhere(error.to_string()))?;
    let program = scratch.path().join("program");

    let mut job = vec![
        OsStr::new("-o").to_owned(),
        program.clone().into_os_string(),
    ];
    for (index, module) in modules.iter().enumerate() {
        let written = scratch.path().join(format!("{index}-{}.ll", module.named));
        fs::write(&written, &module.text).map_err(|error| Unlinked::Nowhere(error.to_string()))?;
        job.push(written.into_os_string());
    }

    let job: Vec<&OsStr> = job.iter().map(AsRef::as_ref).collect();
    let finished = clang(&job, None, target).map_err(Unlinked::Tool)?;

    if !finished.status.success() {
        // Both streams, because the linker and the driver that ran it do not
        // agree about which one to speak on. See `refused`.
        return Err(Unlinked::Tool(refused(&[
            &finished.stdout,
            &finished.stderr,
        ])));
    }

    // What the linker wrote, or that it wrote nothing. `fs::read` answers the
    // second as an error, which is the same fact `assembled` reads off an empty
    // pipe and means the same thing: a successful exit is not a program.
    match fs::read(&program) {
        Ok(bytes) if !bytes.is_empty() => Ok(bytes),
        _ => Err(Unlinked::Tool(Unmade::Silent)),
    }
}

/// Why a module could not be made into a program.
///
/// Two parties, and telling them apart is the whole reason this is not
/// [`Unmade`]. `clang` answers for the first; the second is this compiler
/// failing to find anywhere to work, which is nothing to do with the tool and
/// must not be reported as though the tool were broken. That is [`Unmade`]'s
/// mistake one level over: blaming whoever is nearest.
#[derive(Debug)]
enum Unlinked {
    /// What `clang` did, or did not do.
    Tool(Unmade),
    /// There was nowhere to put the program while it was being made, and this
    /// is what the operating system said about that.
    Nowhere(String),
}

/// One input's module, and the name it goes under.
///
/// **The name is the input's.** `clang` derives the temporary object names it
/// links from the stems it is handed, and those names are what the linker
/// quotes when two inputs define one symbol: two modules written as `0.ll` and
/// `1.ll` produce `0-a43ee2.o : error LNK2005: main is already defined in
/// 1-82f49a.o`, which names nothing the user wrote. Measured. The index keeps
/// two inputs with one stem apart, which `a/x.c` and `b/x.c` are.
struct Module {
    named: String,
    text: String,
}

/// A directory of one link's own, removed however the link ends.
///
/// The only thing this compiler writes outside a path the user named, and it
/// exists because a linker will not answer on a pipe. Not called a workspace,
/// because this file already uses that word for the one `cargo` builds.
///
/// Made with `create_dir` rather than `create_dir_all`, so that a name already
/// taken is an error here rather than a directory shared with whoever holds it.
///
/// **The name cannot be the process and a count alone.** Removal is best effort
/// and acquisition is strict, so a run that is killed leaves its directory
/// behind, and an operating system that reuses process ids hands the name to
/// somebody else: `--emit executable` would then fail for that process every
/// time, permanently, with a message about a tool that is not the trouble. The
/// clock is what makes a leftover harmless; the count is what keeps two threads
/// of one process apart inside the same tick, which the unit tests need.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> io::Result<Self> {
        static LINKS: AtomicUsize = AtomicUsize::new(0);

        let since = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            // Before 1970 on a machine whose clock says so. The count alone is
            // still unique within this process, which is what matters here.
            .unwrap_or(0);

        let path = std::env::temp_dir().join(format!(
            "safec-{}-{since}-{}",
            std::process::id(),
            LINKS.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // Nothing to report it to, and nothing a user could do about it: the
        // program has already been read out of here.
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// What the backend could not write, as a diagnostic.
///
/// The refusal's own sentence is the message, because it is the specific half:
/// "the backend cannot write an indexed place, which counts elements and so
/// needs a width" says more than a heading and a note would. `load_failure` has
/// the same shape for the same reason.
///
/// A [`Refusal`] never carries text out of a source file, only type spellings
/// this compiler wrote and numbers, so nothing here needs `shown`. That is
/// worth stating rather than assuming, because every new place that echoes
/// text has to answer for whether it is a source file's.
fn backend_failure(refusal: &Refusal) -> Diagnostic {
    let reported =
        Diagnostic::error(format!("the backend cannot write {}", refusal.why)).with_code(BACKEND);

    match refusal.at {
        Some(span) => reported.with_label(Label::primary(span, "this is what it could not write")),
        // Only a call among the terminators carries a span, so a refusal about
        // one of the others has nowhere to point and says so.
        None => reported.with_note("the IR does not say where this came from"),
    }
}

/// The tree, as a caller redirecting it would see.
///
/// One node per line, two spaces of indent per level: the kind, where it is,
/// and whatever that node alone carries. Past
/// [`safec_ir::print::DEEPEST_INDENT`] levels the indent stops growing and the
/// line says how many it is short by, which is where the depth of a tree
/// nothing bounds stops being a number of spaces.
///
/// **No node identity.** `clang -Xclang -ast-dump` prints one and it is the
/// node's address, so two runs of the same command on the same file disagree.
/// A corpus case compares byte for byte, and an artifact nobody can pin is an
/// interface nobody can hold this compiler to.
fn dump_ast(sources: &SourceMap, ast: &Ast, out: &mut String) {
    for item in ast.items() {
        dump_item(sources, ast, item, 0, out);
    }
}

fn dump_item(sources: &SourceMap, ast: &Ast, item: &Item, depth: usize, out: &mut String) {
    dump_node(sources, item.name(), item.span(), depth, out);
    match item {
        Item::Function(function) => {
            write!(
                out,
                " {:?} {:?}",
                quoted(sources, function.name),
                spell_type(sources, ast, function.ty)
            )
            .expect("writing to a string cannot fail");
            // The name and the string as written, because the tree records what
            // was read and `sema::resolve` is what says whether it is a hatch.
            // Both are the file's text, written with `{:?}` because source
            // text is content.
            if let Some(attribute) = function.attribute {
                write!(
                    out,
                    " __attribute__ {:?} {:?}",
                    quoted(sources, attribute.name),
                    quoted(sources, attribute.argument)
                )
                .expect("writing to a string cannot fail");
            }
            out.push('\n');
            dump_parameters(sources, ast, function.ty, depth + 1, out);
            dump_stmt(sources, ast, ast.stmt(function.body), depth + 1, out);
        }
        Item::Declaration { declarators, span } => {
            dump_declarators(sources, ast, declarators, item.name(), *span, depth, out);
        }
        Item::Error { .. } => out.push('\n'),
    }
}

/// Every declarator of one declaration, a line each.
///
/// `dump_node` has already written the line the first one goes on, so each one
/// after it opens a line of its own at the same depth and with the same span.
/// That is what keeps `int x;` printing exactly what it printed before there
/// was a list at all, and it is honest about `int a, b;`: both declarators do
/// begin at the specifiers, which is what [`Declaration::span`] says and why
/// the two lines carry the same position.
///
/// [`Declaration::span`]: crate::ast::Declaration::span
fn dump_declarators(
    sources: &SourceMap,
    ast: &Ast,
    declarators: &[InitDeclarator],
    name: &'static str,
    span: Span,
    depth: usize,
    out: &mut String,
) {
    if declarators.is_empty() {
        // `dump_node` has written a prefix and nothing below would end the
        // line. Both variants say in their doc comments that a list is never
        // empty, and #125 is the change that would make one: this is the line
        // it has to find.
        out.push('\n');
        return;
    }

    for (at, declarator) in declarators.iter().enumerate() {
        if at > 0 {
            dump_node(sources, name, span, depth, out);
        }

        dump_declaration(sources, ast, &declarator.declaration, out);
        dump_parameters(sources, ast, declarator.declaration.ty, depth + 1, out);
        if let Some(init) = declarator.init {
            dump_expr(sources, ast, init, depth + 1, out);
        }
    }
}

/// The tail of a line that declares something: the name, the type, and
/// `_Nonnull` where it was written.
///
/// A name is the file's own bytes and is quoted because source text is
/// content. A
/// type is this compiler's spelling of what the declarator derived, quoted
/// beside it so that the two read alike; the only file text inside one is the
/// length of an array, which [`spell_type`] answers for.
///
/// `_Nonnull` is a word on the line rather than part of the type string,
/// because it is not part of the type: [`Declaration::nonnull`] says why.
fn dump_declaration(sources: &SourceMap, ast: &Ast, declaration: &Declaration, out: &mut String) {
    if let Some(name) = declaration.name {
        write!(out, " {:?}", quoted(sources, name)).expect("writing to a string cannot fail");
    }
    write!(out, " {:?}", spell_type(sources, ast, declaration.ty))
        .expect("writing to a string cannot fail");
    if declaration.nonnull.is_some() {
        out.push_str(" _Nonnull");
    }
    out.push('\n');
}

/// The parameters a declarator named, where the type is directly a function.
///
/// Only directly. A parameter of `int (*g)(int x)` is inside a pointer, and
/// what it is called is not something anything can refer to, so the type string
/// is where it stays. What a definition writes is reachable, and #27 resolves
/// it, so it gets a line of its own.
fn dump_parameters(sources: &SourceMap, ast: &Ast, ty: TypeId, depth: usize, out: &mut String) {
    let Type::Function {
        parameters: Parameters::Prototype(parameters),
        ..
    } = ast.ty(ty)
    else {
        return;
    };

    for parameter in parameters {
        dump_node(sources, "Parameter", parameter.span, depth, out);
        dump_declaration(sources, ast, parameter, out);
    }
}

/// One statement and everything under it.
///
/// A recursion, unlike `dump_expr`, and it can be one because every place a
/// statement nests inside another is a recursion in the parser too, and
/// `MAX_NESTING` bounds those. `dump_expr` needs its own stack because an
/// expression tree does not have that property: two of its rules fold with a
/// loop.
///
/// Which optional parts were written goes on the node's own line rather than
/// into a child, because a child would be a line for something the source does
/// not contain, and every line in this artifact carries a position. `clang`
/// says `has_else` for the same reason. Without it `for (i;;) ;` and
/// `for (;;i) ;` would be the same two lines.
fn dump_stmt(sources: &SourceMap, ast: &Ast, stmt: &Stmt, depth: usize, out: &mut String) {
    dump_node(sources, stmt.name(), stmt.span(), depth, out);

    let child =
        |id: StmtId, out: &mut String| dump_stmt(sources, ast, ast.stmt(id), depth + 1, out);

    match stmt {
        Stmt::Compound { body, .. } => {
            out.push('\n');
            for &id in body {
                child(id, out);
            }
        }
        Stmt::Return { value, .. } => {
            out.push('\n');
            if let Some(id) = value {
                dump_expr(sources, ast, *id, depth + 1, out);
            }
        }
        Stmt::Declaration { declarators, span } => {
            dump_declarators(sources, ast, declarators, stmt.name(), *span, depth, out);
        }
        Stmt::Expression { value, .. } => {
            out.push('\n');
            if let Some(id) = value {
                dump_expr(sources, ast, *id, depth + 1, out);
            }
        }
        Stmt::If {
            condition,
            then,
            otherwise,
            ..
        } => {
            // Redundant here, where two children mean no `else` and three mean
            // one, and not redundant on a `for`. One rule for both is worth
            // more than the line it saves.
            if otherwise.is_some() {
                out.push_str(" else");
            }
            out.push('\n');
            dump_expr(sources, ast, *condition, depth + 1, out);
            child(*then, out);
            if let Some(otherwise) = otherwise {
                child(*otherwise, out);
            }
        }
        Stmt::While {
            condition, body, ..
        } => {
            out.push('\n');
            dump_expr(sources, ast, *condition, depth + 1, out);
            child(*body, out);
        }
        Stmt::For {
            initialiser,
            condition,
            step,
            body,
            ..
        } => {
            for (clause, name) in [
                (initialiser, "init"),
                (condition, "condition"),
                (step, "step"),
            ] {
                if clause.is_some() {
                    write!(out, " {name}").expect("writing to a string cannot fail");
                }
            }
            out.push('\n');
            for clause in [initialiser, condition, step].into_iter().flatten() {
                dump_expr(sources, ast, *clause, depth + 1, out);
            }
            child(*body, out);
        }
        Stmt::Error { .. } => out.push('\n'),
    }
}

/// One expression and everything under it.
///
/// **An explicit stack and not recursion.** The tree can be deeper than the
/// parser ever went: `MAX_NESTING` bounds the parser's own recursion, and a
/// left-associative chain (`a + a + ...`) and a run of postfix operators
/// (`a++++`) are folded by a loop, so each adds a level to the tree without the
/// parser calling itself once. A thousand of either is an ordinary generated
/// line, and walking it recursively ended the process at around a thousand with
/// no diagnostic and an exit code nothing here chose. Every later walk of this
/// tree owes itself the same answer, and owes it here rather than borrowing
/// this one: [`Expr::extend_children`] is the half that can be shared, and the stack
/// is the half that cannot.
///
/// A node writes at most one quoted thing after its position: either the file's
/// own text, or the operator this compiler spells. The two are not the same
/// kind of thing. `Number` and `Identifier` echo the source, which is content
/// and why they are quoted; an operator comes from `BinOp::as_str` and is
/// this compiler's own word, so `a  +  b` still prints `"+"`.
fn dump_expr(sources: &SourceMap, ast: &Ast, root: ExprId, depth: usize, out: &mut String) {
    // Children are pushed in reverse, so that they come back off in the order
    // they were written. What that makes is a pre-order walk, the same one the
    // recursive version made.
    let mut pending = vec![(root, depth)];
    // Cleared per node, because `extend_children` appends: this wants the
    // children of one node, not of every node so far. Reused rather than built
    // afresh so that a whole walk allocates once.
    let mut children = Vec::new();

    while let Some((id, depth)) = pending.pop() {
        let expr = ast.expr(id);
        dump_node(sources, expr.name(), expr.span(), depth, out);

        // What this node says about itself, and nothing about what is under it.
        match expr {
            Expr::Number { span } | Expr::Identifier { span } => {
                write!(out, " {:?}", quoted(sources, *span))
                    .expect("writing to a string cannot fail");
            }
            Expr::Unary { op, .. } => {
                // `++` and `--` are the only operators C writes on either side,
                // so they are the only ones that need saying which side this
                // was.
                if let Some(fixity) = op.fixity() {
                    write!(out, " {fixity}").expect("writing to a string cannot fail");
                }
                write!(out, " {:?}", op.as_str()).expect("writing to a string cannot fail");
            }
            Expr::Binary { op, .. } => {
                write!(out, " {:?}", op.as_str()).expect("writing to a string cannot fail");
            }
            Expr::Assign { op, .. } => {
                // `+=` is `+` and `=`, built rather than tabulated: eleven more
                // spellings in a second table is a second table to disagree
                // with the first, and two spellings of one decision have
                // already left a case here answered by neither.
                let spelling = match op {
                    Some(op) => format!("{}=", op.as_str()),
                    None => "=".to_owned(),
                };
                write!(out, " {spelling:?}").expect("writing to a string cannot fail");
            }
            Expr::Comma { .. } => {
                write!(out, " {:?}", ",").expect("writing to a string cannot fail");
            }
            Expr::Conditional { .. }
            | Expr::Call { .. }
            | Expr::Subscript { .. }
            | Expr::Error { .. } => {}
        }
        out.push('\n');

        children.clear();
        expr.extend_children(&mut children);
        for &child in children.iter().rev() {
            pending.push((child, depth + 1));
        }
    }
}

/// One line per token: where it starts, what it is, and the text it covers.
///
/// The text is quoted rather than written plainly. It comes out of the file, so
/// it is content, and this goes to a terminal: quoting escapes a control
/// character rather than obeying it, for the same reason the renderer does not
/// echo one, and it makes a token legible whose text is a space or a newline.
///
/// A position rather than a span, because the quoted text says how far the
/// token reaches and a dump is read down its left edge. Not because the next
/// token begins where this one ends: trivia sits between them more often than
/// not.
fn dump_tokens(file: &SourceFile, tokens: &[Token], out: &mut String) {
    for token in tokens {
        let at = file.line_col(token.span.start());
        write!(
            out,
            "{}:{}:{} {}",
            shown(&file.name().to_string()),
            at.line,
            at.column,
            token.kind.name()
        )
        .expect("writing to a string cannot fail");

        if !token.is_eof() {
            write!(out, " {:?}", &file.contents()[token.span.range()])
                .expect("writing to a string cannot fail");
        }
        out.push('\n');
    }
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

/// Why `clang` could not make what was asked of it, as a diagnostic.
///
/// No code, like the others beside it and unlike `SC0801`: this is the driver
/// saying something about the machine a run is on rather than about the program
/// it was given. `docs/diagnostics.md` draws that line.
///
/// The absent case says what to install, because a user who reaches it has a
/// working compiler and a missing tool, and "cannot run clang" on its own leaves
/// them to guess which clang and why. The refused case passes `clang`'s own
/// words through rather than interpreting them: it knows what is wrong with a
/// module and this does not, and a clang older than LLVM 15 answers about the
/// opaque pointers this writes.
///
/// The other two are the cases where nothing was heard from `clang` at all, and
/// they are separate for exactly that reason. Saying "could not make an object
/// of this module" about a run that never started, or about one that started
/// and answered nothing, points a user at their program when the fault is on
/// their machine.
///
/// `made` is the caller's word for what it asked for, and `kind` is what the
/// user typed. Both callers know which they are, so neither is looked up: a
/// table would need an answer from every kind that never reaches `clang`, and
/// the only honest answers there are unreachable.
fn clang_failure(kind: EmitKind, made: &str, why: &Unmade) -> Diagnostic {
    match why {
        Unmade::Absent => Diagnostic::error(format!(
            "`--emit {}` needs `clang` and found none",
            kind.spelling()
        ))
        .with_note("safec writes LLVM IR and asks clang to make the artifact out of it")
        .with_note("any clang whose LLVM is 15 or newer reads the IR this writes"),
        Unmade::Unrunnable(said) => Diagnostic::error(format!(
            "`--emit {}` found `clang` and could not run it",
            kind.spelling()
        ))
        .with_note(said.trim())
        .with_note("this is what the machine said, not what clang said"),
        Unmade::Refused(said) => {
            // Not "of this module": a link is over as many as the run had
            // inputs, and counting them in a message is a count to keep right
            // for nothing. What it could not make is the part a user needs.
            let reported = Diagnostic::error(format!("clang could not make {made}"));
            // A `clang` that was killed, or that crashed, exits unsuccessfully
            // with nothing to say. An empty note is worse than no note: it
            // reads as a message this compiler failed to fill in.
            match said.trim() {
                "" => reported.with_note("it exited unsuccessfully and said nothing"),
                said => reported.with_note(said),
            }
        }
        Unmade::Silent => Diagnostic::error(format!(
            "clang did not make {made} and said nothing was wrong"
        ))
        .with_note("the clang on this path may be a wrapper rather than a compiler"),
    }
}

/// The same, for a link, which has one answer `clang` has nothing to do with.
///
/// Linking is the one job here that can fail for being asked about another
/// machine. Assembling for one needs no linker and no sysroot, which is why
/// every target in `Target::ALL` assembles on this machine and only the host
/// links; `clang`'s words about a missing linker do not mention the target, so
/// this adds the sentence that does.
///
/// Added rather than replacing: a user with a cross toolchain is not refused,
/// and one without it is told what would have been needed.
fn link_failure(options: &Options, why: &Unlinked) -> Diagnostic {
    let tool = match why {
        Unlinked::Tool(why) => why,
        // Nothing was asked of `clang`, so nothing here is about it. Saying so
        // in `clang`'s words would send a user to look at an installation that
        // is fine.
        Unlinked::Nowhere(said) => {
            return Diagnostic::error("`--emit executable` needs somewhere to work and found none")
                .with_note(said.trim())
                .with_note("a program is linked in a directory of its own and read back from it")
                .with_note("this is where TMPDIR, or TMP and TEMP, point");
        }
    };

    let reported = clang_failure(options.emit, "a program", tool);

    if options.target.triple() == HOST_TRIPLE {
        return reported;
    }
    reported.with_note(format!(
        "linking for {} on a {} machine needs a linker and a sysroot for it",
        options.target.triple(),
        HOST_TRIPLE
    ))
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
