//! The programs in `examples/`, held to what their comments tell a user.
//!
//! An example is for somebody trying this compiler for the first time: they
//! build it, run it, and then change the lines it marks `BREAK IT` to watch
//! `safec` refuse them. A comment that has stopped being true is worse there
//! than anywhere, because that reader has nothing else to go on. So each
//! promise the comment makes is a test here: the program builds and answers
//! what it says, each marked edit is refused with the code its marker names,
//! and the edit marked `NOT CAUGHT` still builds.
//!
//! The example's text comes in through `include_str!`, so moving or deleting
//! the file is a build failure rather than a test that silently reads nothing.
//! Each edit is applied to that text, so the example stays the one copy: a
//! broken version kept as a file of its own could drift from it with the
//! suite still green.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// `examples/fib.c`, from the repository root.
const FIB: &str = include_str!("../../../examples/fib.c");

/// A directory of its own, removed however the test ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("safec_examples_{name}"));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("the temporary directory is writable");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    /// A `.c` file inside it holding `text`.
    fn source(&self, name: &str, text: &str) {
        fs::write(self.0.join(name), text).expect("the temporary directory is writable");
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Whether there is a `clang` on this machine to link a program with.
fn clang_is_here() -> bool {
    Command::new("clang")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// Answer `true` where the test should go on, and say what it skipped where
/// not. `SAFEC_REQUIRE_LLVM`, which CI sets, makes a skip a failure, as it
/// does in `tests/object.rs`.
fn clang_or_skip(what: &str) -> bool {
    if clang_is_here() {
        return true;
    }
    assert!(
        std::env::var_os("SAFEC_REQUIRE_LLVM").is_none(),
        "SAFEC_REQUIRE_LLVM is set and there is no `clang` to link a program with"
    );
    eprintln!("no `clang` on this machine: {what} was not checked");
    false
}

fn safec(arguments: &[&str], within: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_safec"))
        .args(["--color", "never"])
        .args(arguments)
        .current_dir(within)
        .output()
        .expect("the compiler binary was built for this test")
}

/// `text` with `from` replaced by `to`, where `from` occurs exactly once.
///
/// An edit that no longer matches, because the example changed under it or
/// arrived with CRLF line endings, fails here by name rather than leaving the
/// test to check the unedited program. Mutation: write `free( total );` in the
/// example. Both tests that edit the `free` fail here.
fn edited(text: &str, from: &str, to: &str) -> String {
    assert_eq!(
        text.matches(from).count(),
        1,
        "the edit does not match the example exactly once: {from:?}"
    );
    text.replacen(from, to, 1)
}

/// Run the checks on `text` and answer the exit code and what was said.
///
/// `--emit safety-ir` rather than an executable, so the refusals are checked
/// on a machine with no `clang`: the checks run before anything is linked.
fn checked(name: &str, text: &str) -> (Option<i32>, String) {
    let scratch = Scratch::new(name);
    scratch.source("fib.c", text);
    let output = safec(
        &["--emit", "safety-ir", "-o", "fib.ir", "fib.c"],
        scratch.path(),
    );
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Assert that `FIB` carries the marker for `code`, and that `text`, the
/// example with that marker's edit applied, is refused with `code`.
fn refused(name: &str, text: &str, code: &str) {
    let marker = format!("// BREAK IT ({code})");
    assert!(
        FIB.contains(&marker),
        "the example marks no edit with {marker:?}"
    );
    let (status, said) = checked(name, text);
    assert_eq!(status, Some(1), "{said}");
    assert!(said.contains(&format!("error[{code}]")), "{said}");
}

/// The example builds at the default safety level and its program exits with
/// the status its comment names, the sum of `fib(0)` to `fib(10)`.
///
/// Mutation: change `i <= 10` to `i < 10` in `examples/fib.c`. It exits 88 and
/// this fails.
#[test]
fn the_fib_example_builds_and_exits_with_the_sum_it_computes() {
    assert!(
        FIB.contains("exits with status 143"),
        "the comment no longer names the status this checks"
    );
    if !clang_or_skip("whether the fib example runs and exits 143") {
        return;
    }

    let scratch = Scratch::new("fib_runs");
    scratch.source("fib.c", FIB);
    let program = scratch
        .path()
        .join(if cfg!(windows) { "fib.exe" } else { "fib" });

    let output = safec(
        &[
            "--emit",
            "executable",
            "-o",
            &program.to_string_lossy(),
            "fib.c",
        ],
        scratch.path(),
    );
    let said = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{said}");

    let ran = Command::new(&program)
        .status()
        .expect("the program this compiler just wrote can be run");
    assert_eq!(ran.code(), Some(143), "the fib example did not exit 143");
}

/// The `free` at the end of `main`, with the two markers above it.
///
/// **Each edit takes its marker with it**, so a marker moved away from the
/// line it names, or naming another code, is an edit that no longer matches,
/// and fails in [`edited`] rather than passing with the marker misplaced.
const FREE: &str = "    // BREAK IT (SC0402): move this free above the line before.\n    \
                    // BREAK IT (SC0401): write this free twice.\n    free(total);\n";

/// The marked `free` moved above the read before it is refused as a use after
/// free.
///
/// Mutation: return at the top of `report.rs::used` in `safec-ir`. The edit
/// builds and this fails. Mutation: change `SC0402` to `SC0401` in the marker
/// above `free(total);`. This fails in `edited`.
#[test]
fn the_fib_example_freed_before_its_read_is_refused_as_a_use_after_free() {
    let read = "    int result = *total;\n";
    let text = edited(FIB, &format!("{read}{FREE}"), &format!("{FREE}{read}"));
    refused("fib_use_after_free", &text, "SC0402");
}

/// The marked `free` written twice is refused as a double free.
///
/// Mutation: change `SC0401` to `SC0409` in its marker. This fails in
/// `edited`.
#[test]
fn the_fib_example_freed_twice_is_refused_as_a_double_free() {
    let text = edited(FIB, FREE, &format!("{FREE}    free(total);\n"));
    refused("fib_double_free", &text, "SC0401");
}

/// The marked null test deleted is refused, as a dereference of a pointer
/// that may be null.
///
/// The edit takes the marker with it, as [`FREE`]'s do. Mutation: move the
/// marker above `int *total = malloc(4);`. This fails in `edited`. Mutation:
/// write the test as `if (0 == total)`. Likewise.
#[test]
fn the_fib_example_without_its_null_test_is_refused() {
    let text = edited(
        FIB,
        "    // BREAK IT (SC0403): delete this null test.\n    \
         if (total == 0) {\n        return 1;\n    }\n",
        "",
    );
    refused("fib_null", &text, "SC0403");
}

/// The line marked `NOT CAUGHT` deleted still builds, as the comment says: a
/// read of an integer out of memory nothing wrote is not checked yet. The day
/// it is, this fails and the comment has to change with it.
///
/// Mutation: change `NOT CAUGHT` in the example. This fails on the marker.
#[test]
fn the_fib_example_without_its_store_of_zero_still_builds() {
    assert!(
        FIB.contains("// NOT CAUGHT: delete this line and it still builds.\n    *total = 0;\n"),
        "the example no longer marks the store of zero as not caught"
    );
    let text = edited(FIB, "    *total = 0;\n", "");
    let (status, said) = checked("fib_not_caught", &text);
    assert_eq!(status, Some(0), "{said}");
}
