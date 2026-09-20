---
status: "accepted"
date: 2026-09-20
decision-makers: project author
---

# A dereference refines the pointer it went through to non-null, on the path that reached it

## Context and Problem Statement

The nullability check answers `Unknown` for a pointer nothing has said anything
about, which `docs/safety-model.md` asks for and which `docs/roadmap.md` puts in
Phase 5. A parameter is such a pointer, and so is a `malloc` result, because
C17 7.22.3.4 p3 says the function returns either a null pointer or a pointer to
the allocated space.

That answer is right and it is not free. Measured before this was decided: 62 of
the corpus's 219 `.c` files dereference a pointer outside a declaration, and
`crates/safec/src/cli.rs` defaults `--safety` to `memory`, so everything this
check says is said by a bare `safec a.c`. What decides whether that is one
warning per pointer or one per dereference is whether the check learns anything
from a dereference it has already reported.

## Decision Drivers

* `docs/safety-model.md` ranks going quiet about something unproven as the worst
  answer this compiler can give, so nothing here may turn an `Unknown` into
  silence without establishing something.
* A warning a reader has already been given, repeated at every later use of the
  same pointer, is how a check stops being read at all.
* Whatever is chosen has to survive a branch: a fact that holds after a
  dereference does not hold on an arm that skipped it.

## Considered Options

* A dereference refines the pointer to non-null for the rest of the path
* A dereference changes nothing, and every dereference asks again
* Report once per pointer per function, suppressing later findings

## Decision Outcome

Chosen option: "a dereference refines the pointer to non-null for the rest of
the path", because it is the only one of the three that is a *fact* rather than
a presentation rule. A dereference that ran and was not reported did not trap,
so the pointer was not null where it happened, and the next dereference on that
path is nothing to say about. It lives in the lattice, so
`crate::dataflow::Analysis::join` takes it away again wherever an arm that
skipped the dereference meets one that did not.

Suppressing later findings would answer the same thing on the straight-line
program and a different thing after a branch: the suppressed report on the arm
that skipped the first dereference is one nobody proved anything about, and
silence there is the row this project ranks last.

**It is the first dereference that is reported, not the last.** The value the
report reads is what held where the element runs, and the refinement is applied
by the transfer after it, which is the same order `crate::memory` uses for the
same reason.

### Confirmation

`a_pointer_dereferenced_twice_is_reported_once` in `crates/safec/tests/cases`:
dropping the call to `met` in `crate::nullability`'s `Analysis::element` reports
both dereferences and fails it.

`a_pointer_dereferenced_on_one_arm_is_not_proved_after_it` is the half that says
this is per path rather than per function. The mutation is in `Nullness::joined`:
answering `NonNull` whenever either side holds it, so that the refinement
survives a merge instead of being taken away by it. The dereference after the
branch then reports nothing and the case fails.

**Neither mutation's wider set is written down here.** Dropping `met` fails
every case whose expected output depends on a pointer having been dereferenced
before, which is most of the corpus's memory cases and stays true as cases are
added; a count would not. See RK-028.

`error[E0004]` at `Nullness::inverted` and `Nullness::concluded` if a fourth
state is added, which is what makes a state that is neither of these three
answer for what a dereference does to it.

### Consequences

* Good, because the answer to "is this pointer null here" stops depending on how
  many times the program has already asked.
* Good, because it needs nothing new: the lattice already joins, so the fact is
  taken away at a merge without a rule anybody has to remember.
* Bad, because a reader who sees one warning on a pointer dereferenced five
  times may read the other four as proved. They are not: they are on a path
  where the first one was the proof.
* Bad, because it rests on the first dereference having been reported. A future
  change that suppresses a report without also dropping the refinement would
  make the rest of the path quiet, which is why the Confirmation above names the
  transfer rather than the report.
* What would reverse this: an annotation that says a parameter is not null,
  which is #134. Once a pointer can be declared non-null, the warning this
  exists to bound is one a user can remove at the source, and reporting every
  dereference becomes affordable again.

## Pros and Cons of the Options

### A dereference refines the pointer to non-null for the rest of the path

* Good, because it is sound: an execution that reached the second dereference
  went through the first.
* Good, because the join handles the branch with no extra machinery.
* Bad, because the fact is invisible in the report: nothing in the output says
  why the second dereference was quiet.

### A dereference changes nothing

* Good, because it is the smallest implementation and the easiest to explain.
* Bad, because measured against the corpus it is a warning per dereference
  rather than per pointer, on 62 files, at the default safety level.
* Bad, because it asks the same question after it has been answered, which is
  what a reader learns to skip.

### Report once per pointer per function, suppressing later findings

* Good, because the output looks the same as the chosen option on straight-line
  code.
* Bad, because it is a rule about reporting rather than a fact about the
  program, so it suppresses a finding on an arm where nothing was established.
  That is `docs/safety-model.md`'s worst row reached by a presentation choice.

## More Information

* `crates/safec-ir/src/nullability.rs`, where `met` applies it and
  `Analysis::join` takes it away.
* [ADR-0016](0016-an-analysis-is-a-trait-and-an-unreached-block-has-no-value.md):
  the framework this is written against, and why a value with no bottom joins
  the way it does.
* [ADR-0001](0001-promote-unproven-results-in-the-sink.md): what an `Unknown`
  costs a build, which is not this check's to decide.
* `docs/safety-model.md`, whose three-valued model is what makes `Unknown` a
  thing to report rather than a thing to round away.
