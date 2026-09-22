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

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

/// A path with nothing at it, for a test asking what a run leaves behind.
///
/// The process id is in the name for the reason the test below puts it there:
/// two runs of this suite at once must not answer each other's question.
/// Removed first, because every test here asks whether a run wrote a file and
/// one left by an earlier run would answer yes on its behalf.
fn artifact_path(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("safec_emit_{}_{name}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

/// `--emit llvm-ir` over a corpus program, writing the artifact to `path`.
///
/// A corpus program rather than a string, for the reason [`test_file`] gives:
/// the path a user takes is the path under test. The target is named because
/// an artifact is for a machine and this one is not always the host; which
/// machine does not matter here, and naming one keeps the run from depending
/// on where it is.
fn llvm_ir_to(path: &Path, case: &str) -> std::process::Output {
    llvm_ir_to_with(path, case, &[])
}

/// The same, with arguments the caller needs on top.
///
/// One caller needs `--allow-unknown`, because since ADR-0033 an unproven
/// conclusion is an error wherever a check runs and a run that only warns has
/// to ask for it.
fn llvm_ir_to_with(path: &Path, case: &str, extra: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_safec"))
        .args(["--color", "never", "--emit", "llvm-ir"])
        .args(["--target", "x86_64-pc-windows-msvc"])
        .args(extra)
        .arg("-o")
        .arg(path)
        .arg(test_file(&format!("cases/{case}.c")))
        .output()
        .expect("the compiler binary was built for this test")
}

/// A dump keeps what it could read when one input could not be read.
///
/// The other side of the same rule, and the reason it has one: a user who
/// asked for the safety IR of three programs and gave one that is not there
/// wants the two that are. This is the scenario `survives_an_error`'s doc
/// comment names, and nothing held it until #144 moved a kind across and made
/// the split carry the meaning.
///
/// Two inputs rather than one, because one bad input produces an empty
/// artifact and the write rule's other half stops that before this rule is
/// asked. Measured: `--emit safety-ir -o out ok.c bad.c` exits 1 and writes,
/// `--emit safety-ir -o out bad.c` exits 1 and writes nothing.
///
/// Mutation: answer `false` from `EmitKind::survives_an_error` for `SafetyIr`.
/// Nothing is written and this fails on the file being absent.
#[test]
fn a_dump_keeps_what_it_could_read() {
    let path = artifact_path("partial.ir");
    let output = Command::new(env!("CARGO_BIN_EXE_safec"))
        .args(["--color", "never", "--emit", "safety-ir"])
        .args(["--target", "x86_64-pc-windows-msvc"])
        .arg("-o")
        .arg(&path)
        .arg(test_file("cases/a_value_freed_twice.c"))
        .arg(test_file("cases/there_is_no_such_program.c"))
        .output()
        .expect("the compiler binary was built for this test");

    let said = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{said}");
    let kept = std::fs::read_to_string(&path)
        .expect("the input that could be read is still in the artifact");
    let _ = std::fs::remove_file(&path);
    assert!(
        kept.contains("a_value_freed_twice"),
        "the dump lost the input it could read: {kept}"
    );
}

/// A run that proved a program unsafe leaves no LLVM IR behind.
///
/// `clang out.ll -o prog` compiles what this would otherwise write, so it is a
/// build product and not a dump: a file on disk, newer than the source, that a
/// build system reads as finished. The exit code would say the run failed and
/// the file would say it succeeded.
///
/// Mutation: answer `true` from `EmitKind::survives_an_error` for `LlvmIr`.
/// The file is written and this fails on it being there.
#[test]
fn a_proved_unsafe_run_leaves_no_llvm_ir() {
    let path = artifact_path("proved_unsafe.ll");
    let output = llvm_ir_to(&path, "a_value_freed_twice");

    let said = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{said}");
    assert!(
        !path.exists(),
        "a run that proved the program unsafe left LLVM IR for a build to pick up"
    );
}

/// And it leaves what was already there exactly as it was.
///
/// **The scenario the issue is about, which the two tests around it do not
/// reach.** Both of those ask for a path with nothing at it, so what they hold
/// is that no file appears. What #144 describes is a file *newer than its
/// source* that a build system reads as finished, and the way a user meets
/// that is a previous good build sitting at the path. Truncating it would
/// leave a zero-byte artifact newer than everything, which is worse than
/// either outcome the others rule out.
///
/// Mutation: answer `true` from `EmitKind::survives_an_error` for `LlvmIr`.
/// The module replaces the previous build and this fails on the contents.
#[test]
fn a_proved_unsafe_run_leaves_what_was_already_there() {
    let path = artifact_path("previous_build.ll");
    std::fs::write(
        &path,
        "the last good build
",
    )
    .expect("the temporary file can be written");

    let output = llvm_ir_to(&path, "a_value_freed_twice");

    let said = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{said}");
    let kept = std::fs::read_to_string(&path).expect("the previous build is still there");
    let _ = std::fs::remove_file(&path);
    assert_eq!(
        kept,
        "the last good build
",
        "a run that failed replaced the artifact a run that succeeded had left"
    );
}

/// So does a run whose backend refused a function.
///
/// **The one that says which rule this is.** `survives_an_error`'s doc comment
/// gives exactly this case as the reason an object does not survive, and the
/// module written here has the refused function reduced to a declaration: a
/// `.ll` that assembles into a program with a function deleted from it. A rule
/// keyed on a safety conclusion would leave this writing its file, which is why
/// this is here and not only the test above.
///
/// `p + 1` is what the backend refuses today. When it stops refusing, this
/// fails on the exit code rather than passing quietly, which is the right way
/// round and is the shape `object.rs` uses for the same reason.
///
/// Mutation: the same one as above.
#[test]
fn a_backend_refusal_leaves_no_llvm_ir() {
    let path = artifact_path("refused.ll");
    let output = llvm_ir_to(&path, "an_ir_shape_the_backend_cannot_write");

    let said = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{said}");
    assert!(
        !path.exists(),
        "a run whose backend refused a function left the module it wrote anyway"
    );
}

/// A run whose worst report is a warning still writes its LLVM IR.
///
/// The direction the two above must not take with them. An unproven free is
/// `Unknown`, the run exits 0, and there is an artifact to produce; without
/// this, refusing to emit for any program the memory check mentions would pass
/// both of them.
///
/// `--allow-unknown` is what leaves a warning to be the worst report. Since
/// ADR-0033 an unproven conclusion is an error wherever a check runs, so the
/// same program without the flag exits 1 and writes nothing, which is the two
/// tests above rather than this one.
///
/// Mutation: read `has_errors()` as `!is_empty()` in `run_compiler`'s write
/// rule, so that anything reported at all is a failed run. This fails on the
/// file being absent.
#[test]
fn a_warning_still_writes_its_llvm_ir() {
    let path = artifact_path("warned.ll");
    let output = llvm_ir_to_with(
        &path,
        "a_call_this_check_cannot_read_between_two_frees",
        &["--allow-unknown"],
    );

    let said = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{said}");
    assert!(
        said.contains("warning["),
        "the program this is about reports one"
    );
    assert!(
        path.exists(),
        "a run that only warned was denied the artifact it asked for"
    );
    let _ = std::fs::remove_file(&path);
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
