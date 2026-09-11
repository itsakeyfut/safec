//! What the compiler produces, as a caller redirecting it would see.
//!
//! `--emit tokens` is the first artifact there is, and the first invocation
//! that can exit zero. Both of those are the process's business rather than the
//! library's: the split between the streams and the exit code exist so that
//! `safec --emit tokens a.c > a.tok` works, and only a spawned binary can show
//! that it does.
//!
//! What each stream holds is pinned by the corpus in `cases.rs`, one expected
//! file per stream. What is left here is what a pair of files cannot say.

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

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
/// The argument is relative to `tests/`. There is one directory of them,
/// `cases/`, and a program there is a corpus case as well as whatever a test
/// here wants it for.
fn test_file(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(path)
}

/// Redirecting the artifact does not take the diagnostics with it.
///
/// The corpus pins what each stream held, and it captures both at once through
/// pipes, so it never has to send one somewhere. This does: stdout goes to a
/// file the way a shell would send it, and the report has to still arrive on
/// the terminal. That is the whole reason the two streams are separate, it is
/// what this module's comment says the file is for, and it is tested nowhere
/// else.
///
/// The comparison is against the corpus's own expected files rather than
/// against emptiness. Non-emptiness is not the property: swap the two streams
/// and both are still non-empty, each holding the other's contents, which is
/// the failure this test is named after and would have passed.
///
/// Mutation: swap the two writers at the `run_compiler` call in `main`. This
/// fails, because the file then holds the report.
#[test]
fn redirecting_the_artifact_leaves_the_diagnostics_behind() {
    let expected_artifact = std::fs::read(test_file("cases/unexpected_character.stdout"))
        .expect("the corpus pins what this program's artifact is");
    let expected_report = std::fs::read(test_file("cases/unexpected_character.stderr"))
        .expect("the corpus pins what this program's report is");

    let path = std::env::temp_dir().join(format!("safec_emit_{}.tok", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let file = std::fs::File::create(&path).expect("a temporary file can be created");

    let output = Command::new(env!("CARGO_BIN_EXE_safec"))
        .args(["--color", "never", "--emit", "tokens"])
        // Run from `cases/` with a bare name for the same reason the corpus
        // does: the compiler echoes the path it was given, and the expected
        // files were written against the bare one.
        .current_dir(test_file("cases"))
        .arg("unexpected_character.c")
        .stdout(Stdio::from(file))
        .stderr(Stdio::piped())
        .output()
        .expect("the compiler binary was built for this test");

    let redirected = std::fs::read(&path).expect("the redirected artifact is on disk");
    let _ = std::fs::remove_file(&path);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        redirected == expected_artifact,
        "the file took the artifact, not the report"
    );
    assert!(
        output.stderr == expected_report,
        "the report stayed on stderr, and the artifact did not follow it"
    );
}

/// Everything past the parser. Asking for it produces nothing on stdout and a
/// non-zero exit, rather than an empty artifact and a claim of success.
///
/// Hand written rather than a case, because the claim is about the emit ladder
/// rather than about any one artifact: four invocations over one source, and a
/// case is one invocation over one source.
///
/// `ast` was in this list until a parser existed to reach it, and `safety-ir`
/// until a lowering and a printer did. The list is what the compiler cannot do
/// yet, so it shrinks as phases land.
#[test]
fn asking_for_an_artifact_that_does_not_exist_yet_produces_nothing() {
    for emit in ["object", "executable"] {
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
