---
status: "proposed"
date: 2026-09-22
decision-makers: itsakeyfut
---

# Bound what is unchecked inside a declared hatch, and check its boundary rather than its body

## Context and Problem Statement

[`concept.md`](../concept.md) asks how much of Rust's memory, lifetime,
ownership and thread safety can be brought into C. The answer this project is
now built around is that `safec` rejects a program it cannot prove and asks for
a rewrite, rather than accepting every program and commenting on it. That only
works if there is somewhere to put what cannot be proved and must still
compile: type punning, a hardware register, an allocator, inline assembly, and
every call into a C library nobody annotated.

Rust does not work because `unsafe` exists. It works because `unsafe` is a
bounded region that can be enumerated, and because the boundary around it is an
interface the checked side is verified against. Nothing in this project has
decided whether such a boundary exists here, and
[ADR-0033](./0033-a-conclusion-this-analysis-could-not-prove-does-not-build.md)
cannot be taken without it.

Nothing depends on this record yet. It is due before [Phase 6](../roadmap.md),
because Phase 5 already asks whether an annotation is trusted or checked, and
that is this question one level down: a trusted annotation is a hatch boundary
wearing another name.

## Decision Drivers

* The guarantee has to be statable. A guarantee conditional on something nobody
  can enumerate is not a guarantee, it is a hope with a flag on it.
* `CLAUDE.md`'s ranking. A hatch that suppresses a report without recording a
  promise turns row 4, a false positive the reader can see, into row 6, `safec`
  saying safe and being believed.
* [`concept.md`](../concept.md)'s incremental migration. A hatch is what lets a
  half-converted file compile at all.
* The analysis will never have all the code. Separate translation units, the C
  library, and [Phase 10](../roadmap.md)'s Clang adapter all read code this
  compiler did not check.

## Considered Options

* No hatch: Level 5 is reached by inference and annotation alone.
* A suppression: a way to silence a check at a site.
* A declared region that claims nothing about its body, whose boundary carries a
  promise, and where the promise is what the checked side reads.

## Decision Outcome

Chosen option: "a declared region whose boundary carries a promise".

No hatch is not reachable. The analysis has to read code it does not have, and
no phase on the roadmap changes that. Choosing it means Level 5 is unreachable
on any program that calls `malloc`, which makes the top of
[the level ladder](../safety-model.md#incremental-safety-levels) decorative.

A suppression is row 6 by construction. It turns an unproven result into
silence with nothing written down, so afterwards there is no way to say what the
guarantee rests on, and no finite thing for an audit to look at.

The third keeps the conclusion honest in both directions. Inside the region
nothing is claimed, so nothing is silently believed. At the boundary something
is claimed, and a claim written by a person at a named place is a thing a
reviewer can find, count and disagree with. It is the same property that makes
an `unsafe` block auditable, and the cost here is that the promise is believed
rather than proved.

**A hatch that cannot say what it did assumes the worst.** Where a hatch is a
region rather than a declaration, something has to decide what the checked side
may assume on the way out, and the default is that it may assume nothing: every
allocation and every escaped local the region could reach becomes unproven.
Narrowing that, by having the region declare what it writes and what it frees,
is explicit and opt in.

The direction is ranked rather than preferred. Under the conservative default a
region whose effects were never declared produces false positives, which is row
4 on `CLAUDE.md`'s list and something the reader can see. Under the permissive
one the same omission produces silence, which is row 6. The two are not
comparable, and the more convenient default is the one that fails the wrong way.

**The conservative half is already built.** `crates/safec-ir/src/memory.rs` has
`Callee::Opaque`, whose doc comment says it may free what it was passed and that
this cannot tell, and ADR-0029 and ADR-0031 are what a call and a write this
check cannot follow do to an escaped local. A region hatch is that treatment
with a way to narrow it, so what this decision costs is vocabulary rather than
an analysis.

**This record decides the stance and not the syntax.** What the region and the
promise are spelled as belongs with the annotation experiment
[`safety-model.md`](../safety-model.md#annotations) leaves open, and deciding it
before Phase 5 has written one annotation would be deciding it with no evidence.

### Confirmation

**Nothing guards this today, and that is the honest answer.** There is no hatch
in the tree, there is no annotation, and no check consults one. `Certainty` and
`Conclusion` in `crates/safec-ir` are the only things that currently carry what
a check could not establish, and neither has a notion of a promise standing in
for a proof.

What would guard it is the first check that reads a hatch. The guard to build
with it is a test that a hatch does **not** change what a check concluded, only
what the conclusion is about: a program whose hatch is removed and whose
boundary promise stays must still be rejected. The mutation is to make the hatch
suppress the report instead, and if the suite stays green then the suppression
option was taken by accident.

The default decided above needs a guard of its own: a region that declared no
effects, followed by a use of an allocation it could have reached, has to
report. The mutation is to leave what such a region could reach alone. If the
suite stays green, the permissive default was built and nothing said so.

This section is to be rewritten in the change that lands the first hatch, not
afterwards. RK-017 in the review knowledge bank is what happens otherwise.

### Consequences

* Good, because the guarantee becomes a sentence somebody can check: proved
  outside the hatches, given the promises at their boundaries.
* Good, because the hatches in a program are a finite set, so an audit has a
  target rather than a codebase.
* Good, because it answers Phase 5's trusted-or-checked question once. A trusted
  annotation is a boundary promise, and it gets whatever treatment this record's
  boundary gets.
* Bad, because a wrong promise is believed, which is row 6 arrived at
  deliberately. What limits it is that a person wrote it at a place with a name,
  rather than a check concluding it out of nothing.
* Bad, because it is a language extension, and [`concept.md`](../concept.md)
  asks for those to be explored rather than fixed early. Mitigated by deciding
  the stance here and leaving the spelling to the phase that has evidence.
* What would reverse this: a real C program, with its calls into the C library,
  that passes the checked subset with no hatch in it.
  [Phase 9](../roadmap.md)'s **Done when** is the nearest thing to that test
  already on the roadmap.

## Pros and Cons of the Options

### No hatch

* Good, because the guarantee is unconditional, which is the strongest thing a
  compiler can say.
* Bad, because no program that calls code this compiler did not check can reach
  it, and that is every program.

### A suppression

* Good, because it is the cheapest to build and the most familiar.
* Bad, because it records nothing. After it is used, the set of things the
  guarantee rests on is not recoverable from the source.
* Bad, because it is indistinguishable, at the point of use, from having fixed
  the problem.

### A declared region with a promise at its boundary

* Good, because what is trusted is written down, in one place, per hatch.
* Good, because the checked side reads the promise rather than the body, so the
  body can be anything and the analysis does not have to follow it.
* Bad, because the promise can be wrong and nothing here catches that.
* Bad, because it is the largest of the three to design.

## More Information

* [`docs/safety-model.md`](../safety-model.md), which this serves and which has
  no hatch in it today.
* [ADR-0033](./0033-a-conclusion-this-analysis-could-not-prove-does-not-build.md),
  which depends on this one: rejecting every unproven result is only usable if
  there is somewhere to put what cannot be proved.
* [`docs/roadmap.md`](../roadmap.md), Phase 5, whose trusted-or-checked line is
  this question at the scale of one annotation.
* [#210](https://github.com/itsakeyfut/safec/issues/210) is the work that lands
  this and rewrites the Confirmation above. [#134](https://github.com/itsakeyfut/safec/issues/134)
  meets the same decision at the scale of one annotation and answers first.
