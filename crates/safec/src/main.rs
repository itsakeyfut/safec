use std::process::ExitCode;

fn main() -> ExitCode {
    println!("safec {}", env!("CARGO_PKG_VERSION"));
    ExitCode::SUCCESS
}
