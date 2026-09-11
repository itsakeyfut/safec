//! What `--emit object` writes, and what a machine without `clang` is told.
//!
//! An object is not text, so the corpus cannot hold one: `cases.rs` compares
//! bytes this compiler wrote against bytes this compiler wrote, and these bytes
//! came from `clang`. What can be held is what the object *is*, which its own
//! header says, and that something else accepts it.
//!
//! Every test here needs a `clang`, because `--emit object` needs one: ADR-0015
//! says why. They say what they did not check where there is none, and
//! `SAFEC_REQUIRE_LLVM` makes that a failure, which CI sets.

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
    assert!(said.contains("needs `clang`"), "{said}");
    assert!(said.contains("asks clang to make the artifact"), "{said}");
    assert!(said.contains("15 or newer"), "{said}");
    assert!(
        !scratch.path().join("mvp.o").exists(),
        "a run that made nothing left an object"
    );
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
#[test]
fn a_clang_that_answers_nothing_is_not_a_success() {
    if !clang_or_skip("a clang that answers no object") {
        return;
    }

    let scratch = Scratch::new("silent_clang");
    let source = scratch.source("mvp.c", MVP);
    let kept = scratch.path().join("mvp.o");
    fs::write(&kept, "what was there before\n").expect("the temporary directory is writable");

    let (stub, path) = instead_of_clang(&scratch);
    let built = Command::new("clang")
        .arg(scratch.source("stub.c", "int main(void) { return 0; }\n"))
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
        .env("PATH", path)
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
/// Mutation: fold the spawn failure back into `Unassembled::Refused`. The
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
/// Mutation: answer the input's stem from `destination` rather than the
/// machine's name for a program. Nothing is at `a.out` and this fails.
/// Mutation: answer `a.out` from `Target::program_name` on every machine. This
/// fails on Windows and passes elsewhere, which is what the eight rows in
/// `target.rs` are for.
#[test]
fn a_program_with_no_path_is_called_what_cc_calls_it() {
    if !clang_or_skip("what a program with no path is called") {
        return;
    }

    let scratch = Scratch::new("no_path");
    scratch.source("mvp.c", MVP);
    let expected = Target::from_triple(env!("SAFEC_HOST_TRIPLE"))
        .expect("the host is a target this compiler knows")
        .program_name();

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

/// `--emit executable` takes one input at a time, like the two kinds before it.
///
/// A program is made of several translation units by definition, and this links
/// one: the rule follows what the implementation does, because a rule that
/// promises more is a run that silently builds the last input and throws the
/// rest away.
///
/// Mutation: answer `true` from `spans_inputs` for `Executable`. The refusal
/// stops, one of the two inputs is linked, and this fails.
#[test]
fn a_program_is_one_input_at_a_time() {
    let scratch = Scratch::new("two_programs");
    scratch.source("one.c", "int f(int x);\n");
    scratch.source("two.c", "int main(void) { return 3; }\n");

    let output = safec(&["--emit", "executable", "one.c", "two.c"], scratch.path());

    let said = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{said}");
    assert!(
        said.contains("`--emit executable` takes one input"),
        "{said}"
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
