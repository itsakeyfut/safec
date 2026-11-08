---
status: "accepted"
date: 2026-11-08
decision-makers: itsakeyfut
---

# A store's exposure believes nothing written; only code that runs does

## Context and Problem Statement

ADR-0044 has a store that this check cannot place expose what it carries.
`Known::expose` then believes every exposed block written, the
output-parameter idiom's belief, `fill(p); q = *p;` (#778). That belief is
about code this check cannot read writing the block. A store runs no code.
So a store through a pointer read out of memory believed the very block it
stored written, and nothing about that block was asked again:

```c
void *malloc(int n);
void f(int **** _Nonnull out) {
    int **b = malloc(8);
    if (!b) return;
    int ***t = *out;
    if (!t) return;
    *t = b;
}
```

This builds silent, where `*out = b;` is `SC0411` (#858). The same holds for a
read of `*b` after the store.

## Decision Drivers

* A silence about a block nothing wrote.
* A belief about what unknown code does belongs where unknown code runs.

## Considered Options

* A store's exposure leaves the blocks it carries as they were; a later call
  this check cannot read believes them written, as it does any exposed block.
* Keep the belief at every exposure.

## Decision Outcome

1. **The blocks a store carries keep, across the store's exposure, what was
   known of their writes before it.** The exposure still marks them exposed
   (ADR-0044); only the belief that they were written is withheld.
2. **A call this check cannot read believes them written as before**, since
   `Known::expose` believes every exposed block written at each such call.

### Confirmation

Each in `tests/cases/memory/`, with its mutation in its note in
`tests/cases.rs`:

* `a_block_stored_through_a_pointer_read_out_of_memory_unwritten_is_reported_where_the_function_returns`
  and `a_block_stored_through_a_pointer_read_out_of_memory_is_read_back_unwritten`
  go silent when the store's exposure believes the blocks it carries written
  again.
* The three corpus cases that gain a second doubt are
  `a_pointer_stored_and_read_back_two_levels_down_a_parameter_as_one_place_is_doubted`,
  `an_allocation_stored_through_a_load_and_freed_is_doubted_when_read_back_one_level_further`
  and `an_allocation_stored_through_what_a_call_returned_and_freed_is_doubted_when_read_back`.
* This replaces the limit ADR-0076 rule 3 states for such a store (#858).

### Consequences

* Good, because #858's programs are `SC0411`.
* Bad, because three corpus cases gain a second doubt at a caret already
  doubted: a pointer read back out of memory that this check believes may
  be the stored block, which nothing wrote. Measured on a prototype.

## More Information

* [ADR-0044](./0044-a-store-through-a-pointer-that-may-hold-what-this-check-cannot-name-exposes-what-it-carries.md),
  whose exposure this narrows.
