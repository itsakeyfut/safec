//! The nullability check, against IR built by hand.
//!
//! `safec_ir::nullability` answers a `Finding` rather than a diagnostic so that
//! it can be read without a frontend, and this is the file that spends that.
//! The corpus under `crates/safec/tests/cases/` holds the shapes this compiler's
//! own lowering produces; what is here is the shapes it does **not** produce and
//! another frontend can.
//!
//! **That is the whole reason this file exists rather than more corpus cases.**
//! Rules in `safec_ir::nullability` that no C program this frontend lowers can
//! reach: it puts nothing at all between a comparison and the branch that
//! reads it, whether a store through a pointer, a store to a local, a storage
//! boundary or a sequence-point marker, and it gives each scope's variable a
//! local of its own rather than reusing one across such a boundary.
//! Measured, all of them. ADR-0011 makes the Clang adapter a reader of this
//! IR without going through `safec`'s frontend, so an unreachable rule today is
//! a reachable one then, and a rule nothing can break is a rule nobody can
//! trust.

use safec_ir::analysis::Conclusion;
use safec_ir::ir::{
    BinOp, Block, BlockId, Element, Function, LocalId, Operand, Operation, Origin, Parameter,
    Place, Projection, Promise, Rvalue, Terminator, TranslationUnit, Ty, TyId, UnOp,
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
/// Mutation: drop the arm in `nullability::tested_against_null` that gives up on a store
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

/// A comparison the block overwrote the pointer under is a comparison about a
/// value the branch no longer reads.
///
/// `c = (p != 0); p = q; if (c) { *p = 1; }` is the shape. The comparison read
/// what `p` held above the store, and the arm is reached with whatever `q`
/// held, which nothing here established. Refining `p` on that arm is this
/// compiler proving something about a pointer that is no longer there.
///
/// **`Ne` rather than `Eq`, so the wrong answer is the quiet one.**
/// `Nullness::NonNull` has no conclusion at all, so a refinement made here
/// deletes the write's finding rather than changing it, and what this asserts
/// is that the finding is still there. The other order is the one #220 is
/// filed about, where the wrong answer instead exempts a `free` in
/// `safec_ir::memory` and reports nothing; both are the same walk being wrong
/// about the same thing, and this is the half that can be asserted without a
/// `malloc`.
///
/// **This compiler's own frontend cannot produce it**: measured over the
/// corpus, no block whose branch reads a bare local has anything at all
/// between that local's write and the terminator.
///
/// Mutation: have `nullability::tested_against_null` step over a direct store without
/// recording the local it changed. The comparison above it is resolved and
/// nothing refuses the answer, `p` is refined to non-null on the taken arm,
/// the write through it is reported by nobody, and this fails with no findings
/// where it expects one. Dropping the `filter` that applies the record fails
/// this and `a_comparison_above_a_storage_boundary_refines_nothing` together,
/// which is right: one refusal, two ways of reaching it.
#[test]
fn a_comparison_above_a_store_to_the_pointer_it_tested_refines_nothing() {
    let (_sources, names) = sources();

    let mut unit = TranslationUnit::new(
        Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
    );
    let int = unit.push_type(Ty::Int);
    let pointer = unit.push_type(Ty::Pointer(int));
    let mut function = Function::new(names.function, int, vec![pointer, pointer]);

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
                // `p = q`. An ordinary store, naming a local directly, below
                // the comparison that read the local it names.
                Element::Assign(Operation {
                    place: Place::local(p),
                    value: Rvalue::Use(Operand::Copy(Place::local(q))),
                    origin: Origin::Written(names.at[1]),
                }),
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

    // The write through `p` on the taken arm. Nothing established what `q`
    // held, so nothing is established about `p` where it is written through.
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].conclusion, Conclusion::Unknown);
    assert_eq!(found[0].at, names.at[3]);
}

/// A comparison a storage boundary separates from its branch is about an object
/// the arm no longer holds.
///
/// `c = (p != 0); StorageDead(p); StorageLive(p); if (c) { *p = 1; }` is the
/// shape, and it is the neighbouring half of
/// `storage_beginning_or_ending_leaves_a_pointer_holding_nothing_known`: that
/// one is the transfer answering for a boundary it walks over, this one is the
/// backwards walk answering for a boundary it would otherwise step past. The
/// object the comparison read is gone and a different one is in the local, so
/// the comparison says nothing about what the arm dereferences.
///
/// `p` is a local rather than a parameter, because a parameter's storage does
/// not end.
///
/// **This compiler's own frontend cannot produce it**, for the reason the
/// module doc gives: each scope's variable gets a local of its own.
///
/// **Both markers are one arm, and this test is that arm's whole guard.**
/// They conclude the same thing about the local they name, so writing them as
/// one behaviour is what keeps one test enough; split into two, whichever half
/// this block does not reach first would be held by nothing.
///
/// Mutation: have `nullability::tested_against_null` step over a storage boundary without
/// recording the local it changed. `p` is refined to non-null on the taken
/// arm, the write through it is reported by nobody, and this fails with no
/// findings where it expects one.
#[test]
fn a_comparison_above_a_storage_boundary_refines_nothing() {
    let (_sources, names) = sources();
    let (mut unit, mut function, int) = a_unit(&names, &[]);
    let pointer = unit.push_type(Ty::Pointer(int));
    let p = function.push_local(pointer);
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
                // The scope `p` was compared in ends and another begins, both
                // generated: no source text says either, which is ADR-0012.
                Element::StorageDead {
                    local: p,
                    origin: Origin::Generated(names.at[1]),
                },
                Element::StorageLive {
                    local: p,
                    origin: Origin::Generated(names.at[1]),
                },
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

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].conclusion, Conclusion::Unknown);
    assert_eq!(found[0].at, names.at[3]);
}

/// A marker between a comparison and its branch is stepped over, and the
/// refinement survives it.
///
/// `c = (p != 0); Sequenced; if (c) { *p = 1; } else { *p = 2; }` is the shape.
/// `Element::Sequenced` says where C ordered one evaluation before another and
/// computes into no local, so it cannot have replaced what the comparison read,
/// and the walk has to go past it rather than give up.
///
/// **It asserts the direction its siblings do not.** They assert that a
/// refinement is *not* made; this one and
/// `a_write_to_another_local_between_a_comparison_and_its_branch_keeps_the_refinement`
/// assert that one still is, which is what a walk that gave up on everything
/// would take away. The three markers are one arm for the same reason the
/// storage pair is, so this is that arm's whole guard.
///
/// Both arms are written through, so that the assertion is what each arm
/// concluded rather than a count of nothing. The taken arm has `p` proved not
/// null and is reported by nobody; the other arm has it proved null and is the
/// one finding.
///
/// Mutation: have `nullability::tested_against_null` give up at a marker instead of
/// stepping over it. Neither arm is refined, both writes are reported as
/// unproven, and this fails with two findings where it expects one.
#[test]
fn a_marker_between_a_comparison_and_its_branch_is_stepped_over() {
    let (_sources, names) = sources();

    let mut unit = TranslationUnit::new(
        Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
    );
    let int = unit.push_type(Ty::Int);
    let pointer = unit.push_type(Ty::Pointer(int));
    let mut function = Function::new(names.function, int, vec![pointer]);

    let p = function.parameters().next().expect("a first parameter");
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
                // Nobody writes a sequence point, so it carries the span of
                // what asked for it, which here is the branch below.
                Element::Sequenced {
                    origin: Origin::Generated(names.at[2]),
                },
            ],
            terminator: Terminator::Branch {
                condition: Operand::Copy(Place::local(condition)),
                then: taken,
                otherwise: untaken,
                origin: Origin::Written(names.at[2]),
            },
        },
    );
    function.fill_block(taken, goto(after, vec![write_through(p, names.at[1])]));
    function.fill_block(untaken, goto(after, vec![write_through(p, names.at[3])]));
    function.fill_block(after, returns());

    let found = concluded(unit, function);

    // Only the arm the comparison proved `p` null on. The other arm's write is
    // through a pointer this check proved is not null, which is nothing to say.
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].conclusion, Conclusion::Unsafe);
    assert_eq!(found[0].at, names.at[3]);
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
                then: Some(after),
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

/// Two functions that share a caret are asked about apart.
///
/// The findings of a whole unit are sorted by caret and then folded where two
/// say the same thing at one, and a driver decides what to do with each by the
/// function it names: one inside a hatch is listed rather than reported. So a
/// fold that ignored the function would let the first function's finding stand
/// for the second's, and where the first is a hatch the second's unproven
/// dereference would go unreported.
///
/// **This compiler's own frontend cannot produce it**: no two of its functions
/// share a span. A fragment `#include`d into two bodies would, and so could
/// the Clang adapter, which reads this IR without this frontend.
///
/// Mutation: drop `later.function == earlier.function` from the key in
/// `nullability::findings`' `dedup_by`. The two findings fold into the first
/// function's, and this fails with one where it expects two.
#[test]
fn two_functions_that_share_a_caret_are_asked_about_apart() {
    let (_map, names) = sources();
    let (mut unit, _, int) = a_unit(&names, &[]);
    let pointer = unit.push_type(Ty::Pointer(int));

    for hatch in [true, false] {
        let mut function = Function::new(names.function, int, [pointer]);
        let p = function.parameters().next().expect("one parameter");
        let entry = function.reserve_block();
        function.fill_block(
            entry,
            Block {
                elements: vec![write_through(p, names.at[0])],
                terminator: Terminator::Return,
            },
        );
        if hatch {
            function = function.hatched();
        }
        unit.push_function(function);
    }

    let found = nullability::findings(&unit);

    let functions: Vec<_> = found.iter().map(|finding| finding.function).collect();
    assert_eq!(found.len(), 2, "{found:?}");
    assert_ne!(functions[0], functions[1], "{found:?}");
    assert!(
        found
            .iter()
            .all(|finding| finding.conclusion == Conclusion::Unknown && finding.at == names.at[0]),
        "{found:?}"
    );
}

/// A pointer an index selects and a dereference then reads through was read
/// out of memory, and is asked as unproven whatever the local it started from
/// is known to be.
///
/// `t = &x; t[0][..] = 1;` as a place `[Index, Deref]`, with `t` established
/// non-null by the address. The `Deref` reads the pointer the index selected,
/// which has no row in the lattice.
///
/// **This compiler's own frontend cannot produce it**: nothing it lowers
/// builds a `Projection::Index`, and a subscript becomes arithmetic into a
/// temporary. Measured, see #333.
///
/// Mutation: have `nullability::read_out_of_memory` count `Deref`s and answer
/// for two or more, rather than for one after the first element. The index is
/// not counted, nothing is reported, and this fails with no finding where it
/// expects one.
#[test]
fn a_pointer_selected_by_an_index_and_read_through_is_not_proved() {
    let (_sources, names) = sources();

    let mut unit = TranslationUnit::new(
        Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
    );
    let int = unit.push_type(Ty::Int);
    let pointer = unit.push_type(Ty::Pointer(int));
    let pointer_to_pointer = unit.push_type(Ty::Pointer(pointer));
    let mut function = Function::new(names.function, int, vec![]);
    let x = function.push_local(pointer);
    let t = function.push_local(pointer_to_pointer);

    let entry = function.reserve_block();
    function.fill_block(
        entry,
        Block {
            elements: vec![
                // `t = &x;`, which is the one thing this check proves non-null.
                Element::Assign(Operation {
                    place: Place::local(t),
                    value: Rvalue::Address(Place::local(x)),
                    origin: Origin::Written(names.at[0]),
                }),
                Element::Assign(Operation {
                    place: Place {
                        local: t,
                        projection: vec![
                            Projection::Index(Operand::Constant(0)),
                            Projection::Deref,
                        ],
                    },
                    value: Rvalue::Use(Operand::Constant(1)),
                    origin: Origin::Written(names.at[1]),
                }),
            ],
            terminator: Terminator::Return,
        },
    );

    let found = concluded(unit, function);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].conclusion, Conclusion::Unknown);
    assert_eq!(found[0].at, names.at[1]);
}

/// An index after a dereference selects an element of what the pointer points
/// at, and reads no pointer out of memory, so it is not asked.
///
/// `p = &x; (*p)[0] = 1;` as a place `[Deref, Index]`, with `p` established
/// non-null by the address. Only the `Deref` reads a pointer, and it reads
/// `p`'s own value, which this check proved.
///
/// **This compiler's own frontend cannot produce it**, for the reason the
/// test above gives.
///
/// Mutation: have `nullability::read_out_of_memory` answer for any element
/// after the first rather than for a `Deref`. The index is counted, the write
/// is doubted, and this fails with one finding where it expects none.
#[test]
fn an_index_after_a_dereference_reads_no_pointer_out_of_memory() {
    let (_sources, names) = sources();

    let mut unit = TranslationUnit::new(
        Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
    );
    let int = unit.push_type(Ty::Int);
    let pointer = unit.push_type(Ty::Pointer(int));
    let mut function = Function::new(names.function, int, vec![]);
    let x = function.push_local(int);
    let p = function.push_local(pointer);

    let entry = function.reserve_block();
    function.fill_block(
        entry,
        Block {
            elements: vec![
                // `p = &x;`, which is the one thing this check proves non-null.
                Element::Assign(Operation {
                    place: Place::local(p),
                    value: Rvalue::Address(Place::local(x)),
                    origin: Origin::Written(names.at[0]),
                }),
                Element::Assign(Operation {
                    place: Place {
                        local: p,
                        projection: vec![
                            Projection::Deref,
                            Projection::Index(Operand::Constant(0)),
                        ],
                    },
                    value: Rvalue::Use(Operand::Constant(1)),
                    origin: Origin::Written(names.at[1]),
                }),
            ],
            terminator: Terminator::Return,
        },
    );

    let found = concluded(unit, function);

    assert!(found.is_empty(), "{found:?}");
}

/// A write to another local between a comparison and its branch changes
/// nothing the comparison read, and the arm is refined all the same.
///
/// `c = p != 0; x = 1; if (c) { *p = 1; }` is the shape. The walk back from the
/// branch steps over `x = 1`, records that `x` changed, reaches the comparison,
/// and refuses it only if the comparison was about `x`, which it is not.
///
/// **This compiler's own frontend cannot produce it**, for the reason the
/// module doc gives: nothing is lowered between a comparison and its branch.
/// So this is the only guard of the per-local refusal. Its old argument,
/// `int x = 5; if (p) { *p = x; }`, was held by nothing and no longer reaches
/// the walk (#334).
///
/// Mutation: have `nullability::tested_against_null` stop the walk at a write
/// to another local, the positional rule. The comparison is never reached,
/// `p` is not refined, and this fails with one finding where it expects none.
#[test]
fn a_write_to_another_local_between_a_comparison_and_its_branch_keeps_the_refinement() {
    let (_sources, names) = sources();

    let mut unit = TranslationUnit::new(
        Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
    );
    let int = unit.push_type(Ty::Int);
    let pointer = unit.push_type(Ty::Pointer(int));
    let mut function = Function::new(names.function, int, vec![pointer]);

    let p = function.parameters().next().expect("a first parameter");
    let x = function.push_local(int);
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
                // `x = 1`, below the comparison, about a local it did not read.
                Element::Assign(Operation {
                    place: Place::local(x),
                    value: Rvalue::Use(Operand::Constant(1)),
                    origin: Origin::Written(names.at[1]),
                }),
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

    assert!(found.is_empty(), "{found:?}");
}

/// A branch on a pointer tests that pointer, whatever wrote it just above.
///
/// `p = q; if (p) { *p = 1; }` is the shape, with `q` a parameter nothing
/// established. The branch reads `p` where it runs, so the taken arm holds a
/// pointer that is not null, and the write through it is nothing to say. This
/// frontend lowers `int *p = q; if (p)` to exactly this block.
///
/// Mutation: delete the early return for a pointer condition in
/// `nullability::tested_against_null`. The walk reaches `p = q`, a copy is
/// not a comparison it can read, `p` is not refined, and this fails with one
/// finding where it expects none (#334).
#[test]
fn a_pointer_condition_written_just_above_its_branch_is_refined() {
    let (_sources, names) = sources();

    let mut unit = TranslationUnit::new(
        Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
    );
    let int = unit.push_type(Ty::Int);
    let pointer = unit.push_type(Ty::Pointer(int));
    let mut function = Function::new(names.function, int, vec![pointer]);

    let q = function.parameters().next().expect("a first parameter");
    let p = function.push_local(pointer);

    let entry = function.reserve_block();
    let taken = function.reserve_block();
    let untaken = function.reserve_block();
    let after = function.reserve_block();

    function.fill_block(
        entry,
        Block {
            elements: vec![Element::Assign(Operation {
                place: Place::local(p),
                value: Rvalue::Use(Operand::Copy(Place::local(q))),
                origin: Origin::Written(names.at[0]),
            })],
            terminator: Terminator::Branch {
                condition: Operand::Copy(Place::local(p)),
                then: taken,
                otherwise: untaken,
                origin: Origin::Written(names.at[1]),
            },
        },
    );
    function.fill_block(taken, goto(after, vec![write_through(p, names.at[2])]));
    function.fill_block(untaken, goto(after, vec![]));
    function.fill_block(after, returns());

    let found = concluded(unit, function);

    assert!(found.is_empty(), "{found:?}");
}

/// A function that promised its result is not null is asked what the return
/// place holds where it returns, even where the write was in another block.
///
/// `_0 = &x; goto ret; ret: return;`, with the function promising its result.
/// The return place is established not null by the address, and the `Return`
/// is in a block that writes nothing. What is asked is the value, which the
/// analysis carried across the edge, so nothing is reported; only the caret
/// would have been the function's name had anything been.
///
/// **This compiler's own frontend cannot produce it**: a `return` writes the
/// return place in the block its `Return` ends, which `docs/c-family.md`
/// records as a requirement on another frontend and this is the shape that
/// breaks it.
///
/// Mutation: have `nullability::report_return` report the end of the body
/// wherever the block wrote nothing, whatever the return place holds. This
/// fails with one finding where it expects none.
#[test]
fn a_return_place_written_in_an_earlier_block_is_not_taken_for_the_end() {
    let (_sources, names) = sources();

    let mut unit = TranslationUnit::new(
        Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
    );
    let int = unit.push_type(Ty::Int);
    let pointer = unit.push_type(Ty::Pointer(int));
    let mut function =
        Function::new(names.function, pointer, vec![]).promising(Promise::Declared(names.at[0]));
    let x = function.push_local(int);

    let entry = function.reserve_block();
    let ret = function.reserve_block();
    function.fill_block(
        entry,
        goto(
            ret,
            vec![Element::Assign(Operation {
                place: Place::local(function.return_place()),
                value: Rvalue::Address(Place::local(x)),
                origin: Origin::Written(names.at[1]),
            })],
        ),
    );
    function.fill_block(ret, returns());

    let found = concluded(unit, function);

    assert!(found.is_empty(), "{found:?}");
}

/// A copy whose source is overwritten before the branch carries nothing back
/// to the source.
///
/// `c = q; q = r; if (c) { *q = 1; }`, with `q` and `r` parameters nothing
/// established. The branch tests what `q` held before `q = r`, so the arm
/// knows nothing about the `q` it writes through, and the write is reported.
///
/// **This compiler's own frontend cannot produce it**: the copy an
/// assignment used as a condition makes is the last thing above its branch.
///
/// Mutation: have `nullability::copied_from` follow a copy whatever was
/// written to its source below it. `q` is refined on the taken arm, and this
/// fails with no finding where it expects one.
#[test]
fn a_copy_whose_source_is_overwritten_before_the_branch_refines_only_the_copy() {
    let (_sources, names) = sources();

    let mut unit = TranslationUnit::new(
        Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
    );
    let int = unit.push_type(Ty::Int);
    let pointer = unit.push_type(Ty::Pointer(int));
    let mut function = Function::new(names.function, int, vec![pointer, pointer]);

    let q = function.parameters().next().expect("a first parameter");
    let r = function.parameters().nth(1).expect("a second parameter");
    let c = function.push_local(pointer);

    let entry = function.reserve_block();
    let taken = function.reserve_block();
    let untaken = function.reserve_block();
    let after = function.reserve_block();

    function.fill_block(
        entry,
        Block {
            elements: vec![
                Element::Assign(Operation {
                    place: Place::local(c),
                    value: Rvalue::Use(Operand::Copy(Place::local(q))),
                    origin: Origin::Written(names.at[0]),
                }),
                Element::Assign(Operation {
                    place: Place::local(q),
                    value: Rvalue::Use(Operand::Copy(Place::local(r))),
                    origin: Origin::Written(names.at[1]),
                }),
            ],
            terminator: Terminator::Branch {
                condition: Operand::Copy(Place::local(c)),
                then: taken,
                otherwise: untaken,
                origin: Origin::Written(names.at[2]),
            },
        },
    );
    function.fill_block(taken, goto(after, vec![write_through(q, names.at[3])]));
    function.fill_block(untaken, goto(after, vec![]));
    function.fill_block(after, returns());

    let found = concluded(unit, function);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].conclusion, Conclusion::Unknown);
    assert_eq!(found[0].at, names.at[3]);
}

/// A store through a pointer between a copy and its branch carries nothing
/// back to the copy's source.
///
/// `c = q; *s = 1; if (c) { *q = 1; }`, with `q` and `s` parameters. Nothing
/// in the store says which local it lands in, and `q` may be one it reaches,
/// so the arm knows nothing about the `q` it writes through. Both writes are
/// reported, each as unproven.
///
/// **This compiler's own frontend cannot produce it**, for the reason the
/// test above gives.
///
/// Mutation: have `nullability::copied_from` step over a store through a
/// projection as it does a store to another local. `q` is refined on the
/// taken arm, and this fails with one finding where it expects two.
#[test]
fn a_store_through_a_pointer_between_a_copy_and_its_branch_refines_only_the_copy() {
    let (_sources, names) = sources();

    let mut unit = TranslationUnit::new(
        Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
    );
    let int = unit.push_type(Ty::Int);
    let pointer = unit.push_type(Ty::Pointer(int));
    let mut function = Function::new(names.function, int, vec![pointer, pointer]);

    let q = function.parameters().next().expect("a first parameter");
    let s = function.parameters().nth(1).expect("a second parameter");
    let c = function.push_local(pointer);

    let entry = function.reserve_block();
    let taken = function.reserve_block();
    let untaken = function.reserve_block();
    let after = function.reserve_block();

    function.fill_block(
        entry,
        Block {
            elements: vec![
                Element::Assign(Operation {
                    place: Place::local(c),
                    value: Rvalue::Use(Operand::Copy(Place::local(q))),
                    origin: Origin::Written(names.at[0]),
                }),
                write_through(s, names.at[1]),
            ],
            terminator: Terminator::Branch {
                condition: Operand::Copy(Place::local(c)),
                then: taken,
                otherwise: untaken,
                origin: Origin::Written(names.at[2]),
            },
        },
    );
    function.fill_block(taken, goto(after, vec![write_through(q, names.at[3])]));
    function.fill_block(untaken, goto(after, vec![]));
    function.fill_block(after, returns());

    let found = concluded(unit, function);

    let at: Vec<_> = found
        .iter()
        .map(|finding| (finding.at, finding.conclusion))
        .collect();
    assert_eq!(
        at,
        [
            (names.at[1], Conclusion::Unknown),
            (names.at[3], Conclusion::Unknown),
        ],
        "{found:?}"
    );
}

/// Storage beginning between a copy and its branch carries nothing back to
/// the copy's source.
///
/// `c = q; StorageLive(q); if (c) { *q = 1; }`, with `q` a parameter. The
/// branch tests what `q` held before its storage began again, and the `q`
/// the arm writes through is a different object the arm knows nothing about,
/// so the write is reported.
///
/// **This compiler's own frontend cannot produce it**: the copy an
/// assignment used as a condition makes is the last thing above its branch.
///
/// Mutation: have `nullability::copied_from` step over a storage boundary
/// without recording the local it names. `q` is refined on the taken arm,
/// and this fails with no finding where it expects one.
#[test]
fn a_storage_boundary_between_a_copy_and_its_branch_refines_only_the_copy() {
    let (_sources, names) = sources();

    let mut unit = TranslationUnit::new(
        Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
    );
    let int = unit.push_type(Ty::Int);
    let pointer = unit.push_type(Ty::Pointer(int));
    let mut function = Function::new(names.function, int, vec![pointer]);

    let q = function.parameters().next().expect("a first parameter");
    let c = function.push_local(pointer);

    let entry = function.reserve_block();
    let taken = function.reserve_block();
    let untaken = function.reserve_block();
    let after = function.reserve_block();

    function.fill_block(
        entry,
        Block {
            elements: vec![
                Element::Assign(Operation {
                    place: Place::local(c),
                    value: Rvalue::Use(Operand::Copy(Place::local(q))),
                    origin: Origin::Written(names.at[0]),
                }),
                Element::StorageLive {
                    local: q,
                    origin: Origin::Generated(names.at[1]),
                },
            ],
            terminator: Terminator::Branch {
                condition: Operand::Copy(Place::local(c)),
                then: taken,
                otherwise: untaken,
                origin: Origin::Written(names.at[2]),
            },
        },
    );
    function.fill_block(taken, goto(after, vec![write_through(q, names.at[3])]));
    function.fill_block(untaken, goto(after, vec![]));
    function.fill_block(after, returns());

    let found = concluded(unit, function);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].conclusion, Conclusion::Unknown);
    assert_eq!(found[0].at, names.at[3]);
}

/// A copy is followed only to the last write of what it copied, and a
/// write that is not a copy ends the walk there.
///
/// `q = r; q = <computed>; c = q; if (c) { *r = 1; }`, with `r` and `s`
/// parameters, for each way `q` can be written without copying a local: a
/// read out of memory, a constant other than zero, a unary and a binary
/// operation. `c` holds that value, which says nothing about `r`, so the
/// write through `r` on the taken arm is reported each time, along with the
/// read through `s` where there is one. The address of an object is left
/// out: it proves `q` not null, the branch learns nothing new, and nothing
/// is refined whichever way the walk goes.
///
/// **This compiler's own frontend cannot produce it**: the copy an
/// assignment used as a condition makes is the last thing above its branch,
/// and the walk only reaches past that copy to the assignment it copies,
/// which writes its value through a temporary of its own.
///
/// Mutation: have `nullability::copied_from` walk on past any one of these
/// last writes, as it does past a write to another local. It reaches `q =
/// r`, refines `r` on the taken arm, and the write through `r` is not
/// reported for that row.
#[test]
fn a_copy_is_followed_no_further_than_the_write_it_copied() {
    let (_sources, names) = sources();

    let mut unit = TranslationUnit::new(
        Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
    );
    let int = unit.push_type(Ty::Int);
    let pointer = unit.push_type(Ty::Pointer(int));
    let pointer_to_pointer = unit.push_type(Ty::Pointer(pointer));

    let computed = |s: LocalId| {
        [
            Rvalue::Use(Operand::Copy(Place {
                local: s,
                projection: vec![Projection::Deref],
            })),
            Rvalue::Use(Operand::Constant(8)),
            Rvalue::Unary {
                op: UnOp::Neg,
                operand: Operand::Constant(8),
            },
            Rvalue::Binary {
                op: BinOp::Add,
                lhs: Operand::Constant(8),
                rhs: Operand::Constant(0),
            },
        ]
    };

    for row in 0..4 {
        let mut unit = unit.clone();
        let mut function = Function::new(names.function, int, vec![pointer, pointer_to_pointer]);

        let r = function.parameters().next().expect("a first parameter");
        let s = function.parameters().nth(1).expect("a second parameter");
        let q = function.push_local(pointer);
        let c = function.push_local(pointer);
        let value = computed(s)[row].clone();

        let entry = function.reserve_block();
        let taken = function.reserve_block();
        let untaken = function.reserve_block();
        let after = function.reserve_block();

        function.fill_block(
            entry,
            Block {
                elements: vec![
                    Element::Assign(Operation {
                        place: Place::local(q),
                        value: Rvalue::Use(Operand::Copy(Place::local(r))),
                        origin: Origin::Written(names.at[0]),
                    }),
                    Element::Assign(Operation {
                        place: Place::local(q),
                        value,
                        origin: Origin::Written(names.at[1]),
                    }),
                    Element::Assign(Operation {
                        place: Place::local(c),
                        value: Rvalue::Use(Operand::Copy(Place::local(q))),
                        origin: Origin::Written(names.at[2]),
                    }),
                ],
                terminator: Terminator::Branch {
                    condition: Operand::Copy(Place::local(c)),
                    then: taken,
                    otherwise: untaken,
                    origin: Origin::Written(names.at[2]),
                },
            },
        );
        function.fill_block(taken, goto(after, vec![write_through(r, names.at[3])]));
        function.fill_block(untaken, goto(after, vec![]));
        function.fill_block(after, returns());

        unit.push_function(function);
        let found = nullability::findings(&unit);

        let last = found.last().map(|finding| (finding.at, finding.conclusion));
        assert_eq!(
            last,
            Some((names.at[3], Conclusion::Unknown)),
            "row {row}: {found:?}"
        );
    }
}

/// Storage beginning for the local being followed ends the walk, because
/// what that local held above it was a different object's.
///
/// `q = r; StorageLive(q); c = q; if (c) { *r = 1; }`, with `r` a parameter.
/// `c` holds whatever the new `q` held, which `q = r` did not write, so the
/// arm knows nothing about `r` and the write through it is reported.
///
/// **This compiler's own frontend cannot produce it**: it gives each scope's
/// variable a local of its own, so no local's storage begins again between
/// an assignment and the copy that reads it.
///
/// Mutation: have `nullability::copied_from` record a storage boundary of the
/// local it is following and carry on, as it does for any other local. It
/// reaches `q = r`, refines `r` on the taken arm, and this fails with no
/// finding where it expects one.
#[test]
fn a_storage_boundary_of_the_followed_local_ends_the_walk() {
    let (_sources, names) = sources();

    let mut unit = TranslationUnit::new(
        Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
    );
    let int = unit.push_type(Ty::Int);
    let pointer = unit.push_type(Ty::Pointer(int));
    let mut function = Function::new(names.function, int, vec![pointer]);

    let r = function.parameters().next().expect("a first parameter");
    let q = function.push_local(pointer);
    let c = function.push_local(pointer);

    let entry = function.reserve_block();
    let taken = function.reserve_block();
    let untaken = function.reserve_block();
    let after = function.reserve_block();

    function.fill_block(
        entry,
        Block {
            elements: vec![
                Element::Assign(Operation {
                    place: Place::local(q),
                    value: Rvalue::Use(Operand::Copy(Place::local(r))),
                    origin: Origin::Written(names.at[0]),
                }),
                Element::StorageLive {
                    local: q,
                    origin: Origin::Generated(names.at[1]),
                },
                Element::Assign(Operation {
                    place: Place::local(c),
                    value: Rvalue::Use(Operand::Copy(Place::local(q))),
                    origin: Origin::Written(names.at[2]),
                }),
            ],
            terminator: Terminator::Branch {
                condition: Operand::Copy(Place::local(c)),
                then: taken,
                otherwise: untaken,
                origin: Origin::Written(names.at[2]),
            },
        },
    );
    function.fill_block(taken, goto(after, vec![write_through(r, names.at[3])]));
    function.fill_block(untaken, goto(after, vec![]));
    function.fill_block(after, returns());

    let found = concluded(unit, function);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].conclusion, Conclusion::Unknown);
    assert_eq!(found[0].at, names.at[3]);
}

/// Where two dereferences share a caret and disagree, the proof is the one
/// reported.
///
/// `*p` through a parameter nothing tested, and `*q` through a local just
/// given null, both at one span. The first is unproven and the second proved,
/// and the fold in `nullability::findings` keeps one of them. Keeping the
/// first reported a proof as a suspicion, and inside a hatch, where a
/// suspicion is listed rather than reported, a proved null dereference built.
///
/// **Built as IR so that it holds whatever a frontend's spans are.** The C
/// lowering reaches it today only through the two arms of a `?:`, written at
/// the whole conditional's span, which
/// `a_proved_null_dereference_in_one_arm_of_a_conditional_in_a_hatch_is_still_reported`
/// holds; the operands of a `&&` reached it too until #147 narrowed them, and
/// narrowing an arm would retire the `?:` case the same way.
///
/// Mutation: never swap in `nullability::findings`' `dedup_by`, keeping the
/// earlier finding. The conclusion is `Unknown` and this fails.
#[test]
fn a_proof_and_a_suspicion_at_one_caret_report_the_proof() {
    let (_sources, names) = sources();
    let (mut unit, _, int) = a_unit(&names, &[]);
    let pointer = unit.push_type(Ty::Pointer(int));
    let mut function = Function::new(names.function, int, [pointer]);
    let p = function.parameters().next().expect("one parameter");
    let q = function.push_local(pointer);

    let entry = function.reserve_block();
    function.fill_block(
        entry,
        Block {
            elements: vec![
                Element::Assign(Operation {
                    place: Place::local(q),
                    value: Rvalue::Use(Operand::Constant(0)),
                    origin: Origin::Written(names.at[1]),
                }),
                write_through(p, names.at[0]),
                write_through(q, names.at[0]),
            ],
            terminator: Terminator::Return,
        },
    );

    let found = concluded(unit, function);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].at, names.at[0]);
    assert_eq!(
        found[0].conclusion,
        Conclusion::Unsafe,
        "the proof survives"
    );
}

/// Where a plain dereference and one through a pointer read out of memory
/// share a caret, the question kept is the one through memory, in either
/// order.
///
/// `*p` and `**pp` at one span, both unproven. The remedies differ: testing
/// `p` settles the first, and the second needs the pointer read out of `*pp`
/// kept in a local and tested there. One report stands for both, so it has to
/// carry the remedy that settles both, whichever was met first.
///
/// **Built as IR** for the reason the test above gives; in C it is
/// `a_doubt_through_memory_in_either_arm_of_a_conditional_keeps_its_remedy`.
///
/// Mutation: have `Asked::joined` keep the first question as it is. The order
/// with `*p` first fails. Mutation: have it keep the second, `through_memory:
/// other`. The order with `**pp` first fails.
#[test]
fn a_doubt_through_memory_beside_a_plain_one_at_one_caret_keeps_its_remedy() {
    for through_memory_first in [false, true] {
        let (_sources, names) = sources();
        let (mut unit, _, int) = a_unit(&names, &[]);
        let pointer = unit.push_type(Ty::Pointer(int));
        let pointer_to_pointer = unit.push_type(Ty::Pointer(pointer));
        let mut function = Function::new(names.function, int, [pointer, pointer_to_pointer]);
        let mut parameters = function.parameters();
        let p = parameters.next().expect("two parameters");
        let pp = parameters.next().expect("two parameters");

        let plain = write_through(p, names.at[0]);
        let through_memory = Element::Assign(Operation {
            place: Place {
                local: pp,
                projection: vec![Projection::Deref, Projection::Deref],
            },
            value: Rvalue::Use(Operand::Constant(1)),
            origin: Origin::Written(names.at[0]),
        });
        let elements = if through_memory_first {
            vec![through_memory, plain]
        } else {
            vec![plain, through_memory]
        };

        let entry = function.reserve_block();
        function.fill_block(
            entry,
            Block {
                elements,
                terminator: Terminator::Return,
            },
        );

        let found = concluded(unit, function);

        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].at, names.at[0]);
        assert_eq!(found[0].conclusion, Conclusion::Unknown);
        assert_eq!(
            found[0].asked,
            nullability::Asked::Dereference {
                through_memory: true
            },
            "the remedy that settles both, through memory first: {through_memory_first}"
        );
    }
}

/// A call that passes nothing for a `_Nonnull` parameter is not proved: the
/// parameter holds nothing anybody chose, and the body believes it all the
/// same (C17 6.5.2.2 p6).
///
/// **This compiler's own frontend no longer produces it.** A call is type
/// checked against the prototype the translation unit ends with, so `void
/// g(); void h(void) { g(); } void g(int * _Nonnull p);` is too few arguments
/// before anything is lowered. A frontend that checks a call against the
/// declaration in scope at it, as C does, lowers that call, and so would the
/// Clang adapter.
///
/// Mutation: zip the arguments with the parameters in
/// `nullability::report_arguments`; nothing is concluded and this fails.
#[test]
fn a_nonnull_parameter_a_call_passes_no_argument_for_is_not_proved() {
    let (_sources, names) = sources();

    let mut unit = TranslationUnit::new(
        Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
    );
    let int = unit.push_type(Ty::Int);
    let pointer = unit.push_type(Ty::Pointer(int));
    let callee = unit.push_function(Function::declaration_with_parameters(
        names.function,
        int,
        [Parameter {
            ty: pointer,
            nonnull: Some(Promise::Declared(names.at[1])),
        }],
    ));
    let mut function = Function::new(names.function, int, vec![]);

    let entry = function.reserve_block();
    let after = function.reserve_block();
    function.fill_block(
        entry,
        Block {
            elements: vec![],
            terminator: Terminator::Call {
                callee,
                arguments: vec![],
                destination: None,
                then: Some(after),
                origin: Origin::Written(names.at[0]),
            },
        },
    );
    function.fill_block(after, returns());

    let found = concluded(unit, function);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].conclusion, Conclusion::Unknown);
    assert_eq!(found[0].at, names.at[0]);
}
