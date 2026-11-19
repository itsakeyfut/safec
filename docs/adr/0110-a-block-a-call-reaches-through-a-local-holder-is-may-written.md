---
status: "superseded by ADR-0111"
date: 2026-11-19
decision-makers: itsakeyfut
---

# A block a call reaches through a local holder is may-written, widening ADR-0108

## Context and Problem Statement

ADR-0108 may-writes every block in the closure of the sites the arguments of a
call this check cannot read name. A block reached through a local, an
argument's (`fill(&h)` with `h` either `b` or a null) or a holder an earlier
call kept (`keep(&h); if (c) h = b; tick();`), is in no such closure. So it
kept the exposure's belief that it was written, and `*b` was read
indeterminate, reported only by the doubt that the call may have freed `b`
(#1070), on main since before ADR-0108.

## Decision Drivers

* A block a call may reach is not believed written unless something certainly
  names it.
* The output-parameter idiom through a local holder of a block, `fill(&h)` with
  `h` certainly `b`, and `keep(&h); h = b; tick();`, must stay believed.

## Considered Options

* Seed the closure with what the locals the arguments may name hold, and with
  what every exposed local holds. Keep believed what a chain of must-facts
  reaches.
* Keep the closure of the arguments' sites only.

## Decision Outcome

Chosen option: "Seed the closure through locals; keep the must-chain". The
design of #1070 under #1056 settled it, at no measured cost.

`Known::believed_written_only_where_named_alone`:

* seeds the closure with the sites every local a handed value may name holds,
  transitively through locals, and those every exposed local holds;
* keeps believed a block a handed value certainly names, or one a chain of
  `Held::certain` reaches. The chain starts from a local a handed value
  certainly names, or from one an earlier call certainly kept
  (`certainly_exposed`).

### Confirmation

* Seeding the closure with the arguments' sites only silences
  `a_block_a_call_reaches_through_a_local_holder_that_may_hold_a_null_is_doubted_where_read`
  and
  `a_block_a_later_call_reaches_through_a_holder_an_earlier_call_kept_is_doubted_where_read`.
* Leaving the exposed locals out of the seeds silences the second.
* A chain that grants no block gives
  `a_block_a_call_reaches_through_a_local_holder_that_certainly_names_it_is_not_reported_as_indeterminate`
  an `SC0411`.

No other case moved, measured when this was written.

### Consequences

* Good, because what a call may reach through a local no longer counts as
  written by it.
* Bad, because nothing measured.
* What would reverse this: an idiom a corpus program relies on where a block
  is written through a holder that only may name it.

## More Information

ADR-0107, ADR-0108, ADR-0109, #1056, #1070.
