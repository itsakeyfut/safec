---
status: "accepted (its open silence about a copy closed by ADR-0088)"
date: 2026-11-10
decision-makers: itsakeyfut
---

# A copy of part of a pointer is recorded on the block, and asked where the block is held as memory of pointers

## Context and Problem Statement

ADR-0073 rule 4 asks a library copy whether it writes part of a pointer
into memory believed to hold pointers, at the copy, by what was believed
there. The belief is on the value, so a copy of four bytes into a block that
nothing typed had reached yet was asked nothing, though the block was then
held, stored or handed on as memory of pointers:

```c
void b(void ***pp) { int i = 1; if (!pp) return; void *m = malloc(16);
    if (!m) return; memcpy(m, &i, 4); void **q = m; *pp = q; }
```

The caller reads a pointer made of four bytes of an `int` and four others.
An alias made before the typed hold, a block handed to a function that takes
it as a `void **`, and one stored into a `void **` slot with no typed local
were silent the same way (#832).

## Decision Drivers

* A silence about a program whose caller or callee reads a pointer nobody
  wrote whole.
* A copy of part of a pointer into memory that only ever holds `int`s is
  defined and common, and the copy alone cannot tell the two apart.

## Considered Options

* Record the partial copy on the block, and ask where the block is held as
  memory of pointers.
* Record on the block the type it was held as, and ask at the copy.
* Track the block's layout per pointer-sized slot.

## Decision Outcome

Chosen: the record of the copy, by the author.

1. **A library copy that may write part of a pointer records its span on
   each block it lands in**, `Known::part_written`, whatever either side's
   type says. Joined keeping the earlier span, and kept where the site is
   made again, as `left_at` is.
2. **A block that leaves as memory of pointers is asked by it**, as
   `SC0413` at the copy: at the function's exit, at every block the exit asks
   whether it was written (ADR-0076, ADR-0085, ADR-0086), and at a `return`,
   at every block the `return` asks.
3. **A block handed to a parameter whose type holds pointers is asked by it
   at the call**, at the copy: what the argument names, and below it as far
   as the parameter reads pointers, by the same walk. The level below the
   argument is walked with one level less, since a `void **` parameter reads
   the pointer stored in the block, not what that pointer points at.
4. **Once per copy**, whichever door asks first: a copy already reported as
   a store departure, which knows the type, is not said again, and a store
   departure said later takes the place of this report.

A block rewritten whole after the partial copy keeps the record. That is a
doubt the reader sees. A whole copy, or a `realloc`, of a block with the
record does not carry it to the new block, which is a silence left open.

### Confirmation

Each in `tests/cases/memory/`, with its mutation in its note in
`tests/cases.rs`:

* The four `a_partial_pointer_copied_into_a_block_*_is_reported` cases go
  silent when nothing is recorded (rule 1), and the one handed to a call when
  only the exit asks (rule 3).
* `a_partial_pointer_copied_into_a_block_that_never_leaves_is_not_reported`
  is reported when every block with the record is asked (rule 2's leaving).
* `a_partial_pointer_copied_into_a_block_of_ints_that_leaves_is_not_reported`
  is reported when a block is asked wherever it is stored (rule 2's type).
* `a_partial_pointer_copied_into_a_block_already_held_as_pointers_is_reported_once`
  says it twice when the exit's ask is pushed without the check, and
  `a_partial_pointer_copied_into_a_typed_block_after_it_is_handed_on_in_a_loop_is_reported_once`
  when a store departure no longer takes its place (rule 4).
* `a_partial_pointer_copied_into_a_returned_block_is_reported` goes silent
  when the `return` asks nothing of the record (rule 2).
* `a_partial_pointer_copied_into_a_block_handed_on_one_level_down_is_reported`
  goes silent when the call does not walk the level below the argument, and
  `a_partial_pointer_copied_into_a_block_below_a_box_handed_to_a_void_pointer_pointer_is_not_reported`
  is reported when that level is walked with the parameter's full levels
  (rule 3).
* `a_partial_pointer_copied_into_a_block_handed_to_an_int_pointer_parameter_is_not_reported`
  is reported when every parameter is asked whatever its type (rule 3).
* `a_partial_pointer_copied_on_one_arm_into_a_block_handed_to_a_pointer_parameter_is_reported`
  goes silent when the record is left out of the join (rule 1).

### Consequences

* Good, because the order of the copy and the typed hold no longer decides
  whether the copy is asked.
* Bad, because a block rewritten whole after a partial copy is doubted, and
  `Known` gains a table.

## Pros and Cons of the Options

### Record the type it was held as, and ask at the copy

* Good, because ADR-0086 already keeps that record.
* Bad, because a copy made before anything typed reaches the block, and one
  handed to a call, stay silent.

### Track the layout per slot

* Good, because it answers all four and could fold several marks into one.
* Bad, because it is a change the size of the representation of memory, out
  of proportion with this question.

## More Information

* [ADR-0073](./0073-what-leaves-a-function-is-asked-by-how-it-was-declared-and-what-wrote-it.md),
  whose rule 4 this extends.
* [ADR-0086](./0086-a-block-records-the-type-it-was-held-as-and-a-void-pointer-level-is-asked-where-that-record-says-pointers.md),
  whose walk decides which blocks the exit asks.
