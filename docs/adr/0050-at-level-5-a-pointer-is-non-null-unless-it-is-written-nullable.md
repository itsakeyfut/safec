---
status: "accepted"
date: 2026-10-05
decision-makers: itsakeyfut
---

# At level 5 a pointer is non-null unless it is written `_Nullable`

## Context and Problem Statement

C has one way to say a pointer holds nothing, the null pointer, and nothing in a pointer's type says whether it may. At levels 1 to 4 the nullability check reads where a pointer was tested and answers a dereference it could not settle with a doubt; `_Nonnull` on a parameter (ADR-0037) is the only annotation by which a program states that a pointer is not null. Level 5 leaves nothing `Unknown` (ADR-0049), so a pointer whose nullness the check cannot settle has to be one the program says something about.

Forbidding null outright was considered and is not the direction: the free-and-null idiom, `free(p); p = 0;`, is itself a safety practice (ADR-0048), and C has no other way to say "absent" or "not found". What a level-5 program has to make visible is where null is possible.

## Decision Drivers

* A dereference of a pointer that may be null, believed safe, is the worst answer this compiler can give: it says safe and is believed.
* The same source should compile with `clang` and with `safec` at every level.
* Annotating where null is possible should cost less than annotating where it is not, since most pointers in code written to be checked are not null.

## Considered Options

* **Forbid assigning or passing null at level 5.**
* **Non-null by default at level 5, and `_Nullable` where null is possible**, the default `_Nonnull` makes explicit today, inverted.
* **The same spelling under a name of this compiler's own**, such as an attribute.

## Decision Outcome

Chosen option: **non-null by default at level 5, and `_Nullable` where null is possible.**

- At level 5, a pointer not written `_Nullable` may not be given a value that may be null: a null pointer constant, or a pointer the nullability check cannot show is not null. **The nullability check reports it**, as it reports such a value passed to a `_Nonnull` parameter today (`SC0405`, ADR-0037), with a remedy naming `_Nullable` or the test that would settle it, and at level 5 that refuses the build since nothing is left `Unknown`. It is not a diagnostic about the program's text (ADR-0049): whether a value may be null is what the check settles, so the check says it.
- A `_Nullable` pointer may hold null and may not be dereferenced, or passed where a non-null one is expected, until a test on the path settles it, which the nullability check already reads.
- `_Nonnull` keeps its meaning at every level; at level 5 it states the default.
- **At the boundary with code this translation unit cannot see, nothing unannotated is believed**, in either direction. The level is chosen per translation unit (ADR-0049), and a prototype read here, from a header or written out, says nothing about the level its definition was compiled at, so "a declaration not written at level 5" is not something a translation unit can tell.
  - A function this translation unit declares and does not define takes and returns `_Nullable` pointers wherever its declaration says neither. A pointer returned by `malloc`, or by a function in another translation unit, needs a test before it is used, and `free(NULL)` or `time(NULL)` needs nothing.
  - A function with external linkage defined here takes a `_Nullable` pointer parameter wherever its declaration says neither, since a caller in another translation unit is checked by nobody. A `_Nonnull` there is believed, as ADR-0037 already accepts.
  - The cost is a test, or a `_Nonnull` on the declaration, at each use across translation units, level-5 ones included.
- **Which pointers the default covers is decided by [#330](https://github.com/itsakeyfut/safec/issues/330)**, with a prototype and a measurement, and this record amended then. `_Nonnull` is read today only on a parameter's own pointer, and on a return, a local or a pointer inside another it is refused at every level ([`docs/frontend.md`](../frontend.md#where-the-nonnull-annotation-is-read)). Two things are fixed now. Where the frontend reads a nullability specifier does not depend on the level, so widening it widens it at every level. A pointer inside another, such as an element of `argv`, of which `argv[argc]` is null, is not believed non-null until `_Nullable` can be written on it.
- At levels 1 to 4, `_Nullable` restricts nothing; it is read as information.

`_Nullable` and `_Nonnull` are the nullability specifiers `clang` accepts as an extension: measured with clang 20.1.6, both are accepted, and `-pedantic-errors` turns the extension warning into an error, as it does for `_Nonnull` today. A name of this compiler's own was rejected because it would not pair with `_Nonnull`, leaving two spellings for one question. `clang` also accepts `_Null_unspecified` and `_Nullable_result` in C, measured the same way; what this compiler does with them is #330's to decide. Forbidding null was rejected for the reason above.

### Confirmation

Nothing holds this yet: no level-5 restriction is implemented. [#330](https://github.com/itsakeyfut/safec/issues/330) implements it and is to name the cases that hold each bullet above, and this section is to be rewritten then.

### Consequences

* Good, because at level 5 a reader sees where null is possible from the declaration, and the check never has to doubt a pointer the program said is non-null.
* Good, because the same source compiles with `clang`, which accepts both specifiers, subject to the warning below.
* Bad, because every pointer across a translation unit's boundary is `_Nullable` unless annotated, level-5 code on the other side included, and each use pays for a test or an annotation.
* Bad, because `clang` warns under `-Wnullability-completeness` about every unannotated pointer in a header that annotates any, so a header written for level 5 fails a `clang -Werror` build unless it annotates every pointer: measured with clang 20.1.6, three errors for a two-line header. `clang`'s `#pragma clang assume_nonnull begin` states the level-5 default for a region and builds clean; whether this compiler reads it belongs with mixing levels, [#331](https://github.com/itsakeyfut/safec/issues/331).
* Bad, because GCC reads neither specifier, as ADR-0037 already says of `_Nonnull`, and a level-5 file writes `_Nullable` wherever null is possible.
* What would reverse this: a whole-program nullability analysis that could see what code below level 5 returns, which would let the boundary believe more.

## More Information

* [ADR-0037](./0037-a-nonnull-parameter-is-believed-by-its-body-and-checked-at-every-call-in-its-translation-unit.md), `_Nonnull` today; [ADR-0049](./0049-level-5-restricts-what-may-be-written-and-levels-1-to-4-do-not.md), the level this restricts.
* [`docs/frontend.md`](../frontend.md#where-the-nonnull-annotation-is-read), where `_Nonnull` is read, and where `_Nullable` will be.
