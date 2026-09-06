//! The Safe C compiler.
//!
//! The `safec` binary is a thin wrapper around this crate: it parses arguments
//! and maps the result to a process exit code. Everything else lives here so
//! that integration tests can drive the compiler in-process instead of
//! spawning a subprocess.

pub mod cli;
pub mod options;
pub mod source;
