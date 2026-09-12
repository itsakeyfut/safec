//! The framework's only user that is not the framework itself.
//!
//! `safec_ir::dataflow::Analysis` is a trait meant to be implemented outside
//! the crate that defines it, and nothing established that it could be while
//! its only implementation sat inside the module it tested, where every
//! private item is in scope. An integration test is compiled against the crate
//! as a dependency, so this file reaches only what is public: the day
//! something an analysis needs stops being public, this stops compiling, and
//! that is the claim the file exists to hold.
//!
//! Measured, because a claim like that is worth nothing unstated: spelling
//! `pub mod dataflow;` as `mod dataflow;` in `lib.rs` makes this file
//! `error[E0603]`, while the library itself says only that a trait is never
//! used. A warning is what the loss looks like from inside; the error is what
//! it looks like from where an analysis will actually be written.
//!
//! What it implements is the smallest analysis worth having, which locals have
//! been written on every path to a point. Small enough that a reader can work
//! out every expectation below by hand, which is why it was chosen, and it
//! reports nothing, because `docs/roadmap.md` makes Phase 5 the first phase
//! that says anything about a C program.
//!
//! **The solver's own guards live here too, and that is a departure.**
//! `ir.rs`, `cfg.rs` and `interp.rs` all keep their tests beside the code, and
//! this cannot: `Written` is the only analysis there is, a `#[cfg(test)]` item
//! is invisible from an integration test, and publishing an analysis with no
//! caller is the shape `CLAUDE.md` rejects. So either the fixture exists twice
//! and one copy is fixed without the other, or every test moves. They moved.

use safec_ir::cfg::Cfg;
use safec_ir::dataflow::{Analysis, solve};
use safec_ir::ir::{
    Block, BlockId, Element, FuncId, Function, LocalId, Operand, Operation, Origin, Place, Rvalue,
    Terminator, TranslationUnit, Ty, TyId,
};
use safec_ir::source::{SourceMap, Span};
use safec_ir::target::Target;

/// Which locals have been written on **every** path to here.
///
/// A must-analysis, so the join is intersection, and that is what makes a
/// branch and a back edge able to take a write away again: the two shapes
/// most of the tests below turn on.
///
/// **Keyed by the local, not by the place.** `Place`'s own doc comment says a
/// real analysis keys its lattice on a place, because `p` and `*p` have
/// separate states. That is right about the memory and ownership questions and
/// not about this one: writing through `*p` writes what `p` points at and not
/// `p`, so having been written is a property of the local. Nothing here writes
/// through a projection either way.
struct Written(usize);

impl Analysis for Written {
    type Value = Vec<bool>;

    fn on_entry(&self) -> Self::Value {
        vec![false; self.0]
    }

    fn join(&self, into: &mut Self::Value, from: &Self::Value) {
        for (here, there) in into.iter_mut().zip(from) {
            *here = *here && *there;
        }
    }

    fn element(&self, _function: &Function, element: &Element, value: &mut Self::Value) {
        // Every field written out, never `..`: RK-018 in the review
        // knowledge bank is a field added to a variant that already exists
        // walking past an exhaustive match. This is the file Phase 5 will
        // copy from.
        match element {
            Element::Assign(operation) => value[operation.place.local.index()] = true,
            Element::StorageLive { local, origin: _ } => value[local.index()] = false,
            Element::StorageDead { origin: _, local } => value[local.index()] = false,
        }
    }

    fn terminator(&self, _function: &Function, terminator: &Terminator, value: &mut Self::Value) {
        match terminator {
            Terminator::Call {
                callee: _,
                arguments: _,
                destination,
                then: _,
                origin: _,
            } => {
                if let Some(place) = destination {
                    value[place.local.index()] = true;
                }
            }
            Terminator::Goto(_)
            | Terminator::Branch {
                condition: _,
                then: _,
                otherwise: _,
            }
            | Terminator::Return
            | Terminator::Abnormal { to: _ } => {}
        }
    }
}

fn spans() -> (SourceMap, Span) {
    let mut sources = SourceMap::new();
    let file = sources.add_virtual("t.c", "int f(void) { return 1; }\n");
    let span = Span::new(file, 14, 22);
    (sources, span)
}

/// A unit with one function in it, and that function.
///
/// The same helper `cfg.rs` has, for the same reason: a `FuncId` and a
/// `LocalId` cannot be spelled from outside `ir`, so a test that wants one
/// asks the thing that hands them out.
fn a_function(at: Span) -> (TranslationUnit, Function, TyId) {
    let mut unit = TranslationUnit::new(
        Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
    );
    let int = unit.push_type(Ty::Int);
    let function = Function::new(at, int, []);
    (unit, function, int)
}

/// Something for a `Call` to name.
fn callee(unit: &mut TranslationUnit, at: Span) -> FuncId {
    let void = unit.push_type(Ty::Void);
    unit.push_function(Function::declaration(at, void, []))
}

fn write(local: LocalId, origin: Origin) -> Element {
    Element::Assign(Operation {
        place: Place::local(local),
        value: Rvalue::Use(Operand::Constant(1)),
        origin,
    })
}

fn goto(to: BlockId, elements: Vec<Element>) -> Block {
    Block {
        elements,
        terminator: Terminator::Goto(to),
    }
}

/// `a` is written before a loop and `b` only inside it, so after the loop
/// `a` holds on every path and `b` does not: the path that skips the body
/// never writes it.
///
/// Mutation: never run `element`. `a` is answered unwritten and this fails.
/// Mutation: in `solve`, join the first arrival at a block into a fresh
/// `analysis.on_entry()` rather than storing it. `a` is intersected against
/// a value nothing produced, is answered unwritten, and this fails. That
/// second one is what ADR-0016's no-bottom decision costs to reverse.
///
/// **This is not the fixpoint's guard**, which was measured rather than
/// assumed: cutting the solver to one visit per block leaves it passing,
/// because a must-analysis's back edge can only take writes away and this
/// loop only adds one. `the_back_edge_changes_the_answer_after_the_loop` is
/// the guard.
#[test]
fn what_a_loop_writes_and_what_comes_before_it_are_answered_apart() {
    let (_sources, at) = spans();
    let (_unit, mut function, int) = a_function(at);
    let origin = Origin::Written(at);
    let a = function.push_local(int);
    let b = function.push_local(int);

    // The entry is whichever block was handed id 0, so it is reserved
    // first and filled once the blocks it names exist.
    let entry = function.reserve_block();
    let header = function.reserve_block();
    let body = function.reserve_block();
    let exit = function.reserve_block();
    assert_eq!(entry, function.entry());

    function.fill_block(entry, goto(header, vec![write(a, origin)]));
    function.fill_block(
        header,
        Block {
            elements: vec![],
            terminator: Terminator::Branch {
                condition: Operand::Constant(1),
                then: body,
                otherwise: exit,
            },
        },
    );
    function.fill_block(body, goto(header, vec![write(b, origin)]));
    function.fill_block(
        exit,
        Block {
            elements: vec![],
            terminator: Terminator::Return,
        },
    );

    let cfg = Cfg::of(&function);
    let solution = solve(&Written(function.locals().len()), &function, &cfg);

    // Local 0 is the return place, then `a`, then `b`.
    assert_eq!(solution.value(exit), Some(&vec![false, true, false]));
    assert_eq!(solution.value(body), Some(&vec![false, true, false]));
}

/// A local written before a loop whose storage ends inside it is not
/// written after the loop, which only the second time round the header can
/// say.
///
/// **The guard on the fixpoint itself.** Mutation: drop the re-push of a
/// successor whose value changed, so each block is visited once. The header
/// keeps what the entry gave it, the exit is answered written, and this
/// fails. Mutation: in `solve`, answer that nothing moved however the
/// comparison came out. The worklist empties after the first pass and this
/// fails the same way. Both were applied and no other test in the suite
/// fails under either.
#[test]
fn the_back_edge_changes_the_answer_after_the_loop() {
    let (_sources, at) = spans();
    let (_unit, mut function, int) = a_function(at);
    let origin = Origin::Written(at);
    let a = function.push_local(int);

    let entry = function.reserve_block();
    let header = function.reserve_block();
    let body = function.reserve_block();
    let exit = function.reserve_block();

    function.fill_block(entry, goto(header, vec![write(a, origin)]));
    function.fill_block(
        header,
        Block {
            elements: vec![],
            terminator: Terminator::Branch {
                condition: Operand::Constant(1),
                then: body,
                otherwise: exit,
            },
        },
    );
    function.fill_block(
        body,
        goto(header, vec![Element::StorageDead { origin, local: a }]),
    );
    function.fill_block(
        exit,
        Block {
            elements: vec![],
            terminator: Terminator::Return,
        },
    );

    let cfg = Cfg::of(&function);
    let solution = solve(&Written(function.locals().len()), &function, &cfg);

    assert_eq!(solution.value(exit), Some(&vec![false, false]));
}

/// A local written on one arm of a branch and not the other is not written
/// where the arms meet.
///
/// Mutation: join into the first successor only. The meeting block is
/// answered from the arm that writes, says written, and this fails.
#[test]
fn a_local_written_on_one_arm_of_a_branch_is_not_written_after_it() {
    let (_sources, at) = spans();
    let (_unit, mut function, int) = a_function(at);
    let origin = Origin::Written(at);
    let a = function.push_local(int);

    let entry = function.reserve_block();
    let taken = function.reserve_block();
    let untaken = function.reserve_block();
    let after = function.reserve_block();

    function.fill_block(
        entry,
        Block {
            elements: vec![],
            terminator: Terminator::Branch {
                condition: Operand::Constant(1),
                then: taken,
                otherwise: untaken,
            },
        },
    );
    function.fill_block(taken, goto(after, vec![write(a, origin)]));
    function.fill_block(untaken, goto(after, vec![]));
    function.fill_block(
        after,
        Block {
            elements: vec![],
            terminator: Terminator::Return,
        },
    );

    let cfg = Cfg::of(&function);
    let solution = solve(&Written(function.locals().len()), &function, &cfg);

    assert_eq!(solution.value(taken), Some(&vec![false, false]));
    assert_eq!(solution.value(after), Some(&vec![false, false]));
}

/// A call writes the place it names, so what a terminator does reaches the
/// block after it.
///
/// Mutation: never run `terminator`. The call's destination is answered
/// unwritten and this fails. Without this test that method could be
/// dropped from the walk and nothing in the suite would say so, because
/// every other terminator this analysis sees does nothing.
#[test]
fn a_local_a_call_writes_is_written_after_it() {
    let (_sources, at) = spans();
    let (mut unit, mut function, int) = a_function(at);
    let origin = Origin::Written(at);
    let a = function.push_local(int);
    let called = callee(&mut unit, at);

    let entry = function.reserve_block();
    let after = function.reserve_block();

    function.fill_block(
        entry,
        Block {
            elements: vec![],
            terminator: Terminator::Call {
                callee: called,
                arguments: vec![],
                destination: Some(Place::local(a)),
                then: after,
                origin,
            },
        },
    );
    function.fill_block(
        after,
        Block {
            elements: vec![],
            terminator: Terminator::Return,
        },
    );

    let cfg = Cfg::of(&function);
    let solution = solve(&Written(function.locals().len()), &function, &cfg);

    assert_eq!(solution.value(after), Some(&vec![false, true]));
}

/// The elements of a block happen in the order they are written in, which
/// every analysis rests on: `p = malloc(); free(p);` is one block, and a
/// walk that read it the other way round would answer that `p` is live.
///
/// Mutation: run a block's elements in reverse. `b`, whose storage ends
/// before it is written, is answered unwritten and this fails. It was
/// measured after the rest of them were written, and nothing failed under
/// it then, which is why it exists;
/// `storage_beginning_leaves_a_local_holding_nothing` came later and now
/// fails under it too, because it also depends on which of two elements
/// happens second.
#[test]
fn the_elements_of_a_block_happen_in_the_order_they_are_written() {
    let (_sources, at) = spans();
    let (_unit, mut function, int) = a_function(at);
    let origin = Origin::Written(at);
    let a = function.push_local(int);
    let b = function.push_local(int);

    let entry = function.reserve_block();
    let after = function.reserve_block();

    function.fill_block(
        entry,
        goto(
            after,
            vec![
                // Written, then its storage ends: not written afterwards.
                write(a, origin),
                Element::StorageDead { origin, local: a },
                // The same pair the other way round: written afterwards.
                Element::StorageDead { origin, local: b },
                write(b, origin),
            ],
        ),
    );
    function.fill_block(
        after,
        Block {
            elements: vec![],
            terminator: Terminator::Return,
        },
    );

    let cfg = Cfg::of(&function);
    let solution = solve(&Written(function.locals().len()), &function, &cfg);

    assert_eq!(solution.value(after), Some(&vec![false, false, true]));
}

/// Storage beginning leaves a local holding nothing, so a write before it
/// is not a write that survives it.
///
/// C17 6.2.4 p6 begins a local's lifetime at entry into its block, which is
/// why `StorageLive` is an event an analysis answers for rather than a note
/// about the declaration. Mutation: have `StorageLive` leave the local
/// written. This fails, and it is the only test that builds one.
#[test]
fn storage_beginning_leaves_a_local_holding_nothing() {
    let (_sources, at) = spans();
    let (_unit, mut function, int) = a_function(at);
    let origin = Origin::Written(at);
    let a = function.push_local(int);

    let entry = function.reserve_block();
    let after = function.reserve_block();

    function.fill_block(
        entry,
        goto(
            after,
            vec![write(a, origin), Element::StorageLive { local: a, origin }],
        ),
    );
    function.fill_block(
        after,
        Block {
            elements: vec![],
            terminator: Terminator::Return,
        },
    );

    let cfg = Cfg::of(&function);
    let solution = solve(&Written(function.locals().len()), &function, &cfg);

    assert_eq!(solution.value(after), Some(&vec![false, false]));
}

/// What a block sends reaches its own successors and nobody else's.
///
/// `solve` fills one buffer with each block's successors in turn, so this
/// is a guard on it being emptied first. Mutation: drop the
/// `successors.clear()`. The buffer keeps every earlier block's successors,
/// `killer`'s value joins into `reached` as well as into its own successor,
/// and `reached` is answered unwritten though the only way to it never
/// killed anything. This fails and nothing else does.
#[test]
fn what_a_block_sends_reaches_its_own_successors_and_no_others() {
    let (_sources, at) = spans();
    let (_unit, mut function, int) = a_function(at);
    let origin = Origin::Written(at);
    let a = function.push_local(int);

    let entry = function.reserve_block();
    let keeper = function.reserve_block();
    let killer = function.reserve_block();
    let reached = function.reserve_block();
    let elsewhere = function.reserve_block();
    let end = function.reserve_block();

    function.fill_block(
        entry,
        Block {
            elements: vec![write(a, origin)],
            terminator: Terminator::Branch {
                condition: Operand::Constant(1),
                then: keeper,
                otherwise: killer,
            },
        },
    );
    function.fill_block(keeper, goto(reached, vec![]));
    function.fill_block(
        killer,
        goto(elsewhere, vec![Element::StorageDead { origin, local: a }]),
    );
    function.fill_block(reached, goto(end, vec![]));
    function.fill_block(elsewhere, goto(end, vec![]));
    function.fill_block(
        end,
        Block {
            elements: vec![],
            terminator: Terminator::Return,
        },
    );

    let cfg = Cfg::of(&function);
    let solution = solve(&Written(function.locals().len()), &function, &cfg);

    // Only `keeper` reaches `reached`, and `keeper` kills nothing.
    assert_eq!(solution.value(reached), Some(&vec![false, true]));
    // Both arms reach `end`, and one of them killed it.
    assert_eq!(solution.value(end), Some(&vec![false, false]));
}

/// Reading a local is not writing it, and a local nothing writes is never
/// written.
///
/// `_1` is never assigned and is read once, as the operand of the operation
/// that writes `_2`. Afterwards `_2` has been written on every path and `_1`
/// has not.
///
/// Mutation: have `element` mark what an operation reads as well as where it
/// writes, by walking `operation.value` for a `Place`. `_1` is answered
/// written and this fails. It is the only test that reads a local at all:
/// every other operation in this file assigns a constant, so nothing else
/// could notice.
///
/// **The second half of the name is the weaker claim**, and it is here rather
/// than in a test of its own because no mutation distinguishes it: making
/// `on_entry` answer that every local is already written fails most of this
/// file, and a test nothing can uniquely break is weight rather than a guard.
#[test]
fn a_local_that_is_only_read_is_not_a_local_that_was_written() {
    let (_sources, at) = spans();
    let (_unit, mut function, int) = a_function(at);
    let origin = Origin::Written(at);
    let read = function.push_local(int);
    let written = function.push_local(int);

    let entry = function.reserve_block();
    let after = function.reserve_block();

    function.fill_block(
        entry,
        goto(
            after,
            vec![Element::Assign(Operation {
                place: Place::local(written),
                value: Rvalue::Use(Operand::Copy(Place::local(read))),
                origin,
            })],
        ),
    );
    function.fill_block(
        after,
        Block {
            elements: vec![],
            terminator: Terminator::Return,
        },
    );

    let cfg = Cfg::of(&function);
    let solution = solve(&Written(function.locals().len()), &function, &cfg);

    // The return place, then the one that is only read, then the one written.
    assert_eq!(solution.value(after), Some(&vec![false, false, true]));
}

/// A lattice with no top: the join takes the larger of the two and adds one,
/// so a value that meets itself keeps rising and no loop has a fixpoint.
///
/// ADR-0016 records that termination is the analysis's to answer for. This is
/// what the solver does when it is not answered.
struct Climbing(usize);

impl Analysis for Climbing {
    type Value = Vec<u32>;

    fn on_entry(&self) -> Self::Value {
        vec![0; self.0]
    }

    fn join(&self, into: &mut Self::Value, from: &Self::Value) {
        for (here, there) in into.iter_mut().zip(from) {
            *here = (*here).max(*there) + 1;
        }
    }

    fn element(&self, _function: &Function, _element: &Element, _value: &mut Self::Value) {}

    fn terminator(&self, _function: &Function, _terminator: &Terminator, _value: &mut Self::Value) {
    }
}

/// An analysis that cannot converge is stopped, and the message says whose
/// defect it is rather than blaming the code being compiled.
///
/// Mutation: remove the assertion in `solve`. This test **hangs** rather than
/// failing, which is the only signal a walk that never ends has, and is why a
/// suite run under that mutation has to be given a timeout to show anything at
/// all. `cfg.rs` says the same of its own visited check.
///
/// The expectation names this analysis, so dropping the `type_name` from the
/// message fails it too. What it does not hold is the sentence that says the
/// defect is safec's rather than the compiled code's, because a `should_panic`
/// expectation is one substring and that one is not next to this one. It is a
/// literal in the format string and a reader sees it in a diff.
#[test]
#[should_panic(
    expected = "`written::Climbing` analysis did not converge. This is a defect in safec rather than in the code being compiled"
)]
fn an_analysis_that_cannot_converge_is_stopped_and_named() {
    let (_sources, at) = spans();
    let (_unit, mut function, _int) = a_function(at);

    let entry = function.reserve_block();
    let header = function.reserve_block();
    let body = function.reserve_block();
    let exit = function.reserve_block();

    function.fill_block(entry, goto(header, vec![]));
    function.fill_block(
        header,
        Block {
            elements: vec![],
            terminator: Terminator::Branch {
                condition: Operand::Constant(1),
                then: body,
                otherwise: exit,
            },
        },
    );
    function.fill_block(body, goto(header, vec![]));
    function.fill_block(
        exit,
        Block {
            elements: vec![],
            terminator: Terminator::Return,
        },
    );

    let cfg = Cfg::of(&function);
    let _ = solve(&Climbing(function.locals().len()), &function, &cfg);
}

/// Two states per local rather than one: every local counts down from two, and
/// the join keeps the lower. Its lattice is twice as tall as `Written`'s over
/// the same function, which is what the budget's multiplier is slack for.
struct Counting(usize);

impl Analysis for Counting {
    type Value = Vec<u8>;

    fn on_entry(&self) -> Self::Value {
        vec![2; self.0]
    }

    fn join(&self, into: &mut Self::Value, from: &Self::Value) {
        for (here, there) in into.iter_mut().zip(from) {
            *here = (*here).min(*there);
        }
    }

    fn element(&self, _function: &Function, element: &Element, value: &mut Self::Value) {
        if let Element::StorageDead { origin: _, local } = element {
            value[local.index()] = value[local.index()].saturating_sub(1);
        }
    }

    fn terminator(&self, _function: &Function, _terminator: &Terminator, _value: &mut Self::Value) {
    }
}

/// A correct analysis over a large function is not stopped, even when its
/// lattice is taller than the function has locals.
///
/// Sixty four locals, one loop arm each, and a value that takes two steps down
/// per local rather than one. A block is walked once per step its value takes,
/// so this walks the header about twice as many times as there are locals.
///
/// Mutation: make `STATES_PER_KEY` one. The budget becomes the local count, the
/// walk needs twice that, and a correct analysis is stopped as though it were
/// broken. That is the whole reason the budget is a multiple rather than the
/// count itself, and this is the only test that notices.
///
/// Mutation: count the walks across every block rather than per block. This
/// fails too, and it is the only test that notices that either: the others are
/// small enough that the two counts stay under the budget together.
#[test]
fn a_correct_analysis_over_a_large_function_is_not_stopped() {
    let (_sources, at) = spans();
    let (_unit, mut function, int) = a_function(at);
    let origin = Origin::Written(at);
    let locals: Vec<_> = (0..64).map(|_| function.push_local(int)).collect();

    let entry = function.reserve_block();
    let header = function.reserve_block();
    let picks: Vec<_> = locals.iter().map(|_| function.reserve_block()).collect();
    let arms: Vec<_> = locals.iter().map(|_| function.reserve_block()).collect();
    let exit = function.reserve_block();

    function.fill_block(entry, goto(header, vec![]));
    function.fill_block(header, goto(picks[0], vec![]));
    for (i, &local) in locals.iter().enumerate() {
        let otherwise = if i + 1 < picks.len() {
            picks[i + 1]
        } else {
            exit
        };
        function.fill_block(
            picks[i],
            Block {
                elements: vec![],
                terminator: Terminator::Branch {
                    condition: Operand::Constant(1),
                    then: arms[i],
                    otherwise,
                },
            },
        );
        function.fill_block(
            arms[i],
            goto(header, vec![Element::StorageDead { origin, local }]),
        );
    }
    function.fill_block(
        exit,
        Block {
            elements: vec![],
            terminator: Terminator::Return,
        },
    );

    let cfg = Cfg::of(&function);
    let solution = solve(&Counting(function.locals().len()), &function, &cfg);

    // Every local is counted down to nothing on the way round, and the return
    // place, which no arm names, is untouched.
    let at_exit = solution.value(exit).expect("the exit is reachable");
    assert_eq!(at_exit[0], 2);
    assert!(at_exit[1..].iter().all(|&left| left == 0), "{at_exit:?}");
}

/// A block the entry cannot reach is answered with nothing, rather than
/// with a value that would let a check conclude about code no execution
/// reaches.
///
/// Mutation: seed every block with `Some(analysis.on_entry())` rather than
/// the entry alone. This fails, and so does every test whose answer depends
/// on a value having arrived rather than having been put there, because a
/// value waiting in a block also joins into whatever that block reaches.
#[test]
fn a_block_nothing_reaches_has_no_answer() {
    let (_sources, at) = spans();
    let (_unit, mut function, int) = a_function(at);
    let origin = Origin::Written(at);
    let a = function.push_local(int);

    let entry = function.reserve_block();
    let stranded = function.reserve_block();

    function.fill_block(
        entry,
        Block {
            elements: vec![],
            terminator: Terminator::Return,
        },
    );
    // `int f(void) { return 1; return 2; }` makes one of these.
    function.fill_block(
        stranded,
        Block {
            elements: vec![write(a, origin)],
            terminator: Terminator::Return,
        },
    );

    let cfg = Cfg::of(&function);
    let solution = solve(&Written(function.locals().len()), &function, &cfg);

    assert_eq!(solution.value(entry), Some(&vec![false, false]));
    assert_eq!(solution.value(stranded), None);
}
