---
status: "accepted"
date: 2026-10-23
decision-makers: itsakeyfut
---

# A `#pragma clang assume_nonnull` region is read as a written `_Nonnull`

## Context and Problem Statement

ADR-0050 makes a pointer non-null by default at level 5 and believes nothing unannotated across a translation unit's boundary. It measured the cost on `clang`: a header that annotates any pointer gets `-Wnullability-completeness` warnings for every pointer it does not, so a header written for level 5 fails a `clang -Werror` build unless every pointer in it is annotated. `clang`'s `#pragma clang assume_nonnull begin` and `end` state the non-null default for a region and build clean. ADR-0053 left the question here, [#541](https://github.com/itsakeyfut/safec/issues/541), since the pragma states a default rather than choosing a level.

Nothing reads a `#pragma` yet: the preprocessor refuses every directive it does not read as `SC0104`, this one included. So nothing relies on this record, and it decides what the pragma will mean when the preprocessor reads it, so that the meaning is not settled by whoever types the preprocessor change.

## Decision Drivers

* A region the pragma covers is believed across a translation unit's boundary, so what it promises is a promise nobody wrote on any one declaration. Believing more than its author meant is how a null is dereferenced in silence.
* The same source should compile with `clang` and with `safec`, and mean the same to both.
* A promise written in the source and then dropped is what the frontend refuses (ADR-0050, on `_Null_unspecified`).

## Considered Options

* **Read it at every level, as a `_Nonnull` written on each single-level pointer of a declaration the region covers.**
* **Read it only inside a level-5 translation unit**, where it restates the default, and refuse it in a header a lower level reads.
* **Refuse it**, as today.

## Decision Outcome

Chosen option: **read it at every level, as a written `_Nonnull`.**

- Inside a region, a parameter's pointer and the pointer a function returns, where the declaration writes neither specifier, are read as if `_Nonnull` were written there. That is a promise, believed by the body and at every call as ADR-0037 believes a written `_Nonnull`, and believed across a translation unit's boundary as ADR-0050 believes one.
- **Only a single-level pointer.** `int *p` is covered and `int **pp` is not, at either level, which is what `clang` does: measured with clang 20.1.6, `take(0)` to a `void take(int **pp);` declared inside a region warns nothing, where `take(0)` to `void take(int *p);` warns under `-Wnonnull`. Believing more than `clang` reads would be a promise the author's other compiler never checked.
- **A written specifier wins.** `_Nullable` inside a region is `_Nullable`, as `clang` reads it.
- **What a region covers is what ADR-0050's default covers**: a declaration's own pointers, not a local and not a pointer inside another. A region adds a promise; at level 5 it changes nothing a declaration defined here already had, and at the boundary it is what lets a level-5 header be believed without a `_Nonnull` on each pointer.
- **A region that is not well formed is refused, and promises nothing**: a `begin` with no `end` in the same file, an `end` with no `begin`, a `begin` inside a region, and a region whose `begin` and `end` are in different files. `clang` refuses all four, measured with clang 20.1.6 (`'#pragma clang assume_nonnull' was not ended within this file`, `not currently inside '#pragma clang assume_nonnull'`, `already inside '#pragma clang assume_nonnull'`). Closing an unended region at the end of its file instead would believe declarations its author never put inside it.
- Any other `#pragma` stays refused as `SC0104` until something reads it.

Reading it only at level 5 was rejected because the header that needs the pragma is the one a lower level reads: the boundary is where a level-5 header's promises have to be believed, and refusing it there makes the header unreadable below level 5. Refusing it was rejected because its cost is ADR-0050's measured one, every pointer in a level-5 header annotated by hand, for a safety gain no larger than this option's: a wrong pragma is exactly as wrong as a wrong `_Nonnull` on the same declaration.

### Confirmation

**Nothing holds this yet, and nothing can**: the preprocessor reads no `#pragma`, and `no_directive_is_left_in_what_is_preprocessed` and `a_directive_that_is_not_read_keeps_the_code_it_was_assigned` in `crates/safec/src/preprocessor.rs` are what hold that a `#pragma` is refused rather than read today. The change that reads this one owes, before it lands, a case for each bullet above and the mutation that fails it, and replaces this paragraph with them.

### Consequences

* Good, because a header written for level 5 builds clean under `clang -Werror` and is believed here, with one pair of lines instead of a specifier on every pointer.
* Good, because the source means the same to `clang` and to `safec`, multi-level pointers included.
* Bad, because a region believed across a boundary is a promise covering declarations nobody annotated one at a time, and a wrong one is believed: a function in the region that returns null is dereferenced in silence, as one written `_Nonnull` would be.
* What would reverse this: a whole-program nullability analysis that could check what the other side of the boundary returns, which ADR-0050 names as its own reversal too.

## More Information

* [ADR-0050](./0050-at-level-5-a-pointer-is-non-null-unless-it-is-written-nullable.md), the default and the boundary this extends, and the measurement that asks for the pragma.
* [ADR-0037](./0037-a-nonnull-parameter-is-believed-by-its-body-and-checked-at-every-call-in-its-translation-unit.md), what a written `_Nonnull` promises.
* [ADR-0053](./0053-levels-are-not-mixed-inside-a-translation-unit.md), which left this question here.
* [`docs/safety-model.md`](../safety-model.md), where the boundary is described.
