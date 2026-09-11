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

use safec_ir::target::Target;

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
    ("llvm_ir_of_every_operator", "x86_64-pc-windows-msvc"),
    (
        "a_narrow_value_is_extended_where_the_target_asks",
        "x86_64-unknown-linux-gnu",
    ),
    (
        "a_narrow_unsigned_value_is_extended_without_a_sign",
        "armv7-unknown-linux-gnueabihf",
    ),
    (
        "llvm_ir_of_conversions_and_a_constant_condition",
        "x86_64-pc-windows-msvc",
    ),
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

    // A `clang` that gives up before reading its input breaks the pipe, and
    // that is its answer rather than this test's failure. Not hypothetical:
    // Windows buffers about 64 KiB, so a module smaller than that is written
    // whatever `clang` does, and a larger one is not.
    let given = clang
        .stdin
        .take()
        .expect("the pipe was asked for")
        .write_all(module);
    if let Err(error) = given {
        let _ = clang.wait();
        return format!("could not be given the module: {error}");
    }

    let output = clang.wait_with_output().expect("clang was spawned");
    let said = String::from_utf8_lossy(&output.stderr).into_owned();
    if output.status.success() && said.is_empty() {
        return String::new();
    }
    format!("exit {:?}: {said}", output.status.code())
}

/// Every module this compiler writes is one LLVM accepts without a word.
///
/// Mutation: store a comparison's `i1` rather than widening it, **and then
/// re-bless the corpus**. Every expectation agrees again, all 92 cases pass,
/// and this is what still fails, beside the hand-written assertion in
/// `two_pointers_can_be_compared`. Measured, because that is the whole claim:
/// a blessed file is only as good as the run that blessed it, and this is the
/// one test that asks something outside this repository whether the answer is
/// right.
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

/// Every triple this compiler knows, for the differential below.
///
/// Written out rather than read from `Target::ALL`, for the reason
/// `every_target_is_what_clang_says_it_is` gives: a test that asks the table
/// about itself holds for any table. A triple added there and not here fails
/// the count.
const TRIPLES: &[&str] = &[
    "aarch64-apple-darwin",
    "aarch64-unknown-linux-gnu",
    "armv7-unknown-linux-gnueabihf",
    "i686-unknown-linux-gnu",
    "wasm32-unknown-unknown",
    "x86_64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
];

/// The extension attributes on a `define` line, in the order they appear.
///
/// The whole line cannot be compared. `clang` writes `dso_local`, `hidden`,
/// `noundef` and a reference to an attribute group, none of which is about how
/// a narrow value crosses a call and none of which this compiler writes. What
/// is left after those is the claim.
///
/// In order rather than as a set, so that an attribute on the result and one
/// on a parameter are not the same answer.
fn extensions(module: &str, name: &str) -> Vec<&'static str> {
    let wanted = format!("@{name}(");
    let line = module
        .lines()
        .find(|line| line.starts_with("define ") && line.contains(&wanted))
        .unwrap_or_else(|| {
            panic!(
                "no definition of `{name}`:
{module}"
            )
        });

    line.split(|character: char| !character.is_ascii_alphanumeric())
        .filter_map(|word| match word {
            "signext" => Some("signext"),
            "zeroext" => Some("zeroext"),
            _ => None,
        })
        .collect()
}

/// What this compiler writes about a narrow value is what `clang` writes.
///
/// A `char` is passed and returned already widened on five of the eight
/// measured targets and left narrow on the other three, and which is which does
/// not follow from anything else: `aarch64-apple-darwin` and
/// `x86_64-pc-windows-msvc` agree that `char` is signed and disagree about
/// this. So the table is a measurement, and this is what re-measures it.
///
/// This replaces the acceptance criterion that asked for a `clang`-compiled
/// caller linked against a `safec`-compiled callee. That was measured before it
/// was designed against: this host is `x86_64-pc-windows-msvc`, one of the
/// three that ask for nothing, so such a program answers the same with the
/// attribute and without it, and so does `windows-latest`. A test that cannot
/// fail where its author runs it is RK-012's shape. This one covers all eight
/// targets from any host, because `-S -emit-llvm` cross-compiles with no
/// sysroot.
///
/// Mutation: answer `None` from `Emitter::extension` whatever the target. The
/// five that ask fail. Mutation: answer `Some` whatever the target. The three
/// that do not fail. Mutation: always `signext`. The `armv7` row fails.
#[test]
fn the_attributes_are_what_clang_asks_for() {
    // Before the gate below, so that a target added to the table without a row
    // here is caught on a machine with no `clang` too.
    assert_eq!(
        TRIPLES.len(),
        Target::ALL.len(),
        "a target nobody asked `clang` about is one this list forgot"
    );

    if !clang_is_here() {
        assert!(
            std::env::var_os("SAFEC_REQUIRE_LLVM").is_none(),
            "SAFEC_REQUIRE_LLVM is set and there is no `clang` to ask"
        );
        eprintln!("no `clang` on this machine: the ABI attributes were not checked against it");
        return;
    }

    let program = case("a_narrow_value_is_extended_where_the_target_asks");

    for triple in TRIPLES {
        let ours = Command::new(env!("CARGO_BIN_EXE_safec"))
            .args(["--color", "never", "--emit", "llvm-ir", "--target", triple])
            .arg(&program)
            .output()
            .expect("the compiler binary was built for this test");
        let ours = String::from_utf8(ours.stdout).expect("this compiler writes UTF-8");

        let theirs = Command::new("clang")
            .args(["-S", "-emit-llvm", "-O0"])
            .arg(format!("--target={triple}"))
            .args(["-o", "-"])
            .arg(&program)
            .output()
            .expect("clang answered `--version` a moment ago");
        assert!(
            theirs.status.success(),
            "{triple}: clang said {}",
            String::from_utf8_lossy(&theirs.stderr)
        );
        let theirs = String::from_utf8_lossy(&theirs.stdout).into_owned();

        assert_eq!(
            extensions(&ours, "use_it"),
            extensions(&theirs, "use_it"),
            "{triple}"
        );
    }
}
