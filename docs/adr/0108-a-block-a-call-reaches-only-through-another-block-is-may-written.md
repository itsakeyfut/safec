---
status: "accepted (widened through local holders by ADR-0110)"
date: 2026-11-19
decision-makers: itsakeyfut
---

# A block a call reaches only through another block is may-written

## Context and Problem Statement

A call this check cannot read is believed to write every block it is exposed
to (#778). ADR-0104 narrowed that for the blocks an argument names, and
ADR-0105 kept it: such a block is believed written only where an argument
certainly names it. A block reached only through another block was still
believed written. So `*box = p; fill(box); return *b != 0;` with `p` either
`b` or a null read `*b` indeterminate where `p` was null, and only the doubt
that the call may have freed `b` was reported (#1064).

Telling a block that certainly holds the address of another from one that may
needs a fact kept in a block's record, and #1061 dropped that: every write
that changes a record's bytes would have to answer for it, and review found
three that did not.

## Decision Drivers

* No fact kept in a block's record.
* A block a call may not reach should not be believed written by it.

## Considered Options

* May-write every block in the closure of what the arguments name, except
  one an argument certainly names.
* Keep the belief for blocks reached through other blocks.

## Decision Outcome

Chosen option: "May-write the closure", by the owner's decision on #1056.
`Known::believed_written_only_where_named_alone` asks every block in the
closure of the blocks the arguments name. Each keeps what it held before the
call's exposure, may-written, unless an argument certainly names it.

Keeping the belief was rejected because it leaves the read reported only for
another reason, a silence once that reason goes.

### Confirmation

* May-writing only the blocks an argument names silences
  `a_block_a_call_reaches_only_through_another_that_may_hold_a_null_is_doubted_where_read`
  and restores the belief in
  `a_pointer_read_out_of_memory_a_call_reached_through_a_holder_is_doubted_where_read`.

### Consequences

* Good, because what a call may not reach is not believed written by it.
* Bad, because the output-parameter idiom through a block, `*h = box;
  init(h); t = *box;`, is doubted where `*box` is read. One corpus case moved
  and was renamed. Eighteen route matrix controls gain an `SC0411`: the
  `left_behind`, `part_pointer`, `stored_freed` and `stored_out` routes at
  depth 2.
* What would reverse this: a must-fact a block's record can keep honestly,
  which #1061 could not.

## More Information

ADR-0104, ADR-0105, #778, #1056, #1061, #1064.
