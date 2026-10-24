//! The Safe C compiler, as one crate.
//!
//! This crate holds no stage of the compiler, only the `safec` binary that runs
//! them. It presents the parts, each of
//! which is a crate that builds, tests and documents itself with nothing above
//! it, so depend on whichever is the whole of what you need:
//!
//! | for | depend on | here as |
//! |---|---|---|
//! | the compiler, and the `safec` binary | `safec` | this crate |
//! | reading C into the Safety IR, or running the pipeline in process | `safec-c` | [`c`] |
//! | the Safety IR, its analyses, its printer and its interpreter, with no frontend | `safec-ir` | [`ir`] |
//! | the LLVM backend | `safec-llvm` | [`llvm`] |
//!
//! Nothing here is something a part needs: a type a user of one part alone
//! would want lives in that part. One module per part rather than the C part
//! at the root, because the compiler is meant to have more than one frontend.
//! See ADR-0058.

/// Everything that reads C, from the preprocessor to the driver.
pub use safec_c as c;
/// The Safety IR: what a program becomes once the C is gone.
pub use safec_ir as ir;
/// The LLVM backend, which writes the Safety IR out as textual LLVM IR.
pub use safec_llvm as llvm;
