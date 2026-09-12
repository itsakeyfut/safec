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
//! **Termination belongs to the analysis, and the solver only says when it
//! did not happen.** The walk ends when no [`Analysis::join`] moves a value,
//! so a lattice whose values can keep rising forever has no end to reach.
//! What this does about that is stop and say so: a budget per block, which
//! changes no answer any fixpoint reaches and decides only what happens when
//! there is no fixpoint. Choosing a widening, which would change an answer so
//! that it converges, is a different thing and is still not done. ADR-0016
//! carries both.
//!
//! **The tests are in `crates/safec-ir/tests/written.rs`, not beside this.**
//! They implement an analysis against this trait and are compiled against the
//! crate as a dependency, so they reach only what is public, which is the one
//! thing a test in this file could not establish: that an analysis can be
//! written from outside. A reader who finds no `#[cfg(test)]` module here
//! should not conclude there is nothing guarding this.
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
    ///
    /// [`Eq`] because the solver decides whether anything moved by comparing,
    /// rather than by asking the join. That is what removes the one law this
    /// framework used to rest on, and it puts a smaller one in its place:
    /// **the representation has to be canonical.** A join that rebuilds a
    /// value into a different shape with the same meaning never compares
    /// equal and never converges, and the ordinary way to write that is a
    /// union collected through a `HashSet` and back into a `Vec`. Measured,
    /// on eight sites meeting inside a loop: three lines of unremarkable
    /// Rust, and the walk does not end. See ADR-0016.
    ///
    /// [`Eq`] rather than [`PartialEq`], because the walk ends when a value
    /// compares equal to itself and `PartialEq` does not promise that.
    /// Measured: a value holding an `f64`, with a join that does nothing at
    /// all, never terminates, because `NAN != NAN`. Under the shape this
    /// replaced it did terminate, so the bound is what stops the trade this
    /// signature made from costing a whole class of value its answer.
    ///
    /// **A value that carries a span is where the walk stops terminating.**
    /// `docs/safety-model.md` asks a diagnostic to say "p freed here", so the
    /// first value a real check carries will hold one, and a span is not a
    /// lattice element: a `join` that keeps whichever one arrived oscillates
    /// forever where two of them meet below a branch inside a loop. Measured,
    /// and it is a hang with no diagnostic and no stack rather than a failure.
    /// Choose the payload by a rule that cannot depend on which side arrived,
    /// and keep it inside the comparison rather than outside: hiding it from
    /// `eq` ends the walk and is the failure [`Analysis::join`] describes.
    /// ADR-0016 is where it is recorded, and why nothing here bounds the walk
    /// instead.
    type Value: Clone + Eq;

    /// How many steps a value of this analysis can take up its lattice.
    ///
    /// The walk is stopped when a block is visited more than this, because a
    /// walk that does not end has nothing to read. Answering too low stops a
    /// correct analysis; answering too high makes a broken one take longer to
    /// stop. Both are the same panic, and it names this method.
    ///
    /// The default is the locals plus the elements, which covers an analysis
    /// keyed on a local, on a place, or on the site an assignment makes: all
    /// three were measured against it. An analysis whose value has more than
    /// one state per key says so here rather than leaning on it.
    fn height(&self, function: &Function) -> usize {
        function.locals().len()
            + function
                .blocks()
                .map(|block| block.elements.len())
                .sum::<usize>()
    }

    /// What holds where the function starts.
    ///
    /// Not "nothing is known": a check that asks whether a local has been
    /// written starts with the parameters written and the rest not, and a
    /// function's entry is the one place that can say so.
    fn on_entry(&self) -> Self::Value;

    /// Fold `from` into `into`.
    ///
    /// **It does not answer whether anything moved.** The solver compares the
    /// value against what it was, so a join that folded correctly and
    /// reported that it had not is a mutation nobody can write.
    ///
    /// **The answer moved rather than went away**, and it is now
    /// [`PartialEq::eq`]. An `eq` that ignores part of what this writes gives
    /// the old failure back: measured with a value holding a lattice element
    /// and the span a diagnostic would quote, where comparing only the element
    /// leaves the stored span decided by whichever predecessor arrived last
    /// and the solver never looks again. Whatever `join` writes, `eq` has to
    /// see. See ADR-0016.
    fn join(&self, into: &mut Self::Value, from: &Self::Value);

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

    // One more than the analysis said it needs, because a block is walked
    // once more than its value moves: the first walk is what puts a value
    // there. Measured, on an analysis keyed on locals with one state each.
    //
    // Not behind `debug_assertions`: a walk that does not end in a release
    // build is the case this is for, and what it costs is this counter.
    let height = analysis.height(function);
    let budget = height.saturating_add(1);
    let mut visits = vec![0usize; function.blocks().len()];

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

        visits[block.index()] += 1;
        assert!(
            visits[block.index()] <= budget,
            "the `{}` analysis did not converge. Either its lattice is taller \
             than it said, in which case raise what `Analysis::height` answers, \
             or it has no top at all, in which case this is a defect in safec \
             rather than in the code being compiled and is worth reporting. A \
             join that rebuilds its value into a different shape with the same \
             meaning is the usual way to have no top by accident.\n\n\
             Block {} was walked {} times, and `height` answered {}.",
            core::any::type_name::<A>(),
            block.index(),
            visits[block.index()],
            height,
        );

        for element in &function.block(block).elements {
            analysis.element(function, element, &mut value);
        }
        analysis.terminator(function, &function.block(block).terminator, &mut value);

        // What the block sends, and it is the same for every successor, so it
        // is bound again here to stop the loop below writing to it. Folding a
        // successor's value into this one rather than the other way round
        // compiles while the binding is mutable, because `&mut T` coerces to
        // `&T`, and it gives the next successor what two paths agree on rather
        // than what this block sent. Spelled this way that is `error[E0596]`.
        let value = value;

        successors.clear();
        function.block(block).terminator.successors(&mut successors);

        for &successor in &successors {
            let changed = match &mut values[successor.index()] {
                Some(arrived) => {
                    // Compared rather than asked. What a clone per edge buys
                    // is that the join has no answer to be wrong about. See
                    // ADR-0016.
                    //
                    // Inverting this comparison pushes a block whose value did
                    // not move, so the walk does not end on its own. It used to
                    // hang for that reason and no longer does: the budget above
                    // stops it, and the mutation now fails four tests and hangs
                    // none. Measured, after the budget landed and made the
                    // sentence that used to be here false.
                    let before = arrived.clone();
                    analysis.join(arrived, &value);
                    *arrived != before
                }
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
