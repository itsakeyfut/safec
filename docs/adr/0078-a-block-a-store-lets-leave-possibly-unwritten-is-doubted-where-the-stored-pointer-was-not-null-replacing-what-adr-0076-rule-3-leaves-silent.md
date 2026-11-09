---
status: "superseded by ADR-0080"
date: 2026-11-08
decision-makers: itsakeyfut
---

# A block a store lets leave possibly unwritten is doubted where the stored pointer was not null, replacing what ADR-0076 rule 3 leaves silent

## Context and Problem Statement

ADR-0076 rule 3 reports, at the function's exit, a block a store let leave
only where it is certainly unwritten. Where it is possibly unwritten, it is
silent (#855):

```c
void g(int *** _Nonnull out, int c, int *x) {
    int **b = malloc(8);
    if (!b) return;
    *out = b;
    if (c) *b = x;
}
```

Doubting `Maybe` everywhere refused a store after a null test's join,
`if (b) *b = x; *out = b;`. There, the path where the allocation failed stores
a null and leaves the block unwritten. A `return` meets the same trap and asks
`Maybe` only where the returned pointer is established not null at the
`return` (#828).

## Decision Outcome

1. At the exit, a block possibly unwritten is reported, in words of its own,
   where the local the store stored was established not null at the store's
   block (`NullAtTerminators::established_non_null`), as the `return` asks
   it. `Known::left_at` keeps the store's span alone, and the store is found
   by that span: its block and the local it stored, by name.
2. A store of anything but a local by name, or a span that names no store,
   asks nothing possible. Two more shapes are silent, stated limits found by
   review (#861): a write under a test that does not settle the pointer,
   `if (b && c) *b = x; *out = b;`, as a `return` is silent for it (#834);
   and the local reassigned after the store in its block, `*out = b; b = 0;`,
   since the test is read at the block's end, not at the store.
3. Where two stores marked one block, the earlier store is asked, as its
   span is.

### Confirmation

Each in `tests/cases/memory/`, with its mutation in its note in
`tests/cases.rs`:

* `a_block_stored_out_and_written_on_one_arm_is_doubted_where_the_function_returns`,
  `g` above, goes silent when `Maybe` is never asked at the exit.
* `a_block_written_before_it_is_stored_out_is_not_reported_beside_its_failed_allocation`,
  `if (b) *b = x; *out = b;`, is reported when `Maybe` is asked without the
  null test, and so are two `bounds` cases of a callee filling `*p`.
* Two corpus cases gain the doubt: a write through a parameter that may not
  reach the block, which is true, and a write on a loop's first turn under
  `if (i == 0)`, the cost below.

### Consequences

* Good, because `g` is `SC0411`.
* Bad, because a block written on a loop's first turn under `if (i == 0)` is
  doubted, the same cost the `return` carries.

## More Information

* [ADR-0076](./0076-a-block-a-store-lets-leave-is-asked-at-the-functions-exit-whether-it-was-written-replacing-what-adr-0073-rule-3-limits-to-a-return.md),
  whose rule 3 this replaces.
