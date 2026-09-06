//! The `safec` binary.
//!
//! A thin wrapper: it parses the arguments, hands the result to the library,
//! and maps what comes back to a process exit code. The compiler itself lives
//! in the `safec` library, so that tests can drive it in process.

use std::io;
use std::process::ExitCode;

use clap::Parser;

use safec::cli::Cli;
use safec::driver;

fn main() -> ExitCode {
    let options = Cli::parse().into_options();

    // The driver takes the stream so that a test can read what a run said. The
    // binary is the one that decides diagnostics belong on stderr.
    let mut stderr = io::stderr().lock();

    match driver::run_compiler(&options, &mut stderr) {
        Ok(outcome) => outcome.into(),
        // The diagnostics could not be written, so there is no channel left to
        // explain that on. The exit code carries it alone.
        Err(_) => ExitCode::FAILURE,
    }
}
