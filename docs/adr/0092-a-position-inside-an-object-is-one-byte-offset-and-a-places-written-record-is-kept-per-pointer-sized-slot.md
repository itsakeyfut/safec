---
status: "accepted"
date: 2026-11-11
decision-makers: itsakeyfut
---

# A position inside an object is one byte offset, and a place's written record is kept per pointer-sized slot

## Context and Problem Statement

Where a value points inside its object, and what a place's bytes hold, are
answered by fields added one defect at a time:

- **position** is held by `Offset` (ADR-0036), `Held::element` for a local
  array (#667) and `Held::site_element` for a heap block (#873);
- **whether a place was written** is held by `Known::init`, then
  `element_init` per element of a local array, then `block_elements` per
  element of a heap block, with a stamp that every whole-block writer has to
  keep (ADR-0081);
- **whether part of a pointer was written** is held by `part_written` and
  `part_written_since_made` (ADR-0087 to ADR-0090).

The same question is asked from six issues' sides: #703, #894, #893, #656,
#913 and #917. None of these fields can say that element 1 of a block is
inside it (#703), or that a `char` store changed one byte of a pointer
(#917). Each tells a local from a block where the question does not.

## Decision Drivers

* One question answered in one place, for a local and a block alike, so
  that structs (Phase 11) add members to it rather than fields beside it.
* A distance this check cannot represent is never taken as one it can: a
  wrong `Bytes` proves an access inside an object, and that is a silence
  (row 6).

## Considered Options

* One byte offset for a value's position, and a per-slot record of what a
  place's bytes hold.
* One per-element record (#894) and the positions as they are.

## Decision Outcome

Chosen: the byte offset and the per-slot record, by the author.

1. **A value's position is `At`: `Bytes(n)`, `Moved`, or `Unknown`.**
   - `Moved` is *certainly off the start, distance not known*, which is
     today's `NonZero` and keeps `a_free_offset_on_both_arms_is_still_proved`.
   - `Bytes(a)` joined with `Bytes(b)`, `a != b`, is `Moved` where both are
     non-zero, and `Unknown` otherwise.
   - A move by a constant counts the pointee's size. A move the check cannot
     compute, or one that overflows, is `Unknown`, never `Bytes`.
   - `At` replaces `Offset`. **`Held::element` and `Held::site_element`
     stay fields of their own**, by the author's decision on #918: they
     carry which element a value read out of one local names (#706),
     where a load's position is `Unknown` by ADR-0045. Read off `At`
     instead, twelve cases moved, and some lost an `SC0411` for an element
     nothing wrote, a silence; joining them by `At`'s rule alone moved
     none.
   - A null joined with a moved value stays `Unknown`, as ADR-0036 accepts.
2. **Not a position: `inner_address` and `array_edge`.** They say an edge may
   name the wrong object, and that an edge survives only by #639's rule. They
   stay as they are until rule 3 of ADR-0094 replaces the second.
3. **A place's written record is per pointer-sized slot, for a local and a
   block alike, keyed by `Loc`.** Each slot is `Unwritten`, `Maybe` or
   `Written`, and no slot is `Part`: see the last point of this rule.
   Beside the slots the record keeps where its unwritten bytes came from,
   the copy or `realloc` that carried them (#752), which decides no
   conclusion, only what the report about a read says.
   - The whole-place answer is computed from the slots rather than stored
     beside them, so the stamp goes.
   - The cap stays: 2 to `ELEMENTS_KEPT` slots, and one slot past it.
   - The joins stay: forward and back edge, and a site's `Unheld` side.
   - It replaces `init`, `element_init` and `block_elements`, as
     `Known::written`.
   - **Not `part_written` and `part_written_since_made`**, by the author's
     decision on #918, step 9. They stay a record per place of the earliest
     write that may have left part of a pointer, with their two lifetimes,
     and `Part` is not a slot's state. A slot's `Part` would need a join
     with the other three states, a home for one-slot places, and a place
     for the lifetime kept across a site made again, none of which was
     decided; the writers below reach the record as it is.
4. **The record's two lifetimes stay.** One kept across a site made again,
   for the doors a block leaves by. One reset there, for a read inside the
   function (ADR-0089).
5. **Every writer records part of a pointer, however it is spelled.** A
   library copy, a store narrower than a pointer and a `memset` each record
   it where they cover part of a slot, in a block or local wide enough to
   hold a pointer (#913, #917):
   - only where the write is placed, one dereference at a position counted
     from the start: a `char` loop's `c[i] = 0;` may fill every byte of
     the slot over the loop, and is not recorded;
   - nothing in an object narrower than a pointer.

   **Which bytes were written is not kept.** So a pointer written a byte at
   a time at every position is still recorded, a false report the reader
   sees, which a store that certainly lands replacing its slot (ADR-0094)
   removes. A `char` store into memory of pointers is recorded, which
   replaces the `char` exemption the design of #913 had.
6. **One rule asks the record at every door**, a `PartialDoor` matched
   exhaustively (#914). A call asks a handed block by its held record as a
   `return` does, which replaces ADR-0087 rule 3's reading of the
   parameter's type alone. **Both only where the receiving type is a
   `void *`**, which says nothing of what it points at: an `int *`
   receiver reads what its type says, as a case for a call already held,
   and a `return` of one asked the record too until the two were made
   one rule.
7. `Known::held_levels` is not part of it. It is the history of the types a
   block was held as, and stays a field of its own.

### Confirmation

Each measured on the #918 branch by applying the change and restoring it.
The cases are in `tests/cases/`, with their mutations in their notes in
`tests/cases.rs`.

* **Rule 1.** `two_positions_off_the_start_join_off_the_start_whatever_the_distances`
  in `crates/safec-ir/src/memory/parts.rs` fails, and so does
  `a_free_offset_on_both_arms_is_still_proved`, when two positions off the
  start join as `Unknown`. `built::offset_of` counting no distance fails 84
  cases, `an_access_past_the_start_of_a_local_array_is_proved_inside` and
  `a_free_of_a_pointer_moved_back_to_the_start_is_not_an_interior_free`
  among them.
* **Rule 3.** `PlaceInit::unwritten_again` leaving a place kept whole as it
  is fails `a_block_a_loop_shown_to_run_makes_unwritten_again_is_doubted_where_read_on_its_next_turn`.
  The record's size is `a_sites_per_element_record_costs_what_its_types_take`
  in `crates/safec-ir/src/memory.rs`, which fails when `BLOCK_ELEMENTS`
  counts the record alone.
* **Rule 5.** `Allocations::note_narrow_store` recording nothing fails eight
  cases, `an_int_stored_into_a_table_of_pointers_that_leaves_is_reported`
  and `a_char_stored_into_a_pointer_read_back_is_reported` among them; the
  `memset` arm recording nothing fails
  `a_memset_of_part_of_a_pointer_into_a_table_that_leaves_is_reported`.
* **Rule 6.** A door added to `report::PartialDoor` is `error[E0004]` at
  `PartialDoor::record`. A call asking only by the parameter's type fails
  `a_partial_pointer_in_a_block_held_as_pointers_handed_as_void_is_reported`.

### Consequences

* Good, because #703, #894, #913, #914 and #917 become one change. A member
  of a struct is a slot range when structs land.
* Bad, because it is the widest change in the batch, about 300 references,
  and the record grows with each block: `BLOCK_ELEMENTS` was recounted from
  128 to 112 for the slots, and to 136 for where the bytes came from. Too
  low a bound is a panic that names the solver's budget (row 3), not a
  silence.

## Pros and Cons of the Options

### A per-element record only

* Good, because it is smaller (#894).
* Bad, because a `char` store (#917) and element 1's bounds (#703) stay
  unanswered, and structs would add a third representation.

## More Information

* [ADR-0036](./0036-a-pointer-carries-where-in-its-allocation-it-points.md),
  whose `Offset` this replaces.
* [ADR-0081](./0081-a-heap-block-of-pointers-of-a-constant-size-is-written-per-element-as-a-local-array-is.md),
  whose per-element record this generalises.
* ADR-0087 to ADR-0090, whose partial pointer record stays a record per
  place, and whose writers and doors rules 5 and 6 widen; rule 6 replaces
  ADR-0087 rule 3.
