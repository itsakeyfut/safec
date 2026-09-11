//! The LLVM backend: a [`TranslationUnit`] written out as textual LLVM IR.
//!
//! [`TranslationUnit`]: safec_ir::ir::TranslationUnit
//!
//! Text rather than a library. `docs/architecture.md` names `inkwell` as the
//! initial direction and this knowingly goes the other way; ADR-0014 is where
//! that is argued and where the trigger for reversing it is written down.
//!
//! A crate rather than a module of `safec`, because the arrow that must not be
//! crossed is the one `docs/architecture.md` states in a sentence: LLVM must
//! not leak into the frontend and the safety analyses. Nothing here can see the
//! lexer, the parser or the AST, and a line in `Cargo.toml` is what says so.
//! ADR-0011 made the same argument one crate earlier.
//!
//! Nothing here reports. [`emit::Refusal`] is what this crate answers instead,
//! the shape [`interp::Trap`] has and for the same reason: `Diagnostic` lives
//! in `safec`, which this crate cannot see.
//!
//! One module, reached as `safec_llvm::emit::*` rather than re-exported at the
//! root. That is what `safec-ir` does, and it is what a second backend needs: a
//! `safec-wasm` with a `header` and a `functions` of its own would collide at
//! the one call site that wants both.
//!
//! [`interp::Trap`]: safec_ir::interp::Trap

pub mod emit;
