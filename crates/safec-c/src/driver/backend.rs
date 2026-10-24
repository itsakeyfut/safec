//! Running the backend: the LLVM module one unit becomes, `clang` assembling
//! it into an object or linking several into a program, the scratch directory a
//! link works in, and what each of them failing reads as.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::SystemTime;

use crate::cli::HOST_TRIPLE;
use crate::diagnostics::{Code, Diagnostic, DiagnosticSink, Label};
use crate::options::{EmitKind, Options};
use safec_ir::ir::TranslationUnit;
use safec_ir::source::SourceMap;
use safec_ir::target::Target;
use safec_llvm::emit::Refusal;

// Code generation takes `SC08xx`, which `docs/diagnostics.md` allocates. One
// code for every shape of a refusal, for the reason `types.rs` gives for
// `MISMATCH`: what differs between them is the message, and a reader filtering
// on the code wants "the backend could not write this" rather than a list of
// the ways that can happen.
//
// One of the two diagnostics in this file that carry one. The rest are the
// driver saying something about a run or about the machine it is on, and this
// is the backend saying something about a program, which is the line
// `docs/diagnostics.md` draws. No count here: that document carries one, with
// the command that settles it, and two places counting the same thing is one
// place too many.
const BACKEND: Code = Code::new("SC0801");

/// The module one unit becomes, with whatever the backend could not write
/// already reported.
///
/// Shared by the two `--emit` kinds that reach the backend rather than written
/// out in each, for the reason [`Emitted`](super::Emitted) gives for existing: two copies of a
/// rule are two things that have to agree.
///
/// The header goes in here rather than where the artifact is made, so that a
/// run which read none of its inputs leaves nothing rather than a module with
/// no functions in it. `run_compiler` reads emptiness as "made nothing" and
/// declines to write it over a path the user gave; a header alone would pass
/// that test and destroy the file. Once per artifact, because both kinds refuse
/// a second input and a module may carry one `target triple`.
pub(super) fn module(
    sources: &SourceMap,
    unit: &TranslationUnit,
    target: Target,
    diagnostics: &mut DiagnosticSink,
) -> String {
    let mut module = safec_llvm::emit::header(target);

    // The backend answers what it could not write rather than reporting it,
    // because it cannot see a `Diagnostic`: ADR-0011 put those in this crate.
    // Every function it could write is in `module` already, and the ones it
    // could not are declarations.
    for refusal in safec_llvm::emit::functions(sources, unit, &mut module) {
        diagnostics.report(backend_failure(&refusal));
    }

    module
}

/// Why `clang` could not make what was asked of it.
///
/// Four answers rather than two, because a program on the path is not always
/// the program its name says. Only [`Self::Refused`] is `clang` speaking about
/// a module; the other three are the machine speaking about `clang`, and
/// wording them as a refusal blames a user's program for their installation.
/// That has been done here once, to a `clang` that never ran.
#[derive(Debug)]
pub(super) enum Unmade {
    /// There is no `clang` to run.
    Absent,
    /// There is something by that name and it could not be started, and this is
    /// what the operating system said about it.
    Unrunnable(String),
    /// It ran and refused, or could not be spoken to, and this is what it said.
    Refused(String),
    /// It ran, said nothing was wrong, and made nothing.
    Silent,
}

/// Run `clang` over a module and hand back what it said.
///
/// Spawning a tool rather than linking one is ADR-0015, which also says what it
/// costs: this is the only thing here that needs a program at run time, and the
/// two `--emit` kinds that reach it do not work on a machine without one.
///
/// The two callers differ in what they ask for and in what they read back, and
/// share everything about what a tool on the path might do instead of the job,
/// which is why this is one function. The arguments carry the job; `-x ir` and
/// the target are here because both jobs take modules for a named machine.
///
/// `-Wno-override-module` because the module names its own triple and `clang`'s
/// own is more specific, so it warns about agreeing.
///
/// **`-x ir` is sticky**: it says what every file after it is, so a job naming
/// something that is not a module would have that read as one too. Both jobs
/// hand over modules, and a third that does not would move the flag into the
/// jobs rather than add a file to one. The failure is loud, which is why this
/// is a sentence rather than a shape.
///
/// **A module goes in on standard input or does not.** One does, for a compile,
/// because the object comes back on the other pipe and nothing needs a name. A
/// link names files instead, because there can be several and only one of them
/// could be a stream; there is then nothing to write, and the child is given a
/// standard input that is already at its end rather than a pipe nobody writes
/// to.
///
/// **When there is one, it is written on a thread.** Both pipes are open at
/// once, and a module larger than the pipe buffer would otherwise deadlock
/// against a `clang` that has begun answering before it has finished reading.
/// About 64 KiB on this host, which an ordinary `.c` file reaches; the same
/// hazard was measured in `tests/llvm.rs` and is why that one does not `expect`
/// its write.
fn clang(job: &[&OsStr], stdin: Option<&str>, target: Target) -> Result<Output, Unmade> {
    let mut spawned = Command::new("clang")
        .args(["-x", "ir", "-Wno-override-module"])
        .arg(format!("--target={}", target.triple()))
        .args(job)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        // A spawn that failed for any other reason is a third answer and not a
        // refusal: `clang` never ran, so it has said nothing about the module,
        // and wording it as though it had blames the program for a directory
        // named `clang` on the path, or a binary this machine cannot start.
        .map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => Unmade::Absent,
            _ => Unmade::Unrunnable(error.to_string()),
        })?;

    let feeding = stdin.map(|module| {
        let mut input = spawned.stdin.take().expect("the pipe was asked for");
        let written = module.to_owned();
        thread::spawn(move || input.write_all(written.as_bytes()))
    });

    let finished = spawned
        .wait_with_output()
        .map_err(|error| Unmade::Refused(error.to_string()))?;

    // After the output, because a `clang` that gave up early breaks the pipe
    // and its own words are the better answer. A write that failed for any
    // other reason surfaces as a `clang` that got an incomplete module and
    // said so.
    if let Some(feeding) = feeding {
        let _ = feeding.join();
    }

    Ok(finished)
}

/// What `clang` said when it refused, as far as this job can hear it.
///
/// **Which stream carries the words depends on the job.** For a compile they
/// are on standard error, because standard output is the object. For a link
/// they can be on either: `link.exe` writes its own diagnosis to standard
/// output and `clang`'s driver writes the summary to standard error, so on
/// Windows the line that says *why* is the one a compile would have to throw
/// away. Measured on this host: a module with no `main` answers
/// `LINK : fatal error LNK1561` on standard output and
/// `clang: error: linker command failed with exit code 1561` on standard error.
///
/// Another tool's text, in whatever encoding that tool writes: `from_utf8_lossy`
/// rather than a failure, because a mangled sentence is worth more to a user
/// than none. `docs/architecture.md` records this as a divergence from "do not
/// let the host into a diagnostic".
fn refused(streams: &[&[u8]]) -> Unmade {
    let said: Vec<String> = streams
        .iter()
        // A carriage return in the middle is the tool's line ending rather than
        // something it meant to say, and the renderer escapes what it does not
        // recognise, so leaving one in puts a literal escape in a note.
        // Trimming the ends is not enough once a tool writes more than one
        // line, which is what a linker over several modules does.
        .map(|stream| String::from_utf8_lossy(stream).replace("\r\n", "\n"))
        .map(|stream| stream.trim().to_owned())
        .filter(|stream| !stream.is_empty())
        .collect();

    Unmade::Refused(said.join("\n"))
}

/// One module as an object for the machine it names.
///
/// The object comes back on the second pipe, so nothing here needs a temporary
/// file and no temporary name reaches the object: `clang` records what it was
/// given, and the same module assembled twice is the same bytes.
///
/// An exit status is not evidence that an object exists. A `clang` on the path
/// is not always LLVM's: a compiler cache or a distributing wrapper is routinely
/// installed under that name, and one that is misconfigured answers nothing and
/// exits successfully. Taking that as an object writes zero bytes over whatever
/// `-o` named and exits zero, which is the one thing `compile` says a compiler
/// must never do.
pub(super) fn assembled(module: &str, target: Target) -> Result<Vec<u8>, Unmade> {
    let finished = clang(
        &[
            OsStr::new("-c"),
            OsStr::new("-o"),
            OsStr::new("-"),
            OsStr::new("-"),
        ],
        Some(module),
        target,
    )?;

    if !finished.status.success() {
        // Standard output is the object here, not words, whatever is on it.
        return Err(refused(&[&finished.stderr]));
    }
    if finished.stdout.is_empty() {
        return Err(Unmade::Silent);
    }
    Ok(finished.stdout)
}

/// Several modules as one program for the machine it names.
///
/// One spawn and not one per module: `clang -x ir` with no `-c` reads every
/// module it is given and answers a linked program, so there is no object in
/// between and nothing to keep one in. Measured rather than assumed, and it is
/// why this does not go through [`assembled`].
///
/// **The modules go to a directory of this run's own, and so does the
/// program.** Only one of them could have been a stream, and a linker cannot
/// write to one either: `-o -` makes a file called `-` and exits successfully,
/// measured. Reading the program back rather than pointing `clang` at what the
/// user asked for is what keeps `compile` from writing to a path the user
/// named, which is the invariant the write rule in [`run_compiler`](super::run_compiler) rests on.
/// The directory's own name reaches neither, measured: the same modules linked
/// from two differently named directories answer the same bytes. That is not
/// the same as two runs agreeing, and they do not: `link.exe` stamps the time
/// into a program, so two runs a second apart differ in four bytes. What is
/// reproducible is what this compiler decides, which is why the claim is about
/// the directory rather than about the program.
///
/// **One command line names every module, and a command line has a ceiling.**
/// About six hundred inputs with short names on this host, where the limit is
/// 32767 characters; fewer with the paths a real project has. Past it the spawn
/// fails and the run says so, honestly but in the wrong voice: the fault is
/// this compiler's command line rather than the user's installation, and the
/// message is the one about `clang`. A response file is what answers it, and
/// nothing reaches the ceiling while an input is one file a person wrote.
///
/// It does reach a *diagnostic*, because a linker quotes the path it was told
/// to write as well as the files it was given: a duplicate symbol answers
/// `<scratch>\program : fatal error LNK1169`. That is another tool's text and
/// `docs/architecture.md` records passing it through as a divergence; the only
/// way to keep the path out of it is to hand `clang` what the user asked for,
/// which is the invariant above.
pub(super) fn linked(modules: &[Module], target: Target) -> Result<Vec<u8>, Unlinked> {
    let scratch = Scratch::new().map_err(|error| Unlinked::Nowhere(error.to_string()))?;
    let program = scratch.path().join("program");

    let mut job = vec![
        OsStr::new("-o").to_owned(),
        program.clone().into_os_string(),
    ];
    for (index, module) in modules.iter().enumerate() {
        let written = scratch.path().join(format!("{index}-{}.ll", module.named));
        fs::write(&written, &module.text).map_err(|error| Unlinked::Nowhere(error.to_string()))?;
        job.push(written.into_os_string());
    }

    let job: Vec<&OsStr> = job.iter().map(AsRef::as_ref).collect();
    let finished = clang(&job, None, target).map_err(Unlinked::Tool)?;

    if !finished.status.success() {
        // Both streams, because the linker and the driver that ran it do not
        // agree about which one to speak on. See `refused`.
        return Err(Unlinked::Tool(refused(&[
            &finished.stdout,
            &finished.stderr,
        ])));
    }

    // What the linker wrote, or that it wrote nothing. `fs::read` answers the
    // second as an error, which is the same fact `assembled` reads off an empty
    // pipe and means the same thing: a successful exit is not a program.
    match fs::read(&program) {
        Ok(bytes) if !bytes.is_empty() => Ok(bytes),
        _ => Err(Unlinked::Tool(Unmade::Silent)),
    }
}

/// Why a module could not be made into a program.
///
/// Two parties, and telling them apart is the whole reason this is not
/// [`Unmade`]. `clang` answers for the first; the second is this compiler
/// failing to find anywhere to work, which is nothing to do with the tool and
/// must not be reported as though the tool were broken. That is [`Unmade`]'s
/// mistake one level over: blaming whoever is nearest.
#[derive(Debug)]
pub(super) enum Unlinked {
    /// What `clang` did, or did not do.
    Tool(Unmade),
    /// There was nowhere to put the program while it was being made, and this
    /// is what the operating system said about that.
    Nowhere(String),
}

/// One input's module, and the name it goes under.
///
/// **The name is the input's.** `clang` derives the temporary object names it
/// links from the stems it is handed, and those names are what the linker
/// quotes when two inputs define one symbol: two modules written as `0.ll` and
/// `1.ll` produce `0-a43ee2.o : error LNK2005: main is already defined in
/// 1-82f49a.o`, which names nothing the user wrote. Measured. The index keeps
/// two inputs with one stem apart, which `a/x.c` and `b/x.c` are.
pub(super) struct Module {
    pub(super) named: String,
    pub(super) text: String,
}

/// A directory of one link's own, removed however the link ends.
///
/// The only thing this compiler writes outside a path the user named, and it
/// exists because a linker will not answer on a pipe. Not called a workspace,
/// because this file already uses that word for the one `cargo` builds.
///
/// Made with `create_dir` rather than `create_dir_all`, so that a name already
/// taken is an error here rather than a directory shared with whoever holds it.
///
/// **The name cannot be the process and a count alone.** Removal is best effort
/// and acquisition is strict, so a run that is killed leaves its directory
/// behind, and an operating system that reuses process ids hands the name to
/// somebody else: `--emit executable` would then fail for that process every
/// time, permanently, with a message about a tool that is not the trouble. The
/// clock is what makes a leftover harmless; the count is what keeps two threads
/// of one process apart inside the same tick, which the unit tests need.
pub(super) struct Scratch(PathBuf);

impl Scratch {
    pub(super) fn new() -> io::Result<Self> {
        static LINKS: AtomicUsize = AtomicUsize::new(0);

        let since = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            // Before 1970 on a machine whose clock says so. The count alone is
            // still unique within this process, which is what matters here.
            .unwrap_or(0);

        let path = std::env::temp_dir().join(format!(
            "safec-{}-{since}-{}",
            std::process::id(),
            LINKS.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    pub(super) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // Nothing to report it to, and nothing a user could do about it: the
        // program has already been read out of here.
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// What the backend could not write, as a diagnostic.
///
/// The refusal's own sentence is the message, because it is the specific half:
/// "the backend cannot write an indexed place, which counts elements and so
/// needs a width" says more than a heading and a note would. `load_failure` has
/// the same shape for the same reason.
///
/// A [`Refusal`] never carries text out of a source file, only type spellings
/// this compiler wrote and numbers, so nothing here needs `shown`. That is
/// worth stating rather than assuming, because every new place that echoes
/// text has to answer for whether it is a source file's.
pub(super) fn backend_failure(refusal: &Refusal) -> Diagnostic {
    let reported =
        Diagnostic::error(format!("the backend cannot write {}", refusal.why)).with_code(BACKEND);

    match refusal.at {
        Some(span) => reported.with_label(Label::primary(span, "this is what it could not write")),
        // Only a call among the terminators carries a span, so a refusal about
        // one of the others has nowhere to point and says so.
        None => reported.with_note("the IR does not say where this came from"),
    }
}

/// Why `clang` could not make what was asked of it, as a diagnostic.
///
/// No code, like the others beside it and unlike `SC0801`: this is the driver
/// saying something about the machine a run is on rather than about the program
/// it was given. `docs/diagnostics.md` draws that line.
///
/// The absent case says what to install, because a user who reaches it has a
/// working compiler and a missing tool, and "cannot run clang" on its own leaves
/// them to guess which clang and why. The refused case passes `clang`'s own
/// words through rather than interpreting them: it knows what is wrong with a
/// module and this does not, and a clang older than LLVM 15 answers about the
/// opaque pointers this writes.
///
/// The other two are the cases where nothing was heard from `clang` at all, and
/// they are separate for exactly that reason. Saying "could not make an object
/// of this module" about a run that never started, or about one that started
/// and answered nothing, points a user at their program when the fault is on
/// their machine.
///
/// `made` is the caller's word for what it asked for, and `kind` is what the
/// user typed. Both callers know which they are, so neither is looked up: a
/// table would need an answer from every kind that never reaches `clang`, and
/// the only honest answers there are unreachable.
pub(super) fn clang_failure(kind: EmitKind, made: &str, why: &Unmade) -> Diagnostic {
    match why {
        Unmade::Absent => Diagnostic::error(format!(
            "`--emit {}` needs `clang` and found none",
            kind.spelling()
        ))
        .with_note("safec writes LLVM IR and asks clang to make the artifact out of it")
        .with_note("any clang whose LLVM is 15 or newer reads the IR this writes"),
        Unmade::Unrunnable(said) => Diagnostic::error(format!(
            "`--emit {}` found `clang` and could not run it",
            kind.spelling()
        ))
        .with_note(said.trim())
        .with_note("this is what the machine said, not what clang said"),
        Unmade::Refused(said) => {
            // Not "of this module": a link is over as many as the run had
            // inputs, and counting them in a message is a count to keep right
            // for nothing. What it could not make is the part a user needs.
            let reported = Diagnostic::error(format!("clang could not make {made}"));
            // A `clang` that was killed, or that crashed, exits unsuccessfully
            // with nothing to say. An empty note is worse than no note: it
            // reads as a message this compiler failed to fill in.
            match said.trim() {
                "" => reported.with_note("it exited unsuccessfully and said nothing"),
                said => reported.with_note(said),
            }
        }
        Unmade::Silent => Diagnostic::error(format!(
            "clang did not make {made} and said nothing was wrong"
        ))
        .with_note("the clang on this path may be a wrapper rather than a compiler"),
    }
}

/// The same, for a link, which has one answer `clang` has nothing to do with.
///
/// Linking is the one job here that can fail for being asked about another
/// machine. Assembling for one needs no linker and no sysroot, which is why
/// every target in `Target::ALL` assembles on this machine and only the host
/// links; `clang`'s words about a missing linker do not mention the target, so
/// this adds the sentence that does.
///
/// Added rather than replacing: a user with a cross toolchain is not refused,
/// and one without it is told what would have been needed.
pub(super) fn link_failure(options: &Options, why: &Unlinked) -> Diagnostic {
    let tool = match why {
        Unlinked::Tool(why) => why,
        // Nothing was asked of `clang`, so nothing here is about it. Saying so
        // in `clang`'s words would send a user to look at an installation that
        // is fine.
        Unlinked::Nowhere(said) => {
            return Diagnostic::error("`--emit executable` needs somewhere to work and found none")
                .with_note(said.trim())
                .with_note("a program is linked in a directory of its own and read back from it")
                .with_note("this is where TMPDIR, or TMP and TEMP, point");
        }
    };

    let reported = clang_failure(options.emit, "a program", tool);

    if options.target.triple() == HOST_TRIPLE {
        return reported;
    }
    reported.with_note(format!(
        "linking for {} on a {} machine needs a linker and a sysroot for it",
        options.target.triple(),
        HOST_TRIPLE
    ))
}
