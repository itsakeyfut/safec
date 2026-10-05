---
status: "accepted"
date: 2026-10-05
decision-makers: itsakeyfut
---

# At level 5 a pointer is non-null unless it is written `_Nullable`

## Context and Problem Statement

C has one way to say a pointer holds nothing, the null pointer, and nothing in a pointer's type says whether it may. At levels 1 to 4 the nullability check reads where a pointer was tested and answers a dereference it could not settle with a doubt; `_Nonnull` on a parameter (ADR-0037) is the only way a program states that a pointer is not null. Level 5 leaves nothing `Unknown` (ADR-0049), so a pointer whose nullness the check cannot settle has to be one the program says something about.

Forbidding null outright was considered and is not the direction: the free-and-null idiom, `free(p); p = 0;`, is itself a safety practice (ADR-0048), and C has no other way to say "absent" or "not found". What a level-5 program has to make visible is where null is possible.

## Decision Drivers

* A dereference of a pointer that may be null, believed safe, is row 6.
* The same source should compile with `clang` and with `safec` at every level.
* Annotating where null is possible should cost less than annotating where it is not, since most pointers in code written to be checked are not null.

## Considered Options

* **Forbid assigning or passing null at level 5.**
* **Non-null by default at level 5, and `_Nullable` where null is possible**, the default `_Nonnull` makes explicit today, inverted.
* **The same spelling under a name of this compiler's own**, such as an attribute.

## Decision Outcome

Chosen option: **non-null by default at level 5, and `_Nullable` where null is possible.**

- At level 5, a pointer declared without `_Nullable` may not be given a value that may be null: a null pointer constant, or a pointer the nullability check cannot show is not null. That is a level-5 diagnostic (ADR-0049), with a remedy naming `_Nullable` or the test that would settle it.
- A `_Nullable` pointer may hold null and may not be dereferenced, or passed where a non-null one is expected, until a test on the path settles it, which the nullability check already reads.
- `_Nonnull` keeps its meaning at every level; at level 5 it states the default.
- **At the boundary with lower-level code**, a pointer that comes from a declaration not written at level 5 and carries neither annotation is `_Nullable`: a function in another translation unit at level 1 has said nothing about what it returns, and believing it non-null is row 6. The cost is a test, or a `_Nonnull` on that declaration, at each such use.
- At levels 1 to 4, `_Nullable` restricts nothing; it is read as information.

`_Nullable` and `_Nonnull` are the nullability specifiers `clang` accepts as an extension: measured with clang 20.1.6, both are accepted, and `-pedantic-errors` turns the extension warning into an error, as it does for `_Nonnull` today. A name of this compiler's own was rejected because it would not pair with `_Nonnull`, leaving two spellings for one question. Forbidding null was rejected for the reason above.

### Confirmation

Nothing holds this yet: no level-5 restriction is implemented. [#330](https://github.com/itsakeyfut/safec/issues/330) implements it and is to name the cases that hold each bullet above, and this section is to be rewritten then.

### Consequences

* Good, because at level 5 a reader sees where null is possible from the declaration, and the check never has to doubt a pointer the program said is non-null.
* Good, because the same source compiles with `clang`, which accepts both specifiers.
* Bad, because every pointer from code below level 5 is `_Nullable` at the boundary, and each use pays for a test or an annotation.
* What would reverse this: a whole-program nullability analysis that could see what code below level 5 returns, which would let the boundary believe more.

## More Information

* [ADR-0037](./0037-a-nonnull-parameter-is-believed-by-its-body-and-checked-at-every-call-in-its-translation-unit.md), `_Nonnull` today; [ADR-0049](./0049-level-5-restricts-what-may-be-written-and-levels-1-to-4-do-not.md), the level this restricts.
* [`docs/frontend.md`](../frontend.md#where-the-nonnull-annotation-is-read), where `_Nonnull` is read, and where `_Nullable` will be.
