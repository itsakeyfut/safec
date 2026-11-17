---
status: "accepted"
date: 2026-11-17
decision-makers: itsakeyfut
---

# Whether a slot holds a whole pointer is carried in its write state, and the block's record of a partial write names the place

## Context and Problem Statement

ADR-0087 records a write of part of a pointer on the block, in
`Known::part_written`, and asks that record where the block is later held
as memory of pointers; ADR-0088 to ADR-0090 carry it through copies,
`realloc` and locals, and ADR-0089 says which of its two spans a read asks.
The record was the belief: a slot was believed to hold a whole pointer
unless some write had recorded otherwise. A write route that did not record
left the slot believed, and nothing made a route added later say what it
wrote (#1001).

## Decision Drivers

* A write route that forgets to say what it wrote should fail to compile,
  not leave a slot believed.
* No report may be lost at a point whose execution is undefined.
* The belief itself is #940's to change, measured on its own.

## Considered Options

* Keep the belief in the block's record.
* Carry it in the write state, every write saying what it wrote, the record
  naming the place.

## Decision Outcome

Chosen: the write state carries it, by the owner's decision on #940, which
split this from the change of belief (#1019).

1. **`Init` carries a `Whole` in each of its states**, `Unwritten`, `Maybe`
   and `Written`: whether every write that may have landed wrote a whole
   pointer. They meet as `Other` where either side is; a side nothing
   landed in is `Pointer` and takes no part. An unwritten slot carries it
   too, since a copy shorter than a pointer leaves a fresh block fresh
   (ADR-0057) and its part still has to be asked where the block leaves.
2. **Every writer takes a `Whole`**, and each passes what is believed
   today: a narrow store, a partial copy or fill, and a copy or `realloc`
   out of memory that may hold part of a pointer say `Other`, through the
   one body that records them (`Known::note_part_written`,
   `Known::carry_part_written`) and the copy's own answer
   (`Allocations::copy_writes`); every other write, and every belief at a
   boundary, says `Pointer`. Code this check cannot read is believed to
   write whole pointers without writing over what was there.
3. **The partial-pointer doors ask the state; the record names the place.**
   A whole pointer written over the part since ends the report. Where a
   block leaves after its site was made again, `now` is gone and `ever`
   answers alone, as ADR-0089 keeps it, since the state is of the block
   made since.
4. **The third set of bits is counted in the memory bound** by its size.

### Confirmation

Measured on #1019's branch by applying each change and restoring it:

* `Init::may_hold_part` answering `false`, or the record's note not writing
  the state, fails about ninety cases,
  `a_memset_of_part_of_a_pointer_into_a_table_that_leaves_is_reported`
  among them.
* An unwritten slot keeping no part fails
  `a_partial_pointer_copied_into_a_block_before_it_is_held_as_pointers_is_reported`
  among fifty nine.
* Code this check cannot read writing a whole pointer over what was there
  fails `an_int_stored_into_a_table_of_pointers_that_leaves_is_reported`
  among thirty.
* A mixed pair meeting as `Pointer` fails
  `a_partial_pointer_copied_on_one_arm_and_read_after_the_join_is_reported`
  among twelve; `Maybe` meeting as `Pointer` fails
  `an_init_join_is_commutative_idempotent_associative_and_monotone` and
  `a_table_of_pointers_an_arm_stored_an_int_in_handed_to_a_call_is_asked_how_wide`.
* A copy not asking its source fails
  `a_partial_pointer_in_a_block_copied_whole_into_another_that_leaves_is_reported`;
  `realloc` not carrying the state fails
  `a_partial_pointer_in_a_block_moved_by_realloc_that_leaves_is_reported`.
* The doors taking every recorded block as holding part reports
  `a_whole_pointer_stored_over_part_of_one_before_the_block_leaves_is_not_reported`;
  a store at one slot writing the block whole silences
  `part_of_a_pointer_in_a_slot_no_whole_one_was_stored_over_is_reported`.
* **Held by nothing**: the leaving doors answering by `ever` where `now`
  is gone, and the copy's own answer for a length that ends part way
  through a pointer. Each says so where it is.

### Consequences

* Good, because a writer that does not say what it wrote does not compile,
  and #940 changes what each says rather than adding a route.
* Good, because a whole pointer stored over the part is no longer reported:
  the program reads the pointer it wrote.
* Bad, because every slot keeps a third bit, counted in the memory bound,
  which moves the estimate the too-large cases print.

## Pros and Cons of the Options

### Keep the belief in the block's record

* Good, because nothing moves.
* Bad, because a route that does not record is believed, which is what
  #1001 found.

## More Information

#1001 (the structural parent), #1019 (this), #940 (the change of belief
that follows). See ADR-0087, ADR-0088, ADR-0089 and ADR-0090.
