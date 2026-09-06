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
use std::io;
use std::path::Path;
use std::process::ExitCode;

use crate::diagnostics::render::Renderer;
use crate::diagnostics::{Diagnostic, DiagnosticSink, Policy};
use crate::options::Options;
use crate::source::SourceMap;

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
    for input in &options.inputs {
        // The same path twice is one translation unit, not two: reading it
        // twice would report everything in it twice. `Path` compares by
        // component rather than by spelling, so `dir/./a.c` is caught along
        // with an identical spelling. `a.c` and `./a.c` are still two, and so
        // are `main.c` and `MAIN.C` on a filesystem that says otherwise.
        // Deciding when two paths name one file belongs to the source map, and
        // `#include` is what will make it worth deciding.
        if !seen.insert(input.as_path()) {
            continue;
        }

        // Every input is attempted. Reporting the first bad path and stopping
        // would make a user with three of them run the compiler three times.
        if let Err(error) = sources.load(input) {
            diagnostics.report(load_failure(input, &error));
        }
    }

    // Phase 0 ends here: the files are read, and there is nothing yet to read
    // them with. Said on every run rather than only on one that loaded cleanly,
    // because a run that also failed to read a file has still compiled nothing,
    // and leaving it out lets a user with one bad path among several believe
    // the others were checked. Exiting successfully without producing what was
    // asked for is the one thing a compiler must never do.
    diagnostics.report(
        Diagnostic::error("the compilation pipeline is not implemented yet")
            .with_note("safec currently reads its inputs and reports on them")
            .with_note("the lexer and parser arrive in phase 1"),
    );

    Compiled {
        sources,
        diagnostics,
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
/// # Errors
///
/// If the diagnostics could not be written. There is nothing left to report
/// that on, so the caller has only the outcome to say it with.
pub fn run_compiler(options: &Options, report: &mut impl io::Write) -> io::Result<Outcome> {
    let compiled = compile(options);

    Renderer::new(options.color).render_all(&compiled.sources, &compiled.diagnostics, report)?;

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

    fn rendered(options: &Options) -> (String, Outcome) {
        let mut out = Vec::new();
        let outcome = run_compiler(options, &mut out).expect("writing to a vector cannot fail");
        (
            String::from_utf8(out).expect("the renderer writes text"),
            outcome,
        )
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

        let error = run_compiler(&options, &mut Closed)
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
}
