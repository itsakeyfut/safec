---
status: "accepted"
date: 2026-11-11
decision-makers: itsakeyfut
---

# A partial pointer record is kept for a local as for a block

## Context and Problem Statement

ADR-0087 records a copy that may write part of a pointer on the block it
lands in, ADR-0088 carries the record through a copy and a `realloc`, and
ADR-0089 keeps a second record for the block the site names now. All three
are tables over allocation sites, so a local array has nowhere to hold one,
and ADR-0088 says that silence is left open (#908):

```c
char buf[8];
memcpy(m, &x, 8); memcpy(m, &i, 4);
memcpy(buf, m, 8); memcpy(m2, buf, 8);      /* the record is dropped at buf */
memcpy(buf, &x, 8); memcpy(buf, &i, 4);
memcpy(m2, buf, 8);                         /* nothing is recorded at all */
void **q = m2; *pp = q;
```

Each prints nothing and exits 0, and the caller reads a pointer made of four
bytes of an `int` and four others.

## Decision Drivers

* A silence about a program whose caller reads a pointer nobody wrote whole.
* ADR-0063 put sites and locals in one index so that one question is not
  kept in two tables and answered twice (#779).

## Considered Options

* Key both records by `Loc`, a site or a local.
* A second pair of tables over locals.
* A doubt at a copy into a local, with no record kept.

## Decision Outcome

Chosen: key both records by `Loc`, by the author.

1. **`Known::part_written` and `Known::part_written_since_made` are
   `Table<Loc, Option<Span>>`.** A copy notes and carries into the locals it
   lands in by an edge (`Landing::targets`) beside the blocks, and carries out
   of the locals the source names beside the blocks.
2. **A read asks the locals it may read** beside the blocks: a load through a
   pointer that may name a local, and a copy out of one.
3. **A local's records are cleared where its lifetime begins or ends**
   (`Known::clear`): nothing outside the function can hold a local's storage
   once it is gone, so the reason ADR-0087 keeps a block's record does not
   reach a local.
4. **What a site's record does is unchanged.** The exit, the `return` and the
   call ask blocks as before.

A local array rewritten whole after a partial copy keeps the record, as
ADR-0087 accepts for a block. That is a doubt the reader sees.

### Confirmation

Each in `tests/cases/memory/`, with its mutation in its note in
`tests/cases.rs`:

* `a_partial_pointer_copied_through_a_local_array_into_a_block_that_leaves_is_reported`
  and `a_partial_pointer_copied_into_a_local_array_and_out_into_a_block_that_leaves_is_reported`
  go silent when a copy notes and carries into blocks alone, and when it
  carries out of the source's sites alone (rule 1).
* `a_pointer_read_out_of_a_local_array_a_partial_copy_wrote_through_a_pointer_is_reported`
  goes silent when a read asks blocks alone (rule 2).
* `a_local_array_a_partial_copy_wrote_on_an_earlier_turn_is_not_reported_on_the_next`
  is reported when `Known::clear` leaves a local's records alone (rule 3).
* `a_partial_pointer_copied_into_a_local_array_that_never_reaches_pointers_is_not_reported`
  is reported when the exit asks every block with the record (rule 4).

### Consequences

* Good, because a stack buffer between two copies, or a partial copy into
  one, no longer decides whether the partial pointer is asked.
* Bad, because the two records take twice the memory, the size every
  `Table<Loc, _>` already takes.

## Pros and Cons of the Options

### A second pair of tables over locals

* Good, because the site tables stay as they are.
* Bad, because one question kept in two tables is what ADR-0063 removed, and
  a writer that forgets one of the two is silent.

### A doubt at a copy into a local

* Good, because no record.
* Bad, because a stack buffer that only ever holds bytes is doubted, and a
  partial copy made straight into one stays silent.

## More Information

* [ADR-0063](./0063-the-memory-check-has-one-kind-of-place-one-kind-of-cell-and-one-description-of-a-write.md), whose index this
  uses.
* [ADR-0088](./0088-a-copy-or-realloc-carries-the-record-of-a-partial-pointer-and-a-read-of-a-pointer-out-of-the-block-asks-it.md),
  whose silence about a local array this closes.
