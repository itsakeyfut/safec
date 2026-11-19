---
status: "accepted"
date: 2026-11-19
decision-makers: itsakeyfut
---

# A call is believed to write only a block a must-chain names, superseding ADR-0108 and ADR-0110

## Context and Problem Statement

A call this check cannot read is believed to write every block it is exposed
to (#778). ADR-0108 and ADR-0110 kept that belief only where something
certainly names the block, and may-wrote the rest, but only the rest of a
closure seeded from what the call's arguments name and from what the exposed
locals hold. A block the call reaches by any other route was in no such
closure and kept the exposure's belief: the caller's memory a parameter
points at, a block an earlier call kept, a block exposed on one arm only, a
call's result, and a block reached through a local whose address sits in a
block. So `*out = b; tick();` let `b` leave unwritten in silence, and
`keep(hb); *hb = b; tick(); return *b != 0;` read `*b` indeterminate,
reported only by the doubt that the call may have freed `b` (#1077). Every
route ADR-0108 and ADR-0110 did not name was a hole of the same shape, and
each record added one door.

## Decision Drivers

* A block a call may reach is not believed written unless a chain of
  must-facts certainly names it.
* One rule rather than one door per route, so that a route nobody named is
  covered by default.
* The output-parameter idioms ADR-0105, ADR-0107 and ADR-0109 keep believed
  stay believed.

## Considered Options

* May-write every exposed block, except what the must-chain names.
* Widen the seeds of the closure: the exposed sites, and what the locals
  held in blocks hold.

## Decision Outcome

Chosen option: "May-write every exposed block", by the owner's decision on
#1077. `Known::expose` already closes `exposed` over everything code this
check cannot read may reach, whatever the route, so
`Known::believed_written_only_where_named_alone` reads that set rather than
building its own closure. Each exposed block keeps what it held before the
call's exposure, may-written, unless a handed value certainly names it or a
chain of `Held::certain` reaches it from a handed value's certain local or
from a local an earlier call certainly kept (`certainly_exposed`).

**And a call's result is exposed without undoing that.** A call that returns
a value exposes its result after the rule has answered, and
`Known::expose` believes every exposed block written, so `*out = b; tick();`
with `tick` returning an `int` believed `b` written again. The result's
exposure keeps every other block as the rule left it, as a store that
exposes what it carries already does. Found by review.

Widening the seeds gave the same output on the corpus and on every program
the hunt found, and was rejected because it keeps one door per route: the
next route nobody names is a silence again.

What a call is believed to write when the must-chain names the block, what a
call's result is believed to hold, and the locals' side (`expose_locals`) are
unchanged.

### Confirmation

* May-writing only the closure of what the arguments name and of what the
  locals they and the exposed locals name hold, the rule before this, loses
  the `SC0411` of every case added with this record:
  `a_block_that_leaves_through_an_output_parameter_before_a_call_handed_nothing_is_doubted_where_it_leaves`,
  `a_block_that_leaves_inside_another_through_an_output_parameter_before_a_call_handed_nothing_is_doubted_where_it_leaves`,
  `a_block_stored_through_an_output_parameter_is_doubted_where_read_after_a_call_handed_nothing`,
  `a_block_stored_into_memory_an_earlier_call_kept_is_doubted_where_read_after_a_later_call`,
  `a_block_stored_into_memory_an_earlier_call_kept_and_then_cleared_is_doubted_where_read_after_a_later_call`,
  `a_block_an_earlier_call_kept_on_one_arm_only_is_doubted_where_read_after_a_later_call`
  and
  `a_block_a_call_reaches_through_a_local_whose_address_sits_in_a_block_is_doubted_where_read`.
* May-writing only the blocks an argument names also loses the `SC0411` of
  ADR-0108's and ADR-0110's cases,
  `a_block_a_call_reaches_only_through_another_that_may_hold_a_null_is_doubted_where_read`,
  `a_block_a_call_reaches_through_a_local_holder_that_may_hold_a_null_is_doubted_where_read`,
  `a_block_a_later_call_reaches_through_a_holder_an_earlier_call_kept_is_doubted_where_read`
  and
  `a_pointer_read_out_of_memory_a_call_reached_through_a_holder_is_doubted_where_read`.
* A chain that grants no block gives
  `a_block_a_call_reaches_through_a_local_holder_that_certainly_names_it_is_not_reported_as_indeterminate`
  and
  `a_block_a_call_reaches_through_two_local_holders_that_certainly_name_it_is_not_reported_as_indeterminate`
  an `SC0411` each, and nothing else. A chain that stops at a local naming
  a local gives the second one, and nothing else.
* Exposing a call's result without restoring what the other blocks held
  loses the `SC0411` of
  `a_block_that_leaves_before_a_call_that_returns_a_value_is_doubted_where_it_leaves`
  and
  `a_block_an_earlier_call_kept_on_one_arm_only_is_doubted_where_read_after_a_later_call_that_returns_a_value`.
* The rule before this also loses the `SC0411` of
  `a_block_stored_through_a_pointer_a_call_returned_is_doubted_where_read_after_a_later_call`,
  the route through a call's result.
* May-writing every block, exposed or not, turns the certain read in
  `a_block_no_call_may_reach_is_still_read_unwritten_for_certain_after_a_call`
  into a doubt, and nothing else.

Measured when this was written.

### Consequences

* Good, because a route by which code outside may reach a block no longer
  needs naming for the block to stop counting as written by a call.
* Bad, because eleven corpus cases gain one `SC0411` where a block leaves
  before a call, `*pp = q; use();`. Ten hold a 16-byte block with a slot
  nothing but the call may write; the eleventh,
  `a_copy_of_part_of_a_pointer_through_a_void_pointer_destination_is_asked_by_its_source`,
  copies half a pointer to a 4-byte block nothing writes. Each doubt is true.
* Bad, because three route matrix controls, `returned_ret_d0_both` to
  `returned_ret_d2_both`, gain an `SC0411` where they return what
  `pass(x)` returned: the call may return any exposed block (ADR-0066),
  among them the 4-byte block `*x` holds, which nothing certainly names.
  Before this the result's exposure believed that block written again. Two of them are named
  `..._is_reported_once`, which is about the partial-pointer report, and keep
  their names.
* What would reverse this: a must-fact a block's record can keep honestly,
  which #1061 could not, or an idiom a corpus program relies on where a block
  only may be named.

## More Information

ADR-0105, ADR-0107, ADR-0108, ADR-0109, ADR-0110, #1056, #1061, #1077.
