---
status: "accepted (rules 1 and 4 superseded by ADR-0092, rule 2 by ADR-0094)"
date: 2026-11-09
decision-makers: itsakeyfut
---

# A heap block of pointers of a constant size is written per element, as a local array is

## Context and Problem Statement

A heap block's written state was one `Init` per site, so one element written
marked the whole block written:

```c
int **b = malloc(16);
if (!b) return 0;
b[0] = x;
return b;
```

said nothing, though the caller reads `b[1]`, which nothing wrote. A local
array of the same shape is `SC0411`, because ADR-0057 keeps a local array of
at most 64 pointers per element. A heap block had no such record, and a
pointer into it had no element: `b[1]` lowers to `*(b + 1)`, and the memory
check's `Offset` says only `Zero` or `NonZero`.

## Decision Outcome

1. **A block of a constant size of two to 64 pointers is kept per element**
   (`Known::block_elements`), made unwritten where the allocation is, and
   its result points at element 0 (`Held::site_element`). A constant move by
   whole pointers moves the element, as `Held::element` does for a local
   array; any other value knows no element.
2. **A certain store of one pointer at a known element writes that element**,
   and the block's `init` is what its elements say: `Written` where all are,
   `Maybe` where some are (`Known::certainly_written_at`). **A store whose
   element is not known writes the whole block**, as one into a local array
   does (`built::write_effects`), so that a block filled in a loop is not
   doubted; `b[i] = x;` written once and returned is the limit that leaves.
3. **A read at a known element asks that element** (`Known::site_init_at`),
   and any other read asks the block's own `init`.
4. **The elements are read only while the block's `init` is what they made
   it.** Every other path that changes a block's `init` changes it whole,
   and drops them; a block whose elements went is answered by its `init`.
   Where paths meet, and at a loop's back edge, they are joined and made
   unwritten again element by element, a side that kept none counting each
   element as its `init`, and the block's `init` is what they say then.
5. `per_block` counts a site's record at its largest, `BLOCK_ELEMENTS`.

### Confirmation

Each in `tests/cases/memory/`, with its mutation in its note in
`tests/cases.rs`: see the notes on the `a_heap_block_*` cases and
`a_sites_per_element_record_costs_what_its_types_take` in
`crates/safec-ir/src/memory.rs`.

* `a_heap_block_written_at_one_element_and_returned_is_doubted` and its
  stored-out twin go silent when a store at a known element writes the whole
  block.
* `a_heap_block_read_at_the_element_written_is_not_doubted` is doubted when a
  read asks the block's `init`.
* `a_heap_block_filled_in_a_loop_and_returned_is_not_doubted` is reported
  when a store whose element is not known writes nothing of a block kept per
  element.
* `a_heap_block_written_at_every_element_and_returned_is_not_doubted` is
  reported when a constant move leaves the element where it was.
* `a_heap_block_element_written_and_then_copied_over_unwritten_is_doubted_where_read`
  goes silent when neither the copy path's doubt nor the belief at an
  exposure drops the elements; no other drop alone is held by a case.
* `a_heap_block_filled_by_a_pointer_walked_through_it_in_a_loop_is_not_doubted`
  is doubted when a block's elements are joined only where both sides keep
  them, and
  `a_heap_block_written_before_an_inner_loop_makes_it_again_is_doubted_where_read_after`
  goes silent when a back edge keeps a block's elements where its `init` did
  not change.

### Consequences

* Good, because a heap block written in part is `SC0411` where it leaves and
  where its unwritten element is read.
* Bad, because a read through a pointer read out of memory knows no element,
  so a block written in part is doubted there at an element that was
  written: `a_load_moved_by_arithmetic_and_read_after_a_free_is_unproven`
  gains such a doubt as a warning, and
  `a_load_moved_by_arithmetic_and_handed_on_after_a_free_is_unproven` as an
  error. Before, the block was believed written whole, which
  also left a read of its unwritten element silent.
* Bad, because a store whose element is not known writes the whole block, the
  limit a local array has too.
* Neutral, because a function past the size bound by up to 96 bytes a local
  a block is now refused where it was read.

## More Information

* [ADR-0057](./0057-a-pointer-read-out-of-fresh-memory-is-reported-where-it-is-read.md),
  where a local array was first kept per element, and
  [ADR-0069](./0069-a-pointer-read-out-of-memory-nothing-wrote-is-reported-where-it-is-read-and-a-loop-may-run-zero-times-unless-it-is-shown-to-run.md),
  which carries that rule now.
* #873, and #870, whose oblique round found the silence.
