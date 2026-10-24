//! The facade, used the way a program that depends on `safec` alone uses it.
//!
//! Every path here goes through `safec::`, and none names a part's own crate,
//! so this file compiles only while the facade re-exports each part. See
//! ADR-0058.

use safec::c::diagnostics::DiagnosticSink;
use safec::c::lexer::lex;
use safec::c::lowering::lower;
use safec::c::parser::parse;
use safec::c::preprocessor::preprocess;
use safec::c::sema::resolve;
use safec::c::types::check;
use safec::ir::print::dump_ir;
use safec::ir::source::SourceMap;
use safec::ir::target::Target;
use safec::llvm::emit;

/// C goes in through `safec::c`, the unit it becomes is printed through
/// `safec::ir`, and written through `safec::llvm`, with nothing else in hand.
///
/// Mutation: delete any one of the three `pub use` lines in `src/lib.rs`, with
/// its doc comment. The imports through it are `E0432` or `E0433` here,
/// depending on how far each path gets, and this target does not build.
#[test]
fn a_program_reaches_every_part_through_the_facade_alone() {
    let mut sources = SourceMap::new();
    let file = sources.add_virtual("t.c", "int answer(void) { return 42; }\n");
    let mut diagnostics = DiagnosticSink::new();
    let target = Target::from_triple("x86_64-unknown-linux-gnu").expect("a known triple");

    let tokens = lex(file, sources.file(file), &mut diagnostics);
    let tokens = preprocess(file, &tokens, target, &mut sources, &mut diagnostics);
    let mut ast = parse(&tokens, &sources, &mut diagnostics);
    let resolution = resolve(&sources, &ast, &mut diagnostics);
    let types = check(
        &sources,
        &mut ast,
        &resolution,
        target.int(),
        &mut diagnostics,
    );
    let unit = lower(
        &sources,
        &ast,
        &resolution,
        &types,
        target,
        false,
        &mut diagnostics,
    );
    assert!(!diagnostics.has_errors(), "the program did not compile");

    let mut ir = String::new();
    dump_ir(&sources, &unit, &mut ir);
    assert!(ir.contains("answer"), "{ir}");

    let mut module = emit::header(target);
    let refused = emit::functions(&sources, &unit, &mut module);
    assert!(refused.is_empty(), "{refused:?}");
    assert!(module.contains("@answer"), "{module}");
}
