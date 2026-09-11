//! Walking every path at once, until the answer stops changing.
//!
//! [`crate::cfg`] works out the graph; this runs something over it. An analysis
//! says what it knows at the start of a function, what an element and a
//! terminator do to that, and how two answers arriving at one block become one.
//! [`solve`] does the rest, which is a worklist and nothing more.
//!
//! Every analysis `docs/roadmap.md` puts in Phases 5 through 8 is this shape,
//! which is why it is here once rather than four times slightly differently.
//! See [ADR-0016] for what was chosen and what was left out: backward
//! analyses, a transfer that depends on which edge was taken, and a value per
//! program point are all real and none of them is asked for yet.
//!
//! **Nothing here reports.** `docs/roadmap.md` makes Phase 5 the first phase
//! that says anything about a C program. Nothing in this module builds a
//! [`crate::analysis::Conclusion`], and it could not build a diagnostic even if
//! it wanted to: this crate cannot see one, which is ADR-0011. A framework that
//! could report would be one Phase 5 could not be the first to use.
//!
//! **Termination belongs to the analysis, and nothing here can check it.** The
//! walk ends when no [`Analysis::join`] answers that anything changed, so a
//! lattice whose values can keep rising forever does not end at all, and the
//! failure is a hang rather than a diagnostic. Bounding it means choosing a
//! widening, and a widening chosen before any analysis needs one is a guess.
//! ADR-0016 carries this as a consequence rather than leaving it unsaid.
//!
//! [ADR-0016]: https://github.com/itsakeyfut/safec/blob/main/docs/adr/0016-an-analysis-is-a-trait-and-an-unreached-block-has-no-value.md

use crate::cfg::Cfg;
use crate::ir::{BlockId, Element, Function, Terminator};

/// What an analysis is: a value, a join, and what the program does to it.
///
/// Implementing this is the whole of writing one. There is no fifth thing to
/// be right about, which is deliberate: a method added later without a default
/// is `error[E0046]` at every implementation, so what an analysis owes is held
/// by the compiler rather than by a convention somebody remembers. See
/// ADR-0016.
///
/// **There is no bottom.** A block nothing has reached yet holds no value at
/// all, so the first answer to arrive is kept as it is and only the second is
/// joined. Nothing therefore needs a value that joins as an identity, which
/// would be a law each analysis had to obey and no test of this module could
/// check. ADR-0016 is where that was argued.
pub trait Analysis {
    /// What is known at one point in the program.
    ///
    /// A value rather than a fact, because half of these will be
    /// over-approximations: [`crate::analysis::Conclusion`] uses "fact" for
    /// something proven, "unlike a suspicion", and a may-analysis carries
    /// exactly the suspicion.
    ///
    /// [`Clone`] because the solver hands a copy of a block's value to the
    /// transfer rather than editing the answer in place: a block is walked
    /// again every time something arriving at it changes, and a transfer that
    /// consumed the stored value would have nothing to walk from the second
    /// time.
    type Value: Clone;

    /// What holds where the function starts.
    ///
    /// Not "nothing is known": a check that asks whether a local has been
    /// written starts with the parameters written and the rest not, and a
    /// function's entry is the one place that can say so.
    fn on_entry(&self) -> Self::Value;

    /// Fold `from` into `into`, and answer whether `into` moved.
    ///
    /// **The answer is what ends the walk.** A join that says nothing changed
    /// is how the solver learns it has nothing left to do, so one that always
    /// answers `false` stops the fixpoint after a single pass and one that
    /// always answers `true` never stops at all.
    fn join(&self, into: &mut Self::Value, from: &Self::Value) -> bool;

    /// What one element of a block does to what is known.
    ///
    /// The function is handed over rather than held, so that an analysis asking
    /// `TranslationUnit::place_ty` what a place is does not have to capture a
    /// [`Function`] of its own. One that did could be handed to a [`solve`]
    /// over a different function: that compiles, and answers about neither.
    fn element(&self, function: &Function, element: &Element, value: &mut Self::Value);

    /// What the end of a block does to what is known.
    ///
    /// The same value then reaches every successor. Saying something different
    /// on the taken and untaken arms of a branch is what a null check wants,
    /// and `docs/roadmap.md` puts nullability in Phase 5, which is the next
    /// one. It is not built: ADR-0016 says what it would cost, and it was
    /// measured at a defaulted method and three lines of the solver.
    fn terminator(&self, function: &Function, terminator: &Terminator, value: &mut Self::Value);
}

/// What an analysis concluded, one value per block.
///
/// Built by [`solve`] and read by whatever asked for the analysis.
///
/// **A point inside a block is reached by replaying it.** The caller clones a
/// block's value and walks it through [`Analysis::element`] and
/// [`Analysis::terminator`] itself, which is how a check says *here* rather
/// than *somewhere in this block*. That works only because those take `&self`
/// and are reachable by whoever asked, so hiding them inside the solver would
/// take the per-point answer away from every check without failing a test.
#[derive(Debug)]
pub struct Solution<V> {
    /// What holds where each block starts, indexed by [`BlockId::index`].
    ///
    /// `None` is a block the entry cannot reach. It is also what a block holds
    /// before anything has arrived at it, and the two cannot be confused once
    /// the walk is over: every reachable block is arrived at, because the walk
    /// starts at the entry and follows the same edges that made it reachable.
    values: Vec<Option<V>>,
}

impl<V> Solution<V> {
    /// What holds where this block starts, or nothing if the entry cannot
    /// reach it.
    ///
    /// Not spelled `entry`, which [`Function::entry`] already uses for the
    /// block a function starts at: the two would be one word for a block and
    /// for a value, taking opposite arguments.
    ///
    /// **An answer rather than a value that stands for one.** A block that
    /// never runs and a block that runs with nothing known are different
    /// things, and handing back the second for the first would let a check
    /// conclude about code no execution reaches. [`Cfg`] keeps unreachable
    /// blocks out of its predecessors for the same reason.
    ///
    /// **Unreachable here is a claim about the graph, not about executions.**
    /// Nothing arrives at this block along an edge the IR holds. A handler that
    /// a `longjmp` or an unwinding call would reach is such a block today,
    /// because [`Terminator::Abnormal`] exists and nothing builds one and a
    /// call has no edge for the callee not returning. A check that reads `None`
    /// as "nobody runs this" will be silent about that code on the day the
    /// frontend lowers it, which is ADR-0010's whole subject.
    pub fn value(&self, block: BlockId) -> Option<&V> {
        self.values[block.index()].as_ref()
    }
}

/// Run `analysis` over `function` until nothing changes.
///
/// The `Cfg` is taken rather than built, so that a caller holding one does not
/// walk the graph twice and so that the graph this runs over is visibly the one
/// the caller asked about. Only its order is read: this pushes a value forward
/// along the edges rather than pulling one back from each predecessor, and a
/// successor of a reachable block is reachable, so there is nothing here to
/// filter.
///
/// A block is walked from its entry value: every element in the order it is
/// written, then the terminator, and the result is joined into every successor.
/// A successor whose value moved goes back on the list, which is what makes a
/// loop converge rather than being answered from whatever reached it first.
///
/// # Panics
///
/// If `function` is a declaration, which is [`Function::blocks`]'s panic, or if
/// a block the entry **reaches** was reserved and never filled, which is
/// [`Function::block`]'s. Neither is invented here, and [`Cfg::of`] has already
/// made both by the time this is called: a fixpoint over a function nobody
/// finished building would answer about a program that does not exist yet.
///
/// **A hole the entry cannot reach is not one of those**, and it was measured:
/// neither the walk nor this touches such a block, so it is answered `None`,
/// which is what an unreachable block is answered anyway. A caller that
/// reserved a block and forgot to fill it learns nothing from that. Noticing
/// would mean walking every block to look for a hole, on every function, for a
/// mistake the compiler cannot make on its own: [`Function::fill_block`] is the
/// only way a reserved id becomes a block, and outside the tests the only
/// caller that reserves one is `safec`'s lowering.
pub fn solve<A: Analysis>(analysis: &A, function: &Function, cfg: &Cfg) -> Solution<A::Value> {
    let mut values: Vec<Option<A::Value>> = (0..function.blocks().len()).map(|_| None).collect();
    values[function.entry().index()] = Some(analysis.on_entry());

    // Reversed, because this is popped from the back: the first pass then comes
    // out in the order `Cfg` produced, which visits a block after the blocks
    // that reach it wherever the graph allows one. That is not needed for the
    // answer to be right, only for it to be reached in fewer passes.
    let mut worklist: Vec<BlockId> = cfg.order().iter().rev().copied().collect();
    let mut successors = Vec::new();

    while let Some(block) = worklist.pop() {
        // A block can be taken off the list before anything has reached it:
        // the list starts as every reachable block, and the order only puts a
        // block after the blocks that reach it where the graph allows one. It
        // is safe to drop it here because the first arrival counts as a change
        // and pushes it back on, so nothing is lost by walking it later with a
        // value rather than now without one.
        //
        // Skipping rather than seeding is also what keeps `None` meaning
        // "the entry cannot reach this" in the answer rather than "not yet".
        let Some(mut value) = values[block.index()].clone() else {
            continue;
        };

        for element in &function.block(block).elements {
            analysis.element(function, element, &mut value);
        }
        analysis.terminator(function, &function.block(block).terminator, &mut value);

        successors.clear();
        function.block(block).terminator.successors(&mut successors);

        for &successor in &successors {
            let changed = match &mut values[successor.index()] {
                Some(arrived) => analysis.join(arrived, &value),
                // The first answer to arrive is kept rather than joined, which
                // is what lets an analysis have no bottom. See ADR-0016.
                nothing @ None => {
                    *nothing = Some(value.clone());
                    true
                }
            };

            // Already on the list is already going to be walked again, and a
            // block walked twice for one change is a pass wasted rather than a
            // wrong answer. The list is one function wide.
            if changed && !worklist.contains(&successor) {
                worklist.push(successor);
            }
        }
    }

    Solution { values }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{
        Block, FuncId, LocalId, Operand, Operation, Origin, Place, Rvalue, TranslationUnit, Ty,
        TyId,
    };
    use crate::source::{SourceMap, Span};
    use crate::target::Target;

    /// Which locals have been written on **every** path to here.
    ///
    /// The trivial analysis the issue asks for, defined here rather than in the
    /// crate because nothing outside a test wants it: #107 builds the real one.
    /// A must-analysis, so the join is intersection, and that is what makes a
    /// branch and a back edge able to take a write away again.
    ///
    /// **Keyed by the local, not by the place.** `Place`'s own doc comment says
    /// a real analysis keys on a place, because `p` and `*p` have separate
    /// states. Nothing here writes through a projection, and a test analysis
    /// that did would be testing itself rather than the solver.
    struct Written(usize);

    impl Analysis for Written {
        type Value = Vec<bool>;

        fn on_entry(&self) -> Self::Value {
            vec![false; self.0]
        }

        fn join(&self, into: &mut Self::Value, from: &Self::Value) -> bool {
            let mut changed = false;
            for (here, there) in into.iter_mut().zip(from) {
                let both = *here && *there;
                changed |= both != *here;
                *here = both;
            }
            changed
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

        fn terminator(
            &self,
            _function: &Function,
            terminator: &Terminator,
            value: &mut Self::Value,
        ) {
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
    /// fails. Mutation: `join` answers `false` whatever it did. The worklist
    /// empties after the first pass and this fails the same way. Both were
    /// applied and no other test in the crate fails under either.
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
    /// dropped from the walk and nothing in the crate would say so, because
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
}
