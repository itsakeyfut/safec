---
status: "proposed"
date: 2026-09-22
decision-makers: itsakeyfut
---

# A conclusion this analysis could not prove does not build, once a program has asked to be checked

## Context and Problem Statement

[`safety-model.md`](../safety-model.md#safe-unsafe-unknown) reports an
`Unknown` conclusion as a warning and makes `--deny-unknown` the flag that turns
it into an error. `crates/safec/src/cli.rs` resolves it that way today: the flag
sets it, `--safety strict` implies it, and every level below leaves it off.

That default belongs to a compiler that accepts every program and comments on
it. This project's position is the other one: a program is rewritten until it
can be proved, and `safec` says what to change. Under that reading a build that
succeeds while the analysis could not prove something is a build that says
nothing, and `--deny-unknown` is not a policy a user opts into. It is what being
checked means.

This diverges from [`safety-model.md`](../safety-model.md), so that document
gains a pointer here in the same change. Nothing in the code depends on this
record yet, and the flag keeps its current behaviour until the change that
closes it.

## Decision Drivers

* A checked program is accepted or rejected. An analysis that answers "possibly"
  and exits 0 has moved the decision to whoever reads the output, which is the
  one party with no way to check it.
* `CLAUDE.md` puts `safec` saying safe and being believed alone at the bottom of
  its list. A warning in a build that succeeded is the slow road to it: nothing
  distinguishes the warning nobody read from the proof nobody needed.
* [ADR-0001](./0001-promote-unproven-results-in-the-sink.md)'s mechanism is not
  in question. A check still names a conclusion and never reads the policy.
  What changes is one value: which policy the sink is built with by default.
* [`concept.md`](../concept.md)'s incremental migration has to survive it.
  Existing C must still be able to enter the system.

## Considered Options

* Keep the warning as the default, with `--deny-unknown` opt-in. This is what
  is built.
* An unproven conclusion fails the build wherever a safety check runs, so from
  `--safety memory` upward, and the plain invocation stays silent.
* An unproven conclusion fails the build at every level, including a plain
  `safec main.c`.

## Decision Outcome

Chosen option: "an unproven conclusion fails the build wherever a safety check
runs".

Failing at every level breaks the way in. `safec main.c` on unconverted C would
reject at the first line, and
[the level ladder](../safety-model.md#incremental-safety-levels) exists
precisely so that does not happen.

Keeping the warning as the default makes the guarantee unstatable. A successful
build would mean either that the program was proved or that the analysis gave
up, with the exit code identical in both cases, and the whole of
[`safety-model.md`](../safety-model.md#safe-unsafe-unknown)'s distinction would
survive only in text somebody may or may not have read.

The middle one makes asking to be checked and failing unless proved the same
event. `--safety memory` is a user saying they want the answer, and the answer
is only worth having if a build that survives it means something.

**What this costs is paid for by
[ADR-0032](./0032-bound-what-is-unchecked-inside-a-declared-hatch.md).** Without
a hatch this rejects code with nowhere to put what cannot be proved, which is
not a strict compiler, it is an unusable one. The two records are one position
taken in two places.

### Confirmation

**Nothing guards the new rule today, and one test holds the old one.**
`a_lower_safety_level_leaves_deny_unknown_to_its_flag` in
`crates/safec/src/cli.rs` asserts that every level below strict leaves the flag
alone, which is exactly what this decision reverses.
`deny_unknown_is_set_by_its_flag` beside it, and
`the_sink_is_built_from_the_resolved_options` in `crates/safec/src/driver.rs`,
hold the path from the flag to the sink and are unaffected by which way the
default points.

So the record is confirmed by nothing until the change that lands it, and that
change is a rewrite of the first of those tests rather than an addition beside
it. The mutation that must fail afterwards is resolving the default the way it
resolves now: a level above 0 with the flag absent, reaching a sink that does
not promote. If that mutation leaves the suite green, the default was changed
somewhere the resolution does not run through.

### Consequences

* Good, because a build that succeeded becomes a claim. Today it is the absence
  of one.
* Good, because the level a user asked for and the strictness they get stop
  being two settings that can disagree.
* Bad, because the false positive rate becomes something users feel rather than
  something they can ignore. That is row 4 on `CLAUDE.md`'s list and it is the
  whole cost of this decision.
* Bad, because the flag inverts. `--deny-unknown` becomes the default above
  level 0, so what remains is the flag that allows unknown, and Phase 5 should
  wire it that way rather than build the current shape and reverse it.
* What would reverse this: a measurement on real C showing that what is left
  unproven cannot be absorbed by ADR-0032's hatch and by annotations, so that
  the rewrite asked of a user is not some of a file but most of it.

## Pros and Cons of the Options

### Warning by default

* Good, because no existing program stops building, at any level.
* Good, because it is what is built, so it costs nothing.
* Bad, because the exit code stops carrying the distinction the safety model is
  built on.

### Failing wherever a check runs

* Good, because asking for a check and being held to it are one action.
* Good, because level 0 is untouched, so the migration path in
  [`concept.md`](../concept.md) survives.
* Bad, because it needs the hatch before it is usable, so two records have to
  land near each other.

### Failing at every level

* Good, because there is one behaviour to explain.
* Bad, because the plain invocation becomes useless on the code this project
  exists to accept first.

## More Information

* [`docs/safety-model.md`](../safety-model.md#safe-unsafe-unknown), which this
  diverges from and which now points here.
* [ADR-0001](./0001-promote-unproven-results-in-the-sink.md), whose mechanism
  this leaves alone.
* [ADR-0032](./0032-bound-what-is-unchecked-inside-a-declared-hatch.md), which
  this depends on.
* [ADR-0034](./0034-a-diagnostic-that-rejects-a-program-carries-the-change-that-would-make-it-compile.md),
  which is what a user meets when this decision fires.
* [#209](https://github.com/itsakeyfut/safec/issues/209) is the work that lands
  this and rewrites the Confirmation above.
