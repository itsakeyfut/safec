---
status: "accepted"
date: 2026-11-17
decision-makers: itsakeyfut
---

# A pointer read out of a block whose record vouches for it is measured from the recorded position, narrowing ADR-0067 rule 2

## Context and Problem Statement

ADR-0067 rule 2 says a pointer read out of memory proves nothing, because what
a block records is a may-set kept per block, not per slot, and a store this
check cannot place is exposed rather than recorded. Rule 1 holds every site a
load reads at an unknown position. So a pointer moved off its start, stored
and read back is doubted wherever it is used, however the block was written:

```c
int a[4]; ... int **box = malloc(8); ... *box = a + 1;
int *q = *box; return q[2];
```

is `SC0412`, a doubt on a read every path keeps inside `a`, and `q[3]` is the
same doubt where it is past the end on every path (#997).

#1025 measured whether the record could be believed, in two halves. Its names
cannot: one may-set per block with no null, read as what a load names, proves a
free of a pointer that is null on every path and weakens a proved use after
free into a doubt a hatch does not report. Its position can, on conditions
the adversary attacked, with one silence found and closed by rule 3 below.

## Decision Drivers

* No access that may reach past an object goes silent ([`docs/safety-model.md`](../safety-model.md)).
* A load's names, frees of what it names, and liveness stay as ADR-0067 has
  them: #1025 found no sound way to believe them.

## Considered Options

* Keep ADR-0067 rule 2 for every reader.
* Believe the record whole, names included, where it names nothing unmodelled.
* Believe the record's position alone, on the conditions below.

## Decision Outcome

Chosen: the position alone, by the owner's decision on #1025 and #997.

1. **A block's record carries the position of what was stored in it**, joined
   over every store as the rest of the record is, with nothing stored taking no
   part, and widened at a loop's back edge as a value's is (ADR-0099).
2. **A load is vouched for** where it is a typed read assigned to a local,
   through one dereference of a local not read out of the caller's memory,
   whose value names heap blocks this check models and nothing else, each
   block not exposed to code this check cannot read and holding a record that
   names nothing unmodelled and has a position. A vouched value takes the join
   of those records' positions. Any other load assigned to a local is at an
   unknown position. A counted move of a vouched value with no load among its
   operands stays vouched; a join is vouched only where both sides are.
3. **A store records a position only where the bounds rule would trust the
   stored value's own**: a value this check stopped following, or one written
   where it could not see, records an unknown position, and no load out of the
   block is vouched for. Found by the adversary on #1025, where an escaped
   local written through a pointer this check could not place was stored and
   read back as proved inside.
4. **Two readers measure a vouched value from its position**: how far an
   access reaches (`fit::reached`), and whether a free is off the start of
   what it frees (`report::interior`). Every other reader, and these two for
   any value not vouched, answer as ADR-0067 rule 2 has them.
5. **Not here**: the store door asking the position where the memory leaves,
   #1024; a position kept per slot, which one position per block covers by its
   range.

### Confirmation

Measured on #997's branch by applying each change and restoring it:

* `fit::reached` keeping the `loaded` gate alone fails
  `a_pointer_stored_off_its_start_and_read_back_is_measured_from_where_it_was_stored`,
  `a_pointer_stored_near_the_end_and_read_back_past_it_is_proved_outside`,
  `a_pointer_read_back_and_moved_by_a_constant_is_measured_from_where_it_was_stored`
  and `a_pointer_stored_on_one_arm_and_a_null_on_the_other_read_back_past_the_end_is_proved_outside`.
  `Known::load_into` vouching for nothing fails those four and
  `a_free_of_a_pointer_stored_off_its_start_and_read_back_is_proved`.
* `Cell::absorb` keeping its own position, or ignoring `widen`, fails
  `a_blocks_record_joins_the_positions_stored_and_widens_them_at_a_back_edge`.
  A store over the whole block keeping the record it replaces proves
  `a_null_stored_over_a_pointer_past_the_end_and_read_back_is_not_proved_outside`
  outside.
* `Cell::poured` recording the position whatever the value's marks (rule 3)
  silences `a_pointer_an_unplaced_store_wrote_stored_and_read_back_is_doubted`
  and `a_pointer_an_unplaced_copy_wrote_stored_and_read_back_is_doubted`.
* `Held::vouched` joined with `||` silences
  `a_pointer_read_out_of_memory_on_one_arm_is_still_one_after_the_join`.
* A read of bytes vouched for fails
  `a_pointer_a_library_copy_reads_out_of_a_block_is_not_measured_from_its_record`;
  a load that is an operand vouched for silences
  `an_integer_beside_a_load_does_not_decide_where_it_points`,
  `a_live_load_moved_by_a_subscript_is_read_in_silence`,
  `a_load_moved_by_a_subscript_after_a_free_is_unproven` and
  `a_pointer_to_a_local_read_out_of_memory_and_moved_by_an_unknown_offset_reads_what_the_local_holds`.
* `built::built_from` vouching for nothing fails the four bounds cases of the
  first item, since a load assigned to a named local reaches it through a
  copy of a temporary.
* `report::interior` told nothing is vouched for fails
  `a_free_of_a_pointer_stored_off_its_start_and_read_back_is_proved`.

### Consequences

* Good, because a read through a pointer stored and read back is measured as
  its local spelling is, both where it is inside and where it is past the end.
* Bad, because the names stay untrusted, so a free through a loaded pointer
  is still "a pointer this check stopped following" beside any off-start proof.
* Bad, because a load passed straight to a reader without a local, `free(*box)`,
  is not vouched for: the free reader answers a projection as lost.

## Pros and Cons of the Options

### Keep ADR-0067 rule 2 for every reader

* Bad, because every read through a stored pointer moved off its start is a
  doubt.

### Believe the record whole

* Bad, because #1025 showed it proves a free of a null pointer and silences a
  proved use after free in a hatch.

## More Information

#997 (the issue), #1025 (the experiment), #1024 (the store door). See
ADR-0067 and ADR-0099.
