//! The Safety IR: what a program becomes once the C is gone.
//!
//! Depend on this crate alone to build a program in the IR, check it, print it
//! or run it, with no frontend in the graph. The `safec` crate re-exports it as
//! `safec::ir` for a user who wants the whole compiler. See ADR-0058.
//!
//! Everything the safety analyses are written against lives here, and nothing
//! that knows how a program was spelled. There is no lexer, no parser and no
//! syntax tree in this crate, and there cannot be one: `safec-c` depends on this
//! crate, so a dependency the other way is a cycle cargo refuses before it
//! compiles anything. See ADR-0011.
//!
//! That is the whole reason this is a crate rather than four modules.
//! `docs/c-family.md` rests its case for reaching C++ on one arrow, that an
//! analysis must not know which frontend produced the IR, and an arrow nobody
//! can cross by accident is worth more than one everybody agrees with.
//!
//! [`source`] is here rather than in the frontend because a [`source::Span`] is
//! part of the IR: every operation, terminator and trap carries one, and an
//! instruction that cannot say where it came from cannot be reported about.

pub mod analysis;
pub mod cfg;
pub mod dataflow;
pub mod interp;
pub mod ir;
pub mod memory;
pub mod nullability;
pub mod print;
pub mod source;
pub mod target;
