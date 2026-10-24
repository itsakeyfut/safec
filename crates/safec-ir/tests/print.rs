//! The printer, used the way a program that depends on `safec-ir` alone uses
//! it.
//!
//! This crate is a part a user can depend on with nothing above it, and that
//! is a claim about its public API as well as about its manifest: a function
//! has to be something such a user can build and see. The printer's own tests
//! sit inside the crate and can reach what is private; this one cannot. See
//! ADR-0058, and ADR-0011 for the arrow that keeps everything above out.

use safec_ir::ir::{Block, Function, Terminator, TranslationUnit, Ty};
use safec_ir::print::dump_ir;
use safec_ir::source::{SourceMap, Span};
use safec_ir::target::Target;

/// A function named `answer`, of one block that returns, built from the
/// public API and printed under the name its span covers.
///
/// Mutation: make `print::dump_ir` `pub(crate)`. Under
/// `cargo test -p safec-ir` that is `E0603` at its import here, and this
/// target does not build. Across the workspace `safec-c`'s own use of it fails
/// first.
///
/// That nothing above this crate is in its graph is not this test's to say:
/// cargo refuses the cycle before anything compiles, which is ADR-0011.
#[test]
fn a_function_built_by_hand_from_the_public_api_is_printed() {
    let mut sources = SourceMap::new();
    let file = sources.add_virtual("t.c", "int answer(void) { return 42; }\n");
    let name = Span::new(file, 4, 10);

    let mut unit = TranslationUnit::new(
        Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
    );
    let int = unit.push_type(Ty::Int);
    let mut function = Function::new(name, int, []);
    function.push_block(Block {
        elements: Vec::new(),
        terminator: Terminator::Return,
    });
    unit.push_function(function);

    let mut out = String::new();
    dump_ir(&sources, &unit, &mut out);
    assert!(out.contains("Function <t.c>:1:5 \"answer\""), "{out}");
}
