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
