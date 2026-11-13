---
status: "accepted"
date: 2026-11-11
decision-makers: itsakeyfut
---

# A store replaces a slot only where it certainly lands there, and is a may-write where its position is unknown

## Context and Problem Statement

What a block holds is a union that nothing replaces (`Cell::absorb`),
because the slots of a block were not told apart. So two cases are
answered wrongly, in opposite directions:

- `free(*b); *b = 0;` still believes the freed pointer is in the slot, and
  the program is doubted (#656, row 4).
- A store at an element this check cannot name writes the whole block
  (ADR-0081 rule 2), and a block written at one element is believed written
  everywhere (#893, row 6).

ADR-0092 tells the slots apart.

## Decision Drivers

* A replacement is a claim that the old value is gone. Made where it is not
  certain, it is a silence (row 6).
* A site stands for every block one allocation call makes. Inside a loop,
  that is several blocks.

## Considered Options

* Replace a slot where the store certainly lands in it, and treat a store at
  an unknown position as a may-write.
* Never replace, and keep the may-write alone.

## Decision Outcome

Chosen: replace where certain, by the author.

1. **A store replaces a slot's contents and its written record** where all
   of these hold:
   - it lands in exactly one site;
   - at a known `At::Bytes` covering the whole slot;
   - and the site certainly stands for one block: `Allocations::made_once`,
     and not a parameter's site, an opaque call's result, or a `realloc`'s
     result.

   `Known::contents` was to be kept per slot for this; it is not, see below.

   **Taken in the narrow form, by the author** (#656): `contents` stays per
   site, and a store replaces it only where the block is one slot, so the
   slot is the block. `Allocations::replaces` asks one dereference, one
   certain site, `At::Bytes(0)`, a width equal to the block's exact extent,
   and `made_once`. An exact extent already rules out a parameter's site and
   an opaque call's result. A `realloc`'s result is not ruled out: it is
   one block, what is read through the old pointer is reported there, and
   the old site keeps its own record; excluding it held no case, measured.
   A block of several slots keeps adding until `contents` is kept per slot.
2. **Anywhere else a store adds to what the slot may hold**, as today.
3. **A store at an unknown position is a may-write** of every slot it may
   reach: it makes no slot written that was not. This replaces ADR-0081
   rule 2. It holds for a store through a pointer this function placed and
   for one through a pointer read back out of memory alike; #726's decision,
   that the latter writes the one block it certainly lands in, stands for a
   block kept whole. A call this check cannot read is still believed to write
   what it is handed, so `Held::array_edge` and `Known::stored_in_part` stay
   for the call route: without `array_edge`,
   `a_local_array_reached_by_a_call_through_a_moved_pointer_stored_in_an_allocation_is_doubted`
   goes silent. Retiring them waits on that boundary belief, not on this
   record. **And a place kept whole is written for certain only by a store
   as wide as it** (#918's review): once ADR-0092 counted positions, a store
   of one pointer into a block of 65 wrote the whole block, and `b[5]` was
   read in silence where an unknown position had doubted it. The same holds
   of a local array kept whole, and of a block `realloc` grew, whose slots
   past the old size are unwritten.
4. **Precision for a loop that fills a block comes back only through a
   proof.** It does not come back through the store believing itself whole.
   Until such a proof exists, a block filled in a loop is doubted where it is
   read, a doubt the reader sees.

### Confirmation

Each measured on the #918 branch by applying the change and restoring it.
The cases are in `tests/cases/`, with their mutations in their notes in
`tests/cases.rs`. "Certainly one block" is held by a case rather than a
route-matrix row, since the matrix has no axis for a block made in a loop.

* **Rule 1**, each condition of `Allocations::replaces`:
  - never replacing fails `a_block_of_one_slot_overwritten_after_what_it_held_was_freed_is_returned_without_a_doubt`,
    its call form, `a_slot_stored_again_after_what_it_held_was_freed_is_not_doubted`
    and two more;
  - skipping `made_once` fails `a_block_made_in_a_loop_overwritten_whole_keeps_what_an_earlier_turns_block_held`
    and two more;
  - skipping the extent fails `a_stored_bound_is_not_raised_by_a_later_store_since_slots_are_not_told_apart`;
  - skipping the position fails `a_store_at_an_index_not_known_into_a_block_of_one_slot_does_not_replace_what_it_holds`.
* **Rule 3.** The one-step store writing a block kept per slot whole at an
  element not known fails `a_heap_block_filled_by_a_loop_that_stops_short_is_doubted_where_returned`
  and three more. The same through a pointer read back out of memory fails
  `a_heap_block_written_through_a_pointer_read_back_out_of_memory_is_doubted`.
  The `built::write_effects` arm for a local array fails
  `a_local_array_filled_by_a_loop_is_doubted` and three more.
* **Rule 4** is the cost those cases hold: each loop-fill case is doubted,
  and its note says what would make it silent.

### Consequences

* Good, because a slot overwritten for certain is no longer doubted, and an
  element nothing named is no longer believed written.
* Bad, because a block filled in a loop is doubted until a loop proof is
  written. The prototype on #893 measured three cases moving and no exit
  code; the change measured seven, each renamed from `..._is_not_doubted`
  to `..._is_doubted`, and one route row's control, accepted as a cost.

## Pros and Cons of the Options

### Never replace

* Good, because no claim of certainty is made.
* Bad, because #656's false report stays.

## More Information

* [ADR-0081](./0081-a-heap-block-of-pointers-of-a-constant-size-is-written-per-element-as-a-local-array-is.md),
  whose rule 2 this replaces.
* [ADR-0092](./0092-a-position-inside-an-object-is-one-byte-offset-and-a-places-written-record-is-kept-per-pointer-sized-slot.md),
  which tells the slots apart.
