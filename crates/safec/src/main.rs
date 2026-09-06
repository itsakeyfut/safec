//! The `safec` binary.
//!
//! A thin wrapper: it parses the arguments, hands the result to the library,
//! and maps what comes back to a process exit code. The compiler itself lives
//! in the `safec` library, so that tests can drive it in process.

use std::process::ExitCode;

use clap::Parser;

use safec::cli::Cli;

fn main() -> ExitCode {
    let options = Cli::parse().into_options();

    // No driver yet: echo the resolved options so the interface can be
    // exercised end to end. Replaced by the real pipeline in a later change.
    println!("safec: {options:#?}");

    ExitCode::SUCCESS
}
