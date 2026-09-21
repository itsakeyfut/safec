---
status: "proposed"
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
resolve a diagnostic reported alongside it". **Nothing constructs one.** The
only mention outside the enum is an assertion that it spells itself `help`, and
`crates/safec/src/diagnostics/render.rs` collapses a note and a help into one
heading in one of its two rendering paths. A `Diagnostic` holds a severity, a
certainty, a level, a code, a message, labels and notes, so a remedy written
today is a `with_note` string like any other.

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

Notes lose a distinction the renderer is already trying to make. Two headings
exist and one of the two paths collapses them, so the difference between context
and a change to make is already meant to be real, and spelling a remedy as a
note gives that up rather than settling it.

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

Several remedies at several places is the normal case, so the field is a list,
and each entry carries the span it is about. A remedy with no span is a remedy
about the whole diagnostic.

**What this leaves behind is `Severity::Help`.** It then has no constructor and
no meaning as a severity, so either it goes, which `E0004` makes the rest of the
tree answer for, or its doc comment stops promising the reading this record
rejects. That is a decision for the change that lands this, and it is small
either way.

### Confirmation

**Nothing guards this today.** The only thing in the tree about a remedy is
`Severity::Help`'s spelling assertion in `crates/safec/src/diagnostics.rs`,
which holds a string and nothing about where a remedy lives.

What would guard it is the constructor. Once a rejecting safety diagnostic
cannot be built without a remedy, the mutation is to give that parameter a
default or make it optional, and the build stops at every call site that
supplied one. A guard spelled as a test instead, asserting that some particular
diagnostic carries a remedy, is worth less: it holds for the diagnostics
somebody wrote it for and says nothing about the next one, which is the whole
population this record is about.

This section is to be rewritten in the change that lands the field. RK-017 in
the review knowledge bank is what happens otherwise, and ADR-0029 is the
instance where a record's Confirmation outlived the rule it described.

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
