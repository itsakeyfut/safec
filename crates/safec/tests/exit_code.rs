//! What the process reports, as a build system reads it.
//!
//! Every other test drives the library in process, which cannot see the exit
//! code at all: `main` maps an [`Outcome`] to an `ExitCode`, and clap exits
//! before `main` runs for an invocation it could not parse. Those are the two
//! halves of the compiler's contract with `make`, `ninja` and every CI script,
//! and neither is reachable without spawning the binary.
//!
//! [`Outcome`]: safec::driver::Outcome

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use safec::parser::MAX_NESTING;

/// Run the compiler the way a build system would.
fn safec(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_safec"))
        .args(args)
        .output()
        .expect("the compiler binary was built for this test")
}

/// A path in the temporary directory that nothing creates.
fn missing(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}

/// Zero for success and one for a compilation that failed is what `cc` does.
/// Nothing today can reach zero through a compilation, because the pipeline
/// reports that it does not exist, so this pins the failing half and the two
/// invocations that answer without compiling anything.
///
/// The substring check is deliberate and is the only one left in this
/// directory. `cannot read` ends in a note that is the operating system's text,
/// `The system cannot find the file specified.` on Windows and `No such file or
/// directory` on Linux, and the suite runs on three of them. A corpus case
/// compares byte for byte, so it would fail on two by construction. Only the
/// part this compiler wrote is asserted here. See `docs/architecture.md`, which
/// records the divergence, and issue 17, which closes it.
#[test]
fn a_compilation_that_failed_exits_one() {
    let path = missing("safec_exit_code_absent.c");
    let output = safec(&["--color", "never", &path.display().to_string()]);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let report = String::from_utf8(output.stderr).expect("the report is text");
    assert!(report.contains("cannot read"), "{report}");
}

/// Diagnostics go to stderr, so that `safec ... > out` leaves them on the
/// terminal and a caller reading stdout is not handed them by mistake.
#[test]
fn the_report_goes_to_stderr_and_stdout_stays_empty() {
    let path = missing("safec_exit_code_streams.c");
    let output = safec(&["--color", "never", &path.display().to_string()]);

    assert!(!output.stderr.is_empty(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
}

/// Two belongs to the argument parser and nothing else returns it. `driver.rs`
/// says so in a doc comment; this is the thing that makes it true.
#[test]
fn an_invocation_the_parser_could_not_understand_exits_two() {
    for args in [
        &[][..],
        &["--no-such-flag", "a.c"],
        &["--safety", "paranoid"],
    ] {
        let output = safec(args);

        assert_eq!(output.status.code(), Some(2), "{args:?} {output:?}");
    }
}

/// Asking what the compiler is has succeeded, whatever the compiler would make
/// of the source it was not given.
#[test]
fn asking_for_help_or_a_version_exits_zero() {
    for args in [&["--help"][..], &["--version"]] {
        let output = safec(args);

        assert_eq!(output.status.code(), Some(0), "{args:?} {output:?}");
        assert!(!output.stdout.is_empty(), "{args:?}");
    }
}

/// A run whose artifact never reached the pipe does not report success.
///
/// `driver.rs::an_artifact_that_could_not_be_written_is_an_error` covers the
/// library half: given a writer that fails, `run_compiler` returns an error.
/// What nothing covered is the other side of the process boundary, where that
/// error becomes the number a build system reads.
///
/// It does not reach the flush in `main`, and saying it did would be wrong:
/// `run_compiler` has already failed by then, because `Stdout` is a
/// `LineWriter` and every line the token dump produces ends in a newline, so
/// the write reaches the operating system before any buffer holds it.
/// `main.rs` says of that flush that no test guards it, and that is still
/// true. Removing the flush entirely fails nothing.
///
/// The read end is closed before the compiler writes, which is what a shell
/// does for `safec ... | head -1`. Rust ignores `SIGPIPE`, so the child sees a
/// failed write and exits rather than dying of a signal, and an exit code is
/// what a build system reads. That claim is checked on the other two platforms
/// by CI running this test rather than by anything here.
///
/// Mutation: in `main.rs`, return `ExitCode::SUCCESS` from the arm that handles
/// a failed flush. This test fails and no other does.
#[test]
fn a_run_whose_artifact_could_not_be_written_does_not_report_success() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/cases/add.c");

    let mut child = Command::new(env!("CARGO_BIN_EXE_safec"))
        .args(["--color", "never", "--emit", "tokens"])
        .arg(&source)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the compiler binary was built for this test");

    // Closing the read end before the compiler writes. Dropping the handle is
    // the whole of it: there is nothing left to read what it produces.
    drop(child.stdout.take());

    let status = child.wait().expect("the compiler was waited for");

    assert_eq!(
        status.code(),
        Some(1),
        "a run that could not deliver its artifact reported {status:?}"
    );
}

/// `-o` is reported as unsupported, and the path it names is left alone.
///
/// `driver.rs`'s `an_output_path_that_is_not_honoured_is_reported` is the other
/// half and checks the report. Neither it nor anything else looked at the path,
/// so a compiler that started writing there would have been caught by nothing:
/// a build system reading that file would get content this compiler never
/// claimed to have produced, which is the failure `driver.rs` names where it
/// makes the decision.
///
/// Mutation: write anything to `options.output` in `run_compiler`. This test
/// fails and no other does.
#[test]
fn an_output_path_that_is_not_honoured_is_left_alone() {
    let path = std::env::temp_dir().join(format!("safec_output_{}.tok", std::process::id()));
    let _ = std::fs::remove_file(&path);

    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/cases/add.c");
    let output = safec(&[
        "--color",
        "never",
        "-o",
        &path.display().to_string(),
        "--emit",
        "tokens",
        &source.display().to_string(),
    ]);

    let written = path.exists();
    let _ = std::fs::remove_file(&path);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(!written, "the compiler wrote to {}", path.display());
}

/// The deepest nest of statements the parser accepts is printed, rather than
/// ending the process.
///
/// `dump_stmt` recurses where `dump_expr` deliberately does not, and the reason
/// it is allowed to is a bound: every statement nesting is a parser recursion
/// through `Parser::deeper`, so a tree that reaches the printer is at most
/// `MAX_NESTING` statements deep. That is an argument about a number, and this
/// is the number being run.
///
/// The depth is read from `MAX_NESTING` rather than written out, which is the
/// difference between running the number and running a number that used to be
/// it. A copy here would go on passing at 255 while the constant moved, and the
/// claim this test makes is about the constant.
///
/// The two shapes are the ones that write no bracket per level: an `else if`
/// chain is `else` followed by an `if` statement rather than a construct of its
/// own, and a `while` whose body is another `while` reads as one line.
///
/// Mutation: raise `MAX_NESTING` in `parser.rs` far enough that the recursion
/// outruns the stack, 200000 being ample. This fails, with the exit code of a
/// process nothing in `driver.rs` chose. Which recursion dies first, the
/// parser's or the printer's, is not the claim; that neither may be given more
/// levels than it can hold is.
#[test]
fn the_deepest_nest_of_statements_does_not_end_the_process() {
    for (name, body) in [
        (
            "safec_exit_code_whiles.c",
            "while (a) ".repeat(MAX_NESTING - 1),
        ),
        (
            "safec_exit_code_else_ifs.c",
            "if (a) ; else ".repeat(MAX_NESTING - 1),
        ),
    ] {
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, format!("int main(void) {{ {body}; }}\n"))
            .expect("the temporary directory is writable");

        let output = safec(&[
            "--color",
            "never",
            "--emit",
            "ast",
            &path.display().to_string(),
        ]);
        let _ = std::fs::remove_file(&path);

        assert_eq!(
            output.status.code(),
            Some(0),
            "{name}: {:?}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// A long flat expression is read and printed, rather than ending the process.
///
/// Not a corpus case, because the point is the exit code rather than the
/// artifact: the failure this guards is a stack overflow, which is not a panic
/// anything can catch and gives a code nothing in `driver.rs` chose. The corpus
/// harness turns that into "the compiler was killed by a signal", and the
/// expected file would be a megabyte of indentation.
///
/// The shape matters and is not adversarial. `MAX_NESTING` bounds the parser's
/// recursion, and neither of these two shapes recurses in the parser at all:
/// the precedence climb folds a left-associative chain in a loop, and postfix
/// operators are read in a loop, so each adds a level to the *tree* without
/// adding one to the parser. `clang` compiles both. A thousand terms is what a
/// generated `.c` file looks like.
///
/// Mutation: make `dump_expr` in `driver.rs` recurse into its children instead
/// of pushing them onto its own stack. This fails, with exit 101 on the
/// harness's own `unwrap` because the process was killed.
#[test]
fn a_long_flat_expression_does_not_end_the_process() {
    for (name, tail) in [
        ("safec_exit_code_chain.c", " + a".repeat(1000)),
        ("safec_exit_code_postfix.c", "++".repeat(1000)),
    ] {
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, format!("int main(void) {{ return a{tail}; }}\n"))
            .expect("the temporary directory is writable");

        let output = safec(&[
            "--color",
            "never",
            "--emit",
            "ast",
            &path.display().to_string(),
        ]);
        let _ = std::fs::remove_file(&path);

        assert_eq!(
            output.status.code(),
            Some(0),
            "{name}: {:?}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
