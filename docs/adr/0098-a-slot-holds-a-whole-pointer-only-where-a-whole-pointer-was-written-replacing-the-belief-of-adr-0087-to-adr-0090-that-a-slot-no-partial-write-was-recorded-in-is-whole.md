---
status: "accepted"
date: 2026-11-17
decision-makers: itsakeyfut
---

# A slot holds a whole pointer only where a whole pointer was written, replacing the belief of ADR-0087 to ADR-0090 that a slot no partial write was recorded in is whole

## Context and Problem Statement

ADR-0087 to ADR-0090 record a write of part of a pointer on the block and ask
that record where a pointer is read out of the block or it leaves holding
pointers. Anything they did not record was believed to be a whole pointer, so
a write route that did not record left a slot believed (#940, #1001). ADR-0097
moved the belief into the write state, every writer saying what it wrote, and
kept what each said as before.

## Decision Drivers

* A write route that forgets to say what it wrote should leave a slot doubted,
  not believed.
* No report may be lost at a point whose execution is undefined.
* What the narrowing costs, measured on correct programs, against #1018's
  bar: at most twenty new reports, read at their point.

## Considered Options

* Keep the belief: a slot is whole unless a partial write was recorded.
* A slot is whole only where a write that writes a whole pointer said so.

## Decision Outcome

Chosen: a slot is whole only where a whole pointer was written, by the
owner's decision on #940.

1. **The grants.** A store of a pointer type; a library copy of whole
   pointers out of memory no storage of which may hold part; `realloc`'s copy
   of the old block; and, believed, what code this check cannot read writes
   and the state a function starts in (#941's to narrow). Every other write
   says `Whole::Other`: a store of any other type, an aggregate's included, an
   assignment to a local of another type, every fill, and `calloc`'s zero
   bytes, since C17 does not make all bits zero a null pointer.
2. **A store this check cannot place says what it wrote only of the blocks its
   landing names.** What code this check cannot read names beside them keeps
   what it held: a value that is not a pointer landing in memory of pointers is
   undefined at the store (C17 6.5p7), so a defined program never makes such
   memory hold one. Not a library copy or a fill, whose bytes may set what
   allocated memory holds (C17 6.5p6), so the later read is what is undefined.
3. **Where a block leaves, what a block the site made before held**:
   `Known::part_before`, set where the site is made again from the state of
   the block it named, joined by `||`. The state is of the newest block, and a
   loop's older blocks may still hold part (#1019's review). A copy or a
   `realloc` asks it of its source beside the state, since what it moves may
   point at those blocks, and carries the record of where only from a source
   that may hold part.
4. **The report says what was written**: `WrittenAs::LessThanAPointer` in the
   words it had, `WrittenAs::NotAPointer` in words of its own at the write,
   whose place `Known::other_written` records, and `WrittenAs::Unplaced` at the
   door where no write is recorded.
5. **The doors do not ask memory narrower than a pointer**, as the notes do
   not: a pointer read out of an `int` is the bounds rule's.

### Confirmation

Measured on #940's branch by applying each change and restoring it:

* `Known::reborn` keeping nothing of what the site's block held silences
  `zero_bytes_in_a_block_an_earlier_turn_made_leave_after_the_newest_is_written_whole`
  and `part_of_a_pointer_in_a_block_an_earlier_turn_made_leaves_after_the_newest_is_written_whole`.
* A leaving door asking `part_before` alone silences seventy seven cases,
  `a_heap_block_from_calloc_written_at_one_element_and_returned_is_not_doubted_as_unwritten`
  among them.
* `calloc` writing a whole pointer silences
  `a_pointer_read_out_of_memory_calloc_made_is_not_reported_as_indeterminate`
  among eight.
* The copy's write ignoring its source silences
  `a_partial_pointer_in_a_block_copied_whole_into_another_that_leaves_is_reported`
  among five; `other_written` not carried gives
  `a_block_calloc_made_copied_whole_and_read_back_is_reported_at_the_calloc`
  a second report, at the read, in the words of a write nobody recorded.
* A copy or `realloc` asking the newest block of its source alone, and not
  what the site made before, silences
  `part_of_a_pointer_in_a_block_an_earlier_turn_made_leaves_through_a_copy_of_the_newest`
  and its `realloc` twin; a fill over a slot a pointer was stored through
  leaving that pointer believed read back silences two reports in
  `a_pointer_made_of_bytes_a_fill_or_a_copy_wrote_is_no_pointer_this_check_follows`.
  Each was found by review.
* `part_before` joined as `&&` silences the same two, under `||` joined as `&&`.
* A fill writing a whole pointer, or a whole one not noted, silences
  `a_memset_of_a_whole_table_of_pointers_that_leaves_is_reported_as_no_pointer`
  among four.
* The doors asking memory narrower than a pointer reports
  `a_proof_at_an_earlier_argument_of_a_library_call_is_not_displaced_by_a_doubt_at_a_later_one`
  among four and two route rows.
* A store it cannot place saying `Other` of what code outside reaches reports
  `an_integer_stored_through_a_load_lands_nowhere` among six.
* **Held by nothing**: a store of a type that is not a pointer, and an
  assignment to a local of one, saying `Other` themselves, since on the
  targets the corpus runs every such type this frontend reads is narrower
  than a pointer and noted so; a copy or fill this check cannot place keeping
  what code outside reaches as it was, as a store does; and the
  words of `WrittenAs::Unplaced`, which no program reaches now that every
  write saying `Other` records where. Each says so where it is.

### Consequences

* Good, because a write route added later that does not say what it wrote is
  doubted where a pointer is read out of what it wrote.
* Bad, because twenty one reports are new over the corpus and the route
  matrix as they stood, seventeen cases and four route rows, each at a
  `calloc` or a fill whose bytes a pointer is later read out of or leaves
  as, by the two decisions in rule 1: one over #1018's bar of twenty, and
  most on programs correct on every real target, `calloc` and return among
  them. None is lost.

## Pros and Cons of the Options

### Keep the belief

* Good, because nothing moves.
* Bad, because a route that does not record is believed, which is what #1001
  found.

## More Information

#1018 (the measurement's bar), #1019 and ADR-0097 (the state this narrows),
#941 (the boundary beliefs). See ADR-0087, ADR-0088, ADR-0089 and ADR-0090.
