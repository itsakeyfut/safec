---
status: "accepted"
date: 2026-11-09
decision-makers: itsakeyfut
---

# A stored pointer is asked whether it was not null at the store itself, replacing the block-end test ADR-0078 rule 1 reads

## Context and Problem Statement

ADR-0078 rule 1 doubts a block a store let leave possibly unwritten where the
local stored was established not null at the end of the store's block. The
end of the block is not the store, and two programs fall between the two
(#861):

```c
void g(int ***out, int *x, int c) {
    int **b = malloc(8);
    if (!b || !out) return;
    int **t = b;
    *out = b; b = 0;
    if (c) *t = x;
}
```

Here `b` is null at the block's end and not null where it was stored, so the
block leaves unwritten when `c` is zero and nothing is said. And a store whose
block ends in a call, `*s = m; memcpy(*t, y, 8);`, is never established not
null at the end, since the answer at a call reads only "null" (ADR-0027).

## Decision Outcome

1. While the nullability check replays a block to answer at its terminator,
   it records, for each store through a projection whose value is a local by
   name, whether that local was established not null before the store
   (`NullAtTerminators::stored_non_null(block, element)`). ADR-0078 rule 1
   reads that answer for the store found by its span, in place of the
   block-end one.
2. **An answer inside a block, for not null only.** ADR-0027 keeps the
   nullability check's answers at terminators, because the free exemption
   reads "null" and a wrong one is a silence. This answer adds a doubt where
   it holds, so a wrong one is a doubt too many, and nothing that exempts
   reads it.
3. A write under a test that does not settle the pointer,
   `if (b && c) *b = x; *out = b;`, stays silent. It needs whether the block
   was written kept per path beside whether the pointer was null, and is
   designed with the `return` form in #834.

### Confirmation

Each in `tests/cases/memory/`, with its mutation in its note in
`tests/cases.rs`:

* `a_block_stored_out_and_its_local_nulled_after_in_the_same_block_is_doubted`,
  `g` above, goes silent when the store is asked at its block's end.
* `a_block_written_under_its_null_test_stored_out_and_its_local_nulled_after_is_not_reported`,
  `if (b) *b = x; *out = b; b = 0;`, is reported when every store is answered
  not null.
* `a_copy_through_a_parameter_that_may_not_reach_a_block_leaves_it_in_doubt`
  gains the doubt at `*s = m;`, whose block ends in `memcpy`.

### Consequences

* Good, because `g` is `SC0411`, and so is a store whose block ends in a call.
* Neutral, because the block-end row ADR-0078 read is still kept, for the
  `return`.

## More Information

* [ADR-0078](./0078-a-block-a-store-lets-leave-possibly-unwritten-is-doubted-where-the-stored-pointer-was-not-null-replacing-what-adr-0076-rule-3-leaves-silent.md),
  whose rule 1 test this replaces and whose rule 2 limits this narrows.
* [ADR-0027](./0027-an-unsafe-conclusion-is-about-an-execution-this-function-has.md),
  whose conditions this answer keeps by being read only where it adds a doubt.
