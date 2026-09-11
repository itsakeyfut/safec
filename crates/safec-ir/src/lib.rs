//! The Safety IR: what a program becomes once the C is gone.
//!
//! Everything the safety analyses are written against lives here, and nothing
//! that knows how a program was spelled. There is no lexer, no parser and no
//! syntax tree in this crate, and there cannot be one: `safec` depends on this
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

pub mod cfg;
pub mod interp;
pub mod ir;
pub mod print;
pub mod source;
pub mod target;
