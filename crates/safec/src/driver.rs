//! The driver: everything between a parsed command line and an exit code.
//!
//! The phases each know how to do one thing. What is decided here is the order
//! they happen in: which files are read, when diagnostics are rendered, and
//! what the process reports. Keeping that in one place is what lets a phase be
//! written without an opinion about the ones around it.
//!
//! [`compile`] does the work and hands back what it found, so that a caller can
//! read the diagnostics rather than scrape them out of a stream. [`run_compiler`] is the
//! half that reports and decides the outcome.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::io;
use std::path::Path;
use std::process::ExitCode;

use crate::diagnostics::render::Renderer;
use crate::diagnostics::{Diagnostic, DiagnosticSink, Policy};
use crate::lexer::lex;
use crate::options::{EmitKind, Options};
use crate::source::{SourceFile, SourceMap};
use crate::token::Token;

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
    /// `None` means the run was asked for something it cannot produce yet, and
    /// the diagnostics say so. It does not mean nothing was written: a run that
    /// reported a lexical error still emits the tokens, because they are still
    /// what was asked for and they are still worth reading.
    pub artifact: Option<String>,
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
/// Phase 0 has one phase, so nothing here is gated on what an earlier one
/// found. That changes with the lexer: the moment `compile` does per-input work
/// beyond loading it, the gate for that work goes on the input and not on the
/// run. [`DiagnosticSink::has_errors`] answers whether anything in the whole run
/// failed, so gating the lexer on it would let a typo in `a.c` decide that `b.c`
/// is never looked at, and a user with two broken files would fix them one run
/// at a time. The run-level answer is for what genuinely spans the run: the
/// outcome, and later, linking.
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

    // The gate is the input, not the run. `DiagnosticSink::has_errors` answers
    // for everything reported so far, so consulting it here would let a typo in
    // `a.c` decide that `b.c` is never looked at, and a user with two broken
    // files would fix them one run at a time. What genuinely spans the run is
    // the outcome, and later, linking.
    let mut artifact = (options.emit == EmitKind::Tokens).then(String::new);
    for &file in &loaded {
        // `file_owned` rather than `file`: the scan holds its text for longer
        // than a statement, and adds to the map while doing so once `#include`
        // lands. See ADR-0005.
        let source = sources.file_owned(file);
        let tokens = lex(file, &source, &mut diagnostics);

        // Every input appends to one artifact, and every line names its file,
        // so `--emit tokens a.c b.c` reads as one dump rather than needing two
        // destinations.
        if let Some(artifact) = &mut artifact {
            dump_tokens(&source, &tokens, artifact);
        }
    }

    // What the pipeline can produce ends at the token stream. `EmitKind` is
    // declared in pipeline order, so this asks whether what was requested lies
    // beyond the last stage that exists rather than naming the stages that do
    // not, and it stops being true one variant at a time as they land.
    //
    // Said whenever it applies, including on a run that also failed to read a
    // file, because a run that compiled nothing has to say so: leaving it out
    // lets a user with one bad path among several believe the rest were built.
    // Exiting successfully without producing what was asked for is the one
    // thing a compiler must never do.
    if options.emit > EmitKind::Tokens {
        diagnostics.report(
            Diagnostic::error("the compilation pipeline is not implemented yet")
                .with_note("safec currently reads its inputs and scans them into tokens")
                .with_note("`--emit tokens` is what it can produce today"),
        );
    }

    Compiled {
        sources,
        diagnostics,
        artifact,
    }
}

/// One line per token: where it starts, what it is, and the text it covers.
///
/// The text is quoted rather than written plainly. It comes out of the file, so
/// it is content, and this goes to a terminal: quoting escapes a control
/// character rather than obeying it, for the same reason the renderer does not
/// echo one, and it makes a token legible whose text is a space or a newline.
///
/// A position rather than a span. The end of a token is where the next one
/// starts, and a dump is read down its left edge.
fn dump_tokens(file: &SourceFile, tokens: &[Token], out: &mut String) {
    for token in tokens {
        let at = file.line_col(token.span.start());
        write!(
            out,
            "{}:{}:{} {}",
            file.name(),
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
/// The report goes first. If the artifact is being piped into something that
/// stops reading, the write fails, and a user who loses the diagnostics as well
/// learns nothing about why.
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
    let compiled = compile(options);

    Renderer::new(options.color).render_all(&compiled.sources, &compiled.diagnostics, report)?;
    if let Some(emitted) = &compiled.artifact {
        artifact.write_all(emitted.as_bytes())?;
    }

    Ok(if compiled.diagnostics.has_errors() {
        Outcome::Failed
    } else {
        Outcome::Succeeded
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;
    use crate::options::{ColorMode, EmitKind};
    use crate::safety::SafetyLevel;

    fn options(inputs: Vec<PathBuf>) -> Options {
        Options {
            inputs,
            output: None,
            safety: SafetyLevel::Memory,
            emit: EmitKind::Executable,
            deny_unknown: false,
            color: ColorMode::Never,
        }
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

        // Two bad paths, plus the statement that nothing was compiled.
        assert_eq!(compiled.diagnostics.error_count(), 3);
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

    /// The pipeline does not exist, so no artifact was produced, so the run
    /// failed. Reporting success here would be a positive claim that the
    /// compilation happened.
    #[test]
    fn a_compilation_that_produces_no_artifact_reports_an_error() {
        let file = TempFile::new("safec_driver_nothing_yet.c", "int main(void) { return 0; }");
        let compiled = compile(&options(vec![file.path().to_path_buf()]));

        assert!(compiled.diagnostics.has_errors());
        assert!(
            compiled
                .diagnostics
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.message().contains("not implemented")),
        );
    }

    /// A run that failed to read one of its inputs has still compiled nothing,
    /// and has to say so. Leaving it out lets a user with one bad path among
    /// several believe the files that did load were checked.
    #[test]
    fn a_failed_load_is_still_told_that_nothing_was_compiled() {
        let good = TempFile::new("safec_driver_good.c", "int x;\n");
        let compiled = compile(&options(vec![
            good.path().to_path_buf(),
            missing_path("safec_driver_absent.c"),
        ]));

        let messages: Vec<_> = compiled
            .diagnostics
            .diagnostics()
            .iter()
            .map(Diagnostic::message)
            .collect();

        assert!(
            messages
                .iter()
                .any(|message| message.contains("cannot read")),
            "{messages:?}"
        );
        assert!(
            messages
                .iter()
                .any(|message| message.contains("not implemented")),
            "{messages:?}"
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
    #[test]
    fn a_run_that_reported_an_error_failed() {
        let file = TempFile::new("safec_driver_outcome.c", "int x;\n");
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

    /// Everything past the lexer. A run that cannot produce what was asked for
    /// has to say so rather than exit successfully having made nothing.
    #[test]
    fn asking_for_an_artifact_the_pipeline_cannot_reach_is_an_error() {
        let file = TempFile::new("safec_driver_emit_beyond.c", "int x;\n");

        for emit in [
            EmitKind::Ast,
            EmitKind::SafetyIr,
            EmitKind::LlvmIr,
            EmitKind::Object,
            EmitKind::Executable,
        ] {
            let mut options = options(vec![file.path().to_path_buf()]);
            options.emit = emit;

            let compiled = compile(&options);

            assert!(compiled.artifact.is_none(), "{emit:?}");
            assert!(compiled.diagnostics.has_errors(), "{emit:?}");
            assert!(
                compiled
                    .diagnostics
                    .diagnostics()
                    .iter()
                    .any(|diagnostic| diagnostic.message().contains("not implemented")),
                "{emit:?}"
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

    /// Until now every diagnostic the driver produced was unanchored, so
    /// `render_all` never reached the source map at all: the driver's tests
    /// passed against an empty one. The first labelled diagnostic is what
    /// closes that, and this is the test that keeps it closed.
    #[test]
    fn a_diagnostic_from_the_driver_quotes_the_source_it_points_at() {
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
