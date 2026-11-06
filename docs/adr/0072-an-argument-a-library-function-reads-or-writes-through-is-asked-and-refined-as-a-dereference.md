---
status: "proposed"
date: 2026-11-06
decision-makers: itsakeyfut
---

# An argument a library function reads or writes through is asked as a dereference, and refined as one

## Context and Problem Statement

The nullability check asks dereferences, and arguments a callee promised are not
null ([ADR-0037](./0037-a-nonnull-parameter-is-believed-by-its-body-and-checked-at-every-call-in-its-translation-unit.md)). A library function that reads or writes through an argument
promises nothing, so `int *d = 0; memcpy(d, &k, 4);` built silent while `*d =
k;` is `SC0403`, though C17 7.1.4 p1 makes a null argument to such a function
undefined unless its description says otherwise, and 7.24.1 p2 keeps that true
of a copy of no bytes. Where the copy wrote part of a pointer, the memory
check's `SC0413` happened to stand in for it, which held #815 (#819).

## Decision Drivers

* A null argument to these functions is the same undefined behaviour as a null
  dereference, and the reader's remedy is the same: test the pointer.
* `crate::library::LIBRARY` already names, per function, the arguments it reads
  as an object (`reaches`), so that an analysis besides the memory check can read
  them rather than keep a copy to drift (ADR-0063).
* A positive claim this check makes is one it must be able to defend: a refinement
  after the call is sound only if the call could not have returned otherwise.

## Considered Options

* Every argument the library table says a function reaches, asked as a
  dereference of it and refined to not null after the call.
* The copies alone, `memcpy` and `memmove`.
* Asked, but not refined after the call.
* A question of its own, with its own words.

## Decision Outcome

Chosen option: every argument the table says a function reaches, because one rule
over one table answers every row added later, and a function that reads through
its argument is undefined on a null one whichever function it is.

An argument is asked as the dereference of it, `*arg`, through the same `report`
the dereferences of an element go through, so a local's own row and a pointer
read out of memory are asked as they are there (#333). After the call, each such
argument's local is not null on the path that returned, as after a dereference
that did not trap (ADR-0025): a null one made the call undefined and was asked at
the call. The callee is read by name only where this unit does not define it,
as the memory check reads it (ADR-0071 rule 1).

### Confirmation

Filled when #819 lands: the cases its pull request adds, each failing under the
mutation it names, for asking no argument, asking only the copies, and not
refining after the call.

### Consequences

* Good, because `memcpy(d, …)`, `memset(d, …)` and `strlen(s)` with a pointer
  that may be null are `SC0403` as `*d` is, and #815's narrowing no longer
  silences the null destination it stood in for.
* Bad, because every pointer handed to such a function that this check has not
  shown is not null is a doubt now, a parameter above all: measured on a
  prototype, sixty-five corpus cases gain an `SC0403` doubt and nine of them go
  from exit 0 to exit 1. None gains a proof or loses a report, and none changes
  its exit under `--allow-unknown`. This is the doubt `*out = x;` already raises
  about the same parameter.
* What would reverse this: a function that reads through an argument and
  accepts a null one, which C17 says of some outside these rows (`free`,
  `realloc`, `fflush`); such a row would need a field saying so before it could
  join `reaches`.

## Pros and Cons of the Options

### The copies alone

* Good, because fifty-two cases move rather than sixty-five, and seven exits
  rather than nine.
* Bad, because `memset(0, …)`, `strlen(0)` and `memcmp(0, …)` stay silent, the
  same undefined behaviour, and the rule becomes a list of names to keep beside
  the table.

### Asked, but not refined after the call

* Good, because it adds no positive claim.
* Bad, because one defect is said twice: `memcpy(out, in, 4); return *out +
  *in;` is four doubts rather than two, measured, and the second pair says
  nothing the first did not.

### A question of its own, with its own words

* Good, because a reader is told that a library function, not their own code,
  goes through the pointer.
* Bad, because the remedy is the same, and the words a dereference takes already
  point their caret at the call; a fourth `Asked` and its arm in `words.rs` buy a
  sentence and nothing a reader can act on differently.

## More Information

* [ADR-0025](./0025-a-dereference-that-was-reported-refines-the-pointer.md), the
  refinement this extends to a library call.
* [ADR-0063](./0063-the-memory-check-has-one-kind-of-place-one-kind-of-cell-and-one-description-of-a-write.md),
  whose first move put the library table at crate level, where a second
  analysis can read it.
* #819, and #815, which this unblocks in part.
