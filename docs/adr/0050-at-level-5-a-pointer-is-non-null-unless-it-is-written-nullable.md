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
  - A hatch defined here promises what it writes and nothing by default. What its body could not prove is listed rather than reported (ADR-0038), so a default on one would be believed by every caller and asked by nobody, a promise no one wrote. A `_Nonnull` it writes is believed, as a hatch's `_Nonnull` parameter is: its boundary is its prototype.
- **The default covers a declaration's own pointers: a parameter's, and the pointer a function returns** ([#330](https://github.com/itsakeyfut/safec/issues/330), which amended this bullet). Both specifiers are read in those two places at every level, since where the frontend reads one does not depend on the level, and refused everywhere else ([`docs/frontend.md`](../frontend.md#where-a-nullability-specifier-is-read)).
  - A local is not covered. The check sees every write to it in the function it is reading, so covering it would prove nothing more and only move the report from the dereference to the assignment. Measured over the 678 corpus files with a throwaway build: of 1267 writes to a declared pointer local, 171 are established non-null, and 474 of the 484 files with such a write would need `_Nullable` on a local.
  - A pointer inside another, such as an element of `argv`, of which `argv[argc]` is null, is not covered, and is believed of nothing until a specifier can be written on it: believing a declaration of one would need every store through a pointer to a pointer checked against it.
  - The parser reads no storage class, so every function has external linkage and every unannotated parameter falls under the boundary above. What the default changes today is the pointer a function defined here returns.
- **`--safety strict` asks for more than a run delivers until Phase 8 wires the checks below it**, so a level-5 run is refused for that (ADR-0035) and reports what the default finds beside the refusal. The default is resolved where the IR is built, into a promise the IR carries, so nothing that reads the IR asks what level a run is at (ADR-0011).
- At levels 1 to 4, `_Nullable` restricts nothing; it is read as information.

`_Nullable` and `_Nonnull` are the nullability specifiers `clang` accepts as an extension: measured with clang 20.1.6, both are accepted, and `-pedantic-errors` turns the extension warning into an error, as it does for `_Nonnull` today. A name of this compiler's own was rejected because it would not pair with `_Nonnull`, leaving two spellings for one question. `clang` also accepts `_Null_unspecified` and `_Nullable_result` in C, measured the same way, and this compiler refuses both at every level as annotations it does not read (`SC0205`): read and dropped, `_Null_unspecified` would leave a pointer as if unannotated and `_Nullable_result` means nothing in C, and a promise written down and ignored is what the frontend refuses. Forbidding null was rejected for the reason above.

### Confirmation

Each bullet is held by corpus cases in `crates/safec/tests/cases/nullable`, and each mutation below was applied on its own and named the tests that failed.

- **The default, and the check that reports it.** Never making the default in `Lowering::body` fails `a_null_returned_where_level_5_promised_a_pointer_is_refused`, `an_allocation_returned_where_level_5_promised_a_pointer_is_doubted` and `the_end_of_a_function_level_5_promised_a_pointer_from_is_refused`, which go silent, and `a_result_level_5_promised_is_read_without_a_test`, which is doubted.
- **`_Nullable` takes it back.** Letting the default apply where `_Nullable` was written fails `a_nullable_return_is_tested_before_it_is_read`. Lowering `_Nullable` as a promise fails `a_nullable_parameter_is_doubted_by_its_body` and `a_null_passed_to_a_nullable_parameter_is_accepted`.
- **`_Nonnull` keeps its meaning at every level.** Never setting a written promise in `Lowering::body` fails `a_nonnull_return_is_checked_at_its_return_below_level_5`, `an_allocation_returned_from_a_nonnull_return_is_doubted`, `the_end_of_a_function_promising_a_nonnull_return_is_refused` and `a_result_promised_nonnull_is_read_without_a_test`.
- **A hatch promises nothing by default.** Making the default for a hatch, by dropping `!hatch` in `Lowering::body`, fails `a_hatch_promises_nothing_by_default_at_level_5`.
- **The boundary.** Making the default in `Lowering::declare_one` as well fails `a_result_of_a_function_only_declared_here_is_doubted_at_level_5`. Giving an unannotated parameter a promise at level 5 fails `a_parameter_of_a_function_defined_here_is_doubted_at_level_5` and `a_null_passed_to_a_function_only_declared_here_is_accepted_at_level_5`. Never setting a written promise in `Lowering::declare_one` fails `a_result_a_declaration_promises_is_believed`.
- **Levels 1 to 4 change nothing.** Computing the default at every level, in `driver.rs`, fails `a_result_of_a_function_defined_here_is_doubted_below_level_5` and every existing case whose pointer function may return null, `a_pointer_read_out_of_a_freed_table_and_returned` among them.
- **Where the specifiers are read.** Letting `Parser::placed` accept a specifier on a pointer inside a return fails `a_nonnull_on_a_pointer_inside_a_return_is_refused`, and the two refusals of `_Null_unspecified` and `_Nullable_result`, `an_unspecified_nullability_is_refused` and `a_nullable_result_is_refused`, fail when `Parser::unread_specifier` is made to answer that nothing is there.

### Consequences

* Good, because at level 5 a reader sees where null is possible from the declaration, and the check never has to doubt a pointer the program said is non-null.
* Good, because the same source compiles with `clang`, which accepts both specifiers, subject to the warning below.
* Bad, because every pointer across a translation unit's boundary is `_Nullable` unless annotated, level-5 code on the other side included, and each use pays for a test or an annotation.
* Bad, because `clang` warns under `-Wnullability-completeness` about every unannotated pointer in a header that annotates any, so a header written for level 5 fails a `clang -Werror` build unless it annotates every pointer: measured with clang 20.1.6, three errors for a two-line header. `clang`'s `#pragma clang assume_nonnull begin` states the level-5 default for a region and builds clean; whether this compiler reads it belongs with mixing levels, [#331](https://github.com/itsakeyfut/safec/issues/331).
* Bad, because GCC reads neither specifier, as ADR-0037 already says of `_Nonnull`, and a level-5 file writes `_Nullable` wherever null is possible.
* What would reverse this: a whole-program nullability analysis that could see what code below level 5 returns, which would let the boundary believe more.

## More Information

* [ADR-0037](./0037-a-nonnull-parameter-is-believed-by-its-body-and-checked-at-every-call-in-its-translation-unit.md), `_Nonnull` today; [ADR-0049](./0049-level-5-restricts-what-may-be-written-and-levels-1-to-4-do-not.md), the level this restricts.
* [`docs/frontend.md`](../frontend.md#where-a-nullability-specifier-is-read), where both are read and where they are refused.
