---
status: "superseded by ADR-0105"
date: 2026-11-18
decision-makers: itsakeyfut
---

# A block a call is handed is believed written only where an argument names it alone

## Context and Problem Statement

A call this check cannot read is believed to write every block it is exposed
to (#778), the output-parameter idiom for memory: `p = malloc(n); fill(p);
q = *p;` builds. ADR-0101 rule 1 and ADR-0103 narrowed the same belief for a
local, which a call writes for certain only where some holder names it alone,
and a pointer that may be null names nothing alone (#950).

A block was not asked. The second hunt before closing #941 found a block
reached only through a pointer that may be null, or that names another block
beside it, believed written: `p = 0; if (c) p = b; fill(p); return *b != 0;`
reads `*b` indeterminate where `c` is zero, and only the doubt that the call
may have freed `b` was reported (#1046). A store, `memcpy` or `memset` through
the same `p` is a may-write and leaves the read doubted.

## Decision Drivers

* A call and a store through the same pointer should not answer the same write
  two ways.
* The output-parameter idiom, a block handed by a pointer that certainly names
  it, must stay believed: doubting it refused correct programs across the
  corpus (#778, #782).
* The question a local's holders are asked already exists (ADR-0103 rule 1);
  a block should be asked the same of the arguments that name it.

## Considered Options

* Ask the blocks an argument of the call names directly.
* Ask every block the call reaches, through other blocks too, as `named_alone`
  asks of a local's holders.
* Keep the belief for every exposed block.

## Decision Outcome

Chosen option: "Ask the blocks an argument names directly", by the owner's
decision. A block an argument names is believed written only where some
argument names it alone: its value names that one site and nothing else, no
local, is not possibly null, and the site stands for one block
(`Allocations::made_once`, the condition ADR-0103 rule 1 asks of a block
holder). Any other block an argument names keeps what it held before the call,
may-written. `Known::believed_written_only_where_named_alone` holds the rule,
asked in `memory/call.rs` right after the exposure.

Asking every reached block was rejected for now: it is unmeasured, and the
blocks reached through other blocks are where the output-parameter idiom's
deeper forms live, so its cost in refused programs is not known. The belief
for every exposed block was rejected because it is the silence #1046 found.

### Confirmation

In `Known::believed_written_only_where_named_alone`:

* dropping the may-write, so every handed block is written again, silences
  `a_block_a_call_was_handed_through_a_pointer_that_may_be_null_is_doubted_where_read`,
  `a_block_a_call_was_handed_through_a_pointer_that_may_name_another_is_doubted_where_read`
  and
  `a_block_a_call_was_handed_through_a_pointer_that_may_be_null_is_doubted_where_returned`;
* counting a possibly null argument as naming its site alone silences the
  first and the third;
* counting an argument that names two sites as alone silences the second;
* counting an argument that names a local beside the site as alone silences
  `a_block_a_call_was_handed_through_a_pointer_that_may_name_a_local_is_doubted_where_read`.

No case holds the site standing for one block: dropping that condition leaves
the suite green, measured, since a block a loop makes again is one this check
stops following by the next turn. It is kept because it can only add a doubt.

The output-parameter cases that hold #778 pass unchanged, measured when this
was written: no case in `tests/cases.rs` or `tests/routes.rs` moved.

### Consequences

* Good, because a call and a store through a pointer that may be null or name
  another block now answer the block alike.
* Bad, because a block reached only through another block keeps the belief:
  `*box = p; fill(box);` with `p` either `b` or a null still believes `*b`
  written, and is reported, measured when this was written, only by the doubt
  that the call may have freed `b`: a silence once that doubt goes. Two more
  are reported the same way only, measured by review, both older than this
  record: a pointer at one element of a block, `fill(&b[k])`, is taken as the
  whole block (#1050), and a null stored into a local array element,
  `arr[0] = 0; if (c) arr[0] = b; fill(arr[0]);`, is not read back as
  possibly null (#1052).
* What would reverse this: a measurement showing the asked blocks refuse
  correct programs the corpus relies on, or a declaration saying what a callee
  writes, which would replace the belief altogether.

## More Information

ADR-0101, ADR-0103 rule 1, #778, #950, #1046.
