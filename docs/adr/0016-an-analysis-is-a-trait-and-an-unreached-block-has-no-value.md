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
    type Fact: Clone;
    fn on_entry(&self) -> Self::Fact;
    fn join(&self, into: &mut Self::Fact, from: &Self::Fact) -> bool;
    fn element(&self, element: &Element, fact: &mut Self::Fact);
    fn terminator(&self, terminator: &Terminator, fact: &mut Self::Fact);
}
```

A trait rather than closures because the four pieces have names, a place for the
doc comment that says what each owes, and a compiler that answers when the set
grows: a fifth method without a default is `error[E0046]` at every
implementation, while a fifth closure parameter is a silent change of arity at
every call site that already passes four.

**No bottom.** `Solution` answers `Option<&Fact>` because a block the entry
cannot reach has no value and must not be given one, which `Cfg` already decided
for predecessors and for the same reason. That `Option` is also what the solver
holds while it runs, so "nothing has arrived here yet" is already
representable, and the first arrival can simply be stored. Nothing then needs a
value that joins as an identity, so no analysis can get one wrong. It also
avoids a name that points the wrong way: the identity of a must-analysis is
"true of everything", and calling that the bottom makes the weakest-sounding
word mean the strongest claim.

**Forward only, and one value per outgoing edge.** The transfer runs over a
block's elements and then its terminator, and the result is joined into every
successor. A backward analysis and an edge-sensitive one are both real and
neither is asked for: liveness is the usual first backward analysis and no phase
wants one yet, and refining `p` to non-null on the taken arm of a branch is
Phase 6's question. Each is a defaulted method and a change to the solver away,
and neither costs an analysis written today a line.

**A value per block entry, not per program point.** A caller that needs a point
inside a block replays that block's elements from its entry value, which is what
the transfer does anyway. Storing every point would store a value per element
and rewrite all of them on each pass, for a reader that does not exist.

### Confirmation

* Cutting the solver to one visit per block, by dropping the re-push of a
  successor whose value changed, fails
  `the_back_edge_changes_the_answer_after_the_loop`, and fails nothing else:
  measured, and the reason the other loop test is not the guard.
* Making the solver ignore what `join` answered fails the same test and
  nothing else, for the same reason: the worklist empties early either way.
* Seeding every block with `on_entry` rather than the entry alone fails
  `a_block_nothing_reaches_has_no_answer`, and two more besides: a value
  waiting in a block joins into whatever that block reaches.
* Joining the first arrival into `on_entry` rather than storing it fails
  `what_a_loop_writes_and_what_comes_before_it_are_answered_apart`, because a
  must-analysis joined against a fact nothing produced answers that nothing is
  known. That is this record's central claim, and it is the mutation that
  reverses it.
* A fifth method on `Analysis` without a default is `error[E0046]` at every
  implementation, which is what makes the set of questions an analysis answers
  something the compiler holds rather than a convention.

Each of the first four was applied and the named tests observed to fail.

### Consequences

* Good, because an analysis is written by naming a value, a join and two
  transfers, and there is no fifth thing to be right about.
* Good, because the solver cannot be handed a value from a block that never
  runs: `Cfg` leaves unreachable blocks out of both sides of `predecessors`, and
  `Solution` leaves them out of its answers.
* Bad, because termination is the analysis's to answer for and nothing here can
  check it. A lattice with an infinite ascending chain does not reach a
  fixpoint, and the failure is a hang rather than a diagnostic. The framework
  says so and does not bound it, because bounding it means choosing a widening,
  and a widening chosen before any analysis needs one is a guess.
* Bad, because `Fact: Clone` puts a clone on every edge. A bitset per local is
  what the first analyses hold, and a graph is one function wide.
* What would reverse this: an analysis that has to say something different on
  two edges out of one branch, which is the first thing
  [the safety model](../safety-model.md)'s null-pointer case will want. That is
  a defaulted method rather than a new shape, and if it turns out that the fact
  has to be split per edge everywhere, this record is the one to supersede.

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
* Bad, because nothing names the four arguments, and a fifth is a silent change
  of arity rather than `E0046`.
* Bad, because there is nowhere to write what each piece owes, and this project
  puts the local reason next to the decision.

## More Information

* [`docs/roadmap.md`](../roadmap.md): Phase 4, and the four analyses after it.
* [`docs/safety-model.md`](../safety-model.md): what those analyses conclude.
* [ADR-0010](0010-give-the-graph-an-edge-no-statement-produced.md): the edges
  this walks, and why a call is a terminator.
* `crates/safec-ir/src/cfg.rs`: the order, the predecessors, and why an
  unreachable block is nobody's predecessor.
