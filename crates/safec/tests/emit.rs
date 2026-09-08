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
///
/// Not `fixture`: it reaches into both directories, and naming it after one of
/// them would undo the distinction the rest of this comment draws.
///
/// The argument is relative to `tests/`, because there are two directories of
/// them and they mean different things. `cases/` holds the corpus, where a
/// program and its whole expected output are a test on their own; `fixtures/`
/// holds programs that exist for a claim the corpus cannot make.
fn test_file(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(path)
}

/// The artifact goes to stdout and the diagnostics to stderr, so a caller can
/// redirect one without catching the other.
#[test]
fn the_artifact_and_the_diagnostics_use_different_streams() {
    let path = test_file("fixtures/unexpected.c").display().to_string();
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
///
/// Hand written rather than a case, because the claim is about the emit ladder
/// rather than about any one artifact: five invocations over one source, and a
/// case is one invocation over one source.
#[test]
fn asking_for_an_artifact_that_does_not_exist_yet_produces_nothing() {
    for emit in ["ast", "safety-ir", "llvm-ir", "object", "executable"] {
        let output = safec(&[
            "--color",
            "never",
            "--emit",
            emit,
            &test_file("cases/add.c").display().to_string(),
        ]);

        assert_eq!(output.status.code(), Some(1), "{emit}: {output:?}");
        assert!(output.stdout.is_empty(), "{emit}: {output:?}");
    }
}
