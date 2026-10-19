---
status: "accepted"
date: 2026-10-19
decision-makers: itsakeyfut
---

# Levels are not mixed inside a translation unit

## Context and Problem Statement

ADR-0049 chooses the safety level per translation unit, on the command line, and leaves open whether one unit may hold functions at different levels ([#331](https://github.com/itsakeyfut/safec/issues/331)). The case that asks for it is adopting level 5 in an existing code base one function at a time. `--safety` is one value for a whole `safec` run, so every file of a run is checked at one level. The one boundary inside a unit is the hatch (ADR-0038), which lists what its body could not prove instead of reporting it and is not a level.

Phase 8 wires level 5, and how it is wired depends on whether a unit can hold more than one level, so the answer is wanted before that phase plans against either.

## Decision Drivers

* Level 5 believes things lower levels do not: a pointer a function defined at level 5 returns is non-null unless written `_Nullable` (ADR-0050).
* A rule deciding what a level-5 function believes of code checked at a lower level is a rule that can be wrong, and a wrong one believes a promise nobody wrote, which is a silence that is believed: `docs/safety-model.md` calls that the worst thing this compiler can do.
* Adoption should be possible without rewriting a whole file at once.

## Considered Options

* **No mixing:** adoption is by translation unit.
* **A level per function**, spelled as an annotation, with two functions at different levels meeting as if across a unit's boundary.
* **Hold** until level 5 is delivered and real adoption shows what it needs.

## Decision Outcome

Chosen option: **no mixing**, because it is the only one that needs no new boundary rule between levels, and so the only one that adds no new way to be silent. It still believes a `_Nonnull` that is written, across the boundary, of code nothing checked: that is ADR-0050's rule, and this record neither adds to it nor removes it.

A function to be held to level 5 before the rest of its file is moved to a file of its own, and that file is compiled by its own run. It meets the rest of the program through declarations. The boundary between them is the one ADR-0050 already draws between translation units: nothing unannotated is believed in either direction, and a `_Nonnull` that is written is.

A hatch is not a lower level: it does not change the level its callers or its body are checked at. What `--safety strict` does with what a hatch lists is a separate question that ADR-0038 leaves open for the change that wires the level, since level 5 is the level that leaves nothing `Unknown` and a hatch is where an `Unknown` goes instead of the report. This record does not decide it.

Whether `#pragma clang assume_nonnull` is read, which ADR-0050 left with this question, is outside it too: the pragma states a default for a region of declarations rather than choosing a level, and it is [#541](https://github.com/itsakeyfut/safec/issues/541).

### Confirmation

Nothing in the code can spell a level on a function: the attribute reader takes exactly one annotation, the hatch's, and refuses every other as `SC0205`.

* Accepting any annotation string but the hatch's fails `an_annotation_other_than_the_hatch_is_refused` and `an_annotation_naming_a_level_is_refused`.
* Accepting `safec_level(...)` alongside the hatch fails `an_annotation_naming_a_level_is_refused`.

What the decision is about is the translation unit, not the run: each input of a run is lowered as its own unit, so the boundary between two files is the one between units whether or not they share a run. That `--safety` is one value per run is a separate fact, held by the option's type, and a level per input of one run would not reverse this record.

### Consequences

* Good, because no rule is needed for what a level-5 function believes of a lower-level function in the same unit, so no such rule can be wrong.
* Good, because the boundary between levels is one already built and already tested, the one between translation units.
* Bad, because the granularity of adoption is a file. Moving one function to level 5 means moving it to a file of its own, and two levels mean two runs. Since a run has one level and reads every input as C, `safec` cannot link a level-5 file with a lower-level one: each is built with its own `--emit object` run, and the link happens outside `safec`.
* Bad, because a function moved out of its file can no longer reach what has internal linkage there, which C17 6.2.2 p2 confines to that translation unit.
* What would reverse this: adoption showing that file granularity blocks it, together with a boundary rule between levels inside one unit that can be shown never to believe a promise nobody wrote.

## Pros and Cons of the Options

### No mixing

* Good, because its failure is coarseness, not a wrong conclusion.
* Bad, because it is coarse.

### A level per function

* Good, because a single function can be adopted where it is.
* Bad, because two functions at different levels in one unit need a rule for what each believes of the other. A wrong rule believes level 5's non-null default of a function compiled below it, so a null nobody tested is not doubted.
* Bad, because the spelling would be a second annotation the frontend reads, beside the hatch's, before any check needs it.

### Hold

* Good, because adoption would show what it actually needs.
* Bad, because a design for Phase 8 could then assume either answer, and the cost of no mixing is already known.

## More Information

* [ADR-0049](0049-level-5-restricts-what-may-be-written-and-levels-1-to-4-do-not.md), which chose the level per translation unit.
* [ADR-0050](0050-at-level-5-a-pointer-is-non-null-unless-it-is-written-nullable.md), whose boundary is the one between levels.
* [ADR-0038](0038-a-hatch-is-a-function-definition-and-what-it-could-not-prove-is-listed-rather-than-reported.md), the hatch, and its open question about level 5.
* [#249](https://github.com/itsakeyfut/safec/issues/249), a region hatch, the other boundary inside a unit that is planned.
* [`docs/safety-model.md`](../safety-model.md#incremental-safety-levels).
