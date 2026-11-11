---
status: "accepted"
date: 2026-11-11
decision-makers: itsakeyfut
---

# A copy or a `realloc` carries the record of a partial pointer, and a read of a pointer out of the block asks it

## Context and Problem Statement

ADR-0087 records a library copy that may write part of a pointer on the
block it lands in, `Known::part_written`, and asks the record where the block
leaves as memory of pointers or is handed to a parameter that holds them. It
left two things silent, each a pointer made of four bytes of an `int` and
four others read by somebody:

```c
memcpy(m, &x, 8); memcpy(m, &i, 4);
memcpy(m2, m, 8); void **q = m2; *pp = q;    /* the record stays on m (#905) */
void *n = realloc(m, 8); void **q = n; *pp = q;               /* the same */
void **b = m; void *s = *b;       /* read here, and nothing leaves (#907) */
memcpy(&s, m, 8); *pp = s;                      /* read into a local, the same */
```

## Decision Drivers

* A silence about a program that reads a pointer nobody wrote whole.
* One report per copy, at the copy, whichever door asks first (ADR-0087
  rule 4).
* The record has no offset, so nothing can say which bytes a later copy or
  read covers.

## Considered Options

* Carry the record from block to block, and ask it at a read of a pointer.
* Move the record into the block's contents (`Cell`), so that every pour
  carries it.
* Carry it, and leave the read to the doors ADR-0087 already has.

## Decision Outcome

Chosen: carry it and ask it at a read, by the author.

1. **A library copy carries the source blocks' record to every block it
   lands in**, whatever its length, joined keeping the earlier span: the
   blocks the source names at the copy's depth, `memcpy(m2, m, 8)` and
   `memmove` alike. A copy that writes part of a pointer still records its
   own span beside it, and the earlier is kept.
2. **A `realloc` carries the old block's record to the new one**, beside the
   contents it already pours, from the same old blocks.
3. **A load of a pointer out of a block with the record asks it**, at the
   copy: through one dereference, the blocks the pointer names, and below
   that, the blocks the pointer gone through last may name. **Not where the
   load is already reported as reading memory nothing wrote**, which is the
   same defect said more strongly (ADR-0069).
4. **A library copy into a destination whose type holds pointers asks the
   source blocks' record**, at the copy, as a load would, and not where the
   copy is already reported as copying what nothing wrote (ADR-0071 rule 5).

A copy of part of a block carries the record whether or not it reaches the
partial pointer, and a `realloc` that shrinks the block carries it too: the
record has no offset. That is a doubt the reader sees, the same one ADR-0087
accepts for a block rewritten whole.

The record is kept on allocations only, so a local array has nowhere to hold
it: a copy into one carries nothing, and nor does a partial copy made
straight into one, which ADR-0087 left the same way. `memcpy(buf, m, 8);
memcpy(m2, buf, 8);` with `char buf[8]` is silent. That is a silence left
open, and #908 is where it is decided.

### Confirmation

Each in `tests/cases/memory/`, with its mutation in its note in
`tests/cases.rs`:

* `a_partial_pointer_in_a_block_copied_whole_into_another_that_leaves_is_reported`
  and `a_partial_pointer_copied_through_two_whole_copies_into_a_block_that_leaves_is_reported`
  go silent when the copy carries nothing (rule 1).
* `a_partial_pointer_in_a_block_moved_by_realloc_that_leaves_is_reported`
  goes silent when the `realloc` carries nothing (rule 2).
* `a_block_with_no_partial_pointer_copied_whole_into_another_that_leaves_is_not_reported`
  is reported when every copy records its own span as a partial one's
  (rule 1's source).

* `a_pointer_read_out_of_a_block_a_partial_copy_wrote_is_reported` and
  `a_partial_pointer_read_after_an_alias_typed_first_is_reported` go silent
  when a load asks nothing (rule 3), and
  `an_int_read_out_of_a_block_a_partial_copy_wrote_is_not_reported` is
  reported when every load is asked whatever its type.
* `a_byte_copied_out_of_a_local_is_doubted_where_read_back_as_a_pointer`
  gains an `SC0413` beside its `SC0411` when a load already said
  indeterminate is asked too (rule 3's exception).
* `a_partial_pointer_copied_out_of_a_block_into_a_pointer_local_is_reported`
  goes silent when a copy asks nothing (rule 4), and
  `a_partial_pointer_copied_out_of_a_block_into_an_int_local_is_not_reported`
  is reported when every copy is asked whatever its destination;
  `a_partial_pointer_copied_out_unwritten_is_reported_as_unwritten_alone`
  gains an `SC0413` when a copy already said unwritten is asked too.
* `a_partial_pointer_read_out_of_a_block_that_also_leaves_is_reported_once`
  says it twice when the report is pushed without ADR-0087 rule 4's check.

### Consequences

* Good, because what a block holds and where it is read no longer decide
  whether a partial copy is asked.
* Bad, because a copy of the untouched half of a block, a shrinking
  `realloc`, and a pointer read from a slot the partial copy never reached
  are doubted.

## Pros and Cons of the Options

### Move the record into the block's contents

* Good, because `Known::pour` would carry it through a `realloc` with no
  line of its own.
* Bad, because a copy pours what the source's pointers hold, one level below
  the source block, so the copy door still needs its own line, and ADR-0087's
  table would move for one door's sake.

### Leave the read to the doors ADR-0087 has

* Good, because no new door.
* Bad, because a block read inside the function, or read into a local that
  then leaves, is asked nothing (#907).

## More Information

* [ADR-0087](./0087-a-copy-of-part-of-a-pointer-is-recorded-on-the-block-and-asked-where-the-block-is-held-as-memory-of-pointers.md),
  whose record this carries and whose open silence it closes.
* [ADR-0069](./0069-a-pointer-read-out-of-memory-nothing-wrote-is-reported-where-it-is-read-and-a-loop-may-run-zero-times-unless-it-is-shown-to-run.md),
  whose report at a read takes the place of this one.
