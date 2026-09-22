---
status: "accepted"
date: 2026-09-22
decision-makers: itsakeyfut
---

# A diagnostic that rejects a program carries the change that would make it compile

## Context and Problem Statement

[ADR-0033](./0033-a-conclusion-this-analysis-could-not-prove-does-not-build.md)
makes a build fail on a conclusion the analysis could not prove. A user meeting
that is being asked to rewrite their program, and a rejection that does not say
what to write is not a diagnostic, it is a refusal. So the remedy stops being a
nicety and becomes part of what a safety diagnostic is.

The vocabulary is already half here, which is the part worth checking before
deciding anything. `Severity::Help` exists in
`crates/safec/src/diagnostics.rs`, and its doc comment reads "A suggested way to
resolve a diagnostic reported alongside it". **No production code constructs
one**, though two tests in `crates/safec/src/diagnostics/render.rs` do, by
iterating every severity. A `Diagnostic` holds a severity, a certainty, a
level, a code, a message, labels and notes, so a remedy written today is a
`with_note` string like any other.

**Two sentences here were wrong and are corrected rather than preserved.**
This paragraph said that the only mention outside the enum was the spelling
assertion, and that the renderer collapses a note and a help into one heading
on one of its two paths. Reading the code while designing #208 found both to be
false: the two tests above construct one, and the comment in `write_one` says
that `ariadne` collapses them *left to itself*, which is why `ReportKind`
`::Custom` is used, so that both paths agree and a help stays distinguishable
from a note. `CLAUDE.md` calls false prose worse than none because it is
believed, and a record cannot be accepted on a premise its author has since
disproved. What the correction costs the decision is in *Decision Outcome*.

What is undecided is therefore not whether a remedy can be printed. It is
whether a remedy is a separate diagnostic reported beside the one it explains,
which is what that doc comment promises, or something the diagnostic itself
carries. Deciding it after Phases 5 to 8 have emitted means revisiting every
safety diagnostic at once.

## Decision Drivers

* [ADR-0002](./0002-severity-runs-from-least-to-most-severe.md) orders
  `Severity` from least to most severe and makes the worst severity in a set its
  maximum. A remedy that is its own entry in the sink's vector can be sorted,
  filtered or counted apart from the thing it explains, and nothing notices.
* A remedy is about the same program point as the rejection. Any consumer
  grouping by code or by span would have to put the two back together, by a
  convention rather than by a type.
* `CLAUDE.md` prefers a guard the compiler holds over one somebody has to
  remember. A rejection shipped with no remedy should be a build failure in this
  repository, not a review comment.
* [`diagnostics.md`](../diagnostics.md) already treats a code as an interface a
  user greps and suppresses on. Whatever carries the remedy is the same kind of
  thing and gets the same care.

## Considered Options

* Notes. A remedy is prose in `notes`, and nothing is built.
* A separate `Diagnostic` at `Severity::Help`, reported beside its subject.
  This is what the existing doc comment describes.
* A field on `Diagnostic`, so the remedy belongs to the diagnostic that rejected
  the program.

## Decision Outcome

Chosen option: "a field on `Diagnostic`".

Notes lose a distinction the renderer already goes to trouble to keep. Neither
path collapses a note into a help: `ReportKind::Custom` exists in `write_one`
precisely so that `ariadne` does not, and the two rendering paths spell the two
apart. That is a stronger argument than the one first written here, which had
the fact backwards: the distinction is not merely *meant* to be real, it is
maintained at a cost, and spelling a remedy as a note throws away something
already paid for.

A separate diagnostic makes the association a convention. Nothing would stop a
remedy being emitted without its subject, ordered away from it, or counted as an
entry that means nothing failed, and each of those is a thing somebody has to
remember rather than something the build refuses. It also gives
`Severity::Help` a second job: a severity is what decides whether compilation
failed, and a remedy is not a severity of anything.

A field makes the association hold by construction. The shape that follows is
the constructor: a safety diagnostic built where
[ADR-0001](./0001-promote-unproven-results-in-the-sink.md) turns a conclusion
into a severity cannot be built without a remedy, so one omitted is
`error[E0061]` at the call site rather than an empty line in the output. That is
row 1 on `CLAUDE.md`'s list, which is where a decision like this should fail.

The field is a list, because one rejection can have two ways out of it and
choosing between them for the reader is not this layer's job.

**No entry carries a span, and this paragraph proposed that they would.** The
design run found that every place a remedy would point at is already a label,
`freed here` and `allocated here` in `driver.rs`, or is not a span this
compiler holds: there is nowhere written down that says where to test a pointer
before reading through it. A field set by nobody and read by nobody is
breakable by no mutation, so it would have been a guard in name only. The
sentence is corrected rather than left standing because `Remedy`'s own doc
comment sends a reader here for that reason, and a record that answers the
opposite of what the code says is worse than one that says nothing.

**What this leaves behind is `Severity::Help`.** It then has no constructor and
no meaning as a severity, so either it goes, which `E0004` makes the rest of the
tree answer for, or its doc comment stops promising the reading this record
rejects. That is a decision for the change that lands this, and it is small
either way.

### Confirmation

**The constructor, and the build stops rather than a test failing.**
`Diagnostic::concluded` in `crates/safec/src/diagnostics.rs` takes a `Remedy`
as its third argument. Mutation: remove that parameter and the two
`with_remedy` calls in its arms. The library does not compile:
`error[E0061]` at `memory_finding` and at `nullability_finding` in
`crates/safec/src/driver.rs`, which are every place a safety finding is built.
The test modules fail the same way under `cargo test`; how many of them there
are is a fact about the suite rather than about this decision, and the suite
grows, so it is not written down here.

That is the guard this record is about, and it is row 1 on `CLAUDE.md`'s list.
A guard spelled as a test instead, asserting that some particular diagnostic
carries a remedy, would hold for the diagnostics somebody wrote it for and say
nothing about the next one, which is the whole population here.

**Reaching the reader is a second thing and takes two more mutations.**
`a_remedy_is_written_after_the_notes_on_both_rendering_paths` in
`crates/safec/src/diagnostics/render.rs` renders one diagnostic twice, with a
label and without, because `write_one` and `render_header_only` each call
`write_remedies` and dropping either is a silence on half the diagnostics.
Mutation: drop the call from `write_one`, and that test fails; restore it and
drop the one in `render_header_only`, and it fails again. Both were applied.

**The third of the three, and the build stops rather than a test failing.**
*Decision Outcome* rejects a separate diagnostic at `Severity::Help` partly
because nothing would stop one being counted as an entry that means nothing
failed. `Diagnostic::new` is private, so outside
`crates/safec/src/diagnostics.rs` and its child modules there is no way to
build a diagnostic at a severity that means nothing. Mutation: make it `pub`
again and call it from `crates/safec/src/driver.rs` with `Severity::Help`.
That compiles, which is the state [#214](https://github.com/itsakeyfut/safec/issues/214)
closed. With it private the same call is `error[E0624]`, an associated function
that is private, and there is nothing to run. Row 1 on `CLAUDE.md`'s list, like
the constructor above it.

**What is held by nobody**, and it is the hole *Decision Outcome* names:
`Diagnostic::error` is public, so a check that builds a rejection through it
rather than through `concluded` carries no remedy and nothing complains.

### Consequences

* Good, because a rejection and the way out of it cannot be separated by
  anything downstream, including a future consumer nobody has written.
* Good, because the obligation lands on whoever adds a check, at compile time,
  rather than on whoever reviews it.
* Good, because it makes ADR-0033 honest. Rejecting more programs is defensible
  only if each rejection is actionable.
* Bad, because it makes every safety check more expensive to write, and some
  rejections have no single remedy to name. A remedy that says which fact was
  missing is the fallback, and it is still more than a note.
* Bad, because `Severity::Help` is left needing a decision it did not need
  before.
* What would reverse this: a remedy turning out to be worth computing lazily or
  offering behind a second command, in which case it is not data the diagnostic
  carries and the association has to be by code rather than by ownership.

## Pros and Cons of the Options

### Notes

* Good, because it works today and costs nothing.
* Bad, because a remedy and an explanation become indistinguishable to
  everything except a reader.

### A separate diagnostic at `Severity::Help`

* Good, because it is what the existing doc comment already describes.
* Good, because a remedy can be suppressed independently.
* Bad, because the association is a convention, and every counter, filter and
  sort is a place it can break.
* Bad, because it puts a non-failure into a vector whose severities decide
  whether the build failed.

### A field on `Diagnostic`

* Good, because the association is held by the type.
* Good, because the constructor can make a missing remedy a compile error.
* Bad, because it is the most to build, and it touches a type everything in the
  compiler already uses.

## More Information

* [`docs/diagnostics.md`](../diagnostics.md), which this serves.
* [ADR-0033](./0033-a-conclusion-this-analysis-could-not-prove-does-not-build.md),
  which is why a remedy is load bearing rather than polish.
* [ADR-0002](./0002-severity-runs-from-least-to-most-severe.md), whose ordering
  is the argument against a remedy being a severity.
* [#208](https://github.com/itsakeyfut/safec/issues/208) is the work that lands
  this and rewrites the Confirmation above.
