---
status: "accepted"
date: 2026-11-17
decision-makers: itsakeyfut
---

# A join of two counted positions keeps the range between them, and a null takes no part, replacing ADR-0092 rule 1's join

## Context and Problem Statement

ADR-0092 rule 1 makes a value's position `At`: `Bytes(n)`, `Moved` or
`Unknown`. `Bytes(a)` joined with `Bytes(b)`, `a != b`, is `Moved` where both
are off the start and `Unknown` otherwise, and a null joined with a moved
value stays `Unknown`, as ADR-0036 accepted. So two arms that move a pointer
by different constants lose how far it went:

```c
int a[4]; ... int *q; if (c) q = a + 1; else q = a + 2; return q[1];
```

was `SC0412`, a doubt on an access every path keeps inside `a`, and so was
`int *q = 0; if (c) q = a + 1; if (!q) return 0; return q[1];` (#944).

## Decision Drivers

* No access that may reach past an object goes silent, and none that reaches
  past it on one path alone is proved.
* The dataflow walk ends.

## Considered Options

* Keep ADR-0092 rule 1's join.
* Keep the far bound alone, `Off(most)`, for two positions off the start.
* Keep the range, `[least, most]`, for any two counted positions.

## Decision Outcome

Chosen: the range, by the owner's decision on #944.

1. **`At::Within { least, most }`**, `least < most`: somewhere from `least` to
   `most` bytes in, each counted. Two counted positions that differ join to the
   range over both.
2. **Each reader asks the end it needs.** How far an access reaches is from
   `most`; it is outside for certain only where `least` is too, and otherwise a
   doubt. A free is off the start only where `least` is. Every reader that asks
   for one offset reads a range as `Moved` where `least >= 1` and as `Unknown`
   otherwise, which is what it answered before.
3. **A null takes no part in the join of a position**, as it already took none
   in the join of a row (#996): a path holding nothing points nowhere.
4. **At a loop's back edge two positions that differ join as rule 1 had
   them**, `Moved` or `Unknown`, so a pointer a loop moves every turn reaches a
   fixpoint. The range holds across a branch, not across a loop.
5. **Not here**: a position kept in memory, #997, and the element fields read
   off the byte position, #1022.

### Confirmation

Measured on #944's branch by applying each change and restoring it:

* Two counted positions joined as `Moved` fails
  `two_counted_positions_join_to_the_range_between_them` and reports
  `a_pointer_at_one_of_two_counted_positions_is_counted_where_read`.
* Answering outside from the far end alone proves
  `a_pointer_at_one_of_two_positions_reaching_past_the_end_from_one_is_doubted`
  outside.
* The null's start joined in fails
  `a_null_joined_with_a_counted_pointer_keeps_its_position` and
  `a_local_array_written_through_memory_after_an_earlier_turn_stored_a_moved_pointer_is_doubted`;
  `a_pointer_null_on_one_arm_and_moved_on_the_other_is_counted_where_read`
  is held by the range alone, since the start joined in leaves a range
  that is still inside.
* A range walking a row taken as inside it silences
  `a_pointer_at_one_of_two_positions_in_a_row_reaching_past_the_row_from_one_is_doubted`.
* **Held by nothing**: a range never covers a block kept whole
  (`Known::covers_whole`), since a write as wide as the block from a range
  runs past it from the range's far end.
* A free reading the far end proves
  `a_pointer_at_its_start_or_one_element_past_it_is_doubted_where_freed`.
* `widen` ignored fails
  `a_counted_position_meeting_another_at_a_back_edge_is_widened`; a loop that
  moves a pointer would not end, which no case can show as a failure.

### Consequences

* Good, because twelve `SC0412` doubts on accesses every path keeps inside
  their object go, eleven in corpus cases and one in a route row, and none
  that may reach past one is lost.
* Bad, because the range does not survive memory: the same two arms stored
  through a box and read back are still doubted, which is #997's.

## Pros and Cons of the Options

### Keep ADR-0092 rule 1's join

* Bad, because the two programs above are doubted.

### The far bound alone

* Good, because it is one number.
* Bad, because the start joined with a position off it stays `Unknown`, so
  `p` and `p + 1` met are doubted wherever read.

## More Information

#944 (the issue), #996 (the row's null identity), #997 (a position in
memory), #1022 (the element fields). See ADR-0036 and ADR-0092.
