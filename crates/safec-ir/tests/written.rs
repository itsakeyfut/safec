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
    /// One step per local. A local's bit starts set or clear and the
    /// intersection only ever clears it, so it falls at most once.
    fn height(&self, _function: &Function) -> usize {
        self.0
    }

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
    /// Any answer here is wrong, because this lattice has no top and no number
    /// bounds a walk with no end. It answers the locals, which is what an
    /// author who had not noticed would answer, and what the solver does about
    /// that is the test below.
    fn height(&self, _function: &Function) -> usize {
        self.0
    }

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
/// **The expectation is the whole message rather than a phrase from it**, so
/// every part is held: the analysis's name, both causes and what to do about
/// each, whose defect it is, and the three numbers. It was a phrase, and the
/// three numbers were then the only part of a diagnostic nothing read, so
/// adding one to the block index, to the visit count or to the height left the
/// suite green. Mutation: any of those three, or reversing the sentence that
/// says whose defect it is. Each fails here, and nothing else in the suite
/// reads this message at all.
#[test]
#[should_panic(
    expected = "the `written::Climbing` analysis did not converge. Either its lattice is taller than it said, in which case raise what `Analysis::height` answers, or it has no top at all, in which case this is a defect in safec rather than in the code being compiled and is worth reporting. A join that rebuilds its value into a different shape with the same meaning is the usual way to have no top by accident.\n\nBlock 1 was walked 3 times, and `height` answered 1."
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

/// Four states per local rather than one: every local counts down from four,
/// and the join keeps the lower. Its lattice is four times as tall as its
/// locals, which is what more than one state per key looks like and why no
/// formula over the function could have answered for it.
struct Counting(usize);

impl Analysis for Counting {
    type Value = Vec<u8>;

    /// Four steps for every local but the return place, which no arm counts
    /// down. Exact rather than generous, so that the one the solver adds to it
    /// is the difference between this passing and being stopped, which makes
    /// this the only test in the suite holding that `+ 1`.
    fn height(&self, function: &Function) -> usize {
        (function.locals().len() - 1) * 4
    }

    fn on_entry(&self) -> Self::Value {
        vec![4; self.0]
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

/// An analysis that says how tall it is is believed.
///
/// Sixty four locals, one loop arm each, and a value that takes four steps
/// down per local. A block is walked once per step its value takes, so this
/// walks the header about four times as many times as there are locals.
///
/// Mutation: delete `Counting::height`. That is `error[E0046]` rather than a
/// failing test, because the trait carries no default, and a reversal nobody
/// can compile is the row above one somebody has to remember to run.
///
/// Mutation: have `solve` answer the locals plus the elements itself, which is
/// the default this trait used to carry and which was taken away for being
/// wrong about exactly this shape. The walk is stopped and a correct analysis
/// is reported as broken.
///
/// Mutation: drop the one the solver adds to the declared height. This
/// analysis declares exactly what it needs, so the walk is one visit taller
/// than its height and that one is what lets it finish. Nothing else in the
/// suite declares tightly enough to notice.
///
/// Mutation: count the walks across every block rather than per block. This
/// fails too, along with every other test whose function has enough blocks for
/// the two counts to come apart. Four of them, when it was measured, and the
/// number is not the claim.
#[test]
fn an_analysis_that_says_how_tall_it_is_is_believed() {
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
    assert_eq!(at_exit[0], 4);
    assert!(at_exit[1..].iter().all(|&left| left == 0), "{at_exit:?}");
}

/// Reaching definitions: which assignment sites may have written, keyed on the
/// site rather than on the local. Union, monotone, finite, and its height is
/// the number of sites, which has nothing to do with the number of locals.
///
/// `Place`'s own doc comment says an analysis keys its lattice on a place
/// rather than on a local, and this is the shape that says why no formula over
/// the function answers for every analysis.
struct Reaching(usize);

impl Analysis for Reaching {
    type Value = Vec<bool>;
    /// One step per site. A site's bit is set once and the union never clears
    /// it, so the value rises once per site and not at all per local, which is
    /// the whole reason this analysis is in the suite.
    fn height(&self, _function: &Function) -> usize {
        self.0
    }

    fn on_entry(&self) -> Self::Value {
        vec![false; self.0]
    }

    fn join(&self, into: &mut Self::Value, from: &Self::Value) {
        for (here, there) in into.iter_mut().zip(from) {
            *here = *here || *there;
        }
    }

    fn element(&self, _function: &Function, element: &Element, value: &mut Self::Value) {
        if let Element::Assign(operation) = element {
            value[operation.origin.span().start() as usize] = true;
        }
    }

    fn terminator(&self, _function: &Function, _terminator: &Terminator, _value: &mut Self::Value) {
    }
}

/// An analysis keyed on something the function has more of than locals is not
/// stopped.
///
/// One local, two hundred sites that assign to it, each in its own loop arm.
/// The walk needs one visit per site and the function has two locals, so any
/// budget counting locals stops it on its third pass.
///
/// Mutation: have `Reaching::height` answer the locals rather than the sites.
/// The budget becomes three and a correct analysis is stopped as though it
/// were broken. This is the only test that notices, because every other one
/// keys on locals.
#[test]
fn an_analysis_that_keys_on_more_than_its_locals_is_not_stopped() {
    const SITES: usize = 200;

    let mut sources = SourceMap::new();
    let text = "a".repeat(SITES + 8);
    let file = sources.add_virtual("t.c", &text);
    let at = Span::new(file, 0, 1);
    let mut unit = TranslationUnit::new(
        Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
    );
    let int = unit.push_type(Ty::Int);
    let mut function = Function::new(at, int, []);
    let a = function.push_local(int);

    let entry = function.reserve_block();
    let header = function.reserve_block();
    let picks: Vec<_> = (0..SITES).map(|_| function.reserve_block()).collect();
    let arms: Vec<_> = (0..SITES).map(|_| function.reserve_block()).collect();
    let exit = function.reserve_block();

    function.fill_block(entry, goto(header, vec![]));
    function.fill_block(header, goto(picks[0], vec![]));
    for site in 0..SITES {
        let otherwise = if site + 1 < SITES {
            picks[site + 1]
        } else {
            exit
        };
        function.fill_block(
            picks[site],
            Block {
                elements: vec![],
                terminator: Terminator::Branch {
                    condition: Operand::Constant(1),
                    then: arms[site],
                    otherwise,
                },
            },
        );
        function.fill_block(
            arms[site],
            goto(
                header,
                vec![Element::Assign(Operation {
                    place: Place::local(a),
                    value: Rvalue::Use(Operand::Constant(1)),
                    origin: Origin::Written(Span::new(file, site as u32, site as u32 + 1)),
                })],
            ),
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
    let solution = solve(&Reaching(text.len()), &function, &cfg);

    // Every site reaches the exit: each is on a path that leaves the loop.
    let at_exit = solution.value(exit).expect("the exit is reachable");
    assert_eq!(at_exit.iter().filter(|&&reached| reached).count(), SITES);
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
