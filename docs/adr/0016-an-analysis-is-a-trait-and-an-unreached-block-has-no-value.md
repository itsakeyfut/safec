---
status: "accepted"
date: 2026-09-12
decision-makers: itsakeyfut
---

# An analysis is a trait, and a block nothing has reached has no value

## Context and Problem Statement

[The roadmap](../roadmap.md) puts four analyses in Phases 5 through 8, and every
one of them is the same shape: a value per program point, a rule for what an
operation does to it, and a join where control flows together. Phase 4 builds
that once so the four are not written four slightly different ways.

Nothing in the workspace iterates anything to a fixpoint. `crate::interp` walks
one path with the values it was given; an analysis walks every path at once and
answers what is true whichever way control went. They share the IR and nothing
else.

So this decides what an analysis author writes and what the solver hands back,
before the first real analysis exists. The only consumer designed against it
today is the framework's own first user, which deliberately reports nothing.

## Decision Drivers

* The first safety analysis is Phase 5's and is not here to validate an
  interface. Prefer the smallest shape that cannot be silently wrong over the
  most expressive one.
* **A law nothing checks is not a guard.** A framework that requires each
  analysis to supply a join identity requires each analysis to be right about
  something no test of the framework can see, and the failure is the one
  `Cfg`'s own doc comment names: a value that should not have been joined makes
  the fixpoint answer that nothing is known anywhere.
* The fixpoint has to be *testable*. Measured while this was decided: a test
  over a loop whose body only writes a local passes unchanged when the solver is
  cut down to one visit per block, because a must-analysis's back edge cannot
  add a write. A framework whose central claim is held by a test like that is
  held by nothing.
* Nothing here may report. `docs/roadmap.md` makes Phase 5 the first phase that
  says anything about a C program.

## Considered Options

* **A trait per analysis, and the first value to arrive at a block is stored
  rather than joined.**
* **A trait per analysis, and every block starts at the analysis's bottom.**
* **Closures passed to a solver function.**

## Decision Outcome

Chosen option: **a trait, and the first arrival is stored rather than joined**.

```rust
pub trait Analysis {
    type Value: Clone + Eq;
    fn height(&self, function: &Function) -> usize;
    fn on_entry(&self) -> Self::Value;
    fn join(&self, into: &mut Self::Value, from: &Self::Value);
    fn element(&self, function: &Function, element: &Element, value: &mut Self::Value);
    fn terminator(&self, function: &Function, terminator: &Terminator, value: &mut Self::Value);
    // Added ahead of Phase 5's null check, which is the caller it exists for
    // and has not landed. The only one with a body here: an analysis that
    // tells both arms the same thing need not answer it.
    fn edge(&self, function: &Function, block: BlockId, terminator: &Terminator, index: usize, value: &mut Self::Value) {}
}
```

**`join` folds and does not report.** It answered whether it had folded when
this was written, and that answer was the only thing that ended the walk: a
join that folded correctly and under-reported handed back something that was
not a fixpoint and said nothing about it. The solver compares instead, which
is what the `Eq` bound is for. The answer moved rather than went away: it is
`Eq::eq` now, and an `eq` that ignores part of what `join` writes gives the
old failure straight back, which is said on the trait where somebody writing
one will read it. What the bound does hold is that a value compares equal to
itself, and a value holding an `f64` does not, which under `PartialEq` was a
walk that never ended and under `Eq` does not compile. Keeping the answer and
checking it under
`debug_assertions` was the alternative and was rejected on where it runs:
nothing in CI builds a release binary, so the shipped compiler would have kept
the failure. What it costs is a clone and a comparison on every edge, in every
build, which has not been measured against anything because nothing here has a
budget to measure it against.

The function is handed to the transfers rather than left for an analysis to
hold. An analysis that asks what a place is reaches `TranslationUnit::place_ty`,
which takes one, and one holding its own could be handed to a `solve` over a
different function: that compiles and answers about neither. Passing it is what
makes "nothing outside these methods" true of the pair and not only of
the trait.

A trait rather than closures because the pieces have names, a place for the
doc comment that says what each owes, and a compiler that answers when the set
grows: a method without a default is `error[E0046]` at every implementation,
while a closure parameter added is a silent change of arity at every call site.
That is not hypothetical. `height` was added after this record was first
written, and the compiler told every implementation about it.

**No bottom.** `Solution` answers `Option<&Value>` because a block the entry
cannot reach has no value and must not be given one, which `Cfg` already decided
for predecessors and for the same reason. That `Option` is also what the solver
holds while it runs, so "nothing has arrived here yet" is already
representable, and the first arrival can simply be stored. Nothing then needs a
value that joins as an identity, so no analysis can get one wrong. It also
avoids a name that points the wrong way: the identity of a must-analysis is
"true of everything", and calling that the bottom makes the weakest-sounding
word mean the strongest claim.

There is a second reason, found later and worth more than the first. `on_entry`
takes `&self`, so what holds at a function's start is a property of the analysis
value rather than of the lattice. A driver that runs one function's body under
two callers, or under two threads, builds two analysis values and gets two entry
facts. Had the entry been the lattice's bottom it would have been one constant
per analysis, and that driver could not be written without a second mechanism.

**Forward only, and one value per outgoing edge.** The transfer runs over a
block's elements and then its terminator, and what that leaves is joined into
every successor. Two things were left out, they cost differently, and one of
them has since landed.

Refining `p` to non-null on the taken arm of a branch was not built, and now is.
It is `Analysis::edge`, a defaulted method taking the successor's *index*, and
the solver clones what a block sends once per outgoing edge and hands each clone
to it. The index rather than the destination, because
`Branch { then: b, otherwise: b }` is two edges and `Cfg` keeps it that way on
purpose, so a `BlockId` cannot tell those two arms apart. `docs/roadmap.md` puts
nullability in Phase 5, which is the caller this was measured for.

**It also takes the block control is leaving, and that was found after it
landed.** The index and the terminator answer `if (p)` and nothing else. Every
other spelling of the same test puts the comparison in an element: `if (p != 0)`
lowers to a `Binary` written to a temporary and a copy of that temporary in the
`Branch`, so the condition an analysis has to read is three lines above the
terminator it is handed and there was no way to name the block holding it. The
block is what reaches it, and it is the leaving end rather than the arriving one
because the index already names the arriving one.

**The terminator was kept beside it, and that is the weaker half of this.**
`solve` passes `&function.block(block).terminator`, so the second argument is a
function of the first, and the two are now a pair a future caller can hand over
disagreeing. Passing the block alone would make that unspellable, which is a
compile error where this is a test, and `CLAUDE.md` ranks the first above the
second. What was bought instead is that
`the_terminator_an_edge_is_walked_with_is_the_one_that_named_it` keeps guarding
what it was written to guard, and that no existing implementation of the method
moved. Those are real and they are smaller. Revisit this the first time a second
caller of `edge` exists: `memory.rs`'s replay is already a second caller of
`element` and `terminator`, so a per-edge report is the shape that would make
the pair something more than a convention.

**What it cost, measured when it landed rather than when it was deferred.** The
prediction here was a defaulted method and three lines of the solver, with every
test in the crate passing unchanged. The second half held: nothing that existed
moved. The first half was short by the clone. The value a block sends can no
longer be shared by every successor, so where the solver passed one `&value` to
each join it now clones per edge, on top of the clone the comparison already
makes. `Terminator::successors`' push order became an interface at the same
time, because an index means nothing without one, and that is a sentence in a
doc comment and an assertion in a test rather than a line of code.

**The method refines and cannot prune.** An analysis that has proved an arm is
never taken still has that arm walked. Letting `edge` drop an edge was rejected
on what it does to `Solution::value`: `None` would mean both a block the entry
cannot reach and a block an analysis decided nothing reaches, and a check
reading that as "nobody runs this" would go silent about code, which is the
failure this project ranks last. Walking an arm that cannot run costs a report
naming code no execution reaches, which is the row above. Reversing that is this
method's signature and nothing else.

A backward analysis is not a defaulted method. It needs a second entry point,
and `on_entry` and `Solution::entry` both come to mean the other end of a block
and of a function. Nothing in the type system says so, which is the hazard worth
writing down: a liveness pass written as an `impl Analysis` and handed to
`solve` compiles and answers forwards. Liveness is the usual first backward
analysis and no phase asks for one yet.

**A value per block entry, not per program point.** A caller that needs a point
inside a block replays that block's elements from its entry value, which is what
the transfer does anyway. Storing every point would store a value per element
and rewrite all of them on each pass, for a reader that does not exist.

### Confirmation

* Cutting the solver to one visit per block, by dropping the re-push of a
  successor whose value changed, fails
  `the_back_edge_changes_the_answer_after_the_loop`, and fails nothing else:
  measured, and the reason the other loop test is not the guard.
* Making the solver answer that nothing moved, however the comparison came
  out, fails the same test and nothing else, for the same reason: the worklist
  empties early either way.
* An analysis whose `join` returns a `bool` is `error[E0053]` at that
  implementation, and dropping the `Eq` bound is `error[E0369]` in the
  solver. Those two are what replaced a law this record used to rest on.
* Seeding every block with `on_entry` rather than the entry alone fails
  `a_block_nothing_reaches_has_no_answer`, and with it every test whose answer
  depends on a value having arrived rather than having been put there: a value
  waiting in a block joins into whatever that block reaches. **How many is not
  written down here on purpose.** It was, twice, and was wrong within the week
  both times, because a count counts the tests that happen to exist rather than
  anything this decision is about, and a guard is added to this suite most
  weeks. What governs the set is the sentence before this one.
* Joining the first arrival into `on_entry` rather than storing it fails
  `what_a_loop_writes_and_what_comes_before_it_are_answered_apart`, and every
  other test that writes something before the block it asks about. A
  must-analysis joined against a value nothing produced answers that nothing is
  known, which is what having no identity element buys and the mutation that
  reverses this record.
* Every method on `Analysis` is `error[E0046]` when an implementation leaves it
  out, which is what makes the set of questions an analysis answers something
  the compiler holds rather than a convention. `height` is where that was
  tested: it landed carrying a default, the default was measured wrong for two
  of the four analyses the roadmap asks for, and taking the default away moved
  the failure from a panic in a shipped compiler to a build that does not
  finish.
* Handing `Analysis::edge` the index `0` for every successor, or calling it once
  and folding the one result into every successor, fails every test whose answer
  depends on two edges out of one block being told apart.
  `each_arm_of_a_branch_is_told_something_different` is the one named for that.
  Running it before the terminator rather than after fails every test that
  solves `Arrived`, which is what
  `an_edge_is_walked_after_the_terminator_that_named_it` is named for. Handing
  it the terminator of a block other than the one control is leaving fails
  `the_terminator_an_edge_is_walked_with_is_the_one_that_named_it` alone, and
  that one is the narrowest of them, which is why it is the guard on the
  argument rather than on the walk. Taking the default off it is
  `error[E0046]`, at `memory.rs`'s `Allocations` before the test suite is
  reached, which is what makes "an analysis that does not implement it is
  unaffected" a claim the compiler holds rather than one this record asserts.
* Handing `Analysis::edge` the entry block's id rather than the block control
  is leaving fails
  `the_block_an_edge_leaves_is_the_one_whose_elements_it_can_read`, which is
  the only test that reads an element through it. An implementation that takes
  the block and answers from the terminator alone fails the same one, and that
  is the half that says the argument is load-bearing rather than present.
* Swapping `then` and `otherwise` in `Terminator::successors` fails every test
  that reads the order rather than the set. That is a wide set and reaches
  another crate, because the lowering builds its arms in that order too:
  `every_terminator_says_where_control_can_go` is the one that asserts the
  order as such, and `each_arm_of_a_branch_is_told_something_different` is the
  one that says what an analysis loses when it moves. The order is what an
  index handed to `edge` means, which is what makes it an interface rather than
  an ordering nobody looks at.
* A value that does not compare equal to itself is `error[E0277]`, and folding
  a successor's value into what a block sends rather than the other way round
  is `error[E0596]`, because what a block sends is bound again before the loop
  that reads it. Both are things a test could not hold: the first is a hang
  and the second passed the whole suite.

Each of the first four was applied and the named tests observed to fail.

### Consequences

* Good, because an analysis is written by naming a height, a value, a join and
  two transfers, and nothing outside those five is left to be right about. A
  sixth was added later and answers itself where an analysis says nothing.
* Good, because the solver cannot be handed a value from a block that never
  runs: `Cfg` leaves unreachable blocks out of both sides of `predecessors`, and
  `Solution` leaves them out of its answers.
* Bad, because termination is the analysis's to answer for and nothing here
  can check it. A lattice with an infinite ascending chain does not reach a
  fixpoint. What the framework does about that is stop and say so, with a
  budget per block, so the failure is a panic naming the analysis rather
  than a hang naming nothing.

  **A budget is not a widening**, and this record said it was. A widening
  changes the answer a converging analysis reaches, and choosing one before
  an analysis needs it is still a guess; a budget changes no answer any
  fixpoint reaches and decides only what happens when there is none. The
  first reason is sound and never reached the second.

  **The budget is what the analysis says it needs, not what the function
  suggests.** Deriving it from the function was tried and is wrong: what
  bounds the visits is the lattice's height, height is a property of the
  value's type, and no number read off a function bounds every analysis. An
  analysis keyed on assignment sites has a height that grows with the code,
  and one keyed on pairs of places grows faster. Measured: reaching
  definitions over one local was stopped by a budget counting locals, while
  converging in well under a second when allowed to.

  So `height` is a fifth method, and it has no default. One was written,
  answering the locals plus the elements, and it was taken away again
  before the record was closed. It was short for the ownership lattice
  [the safety model](../safety-model.md) draws, at every size, because four
  states per place is about three steps per place where that answers about
  two per local. It was short without bound for a points-to value, which
  fills a matrix over places and regions while the answer only grows with
  the code that fills it. And it could not see a key a terminator
  introduces, because it counted elements and a call's destination is a
  terminator's. A default that no analysis it was written for can use is a
  trap shaped like help, so the fifth method answers `error[E0046]` like
  the other four. That is not a different mechanism from the one this
  record chose a trait for; it is that mechanism applied to the method the
  amendment is about.

  What is left unheld is the declaration itself. Nothing checks that an
  analysis is as tall as it says. Too low stops a correct analysis, and far
  enough too high lets a broken one climb long enough to be the hang this
  was built to replace; both are the same panic and it names the method.
* Bad, because `Value: Clone` puts a clone on every edge. A bitset per local is
  what the first analyses hold, and a graph is one function wide.
* Bad, because the `Eq` bound puts a smaller law where a larger one was.
  `join` used to answer whether it had folded and nothing checked the answer;
  now nothing asks, and what is left is that the representation has to be
  canonical. A join that rebuilds a value into a different shape with the same
  meaning never compares equal and never converges, which is a hang and is the
  row below the one the old law failed at. Measured on a union collected
  through a `HashSet`, which is how anyone writes a union. The law is smaller,
  more local, and testable by a test of the analysis rather than only of the
  framework, and the half of it the compiler can hold, that a value compares
  equal to itself, is held by the bound rather than by prose.
* Bad, because a fact that carries a span, which
  [the safety model](../safety-model.md) requires for "p freed here", can fail
  to terminate: a span is not a lattice element, and a `join` that takes the
  arriving one oscillates where two move sites meet below a branch inside a
  loop. Measured as a hang rather than a failure. Choosing the payload stably is
  one line and nothing says to.
* Bad, because a block's value is cloned once per outgoing edge, where it used
  to be shared. That is what saying something different on two edges costs, and
  it is paid by every analysis including the ones that say the same thing on
  both. Measured on the memory check, release, over 400 blocks each holding a
  `malloc`, an `if` and a `free`: about 15% slower than sharing one value, 54
  seconds against 62. Reusing one buffer across the edges with `clone_from`,
  which is the obvious repair, was measured at 65 seconds and is therefore not
  one. The numbers belong to one machine and one input and will not survive
  either changing; what they are here for is that the shape of the answer does,
  and the obvious repair being slower is the part worth not rediscovering.
* What would have reversed this: an analysis that has to say something different
  on two edges out of one branch, which is the first thing
  [the safety model](../safety-model.md)'s null-pointer case wants, and which
  `docs/roadmap.md` puts in Phase 5. **It arrived and did not reverse this**: it
  was a defaulted method and not a new shape, and the fact is refined per edge
  rather than split per edge, so the value is still one per block entry and
  everything above still holds. This record is the one to supersede on the day
  an analysis needs the fact itself to live on the edge.

## Pros and Cons of the Options

### A trait, first arrival stored

* Good, because no analysis has to supply an identity element for `join`.
* Good, because `E0046` answers for a method added later.
* Bad, because the solver carries one `match` that the bottom version does not.

### A trait, every block starts at bottom

* Good, because it is the textbook shape, and the solver's inner loop is one
  line shorter.
* Bad, because "bottom is the identity of join" is a law each analysis obeys and
  no test of the framework can check. Breaking it answers that nothing is known
  anywhere, which looks like a conservative result and is not one.
* Bad, because the name is upside down for a must-analysis.

### Closures

* Good, because it adds no type.
* Bad, because nothing names the arguments, and one added later is a silent
  change of arity rather than `E0046`.
* Bad, because there is nowhere to write what each piece owes, and this project
  puts the local reason next to the decision.

## More Information

* [`docs/roadmap.md`](../roadmap.md): Phase 4, and the four analyses after it.
* [`docs/safety-model.md`](../safety-model.md): what those analyses conclude.
* [ADR-0010](0010-give-the-graph-an-edge-no-statement-produced.md): the edges
  this walks, and why a call is a terminator.
* `crates/safec-ir/src/cfg.rs`: the order, the predecessors, and why an
  unreachable block is nobody's predecessor.
