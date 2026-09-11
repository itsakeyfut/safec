//! Whether what `--emit llvm-ir` writes is LLVM IR.
//!
//! The corpus pins the text and this pins what the text *is*. Two claims and
//! two tests on purpose: spelling `Add` as `Sub` changes every byte the corpus
//! holds and leaves this passing, which is what says they are not one test
//! written twice.
//!
//! `clang` is what checks it rather than `llc`, because `clang -x ir` reads the
//! same textual IR and is the LLVM tool that is actually installed beside a C
//! compiler. `-S -emit-llvm -o -` parses and verifies a module and writes
//! nothing to disk, so this needs no temporary file and no linker.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Programs whose IR is worth handing to LLVM, and the machine each is for.
///
/// Listed rather than discovered, the way `cases.rs` lists its cases and for
/// ADR-0007's reason. Each is also a corpus case, so the pair says two things
/// about one program: what the text is, and that LLVM takes it.
///
/// The last one is a run that refused a function. A module that is only a
/// declaration still has to parse, because the exit code is what says the run
/// failed and a broken artifact would say it twice and mean something else.
const PROGRAMS: &[(&str, &str)] = &[
    ("the_mvp_becomes_llvm_ir", "x86_64-pc-windows-msvc"),
    ("llvm_ir_follows_the_target", "aarch64-unknown-linux-gnu"),
    (
        "llvm_ir_of_pointers_branches_and_a_loop",
        "x86_64-pc-windows-msvc",
    ),
    (
        "an_ir_shape_the_backend_cannot_write",
        "x86_64-pc-windows-msvc",
    ),
];

/// Whether there is a `clang` on this machine at all.
fn clang_is_here() -> bool {
    Command::new("clang")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// A `.c` file from the corpus, by the name of its case.
fn case(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("cases")
        .join(format!("{name}.c"))
}

/// What LLVM says about a module, which should be nothing.
///
/// `--target` is passed because an artifact is for a machine and this one is
/// not always the host: `llvm_ir_follows_the_target` is for a machine nothing
/// here runs. RK-011 in the review knowledge bank is the entry, and it is the
/// same rule from the other side.
///
/// `-Wno-override-module` because `clang`'s own default triple is more
/// specific than any of the measured ones (`x86_64-pc-windows-msvc19.51.36256`
/// on this host), so it warns that it is overriding a triple it agrees with.
/// Silencing it is what makes "said nothing" mean something.
fn llvm_says(module: &[u8], triple: &str) -> String {
    let mut clang = Command::new("clang")
        .args(["-x", "ir", "-S", "-emit-llvm", "-Wno-override-module"])
        .arg(format!("--target={triple}"))
        .args(["-o", "-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("clang answered `--version` a moment ago");

    clang
        .stdin
        .take()
        .expect("the pipe was asked for")
        .write_all(module)
        .expect("clang reads its whole input before answering");

    let output = clang.wait_with_output().expect("clang was spawned");
    let said = String::from_utf8_lossy(&output.stderr).into_owned();
    if output.status.success() && said.is_empty() {
        return String::new();
    }
    format!("exit {:?}: {said}", output.status.code())
}

/// Every module this compiler writes is one LLVM accepts without a word.
///
/// Mutation: emit a `store` of an `i1` rather than widening a comparison. The
/// corpus still matches nothing, so it fails there too, but `clang` is what
/// says *why*: a value of the wrong type. Mutation: drop the `entry:` block's
/// `br`. LLVM refuses a block with no terminator and this is the only test
/// that would say so.
///
/// Where there is no `clang`, this says what it did not check and passes,
/// unless `SAFEC_REQUIRE_LLVM` is set. CI sets it, so "nobody has `clang` any
/// more" is a red build rather than a quiet one: a check whose command cannot
/// fail loudly reports the state it was asked to prove, which is RK-012 and
/// has already produced a false result here twice.
#[test]
fn the_emitted_ir_is_what_llvm_accepts() {
    if !clang_is_here() {
        assert!(
            std::env::var_os("SAFEC_REQUIRE_LLVM").is_none(),
            "SAFEC_REQUIRE_LLVM is set and there is no `clang` to check the IR with"
        );
        eprintln!("no `clang` on this machine: the emitted IR was not checked against LLVM");
        return;
    }

    for (name, triple) in PROGRAMS {
        let emitted = Command::new(env!("CARGO_BIN_EXE_safec"))
            .args(["--color", "never", "--emit", "llvm-ir", "--target", triple])
            .arg(case(name))
            .output()
            .expect("the compiler binary was built for this test");

        assert!(
            !emitted.stdout.is_empty(),
            "{name}: nothing was written to check"
        );

        let said = llvm_says(&emitted.stdout, triple);
        assert!(said.is_empty(), "{name}: clang said {said}");
    }
}
