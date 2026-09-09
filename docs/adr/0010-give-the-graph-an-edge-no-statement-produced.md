---
status: "accepted"
date: 2026-09-10
decision-makers: itsakeyfut
---

# Give the control-flow graph an edge no statement produced

## Context and Problem Statement

The Safety IR is block-structured from the start, and every analysis this
project exists to write walks its edges. So the set of edges is an interface
in the strongest sense: a check written against four kinds cannot be handed a
fifth without being read again.

C makes it easy to get this wrong, because C alone does not force the question.
`if`, `while`, `for`, `return` and eventually `goto` produce every edge a C
program needs, and a graph built from exactly those is complete for C and
unable to say what an exception is. `docs/c-family.md` states the cost:

> A CFG whose edges come only from `if`, `while`, `return` and `goto` cannot
> represent an exception, and adding a new kind of edge afterwards means
> revisiting every analysis that assumed the old set.

The same document schedules this record, in its table of when each decision is
due: "Phase 2 builds the CFG · Whether an edge can exist that no statement
produced · An ADR, because every later analysis is written against the answer."

C is not free of the shape either. `longjmp` is an edge no statement produced,
and so is every path an error handler runs on.

## Decision Drivers

* **The reversal is not local.** Adding an edge kind later is one line in the
  IR and a re-reading of every analysis, every printer and the interpreter,
  because each one matches on the kind. The cost is paid by whoever adds the
  fifth thing, not by whoever wrote the four.
* **This project's other guards are held by the compiler.** ADR-0003 is
  confirmed by `E0502` and ADR-0004 by `E0451`. A rule about a `match` has the
  same option available: `E0004` makes a variant nobody thought about into a
  build failure rather than a silent default.
* **The feature is not wanted, only the room.** `docs/roadmap.md` says so in
  the same breath as asking for it: "The cheap version now is to leave room in
  the edge representation rather than to build unwinding". Nothing in Phase 2
  unwinds, and nothing should.
* **A C++ frontend is the reason the boundary exists.** `docs/c-family.md`'s
  whole argument is that the IR must be reachable by an adapter the C frontend
  knows nothing about, and an adapter that cannot express `throw` is not one.

## Considered Options

* **A terminator variant nothing produces**, present from the first commit.
* **The C kinds only**, with a doc comment promising the enum will grow.
* **An edge that carries its cause**, where the absence of a cause is the room.
* **A trait or an open set**, so that a kind can be added without touching the
  enum.

## Decision Outcome

Chosen option: **a terminator variant nothing produces**, spelled `Abnormal`
and documented as the shape an exception, a `longjmp` and an error path all
have.

It is the only option that makes the room mechanical. Every `match` on a
terminator written from now on has to answer for it, and the answer of an
analysis that has not thought about exceptions is allowed to be conservative,
but it cannot be absent.

### Confirmation

`E0004`. `Terminator::successors` in `crates/safec/src/ir.rs` matches every
kind and is written out rather than wildcarded, so a kind added later stops it
and every other walk from compiling until somebody says what it means there.
`an_edge_no_statement_produced_can_be_built` in the same file constructs one,
so deleting the variant stops that test compiling.

A call is a terminator for this record's sake as much as for its own: control
leaves a function abnormally where a callee is entered, and an operation in the
middle of a block has nowhere to take an edge from. That is where the room
reserved here will first be used.

That is the guard this record wants and it is exactly as strong as the ones
ADR-0003 and ADR-0004 rely on. What it does not hold is the *quality* of the
answers: an analysis is free to write `Abnormal => {}` and say nothing, and
RK-015 in the review knowledge bank is the entry recording that `E0004` makes
somebody look and nothing more. The review of the first analysis that walks
these edges is where that is caught.

### Consequences

* Good, because the fifth kind, when it arrives, is a variant beside this one
  rather than a change of shape. The analyses that already answer for
  `Abnormal` will mostly answer for it the same way.
* Good, because the Clang adapter has something to lower `throw` into on the
  day it exists, and `docs/c-family.md` names that as the reason the boundary
  is worth anything.
* Bad, because a variant nothing produces is dead weight in every match until
  something produces one, and a reader meets it before meeting a reason. The
  doc comment on it is what pays that back, and it has to keep being true.
* Bad, because "abnormal" is a name chosen before the thing it names exists.
  If unwinding arrives and wants three kinds rather than one, this variant is
  the one that gets split.
* What would reverse this: deciding that the IR is for C and that a C++
  adapter reaches a different IR. `docs/concept.md` and `docs/c-family.md` both
  argue the opposite, so reversing this means reversing them first.

## Pros and Cons of the Options

### A terminator variant nothing produces

* Good, because the room is held by the compiler rather than by a promise.
* Good, because it costs one variant and one doc comment.
* Bad, because it is unreachable code until Phase 9 or a C++ adapter.

### The C kinds only, with a doc comment

* Good, because nothing carries weight it does not use.
* Bad, because the promise is worth exactly as much as the next person's
  reading of it, and the day it is kept every analysis breaks at once. That is
  the cost this record exists to avoid rather than schedule.

### An edge that carries its cause

* Good, because it is more general: an edge with no cause is the room, and the
  cause is a thing a diagnostic could use.
* Bad, because it moves the question into every analysis. "Is this edge
  abnormal" becomes "is this cause absent, and what does absent mean", and the
  answer is a convention rather than a variant. `docs/safety-model.md`'s
  three-valued model is the shape this project uses when something is genuinely
  unknown, and an edge's origin is not unknown: it is known and there is no
  statement to point at.

### A trait or an open set

* Good, because a kind could be added by a downstream crate.
* Bad, because it gives up `E0004`, which is the whole mechanism this record is
  choosing. Nothing in this project has asked to extend the IR from outside,
  and `docs/repository.md` keeps the crate graph small on purpose.

## More Information

* [`docs/c-family.md`](../c-family.md), *The CFG must carry edges no statement
  produced*, and the table under *When each decision is due* that schedules
  this record.
* [`docs/roadmap.md`](../roadmap.md), Phase 2, which asks for the room and not
  the feature.
* [ADR-0004](0004-resolve-the-strictest-level-where-the-policy-is-built.md):
  the same shape of guard, held by `E0451` rather than `E0004`.
