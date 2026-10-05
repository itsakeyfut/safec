use super::*;
use crate::lexer::lex;
use crate::parser::parse;
use crate::sema::resolve;
use crate::types::check;

/// The assignments in a block, in order, with the storage markers dropped.
///
/// Most of these tests are about what a program computes, and a marker is
/// not that. The ones that are about markers read `block.elements`
/// directly, which is the only way to see both.
fn assigns(block: &Block) -> Vec<&Operation> {
    block
        .elements
        .iter()
        .filter_map(|element| match element {
            Element::Assign(operation) => Some(operation),
            Element::Evaluate { .. }
            | Element::Sequenced { .. }
            | Element::ArgumentsEvaluated { .. }
            | Element::StorageLive { .. }
            | Element::StorageDead { .. } => None,
        })
        .collect()
}

/// One program, lowered, and whatever was said about it on the way.
struct Lowered {
    unit: TranslationUnit,
    diagnostics: DiagnosticSink,
    sources: SourceMap,
}

/// Lower a program the way the driver would, gate and all.
///
/// The gate matters: `lower` is written for a tree nothing reported about,
/// so a test that fed it a broken one would be testing a case the compiler
/// does not produce. What is asserted below about a refusal is therefore
/// always about a program the frontend accepted in silence.
fn lowered(text: &str) -> Lowered {
    compiled(text, true)
}

/// The same, for a program the frontend has already reported about.
///
/// The gate `lowered` applies is right for everything else and wrong for
/// exactly one case: a constant `types.rs` could not read has no type, so
/// the frontend reports and the driver lowers anyway. What is asserted
/// through here is that this stage adds nothing to what was already said,
/// which is a claim about a tree that did not check.
fn lowered_after_a_report(text: &str) -> Lowered {
    compiled(text, false)
}

/// Both of the above. `must_check` is the gate, and it is a parameter
/// rather than two copies of the pipeline.
fn compiled(text: &str, must_check: bool) -> Lowered {
    let mut sources = SourceMap::new();
    let file = sources.add_virtual("t.c", text);
    let mut diagnostics = DiagnosticSink::new();

    let tokens = lex(file, sources.file(file), &mut diagnostics);
    let mut ast = parse(file, &tokens, &mut diagnostics);
    assert!(!diagnostics.has_errors(), "the input did not parse");

    let resolution = resolve(&sources, &ast, &mut diagnostics);
    // A target this test suite does not otherwise care about: what a
    // program lowers to does not turn on the machine, and the one test
    // that is about the machine names its own. It reaches the type check
    // as well now, because the range of `int` is what says whether a
    // constant has a type.
    let target = Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple");
    let types = check(
        &sources,
        &mut ast,
        &resolution,
        target.int(),
        &mut diagnostics,
    );
    assert!(
        !must_check || !diagnostics.has_errors(),
        "the input did not check"
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
    Lowered {
        unit,
        diagnostics,
        sources,
    }
}

/// The codes of everything reported, in order.
fn codes(lowered: &Lowered) -> Vec<String> {
    lowered
        .diagnostics
        .diagnostics()
        .iter()
        .filter_map(|diagnostic| diagnostic.code().map(|code| code.as_str().to_owned()))
        .collect()
}

/// The function that name belongs to.
fn function<'a>(lowered: &'a Lowered, name: &str) -> &'a Function {
    let id = lowered
        .unit
        .functions()
        .find(|id| lowered.sources.snippet(lowered.unit.function(*id).name) == name)
        .expect("a function of that name");
    lowered.unit.function(id)
}

/// Which blocks each block can reach, in order.
fn edges(function: &Function) -> Vec<Vec<usize>> {
    function
        .blocks()
        .map(|block| {
            let mut successors = Vec::new();
            block.terminator.successors(&mut successors);
            successors.iter().map(|block| block.index()).collect()
        })
        .collect()
}

/// Every storage marker in a function, as `(kind, local)` in order.
fn markers(function: &Function) -> Vec<(&'static str, usize)> {
    function
        .blocks()
        .flat_map(|block| block.elements.iter())
        .filter_map(|element| match element {
            Element::Assign(_)
            | Element::Evaluate { .. }
            | Element::Sequenced { .. }
            | Element::ArgumentsEvaluated { .. } => None,
            Element::StorageLive { local, origin: _ }
            | Element::StorageDead { local, origin: _ } => Some((element.name(), local.index())),
        })
        .collect()
}

/// Where every element sits, as a kind per element, function-wide.
///
/// Kinds and not spans, because what these tests are about is what falls
/// between what. A span would say the same thing in a form that moves
/// whenever a word in the program does.
fn kinds(function: &Function) -> Vec<&'static str> {
    function
        .blocks()
        .flat_map(|block| block.elements.iter().map(Element::name))
        .collect()
}

/// A comma at the top of a full expression is a sequence point.
///
/// C17 6.5.17 p2 puts one between its operands, and nothing encloses this
/// one, so the free on the left happens before the write on the right and
/// the IR says so. A check reading it may prove a use after free here.
///
/// Mutation: make `sequences` answer `false` for `Expr::Comma`. The marker
/// moves to the end of the statement, after the write, and this fails on
/// the order.
#[test]
fn a_comma_at_the_top_of_a_full_expression_is_a_sequence_point() {
    let lowered = lowered(
        "void *malloc(int n);\nvoid free(void *p);\n\nint f(void) {\n    int *p = malloc(4);\n    free(p), *p = 42;\n    return 0;\n}\n",
    );
    assert_eq!(codes(&lowered), Vec::<String>::new());

    let f = function(&lowered, "f");
    let blocks: Vec<_> = f.blocks().collect();

    // The free ends its block, so the comma's own sequence point is the
    // first thing in the next one and the write follows it.
    let Terminator::Call { .. } = &blocks[1].terminator else {
        panic!("{:?}", blocks[1].terminator);
    };
    assert_eq!(blocks[2].elements[0].name(), "Sequenced");
}

/// A comma inside an operand C leaves unsequenced is not one.
///
/// `*p + (free(p), 0)` has a comma, and 6.5.17 p2 does sequence the free
/// before the `0`. It does not sequence it against the read of `*p`, which
/// is the other operand of the `+` and which 6.5 p3 leaves unordered
/// against both. Recording it would tell a check the free happens first,
/// and the check would prove a use after free in a program C defines under
/// one of the orders it allows.
///
/// **This is the direction that matters.** The mutation below does not
/// make the compiler quieter; it makes it certain about something C has
/// not decided.
///
/// Mutation: drop the `if self.top_level` guard in `Lowering::discard`.
/// A `Sequenced` appears between the call and the addition that reads
/// `*p`, and this fails on the first kind in that block.
#[test]
fn a_comma_inside_an_unsequenced_operand_is_not_one() {
    let lowered = lowered(
        "void *malloc(int n);\nvoid free(void *p);\n\nint f(void) {\n    int *p = malloc(4);\n    int x = *p + (free(p), 0);\n    return x;\n}\n",
    );
    assert_eq!(codes(&lowered), Vec::<String>::new());

    let f = function(&lowered, "f");
    let blocks: Vec<_> = f.blocks().collect();

    let Terminator::Call { .. } = &blocks[1].terminator else {
        panic!("{:?}", blocks[1].terminator);
    };
    // The addition that reads `*p`, with nothing between it and the free.
    assert_eq!(blocks[2].elements[0].name(), "Operation");
    // And the statement's own end is still recorded, after both.
    assert!(kinds(f).contains(&"Sequenced"), "{:?}", kinds(f));
}

/// Two programs that differ in one pair of braces are two IRs.
///
/// This is the whole point of the markers. Before them the two below were
/// byte for byte the same artifact, so an analysis handed either had the
/// same material and had to answer both the same way: silent about the
/// dangling one, or wrong about the other. ADR-0012 has the argument.
///
/// Mutation: delete the `StorageDead` loop from the `Compound` arm. The two
/// differ only in `StorageLive` then, and the assertion that the flat
/// program has no markers at all still holds, so this fails on the first
/// comparison. Mutation: delete both loops, and the two are equal again.
#[test]
fn the_same_program_in_a_nested_scope_is_not_the_same_ir() {
    let nested = lowered("int g(void) { int *p; { int x; x = 42; p = &x; } return *p; }\n");
    let flat = lowered("int g(void) { int *p; int x; x = 42; p = &x; return *p; }\n");

    let (nested, flat) = (function(&nested, "g"), function(&flat, "g"));

    // The same locals in the same order, so the difference is not that one
    // program declares something the other does not.
    assert_eq!(nested.locals().len(), flat.locals().len());

    let marked = markers(nested);
    let [(opens, live), (closes, dead)] = marked[..] else {
        panic!("one pair, and nothing else: {marked:?}");
    };
    assert_eq!((opens, closes), ("StorageLive", "StorageDead"));
    assert_eq!(live, dead, "one local, opened and closed");

    assert_eq!(markers(flat), [], "the flat program has nothing to say");
}

/// A local the function itself declares gets no marker.
///
/// Its storage is the frame's, which every consumer already models: the
/// interpreter pops the frame and a pointer into a returned one is caught
/// by its generation. ADR-0012 argues why that is enough, and the cost of
/// marking them anyway would be a pair per local in every artifact.
///
/// Mutation: drop the `self.scopes.len() > 1` test in the `Declaration`
/// arm. Every local gets a pair and this fails.
#[test]
fn a_local_the_function_declares_has_no_marker() {
    let lowered = lowered("int f(int n) { int a; int b; a = n; b = a; return b; }\n");

    assert_eq!(markers(function(&lowered, "f")), []);
}

/// A scope that a `return` leaves ends no storage.
///
/// The frame is going, so there is nothing for a marker to say, and an
/// element written after a block has ended would be written into the next
/// block instead.
///
/// Mutation: emit the `StorageDead` loop whether or not `builder.reachable`
/// says the end is reachable. A `StorageDead` appears and this fails.
#[test]
fn a_scope_a_return_leaves_ends_no_storage() {
    let lowered = lowered("int f(int n) { if (n) { int x; x = 1; return x; } return 0; }\n");

    let marked = markers(function(&lowered, "f"));
    let [(opens, _)] = marked[..] else {
        panic!("the scope opens and nothing closes it: {marked:?}");
    };
    assert_eq!(opens, "StorageLive");
}

/// A local in a loop body gets its storage back on each iteration.
///
/// C17 6.2.4 p6: an automatic object's lifetime "extends from entry into
/// the block with which it is associated until execution of that block ends
/// in any way". Each iteration enters and ends the body, so each iteration
/// is a fresh lifetime. Without the `StorageLive` the second iteration
/// would be writing into storage the IR says is gone.
///
/// Mutation: emit `StorageDead` and no `StorageLive`. This fails, and the
/// interpreter stops on the second iteration of any loop that declares
/// anything.
#[test]
fn a_local_in_a_loop_body_gets_its_storage_back() {
    let lowered = lowered(
        "int f(void) {\n    int n; int s;\n    n = 0; s = 0;\n    while (n < 3) { int x; x = n; s = s + x; n = n + 1; }\n    return s;\n}\n",
    );
    let f = function(&lowered, "f");

    let marked = markers(f);
    let [(opens, live), (closes, dead)] = marked[..] else {
        panic!("one pair, and nothing else: {marked:?}");
    };
    assert_eq!((opens, closes), ("StorageLive", "StorageDead"));
    assert_eq!(live, dead, "one local, opened and closed");

    // Both are in the body rather than around the loop, which is what makes
    // the second iteration a fresh lifetime rather than a read of storage
    // the first one ended.
    let body: Vec<usize> = f
        .blocks()
        .enumerate()
        .filter(|(_, block)| {
            block.elements.iter().any(|element| {
                matches!(
                    element,
                    Element::StorageLive { .. } | Element::StorageDead { .. }
                )
            })
        })
        .map(|(index, _)| index)
        .collect();
    assert_eq!(body.len(), 1, "both markers are in one block");
}

/// `main` is given its zero before its body, and nothing else is.
///
/// C17 5.1.2.2.3 p1: reaching the `}` that terminates `main` returns zero,
/// unconditionally. 6.9.1 p12 leaves falling off the end of any other
/// value-returning function undefined, so only `main` gets one.
///
/// Written at the top rather than beside the fall-off, because a `goto` can
/// reach that `}` from anywhere, and a `return` of its own overwrites it.
///
/// **The interpreter cannot show this.** `interp.rs` answers zero for a top
/// frame whose return place nothing wrote, which is the same rule read from
/// the other end, so a program run through it gave the right answer while
/// the IR said nothing. It took a backend, whose output a real machine runs,
/// for that to be a wrong answer: measured, `int main() { fill(7); }` built
/// through `--emit llvm-ir` and `clang -O2` exited 108 where `clang`
/// compiling the same C exited 0.
///
/// Mutation: drop the zero. This fails on the first element, and seven
/// corpus expectations fail with it.
#[test]
fn main_is_given_its_zero_and_nothing_else_is() {
    let lowered = lowered("int main() { } int other() { }");

    let main = function(&lowered, "main");
    let entry = main.blocks().next().expect("a first block");
    let [zero] = assigns(entry)[..] else {
        panic!("one operation");
    };
    assert_eq!(zero.place, Place::local(main.return_place()));
    assert_eq!(zero.value, Rvalue::Use(Operand::Constant(0)));
    assert!(matches!(zero.origin, Origin::Generated(_)));

    let other = function(&lowered, "other");
    let entry = other.blocks().next().expect("a first block");
    assert!(assigns(entry).is_empty());
}

/// The MVP program of `docs/roadmap.md` lowers, which is the first clause
/// of Phase 2's Done-when.
///
/// Mutation: have the `Call` arm write its result into the return place
/// rather than into a temporary. `main` stops reading the call's answer
/// back and this fails.
///
/// Mutation: lower `a + b` as `BinOp::Sub`. The assertion on `add`'s
/// operation fails.
#[test]
fn the_mvp_lowers() {
    let lowered = lowered(
        "int add(int a, int b) {\n    return a + b;\n}\n\nint main() {\n    return add(1, 2);\n}\n",
    );
    assert_eq!(codes(&lowered), Vec::<String>::new());

    let add = function(&lowered, "add");
    assert!(add.is_defined());
    let [entry] = add.blocks().collect::<Vec<_>>()[..] else {
        panic!("one block");
    };
    let [sum, returned] = assigns(entry)[..] else {
        panic!("{:?}", entry.elements);
    };
    let [a, b] = add.parameters().collect::<Vec<_>>()[..] else {
        panic!("two parameters");
    };
    assert_eq!(
        sum.value,
        Rvalue::Binary {
            op: BinOp::Add,
            lhs: Operand::Copy(Place::local(a)),
            rhs: Operand::Copy(Place::local(b)),
        }
    );
    assert_eq!(returned.place, Place::local(add.return_place()));
    assert_eq!(
        returned.value,
        Rvalue::Use(Operand::Copy(sum.place.clone()))
    );
    assert_eq!(entry.terminator, Terminator::Return);

    // A call ends a block, so `return add(1, 2);` is two of them.
    let main = function(&lowered, "main");
    let [entry, after] = main.blocks().collect::<Vec<_>>()[..] else {
        panic!("two blocks");
    };
    // One operation before the call, and it is `main`'s zero: C17
    // 5.1.2.2.3 p1 puts it there whatever the body does.
    let [zero] = assigns(entry)[..] else {
        panic!("one operation before the call");
    };
    assert_eq!(zero.place, Place::local(main.return_place()));
    assert_eq!(zero.value, Rvalue::Use(Operand::Constant(0)));
    assert!(matches!(zero.origin, Origin::Generated(_)));

    let Terminator::Call {
        callee,
        arguments,
        destination,
        then,
        origin,
    } = &entry.terminator
    else {
        panic!("{:?}", entry.terminator);
    };
    assert_eq!(
        lowered.sources.snippet(lowered.unit.function(*callee).name),
        "add"
    );
    assert_eq!(arguments, &[Operand::Constant(1), Operand::Constant(2)]);
    assert_eq!(then.map(|then| then.index()), Some(1));
    assert_eq!(lowered.sources.snippet(origin.span()), "add(1, 2)");

    let destination = destination.clone().expect("somewhere to put the answer");
    let [returned] = assigns(after)[..] else {
        panic!("{:?}", after.elements);
    };
    assert_eq!(returned.place, Place::local(main.return_place()));
    assert_eq!(returned.value, Rvalue::Use(Operand::Copy(destination)));
    assert_eq!(after.terminator, Terminator::Return);
}

/// A `while` is blocks and edges, and the body comes back to the condition.
///
/// Mutation: end the body with `Goto` the block after the loop rather than
/// the header. The back edge goes and this fails.
///
/// Mutation: evaluate the condition before the header rather than in it.
/// The comparison moves into the block before the loop, the condition is
/// asked once however many turns the loop takes, and this fails.
#[test]
fn a_while_loop_comes_back_to_its_condition() {
    // `n > 0` rather than `n`, because the comparison is an operation and
    // which block holds it is the whole point: a condition evaluated once,
    // before the loop, is a different program that the shape of the edges
    // alone cannot tell apart.
    let lowered = lowered(
        "int f(int n) {\n    while (n > 0) {\n        n = n - 1;\n    }\n    return n;\n}\n",
    );
    assert_eq!(codes(&lowered), Vec::<String>::new());

    let f = function(&lowered, "f");
    let edges = edges(f);
    // The entry goes to the header, the header branches, the body comes
    // back, and the exit returns.
    assert_eq!(edges, vec![vec![1], vec![2, 3], vec![1], vec![]]);

    let blocks: Vec<_> = f.blocks().collect();
    assert!(matches!(blocks[1].terminator, Terminator::Branch { .. }));

    // The same edge again, said as the body's terminator rather than as an
    // index, so that the two ways of asking agree.
    let mut header = Vec::new();
    blocks[0].terminator.successors(&mut header);
    assert_eq!(blocks[2].terminator, Terminator::Goto(header[0]));

    // The condition is asked in the header, which is where the back edge
    // arrives, so the comparison is written there and not before the loop.
    assert!(assigns(blocks[0]).is_empty());
    let [compared] = assigns(blocks[1])[..] else {
        panic!("{:?}", blocks[1].elements);
    };
    assert!(matches!(
        compared.value,
        Rvalue::Binary { op: BinOp::Gt, .. }
    ));
}

/// An `if` with no `else` still has an edge that skips the body.
///
/// Mutation: give the arm with no `else` no block, branching straight to
/// the join. The skipping edge stops being a block of its own and the
/// shape asserted here fails.
#[test]
fn an_if_with_no_else_still_joins() {
    let lowered =
        lowered("int f(int n) {\n    if (n) {\n        n = 0;\n    }\n    return n;\n}\n");
    assert_eq!(codes(&lowered), Vec::<String>::new());

    let f = function(&lowered, "f");
    // The entry branches to the body and to the empty arm, and both reach
    // the join, which returns.
    assert_eq!(edges(f), vec![vec![1, 2], vec![3], vec![3], vec![]]);
}

/// `&&` is a branch, because C says the right operand may not be evaluated.
///
/// C17 6.5.13 p4. Mutation: lower it as an `ir::BinOp`; there is no variant
/// to lower it to, so that mutation does not compile, which is the guard
/// `ir::BinOp`'s doc comment claims.
///
/// Mutation: write the left operand into the answer after the branch rather
/// than before it. The answer of `0 && x` stops being written at all and
/// the assertion on the first block's operations fails.
///
/// Mutation: write the operand itself into the answer rather than whether
/// it is non-zero. `3 && 5` becomes five where C17 6.5.13 p3 says one, and
/// both assertions on the comparison fail.
///
/// Mutation: give the branch the right operand's span, or the whole binary
/// expression's. The last assertion fails and nothing else in the suite
/// does, which is why it is here: this is the one branch the frontend
/// builds whose condition is a temporary, so no diagnostic will ever point
/// at its span and no corpus artifact holds a short circuit.
#[test]
fn a_short_circuit_is_a_branch_and_answers_one_or_zero() {
    let lowered = lowered("int f(int a, int b) {\n    return a && b;\n}\n");
    assert_eq!(codes(&lowered), Vec::<String>::new());

    let f = function(&lowered, "f");
    // The entry branches on the left operand; one arm evaluates the right
    // and both arrive at the join.
    assert_eq!(edges(f), vec![vec![2, 1], vec![], vec![1]]);

    let blocks: Vec<_> = f.blocks().collect();
    let [a, b] = f.parameters().collect::<Vec<_>>()[..] else {
        panic!("two parameters");
    };

    // Each arm writes whether its operand is non-zero, because 6.5.13 p3
    // makes the answer 1 or 0 rather than whatever decided it.
    let [decided] = assigns(blocks[0])[..] else {
        panic!("{:?}", blocks[0].elements);
    };
    assert_eq!(
        decided.value,
        Rvalue::Binary {
            op: BinOp::Ne,
            lhs: Operand::Copy(Place::local(a)),
            rhs: Operand::Constant(0),
        }
    );

    let [answered] = assigns(blocks[2])[..] else {
        panic!("{:?}", blocks[2].elements);
    };
    assert_eq!(answered.place, decided.place);
    assert_eq!(
        answered.value,
        Rvalue::Binary {
            op: BinOp::Ne,
            lhs: Operand::Copy(Place::local(b)),
            rhs: Operand::Constant(0),
        }
    );

    // What decides is the left operand, so that is the expression the
    // branch names. Written out as the text rather than compared against a
    // span this test computed, because a test that asks the lowering where
    // it put something agrees with it however wrong both are.
    let Terminator::Branch { origin, .. } = blocks[0].terminator else {
        panic!("{:?}", blocks[0].terminator);
    };
    assert_eq!(lowered.sources.snippet(origin.span()), "a");
}

/// A conditional operator answers what its arm is worth, and not a truth
/// value.
///
/// C17 6.5.15 p4: the value is the operand's, which is what separates it
/// from `&&` and `||` a few lines above in this module.
///
/// Mutation: normalise a `?:` arm the way a short circuit is normalised.
/// `a ? b : c` starts answering 1 or 0 and this fails.
#[test]
fn a_conditional_answers_the_arm_it_took() {
    let lowered = lowered("int f(int a, int b, int c) {\n    return a ? b : c;\n}\n");
    assert_eq!(codes(&lowered), Vec::<String>::new());

    let f = function(&lowered, "f");
    let [_, b, c] = f.parameters().collect::<Vec<_>>()[..] else {
        panic!("three parameters");
    };
    let blocks: Vec<_> = f.blocks().collect();

    // Block 1 is the join, and the arms are the two blocks the branch
    // names, in the order they were reserved.
    assert_eq!(edges(f), vec![vec![2, 3], vec![], vec![1], vec![1]]);
    let [taken] = assigns(blocks[2])[..] else {
        panic!("{:?}", blocks[2].elements);
    };
    let [skipped] = assigns(blocks[3])[..] else {
        panic!("{:?}", blocks[3].elements);
    };
    assert_eq!(taken.value, Rvalue::Use(Operand::Copy(Place::local(b))));
    assert_eq!(skipped.value, Rvalue::Use(Operand::Copy(Place::local(c))));
    assert_eq!(taken.place, skipped.place);
}

/// A function can call itself, which needs its id before its body.
///
/// Mutation: give a function its id only once its body is built. `f` has
/// no id to call and this fails.
#[test]
fn a_recursive_call_names_the_function_it_is_in() {
    let lowered = lowered("int f(int n) {\n    return f(n);\n}\n");
    assert_eq!(codes(&lowered), Vec::<String>::new());

    let f = function(&lowered, "f");
    let [entry, _] = f.blocks().collect::<Vec<_>>()[..] else {
        panic!("two blocks");
    };
    let Terminator::Call { callee, .. } = &entry.terminator else {
        panic!("{:?}", entry.terminator);
    };
    assert_eq!(
        lowered.sources.snippet(lowered.unit.function(*callee).name),
        "f"
    );
}

/// A function whose body is elsewhere can still be called.
///
/// Mutation: skip `Item::Declaration` when handing out ids. The call has no
/// callee, `SC0304` is reported, and this fails on both counts.
#[test]
fn a_declared_function_can_be_called() {
    let lowered = lowered("int g(int n);\n\nint f(int n) {\n    return g(n);\n}\n");
    assert_eq!(codes(&lowered), Vec::<String>::new());

    let g = function(&lowered, "g");
    assert!(!g.is_defined());
    assert!(function(&lowered, "f").is_defined());
}

/// An expression the frontend could not type is reported, and its function
/// keeps no body.
///
/// `x[0]` where `x` is an `int` is the case that reaches here: `types.rs`
/// answers `None` for it and reports nothing, so nothing before this stage
/// says the program has a hole in it.
///
/// Mutation: lower an untyped expression as `Ty::Int`. No code is reported
/// and this fails. Mutation: report and lower the rest of the function
/// anyway. `is_defined` starts answering true and this fails.
#[test]
fn an_expression_with_no_type_is_reported_and_lowers_nothing() {
    let lowered = lowered("int f(void) {\n    int x;\n    return x[0];\n}\n");
    assert_eq!(codes(&lowered), ["SC0304"]);
    assert!(!function(&lowered, "f").is_defined());
}

/// A constant reaches the IR as the number `types.rs` read, whatever base
/// it was written in.
///
/// This stage no longer reads a spelling at all, so what is asserted is
/// that it asks and does not re-derive: `010` is eight here because
/// 6.4.4.1 p2 makes it eight there.
///
/// Mutation: have the `Expr::Number` arm push `Operand::Constant(0)`.
/// This fails, and so do `the_mvp_lowers` and three others, which is what
/// says the operand reaches the IR rather than only the test.
#[test]
fn a_constant_reaches_the_ir_as_the_value_the_frontend_read() {
    for (spelling, value) in [("42", 42), ("0", 0), ("0x10", 16), ("010", 8)] {
        let lowered = lowered(&format!("int f(void) {{\n    return {spelling};\n}}\n"));
        assert_eq!(codes(&lowered), Vec::<String>::new(), "{spelling}");

        let f = function(&lowered, "f");
        let [block] = f.blocks().collect::<Vec<_>>()[..] else {
            panic!("one block");
        };
        let [returned] = assigns(block)[..] else {
            panic!("{:?}", block.elements);
        };
        assert_eq!(
            returned.value,
            Rvalue::Use(Operand::Constant(value)),
            "{spelling}"
        );
    }
}

/// A constant the frontend could not read is said once and lowered not at
/// all.
///
/// Two halves, and the first is the one that is easy to lose. `types.rs`
/// has already pointed a caret at the spelling, so a report here would be
/// a second one at the same place, and for `123abc` its note would say the
/// fault is this compiler's when it is the program's.
///
/// The second half, that `f` is left a declaration, is held by `typed`
/// rather than by the `Expr::Number` arm: the arm is never reached for
/// these programs, which its own comment says and which was measured. So
/// the mutation below is the only one that reaches either half from here.
///
/// Mutation: let `typed` report for an `Expr::Number` too. Two codes and
/// this fails, and so do the corpus cases
/// `a_constant_no_integer_type_here_can_hold`,
/// `a_spelling_that_is_not_a_constant` and
/// `a_suffixed_constant_has_no_type_here`, whose blessed stderr picks up
/// the second diagnostic. Three guards rather than one, which is worth
/// knowing because this test is the only one of the four that also says
/// the function is left a declaration.
#[test]
fn a_constant_the_frontend_could_not_read_is_reported_once_and_lowers_nothing() {
    for (spelling, code) in [
        ("1.5", "SC0305"),
        ("2147483648", "SC0305"),
        ("9999999999999999999999999999999999999999", "SC0305"),
        ("123abc", "SC0106"),
        ("09", "SC0106"),
    ] {
        let lowered =
            lowered_after_a_report(&format!("int f(void) {{\n    return {spelling};\n}}\n"));
        assert_eq!(codes(&lowered), [code], "{spelling}");
        assert!(!function(&lowered, "f").is_defined(), "{spelling}");
    }
}

/// A type the IR cannot hold is reported at the declaration that wrote it.
///
/// The IR has `int`, `char`, `void` and pointers, and nothing has asked it
/// for more. Lowering an array as a pointer would tell the memory analysis
/// that one object is another.
///
/// Mutation: lower `Type::Array` as a pointer to its element. Nothing is
/// reported and this fails.
#[test]
fn a_type_the_ir_cannot_hold_is_reported() {
    let lowered = lowered("int f(void) {\n    int a[3];\n    return 0;\n}\n");
    assert_eq!(codes(&lowered), ["SC0304"]);
    assert!(!function(&lowered, "f").is_defined());
}

/// A name that is not a local is reported where it is used.
///
/// Every place the IR can name starts at a local, so an object at file
/// scope has nothing to be until #74.
///
/// Mutation: give a use with no local the return place instead. Nothing is
/// reported, `f` gains a body that writes to the wrong place, and this
/// fails.
#[test]
fn a_name_the_ir_cannot_reach_is_reported() {
    let lowered = lowered("int g;\n\nint f(void) {\n    return g;\n}\n");
    assert_eq!(codes(&lowered), ["SC0304"]);
    assert!(!function(&lowered, "f").is_defined());
}

/// One function's refusal is not another's.
///
/// Mutation: stop lowering at the first function that fails. `good` loses
/// its body and this fails.
#[test]
fn a_function_that_cannot_be_lowered_does_not_take_the_others_with_it() {
    let lowered = lowered(
        "int bad(void) {\n    int a[3];\n    return 0;\n}\n\nint good(void) {\n    return 1;\n}\n",
    );
    assert_eq!(codes(&lowered), ["SC0304"]);
    assert!(!function(&lowered, "bad").is_defined());
    assert!(function(&lowered, "good").is_defined());
}

/// What a statement after a `return` becomes.
///
/// C allows it and the parser reads it, so the lowering has to put it
/// somewhere: it opens a block nothing jumps to, which is what unreachable
/// code is.
///
/// Mutation: keep the block open after a terminator. The statement after
/// the `return` is written into a block that was already filled, which is
/// `fill_block`'s panic.
#[test]
fn a_statement_after_a_return_is_a_block_nothing_reaches() {
    let lowered = lowered("int f(int n) {\n    return n;\n    n = 1;\n}\n");
    assert_eq!(codes(&lowered), Vec::<String>::new());

    let f = function(&lowered, "f");
    let edges = edges(f);
    assert_eq!(edges.len(), 2);
    assert!(edges.iter().all(|successors| successors.is_empty()));
    assert!(
        !edges
            .iter()
            .skip(1)
            .any(|successors| successors.contains(&1))
    );
}

/// Every operation says where in the source it came from.
///
/// The program touches every place an operation is made: an assignment, a
/// binary operator, a unary one, an address, an increment, a short circuit,
/// a conditional, a call and a return. A shorter one would leave most of
/// those unwatched, which is what this test did before somebody mutated the
/// arm it was not watching and nothing failed.
///
/// Mutation: give any one of them `Origin::Generated` instead. The
/// assertion that the source wrote it fails, and with it the promise that a
/// diagnostic about this operation can quote what the user typed.
#[test]
fn every_operation_says_the_source_wrote_it() {
    let lowered = lowered(
        "int g(int *q);\n\nint f(int n) {\n    int *p;\n    p = &n;\n    n = -n;\n    n++;\n    n = n && 1;\n    n = n ? 2 : 3;\n    n = n + g(p);\n    return n;\n}\n",
    );
    assert_eq!(codes(&lowered), Vec::<String>::new());

    let f = function(&lowered, "f");
    let mut seen = 0;
    for block in f.blocks() {
        for operation in assigns(block) {
            assert!(
                matches!(operation.origin, Origin::Written(_)),
                "{:?}",
                operation.origin
            );
            assert!(!lowered.sources.snippet(operation.origin.span()).is_empty());
            seen += 1;
        }

        // A call is a terminator and carries the same field, so it answers
        // the same question the operations do.
        if let Terminator::Call { origin, .. } = &block.terminator {
            assert!(matches!(origin, Origin::Written(_)), "{origin:?}");
            seen += 1;
        }
    }

    assert!(seen > 15, "the program makes more than fifteen of them");
}

/// The one operation a function's only block ends up holding before it
/// returns, for a program of the shape the operator tests below use.
fn only_operation(lowered: &Lowered, name: &str) -> Operation {
    let function = function(lowered, name);
    let blocks: Vec<_> = function.blocks().collect();
    let [computed, _returned] = assigns(blocks[0])[..] else {
        panic!("{:?}", blocks[0].elements);
    };
    computed.clone()
}

/// Every binary operator becomes the one it means.
///
/// The expected side is written out rather than taken from `binary`,
/// because a table built the way the code builds one compares the code
/// with itself. `&&` and `||` are absent because they are
/// branches, which the test above holds.
///
/// Mutation: map any one of these to another variant, `Mul` to `Div` say.
/// This fails, naming the operator that moved.
#[test]
fn every_binary_operator_lowers_to_the_one_it_means() {
    for (spelling, expected) in [
        ("*", BinOp::Mul),
        ("/", BinOp::Div),
        ("%", BinOp::Rem),
        ("+", BinOp::Add),
        ("-", BinOp::Sub),
        ("<<", BinOp::Shl),
        (">>", BinOp::Shr),
        ("<", BinOp::Lt),
        (">", BinOp::Gt),
        ("<=", BinOp::Le),
        (">=", BinOp::Ge),
        ("==", BinOp::Eq),
        ("!=", BinOp::Ne),
        ("&", BinOp::BitAnd),
        ("^", BinOp::BitXor),
        ("|", BinOp::BitOr),
    ] {
        let lowered = lowered(&format!(
            "int f(int a, int b) {{\n    return a {spelling} b;\n}}\n"
        ));
        assert_eq!(codes(&lowered), Vec::<String>::new(), "{spelling}");

        let f = function(&lowered, "f");
        let [a, b] = f.parameters().collect::<Vec<_>>()[..] else {
            panic!("two parameters");
        };
        assert_eq!(
            only_operation(&lowered, "f").value,
            Rvalue::Binary {
                op: expected,
                lhs: Operand::Copy(Place::local(a)),
                rhs: Operand::Copy(Place::local(b)),
            },
            "{spelling}"
        );
    }
}

/// Every unary operator becomes the one it means, and `+` becomes nothing.
///
/// C17 6.5.3.3 p2 makes unary `+` the value of its operand, so there is no
/// operation for it to become.
///
/// Mutation: map `-` to `UnOp::Not`, or give `+` an operation of its own.
/// This fails on the operator that moved.
#[test]
fn every_unary_operator_lowers_to_the_one_it_means() {
    for (spelling, expected) in [
        ("-", Some(UnOp::Neg)),
        ("!", Some(UnOp::Not)),
        ("~", Some(UnOp::BitNot)),
        ("+", None),
    ] {
        let lowered = lowered(&format!("int f(int a) {{\n    return {spelling}a;\n}}\n"));
        assert_eq!(codes(&lowered), Vec::<String>::new(), "{spelling}");

        let f = function(&lowered, "f");
        let [a] = f.parameters().collect::<Vec<_>>()[..] else {
            panic!("one parameter");
        };
        let blocks: Vec<_> = f.blocks().collect();

        match expected {
            Some(op) => {
                let [applied, _] = assigns(blocks[0])[..] else {
                    panic!("{:?}", blocks[0].elements);
                };
                assert_eq!(
                    applied.value,
                    Rvalue::Unary {
                        op,
                        operand: Operand::Copy(Place::local(a)),
                    },
                    "{spelling}"
                );
            }
            None => {
                let [returned] = assigns(blocks[0])[..] else {
                    panic!("{:?}", blocks[0].elements);
                };
                assert_eq!(
                    returned.value,
                    Rvalue::Use(Operand::Copy(Place::local(a))),
                    "{spelling}"
                );
            }
        }
    }
}

/// What a pointer reaches is a place, and it is not the pointer's place.
///
/// The memory axis of `docs/safety-model.md` turns on this: `p` and `*p`
/// are two places, and an analysis told they are one would say a pointer is
/// live when what it points at is not.
///
/// Mutation: drop the `Deref` that `finish_place` pushes. Reading and
/// writing both land on the pointer itself and this fails.
#[test]
fn a_pointer_is_read_and_written_through_a_deref() {
    let lowered = lowered("int f(int *p) {\n    *p = 1;\n    return *p;\n}\n");
    assert_eq!(codes(&lowered), Vec::<String>::new());

    let f = function(&lowered, "f");
    let [p] = f.parameters().collect::<Vec<_>>()[..] else {
        panic!("one parameter");
    };
    let pointee = Place {
        local: p,
        projection: vec![Projection::Deref],
    };
    let blocks: Vec<_> = f.blocks().collect();
    let [written, held, returned] = assigns(blocks[0])[..] else {
        panic!("{:?}", blocks[0].elements);
    };

    assert_eq!(written.place, pointee);
    assert_eq!(written.value, Rvalue::Use(Operand::Constant(1)));
    assert_eq!(held.value, Rvalue::Use(Operand::Copy(pointee.clone())));
    assert_eq!(returned.place, Place::local(f.return_place()));
    // Reading through the pointer reads what it points at, which is the
    // assertion a lowering that dropped the projection would fail.
    assert_eq!(returned.value, Rvalue::Use(Operand::Copy(pointee)));
    assert_ne!(written.place, Place::local(p));
}

/// A pointer's type is a pointer, however many times over.
///
/// Mutation: have `ty` stop wrapping, so every pointer becomes what it
/// points at. The three parameters below become one type and this fails.
#[test]
fn a_pointer_type_is_not_the_type_it_points_at() {
    let lowered = lowered("int f(int **pp, int *p, char c) {\n    return 0;\n}\n");
    assert_eq!(codes(&lowered), Vec::<String>::new());

    let f = function(&lowered, "f");
    let [pp, p, c] = f.parameters().collect::<Vec<_>>()[..] else {
        panic!("three parameters");
    };
    let int = f.local(f.return_place());

    assert_eq!(lowered.unit.ty(f.local(p)), Ty::Pointer(int));
    assert_eq!(lowered.unit.ty(f.local(pp)), Ty::Pointer(f.local(p)));
    assert_eq!(lowered.unit.ty(f.local(c)), Ty::Char);
    assert_ne!(f.local(c), int);
}

/// Taking an address is the one operation that turns a place into a value.
///
/// Mutation: lower `&x` as a copy of `x`. The lifetime analysis loses the
/// only shape it starts from, and this fails.
#[test]
fn an_address_is_taken_of_the_place_it_names() {
    let lowered = lowered("int f(int n) {\n    int *p;\n    p = &n;\n    return 0;\n}\n");
    assert_eq!(codes(&lowered), Vec::<String>::new());

    let f = function(&lowered, "f");
    let [n] = f.parameters().collect::<Vec<_>>()[..] else {
        panic!("one parameter");
    };
    let taken = f
        .blocks()
        .flat_map(|block| assigns(block).into_iter().cloned().collect::<Vec<_>>())
        .find(|operation| matches!(operation.value, Rvalue::Address(_)))
        .expect("an address is taken");

    assert_eq!(taken.value, Rvalue::Address(Place::local(n)));
}

/// A subscript and the arithmetic it is defined as are one shape.
///
/// C17 6.5.2.1 p2 makes `E1[E2]` mean `(*((E1)+(E2)))`, so both spellings
/// work the address out into a local and reach through it. An analysis
/// asking what an access touches then has one thing to read rather than
/// two spellings of it.
///
/// Mutation: have `begin_value` ask for the operand's place under a `*`.
/// `*(p + i)` stops lowering, `SC0304` is reported, and this fails.
///
/// Mutation: swap the base and the index in `finish_place`. The address is
/// worked out from the wrong operand and this fails.
///
/// Mutation: give the subscript a `Projection::Index` on the base's place
/// instead. The two spellings stop agreeing and this fails.
#[test]
fn an_element_is_reached_by_a_subscript_and_by_arithmetic() {
    let lowered =
        lowered("int f(int *p, int i) {\n    p[i] = 1;\n    *(p + i) = 2;\n    return p[i];\n}\n");
    assert_eq!(codes(&lowered), Vec::<String>::new());

    let f = function(&lowered, "f");
    let [p, i] = f.parameters().collect::<Vec<_>>()[..] else {
        panic!("two parameters");
    };
    let address = Rvalue::Binary {
        op: BinOp::Add,
        lhs: Operand::Copy(Place::local(p)),
        rhs: Operand::Copy(Place::local(i)),
    };
    let operations: Vec<_> = f
        .blocks()
        .flat_map(|block| assigns(block).into_iter().cloned().collect::<Vec<_>>())
        .collect();

    // Both spellings work the address out first and reach through it, and
    // neither leaves a projection rooted at the pointer itself.
    let addressed: Vec<_> = operations
        .iter()
        .filter(|operation| operation.value == address)
        .map(|operation| operation.place.local)
        .collect();
    let reached: Vec<_> = operations
        .iter()
        .filter(|operation| operation.place.projection == vec![Projection::Deref])
        .map(|operation| operation.place.local)
        .collect();

    assert_eq!(addressed.len(), 3, "one per subscript and one per `p + i`");
    assert_eq!(reached, addressed[..2], "the two writes reach through it");
    assert!(
        operations
            .iter()
            .all(|operation| operation.place.local != p || operation.place.projection.is_empty()),
        "nothing is a projection of the pointer itself"
    );

    // The last one is the read, and it reaches through an address of its
    // own rather than through either write's.
    let [returned] = &operations[operations.len() - 1..] else {
        panic!("a return");
    };
    assert_eq!(returned.place, Place::local(f.return_place()));
    assert_eq!(
        returned.value,
        Rvalue::Use(Operand::Copy(Place {
            local: addressed[2],
            projection: vec![Projection::Deref],
        }))
    );
}

/// A comma evaluates its left operand and answers its right.
///
/// C17 6.5.17 p2. Mutation: answer the left operand. This fails.
#[test]
fn a_comma_answers_its_right_operand() {
    let lowered = lowered("int f(int a, int b) {\n    return (a, b);\n}\n");
    assert_eq!(codes(&lowered), Vec::<String>::new());

    let f = function(&lowered, "f");
    let [_, b] = f.parameters().collect::<Vec<_>>()[..] else {
        panic!("two parameters");
    };
    // The left operand is evaluated and dropped, and a name needs no
    // operation to be evaluated, so the only operation is the return.
    let [returned] = assigns(f.blocks().next().expect("a block"))[..] else {
        panic!("one operation");
    };
    assert_eq!(returned.place, Place::local(f.return_place()));
    assert_eq!(returned.value, Rvalue::Use(Operand::Copy(Place::local(b))));
}

/// A compound assignment is the assignment C says it stands for.
///
/// C17 6.5.16.2 p3 makes `a -= b` mean `a = a - b` except that `a` is
/// evaluated once, so it lowers to two operations: the subtraction into a
/// temporary of the promoted type, and the place written from that. Writing
/// the subtraction straight into `a` is shorter and says the arithmetic
/// happened at `a`'s width, which is false whenever `a` is narrower than
/// `int`, and is what made `char c; c += 100;` a program this compiler
/// stopped on.
///
/// Mutation: swap the operands, so `a -= b` means `b - a`. This fails.
/// Mutation: write the `Binary` into `a` and drop the temporary. The
/// assertions below fail, and so does
/// `a_compound_assignment_on_a_char_is_not_undefined` in
/// `crates/safec/tests/interp.rs`.
#[test]
fn a_compound_assignment_is_the_assignment_it_stands_for() {
    let lowered = lowered("int f(int a, int b) {\n    a -= b;\n    return a;\n}\n");
    assert_eq!(codes(&lowered), Vec::<String>::new());

    let f = function(&lowered, "f");
    let [a, b] = f.parameters().collect::<Vec<_>>()[..] else {
        panic!("two parameters");
    };
    let operations: Vec<_> = f
        .blocks()
        .flat_map(|block| assigns(block).into_iter().cloned().collect::<Vec<_>>())
        .collect();

    let computed = operations[0].place.local;
    assert!(operations[0].place.projection.is_empty());
    assert_ne!(
        computed, a,
        "the subtraction does not happen at `a`'s width"
    );
    assert_eq!(
        f.local(computed),
        f.local(a),
        "`int`, being the promoted type"
    );
    assert_eq!(
        operations[0].value,
        Rvalue::Binary {
            op: BinOp::Sub,
            lhs: Operand::Copy(Place::local(a)),
            rhs: Operand::Copy(Place::local(b)),
        }
    );

    assert_eq!(operations[1].place, Place::local(a));
    assert_eq!(
        operations[1].value,
        Rvalue::Use(Operand::Copy(Place::local(computed)))
    );
}

/// An increment answers differently before and after.
///
/// C17 6.5.2.4 p2 makes the postfix form answer the value before the
/// increment, and 6.5.3.1 p2 makes the prefix form answer the one after.
/// Both write the same thing to the same place.
///
/// Mutation: keep the old value for the prefix form rather than the
/// postfix one. The two functions below lower alike and this fails.
#[test]
fn an_increment_answers_before_or_after_the_write() {
    let after = lowered("int f(int n) {\n    return n++;\n}\n");
    let before = lowered("int f(int n) {\n    return ++n;\n}\n");
    assert_eq!(codes(&after), Vec::<String>::new());
    assert_eq!(codes(&before), Vec::<String>::new());

    let taken = |lowered: &Lowered| {
        let f = function(lowered, "f");
        let operations: Vec<_> = f
            .blocks()
            .flat_map(|block| assigns(block).into_iter().cloned().collect::<Vec<_>>())
            .collect();
        // Whether the copy of the old value is taken before the write or
        // the copy of the new one after it is the whole difference.
        let stepped = operations
            .iter()
            .position(|operation| matches!(operation.value, Rvalue::Binary { .. }))
            .expect("an increment");
        let held = operations
            .iter()
            .position(|operation| {
                matches!(&operation.value, Rvalue::Use(Operand::Copy(place))
                        if place.local == f.parameters().next().expect("a parameter"))
            })
            .expect("a copy of the place");
        held < stepped
    };

    assert!(taken(&after), "the postfix form keeps the value it had");
    assert!(
        !taken(&before),
        "the prefix form answers the value it wrote"
    );
}

/// A `for` becomes a header, a body, a step and an exit.
///
/// C17 6.8.5.3 p1 puts the step at the end of the body and the condition
/// before each turn, and p2 makes an absent condition a non-zero constant,
/// so a `for (;;)` has no exit edge of its own.
///
/// Mutation: send a `for` with no condition to the block after the loop.
/// The loop stops being one and this fails.
///
/// Mutation: put the step before the body rather than after it. The
/// assertion on which block holds it fails.
#[test]
fn a_for_loop_asks_before_each_turn_and_steps_after_each_body() {
    let counted = lowered(
        "int f(int n) {\n    int i;\n    for (i = 0; i < n; i = i + 1) {\n        n = n - 1;\n    }\n    return n;\n}\n",
    );
    assert_eq!(codes(&counted), Vec::<String>::new());

    let f = function(&counted, "f");
    // The entry runs the initialiser and goes to the header; the header
    // asks and branches; the body runs, steps, and comes back.
    assert_eq!(edges(f), vec![vec![1], vec![2, 3], vec![1], vec![]]);

    // The body holds its own statement and the step after it, and each
    // assignment carries the copy that 6.5.16 p3 fixes its value with.
    let blocks: Vec<_> = f.blocks().collect();
    let stepped = assigns(blocks[2])
        .last()
        .copied()
        .expect("the step is written after the body");
    assert!(matches!(stepped.value, Rvalue::Use(Operand::Copy(_))));
    assert_eq!(
        assigns(blocks[2])[3].value,
        Rvalue::Binary {
            op: BinOp::Add,
            lhs: Operand::Copy(assigns(blocks[2])[4].place.clone()),
            rhs: Operand::Constant(1),
        }
    );

    let forever = lowered("int f(int n) {\n    for (;;) {\n        n = n - 1;\n    }\n}\n");
    assert_eq!(codes(&forever), Vec::<String>::new());

    let f = function(&forever, "f");
    let edges = edges(f);
    let [entry, header, body, after] = &edges[..] else {
        panic!("{edges:?}");
    };
    assert_eq!(entry, &vec![1]);
    assert_eq!(header, &vec![2]);
    assert_eq!(body, &vec![1]);
    assert_eq!(after, &Vec::<usize>::new());
}

/// A prototype and the definition under it are one function.
///
/// Keyed by the span of the name they would be two, because the resolver
/// answers with whichever declaration was in scope, so a call written
/// between them would reach a function with no body.
///
/// Mutation: key the function map on the name's span. Two `add`s appear,
/// the call reaches the one with no body, and this fails.
#[test]
fn a_prototype_and_its_definition_are_one_function() {
    let lowered = lowered(
        "int add(int a, int b);\n\nint main(void) {\n    return add(1, 2);\n}\n\nint add(int a, int b) {\n    return a + b;\n}\n",
    );
    assert_eq!(codes(&lowered), Vec::<String>::new());
    assert_eq!(lowered.unit.functions().len(), 2);

    let add = function(&lowered, "add");
    assert!(add.is_defined());

    let main = function(&lowered, "main");
    let entry = main.blocks().next().expect("a block");
    let Terminator::Call { callee, .. } = &entry.terminator else {
        panic!("{:?}", entry.terminator);
    };
    assert!(lowered.unit.function(*callee).is_defined());
}

/// A name defined twice keeps the first body and is reported.
///
/// C17 6.9 p5 allows one external definition and nothing before this stage
/// checks it, so the check that would panic is answered here instead.
///
/// Mutation: fill the function with the second definition anyway. The
/// assertion in `fill_function` panics, which is a different failure and
/// the reason this arm exists.
#[test]
fn a_name_defined_twice_keeps_the_first_body() {
    let lowered = lowered("int f(void) {\n    return 1;\n}\n\nint f(void) {\n    return 2;\n}\n");
    assert_eq!(codes(&lowered), ["SC0304"]);

    let f = function(&lowered, "f");
    assert!(f.is_defined());
    assert_eq!(
        assigns(f.blocks().next().expect("a block"))[0].value,
        Rvalue::Use(Operand::Constant(1))
    );
}

/// The value of an assignment is what was assigned, and a call cannot
/// change it afterwards.
///
/// C17 6.5.16 p3 fixes the value at the assignment, and 6.5.2.2 p10 makes a
/// callee's execution indeterminately sequenced with the rest of the
/// expression, so `(b = 1) + g(&b)` is one plus whatever `g` answers
/// however `g` treats `b`.
///
/// Mutation: push the assigned place itself as the value rather than a copy
/// of it. The addition reads `b` after the call and this fails.
#[test]
fn the_value_of_an_assignment_is_taken_before_a_call_can_change_it() {
    let lowered =
        lowered("int g(int *p);\n\nint f(void) {\n    int b;\n    return (b = 1) + g(&b);\n}\n");
    assert_eq!(codes(&lowered), Vec::<String>::new());

    let f = function(&lowered, "f");
    let blocks: Vec<_> = f.blocks().collect();
    let Terminator::Call { .. } = &blocks[0].terminator else {
        panic!("{:?}", blocks[0].terminator);
    };

    // Whatever the addition reads for the left operand was written before
    // the call, which is what the block boundary says.
    let added = assigns(blocks[1])
        .into_iter()
        .find_map(|operation| match &operation.value {
            Rvalue::Binary {
                op: BinOp::Add,
                lhs,
                ..
            } => Some(lhs.clone()),
            _ => None,
        })
        .expect("an addition");
    let Operand::Copy(read) = added else {
        panic!("{added:?}");
    };
    assert!(
        assigns(blocks[0])
            .into_iter()
            .any(|operation| operation.place == read),
        "the left operand is a place written before the call"
    );
}

/// An expression that names no place is reported where it is written.
///
/// `1 = 2` type-checks today, because `types.rs` leaves the
/// modifiable-lvalue constraint of C17 6.5.16 p2 to a later phase, so this
/// stage is the first thing that has to say anything about it.
///
/// Mutation: refuse it without reporting. Nothing is said about a program
/// nobody can compile and this fails.
#[test]
fn an_expression_that_names_no_place_is_reported() {
    let lowered = lowered("int f(void) {\n    1 = 2;\n    return 0;\n}\n");
    assert_eq!(codes(&lowered), ["SC0304"]);
    assert!(!function(&lowered, "f").is_defined());
}

/// A call to something that is not a function is reported.
///
/// The report comes from the type check having no type for the call rather
/// than from `callee`, which is what `callee`'s own doc comment says: a
/// callee that could name a pointer is refused at its declaration, because
/// the IR has no function type to give it.
///
/// Mutation: refuse an untyped expression without reporting. This fails.
#[test]
fn a_call_to_something_that_is_not_a_function_is_reported() {
    let lowered = lowered("int f(int p) {\n    return p(1);\n}\n");
    assert_eq!(codes(&lowered), ["SC0304"]);
    assert!(!function(&lowered, "f").is_defined());
}

/// A call to a function whose signature was refused says nothing more.
///
/// The declaration is where the problem is and where it is reported; a
/// second diagnostic on an ordinary call would be blaming code that is
/// fine.
///
/// Mutation: report at the call as well. Two codes come back and this
/// fails.
#[test]
fn a_call_to_a_refused_function_is_not_reported_twice() {
    let lowered = lowered("int g(int a[3]);\n\nint f(void) {\n    return g(0);\n}\n");
    assert_eq!(codes(&lowered), ["SC0304"]);
}

/// An expression deeper than the parser's own nesting limit still lowers.
///
/// A chain folded by a loop adds a level to the tree per operator and none
/// to the parser's count, which is what killed the tree printer once. Ten
/// thousand terms is far past `parser::MAX_NESTING` and nowhere near the
/// native stack.
///
/// Mutation: walk an expression by recursion. The process dies rather than
/// failing, which is why this test exists at all: a stack overflow is not a
/// panic anything can catch.
#[test]
fn an_expression_deeper_than_the_parser_nests_still_lowers() {
    let terms = 10_000;
    let mut program = String::from("int f(int a) {\n    return a");
    for _ in 0..terms {
        program.push_str(" + a");
    }
    program.push_str(";\n}\n");

    let lowered = lowered(&program);
    assert_eq!(codes(&lowered), Vec::<String>::new());

    let f = function(&lowered, "f");
    let operations: usize = f.blocks().map(|block| assigns(block).len()).sum();
    // One per operator, and one more writing the answer into the return
    // place.
    assert_eq!(operations, terms + 1);
}
