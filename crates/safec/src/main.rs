//! The `safec` binary.
//!
//! A thin wrapper: it parses the arguments, hands the result to the library,
//! and maps what comes back to a process exit code. The compiler itself lives
//! in the `safec` library, so that tests can drive it in process.

use std::io::{self, Write as _};
use std::process::ExitCode;

use clap::Parser;

use safec::cli::Cli;
use safec::driver;

fn main() -> ExitCode {
    let options = Cli::parse().into_options();

    // The driver takes both streams so that a test can read what a run said and
    // what it made. The binary is the one that decides diagnostics belong on
    // stderr and an artifact on stdout, so that `safec --emit tokens a.c > a.tok`
    // captures the tokens and leaves the diagnostics on the terminal.
    let mut stderr = io::stderr().lock();
    let mut stdout = io::stdout().lock();

    let outcome = driver::run_compiler(&options, &mut stderr, &mut stdout);

    // Flushed here rather than left to the runtime, because the exit code is a
    // claim about the artifact. A buffered write that never reached the file is
    // a run that exited successfully without producing what was asked for.
    //
    // The runtime does flush at exit, so this changes nothing that a test can
    // reach and no test guards it. What it changes is what happens when the
    // flush *fails*, on a full disk or a closed pipe: the runtime discards that
    // and exits with whatever was returned, and this reports it.
    match outcome.and_then(|outcome| stdout.flush().map(|()| outcome)) {
        Ok(outcome) => outcome.into(),
        // Nothing could be written, so there is no channel left to explain that
        // on. The exit code carries it alone.
        Err(_) => ExitCode::FAILURE,
    }
}
