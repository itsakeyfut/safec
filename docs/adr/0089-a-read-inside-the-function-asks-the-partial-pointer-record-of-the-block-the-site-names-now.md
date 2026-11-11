---
status: "accepted"
date: 2026-11-11
decision-makers: itsakeyfut
---

# A read inside the function asks the partial pointer record of the block the site names now

## Context and Problem Statement

ADR-0088 rules 3 and 4 ask `Known::part_written` where a pointer is read
out of a block inside the function. ADR-0087 rule 1 keeps that record where
the site is made again, because a caller may still hold the block an
earlier turn made. A read inside the function reads the block the site
names now, so the kept record doubted a correct loop:

```c
for (k = 0; k < n; k++) {
    void *m = malloc(8); if (!m) return r;
    memcpy(m, &x, 8); void **b = m; void *s = *b;   /* a whole pointer */
    memcpy(m, &i, 4); free(m);                      /* doubted here */
}
```

## Decision Drivers

* A doubt about a correct program, on every turn of a common loop.
* The doors that ask where a block leaves still need the kept record, or a
  block one turn let leave goes silent when the next turn allocates again.

## Considered Options

* A second record, cleared where the site is made again, asked by the reads.
* Clear the one record where the site is made again, for every door.
* Keep the doubt.

## Decision Outcome

Chosen: a second record, by the author.

1. **`Known::part_written_since_made` is `part_written` for the allocation
   the site names now**: written wherever `part_written` is, carried by a
   copy and a `realloc` from the same record of the source, joined the same
   way, and cleared where the site is made again.
2. **The two read doors of ADR-0088, rules 3 and 4, ask it** in place of
   `part_written`. The exit, the `return` and the call ask `part_written`
   as before.

This replaces which record ADR-0088 rules 3 and 4 ask, and nothing else of
them.

### Confirmation

Each in `tests/cases/memory/`, with its mutation in its note in
`tests/cases.rs`:

* `a_pointer_read_out_of_a_block_each_turn_makes_before_a_partial_copy_into_it_is_not_reported`
  is reported when a read asks `part_written`, and when the site's own
  record is not cleared where it is made.
* `a_partial_pointer_copied_and_read_in_each_turn_of_a_loop_is_reported`
  and the other read cases go silent when a copy is noted in the kept record
  alone.
* `a_pointer_read_out_of_a_block_a_realloc_moved_after_a_partial_copy_is_reported`
  goes silent when a `realloc` carries the kept record alone.
* `a_partial_pointer_copied_on_one_arm_and_read_after_the_join_is_reported`
  goes silent when the record is left out of the join.

### Consequences

* Good, because a read is asked about the block it reads, and the doors
  where a block leaves keep what they had.
* Bad, because `Known` gains a second table that has to be kept in step
  with the first; one body, `Known::note_part_written` and
  `Known::carry_part_written`, writes both.

## Pros and Cons of the Options

### Clear the one record where the site is made again

* Good, because no second table.
* Bad, because it reverses ADR-0087 rule 1: a block one turn of a loop let
  leave with a partial pointer is asked nothing once the next turn
  allocates again.

### Keep the doubt

* Good, because no change.
* Bad, because a correct and common loop is doubted at a copy that comes
  after the read.

## More Information

* [ADR-0088](./0088-a-copy-or-realloc-carries-the-record-of-a-partial-pointer-and-a-read-of-a-pointer-out-of-the-block-asks-it.md),
  whose rules 3 and 4 this changes in part.
* [ADR-0087](./0087-a-copy-of-part-of-a-pointer-is-recorded-on-the-block-and-asked-where-the-block-is-held-as-memory-of-pointers.md),
  whose rule 1 keeps the record this leaves to the doors where a block leaves.
