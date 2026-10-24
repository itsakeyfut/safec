//! Everything that reads C, from the preprocessor to the driver.
//!
//! Depend on this crate to read C into the Safety IR, or to run the whole
//! pipeline in process with [`driver::run_compiler`]. Depend on `safec` for
//! the compiler as one crate, which re-exports this one as `safec::c` and
//! holds the `safec` binary: a thin wrapper that parses arguments with
//! [`cli::Cli`] and maps what [`driver`] answers to an exit code. See
//! ADR-0058.
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
pub mod preprocessor;
pub mod safety;
pub mod sema;
pub mod token;
pub mod types;
