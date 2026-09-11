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
    Ast, Declaration, Expr, ExprId, Item, Parameters, Stmt, StmtId, Type, TypeId, spell_type,
};
use crate::cli::HOST_TRIPLE;
use crate::diagnostics::render::Renderer;
use crate::diagnostics::{Code, Diagnostic, DiagnosticSink, Label, Policy};
use crate::lexer::lex;
use crate::lowering::lower;
use crate::options::{EmitKind, Options};
use crate::parser::parse;
use crate::sema::{Resolution, resolve};
use crate::token::Token;
use crate::types::{Types, check};
use safec_ir::ir::TranslationUnit;
use safec_ir::print::{dump_ir, dump_node, quoted, shown};
use safec_ir::source::{FileId, FileName, SourceFile, SourceMap};
use safec_ir::target::Target;
use safec_llvm::emit::Refusal;

// Code generation takes `SC08xx`, which `docs/diagnostics.md` allocates. One
// code for every shape of a refusal, for the reason `types.rs` gives for
// `MISMATCH`: what differs between them is the message, and a reader filtering
// on the code wants "the backend could not write this" rather than a list of
// the ways that can happen.
//
// The only diagnostic in this file that carries one. The others are the driver
// saying something about a run or about the machine it is on, and this is the
// backend saying something about a program, which is the line
// `docs/diagnostics.md` draws. No count here: that document carries one, with
// the command that settles it, and two places counting the same thing is one
// place too many.
const BACKEND: Code = Code::new("SC0801");

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
/// own work instead, and the run-level answer is left for what genuinely spans
/// the run: the outcome, and later, linking.
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
        EmitKind::LlvmIr => Emitted::LlvmIr(String::new()),
        EmitKind::Object => Emitted::Object(Vec::new()),
        EmitKind::Executable => Emitted::Program(Vec::new()),
    };
    // What the loop makes for a kind that does not finish an artifact per
    // input. A program is the only one: it is made out of every input at once,
    // so the loop keeps the modules and `finish` turns them into one.
    let mut modules: Vec<Module> = Vec::new();
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
                let Some(analysed) =
                    analysed(&sources, file, &tokens, read_whole, &mut diagnostics)
                else {
                    continue;
                };
                dump_ast(&sources, &analysed.ast, out);
            }
            Emitted::SafetyIr(out) => {
                let Some(unit) = lowered(
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
            Emitted::LlvmIr(out) => {
                let Some(unit) = lowered(
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
                let Some(unit) = lowered(
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
            Emitted::Program(_) => {
                let Some(unit) = lowered(
                    &sources,
                    file,
                    &tokens,
                    read_whole,
                    options,
                    &mut diagnostics,
                ) else {
                    continue;
                };
                // **Nothing is linked here.** A program is one artifact out of
                // every input, so the only thing this arm can do about one of
                // them is keep it. `finish` below is where they become a
                // program, and this arm has nothing to link with: the modules
                // are not in the artifact.
                let text = module(&sources, &unit, options.target, &mut diagnostics);
                if said_something(&diagnostics, read_whole) {
                    continue;
                }

                modules.push(Module {
                    named: stem(sources.file(file).name()),
                    text,
                });
            }
        }
    }

    finish(&mut artifact, modules, options, &mut diagnostics);

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
/// on `EmitKind` that have to agree. RK-003 in the review knowledge bank
/// records what the two-copy version of this cost: a gate written as two
/// comparisons left `--emit preprocessed` answered by neither, and the compiler
/// exited zero having written nothing to either stream.
enum Emitted {
    /// What `--emit tokens` asked for.
    Tokens(String),
    /// What `--emit ast` asked for.
    Ast(String),
    /// What `--emit safety-ir` asked for.
    SafetyIr(String),
    /// What `--emit llvm-ir` asked for.
    LlvmIr(String),
    /// What `--emit object` asked for.
    ///
    /// The one kind that is not text, and the reason [`Compiled::artifact`] is
    /// bytes. `clang` made these and this compiler only carries them.
    Object(Vec<u8>),
    /// What `--emit executable` asked for.
    ///
    /// Bytes for the same reason, and the one artifact that needs a mode as
    /// well as bytes: see [`EmitKind::is_a_program`] and where [`run_compiler`]
    /// writes.
    Program(Vec<u8>),
}

impl Emitted {
    fn into_bytes(self) -> Vec<u8> {
        match self {
            Self::Tokens(text) | Self::Ast(text) | Self::SafetyIr(text) | Self::LlvmIr(text) => {
                text.into_bytes()
            }
            Self::Object(bytes) | Self::Program(bytes) => bytes,
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
    let types = check(sources, &mut ast, &resolution, diagnostics);

    Some(Analysed {
        ast,
        typed: Some((resolution, types)),
    })
}

/// One input's IR, or nothing where the frontend could not get that far.
///
/// Shared by the two `--emit` kinds that read the IR rather than duplicated in
/// each, for the reason [`Emitted`] gives for existing at all: two copies of a
/// gate are two things that have to agree, and RK-003 is what that costs when
/// they stop.
fn lowered(
    sources: &SourceMap,
    file: FileId,
    tokens: &[Token],
    read_whole: usize,
    options: &Options,
    diagnostics: &mut DiagnosticSink,
) -> Option<TranslationUnit> {
    let analysed = analysed(sources, file, tokens, read_whole, diagnostics)?;
    // Nothing to lower from a tree whose names and types are not known: the IR
    // would be built out of what the frontend could not work out, and the
    // lowering says so about each piece rather than saying it once here.
    let (resolution, types) = analysed.typed.as_ref()?;

    Some(lower(
        sources,
        &analysed.ast,
        resolution,
        types,
        options.target,
        diagnostics,
    ))
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
    // Exhaustive, and not two `matches!`, for the reason RK-003 records: a
    // kind added and forgotten in a list is answered by no arm and goes to the
    // stream, which for a kind with a default name is silent and wrong.
    match options.emit {
        EmitKind::Tokens | EmitKind::Ast | EmitKind::SafetyIr | EmitKind::LlvmIr => None,
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
/// and has nothing left to do, which is what the empty arm says.
fn finish(
    artifact: &mut Emitted,
    modules: Vec<Module>,
    options: &Options,
    diagnostics: &mut DiagnosticSink,
) {
    let Emitted::Program(out) = artifact else {
        return;
    };

    // Nothing is linked for a run that reported anything, and there is no
    // second condition: a run with no modules is a run whose inputs all
    // reported, or one that was given none, and both of those have said so
    // already. Spawning here would answer a linker's words about an empty
    // program on top of the reason the user already has.
    if diagnostics.has_errors() {
        return;
    }

    match linked(&modules, options.target) {
        Ok(program) => *out = program,
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
/// RK-024 in the review knowledge bank is what that cost.
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
        .map(|stream| String::from_utf8_lossy(stream).trim().to_owned())
        .filter(|stream| !stream.is_empty())
        .collect();

    Unmade::Refused(said.join(
        "
",
    ))
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
/// The directory's own name reaches neither: two runs of the same modules from
/// differently named directories answer byte for byte, measured.
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
/// must not be reported as though the tool were broken. That is RK-024's
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
/// this compiler wrote and numbers, so nothing here needs `shown`. RK-002 in
/// the review knowledge bank is why that is worth stating rather than assuming.
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
/// and whatever that node alone carries. Past [`DEEPEST_INDENT`] levels the
/// indent stops growing and the line says how many it is short by, which is
/// where the depth of a tree nothing bounds stops being a number of spaces.
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
            out.push('\n');
            dump_parameters(sources, ast, function.ty, depth + 1, out);
            dump_stmt(sources, ast, ast.stmt(function.body), depth + 1, out);
        }
        Item::Declaration(declaration) => {
            dump_declaration(sources, ast, declaration, out);
            dump_parameters(sources, ast, declaration.ty, depth + 1, out);
        }
        Item::Error { .. } => out.push('\n'),
    }
}

/// The tail of a line that declares something: the name, then the type.
///
/// A name is the file's own bytes and is quoted for the reason RK-002 gives. A
/// type is this compiler's spelling of what the declarator derived, quoted
/// beside it so that the two read alike; the only file text inside one is the
/// length of an array, which [`spell_type`] answers for.
fn dump_declaration(sources: &SourceMap, ast: &Ast, declaration: &Declaration, out: &mut String) {
    if let Some(name) = declaration.name {
        write!(out, " {:?}", quoted(sources, name)).expect("writing to a string cannot fail");
    }
    write!(out, " {:?}", spell_type(sources, ast, declaration.ty))
        .expect("writing to a string cannot fail");
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
        Stmt::Declaration(declaration) => {
            dump_declaration(sources, ast, declaration, out);
            dump_parameters(sources, ast, declaration.ty, depth + 1, out);
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
/// kind of thing. `Number` and `Identifier` echo the source, which is why
/// RK-002 asks for the quoting; an operator comes from `BinOp::as_str` and is
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
                // with the first, which is what RK-003 records the cost of.
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
    // first. RK-003 in the review knowledge bank is what that costs: a gate
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
                let written = fs::write(path, emitted)
                    .and_then(|()| make_runnable(path, options.emit.is_a_program()));

                if let Err(error) = written {
                    // A program that was written and could not be made runnable
                    // is the state the rule above exists to avoid, arrived at
                    // from the other side: the run is about to fail and there
                    // is a build product on disk, newer than the source. Taking
                    // it away is best effort, because whatever stopped the mode
                    // being set can stop this too, and the diagnostic is the
                    // same either way.
                    let _ = fs::remove_file(path);
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
            let reported = Diagnostic::error(format!("clang could not make {made} of this module"));
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
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use clap::ValueEnum as _;

    use super::*;
    use crate::options::{ColorMode, EmitKind};
    use crate::safety::SafetyLevel;
    use safec_ir::source::Span;
    use safec_ir::target::Target;

    /// Answer `true` where a test that needs `clang` should go on, and say what
    /// it skipped where there is none.
    ///
    /// The same gate `tests/object.rs` has, for the same reason and not shared
    /// with it: an integration test is its own crate. A check that cannot fail
    /// loudly reports the state it was asked to prove, which is RK-012, so
    /// `SAFEC_REQUIRE_LLVM` makes the skip a failure and CI sets it.
    fn clang_or_skip(what: &str) -> bool {
        let here = Command::new("clang")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok();
        if here {
            return true;
        }
        assert!(
            std::env::var_os("SAFEC_REQUIRE_LLVM").is_none(),
            "SAFEC_REQUIRE_LLVM is set and there is no `clang` to run"
        );
        eprintln!("no `clang` on this machine: {what} was not checked");
        false
    }

    fn options(inputs: Vec<PathBuf>) -> Options {
        Options {
            inputs,
            output: None,
            safety: SafetyLevel::Memory,
            // The cheapest kind that still runs every stage of the loop. It
            // was `Executable` while nothing could produce one, which made it
            // the kind that did nothing; now the two kinds past `llvm-ir`
            // spawn `clang`, and a test about reading inputs should not need
            // one on the machine.
            emit: EmitKind::Tokens,
            target: Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
            deny_unknown: false,
            color: ColorMode::Never,
        }
    }

    /// A link's directory is gone when the link is.
    ///
    /// The only thing this compiler writes outside a path the user named, and
    /// the only one nothing else would notice: a `--emit executable` run that
    /// left one behind would leave one every time, in a directory nobody looks
    /// at, and every test in the suite would still pass.
    ///
    /// Its own path rather than a count of what is in the temporary directory,
    /// because the tests here run at once and another thread's link is allowed
    /// to be halfway through.
    ///
    /// Mutation: empty the `Drop` body. The directory is still there and this
    /// fails.
    #[test]
    fn a_links_directory_is_gone_when_the_link_is() {
        let scratch = Scratch::new().expect("the temporary directory is writable");
        let path = scratch.path().to_path_buf();
        assert!(path.is_dir(), "{}", path.display());

        // A program is what would be in it, and a directory with something in
        // it is the case `remove_dir` alone would not answer.
        fs::write(path.join("program"), b"bytes").expect("the directory is writable");
        drop(scratch);

        assert!(!path.exists(), "{} was left behind", path.display());
    }

    /// Two links at once are two directories.
    ///
    /// One name per link, or the second `create_dir` fails and a link that
    /// could have worked answers that there was nowhere to work. The unit tests
    /// run at once and are what reaches this first.
    ///
    /// Mutation: drop the count from the name, or the clock. Two scratches in
    /// one tick collide and this fails on the second `expect`.
    #[test]
    fn two_links_at_once_are_two_directories() {
        let first = Scratch::new().expect("the temporary directory is writable");
        let second = Scratch::new().expect("a second directory can be made");

        assert_ne!(first.path(), second.path());
    }

    /// What `clang` refused reaches the user in `clang`'s own words.
    ///
    /// The one path that needs a `clang` which runs and says no, and nothing
    /// this backend writes can produce one: the module is always valid IR, and
    /// a function it could not write becomes a `declare`. So the module is
    /// handed over directly rather than compiled from C, which is also why this
    /// is here rather than in `tests/object.rs`: `assembled` is private.
    ///
    /// Its own gate, for the same reason the tests over there have one. A
    /// feature that needs a tool cannot be tested without it, and a check that
    /// cannot fail loudly reports the state it was asked to prove, which is
    /// RK-012.
    ///
    /// Mutation: answer `Ok` whatever the exit status. The artifact becomes
    /// whatever `clang` wrote before giving up, the run exits successfully, and
    /// this fails on the error it did not get.
    #[test]
    fn what_clang_refused_is_said_in_clang_s_own_words() {
        if !clang_or_skip("what clang says about a bad module") {
            return;
        }

        let target = Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple");
        let why = assembled(
            "this is not LLVM IR at all
",
            target,
        )
        .expect_err("clang has nothing to make an object of");

        let Unmade::Refused(said) = &why else {
            panic!("{why:?}");
        };
        assert!(!said.is_empty(), "clang refused without saying why");

        let reported = clang_failure(EmitKind::Object, "an object", &why);
        assert!(
            reported.message().contains("could not make an object"),
            "{reported:?}"
        );
        assert_eq!(reported.notes().len(), 1, "{reported:?}");
    }

    /// A file on disk that removes itself. Named per test, because the suite
    /// runs in parallel and the temporary directory is shared.
    struct TempFile(PathBuf);

    impl TempFile {
        fn new(name: &str, contents: impl AsRef<[u8]>) -> Self {
            let path = std::env::temp_dir().join(name);
            fs::write(&path, contents).expect("the temporary directory is writable");
            Self(path)
        }

        /// A path in the same place that nothing has created.
        ///
        /// For a destination rather than a source: what the test is about is
        /// what the compiler writes there, and `Drop` still removes it however
        /// the test ends.
        fn reserve(name: &str) -> Self {
            let path = std::env::temp_dir().join(name);
            let _ = fs::remove_file(&path);
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    fn missing_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    /// A stream that is already gone, which is what `safec ... | head -1`
    /// leaves behind.
    struct Closed;

    impl io::Write for Closed {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Both of a run's streams: what it reported, and what it made.
    fn run(options: &Options) -> (String, String, Outcome) {
        let mut report = Vec::new();
        let mut artifact = Vec::new();
        let outcome = run_compiler(options, &mut report, &mut artifact)
            .expect("writing to a vector cannot fail");
        (
            String::from_utf8(report).expect("the renderer writes text"),
            String::from_utf8(artifact).expect("the emitter writes text"),
            outcome,
        )
    }

    fn rendered(options: &Options) -> (String, Outcome) {
        let (report, _, outcome) = run(options);
        (report, outcome)
    }

    /// The wiring ADR-0001 is about. Without it `--deny-unknown` parses,
    /// documents itself, and does nothing.
    #[test]
    fn the_sink_is_built_from_the_resolved_options() {
        for deny_unknown in [false, true] {
            let mut options = options(Vec::new());
            options.deny_unknown = deny_unknown;

            assert_eq!(
                compile(&options).diagnostics.policy().deny_unknown(),
                deny_unknown,
            );
        }
    }

    /// The strictest level is defined as leaving nothing `Unknown`, so its
    /// implication has to reach the sink even when the flag was not given as
    /// well. The level itself does not: a sink holds a [`Policy`] and never
    /// learns which checks ran.
    #[test]
    fn the_strictest_safety_level_denies_unknown_in_the_sink() {
        let mut options = options(Vec::new());
        options.safety = SafetyLevel::Strict;

        assert!(compile(&options).diagnostics.policy().deny_unknown());
    }

    #[test]
    fn every_input_is_read_in_the_order_it_was_given() {
        let first = TempFile::new("safec_driver_order_a.c", "int a;\n");
        let second = TempFile::new("safec_driver_order_b.c", "int b;\n");

        let compiled = compile(&options(vec![
            first.path().to_path_buf(),
            second.path().to_path_buf(),
        ]));

        assert_eq!(compiled.sources.len(), 2);
        let contents: Vec<_> = compiled
            .sources
            .files()
            .map(|(_, file)| file.contents().to_owned())
            .collect();
        assert_eq!(contents, ["int a;\n", "int b;\n"]);
    }

    /// One path named twice is one translation unit. Reading it twice would
    /// report everything in it twice and double the error count a user reads.
    #[test]
    fn the_same_input_twice_is_read_once() {
        let file = TempFile::new("safec_driver_duplicate.c", "int x;\n");
        let path = file.path().to_path_buf();

        let compiled = compile(&options(vec![path.clone(), path]));

        assert_eq!(compiled.sources.len(), 1);
    }

    /// The rule is component equality, which is what `Path` compares, and not
    /// the spelling. A redundant `.` in the middle of a path names the same
    /// file and is caught; a leading `./` is a different first component and is
    /// not. Without both halves the comment above is a claim about behaviour
    /// that nothing holds to, and a later `canonicalize` would arrive looking
    /// like a fix rather than like a change.
    #[test]
    fn two_spellings_are_one_input_only_when_their_components_match() {
        let file = TempFile::new(
            "safec_driver_spelling.c",
            "int x;
",
        );
        let directory = file.path().parent().expect("the file has a parent");
        let name = file.path().file_name().expect("the file has a name");

        let matching = compile(&options(vec![
            directory.join(name),
            directory.join(".").join(name),
        ]));

        assert_eq!(matching.sources.len(), 1);

        let differing = compile(&options(vec![
            PathBuf::from("safec_driver_spelling_relative.c"),
            PathBuf::from("./safec_driver_spelling_relative.c"),
        ]));
        let attempted = differing
            .diagnostics
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.message().contains("cannot read"))
            .count();

        assert_eq!(attempted, 2, "{:?}", differing.diagnostics.diagnostics());
    }

    /// A path that is not there is an ordinary diagnostic. Reaching for
    /// `unwrap` here would answer a typo with a backtrace.
    #[test]
    fn a_missing_input_is_reported_rather_than_panicking() {
        let path = missing_path("safec_driver_no_such_file.c");
        let compiled = compile(&options(vec![path.clone()]));

        let reported = &compiled.diagnostics.diagnostics()[0];
        assert!(
            reported.message().contains(&path.display().to_string()),
            "{:?}",
            reported.message()
        );
        // The message says which file; the note is the only thing that says
        // why, so it carries the actionable half.
        assert!(
            !reported.notes().is_empty(),
            "the reason was dropped: {reported:?}"
        );
        assert!(compiled.diagnostics.has_errors());
        assert!(
            compiled.sources.is_empty(),
            "a failed load must not add a file"
        );
    }

    /// A C file with a Latin-1 comment reads perfectly; only the decoding
    /// fails. Calling that unreadable points the user at permissions.
    ///
    /// The message is asserted rather than the whole diagnostic, because the
    /// message is this compiler's and one of the notes is not: `read_to_string`
    /// contributes `stream did not contain valid UTF-8`. That is `std`'s
    /// wording, which is what keeps the diagnostic out of the corpus, where a
    /// case pins text that is entirely ours.
    ///
    /// Unlike `cannot read`, it does not vary by host: it is a constant in
    /// `std` rather than a message from the operating system, so the reason
    /// `docs/architecture.md` records for that one does not apply here. What
    /// this note follows is the Rust version, not the platform.
    #[test]
    fn a_file_that_is_not_utf8_is_not_reported_as_unreadable() {
        let file = TempFile::new(
            "safec_driver_latin1.c",
            b"/* caf\xE9 */\nint main(void) { return 0; }\n",
        );

        let compiled = compile(&options(vec![file.path().to_path_buf()]));
        let reported = &compiled.diagnostics.diagnostics()[0];

        assert!(
            reported.message().contains("is not valid UTF-8"),
            "{:?}",
            reported.message()
        );
        assert!(!reported.message().contains("cannot read"), "{reported:?}");
        assert!(
            reported.notes().iter().any(|note| note.contains("UTF-8")),
            "{reported:?}"
        );
    }

    /// Stopping at the first bad path would make a user with three of them run
    /// the compiler three times.
    #[test]
    fn every_unreadable_input_is_reported_not_just_the_first() {
        let compiled = compile(&options(vec![
            missing_path("safec_driver_missing_a.c"),
            missing_path("safec_driver_missing_b.c"),
        ]));

        // One per bad path. Every input is attempted, so a user with two of
        // them fixes both after one run rather than after two.
        assert_eq!(compiled.diagnostics.error_count(), 2);
    }

    /// Reachable only from an `Options` the parser did not build, which is what
    /// the Clang adapter will do. Saying only that the pipeline is missing
    /// would be true and beside the point.
    #[test]
    fn a_run_with_no_inputs_says_so() {
        let compiled = compile(&options(Vec::new()));

        assert!(
            compiled.diagnostics.diagnostics()[0]
                .message()
                .contains("no input files"),
            "{:?}",
            compiled.diagnostics.diagnostics()[0].message()
        );
    }

    /// `run_compiler` writes where it is told rather than to a stream a test
    /// cannot read. Without this the render-and-decide half is unguarded.
    #[test]
    fn run_compiler_writes_its_report_to_the_writer_it_was_given() {
        let (report, _) = rendered(&options(vec![missing_path("safec_driver_run_report.c")]));

        assert!(report.contains("cannot read"), "{report}");
        assert!(report.contains("safec_driver_run_report.c"), "{report}");
    }

    /// The outcome is what the process reports, so it has to follow what was
    /// reported rather than being decided some other way.
    ///
    /// A character the lexer refuses, because something has to be reported and
    /// what it is does not matter here. This read `int x;` while
    /// `--emit executable` was a kind nothing could produce: that reported, and
    /// the run failed for a reason the test was not about.
    #[test]
    fn a_run_that_reported_an_error_failed() {
        let file = TempFile::new("safec_driver_outcome.c", "int x = @;\n");
        let (_, outcome) = rendered(&options(vec![file.path().to_path_buf()]));

        assert_eq!(outcome, Outcome::Failed);
    }

    /// Zero for success and one for failure is what `cc` does and what every
    /// build system reads.
    #[test]
    fn an_outcome_maps_to_the_conventional_exit_code() {
        assert_eq!(ExitCode::from(Outcome::Succeeded), ExitCode::SUCCESS);
        assert_eq!(ExitCode::from(Outcome::Failed), ExitCode::FAILURE);
    }

    /// A run that could not write its report has to say so rather than hand
    /// back an outcome. The outcome is what the process reports, and reporting
    /// one for a run whose diagnostics nobody saw claims the user was told.
    #[test]
    fn a_report_that_could_not_be_written_is_an_error() {
        let options = options(vec![missing_path("safec_driver_closed.c")]);

        let error = run_compiler(&options, &mut Closed, &mut Vec::new())
            .expect_err("a failed write must not come back as an outcome");

        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }

    /// The first artifact the compiler can produce. Each line names where a
    /// token starts, what it is, and the text it covers, which is read out of
    /// the source rather than carried by the token. See ADR-0006.
    #[test]
    fn asking_for_tokens_produces_them() {
        let file = TempFile::new("safec_driver_emit_tokens.c", "int x;\n");
        let mut options = options(vec![file.path().to_path_buf()]);
        options.emit = EmitKind::Tokens;

        let (_, artifact, _) = run(&options);

        let lines: Vec<_> = artifact
            .lines()
            .map(|line| line.split_once(' ').unwrap().1)
            .collect();
        assert_eq!(
            lines,
            ["keyword \"int\"", "identifier \"x\"", "punct \";\"", "eof",]
        );
    }

    /// The whole line, not a piece of it. `--emit tokens` is an artifact people
    /// redirect and diff, and `options.rs` calls `--emit` a stable interface
    /// rather than a debug convenience, so the format is the interface: the
    /// position, its order, the separators, the quoting and the trailing
    /// newline. Asserting a substring leaves every one of those free to change.
    #[test]
    fn the_token_dump_has_one_exact_line_per_token() {
        let file = TempFile::new("safec_driver_dump_exact.c", "int x;\n");
        let mut options = options(vec![file.path().to_path_buf()]);
        options.emit = EmitKind::Tokens;

        let (_, artifact, _) = run(&options);

        let name = file.path().display();
        assert_eq!(
            artifact,
            format!(
                "{name}:1:1 keyword \"int\"\n\
                 {name}:1:5 identifier \"x\"\n\
                 {name}:1:6 punct \";\"\n\
                 {name}:2:1 eof\n"
            )
        );
    }

    /// A column counts characters, not bytes, so a caret in an editor lands
    /// where the dump says. A tab counts as one, which is what `LineCol` means
    /// and what the renderer's gutter agrees with.
    #[test]
    fn the_token_dump_counts_columns_in_characters() {
        let file = TempFile::new("safec_driver_dump_columns.c", "\tint caf\u{e9};\n");
        let mut options = options(vec![file.path().to_path_buf()]);
        options.emit = EmitKind::Tokens;

        let (_, artifact, _) = run(&options);

        let columns: Vec<_> = artifact
            .lines()
            .map(|line| line.rsplit(':').next().unwrap().split(' ').next().unwrap())
            .collect();
        // `int` at 2, `caf` at 6, the non-ASCII character at 9, `;` at 10.
        // The character is two bytes and advances the column by one, which is
        // the whole point.
        assert_eq!(columns, ["2", "6", "9", "10", "1"], "{artifact}");
    }

    /// A file's name reaches the terminal on every line of the token dump.
    ///
    /// `dump_tokens` writes it directly rather than through `dump_node`, so
    /// `print.rs`'s `a_file_name_is_escaped_wherever_an_artifact_prints_one`
    /// answers for `--emit ast` and `--emit safety-ir` and not for this one.
    /// A name is content: it comes from a command line today and from a
    /// `#include` later, and RK-002 records what a `.c` file did to somebody's
    /// terminal when its bytes were echoed verbatim.
    ///
    /// Written against `dump_tokens` rather than through `run`, because a file
    /// whose name holds an escape is not a file this platform will create.
    ///
    /// Mutation: write the name with `{}` rather than through `shown`. The
    /// escape reaches the artifact and this fails. Until it was written the
    /// only thing that caught it was `unused import: shown`, which is not a
    /// claim about escaping and stops holding the day a second caller of
    /// `shown` appears in this file.
    #[test]
    fn a_file_name_is_escaped_on_every_line_of_the_token_dump() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("evil\u{1b}[31m.c", "int x;\n");

        let mut diagnostics = DiagnosticSink::new();
        let tokens = lex(file, sources.file(file), &mut diagnostics);

        let mut out = String::new();
        dump_tokens(sources.file(file), &tokens, &mut out);

        assert!(out.contains("evil\\u{1b}"), "{out:?}");
        assert!(!out.contains('\u{1b}'), "{out:?}");
    }

    /// The text is quoted rather than written plainly, which is what keeps a
    /// control character out of a source file from reaching the terminal
    /// through stdout. The renderer answers the same question for diagnostics
    /// and has its own tests; this is the other place a file's text is echoed.
    ///
    /// It also keeps a line parseable: a `"` inside a string literal has to be
    /// escaped or the line's own quoting is ambiguous.
    #[test]
    fn the_token_dump_escapes_the_text_it_quotes() {
        let file = TempFile::new(
            "safec_driver_dump_escape.c",
            "s = \"a\u{1b}[2Jb\"; c = '\\\"';\n",
        );
        let mut options = options(vec![file.path().to_path_buf()]);
        options.emit = EmitKind::Tokens;

        let (_, artifact, _) = run(&options);

        assert!(!artifact.contains('\u{1b}'), "{artifact:?}");
        assert!(
            artifact.contains(r#"string "\"a\u{1b}[2Jb\"""#),
            "{artifact:?}"
        );
    }

    /// Every kind a scan can produce has a word, and the words are what a
    /// reader greps for. An exhaustive `match` makes a missing arm a compile
    /// error; nothing makes a wrong word one.
    #[test]
    fn every_token_kind_is_named_in_the_dump() {
        let file = TempFile::new(
            "safec_driver_dump_kinds.c",
            "#define X 1\nint x = 1 + 'c' + @; char *s = \"t\";\n",
        );
        let mut options = options(vec![file.path().to_path_buf()]);
        options.emit = EmitKind::Tokens;

        let (_, artifact, _) = run(&options);

        let named: Vec<_> = artifact
            .lines()
            .filter_map(|line| line.split(' ').nth(1))
            .collect();
        for kind in [
            "keyword",
            "identifier",
            "number",
            "string",
            "character",
            "punct",
            "directive",
            "unknown",
            "eof",
        ] {
            assert!(named.contains(&kind), "no {kind} in {artifact}");
        }
    }

    /// The first run that can succeed. Until `--emit` reached something the
    /// pipeline produces, every run reported that it had built nothing, so
    /// `Outcome::Succeeded` was unreachable and the branch that returns it was
    /// guarded by nothing.
    #[test]
    fn asking_for_tokens_is_a_run_that_can_succeed() {
        let file = TempFile::new("safec_driver_emit_ok.c", "int x;\n");
        let mut options = options(vec![file.path().to_path_buf()]);
        options.emit = EmitKind::Tokens;

        let (report, artifact, outcome) = run(&options);

        assert_eq!(outcome, Outcome::Succeeded);
        assert_eq!(report, "", "a clean run says nothing");
        assert!(!artifact.is_empty());
    }

    /// A path that cannot be written is reported, and the run fails.
    ///
    /// The other end of `load_failure`'s question, and the reason it is
    /// answered before the diagnostics are rendered: after that there is no
    /// sink left to say it into, and a run that could not write what it was
    /// asked for must not exit zero.
    ///
    /// The path names a directory that does not exist, which every platform
    /// refuses and none refuses in the same words. What is asserted is this
    /// compiler's half of the sentence; the operating system's goes in a note,
    /// the way `cannot read` already carries one.
    ///
    /// Mutation: ignore the `Err` from `fs::write`. The run succeeds and this
    /// fails twice over. Mutation: drop the `with_note` from `write_failure`.
    /// The path is still reported and the reason is not, which is half of what
    /// #89's second acceptance criterion asks for, and nothing else in
    /// the workspace notices.
    #[test]
    fn an_output_path_that_cannot_be_written_is_reported() {
        let file = TempFile::new("safec_driver_output_unwritable.c", "int x;\n");
        let mut options = options(vec![file.path().to_path_buf()]);
        options.emit = EmitKind::Tokens;
        options.output = Some(missing_path("safec_no_such_dir").join("out.tok"));

        let (report, artifact, outcome) = run(&options);

        assert_eq!(outcome, Outcome::Failed);
        assert!(report.contains("cannot write"), "{report}");
        assert!(report.contains("out.tok"), "{report}");
        // That there is a reason, not which reason. The words are the
        // operating system's and differ across the three platforms CI runs;
        // `docs/architecture.md` records that divergence for `cannot read`,
        // which carries its note the same way. Asserting the text would make
        // this a dictionary of other people's error messages.
        assert!(
            report.contains("= note:"),
            "reported the path and not the reason: {report}"
        );
        assert_eq!(
            artifact, "",
            "nothing reaches the stream when a path was given"
        );
    }

    /// A path does not move the diagnostics.
    ///
    /// They are this compiler's speech and go where speech goes, whatever the
    /// artifact does. A reader running `safec -o a.tok x.c` and seeing nothing
    /// would take a failed run for a clean one.
    ///
    /// Mutation: render into the artifact stream. The report is empty and this
    /// fails.
    #[test]
    fn an_output_path_does_not_move_the_diagnostics() {
        let file = TempFile::new("safec_driver_output_says.c", "int x = @;\n");
        let written = TempFile::reserve("safec_driver_output_says.tok");
        let mut options = options(vec![file.path().to_path_buf()]);
        options.emit = EmitKind::Tokens;
        options.output = Some(written.path().to_path_buf());

        let (report, _, outcome) = run(&options);

        assert_eq!(outcome, Outcome::Failed);
        assert!(report.contains("unexpected character"), "{report}");
    }

    /// A run that reported an error still writes what it produced.
    ///
    /// `a_lexical_error_does_not_withhold_the_tokens` decided that for the
    /// stream: the tokens are still what was asked for and still worth reading.
    /// A path changes where the artifact goes and not whether there is one.
    ///
    /// This is a deliberate divergence from `clang`, which leaves no file on a
    /// failed compile and deletes one that was already there: `clang -c b.c -o
    /// probe.o` removed a `probe.o` that a previous run had made. `rustc` is
    /// not the same and is not cited for it: after `E0308` the binary an
    /// earlier run wrote was still there, same size and same mtime. Both
    /// measured on this host rather than recalled. The exit code still says the
    /// run failed, which is what a build system reads.
    ///
    /// Mutation: write only when the sink has no errors. The file is missing
    /// and this fails.
    #[test]
    fn a_failing_run_still_writes_what_it_produced() {
        let file = TempFile::new("safec_driver_output_failing.c", "int x = @;\n");
        let written = TempFile::reserve("safec_driver_output_failing.tok");
        let mut options = options(vec![file.path().to_path_buf()]);
        options.emit = EmitKind::Tokens;
        options.output = Some(written.path().to_path_buf());

        let (_, _, outcome) = run(&options);

        assert_eq!(outcome, Outcome::Failed);
        let text = fs::read_to_string(written.path()).expect("the artifact was written");
        assert!(text.contains("keyword"), "{text}");
    }

    /// A run that produced nothing leaves no file.
    ///
    /// Not an empty one: a build system compares timestamps, and a file created
    /// by a run that made nothing is newer than the source it did not compile.
    ///
    /// A refusal is what makes a run produce nothing now. It was `--emit
    /// llvm-ir` until a backend reached it, `--emit object` until one was
    /// assembled and `--emit executable` until one was linked, and there is no
    /// kind the pipeline cannot reach any more. `Compiled::artifact` is still
    /// an option because of this: a run refused before it reads anything has no
    /// artifact to answer with, which is a different thing from an empty one.
    ///
    /// Mutation: create the file before asking whether there is an artifact.
    /// The file exists and this fails.
    #[test]
    fn a_run_that_produces_nothing_writes_no_file() {
        let first = TempFile::new("safec_driver_output_nothing_a.c", "int x;\n");
        let second = TempFile::new("safec_driver_output_nothing_b.c", "int y;\n");
        let written = TempFile::reserve("safec_driver_output_nothing.o");
        let mut options = options(vec![
            first.path().to_path_buf(),
            second.path().to_path_buf(),
        ]);
        options.emit = EmitKind::Object;
        options.output = Some(written.path().to_path_buf());

        let (report, _, outcome) = run(&options);

        assert_eq!(outcome, Outcome::Failed);
        assert!(report.contains("one input at a time"), "{report}");
        assert!(
            !written.path().exists(),
            "a run that made nothing left {}",
            written.path().display()
        );
    }

    /// The same, for `--emit llvm-ir`, whose artifact has a header in it.
    ///
    /// The header is written where the first unit is rather than where the
    /// artifact is made, so that a run which read nothing leaves an artifact
    /// that is empty rather than one holding a module with no functions. A
    /// header alone would pass the guard above and destroy the file, and it is
    /// worse than a zero-byte one: `clang` accepts a module with nothing in it,
    /// so whatever reads the file next succeeds.
    ///
    /// Mutation: write the header where the artifact is made. The file holds
    /// one `target triple` line and this fails on its contents.
    #[test]
    fn a_module_with_nothing_in_it_does_not_overwrite_the_file() {
        let written = TempFile::new(
            "safec_driver_module_kept.ll",
            "what was there before
",
        );
        let mut options = options(vec![missing_path("safec_driver_module_kept.c")]);
        options.emit = EmitKind::LlvmIr;
        options.output = Some(written.path().to_path_buf());

        let (report, _, outcome) = run(&options);

        assert_eq!(outcome, Outcome::Failed);
        assert!(report.contains("cannot read"), "{report}");
        assert_eq!(
            fs::read_to_string(written.path()).expect("the file is still there"),
            "what was there before
"
        );
    }

    /// `--emit llvm-ir` takes one input at a time, and says so.
    ///
    /// A module is one translation unit. `int f(int);` in one input and
    /// `int f(int x) { ... }` in another is ordinary, correct C, and appending
    /// both into one artifact puts a `declare` beside a `define` of one name,
    /// which LLVM refuses to parse. Saying no beats writing something nothing
    /// can read, and `--emit object` is where one artifact per input arrives.
    ///
    /// Mutation: let the run through. The artifact holds two units, `clang`
    /// refuses it, and this fails on the outcome.
    /// Mutation: answer a fixed kind from `EmitKind::spelling`. The refusal
    /// names `--emit object` for a run that asked for llvm-ir and this fails.
    #[test]
    fn an_llvm_module_is_one_input_at_a_time() {
        let first = TempFile::new(
            "safec_driver_two_units_a.c",
            "int f(int x);
",
        );
        let second = TempFile::new(
            "safec_driver_two_units_b.c",
            "int f(int x) { return x; }
",
        );
        let mut options = options(vec![
            first.path().to_path_buf(),
            second.path().to_path_buf(),
        ]);
        options.emit = EmitKind::LlvmIr;

        let (report, artifact, outcome) = run(&options);

        assert_eq!(outcome, Outcome::Failed);
        assert!(artifact.is_empty(), "{artifact}");
        assert!(
            report.contains("`--emit llvm-ir` takes one input at a time"),
            "{report}"
        );
        // The other kinds are a dump rather than a module, and appending is
        // what they are for.
        options.emit = EmitKind::SafetyIr;
        assert_eq!(run(&options).2, Outcome::Succeeded);
    }

    /// A refusal the IR could not place says so rather than pointing anywhere.
    ///
    /// Only a call among the terminators carries a span, so a refusal about one
    /// of the others has nothing to point at. Nothing the frontend builds
    /// reaches it, which is why this asks `backend_failure` directly rather
    /// than compiling something.
    ///
    /// Mutation: drop the note. The diagnostic says what could not be written
    /// and nothing about why it points nowhere, and this fails.
    #[test]
    fn a_refusal_with_nowhere_to_point_says_so() {
        let reported = backend_failure(&Refusal {
            why: "an edge no statement produced, which has no LLVM spelling".to_owned(),
            at: None,
        });

        assert!(reported.primary_label().is_none(), "{reported:?}");
        assert_eq!(
            reported.notes(),
            ["the IR does not say where this came from"]
        );
    }

    /// A run that could not read anything does not empty the file it was given.
    ///
    /// `--emit tokens` answers `Some("")` for an input it never opened, so this
    /// reaches the arm above through a path rather than through a missing
    /// artifact, and writing it would leave exactly the zero-byte file newer
    /// than every source that the arm above exists to avoid.
    ///
    /// Mutation: drop the `is_empty` and `has_errors` guard on the write. The
    /// file is truncated and this fails on its contents.
    #[test]
    fn a_run_that_read_nothing_does_not_empty_the_file_it_was_given() {
        let written = TempFile::new("safec_driver_output_kept.tok", "what was there before\n");
        let mut options = options(vec![missing_path("safec_driver_output_kept.c")]);
        options.emit = EmitKind::Tokens;
        options.output = Some(written.path().to_path_buf());

        let (report, _, outcome) = run(&options);

        assert_eq!(outcome, Outcome::Failed);
        assert!(report.contains("cannot read"), "{report}");
        assert_eq!(
            fs::read_to_string(written.path()).expect("the file is still there"),
            "what was there before\n"
        );
    }

    /// A stream that stopped being read does not take the report with it.
    ///
    /// This is the ordering the doc comment on `run_compiler` promises: a user
    /// piping the artifact into something that exits early still learns why the
    /// run failed. A path is the other way round and is written first, because
    /// a path that cannot be written is a diagnostic and needs the report.
    ///
    /// Mutation: write the stream inside the `match`, above `render_all`. The
    /// write fails, `?` returns, and the report arrives empty.
    #[test]
    fn a_closed_artifact_stream_does_not_swallow_the_report() {
        let file = TempFile::new("safec_driver_closed_report.c", "@\n");
        let mut options = options(vec![file.path().to_path_buf()]);
        options.emit = EmitKind::Tokens;
        options.color = ColorMode::Never;
        let mut report = Vec::new();

        let error = run_compiler(&options, &mut report, &mut Closed)
            .expect_err("a failed write must not come back as an outcome");

        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        let said = String::from_utf8(report).expect("the renderer writes UTF-8");
        assert!(said.contains("unexpected character"), "{said}");
    }

    /// Every input appends to one file, which is what standard output does.
    ///
    /// `every_input_appends_to_one_artifact` is the same claim for the stream,
    /// and a path changes nothing about it: there is one artifact per run here
    /// rather than one per input, so `-o` names it unambiguously. `clang`
    /// refuses the same spelling because it writes one output per input, and
    /// `--emit object` is where that distinction arrives.
    ///
    /// Mutation: truncate per input rather than writing once at the end. Only
    /// the last input's tokens are in the file and this fails.
    #[test]
    fn every_input_appends_to_one_file() {
        let first = TempFile::new("safec_driver_output_one.c", "int a;\n");
        let second = TempFile::new("safec_driver_output_two.c", "int b;\n");
        let written = TempFile::reserve("safec_driver_output_both.tok");
        let mut options = options(vec![
            first.path().to_path_buf(),
            second.path().to_path_buf(),
        ]);
        options.emit = EmitKind::Tokens;
        options.output = Some(written.path().to_path_buf());

        let (_, _, outcome) = run(&options);

        assert_eq!(outcome, Outcome::Succeeded);
        let text = fs::read_to_string(written.path()).expect("the artifact was written");
        assert!(text.contains("\"a\""), "{text}");
        assert!(text.contains("\"b\""), "{text}");
    }

    /// The rule, over every kind there is: a run over a program this compiler
    /// can compile answers with something for every one of them. Driven by
    /// `EmitKind::value_variants` rather than by a list, so a kind added
    /// anywhere in the pipeline is covered without anyone remembering this test
    /// exists, which is the only guard of that shape here: `E0004` makes
    /// somebody write an arm and nothing makes the arm they write right, and a
    /// total roster is what answers for a variant nobody has written yet. See
    /// RK-015.
    ///
    /// It held that a kind either produced what was asked for **or said it
    /// could not**, which was the weaker half and is gone with the kind that
    /// needed it: there is no longer a `--emit` the pipeline cannot reach, so
    /// an empty answer is a defect rather than a state.
    ///
    /// The MVP rather than a declaration, because two of these kinds link and a
    /// translation unit with no `main` is not a program. For the host's own
    /// machine, for the same reason: one of them links, and nothing links for
    /// another machine without a toolchain for it.
    ///
    /// Mutation: leave `out` alone in any one arm of the loop in `compile`.
    /// That kind answers nothing and this fails naming it.
    #[test]
    fn every_emit_kind_makes_something_of_a_program() {
        if !clang_or_skip("what every emit kind makes of a program") {
            return;
        }

        let file = TempFile::new(
            "safec_driver_emit_every.c",
            "int add(int a, int b) { return a + b; }
int main(void) { return add(1, 2); }
",
        );

        for &emit in EmitKind::value_variants() {
            let mut options = options(vec![file.path().to_path_buf()]);
            options.emit = emit;
            // The machine this is running on, and not the one the helper names.
            // Every other kind is the same work for any target, and one of them
            // links: assembling for another machine needs no sysroot and
            // linking for one needs a toolchain no runner has, so a fixed
            // triple here asks three runners a question only one of them can
            // answer. CI found this rather than a reader.
            options.target = Target::from_triple(HOST_TRIPLE).expect("the host is a known triple");

            let compiled = compile(&options);

            assert!(
                !compiled.diagnostics.has_errors(),
                "{emit:?}: {:?}",
                compiled.diagnostics.diagnostics()
            );
            assert!(
                compiled.artifact.is_some_and(|made| !made.is_empty()),
                "{emit:?} answered nothing"
            );
        }
    }

    /// A reader of one must not be handed the other. The binary sends them to
    /// stdout and stderr, so `safec --emit tokens a.c > a.tok` captures the
    /// tokens and leaves the diagnostics on the terminal.
    #[test]
    fn the_artifact_and_the_report_go_to_different_streams() {
        let file = TempFile::new("safec_driver_emit_streams.c", "int x = @;\n");
        let mut options = options(vec![file.path().to_path_buf()]);
        options.emit = EmitKind::Tokens;

        let (report, artifact, _) = run(&options);

        assert!(report.contains("unexpected character"), "{report}");
        assert!(!report.contains("keyword"), "{report}");
        assert!(artifact.contains("keyword"), "{artifact}");
        assert!(!artifact.contains("unexpected character"), "{artifact}");
    }

    /// The scan keeps going, so it still has tokens to hand over, and they are
    /// still what was asked for. The run fails on the diagnostics rather than
    /// by withholding the artifact.
    #[test]
    fn a_lexical_error_does_not_withhold_the_tokens() {
        let file = TempFile::new("safec_driver_emit_broken.c", "int x = @;\n");
        let mut options = options(vec![file.path().to_path_buf()]);
        options.emit = EmitKind::Tokens;

        let (_, artifact, outcome) = run(&options);

        assert_eq!(outcome, Outcome::Failed);
        assert!(artifact.contains("unknown \"@\""), "{artifact}");
    }

    /// One dump for the run, not one per input. Every line names its file, so
    /// the concatenation is unambiguous and there is one thing to redirect.
    #[test]
    fn every_input_appends_to_one_artifact() {
        let first = TempFile::new("safec_driver_emit_a.c", "int a;\n");
        let second = TempFile::new("safec_driver_emit_b.c", "int b;\n");
        let mut options = options(vec![
            first.path().to_path_buf(),
            second.path().to_path_buf(),
        ]);
        options.emit = EmitKind::Tokens;

        let (_, artifact, _) = run(&options);

        assert!(artifact.contains("safec_driver_emit_a.c"), "{artifact}");
        assert!(artifact.contains("safec_driver_emit_b.c"), "{artifact}");
        assert_eq!(artifact.lines().count(), 8, "{artifact}");
    }

    /// The same for the tree, which reaches the artifact by a different path:
    /// the tokens dump writes what the loop already has, and this one parses
    /// first, so the two arms cannot vouch for each other.
    ///
    /// Mutation: clear the artifact before dumping each input. This fails,
    /// because the second file's tree is then all there is.
    ///
    /// It also pins that each node is placed against the file its own span
    /// names rather than the file the loop is on, which is the property the
    /// second name below would lose.
    #[test]
    fn every_input_appends_to_one_tree_dump() {
        let first = TempFile::new(
            "safec_driver_ast_a.c",
            "int a(void) { return 0; }
",
        );
        let second = TempFile::new(
            "safec_driver_ast_b.c",
            "int b(void) { return 1; }
",
        );
        let mut options = options(vec![
            first.path().to_path_buf(),
            second.path().to_path_buf(),
        ]);
        options.emit = EmitKind::Ast;

        let (_, artifact, _) = run(&options);

        assert!(artifact.contains("safec_driver_ast_a.c"), "{artifact}");
        assert!(artifact.contains("safec_driver_ast_b.c"), "{artifact}");
        assert!(artifact.contains("\"a\""), "{artifact}");
        assert!(artifact.contains("\"b\""), "{artifact}");
        assert_eq!(artifact.lines().count(), 8, "{artifact}");
    }

    /// Each node is placed against the file its own span names.
    ///
    /// One tree holds spans from one file today, and will hold several the
    /// moment `#include` lands. Resolving them all against whichever file the
    /// driver's loop happens to be on prints one file's text at another file's
    /// line, and panics outright when the other file is shorter. The renderer
    /// looks a label's file up per label for the same reason; ADR-0003 argues
    /// it. The tree here is built by hand because the parser cannot yet produce
    /// one that spans two files.
    ///
    /// Mutation: have `dump_node` and `quoted` take the loop's `SourceFile`
    /// again. This fails.
    #[test]
    fn a_node_is_placed_against_the_file_its_span_names() {
        use crate::ast::Function;

        let mut sources = SourceMap::new();
        let first = sources.add_virtual(
            "first.c",
            "int outer(void) { return 0; }
",
        );
        let second = sources.add_virtual(
            "second.c",
            "


      inner
",
        );

        let mut ast = Ast::new();
        let body = ast.push_stmt(Stmt::Compound {
            body: Vec::new(),
            span: Span::new(second, 9, 14),
        });
        let ty = ast.push_type(Type::Int);
        ast.push_item(Item::Function(Function {
            ty,
            name: Span::new(first, 4, 9),
            body,
            span: Span::new(first, 0, 29),
        }));

        let mut out = String::new();
        dump_ast(&sources, &ast, &mut out);

        assert_eq!(
            out,
            "Function <first.c>:1:1 \"outer\" \"int\"
  Compound <second.c>:4:7
"
        );
    }

    /// The artifact is what was asked for, so failing to write it is a failed
    /// run even though every diagnostic was delivered.
    #[test]
    fn an_artifact_that_could_not_be_written_is_an_error() {
        let file = TempFile::new("safec_driver_emit_closed.c", "int x;\n");
        let mut options = options(vec![file.path().to_path_buf()]);
        options.emit = EmitKind::Tokens;

        let error = run_compiler(&options, &mut Vec::new(), &mut Closed)
            .expect_err("a failed write must not come back as an outcome");

        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }

    /// Colour is asked for through the options and has to reach the output.
    /// Every diagnostic the driver produces today is unanchored, so this covers
    /// the path that renders without pointing at source.
    #[test]
    fn the_colour_mode_reaches_the_output() {
        let mut options = options(vec![missing_path("safec_driver_colour.c")]);

        options.color = ColorMode::Never;
        let (plain, _) = rendered(&options);
        options.color = ColorMode::Always;
        let (coloured, _) = rendered(&options);

        assert!(!plain.contains('\u{1b}'), "{plain:?}");
        assert!(coloured.contains('\u{1b}'), "{coloured:?}");
    }

    /// Every file that loaded is scanned, whatever happened to the others.
    /// Gating on the run would make a user fix one file per run.
    #[test]
    fn a_bad_input_does_not_stop_the_others_being_scanned() {
        let scanned = TempFile::new("safec_driver_still_scanned.c", "int x = @;\n");
        let compiled = compile(&options(vec![
            missing_path("safec_driver_absent_first.c"),
            scanned.path().to_path_buf(),
        ]));

        let messages: Vec<_> = compiled
            .diagnostics
            .diagnostics()
            .iter()
            .map(Diagnostic::message)
            .collect();

        assert!(
            messages.iter().any(|m| m.contains("cannot read")),
            "{messages:?}"
        );
        assert!(
            messages.iter().any(|m| m.contains("unexpected character")),
            "{messages:?}"
        );
    }

    /// A lexical error stops the input it is in, and stops nothing else.
    ///
    /// Two things at once, because they are two halves of one rule. The file
    /// the lexer could not read whole is not parsed, so its one problem
    /// produces one diagnostic rather than a syntax error behind it. And the
    /// other file is read all the way through, because the gate is on the
    /// input rather than on the run.
    ///
    /// Mutation: gate on `diagnostics.has_errors()` rather than on the count
    /// taken either side of this input's own work. The second file stops being
    /// parsed because the first one failed, its tree leaves the artifact, and
    /// this fails. Mutation: take the `continue` out of the `Emitted::Ast`
    /// arm. The broken file is parsed after all, a second diagnostic arrives,
    /// and this fails from the other side.
    #[test]
    fn a_lexical_error_stops_that_input_and_no_other() {
        let broken = TempFile::new("safec_driver_gate_broken.c", "int a(void) { return @; }\n");
        let whole = TempFile::new("safec_driver_gate_whole.c", "int b(void) { return 1; }\n");

        let mut options = options(vec![
            broken.path().to_path_buf(),
            whole.path().to_path_buf(),
        ]);
        options.emit = EmitKind::Ast;

        let compiled = compile(&options);

        // The code rather than the message, because a message is what gets
        // reworded and a code is what does not.
        let codes: Vec<_> = compiled
            .diagnostics
            .diagnostics()
            .iter()
            .filter_map(|diagnostic| diagnostic.code())
            .map(|code| code.to_string())
            .collect();
        assert_eq!(
            codes,
            ["SC0103"],
            "{:?}",
            compiled.diagnostics.diagnostics()
        );

        // Text, because every kind but `--emit object` is UTF-8 this compiler
        // wrote. `Compiled::artifact` is bytes for the one that is not.
        let artifact = String::from_utf8(compiled.artifact.expect("`--emit ast` produces one"))
            .expect("`--emit ast` writes text");
        assert!(artifact.contains("safec_driver_gate_whole.c"), "{artifact}");
        assert!(
            !artifact.contains("safec_driver_gate_broken.c"),
            "{artifact}"
        );
    }

    #[test]
    fn a_lexical_error_in_one_input_does_not_hide_one_in_another() {
        let first = TempFile::new("safec_driver_lex_a.c", "int a = @;\n");
        let second = TempFile::new("safec_driver_lex_b.c", "int b = `;\n");

        let compiled = compile(&options(vec![
            first.path().to_path_buf(),
            second.path().to_path_buf(),
        ]));

        let unexpected = compiled
            .diagnostics
            .diagnostics()
            .iter()
            .filter(|d| d.message().contains("unexpected character"))
            .count();

        assert_eq!(unexpected, 2);
    }

    /// Until now every diagnostic the compiler produced was unanchored, so
    /// `render_all` never reached the source map at all and the driver's tests
    /// passed against an empty one. The scan is what raises the first labelled
    /// diagnostic; what is under test here is the driver's wiring, that the map
    /// it loaded is the map the renderer is given.
    #[test]
    fn a_labelled_diagnostic_reaches_the_source_map_the_driver_loaded() {
        let file = TempFile::new("safec_driver_quotes_source.c", "int x = @;\n");
        let mut report = Vec::new();

        run_compiler(
            &options(vec![file.path().to_path_buf()]),
            &mut report,
            &mut Vec::new(),
        )
        .expect("writing to a vector cannot fail");
        let report = String::from_utf8(report).expect("the renderer writes text");

        assert!(report.contains("int x = @;"), "{report}");
        assert!(report.contains("not part of any token"), "{report}");
    }
}
