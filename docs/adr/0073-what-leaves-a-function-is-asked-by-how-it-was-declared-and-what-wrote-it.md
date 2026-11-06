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

Their reviews found three more, each the edge of one of the four rules:

- `char b[8]; b[0] = 0;` then `b` read as a pointer builds silent: a write of
  part of a local kept whole writes all of it (#826).
- `if (!b) return 0; if (c) *b = p; return b;` builds silent: a block left
  unwritten on a path where its allocation succeeded is not certainly
  unwritten (#828).
- `void **q = malloc(16); void *d = q; memcpy(d, &i, 4);` builds silent, and so
  does `int **q`: an allocator's result arrives as no type (#830).

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
   of an integer by name is asked nothing, as before (#822). **A write of part
   of a local kept whole leaves it possibly unwritten** rather than written:
   a store at an index this check reads into an array of integers or
   characters, and a library copy of a length it reads that is shorter than
   the local. A store at an index it cannot read, and a copy of a length it
   cannot read, still write it whole, so a loop that fills it is not doubted.
   An array of pointers is written whole by a store, which writes a pointer
   whole, so one kept whole is put in doubt only by a copy that ends part way
   through a pointer, and one kept per element is written by a copy only in
   the elements it covers whole (#826).
3. **A `return` of a block unwritten is doubted** where the function's
   return type points at a pointer, as `SC0411` with words of its own: the
   caller reads pointers out of it and this check reads one function at a time
   (ADR-0027). Certainly unwritten in general, since a block on a path where
   its allocation failed joins the written one as possibly unwritten, measured
   as three false reports in the corpus (#823). **Possibly unwritten where the
   nullability check established the pointer not null at the `return`**, since
   then no path that reaches it is one where the allocation failed (#828): the
   nullability check hands that fact over beside the null it hands over
   already (ADR-0027), and a wrong answer from it is a doubt too many, not one
   too few. Only a `return`: a block handed to a call is how a callee is given
   memory to fill (#823).
4. **A value records whether the type it arrived as, or the local it is held
   in, holds a pointer in its first level**, beside the width it believes
   there, and an operand of a library copy is asked by it: `void **` holds
   `void *`s, `int *` holds none, which the width `0` both say. Joined by
   union, so a path that says pointers asks (#818). Set where a value arrives
   from a type, a parameter, a load or an opaque call's result, and added by
   every assignment into a local whose declared type says so, so that an
   allocator's result, which arrives as no type (ADR-0060), says it once it is
   held as a `void **` or an `int **` (#830). The width ADR-0060 believes is
   not changed: a pointer held so is asked whether it starts an object. **On
   the value, not on the allocation**, so a block reached by a pointer that
   never held the belief is still asked nothing: an alias made before the
   typed hold, a copy before it, a block handed to a function that takes it
   as a `void **`, and one stored into a `void **` slot with no typed local;
   that is #832.

### Confirmation

Each of #821, #822, #823 and #818 added its rule's cases and the mutation that
fails each here. Proposed still, since #828 and #830 may change what rules 3
and 4 decide, and a record accepted before them would have to be superseded
rather than amended.

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
3. `report::report_returned_unwritten` never doubting fails
   `a_block_nothing_wrote_returned_as_a_table_of_pointers_is_doubted` and
   `a_block_a_copy_wrote_half_a_pointer_into_is_doubted_where_it_is_returned`;
   doubting every block returned, written or not, fails
   `a_block_written_before_it_is_returned_is_not_doubted`; doubting a block
   possibly unwritten fails
   `a_block_written_on_the_path_its_allocation_succeeded_is_not_doubted`;
   doubting whatever the return type fails
   `a_block_returned_as_a_table_of_integers_is_not_doubted`; doubting only
   when every block the pointer may name is unwritten fails
   `a_pointer_that_may_name_a_block_nothing_wrote_returned_is_doubted`; and
   asking the return type's pointee without looking through an array fails
   `a_block_of_arrays_of_pointers_nothing_wrote_returned_is_doubted`.
4. `Allocations::first_level_holds_a_pointer` never answering `true` fails
   `a_copy_of_an_int_into_a_void_pointer_pointer_behind_a_void_pointer_is_asked`
   and the five cases after it; answering `true` for every pointer type fails
   `a_byte_copy_of_an_int_through_an_int_pointer_parameter_behind_a_void_pointer_is_not_asked`;
   and not reading through an array fails
   `a_copy_into_a_void_pointer_made_from_a_pointer_to_an_array_of_void_pointers_is_asked`.
   `Held::joined` keeping the field by intersection fails
   `a_copy_into_a_void_pointer_that_may_have_arrived_as_a_void_pointer_pointer_is_asked`,
   `Held::accumulated` dropping it fails
   `a_copy_into_a_void_pointer_moved_along_a_void_pointer_pointer_is_asked`, and
   `Held::clear` keeping it fails the unit test
   `clearing_a_local_leaves_it_no_site_and_no_edge`. A parameter leaving it
   unset fails the base, one-path, array and arithmetic cases above; the load
   leaving it unset fails
   `a_copy_into_a_void_pointer_pointer_read_out_of_memory_behind_a_void_pointer_is_asked`,
   an opaque call's result leaving it unset fails
   `a_copy_into_a_void_pointer_pointer_a_call_returned_behind_a_void_pointer_is_asked`,
   and a library copy's read setting it fails
   `a_void_pointer_a_library_copy_wrote_is_not_believed_to_point_at_pointers`.

### Consequences

* Good, because each of the four programs above is reported.
* Bad, because rule 1 refuses what `clang -std=c17` builds with a warning
  (`-Wincompatible-library-redeclaration`), where the memory check runs.
* Bad, because rule 3 doubts a function that allocates a table of pointers and
  hands it back unfilled for the caller to fill, a common shape.
* Bad, because rule 2 doubts `char b[8]` written one element at a time at
  every index and then read as a pointer, since a local kept whole cannot say
  which parts were written (#826).
* Bad, because rule 3 doubts `if (b) *b = p; if (!b) return 0; return b;`,
  where the pointer is shown not null only after the join that made the block
  possibly unwritten (#828).
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

### Rule 2, rejected: keep every array of at most `ELEMENTS_KEPT` elements per element

* Good, because a buffer written one element at a time is not doubted, and a
  read as a pointer asks exactly the elements it spans.
* Bad, because `Held::element` is counted in innermost elements and would
  change for every array of integers or characters, a wide change to the
  machinery #667 built, where a mistake is a silence.

### Rule 2, rejected: put an array of pointers kept whole in doubt on any store at an index

* Good, because a read of an element no store wrote is doubted.
* Bad, because `big[0] = &x; p = big[0];` is doubted, a common shape, and the
  element read cannot be told from the one written in a local kept whole.

### Rule 3, rejected: hold, until a check follows a pointer across a call

* Good, because no allocator idiom is doubted.
* Bad, because `int **b = malloc(8); return b;` and the same after a copy of half
  a pointer stay silent with no date on what would end it.

### Rule 3, rejected: doubt a block possibly unwritten wherever it is returned

* Good, because nothing is asked of the nullability check.
* Bad, because three corpus cases were doubted on the path where the
  allocation failed, `if (m) *m = x; return m;` among them, which C defines.

### Rule 4, rejected: give `Levels` a pointer bit at every level

* Good, because the ambiguity is gone at every level, not the first.
* Bad, because `contents_at_most` is read and written in fifty-four places across
  ADR-0060's machinery, for a question asked at one.

### Rule 4, rejected: add it only where an allocator's result is held

* Good, because ADR-0060's rule for an allocator's result moves least.
* Bad, because `void f(void *v) { void **q = v; void *d = q; ... }` stays
  silent, the same loss by another route.

### Rule 4, rejected: record on the allocation the type it was held as

* Good, because it survives a pointer stored in memory and read back, and
  answers an alias made before the typed hold and a block stored into a
  `void **` slot with no typed local, which the local's type does not (#832).
* Bad, because it adds a fact to every site for a question one door asks, and
  still answers nothing where the copy comes before anything typed reaches
  the block, so it does not close #832 either.

## More Information

* [ADR-0071](./0071-a-write-spelled-as-a-copy-or-through-a-loaded-pointer-is-asked-at-its-doors-as-the-store-it-is.md),
  whose doors these four silences were found beside.
* [ADR-0072](./0072-an-argument-a-library-function-reads-or-writes-through-is-asked-and-refined-as-a-dereference.md),
  the second reader of the library table that rule 1 serves.
* [ADR-0069](./0069-a-pointer-read-out-of-memory-nothing-wrote-is-reported-where-it-is-read-and-a-loop-may-run-zero-times-unless-it-is-shown-to-run.md),
  whose rule 1 asks a read as a pointer, which rule 2 extends to every local,
  and whose rule 2 started only a local that may hold a pointer unwritten,
  which rule 2 here widens to every local but a parameter.
