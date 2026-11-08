---
status: "accepted"
date: 2026-11-08
decision-makers: itsakeyfut
---

# A block a store lets leave is asked at the function's exit whether it was written, replacing what ADR-0073 rule 3 limits to a return

## Context and Problem Statement

ADR-0073 rule 3 reports a `return` of a pointer to a block nothing wrote, where
the caller reads pointers out of it, and only a `return`. Its reason covers a
block handed to a call, which is how a callee is given memory to fill. It does
not cover a block stored where the caller will read it:

```c
void *malloc(int n);
int **returned(void) { int **b = malloc(8); return b; }
void stored(int *** _Nonnull out) { int **b = malloc(8); *out = b; }
```

`returned` is `SC0411`; `stored` prints nothing (#852).

## Decision Drivers

* A silence about a program the caller cannot use without reading an
  indeterminate pointer.
* The caller reads the block after the function returns, not at the store:
  `*out = b; *b = x;` fills it before anyone outside can read it.

## Considered Options

* Ask at the function's exit, for each block a store let leave.
* Ask at the store, as the `return` is asked at the `return`.
* Keep the `return` alone, and write the store down as a limit.

## Decision Outcome

1. **A store into memory that leaves marks the block its value names as having
   left holding pointers**, where the caller reads pointers out of it (the
   stored-into type, through `stored_pointer`), with the store's span:
   `Known::left_at`, a may-fact joined by union, keeping the earlier store's
   span, and kept where the site is made again, since the caller may hold the
   allocation it named before: cleared, a block one turn of a loop let leave
   unwritten went silent; kept, a block stored out and filled every turn is
   doubted at an early return inside the loop, a doubt the reader sees.
2. **At each `return`, a block that left and is certainly unwritten is
   reported** at the store that let it leave, `SC0411`, `Unknown`. Asked by
   the report's one question of what leaves (#851), so the first-turn walk asks
   it too.
3. **Certainly, not possibly.** The mark and `Known::init` join apart, so a
   store after a null test's join, `if (b) *b = x; *out = b;`, is `Maybe` at
   the exit from the path where the allocation failed and the stored pointer
   is null; doubting that refused it, and two corpus cases of a callee
   filling `*p` the same way. A block possibly unwritten at the exit is not
   reported, whatever made it so, a write on one arm after the store among
   them, `*out = b; if (c) *b = x;`: a stated limit, #855. Nor is a block
   freed since it left, which `SC0414` reports. Nor a block stored through a
   pointer read out of memory, `t = *out; *t = b;`: such a store is one this
   check cannot place, so ADR-0074 rule 2 makes it a may-write of every
   block, the stored one among them, #858.

### Confirmation

Each in `tests/cases/memory/`, with its mutation in its note in
`tests/cases.rs`:

* `a_block_a_store_lets_leave_unwritten_is_reported_where_the_function_returns`
  goes silent when `report_left_unwritten` asks no site (rules 1 and 2).
* `a_block_stored_out_before_it_is_filled_is_not_reported` is reported when a
  block is also asked at every element, as soon as it has left (rule 2's
  timing).
* `a_block_stored_out_on_one_arm_unwritten_is_reported_where_the_function_returns`
  goes silent when the mark is joined as a must-fact (rule 1's union).
* `a_block_one_turn_stores_out_and_a_later_turn_leaves_unwritten_is_reported`
  goes silent when the mark is cleared where the site is made again, and
  `a_block_made_again_each_turn_and_filled_after_it_is_stored_out_is_doubted`
  then loses its doubt (rule 1's rebirth).
* `a_block_written_before_it_is_stored_out_is_not_reported_beside_its_failed_allocation`
  is reported when `Maybe` is asked at the exit (rule 3).
* `a_char_buffer_stored_out_unwritten_is_not_reported` is reported when every
  store that leaves marks its blocks (rule 1's type test).
* Five corpus cases gain the doubt, each a store out followed by an early
  `return` before the block is written: true doubts.

### Consequences

* Good, because `stored` is `SC0411`, and `*out = b; *b = x;` stays silent.
* Bad, because a block that may be unwritten when it leaves is silent, and the
  state of `Known` gains a field.

## Pros and Cons of the Options

### Ask at the store

* Good, because no field is added. Measured on a prototype: 12 cases moved.
* Bad, because a store before the fill is doubted, a correct program among the
  12.

### Keep the `return` alone

* Good, because nothing moves.
* Bad, because `stored` stays silent.

## More Information

* [ADR-0073](./0073-what-leaves-a-function-is-asked-by-how-it-was-declared-and-what-wrote-it.md),
  whose rule 3 this replaces for a store.
* #650 asks a store of a pointer later freed at the free, for the same reason
  of timing.
