//! What the compiler produces, as a caller redirecting it would see.
//!
//! `--emit tokens` is the first artifact there is, and the first invocation
//! that can exit zero. Both of those are the process's business rather than the
//! library's: the split between the streams and the exit code exist so that
//! `safec --emit tokens a.c > a.tok` works, and only a spawned binary can show
//! that it does.

use std::path::PathBuf;
use std::process::{Command, Output};

fn safec(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_safec"))
        .args(args)
        .output()
        .expect("the compiler binary was built for this test")
}

/// A real `.c` file rather than a string built in a test, so that the path a
/// user takes is the path under test: reading from disk, scanning, and writing
/// the result to a stream.
fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// The first invocation of this compiler that succeeds. Every run before
/// `--emit tokens` reported that it had built nothing, because it had.
#[test]
fn asking_for_tokens_of_a_clean_file_succeeds_and_says_nothing() {
    let output = safec(&[
        "--color",
        "never",
        "--emit",
        "tokens",
        &fixture("add.c").display().to_string(),
    ]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");

    let tokens = String::from_utf8(output.stdout).expect("the emitter writes text");
    assert!(tokens.contains("keyword \"int\""), "{tokens}");
    assert!(tokens.contains("identifier \"add\""), "{tokens}");
    assert!(tokens.contains("punct \"{\""), "{tokens}");
    assert!(tokens.trim_end().ends_with("eof"), "{tokens}");

    // The comment and the whitespace are not tokens, so nothing in the dump
    // came from either.
    assert!(!tokens.contains("phase 1 target"), "{tokens}");
}

/// The artifact goes to stdout and the diagnostics to stderr, so a caller can
/// redirect one without catching the other.
#[test]
fn the_artifact_and_the_diagnostics_use_different_streams() {
    let path = fixture("unexpected.c").display().to_string();
    let output = safec(&["--color", "never", "--emit", "tokens", &path]);

    assert_eq!(output.status.code(), Some(1), "{output:?}");

    let report = String::from_utf8(output.stderr).expect("the renderer writes text");
    let tokens = String::from_utf8(output.stdout).expect("the emitter writes text");

    assert!(report.contains("unexpected character"), "{report}");
    assert!(
        report.contains("int x = @;"),
        "the source line is quoted: {report}"
    );
    assert!(tokens.contains("unknown \"@\""), "{tokens}");
    assert!(!tokens.contains("unexpected character"), "{tokens}");
}

/// Everything past the lexer. Asking for it produces nothing on stdout and a
/// non-zero exit, rather than an empty artifact and a claim of success.
#[test]
fn asking_for_an_artifact_that_does_not_exist_yet_produces_nothing() {
    for emit in ["ast", "safety-ir", "llvm-ir", "object", "executable"] {
        let output = safec(&[
            "--color",
            "never",
            "--emit",
            emit,
            &fixture("add.c").display().to_string(),
        ]);

        assert_eq!(output.status.code(), Some(1), "{emit}: {output:?}");
        assert!(output.stdout.is_empty(), "{emit}: {output:?}");
    }
}
