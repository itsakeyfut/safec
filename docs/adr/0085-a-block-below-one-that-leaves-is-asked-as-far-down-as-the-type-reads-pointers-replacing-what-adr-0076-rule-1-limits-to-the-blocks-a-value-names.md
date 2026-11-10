---
status: "accepted"
date: 2026-11-10
decision-makers: itsakeyfut
---

# A block below one that leaves is asked as far down as the type reads pointers, replacing what ADR-0076 rule 1 limits to the blocks a value names

## Context and Problem Statement

ADR-0076 rule 1 marks the blocks a leaving store's value names as having left,
"not what they hold below". A block put into a box before the box leaves is
therefore never asked whether it was written:

```c
void s(int **** _Nonnull out) { int **b = malloc(8); int ***box = malloc(8);
    if (!box) return; *box = b; *out = box; }
```

The caller reads `***out` out of `b`'s block, which nothing wrote, and this
was silent. The same two statements in the other order, `*out = box; *box =
b;`, are `SC0411`, because the second store lands in memory that has already
left. A `return box;` was silent too, since ADR-0073 rule 3 asks the returned
blocks alone. Order should not decide whether an indeterminate pointer reaches
the caller (#900).

## Decision Drivers

* A silence about a program the caller cannot use without reading an
  indeterminate pointer.
* The store, the library copy and the `return` are three doors for one
  question, and #890 found two of them answering it differently.

## Considered Options

* Mark the blocks below when the box leaves, as far down as the type reads
  pointers.
* Walk what a block that left holds at the function's exit.
* Keep rule 1, and write the order down as a limit.

## Decision Outcome

Chosen: mark at the moment of leaving, by the author.

1. **A write that leaves marks the blocks its value names and the blocks they
   hold**, level by level, for as many levels as its type reads pointers
   (`Allocations::levels_holding_pointers`): `int ***` stored into reads two,
   the box and what it holds. A library copy reads the more of its two sides,
   and a side whose type says nothing follows every level.
2. **A `return` asks the same levels** of the returned value, by the return
   type. Where only a block below is in doubt, the finding names no local,
   since "`box` points at memory nothing wrote" is false of a box that was
   written.
3. **One walk for the three doors**, `Known::held_below`, so one cannot
   follow a level the others stop at.

What a box held when it left stays marked if it is swapped out later. That
is a doubt the reader sees.

### Confirmation

Each in `tests/cases/memory/`, with its mutation in its note in
`tests/cases.rs`:

* `a_block_put_into_a_box_before_the_box_leaves_is_reported_where_the_function_returns`
  goes silent when a store marks one level whatever its type (rule 1).
* `a_block_put_into_a_box_that_is_copied_out_is_reported_where_the_function_returns`
  goes silent when the copy marks one level (rule 1, the copy).
* `a_block_put_into_a_box_that_is_returned_is_reported_at_the_return` goes
  silent when the `return` asks the blocks it names alone, and says `box`
  when the local is named whatever level is unwritten (rule 2).
* `a_block_of_ints_put_into_a_box_that_leaves_is_not_asked` is reported when
  every level is followed whatever the type (rule 1's levels).

### Consequences

* Good, because the order of filling a box and letting it leave no longer
  decides whether its contents are asked, at any of the three doors.
* Good, because two corpus cases gain a true doubt: a caller reads a pointer
  out of a block nothing wrote, one held in a box stored through a pointer
  read out of memory, one copied out through a `char` buffer.
* Bad, because a block swapped out of a box after the box left is still
  asked.

## Pros and Cons of the Options

### Walk at the exit

* Good, because it asks what the box holds at the exit, so a block swapped
  out is not asked.
* Bad, because each marked block would need to record the level it left at,
  to know how far its own contents are read, which is a second table beside
  `left_at` for the same visible doubt.

### Keep rule 1

* Good, because nothing moves.
* Bad, because the first program stays silent.

## More Information

* [ADR-0076](./0076-a-block-a-store-lets-leave-is-asked-at-the-functions-exit-whether-it-was-written-replacing-what-adr-0073-rule-3-limits-to-a-return.md),
  whose rule 1 this replaces for the levels below.
* [ADR-0073](./0073-what-leaves-a-function-is-asked-by-how-it-was-declared-and-what-wrote-it.md),
  whose rule 3 this extends to the levels below a returned block.
