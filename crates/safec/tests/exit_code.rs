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
use std::process::{Command, Output};

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
