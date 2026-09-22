---
status: "accepted"
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

This diverged from [`safety-model.md`](../safety-model.md) when it was written.
[#209](https://github.com/itsakeyfut/safec/issues/209) landed it, and that
document now says what this decided rather than pointing at a proposal.

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

**What the default level makes this mean, recorded at acceptance.** The
paragraph above says the plain invocation stays silent, and it does not:
`--safety` has defaulted to `memory` since the command line was defined, so
`safec main.c` is a checked invocation and the option chosen here and the option
rejected above are the same thing for it. That was checked in the code while
[#209](https://github.com/itsakeyfut/safec/issues/209) was designed rather than
assumed, and the default was left where it is. Moving it to `off` would make a
successful bare run mean either "checked and proved" or "nothing ran", with one
exit code for both, which is
[#62](https://github.com/itsakeyfut/safec/issues/62) and the bottom row of
`CLAUDE.md`'s list; rejecting more programs is row 4, where the reader can see
it. So the way in is `--safety off`, typed out, or `--allow-unknown` while a
program is being migrated.

**What this costs is paid for by
[ADR-0032](./0032-bound-what-is-unchecked-inside-a-declared-hatch.md).** Without
a hatch this rejects code with nowhere to put what cannot be proved, which is
not a strict compiler, it is an unusable one. The two records are one position
taken in two places.

### Confirmation

`every_level_that_runs_a_check_denies_unknown_unless_it_was_allowed` in
`crates/safec/src/diagnostics.rs` is the rule. It walks a written-out table of
every level against both answers, and checks that table's length against
`SafetyLevel::value_variants()`, so a level added without an answer fails it by
name rather than passing quietly.

The mutation is resolving the default the way it resolved before this landed,
`deny_unknown: allow_unknown || safety >= SafetyLevel::Strict`. **Two halves
have to go red**: that test, and the corpus, where every case that reports an
unproven conclusion and asks for nothing about it now carries a non-zero
expected exit code. If only one does, the default is being decided in two
places, which is what ADR-0001 and ADR-0004 exist to prevent. Applied, and both
were observed to fail. No count is given: a count of the cases that happen to
exist is a fact about the suite, which grows every week, rather than about the
rule. See RK-028.

The other side is held too, which is what keeps the promotion from being correct
for the wrong reason:
`an_unproven_free_is_a_warning_under_allow_unknown` and its `use` and
`dereference` siblings are the only cases that reach the unpromoted arm of
`certainty_note` from a command line.
`every_safety_level_carries_the_request_across_the_boundary_unchanged` in
`crates/safec/src/cli.rs`, which replaced the test that held the old default,
says the command line does not decide it early;
`allowing_unknown_is_refused_at_the_level_defined_as_leaving_none` beside it and
`an_invocation_that_asks_for_two_different_things_exits_two` in
`crates/safec/tests/exit_code.rs` hold the refusal and its wiring.

### Consequences

* Good, because a build that succeeded becomes a claim. Today it is the absence
  of one.
* Good, because the level a user asked for and the strictness they get stop
  being two settings that can disagree.
* Bad, because the false positive rate becomes something users feel rather than
  something they can ignore. That is row 4 on `CLAUDE.md`'s list and it is the
  whole cost of this decision.
* Bad, because the flag inverts. Denying became the default above level 0 and
  `--deny-unknown` was removed rather than left as a flag whose name describes
  the default; `--allow-unknown` is what remains.
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
* Good, because level 0 is untouched. What that is worth is less than it reads:
  `--safety` defaults to `memory`, so reaching level 0 means editing the build
  that was meant to be able to set `CC=safec` and change nothing else. See
  *What the default level makes this mean* above.
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
