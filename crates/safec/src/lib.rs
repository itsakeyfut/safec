//! The Safe C compiler.
//!
//! The `safec` binary is a thin wrapper around this crate: it parses arguments
//! and maps the result to a process exit code. Everything else lives here so
//! that integration tests can drive the compiler in-process instead of
//! spawning a subprocess.
//!
//! Everything a program becomes once the C is gone is in `safec_ir` instead:
//! the IR, its printer, its interpreter, and the source map a span is an
//! offset into. That crate does not depend on this one and must not; ADR-0011
//! is why, and cargo is what holds it.

pub mod ast;
pub mod cli;
pub mod diagnostics;
pub mod driver;
pub mod lexer;
pub mod lowering;
pub mod options;
pub mod parser;
pub mod safety;
pub mod sema;
pub mod token;
pub mod types;
