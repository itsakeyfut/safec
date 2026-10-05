use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use clap::ValueEnum as _;

use super::backend::{Scratch, Unmade, backend_failure};
use super::*;
use crate::ast::{Item, Stmt, Type};
use crate::cli::HOST_TRIPLE;
use crate::options::{ColorMode, EmitKind};
use crate::safety::SafetyLevel;
use safec_ir::source::Span;
use safec_ir::target::Target;
use safec_llvm::emit::Refusal;

/// Answer `true` where a test that needs `clang` should go on, and say what
/// it skipped where there is none.
///
/// The same gate `tests/object.rs` has, for the same reason and not shared
/// with it: an integration test is its own crate. A check that cannot fail
/// loudly reports the state it was asked to prove, so
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

/// An invocation for the tests that are about reading inputs and producing
/// artifacts rather than about safety.
///
/// `Off` beside a `Tokens` artifact because the pair has to be coherent: a
/// run asking for `memory` from an artifact that stops before the IR is
/// asking for a level it cannot be given, and `undelivered` reports it. That
/// is what `Cli::into_options` resolves an unnamed `--safety` to for this
/// kind, so the fixture says what the command line would have said. Six
/// tests here failed on the day the report landed, every one of them on this
/// line rather than on its own subject.
fn options(inputs: Vec<PathBuf>) -> Options {
    Options {
        inputs,
        output: None,
        safety: SafetyLevel::Off,
        // The cheapest kind that still runs every stage of the loop. It
        // was `Executable` while nothing could produce one, which made it
        // the kind that did nothing; now the two kinds past `llvm-ir`
        // spawn `clang`, and a test about reading inputs should not need
        // one on the machine.
        emit: EmitKind::Tokens,
        target: Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
        allow_unknown: false,
        color: ColorMode::Never,
    }
}

/// What a module is called, for both kinds of name there are.
///
/// Written out rather than derived, because the answer is what a linker
/// quotes back at a user and an arm nobody wrote down is an arm anybody can
/// change. `Virtual` is not reachable from the command line, since every
/// input is a path, and it is here because the match has to answer for it
/// and because the interpreter's tests build one.
///
/// Mutation: answer the whole file name rather than its stem. The first row
/// fails. Mutation: answer nothing for a name with no path. The last fails.
#[test]
fn a_module_is_called_what_its_input_is_called() {
    assert_eq!(stem(&FileName::Real(PathBuf::from("add.c"))), "add");
    assert_eq!(stem(&FileName::Real(PathBuf::from("sub/add.c"))), "add");
    assert_eq!(stem(&FileName::Real(PathBuf::from("a.tar.c"))), "a.tar");
    // A name that is all extension keeps the lot: `Path::file_stem`
    // answers `Some(".c")` here, not `None`, because a leading dot with
    // nothing after it is a name rather than an extension. Measured, and
    // written down because the obvious reading is the other one.
    assert_eq!(stem(&FileName::Real(PathBuf::from(".c"))), ".c");
    // What `file_stem` really has no answer for, and where the fallback
    // is. Neither can be opened, so neither reaches this from a run.
    assert_eq!(stem(&FileName::Real(PathBuf::from(".."))), "..");
    assert_eq!(stem(&FileName::Real(PathBuf::from("."))), ".");
    assert_eq!(stem(&FileName::Virtual("held".to_owned())), "held");
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
/// cannot fail loudly reports the state it was asked to prove.
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
    let outcome =
        run_compiler(options, &mut report, &mut artifact).expect("writing to a vector cannot fail");
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

/// The wiring ADR-0001 is about. Without it `--allow-unknown` parses,
/// documents itself, and does nothing.
///
/// The name says "the options the run was given" rather than "the resolved
/// options", which is what these deliberately are not since #209: the
/// field is the request, and `Policy::new` is where it becomes an answer.
///
/// The level and the artifact are set here rather than taken from the
/// fixture, which asks for nothing: `Policy::new` reads the level, so a run
/// at `Off` has nothing to deny and both rows would answer `false` whatever
/// the sink was built from. `SafetyIr` beside it keeps the pair coherent, so
/// that this fixture is not also asking for a level its artifact cannot
/// deliver.
///
/// Mutation: build the sink with `DiagnosticSink::new()`. Both rows fail,
/// because this compiles at `SafetyLevel::Memory`, where the default is to
/// deny.
#[test]
fn the_sink_is_built_from_the_options_the_run_was_given() {
    for allow_unknown in [false, true] {
        let mut options = options(Vec::new());
        options.safety = SafetyLevel::Memory;
        options.emit = EmitKind::SafetyIr;
        options.allow_unknown = allow_unknown;

        assert_eq!(
            compile(&options).diagnostics.policy().deny_unknown(),
            !allow_unknown,
        );
    }
}

/// The strictest level is defined as leaving nothing `Unknown`, so it
/// denies them in the sink even where the run asked to keep them. The level
/// itself does not reach the sink: it holds a [`Policy`] and never learns
/// which checks ran.
#[test]
fn the_strictest_safety_level_denies_unknown_in_the_sink() {
    let mut options = options(Vec::new());
    options.safety = SafetyLevel::Strict;
    options.allow_unknown = true;

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
/// `#include` later, and a `.c` file here has already cleared somebody's
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

/// A file this run could not write is still the user's.
///
/// The other way of not writing, and the one the compiler used to answer by
/// deleting. A read-only file is one this compiler can delete and cannot
/// write, because removing a directory entry needs permission over the
/// directory and writing needs it over the file, so the open refuses with
/// nothing of ours on disk and what is there is what the user put there.
/// `Permissions::set_readonly` is the one spelling both platforms have, and
/// `fs::remove_file` takes a read-only file away on Windows as well,
/// measured, so `TempFile` still clears up after this.
///
/// `cannot write` is asserted rather than assumed. On a machine where the
/// write succeeds anyway, root on Unix being the one that happens, this
/// fails rather than passing over a setup that did not take.
///
/// This is the refused open. The other side of the rule, a failure after
/// the open, is what takes the file away, and nothing holds it: it wants a
/// write that fails on a file this process has just created and owns.
///
/// Mutation: take the file away on any failure again, by answering `true`
/// where the open is mapped to `false`. This fails where it reads the file
/// back, on `NotFound` rather than on the contents, and it is the only test
/// in the workspace that fails.
#[test]
fn a_read_only_output_file_is_still_there_after_a_run_that_could_not_write_it() {
    let file = TempFile::new("safec_driver_output_read_only.c", "int x;\n");
    let written = TempFile::new(
        "safec_driver_output_read_only.tok",
        "what was there before\n",
    );
    let mut permissions = fs::metadata(written.path())
        .expect("the file was just written")
        .permissions();
    permissions.set_readonly(true);
    fs::set_permissions(written.path(), permissions).expect("the file is this test's own");

    let mut options = options(vec![file.path().to_path_buf()]);
    options.emit = EmitKind::Tokens;
    options.output = Some(written.path().to_path_buf());

    let (report, _, outcome) = run(&options);

    assert_eq!(outcome, Outcome::Failed);
    assert!(report.contains("cannot write"), "{report}");
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

/// The level is a fact about the invocation, so it is reported once and
/// before anything an input has to say.
///
/// **A corpus case cannot hold either half.** The harness runs one file per
/// case, so a report made per input and a report made once per run look
/// identical to every case there is, and with one diagnostic in the output
/// there is no position to be wrong about. Two inputs with a lexical error
/// each is the smallest thing that separates them.
///
/// Mutation: report inside the per-input loop rather than once before it.
/// The count fails with two, and nothing else in the workspace fails.
///
/// Mutation: report after the input loop instead of before it. The position
/// fails with two, and again nothing else does. A reader meeting the
/// paragraph after a screenful of findings has no way to tell it is not
/// about the last of them.
#[test]
fn the_undelivered_level_is_reported_once_for_the_run_and_before_its_inputs() {
    // Two inputs, each with a lexical error of its own, so that the
    // undelivered report has something to be counted against and something
    // to be positioned relative to. A corpus case cannot hold this: the
    // harness runs one file per case, so a report made per input and a
    // report made per run look identical to every one of them.
    let first = TempFile::new("safec_undelivered_one.c", "int a = \"unterminated;\n");
    let second = TempFile::new("safec_undelivered_two.c", "int b = \"unterminated;\n");
    let mut options = options(vec![
        first.path().to_path_buf(),
        second.path().to_path_buf(),
    ]);
    options.safety = SafetyLevel::Strict;

    let reported = compile(&options).diagnostics;
    let reported = reported.diagnostics();
    let undelivered: Vec<usize> = reported
        .iter()
        .enumerate()
        .filter(|(_, diagnostic)| diagnostic.message().starts_with("`--safety strict` asks"))
        .map(|(at, _)| at)
        .collect();

    assert_eq!(
        undelivered.len(),
        1,
        "the level is a fact about the invocation, not about an input: {reported:?}"
    );
    assert_eq!(undelivered[0], 0, "{reported:?}");
    assert!(
        reported.len() > 1,
        "the inputs reported nothing, so neither assertion above was tested"
    );
}

/// The level this compiler says it implements runs a check the level below it
/// does not.
///
/// **This is what stops [`SafetyLevel::IMPLEMENTED`] moving ahead of the
/// checks.** Moving it alone leaves `undelivered` silent about a level
/// nothing checks, and silence there *means* the level was delivered, so
/// `int *foo(void) { int x = 42; return &x; }` at `--safety lifetime` would
/// exit 0 with an empty stderr. Measured: that mutation fails five tests,
/// and the other four are three `.stderr` files and a table row, every one
/// of which reads as an expectation to re-bless rather than as a check that
/// is missing.
///
/// It does not say *which* check the top level owes, because that cannot be
/// written before the check exists. It says the top level earns its place by
/// reporting something the one below it does not, which is true of every
/// level this ladder will ever have.
///
/// Mutation: move [`SafetyLevel::IMPLEMENTED`] up one without wiring a check
/// for the level it moves to. This fails, and it is the only one of the five
/// failures that says something is absent rather than stale.
#[test]
fn the_implemented_level_runs_a_check_the_level_below_it_does_not() {
    let levels = SafetyLevel::value_variants();
    let top = levels
        .iter()
        .position(|level| *level == SafetyLevel::IMPLEMENTED)
        .expect("the implemented level is one of the levels");
    assert!(
        top > 0,
        "`IMPLEMENTED` is the level that runs nothing, so it claims nothing"
    );
    let below = levels[top - 1];

    // A double free, which the lowest implemented level proves today. The
    // program belongs to the level below the one being claimed only in the
    // sense that it is what the *existing* checks catch; that is the point,
    // because a program for a check nobody has written cannot be the fixture.
    let input = TempFile::new(
        "safec_implemented_level.c",
        "void *malloc(int n);
void free(void *p);
int main(void) {
    int *p = malloc(8);
    free(p);
    free(p);
    return 0;
}
",
    );
    let mut options = options(vec![input.path().to_path_buf()]);
    options.emit = EmitKind::SafetyIr;

    options.safety = SafetyLevel::IMPLEMENTED;
    let at_top = compile(&options).diagnostics.diagnostics().len();
    options.safety = below;
    let at_below = compile(&options).diagnostics.diagnostics().len();

    assert!(
        at_top > at_below,
        "{:?} reported {at_top} and {below:?} reported {at_below}: the level              this compiler says it implements runs no check the one below it does              not, so `IMPLEMENTED` has moved ahead of them",
        SafetyLevel::IMPLEMENTED
    );
}

/// The rule, over every kind there is: a run over a program this compiler
/// can compile answers with something for every one of them. Driven by
/// `EmitKind::value_variants` rather than by a list, so a kind added
/// anywhere in the pipeline is covered without anyone remembering this test
/// exists, which is the only guard of that shape here: `E0004` makes
/// somebody write an arm and nothing makes the arm they write right, and a
/// total roster is what answers for a variant nobody has written yet.
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

    // `add` is a hatch because `--emit hatches` lists hatches, and a
    // program with none is one it rightly answers nothing for. Every other
    // kind makes the same thing of it either way.
    let file = TempFile::new(
        "safec_driver_emit_every.c",
        "__attribute__((annotate(\"safec_unchecked\")))
int add(int a, int b) { return a + b; }
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
        attribute: None,
        return_nullability: None,
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
