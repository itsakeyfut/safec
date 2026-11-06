---
status: "proposed"
date: 2026-11-06
decision-makers: itsakeyfut
---

# What leaves a function is asked by how it was declared and what wrote it

## Context and Problem Statement

Four silences were left by the doors ADR-0071 made alike, each measured on main
and each an undefined program built with nothing said:

- `char *strchr(int c, char *s); strchr(1, 0);` builds silent: both checks read
  a call to a library name at its row's argument positions, whatever the unit
  declared (#821).
- `char b[8]; memcpy(out, b, 8);` with `out` an `int **` builds silent: a local
  that holds no pointer starts out believed written (#822).
- `int **b = malloc(8); return b;` builds silent, where a read of `*b` in the
  same function is `SC0411`: a block nothing wrote is asked nothing where it is
  handed back (#823).
- `void *d = out; memcpy(d, &i, 4);` with `out` a `void **` builds silent: the
  belief a value arrived with says `0` alike for a level holding a `void *` and
  for one holding no pointer (#818).

## Decision Drivers

* Each is row 6: `safec` says nothing about a program C leaves undefined.
* Each fix must not move a report onto a program C defines that the corpus
  holds, and where one does, the cost is measured and named.
* One question is answered in one place, so that the memory check and the
  nullability check, both readers of the library table, cannot read one
  declaration two ways (ADR-0063, ADR-0072).

## Considered Options

Per rule, the option taken and the one rejected; the rejected ones are in Pros
and Cons below.

## Decision Outcome

1. **A library name declared with a shape other than its row's is refused where
   the memory check runs**, as `SC0320`, as a definition of one is `SC0319`
   (ADR-0071 rule 1). The shape is the number of parameters and whether each is
   a pointer or an integer, since this subset has neither `const` nor `size_t`
   and the corpus declares `memcpy` as `(void *, void *, int)`. A declaration
   with no prototype is refused too: it is compatible with any (C17 6.7.6.3
   p15), but nothing then compares a call's arguments with the row, and a call
   whose arguments differ is undefined (6.5.2.2 p6), so `char *strchr();
   strchr(1, 0);` would build silent. Where IR reaches
   either check without that refusal, a call through such a declaration is read
   as any call, by one function both checks read (#821).
2. **Every local but a parameter starts out unwritten**, and is asked where it
   is read as a pointer, by a load or by a copy into memory that leaves; a read
   of an integer by name is asked nothing, as before (#822). A local that holds
   no pointer is kept whole, so a write of part of it writes all of it, which
   leaves a read of the rest unasked; that is #826's.
3. **A `return` of a block certainly unwritten is doubted** where the function's
   return type points at a pointer, as `SC0411` with words of its own: the
   caller reads pointers out of it and this check reads one function at a time
   (ADR-0027). Certainly, not possibly: a block on a path where its allocation
   failed joins the written one as possibly unwritten, measured as three false
   reports in the corpus. The cost is that a block left unwritten on a path
   where its allocation succeeded is silent too, which only knowing the pointer
   is not null at the `return` tells apart (#828). Only a `return`: a block
   handed to a call is how a callee is given memory to fill (#823).
4. **A value records whether the type it arrived as holds a pointer in its first
   level**, beside the width it believes there, and an operand of a library copy
   is asked by it: `void **` holds `void *`s, `int *` holds none, which the width
   `0` both say. Joined by union, so a path that says pointers asks (#818).

### Confirmation

Proposed until the last of #821, #822, #823 and #818 lands; each pull request
adds its rule's cases and the mutation that fails each here.

1. `library::misdeclares_library` answering `false` fails
   `a_library_name_declared_with_its_parameters_swapped_is_refused`, the
   corpus case and the `sema.rs` test of that name, and the block-scope and
   too-few cases beside the latter; reading no prototype as a match fails
   `a_library_name_declared_without_a_prototype_is_refused`; refusing whatever
   the safety level fails
   `a_library_name_declared_unlike_its_row_is_refused_only_where_the_memory_check_runs`;
   and a row's shape written wrong fails
   `each_library_name_declared_with_its_own_shape_is_not_refused` for that
   name. In IR, `library::read_as_library` skipping the shape fails
   `a_library_name_declared_with_another_shape_is_read_as_any_call` in
   `crates/safec-ir/tests/nulls.rs`, `Allocations::library` looking the row up
   by name alone fails `a_free_declared_with_another_shape_is_read_as_any_call`
   in `crates/safec-ir/tests/freed.rs`, and either side reading a `char` as no
   integer fails `a_library_name_declared_over_a_char_is_not_refused` or
   `a_library_call_declared_over_a_char_is_read_by_its_row`.
2. `Allocations::unwritten_at_entry` keeping only a local that holds a
   pointer fails `a_char_buffer_nothing_wrote_copied_into_caller_memory_is_doubted`
   and `a_char_buffer_nothing_wrote_read_as_a_pointer_is_reported`; `StorageLive`
   keeping its old rule for a local that holds no pointer fails
   `a_char_buffer_written_in_one_turn_is_unwritten_again_in_the_next`, and the
   two together fail
   `an_integer_local_made_again_in_a_loop_and_read_as_a_pointer_is_reported`;
   `Known::written_local` writing nothing fails
   `a_char_buffer_written_whole_and_read_as_a_pointer_is_not_reported`; and
   `report::unwritten_read` dropping its `holds_a_pointer` filter fails
   `an_integer_local_whose_address_is_taken_read_before_anything_wrote_it_is_not_followed`.

### Consequences

* Good, because each of the four programs above is reported.
* Bad, because rule 1 refuses what `clang -std=c17` builds with a warning
  (`-Wincompatible-library-redeclaration`), where the memory check runs.
* Bad, because rule 3 doubts a function that allocates a table of pointers and
  hands it back unfilled for the caller to fill, a common shape.
* Bad, because rule 2 turns a case written to hold the old boundary, an
  `int` made again in a loop and read as a pointer, into a report, which it now
  is, renamed
  `an_integer_local_made_again_in_a_loop_and_read_as_a_pointer_is_reported`.
* What would reverse rule 3: a check that follows a pointer across a call, so
  the caller's read is where it is asked.

## Pros and Cons of the Options

### Rule 1, rejected: read a misdeclared name as any call, and refuse nothing

* Good, because nothing `clang` builds is refused.
* Bad, because `strchr(1, 0)` through the misdeclaration is then any call and
  says nothing, row 6, and the declaration C makes undefined is never said.

### Rule 2, rejected: keep the boundary at locals that hold a pointer

* Good, because nothing moves.
* Bad, because a `char` buffer read as a pointer stays silent, and nothing in the
  representation needed the boundary: `Known::init` holds every local already.

### Rule 3, rejected: hold, until a check follows a pointer across a call

* Good, because no allocator idiom is doubted.
* Bad, because `int **b = malloc(8); return b;` and the same after a copy of half
  a pointer stay silent with no date on what would end it.

### Rule 4, rejected: give `Levels` a pointer bit at every level

* Good, because the ambiguity is gone at every level, not the first.
* Bad, because `contents_at_most` is read and written in fifty-four places across
  ADR-0060's machinery, for a question asked at one.

## More Information

* [ADR-0071](./0071-a-write-spelled-as-a-copy-or-through-a-loaded-pointer-is-asked-at-its-doors-as-the-store-it-is.md),
  whose doors these four silences were found beside.
* [ADR-0072](./0072-an-argument-a-library-function-reads-or-writes-through-is-asked-and-refined-as-a-dereference.md),
  the second reader of the library table that rule 1 serves.
* [ADR-0069](./0069-a-pointer-read-out-of-memory-nothing-wrote-is-reported-where-it-is-read-and-a-loop-may-run-zero-times-unless-it-is-shown-to-run.md),
  whose rule 1 asks a read as a pointer, which rule 2 extends to every local,
  and whose rule 2 started only a local that may hold a pointer unwritten,
  which rule 2 here widens to every local but a parameter.
