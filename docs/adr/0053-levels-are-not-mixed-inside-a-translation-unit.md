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
* A promise believed of code that no level-5 check read is a silence that is believed, which `docs/safety-model.md` calls the worst thing this compiler can do.
* Adoption should be possible without rewriting a whole file at once.

## Considered Options

* **No mixing:** adoption is by translation unit.
* **A level per function**, spelled as an annotation, with two functions at different levels meeting as if across a unit's boundary.
* **Hold** until level 5 is delivered and real adoption shows what it needs.

## Decision Outcome

Chosen option: **no mixing**, because it is the only one whose failure cannot be a silence.

A function to be held to level 5 before the rest of its file is moved to a file of its own, and that file is compiled by its own run. It meets the rest of the program through declarations. The boundary between them is the one ADR-0050 already draws between translation units: nothing unannotated is believed in either direction, and a `_Nonnull` that is written is.

A hatch stays what ADR-0038 makes it, a place inside a unit where what was not proved is listed. It is not a lower level, and nothing about a hatch changes the level its callers or its body are checked at.

### Confirmation

Nothing in the code can spell a level on a function: the attribute reader takes exactly one annotation, the hatch's, and refuses every other as `SC0205`.

* Accepting any annotation string but the hatch's fails `an_annotation_other_than_the_hatch_is_refused` and `an_annotation_naming_a_level_is_refused`.
* Accepting `safec_level(...)` alongside the hatch fails `an_annotation_naming_a_level_is_refused`.

That `--safety` is one value per run is the option's type. Nothing tests it, and nothing needs to, since a second value has nowhere to go.

### Consequences

* Good, because no rule is needed for what a level-5 function believes of a lower-level function in the same unit, so no such rule can be wrong.
* Good, because the boundary between levels is one already built and already tested, the one between translation units.
* Bad, because the granularity of adoption is a file. Moving one function to level 5 means moving it to a file of its own, and two levels mean two runs.
* Bad, because a function moved out of its file can no longer reach what has internal linkage there, which C17 6.2.2 p3 confines to that file.
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
* [ADR-0038](0038-a-hatch-is-a-function-definition-and-what-it-could-not-prove-is-listed-rather-than-reported.md), the hatch.
* [`docs/safety-model.md`](../safety-model.md#incremental-safety-levels).
