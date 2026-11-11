---
status: "proposed"
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
   block alike, keyed by `Loc`.** Each slot is `Unwritten`, `Maybe`,
   `Written` or `Part`. `Part` carries the earliest span of a write that
   covered some of the slot's bytes and not all of them, and which bytes.
   - The whole-place answer is computed from the slots rather than stored
     beside them, so the stamp goes.
   - The cap stays: 2 to `ELEMENTS_KEPT` slots, and one slot past it.
   - The joins stay: forward and back edge, and a site's `Unheld` side.
   - It replaces `init`, `element_init`, `block_elements`, `part_written`
     and `part_written_since_made`.
4. **The record's two lifetimes stay.** One kept across a site made again,
   for the doors a block leaves by. One reset there, for a read inside the
   function (ADR-0089).
5. **Every writer writes slots, however it is spelled.** A store, a library
   copy, a `memset` and a `char` store each mark the slots their bytes cover:
   - whole where they cover all of a slot;
   - `Part` where they cover some of it;
   - nothing where the slot is in an object narrower than a pointer.

   A `char` store into memory of pointers is therefore recorded (#917), and a
   pointer copied a byte at a time is not doubted once all its bytes are
   written. This replaces the `char` exemption the design of #913 had.
6. **One rule asks the record at every door**, a `PartialDoor` matched
   exhaustively (#914). A call asks a handed block by its held record as a
   `return` does, which replaces ADR-0087 rule 3's reading of the
   parameter's type alone.
7. `Known::held_levels` is not part of it. It is the history of the types a
   block was held as, and stays a field of its own.

### Confirmation

To be filled by the change for #918, one case per rule, and the route-matrix
rows ADR-0091 asks for.

### Consequences

* Good, because #703, #894, #913, #914 and #917 become one change. A member
  of a struct is a slot range when structs land.
* Bad, because it is the widest change in the batch, about 300 references,
  and `Analysis::height` has to be recomputed for the slots. Too low a bound
  is a panic that names the solver's budget (row 3), not a silence.

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
* ADR-0087 to ADR-0090, whose partial pointer record this folds into the
  slots.
