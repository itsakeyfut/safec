//! The nullability check, against IR built by hand.
//!
//! `safec_ir::nullability` answers a `Finding` rather than a diagnostic so that
//! it can be read without a frontend, and this is the file that spends that.
//! The corpus under `crates/safec/tests/cases/` holds the shapes this compiler's
//! own lowering produces; what is here is the shapes it does **not** produce and
//! another frontend can.
//!
//! **That is the whole reason this file exists rather than more corpus cases.**
//! Two rules in `safec_ir::nullability` cannot be reached by a C program this
//! frontend lowers: it never puts a store through a pointer between a
//! comparison and the branch that reads it, and it gives each scope's variable
//! a local of its own rather than reusing one across a storage boundary.
//! Measured, both of them. ADR-0011 makes the Clang adapter a reader of this
//! IR without going through `safec`'s frontend, so an unreachable rule today is
//! a reachable one then, and a rule nothing can break is a rule nobody can
//! trust.

use safec_ir::analysis::Conclusion;
use safec_ir::ir::{
    BinOp, Block, BlockId, Element, Function, LocalId, Operand, Operation, Origin, Place,
    Projection, Rvalue, Terminator, TranslationUnit, Ty, TyId,
};
use safec_ir::nullability;
use safec_ir::source::{SourceMap, Span};
use safec_ir::target::Target;

/// The spans this file hands out, one per element that can be reported.
struct Names {
    function: Span,
    /// In the order they appear in the file, so a finding's span says which
    /// element made it.
    at: [Span; 4],
}

fn sources() -> (SourceMap, Names) {
    let mut map = SourceMap::new();
    let file = map.add_virtual("t.c", "f a b c d\n");

    let names = Names {
        function: Span::new(file, 0, 1),
        at: [
            Span::new(file, 2, 3),
            Span::new(file, 4, 5),
            Span::new(file, 6, 7),
            Span::new(file, 8, 9),
        ],
    };

    (map, names)
}

/// A unit and a function taking the pointer parameters this check is about.
///
/// `int` and `int *` and `int **`, because `Nullability` asks a local's type
/// before it refines one: a branch on an `int` says nothing about a pointer,
/// and a test that built its parameters as `int` would be testing that.
fn a_unit(names: &Names, parameters: &[TyId]) -> (TranslationUnit, Function, TyId) {
    let mut unit = TranslationUnit::new(
        Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
    );
    let int = unit.push_type(Ty::Int);
    let function = Function::new(names.function, int, parameters.to_vec());
    (unit, function, int)
}

/// `*place = 1;`, which is a write through whatever `place` names.
fn write_through(local: LocalId, at: Span) -> Element {
    Element::Assign(Operation {
        place: Place {
            local,
            projection: vec![Projection::Deref],
        },
        value: Rvalue::Use(Operand::Constant(1)),
        origin: Origin::Written(at),
    })
}

fn returns() -> Block {
    Block {
        elements: vec![],
        terminator: Terminator::Return,
    }
}

fn goto(to: BlockId, elements: Vec<Element>) -> Block {
    Block {
        elements,
        terminator: Terminator::Goto(to),
    }
}

/// What the check concluded about one function, in span order.
fn concluded(mut unit: TranslationUnit, function: Function) -> Vec<nullability::Finding> {
    unit.push_function(function);
    nullability::findings(&unit)
}

/// A comparison this check cannot reach past a store through a pointer is a
/// comparison it does not read, and the arm is refined by nothing.
///
/// `c = p != 0; *q = 0; if (c)` is the shape. Nothing in the store says which
/// local it lands in, because that is a question about what `q` holds, so it
/// may have written `c`; walking past it would resolve a comparison the program
/// may already have overwritten and refine `p` on an arm that can be reached
/// with `p` null. That is this compiler quiet about something it did not prove.
///
/// **This compiler's own frontend cannot produce it**, which is why the IR is
/// built here rather than as a corpus case: the condition's temporary is always
/// written immediately above the branch that reads it. Measured on the
/// lowering.
///
/// Mutation: drop the arm in `Nullability::tested` that gives up on a store
/// through a projection. The comparison above it is resolved, `p` is refined on
/// the taken arm, the write through `p` stops being reported, and this fails
/// with one finding where it expects two.
#[test]
fn a_comparison_behind_a_store_through_a_pointer_refines_nothing() {
    let (_sources, names) = sources();

    let mut unit = TranslationUnit::new(
        Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
    );
    let int = unit.push_type(Ty::Int);
    let pointer = unit.push_type(Ty::Pointer(int));
    let pointer_to_pointer = unit.push_type(Ty::Pointer(pointer));
    let mut function = Function::new(names.function, int, vec![pointer, pointer_to_pointer]);

    // `_1` is `p`, `_2` is `q`, and the return place is `_0`.
    let p = function.parameters().next().expect("a first parameter");
    let q = function.parameters().nth(1).expect("a second parameter");
    let condition = function.push_local(int);

    let entry = function.reserve_block();
    let taken = function.reserve_block();
    let untaken = function.reserve_block();
    let after = function.reserve_block();

    function.fill_block(
        entry,
        Block {
            elements: vec![
                Element::Assign(Operation {
                    place: Place::local(condition),
                    value: Rvalue::Binary {
                        op: BinOp::Ne,
                        lhs: Operand::Copy(Place::local(p)),
                        rhs: Operand::Constant(0),
                    },
                    origin: Origin::Written(names.at[0]),
                }),
                // `*q = 1`. Its place names `q`, and nothing in it says `c` was
                // left alone.
                write_through(q, names.at[1]),
            ],
            terminator: Terminator::Branch {
                condition: Operand::Copy(Place::local(condition)),
                then: taken,
                otherwise: untaken,
                origin: Origin::Written(names.at[2]),
            },
        },
    );
    function.fill_block(taken, goto(after, vec![write_through(p, names.at[3])]));
    function.fill_block(untaken, goto(after, vec![]));
    function.fill_block(after, returns());

    let found = concluded(unit, function);

    assert_eq!(found.len(), 2, "{found:?}");
    // The store through `q`, which nothing has said anything about.
    assert_eq!(found[0].conclusion, Conclusion::Unknown);
    assert_eq!(found[0].at, names.at[1]);
    // The write through `p` on the taken arm, which the comparison would have
    // refined had this check been allowed to read it.
    assert_eq!(found[1].conclusion, Conclusion::Unknown);
    assert_eq!(found[1].at, names.at[3]);
}

/// Storage beginning or ending leaves a local holding nothing this check knows.
///
/// A local whose address was taken is the one thing this analysis can prove not
/// null, and the proof belongs to the object that was there. Storage ending
/// takes that object away and storage beginning brings a different one, so a
/// proof that outlived either would be about a local the program has since
/// reused.
///
/// **This compiler's own frontend cannot produce it**, because each scope's
/// variable gets a local of its own rather than reusing one across a storage
/// boundary. Measured: two nested blocks each declaring `int *p` lower to `_2`
/// and `_4`.
///
/// Mutation: have either storage arm of `Nullability::element` leave the local
/// alone. The write after that arm stops being reported and this fails with one
/// finding where it expects two.
#[test]
fn storage_beginning_or_ending_leaves_a_pointer_holding_nothing_known() {
    let (_sources, names) = sources();
    let (mut unit, mut function, int) = a_unit(&names, &[]);
    let pointer = unit.push_type(Ty::Pointer(int));

    let object = function.push_local(int);
    let p = function.push_local(pointer);

    let address = |at: Span| {
        Element::Assign(Operation {
            place: Place::local(p),
            value: Rvalue::Address(Place::local(object)),
            origin: Origin::Written(at),
        })
    };

    let entry = function.reserve_block();
    function.fill_block(
        entry,
        Block {
            elements: vec![
                // Proved not null, and then the local holding the proof ends.
                address(names.at[0]),
                Element::StorageDead {
                    origin: Origin::Generated(names.at[0]),
                    local: p,
                },
                write_through(p, names.at[1]),
                // Proved again, and then the same local begins afresh.
                address(names.at[2]),
                Element::StorageLive {
                    local: p,
                    origin: Origin::Generated(names.at[2]),
                },
                write_through(p, names.at[3]),
            ],
            terminator: Terminator::Return,
        },
    );

    let found = concluded(unit, function);

    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(found[0].conclusion, Conclusion::Unknown);
    assert_eq!(found[0].at, names.at[1]);
    assert_eq!(found[1].conclusion, Conclusion::Unknown);
    assert_eq!(found[1].at, names.at[3]);
}

/// A local assigned from its own dereference keeps nothing the dereference
/// established about the pointer it used to hold.
///
/// `pp = *pp;` is the shape, and `p = p->next;` is what it will be written as
/// once this compiler has structs. The dereference says the *old* pointer was
/// not null; the assignment then puts a different pointer there, and nothing is
/// known about that one. An analysis that applied the two the other way round
/// would answer the question about the new value with a fact about the old one
/// and go quiet about the next dereference.
///
/// **This compiler's own frontend cannot produce it**, because a pointer whose
/// dereference has its own type needs a struct and `Ty` has none. The lowering
/// builds the shape regardless, which is the half that matters: `pp = *pp;`
/// reaches `--emit safety-ir` as this IR and is refused by the type checker
/// beside it.
///
/// Mutation: move `met` in `Nullability::element` back below the match, so the
/// dereference is applied after the assignment. `pp` is left proved non-null,
/// the second dereference is reported by nothing, and this fails with one
/// finding where it expects two.
#[test]
fn a_local_assigned_from_its_own_dereference_keeps_nothing() {
    let (_sources, names) = sources();

    let mut unit = TranslationUnit::new(
        Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
    );
    let int = unit.push_type(Ty::Int);
    let pointer = unit.push_type(Ty::Pointer(int));
    let pointer_to_pointer = unit.push_type(Ty::Pointer(pointer));
    let mut function = Function::new(names.function, int, vec![pointer_to_pointer]);

    let pp = function.parameters().next().expect("a first parameter");

    let entry = function.reserve_block();
    function.fill_block(
        entry,
        Block {
            elements: vec![
                // `pp = *pp;`
                Element::Assign(Operation {
                    place: Place::local(pp),
                    value: Rvalue::Use(Operand::Copy(Place {
                        local: pp,
                        projection: vec![Projection::Deref],
                    })),
                    origin: Origin::Written(names.at[0]),
                }),
                write_through(pp, names.at[1]),
            ],
            terminator: Terminator::Return,
        },
    );

    let found = concluded(unit, function);

    assert_eq!(found.len(), 2, "{found:?}");
    // The parameter, which nothing has said anything about.
    assert_eq!(found[0].conclusion, Conclusion::Unknown);
    assert_eq!(found[0].at, names.at[0]);
    // What it was assigned from, which is a different pointer.
    assert_eq!(found[1].conclusion, Conclusion::Unknown);
    assert_eq!(found[1].at, names.at[1]);
}

/// A call reads its arguments where it is reached and writes its destination
/// when it returns, and a pointer that is both keeps neither fact by accident.
///
/// `p = g(*p);` with the result landing in `p` itself. The dereference says the
/// pointer that was there was not null; the call then puts a different one in
/// the same local, and nothing is known about that one. Applied the other way
/// round, the dereference would land on the returned pointer and leave it
/// proved non-null.
///
/// **This compiler's own frontend cannot produce it**, because a call's result
/// always lands in a fresh temporary and is copied out in an element of its
/// own. Measured: `p = g(*p);` reaches the IR as a call into `_2` and then
/// `_1 = Copy(_2)`.
///
/// Mutation: move `met` in `Nullability::terminator` below the match, so the
/// dereference is applied after the destination is written. The write after the
/// call stops being reported and this fails with one finding where it expects
/// two.
///
/// Mutation: have the `Terminator::Call` arm leave the destination alone. The
/// pointer keeps what the dereference established, and this fails the same way.
/// That is the rule which makes a `malloc` result unprovable, which is this
/// module's headline and the roadmap's own example.
#[test]
fn a_call_that_reads_a_pointer_and_writes_it_keeps_neither() {
    let (_sources, names) = sources();

    let mut unit = TranslationUnit::new(
        Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
    );
    let int = unit.push_type(Ty::Int);
    let pointer = unit.push_type(Ty::Pointer(int));
    let callee = unit.push_function(Function::declaration(names.function, pointer, [int]));
    let mut function = Function::new(names.function, int, vec![pointer]);

    let p = function.parameters().next().expect("a first parameter");

    let entry = function.reserve_block();
    let after = function.reserve_block();

    function.fill_block(
        entry,
        Block {
            elements: vec![],
            terminator: Terminator::Call {
                callee,
                arguments: vec![Operand::Copy(Place {
                    local: p,
                    projection: vec![Projection::Deref],
                })],
                destination: Some(Place::local(p)),
                then: after,
                origin: Origin::Written(names.at[0]),
            },
        },
    );
    function.fill_block(
        after,
        Block {
            elements: vec![write_through(p, names.at[1])],
            terminator: Terminator::Return,
        },
    );

    let found = concluded(unit, function);

    assert_eq!(found.len(), 2, "{found:?}");
    // The argument, read through the pointer that was there.
    assert_eq!(found[0].conclusion, Conclusion::Unknown);
    assert_eq!(found[0].at, names.at[0]);
    // The pointer the call returned, which is a different one.
    assert_eq!(found[1].conclusion, Conclusion::Unknown);
    assert_eq!(found[1].at, names.at[1]);
}
