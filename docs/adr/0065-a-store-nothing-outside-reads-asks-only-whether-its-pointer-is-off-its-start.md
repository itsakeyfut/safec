---
status: "accepted"
date: 2026-11-05
decision-makers: itsakeyfut
---

# A store nothing outside reads asks only whether its pointer is off its start, replacing what ADR-0060 says of the store door

## Context and Problem Statement

ADR-0060 asks a pointer at three doors where the other side believes it, and
the third is a store: "a later load believes what was stored". So
`int **m = malloc(8); *m = v;` asks `v` whether it points at one whole object
of its type, `SC0413` "stored here", even where `m` never leaves the function.
The same write into a local, `s = v;`, is asked nothing, so a route answers
differently through a block and through a local: the route matrix's pairs at
depth zero and one differ in about sixteen rows on this alone (#791), and the
answer at depth one falls on correct programs too, `caller_ret_d1` among the
controls.

ADR-0061 has since made a load believe only the width recorded where it reads,
so a narrow pointer stored and read back is asked where it is used. What a load
does not carry is a distance from the start, since no distance is carried at
all (ADR-0036): where nothing reads the pointer back, the store is the only
place `*box = p + 5;` with `p` four bytes wide is reported, and that arithmetic
is itself undefined (C17 6.5.6 p8).

This record replaces what ADR-0060 says of a store, and nothing else of it.

## Decision Drivers

* **A belief is asked where it is held.** Memory only this function reads is
  read back by a load this check also follows, and that load asks the width.
* **A report that is the only report must stay.** Dropping the store door
  outright left `*box = p + 5;` silent, measured, a program C does not define.

## Considered Options

* Keep the store door at every store into memory.
* Ask a store only where code outside this function may read what it writes.
* As the second, and ask a store nothing outside reads only whether its
  pointer may be off its start.

## Decision Outcome

Chosen option: the third, by the user's decision on #791. A store through a
pointer into memory code this check cannot read may read (`Store::leaves`, the
answer a direct write into a local already takes) is asked as ADR-0060 says.
Any other is asked only whether the pointer it stores may be off its start; its
width, and what it points at, are asked by the load that reads it back
(ADR-0061) and by the door the memory leaves by, if it does.

### Confirmation

* `a_narrow_pointer_stored_into_a_block_nothing_outside_reads_is_not_asked_where_it_is_stored`
  and `a_local_s_address_stored_in_memory_is_asked_by_what_the_local_believes_where_it_leaves`
  gain an `SC0413` at the store if the store door asks every store through a
  pointer again.
* `a_pointer_moved_off_its_start_stored_into_memory` goes silent, with
  eighteen other cases losing their `SC0413`, if such a store is asked
  nothing.
* In `tests/routes.rs`, the pairs `APART` lists for this record,
  `ASKED_WHERE_STORED`, are where a pointer this check cannot say starts an
  object is stored into memory; `every_pair_kept_apart_still_differs` fails if
  one of them comes to agree, and `a_route_answers_alike_at_depth_zero_and_one`
  if a pair outside the table parts.

### Consequences

* Good, because a write into a block this function owns answers as the same
  write into a local.
* Bad, because a narrow pointer stored and never read back is no longer
  reported where it is stored; it was never used.
* What would reverse this: a load that believes more than its record, which
  ADR-0061 rejected.

## More Information

* [ADR-0060](./0060-an-access-is-proved-inside-its-object-only-where-its-size-was-read.md),
  whose store door this narrows.
* [ADR-0061](./0061-a-pointer-read-out-of-memory-is-believed-only-as-wide-as-was-recorded.md),
  which makes the load ask what the store no longer does.
* [ADR-0036](./0036-a-pointer-carries-where-in-its-allocation-it-points.md),
  why no distance is carried.
