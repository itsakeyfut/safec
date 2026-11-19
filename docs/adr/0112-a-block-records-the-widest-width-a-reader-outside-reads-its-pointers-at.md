---
status: "accepted"
date: 2026-11-19
decision-makers: itsakeyfut
---

# A block records the widest width a reader outside reads its pointers at

## Context and Problem Statement

A pointer that leaves the function is asked whether what it points at is as
wide as the reader outside will read it (ADR-0068 rule 6). That width came
from the types of the places the departure is spelled through, and, for a
library copy since #1079, from what the destination pointer's value carries
from the type it arrived as (`Held::contents_at_most`). That value is an upper
bound for a read, and a join keeps the narrower: with `t` either `out`
(`int **`) or `other` (`char **`), `void **vo = t; memcpy(vo, &c, 8);` was
asked at one byte and let a pointer to three bytes leave where `out`'s reader
reads four (#1082). A block this function allocates carries no such width at
all (#1081), and the store door reads none (#1083).

## Decision Drivers

* A departure is asked at the widest width any reader of the memory it lands
  in reads, which a read's upper bound cannot say.
* No fact a write has to answer for, which #1061 could not keep honest.
* One answer for every door that asks a departure through a place.

## Considered Options

* A per-site record, `Known::receiver_width`, of the widest width a reader
  outside reads the block's pointers at.
* A second bound on `Held`, the widest, joined by the larger.

## Decision Outcome

Chosen option: "A per-site record", by the owner's decision on #1085.

* `Known::receiver_width: Table<Site, u64>`, in bytes, `0` where no reader's
  type says. A pointer parameter's site is set from the parameter's type where
  the function starts (`fit::slot_width`). A may-fact, joined by the larger,
  and kept where the site is made again, as `Known::held_levels` is
  (ADR-0086).
* It is a fact about the reader's type, not about the block's bytes, so no
  write answers for it.
* `fit::carried_width` is the one answer the doors read: the larger of what
  the pointer's value carries (#1079, kept by the owner's decision) and the
  largest record among the sites it may name.

A bound on `Held` was rejected: it is lost through a pointer read out of
memory, a block this function allocated would still need a record of its own,
and every operation on `Held` would have to answer for it; where it is lost
the departure is silent.

#1081 sets the record where a block leaves and where one is handed to a call
this check cannot read; #1083 makes the store door read it. Both are part of
this decision and land after it.

### Confirmation

* Joining the record by the smaller, seeding nothing at entry, or having
  `carried_width` read the value's width alone each silence
  `a_copy_into_one_of_two_caller_memories_is_asked_at_the_wider_ones_width`
  and
  `a_copy_into_caller_memory_or_a_void_pointer_parameters_memory_is_asked_at_the_wider_ones_width`.
* `a_copy_of_a_pointer_wide_enough_into_one_of_two_caller_memories_is_not_asked`
  holds the other side: a pointer as wide as the wider reader is not asked.
* A field added to `Known` is `error[E0027]` at its join, its entry and a
  rebirth, and then a failure of
  `every_field_of_the_memory_checks_value_is_a_belief_or_an_exemption` until
  it has a row.

### Consequences

* Good, because a departure is asked at the widest reader whatever join the
  pointer came through, and a route nobody named that reaches a recorded
  block is covered.
* Bad, because the largest record is taken where a pointer may name several
  blocks, so a pointer as narrow as the narrower reader is doubted where it
  lands in that one: a false report the reader sees.
* What would reverse this: a value that keeps both bounds through every
  operation, including a load, honestly.

## More Information

ADR-0068, ADR-0086, #1061, #1079, #1081, #1082, #1083, #1085.
