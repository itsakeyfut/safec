//! What `--emit object` and `--emit executable` write, and what a machine
//! without `clang` is told.
//!
//! The two kinds `clang` makes, in one file because they are one subject: the
//! same tool, the same four ways it can fail, and the same question about what
//! a run leaves on disk. Neither is text, so the corpus cannot hold one:
//! `cases.rs` compares bytes this compiler wrote against bytes this compiler
//! wrote, and these came from `clang`. What can be held is what the object
//! *is*, which its own header says, that something else accepts it, and that
//! the program answers 3.
//!
//! Most tests here need a `clang`, because both kinds do: ADR-0015 says why.
//! They say what they did not check where there is none, and
//! `SAFEC_REQUIRE_LLVM` makes that a failure, which CI sets. The ones that do
//! not are the refusals, which answer before anything is spawned, and
//! `a_missing_clang_says_what_to_install`, which needs `clang` to be absent.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use safec_ir::target::Target;

/// A directory of its own, removed however the test ends.
///
/// A directory rather than a file because one of these tests is about what a
/// run writes when nobody named a path, and the answer is "beside wherever it
/// was run", which is only a safe thing to test somewhere disposable.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("safec_object_{name}"));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("the temporary directory is writable");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    /// A `.c` file inside it, and the path to it.
    fn source(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, text).expect("the temporary directory is writable");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The MVP of `docs/roadmap.md`, which is what Phase 3's Done-when is about.
const MVP: &str =
    "int add(int a, int b) {\n    return a + b;\n}\n\nint main() {\n    return add(1, 2);\n}\n";

/// Whether there is a `clang` on this machine at all.
fn clang_is_here() -> bool {
    Command::new("clang")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// Answer `true` where the test should go on, and say what it skipped where not.
fn clang_or_skip(what: &str) -> bool {
    if clang_is_here() {
        return true;
    }
    assert!(
        std::env::var_os("SAFEC_REQUIRE_LLVM").is_none(),
        "SAFEC_REQUIRE_LLVM is set and there is no `clang` to make an object with"
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

/// What machine an object is for, read out of its own header.
///
/// Four formats and no tool. Each puts what it is at a fixed offset, so this
/// needs neither `llvm-objdump`, which is on this host and promised nowhere,
/// nor a crate to read one with.
///
/// COFF is answered last because it has no magic of its own: a COFF object
/// begins with the machine field, so anything that is not one of the other
/// three is read as one.
fn machine(object: &[u8]) -> String {
    let word = |at: usize| u16::from_le_bytes([object[at], object[at + 1]]);
    let long = |at: usize| {
        u32::from_le_bytes([object[at], object[at + 1], object[at + 2], object[at + 3]])
    };

    assert!(object.len() > 20, "too short to be an object");

    if object.starts_with(b"\x7fELF") {
        // EI_DATA, which is 1 for little-endian. Asserted rather than handled:
        // every measured target is little-endian, and a big-endian one added
        // later should fail here rather than read `e_machine` backwards.
        assert_eq!(object[5], 1, "a big-endian ELF, which nothing measured is");
        return format!("elf {:#06x}", word(18));
    }
    if object.starts_with(&[0xcf, 0xfa, 0xed, 0xfe]) {
        return format!("mach-o {:#010x}", long(4));
    }
    if object.starts_with(b"\0asm") {
        return "wasm".to_owned();
    }
    format!("coff {:#06x}", word(0))
}

/// Whether this `clang` was built with the backend a triple needs.
///
/// A `clang` is not one list of targets. Apple's, on `macos-latest`, has no
/// WebAssembly backend and answers `unable to create target` for
/// `wasm32-unknown-unknown` while assembling the other seven. Asked by handing
/// it an empty module, which is valid IR and says nothing about this compiler,
/// so the answer is about the tool and not about what safec wrote.
///
/// Read rather than matched on the message: a string comparison against another
/// tool's wording is a guard that breaks when it is reworded.
fn targetable(triple: &str) -> bool {
    let mut probe = Command::new("clang")
        .args(["-c", "-x", "ir", "-Wno-override-module"])
        .arg(format!("--target={triple}"))
        .args(["-o", "-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("there is a clang, which the caller checked for");

    drop(probe.stdin.take().expect("the pipe was asked for"));
    probe
        .wait()
        .expect("a process that was spawned can be waited for")
        .success()
}

/// An object is for the machine the run named, and not for the host.
///
/// Written out rather than walked, the way `Target::ALL`'s own test is and for
/// RK-001's reason: asking the table what it says would hold for any table.
/// These came from `clang 20.1.6 --target=<triple>` on the MVP, read out of the
/// bytes with the helper above.
///
/// A row whose backend this `clang` does not have is said out loud and skipped,
/// which is a weaker check on that machine and is the honest one: what it would
/// otherwise hold is that whoever built `clang` chose to include a target, and
/// that is not a fact about this compiler. The host's own triple is not allowed
/// to be one of them, so the test cannot quietly check nothing.
///
/// Mutation: pass the host's triple to `clang` rather than the one the run
/// named. Seven rows fail on this host and a different seven on each CI runner.
/// Mutation: drop `--target` altogether. The same.
#[test]
fn an_object_is_for_the_machine_the_run_named() {
    if !clang_or_skip("which machine an object is for") {
        return;
    }

    let measured: &[(&str, &str)] = &[
        ("aarch64-apple-darwin", "mach-o 0x0100000c"),
        ("aarch64-unknown-linux-gnu", "elf 0x00b7"),
        ("armv7-unknown-linux-gnueabihf", "elf 0x0028"),
        ("i686-unknown-linux-gnu", "elf 0x0003"),
        ("wasm32-unknown-unknown", "wasm"),
        ("x86_64-apple-darwin", "mach-o 0x01000007"),
        ("x86_64-pc-windows-msvc", "coff 0x8664"),
        ("x86_64-unknown-linux-gnu", "elf 0x003e"),
    ];

    assert_eq!(
        measured.len(),
        Target::ALL.len(),
        "a target nobody made an object for is one this list forgot"
    );

    let scratch = Scratch::new("machines");
    let source = scratch.source("mvp.c", MVP);
    let source = source.to_string_lossy().into_owned();

    for &(triple, expected) in measured {
        if !targetable(triple) {
            assert_ne!(
                triple,
                env!("SAFEC_HOST_TRIPLE"),
                "this clang cannot target the machine it is running on"
            );
            eprintln!("this clang has no backend for {triple}: that row was not checked");
            continue;
        }

        let object = scratch.path().join(format!("{triple}.o"));
        let output = safec(
            &[
                "--emit",
                "object",
                "--target",
                triple,
                "-o",
                &object.to_string_lossy(),
                &source,
            ],
            scratch.path(),
        );

        assert!(
            output.status.success(),
            "{triple}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let written = fs::read(&object).expect("the object was written");
        assert_eq!(machine(&written), expected, "{triple}");
    }
}

/// The MVP becomes an object the platform's linker accepts, and the program it
/// makes answers 3.
///
/// Phase 3's Done-when, minus the linking this compiler will do itself in #93.
/// The host's triple, because a cross-compiled binary is one nothing here can
/// run.
///
/// Mutation: write the LLVM IR to the path rather than the object. `clang`
/// refuses to link text and this fails on its exit code.
#[test]
fn an_object_the_linker_accepts() {
    if !clang_or_skip("whether a linker takes what this writes") {
        return;
    }

    let scratch = Scratch::new("linked");
    let source = scratch.source("mvp.c", MVP);
    let object = scratch.path().join("mvp.o");
    let program = scratch.path().join("mvp.exe");

    let output = safec(
        &[
            "--emit",
            "object",
            "--target",
            env!("SAFEC_HOST_TRIPLE"),
            "-o",
            &object.to_string_lossy(),
            &source.to_string_lossy(),
        ],
        scratch.path(),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let linked = Command::new("clang")
        .arg(&object)
        .arg("-o")
        .arg(&program)
        .output()
        .expect("clang answered `--version` a moment ago");
    assert!(
        linked.status.success(),
        "the linker refused: {}",
        String::from_utf8_lossy(&linked.stderr)
    );

    let ran = Command::new(&program)
        .status()
        .expect("the linker wrote a program");
    assert_eq!(ran.code(), Some(3), "the MVP returns 3");
}

/// With no `-o`, an object is named after its input and lands where the run
/// was rather than beside the input, which is what `cc` and `rustc` both do.
///
/// The input is in a directory of its own and its name has two dots, because
/// those are the two things the answer could get wrong: `sub/a.tar.c` becomes
/// `./a.tar.o` and not `sub/a.tar.o`, and not `a.o`.
///
/// Mutation: write it to the stream instead. Nothing is named after the input
/// and this fails. Mutation: use `Path::with_extension` on the whole path. The
/// object lands beside the source and this fails on where it is. (Not on its
/// name: `with_extension` spells `a.tar.c` the same way, which is why the
/// comment beside the code says the difference is the directory.)
#[test]
fn an_object_with_no_path_is_named_after_its_input() {
    if !clang_or_skip("where an object goes when nobody says") {
        return;
    }

    let scratch = Scratch::new("named");
    fs::create_dir_all(scratch.path().join("sub")).expect("the temporary directory is writable");
    scratch.source("sub/a.tar.c", MVP);

    let output = safec(
        &[
            "--emit",
            "object",
            "--target",
            env!("SAFEC_HOST_TRIPLE"),
            "sub/a.tar.c",
        ],
        scratch.path(),
    );

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(
        scratch.path().join("a.tar.o").exists(),
        "nothing is named after the input in {}",
        scratch.path().display()
    );
    assert!(
        !scratch.path().join("sub").join("a.tar.o").exists(),
        "an object was left beside the input"
    );
}

/// A run that could not read its input makes no object, and does not empty the
/// file it was given.
///
/// `clang` answers a valid, useless object for a module with no functions in it,
/// and that object would pass the emptiness test `run_compiler` uses to decide
/// whether a failed run may overwrite a path. What stops it is where the module
/// is built: in the arm, which an input that could not be read never reaches, so
/// the artifact stays empty and nothing is written.
///
/// Mutation: build the module before the loop, the way `--emit llvm-ir`'s header
/// was until #91. The file is replaced by an object with nothing in it and this
/// fails on its contents.
#[test]
fn a_run_that_read_nothing_makes_no_object() {
    if !clang_or_skip("what a failed run leaves where an object would go") {
        return;
    }

    let scratch = Scratch::new("read_nothing");
    let kept = scratch.source(
        "kept.o",
        "what was there before
",
    );

    let output = safec(
        &[
            "--emit",
            "object",
            "--target",
            env!("SAFEC_HOST_TRIPLE"),
            "-o",
            &kept.to_string_lossy(),
            "nosuch.c",
        ],
        scratch.path(),
    );

    let said = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{said}");
    assert!(said.contains("cannot read"), "{said}");
    assert_eq!(
        fs::read_to_string(&kept).expect("the file is still there"),
        "what was there before
"
    );
}

/// A machine with no `clang` is told which tool it is missing, rather than
/// finding out at link time from something opaque.
///
/// The one test here that needs `clang` to be *absent*, which it arranges by
/// emptying the path the child searches. Nothing else this compiler does reads
/// it: the binary is given by an absolute path and its own libraries are not
/// found through it.
///
/// Mutation: report the spawn failure as an ordinary I/O error. The message
/// stops naming `clang` and this fails.
/// Mutation: name a fixed kind in the message rather than the one that was
/// asked for. One of the two halves says the wrong flag and this fails.
#[test]
fn a_missing_clang_says_what_to_install() {
    let scratch = Scratch::new("no_clang");
    let source = scratch.source("mvp.c", MVP);

    let output = Command::new(env!("CARGO_BIN_EXE_safec"))
        .args(["--color", "never", "--emit", "object"])
        .arg(&source)
        .env("PATH", "")
        .current_dir(scratch.path())
        .output()
        .expect("the compiler binary was built for this test");

    let said = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{said}");
    assert!(said.contains("`--emit object` needs `clang`"), "{said}");
    assert!(said.contains("asks clang to make the artifact"), "{said}");
    assert!(said.contains("15 or newer"), "{said}");
    assert!(
        !scratch.path().join("mvp.o").exists(),
        "a run that made nothing left an object"
    );

    // The kind the user typed, not the kind this was written for. Linking needs
    // the same tool and a user who asked for a program is not helped by being
    // told about `--emit object`.
    let linked = Command::new(env!("CARGO_BIN_EXE_safec"))
        .args(["--color", "never", "--emit", "executable"])
        .arg(&source)
        .env("PATH", "")
        .current_dir(scratch.path())
        .output()
        .expect("the compiler binary was built for this test");

    let said = String::from_utf8_lossy(&linked.stderr);
    assert_eq!(linked.status.code(), Some(1), "{said}");
    assert!(said.contains("`--emit executable` needs `clang`"), "{said}");
}

/// `--emit object` takes one input at a time, and says which kind it refused.
///
/// The same rule `--emit llvm-ir` has and for the same reason: one run makes
/// one artifact and writes it to one place, and `cc -c a.c b.c` makes two
/// objects.
///
/// Mutation: let a second input through. The refusal stops and this fails.
/// Mutation: name the kind `llvm-ir` whatever was asked for. The message is
/// about the wrong flag and this fails.
#[test]
fn an_object_is_one_input_at_a_time() {
    let scratch = Scratch::new("two_inputs");
    scratch.source("one.c", "int f(int x);\n");
    scratch.source("two.c", "int f(int x) { return x; }\n");

    let output = safec(&["--emit", "object", "one.c", "two.c"], scratch.path());

    let said = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{said}");
    assert!(said.contains("`--emit object` takes one input"), "{said}");
}

/// A directory to find `clang` in, and the `PATH` that finds nothing else.
///
/// The directory is made; what goes in it is the caller's. Windows needs the
/// `.exe`, because a program is found by that name and not by a mode bit.
///
/// **The only entry, rather than the first.** A search that finds something it
/// cannot execute does not stop: `execvp` remembers the error and carries on to
/// the next entry, so a `clang` that cannot be run is found and then walked past
/// on Linux and macOS, and the real one answers. Leaving nothing else to find is
/// what makes the same arrangement mean the same thing on the three runners.
fn instead_of_clang(scratch: &Scratch) -> (PathBuf, OsString) {
    let tools = scratch.path().join("tools");
    fs::create_dir_all(&tools).expect("the temporary directory is writable");

    let path = env::join_paths([tools.clone()]).expect("one directory is a path");

    (
        tools.join(if cfg!(windows) { "clang.exe" } else { "clang" }),
        path,
    )
}

/// An exit status is not an object, and a `clang` that answers nothing is not a
/// successful run.
///
/// Not a hypothetical machine. A compiler cache or a distributing wrapper is
/// routinely installed under the name `clang`, and one that is misconfigured
/// answers nothing and exits zero. Believing it wrote whatever it did not
/// answer over the path the user named, which is zero bytes, and said so with
/// exit zero.
///
/// The stub is built by the real `clang`, which every test in this file needs
/// anyway, so nothing new has to be on the machine for this to run.
///
/// Mutation: answer `Ok(finished.stdout)` whenever the status is a success.
/// The run exits 0, the kept file becomes empty, and both assertions fail.
/// Mutation: read the program back with `unwrap_or_default`. A link that made
/// nothing answers an empty program, which is written, and the second half of
/// this fails.
#[test]
fn a_clang_that_answers_nothing_is_not_a_success() {
    if !clang_or_skip("a clang that answers no object") {
        return;
    }

    let scratch = Scratch::new("silent_clang");
    let source = scratch.source("mvp.c", MVP);
    let kept = scratch.path().join("mvp.o");
    fs::write(&kept, "what was there before\n").expect("the temporary directory is writable");

    let (stub, searched) = instead_of_clang(&scratch);
    let built = Command::new("clang")
        .arg(scratch.source("stub.c", STUB))
        .arg("-o")
        .arg(&stub)
        .output()
        .expect("there is a clang, which this test checked for");
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let output = Command::new(env!("CARGO_BIN_EXE_safec"))
        .args(["--color", "never", "--emit", "object"])
        .arg(&source)
        .env("PATH", &searched)
        .current_dir(scratch.path())
        .output()
        .expect("the compiler binary was built for this test");

    let said = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{said}");
    assert!(said.contains("did not make an object"), "{said}");
    assert_eq!(
        fs::read_to_string(&kept).expect("the file is still there"),
        "what was there before
",
        "a run that made no object wrote one anyway"
    );

    // The same tool and the same question for the other kind that runs it. A
    // link reads its answer off a file rather than a pipe, so the two are
    // different code and the same defect.
    let linked = Command::new(env!("CARGO_BIN_EXE_safec"))
        .args(["--color", "never", "--emit", "executable", "-o", "prog"])
        .arg(&source)
        .env("PATH", searched)
        .current_dir(scratch.path())
        .output()
        .expect("the compiler binary was built for this test");

    let said = String::from_utf8_lossy(&linked.stderr);
    assert_eq!(linked.status.code(), Some(1), "{said}");
    assert!(said.contains("did not make a program"), "{said}");
    assert!(
        !scratch.path().join("prog").exists(),
        "a run that made no program wrote one anyway"
    );
}

/// A `clang` that could not be started said nothing about the module, and the
/// diagnostic does not pretend otherwise.
///
/// A directory named `clang` is the cheapest way to arrange it and is a real
/// shape: a stale build tree on the path, a binary for another architecture, a
/// file somebody cannot execute. Spawning answers an error that is not "no such
/// file", which is the only thing that separates this from the missing case.
///
/// Needs no `clang`, and passes on a machine with none: what it arranges is
/// found first either way.
///
/// Mutation: fold the spawn failure back into `Unmade::Refused`. The
/// message becomes "clang could not make an object of this module" and the
/// second assertion fails.
#[test]
fn a_clang_that_cannot_be_run_does_not_blame_the_module() {
    let scratch = Scratch::new("unrunnable_clang");
    let source = scratch.source("mvp.c", MVP);

    let (stub, path) = instead_of_clang(&scratch);
    fs::create_dir_all(&stub).expect("the temporary directory is writable");

    let output = Command::new(env!("CARGO_BIN_EXE_safec"))
        .args(["--color", "never", "--emit", "object"])
        .arg(&source)
        .env("PATH", path)
        .current_dir(scratch.path())
        .output()
        .expect("the compiler binary was built for this test");

    let said = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{said}");
    assert!(said.contains("could not run it"), "{said}");
    assert!(!said.contains("of this module"), "{said}");
}

/// A run that reported an error leaves no object, even though `clang` made one.
///
/// The backend refusing a function is the case that separates this from a run
/// that produced nothing: the module still assembles, so there are real bytes
/// to write, and they are a program with a function deleted from it. A build
/// system reads whatever is on disk as finished and newer than the source.
///
/// `p[1]` is what the backend refuses today. When it stops refusing, this test
/// fails on the exit code rather than passing quietly, which is the right way
/// round.
///
/// Mutation: drop `EmitKind::survives_an_error` from the write rule in
/// `run_compiler`, leaving only the empty case. The object is written and this
/// fails.
#[test]
fn a_run_that_reported_an_error_leaves_no_object() {
    if !clang_or_skip("what a failed run leaves behind") {
        return;
    }

    let scratch = Scratch::new("refused_function");
    scratch.source(
        "index.c",
        "int g(int *p) {\n    return p[1];\n}\n\nint main() {\n    return 3;\n}\n",
    );

    let output = safec(&["--emit", "object", "index.c"], scratch.path());

    let said = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{said}");
    assert!(
        !scratch.path().join("index.o").exists(),
        "a run that failed left an object with a function missing from it"
    );
}

/// The name an object is given is refused when it is the name of the input.
///
/// This compiler does not look at an extension to decide what an input is, so
/// `safec --emit object x.o` is a run over a C file that happens to be called
/// `x.o`, and the name it derives is the one it was given. Writing it would
/// destroy the source.
///
/// Mutation: drop the check and let the derived name through. The source is
/// replaced by an object and the last assertion fails.
#[test]
fn an_object_is_not_written_over_its_own_input() {
    let scratch = Scratch::new("named_like_an_object");
    scratch.source("mvp.o", MVP);

    let output = safec(&["--emit", "object", "mvp.o"], scratch.path());

    let said = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{said}");
    assert!(said.contains("would write over its own input"), "{said}");
    assert_eq!(
        fs::read_to_string(scratch.path().join("mvp.o")).expect("the file is still there"),
        MVP,
        "the input was overwritten by the object made from it"
    );
}

/// The MVP program compiles to a native program that exits with 3, which is
/// Phase 3's *Done when*.
///
/// The host's triple, because a program for another machine is one nothing here
/// can run. `an_object_the_linker_accepts` is the same number through a
/// different path and both are worth having: that one holds that `clang`
/// accepts an object this compiler made, and this holds that one spawn from
/// source to program answers the number `docs/roadmap.md` asks for.
///
/// Mutation: answer `Ok(Vec::new())` from `linked`. Nothing is written, the run
/// fails, and this fails on the exit code.
#[test]
fn the_mvp_program_runs_and_answers_three() {
    if !clang_or_skip("whether a linked program answers 3") {
        return;
    }

    let scratch = Scratch::new("mvp_program");
    scratch.source("mvp.c", MVP);
    let program = scratch
        .path()
        .join(if cfg!(windows) { "mvp.exe" } else { "mvp" });

    let output = safec(
        &[
            "--emit",
            "executable",
            "-o",
            &program.to_string_lossy(),
            "mvp.c",
        ],
        scratch.path(),
    );

    let said = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{said}");

    let ran = Command::new(&program)
        .status()
        .expect("the program this compiler just wrote can be run");
    assert_eq!(ran.code(), Some(3), "the program did not answer 3");
}

/// A run that names no path leaves the program where `cc` leaves one, and the
/// plainest invocation there is makes one.
///
/// No `--emit` either, because `EmitKind::Executable` is what `--emit` defaults
/// to: `safec mvp.c` is the run a user who has never read `--help` types, and
/// until this change it said the pipeline was not implemented.
///
/// The name is written out here rather than asked of `Target::program_name`,
/// which is the function under test: a test that computes what it expects from
/// the code it is checking agrees with that code however wrong both are, and
/// this one said in its own note that it caught a mutation it cannot see. That
/// is RK-001's shape and it was found in review.
///
/// Mutation: answer the input's stem from `destination` rather than the
/// machine's name for a program. Nothing is at `a.out` and this fails.
/// Mutation: answer `a.out` from `Target::program_name` on every machine. This
/// fails here on Windows; `every_machine_says_what_a_program_on_it_is_called`
/// in `target.rs` is what fails on all eight rows wherever it runs.
#[test]
fn a_program_with_no_path_is_called_what_cc_calls_it() {
    if !clang_or_skip("what a program with no path is called") {
        return;
    }

    let scratch = Scratch::new("no_path");
    scratch.source("mvp.c", MVP);
    let expected = if cfg!(windows) { "a.exe" } else { "a.out" };

    let output = safec(&["mvp.c"], scratch.path());

    let said = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{said}");

    let ran = Command::new(scratch.path().join(expected))
        .status()
        .expect("the program is where cc would have left it");
    assert_eq!(ran.code(), Some(3), "{expected} did not answer 3");
}

/// A program is runnable by whoever can read it.
///
/// An artifact is bytes and a mode is not one of them, so the mode is put on
/// where the file is written. `fs::write` creates `0o666` before the umask, and
/// a program nobody can execute is not a program.
///
/// Unix only, because Windows has no such bit: a file there is runnable for
/// being a file, and it is the name that decides.
///
/// Mutation: drop the call to `runnable` from `run_compiler`. The mode is
/// `0o644` and this fails.
#[test]
#[cfg(unix)]
fn a_program_is_runnable_by_whoever_can_read_it() {
    use std::os::unix::fs::PermissionsExt as _;

    if !clang_or_skip("whether a program is runnable") {
        return;
    }

    let scratch = Scratch::new("mode");
    scratch.source("mvp.c", MVP);

    let output = safec(
        &["--emit", "executable", "-o", "prog", "mvp.c"],
        scratch.path(),
    );
    let said = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{said}");

    let mode = fs::metadata(scratch.path().join("prog"))
        .expect("the program is on disk")
        .permissions()
        .mode();

    // Wherever the file can be read it can be run, which is what the mode the
    // file was created with decides. The umask is the user's to set and this
    // does not overrule it.
    assert_eq!(mode & 0o444, (mode & 0o111) << 2, "mode {mode:o}");
    assert!(mode & 0o100 != 0, "mode {mode:o}");
}

/// Several translation units become one program, which is what linking is.
///
/// `add.c` defines what `main.c` calls, so a program that answers 3 is one that
/// holds both: each module alone links against nothing. The same number Phase
/// 3's *Done when* asks for, which is the point of choosing it.
///
/// Mutation: link inside the per-input loop again. Each link is missing the
/// other input's symbols and this fails on the exit code.
/// Mutation: keep only the last module. The link answers an undefined symbol
/// and this fails the same way.
/// Mutation: answer `false` from `spans_inputs` for `Executable`. The run is
/// refused and this fails.
#[test]
fn two_inputs_become_one_program() {
    if !clang_or_skip("whether several inputs become one program") {
        return;
    }

    let scratch = Scratch::new("several");
    scratch.source("add.c", "int add(int a, int b) {\n    return a + b;\n}\n");
    scratch.source(
        "main.c",
        "int add(int a, int b);\n\nint main(void) {\n    return add(1, 2);\n}\n",
    );
    let program = scratch
        .path()
        .join(if cfg!(windows) { "prog.exe" } else { "prog" });

    let output = safec(
        &[
            "--emit",
            "executable",
            "-o",
            &program.to_string_lossy(),
            "add.c",
            "main.c",
        ],
        scratch.path(),
    );

    let said = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{said}");

    let ran = Command::new(&program)
        .status()
        .expect("the program this compiler just wrote can be run");
    assert_eq!(ran.code(), Some(3), "the program did not answer 3");
}

/// What the linker says about two inputs names one of them.
///
/// `clang` derives the temporary object names it links from the stems it is
/// handed, and those names are what a linker quotes when two inputs define one
/// symbol. Modules written as `0.ll` and `1.ll` would answer
/// `0-a43ee2.o : error LNK2005: main is already defined in 1-82f49a.o`, which
/// names nothing the user wrote.
///
/// Two `main`s rather than anything subtler, because it is the one link failure
/// that is about which inputs there were rather than about what is in them.
///
/// It also says the order the inputs were given in, which nothing else does:
/// the two modules go under `0-` and `1-`, so the message names which input was
/// which. A test that looked only for the stem passed with the modules handed
/// over backwards, which is how that hole was found.
///
/// Mutation: drop `named` from the file the module is written to. The message
/// names the index alone and this fails.
/// Mutation: hand the modules over in reverse. The indices swap and this
/// fails.
#[test]
fn what_the_linker_says_names_a_file_the_user_named() {
    if !clang_or_skip("what a linker says about two inputs") {
        return;
    }

    let scratch = Scratch::new("two_mains");
    scratch.source("first.c", "int main(void) {\n    return 1;\n}\n");
    scratch.source("second.c", "int main(void) {\n    return 2;\n}\n");

    let output = safec(
        &["--emit", "executable", "-o", "prog", "first.c", "second.c"],
        scratch.path(),
    );

    let said = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{said}");
    // With the index as well as the stem, because the index is the only thing
    // that says the order the user typed reached the linker: `second` alone
    // appears whichever way round the two modules were handed over.
    assert!(said.contains("0-first"), "{said}");
    assert!(said.contains("1-second"), "{said}");
    assert!(
        !scratch.path().join("prog").exists(),
        "a link that failed left a program"
    );
}

/// A program is all of its inputs or none of them.
///
/// The loop is gated on each input and this is gated on the run, which is the
/// difference the two halves of `compile` exist for: a program made of the
/// inputs that happened to compile is not the program that was asked for, and
/// the linker's account of what is missing from it is not a thing a user can
/// act on beside the reason they already have.
///
/// **The input that fails is the one that defines what the other calls**, which
/// is what makes the gate the thing under test. A run whose survivor links by
/// itself would pass either way, because nothing writes a program from a run
/// that reported, and a test resting on that while claiming to hold the gate is
/// a note nobody can act on.
///
/// Mutation: gate `finish` on nothing. The link runs over the caller alone,
/// answers the symbol it cannot find, and this fails on `clang` reaching the
/// report.
#[test]
fn a_program_from_several_inputs_is_all_of_them_or_none() {
    if !clang_or_skip("what a run leaves when one of several inputs failed") {
        return;
    }

    let scratch = Scratch::new("one_of_several");
    scratch.source(
        "caller.c",
        "int add(int a, int b);

int main(void) {
    return add(1, 2);
}
",
    );
    scratch.source(
        "broken.c",
        "int add(int a, int b) {
    return a + b;
}

int f(void) {
    int xs[3];
    return 0;
}
",
    );

    let output = safec(
        &["--emit", "executable", "-o", "prog", "caller.c", "broken.c"],
        scratch.path(),
    );

    let said = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{said}");
    assert!(said.contains("SC0304"), "{said}");
    assert!(!said.contains("clang"), "{said}");
    assert!(
        !scratch.path().join("prog").exists(),
        "a run that could not compile every input left a program"
    );
}

/// A link for another machine says what that would need.
///
/// Assembling for one of the eight targets needs no linker and no sysroot;
/// linking for it needs both, and `clang`'s own words about a missing linker do
/// not mention the target. A machine that *can* cross-link is not refused, so
/// this asserts about the failure rather than asserting that there is one, and
/// says so where there is not.
///
/// Mutation: drop the note from `link_failure`. The message no longer names a
/// sysroot and this fails wherever the link fails, which is every runner.
#[test]
fn linking_for_another_machine_says_what_it_needs() {
    if !clang_or_skip("what a link for another machine says") {
        return;
    }

    let elsewhere = Target::ALL
        .iter()
        .map(|target| target.triple())
        .find(|triple| *triple != env!("SAFEC_HOST_TRIPLE"))
        .expect("more than one machine is known");

    let scratch = Scratch::new("another_machine");
    scratch.source("mvp.c", MVP);

    let output = safec(
        &[
            "--emit",
            "executable",
            "--target",
            elsewhere,
            "-o",
            "prog",
            "mvp.c",
        ],
        scratch.path(),
    );

    let said = String::from_utf8_lossy(&output.stderr);
    if output.status.success() {
        eprintln!(
            "this machine links for {elsewhere}: what it says when it cannot was not checked"
        );
        return;
    }
    assert!(said.contains(elsewhere), "{said}");
    assert!(said.contains("sysroot"), "{said}");
}

/// A run that reported an error leaves no program, though `clang` linked one.
///
/// The same rule `a_run_that_reported_an_error_leaves_no_object` holds, and for
/// the same reason: a module missing a function the backend refused still
/// links, and a program with a function deleted from it is worse on disk than
/// absent.
///
/// Mutation: answer `true` from `survives_an_error` for `Executable`. The
/// program is written and this fails.
#[test]
fn a_run_that_reported_an_error_leaves_no_program() {
    if !clang_or_skip("what a failed link leaves behind") {
        return;
    }

    let scratch = Scratch::new("refused_program");
    scratch.source(
        "index.c",
        "int g(int *p) {\n    return p[1];\n}\n\nint main(void) {\n    return 3;\n}\n",
    );

    let output = safec(
        &["--emit", "executable", "-o", "prog", "index.c"],
        scratch.path(),
    );

    let said = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{said}");
    assert!(
        !scratch.path().join("prog").exists(),
        "a run that failed left a program with a function missing from it"
    );
}

/// A `clang` that exits successfully and makes an empty file wherever `-o`
/// named, which is what a compiler cache or a distributing wrapper does when it
/// is misconfigured.
///
/// **It creates the file rather than leaving nothing**, because those are two
/// different silences and only one of them is caught by an exit status: a run
/// that reads its answer back off disk sees a file that is there and empty, and
/// a run that reads its answer off a pipe sees the same nothing either way.
const STUB: &str = r#"#include <stdio.h>

int main(int argc, char **argv) {
    for (int i = 1; i + 1 < argc; i++) {
        if (argv[i][0] == '-' && argv[i][1] == 'o' && argv[i][2] == 0) {
            FILE *made = fopen(argv[i + 1], "wb");
            if (made) {
                fclose(made);
            }
        }
    }
    return 0;
}
"#;

/// A program with nothing to start from, which is the commonest link failure
/// there is.
const NO_MAIN: &str = "int f(int x) {\n    return x;\n}\n";

/// A link that failed on this machine says nothing about a sysroot.
///
/// The note that says what linking for another machine needs is added only when
/// the target is not the host, and a user whose own link failed for an ordinary
/// reason should not be sent to look for a cross toolchain they do not need.
///
/// Mutation: add the note whatever the target. This fails, and
/// `linking_for_another_machine_says_what_it_needs` goes on passing, which is
/// why the branch needs both halves.
#[test]
fn a_failed_link_here_says_nothing_about_a_sysroot() {
    if !clang_or_skip("what a link that failed on this machine says") {
        return;
    }

    let scratch = Scratch::new("no_main");
    scratch.source("nomain.c", NO_MAIN);

    let output = safec(
        &["--emit", "executable", "-o", "prog", "nomain.c"],
        scratch.path(),
    );

    let said = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{said}");
    assert!(!said.contains("sysroot"), "{said}");
}

/// What the linker said reaches the user, even where it says it on the other
/// stream.
///
/// `link.exe` writes its own diagnosis to standard output and `clang`'s driver
/// writes the summary to standard error, so on Windows the line that says *why*
/// is the one a compile would have to throw away, because for a compile that
/// stream is the object. Measured here: a module with no `main` answers
/// `LINK : fatal error LNK1561` on standard output.
///
/// Windows only, because it is the only machine where the two streams disagree.
/// Where the linker speaks on standard error the diagnostic already carried it,
/// which is what made this invisible.
///
/// Mutation: keep only standard error for a link, the way a compile does. The
/// diagnostic falls back to the exit code alone and this fails.
#[test]
#[cfg(windows)]
fn a_failed_link_says_more_than_an_exit_code() {
    if !clang_or_skip("whether the linker's own words reach the user") {
        return;
    }

    let scratch = Scratch::new("linker_words");
    scratch.source("nomain.c", NO_MAIN);

    let output = safec(
        &["--emit", "executable", "-o", "prog", "nomain.c"],
        scratch.path(),
    );

    let said = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{said}");
    assert!(said.contains("linker command failed"), "{said}");
    // The linker's own code for "an entry point must be specified". Its
    // sentence is in the machine's code page and arrives mangled; the code is
    // ASCII and is the part worth holding.
    assert!(said.contains("LNK"), "{said}");
}

/// A machine with nowhere to work is not told about `clang`.
///
/// A program is linked in a directory of its own, so a temporary directory that
/// cannot be made is a failure with nothing to do with the tool. Reporting it in
/// the tool's words sends a user to inspect an installation that is fine, which
/// is the mistake RK-024 records one level over.
///
/// Mutation: answer `Unlinked::Tool(Unmade::Unrunnable(..))` when the directory
/// cannot be made. The message names `clang` and this fails.
#[test]
fn a_machine_with_nowhere_to_work_is_not_told_about_clang() {
    let scratch = Scratch::new("nowhere");
    let source = scratch.source("mvp.c", MVP);
    let nowhere = scratch.path().join("no").join("such").join("directory");

    let output = Command::new(env!("CARGO_BIN_EXE_safec"))
        .args(["--color", "never", "--emit", "executable", "-o", "prog"])
        .arg(&source)
        // The three a temporary directory is read from, so that this says the
        // same thing on the three runners.
        .env("TMPDIR", &nowhere)
        .env("TMP", &nowhere)
        .env("TEMP", &nowhere)
        .current_dir(scratch.path())
        .output()
        .expect("the compiler binary was built for this test");

    let said = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{said}");
    assert!(said.contains("needs somewhere to work"), "{said}");
    assert!(!said.contains("clang"), "{said}");
}

/// The refusal to write over an input says which name it means, and how it got
/// it.
///
/// One check, two kinds, and the notes were written for the one that came
/// first: an object is named after its input, and a program is named after
/// nothing. A user who wrote `-o` is also not helped by being told to write
/// `-o`, which is how they got here.
///
/// Mutation: answer the object's note for both kinds. The first assertion
/// fails.
/// Mutation: ignore `options.output` when saying where the name came from. The
/// second run is told to name it with `-o`, which it did, and this fails.
#[test]
fn what_a_refused_destination_says_fits_the_kind_that_asked() {
    let scratch = Scratch::new("named_like_a_program");
    let program = if cfg!(windows) { "a.exe" } else { "a.out" };
    scratch.source(program, MVP);
    scratch.source("mvp.c", MVP);

    let derived = safec(&["--emit", "executable", program], scratch.path());

    let said = String::from_utf8_lossy(&derived.stderr);
    assert_eq!(derived.status.code(), Some(1), "{said}");
    assert!(said.contains("would write over its own input"), "{said}");
    assert!(!said.contains("an object"), "{said}");

    let given = safec(
        &["--emit", "executable", "-o", "mvp.c", "mvp.c"],
        scratch.path(),
    );

    let said = String::from_utf8_lossy(&given.stderr);
    assert_eq!(given.status.code(), Some(1), "{said}");
    assert!(said.contains("`-o` named it"), "{said}");
    assert!(!said.contains("name it with `-o`"), "{said}");
}

/// A unit the frontend reported on never reaches `clang`.
///
/// A type the lowering cannot hold leaves a module with a function missing from
/// it, which still assembles and links against nothing. Without this the run
/// ends with the frontend saying what is wrong and a linker saying `exit code
/// 1561`, and the second one names another program and a number the user can do
/// nothing with.
///
/// Mutation: drop the `said_something` gate from either arm. The link runs, its
/// exit code reaches the report, and this fails.
#[test]
fn a_unit_the_frontend_reported_on_never_reaches_clang() {
    if !clang_or_skip("what a unit the frontend reported on is handed to") {
        return;
    }

    let scratch = Scratch::new("reported_on");
    scratch.source(
        "array.c",
        "int f(void) {\n    int xs[3];\n    return 0;\n}\n\nint main(void) {\n    return f();\n}\n",
    );

    for kind in ["object", "executable"] {
        let output = safec(&["--emit", kind, "-o", "made", "array.c"], scratch.path());

        let said = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{kind}: {said}");
        assert!(said.contains("SC0304"), "{kind}: {said}");
        assert!(!said.contains("clang"), "{kind}: {said}");
        assert!(
            !scratch.path().join("made").exists(),
            "{kind} left something behind"
        );
    }
}

/// Two inputs that share a stem are two modules.
///
/// `a/x.c` and `b/x.c` are ordinary in a project with a directory per part, and
/// the name a module goes under is the input's. Without the index they would be
/// one name written twice, so the second would overwrite the first and the link
/// would be handed one module twice: every symbol in it defined twice, from a
/// file the user cannot see.
///
/// Mutation: drop the index from the name a module is written under. The link
/// answers a duplicate symbol and this fails on the exit code.
#[test]
fn two_inputs_that_share_a_stem_are_two_modules() {
    if !clang_or_skip("whether two inputs with one stem are two modules") {
        return;
    }

    let scratch = Scratch::new("one_stem");
    fs::create_dir_all(scratch.path().join("sub")).expect("the temporary directory is writable");
    scratch.source(
        "same.c",
        "int add(int a, int b);\n\nint main(void) {\n    return add(1, 2);\n}\n",
    );
    scratch.source(
        "sub/same.c",
        "int add(int a, int b) {\n    return a + b;\n}\n",
    );
    let program = scratch
        .path()
        .join(if cfg!(windows) { "both.exe" } else { "both" });

    let output = safec(
        &[
            "--emit",
            "executable",
            "-o",
            &program.to_string_lossy(),
            "same.c",
            "sub/same.c",
        ],
        scratch.path(),
    );

    let said = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{said}");

    let ran = Command::new(&program)
        .status()
        .expect("the program this compiler just wrote can be run");
    assert_eq!(ran.code(), Some(3), "the program did not answer 3");
}
