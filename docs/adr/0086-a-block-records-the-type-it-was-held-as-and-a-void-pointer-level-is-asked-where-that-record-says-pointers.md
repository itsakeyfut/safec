---
status: "accepted"
date: 2026-11-10
decision-makers: itsakeyfut
---

# A block records the type it was held as, and a `void *` level is asked where that record says pointers

## Context and Problem Statement

ADR-0073 rule 3 asks a `return` only where the return type points at a
pointer, and rule 4 reads a `void *` as holding none. ADR-0085 follows the
blocks below a leaving one only as far as the type reads pointers. A `void *`
level therefore ends every question:

```c
void *r(void) { int **b = malloc(8); return b; }
void s(void *** _Nonnull out) { int **b = malloc(8); void **box = malloc(8);
    if (!box) return; *box = b; *out = box; }
```

Both were silent. A caller that converts what it is handed back to `int **`,
which C17 6.3.2.3 p1 defines for a `void *` that came from one, reads an
indeterminate pointer out of `b`'s block. The same programs typed `int **`
are `SC0411` (#902).

## Decision Drivers

* A silence about a program the caller cannot use without reading an
  indeterminate pointer.
* A function that hands its caller a buffer to fill, `void *p = malloc(n);
  return p;`, is correct and common, and doubting it is a doubt at every one.

## Considered Options

* Record on each block the type it was held as, and ask a `void *` level
  where that record says pointers.
* State the silence as a limit.
* Read every `void *` level as saying nothing, and ask it.

## Decision Outcome

Chosen: the record, by the author.

1. **A block records the most levels a local it was held in reads pointers
   at**, `Known::held_levels`, set where a value is copied into a local, the
   point where #830 adds the belief to the value. Joined by the larger, and
   kept where the site is made again, as `left_at` is.
2. **The walk below a leaving block follows each block with the larger of
   the levels its parent left it and its own record** (`Known::held_below`),
   so the store, the copy and the `return` ask a `void *` level alike.
3. **A `return` whose type is a pointer that reads no pointer at its first
   level is asked where a block it names records pointers.** A pointer only:
   asking an `int` read out of such a block moved 26 cases on a prototype.

A block held as `int **` and then used as bytes is asked. That is a doubt the
reader sees.

### Confirmation

Each in `tests/cases/memory/`, with its mutation in its note in
`tests/cases.rs`:

* Five cases go silent when the walk ignores the record, each a block below
  a `void **` box or stored through a `void **` slot (rules 1 and 2).
* `a_block_held_as_pointers_returned_as_a_void_pointer_is_reported_at_the_return`
  goes silent when the `return`'s ask ignores the record (rule 3).
* `a_void_pointer_buffer_never_held_typed_returned_unwritten_is_not_asked`
  is reported when the `return`'s ask takes any pointer return without the
  record, and
  `a_void_pointer_buffer_never_held_typed_stored_out_unwritten_is_not_asked`
  when the walk follows every block one level at least (rule 1's evidence).
* `a_block_held_as_ints_returned_as_a_void_pointer_is_not_asked` is reported
  when any pointer-typed local records a level (rule 1's levels).
* `a_copy_does_not_write_its_source` is reported when the `return`'s ask
  drops the pointer condition (rule 3).

### Consequences

* Good, because a block the function held as pointers is asked whatever
  type it leaves through, and a buffer it never held typed is not.
* Bad, because a block held as pointers and then used as bytes is doubted,
  and `Known` gains a table.

## Pros and Cons of the Options

### State the limit

* Good, because nothing moves.
* Bad, because the reader is told of a silence about a program that reads an
  indeterminate pointer, and it stays one.

### Ask every `void *` level

* Good, because no table is added.
* Bad, because every buffer a function hands its caller to fill is doubted.

## More Information

* [ADR-0073](./0073-what-leaves-a-function-is-asked-by-how-it-was-declared-and-what-wrote-it.md),
  whose rules 3 and 4 this extends.
* [ADR-0085](./0085-a-block-below-one-that-leaves-is-asked-as-far-down-as-the-type-reads-pointers-replacing-what-adr-0076-rule-1-limits-to-the-blocks-a-value-names.md),
  whose walk this widens.
