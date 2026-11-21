---
status: "accepted"
date: 2026-11-21
decision-makers: itsakeyfut
---

# Read-only storage is asked at every level of what leaves, and believed at every level of what arrives

## Context and Problem Statement

ADR-0120 marks a pointer that may name storage the program may not modify. It
asks that pointer where it is written through, and where it leaves by a call
or a `return`. In each case it asks the pointer's own value only.

The second hunt before #615 closed found that one level down nothing asks
(#1134). A read-only pointer can sit in a local whose address is handed on, in
a returned block, behind an out-parameter, or in a static. The other side reads
it out of memory with no mark and writes through it. Each such program builds
clean and faults when run.

The same holds on the arriving side. A callee that casts a `const int **`
parameter to `int **` and loads through it holds an unmarked pointer, although
the type it arrived as said `const` at that level. Until #1122 the cast covered
some of these routes; with the cast followed, nothing does.

## Decision Drivers

* Phase 11's rule: a construct the check cannot read is `Unknown`, never safe.
* ADR-0068 already asks a pointer where it leaves, by a call, a return or a
  store, at every level the width rule reaches (ADR-0085). A second walk of its
  own would drift from that one.
* ADR-0120's rule is about the object, not the spelling. A level of
  indirection is one more spelling.

## Considered Options

* Ask every level of what leaves, by call, return and store into memory that
  leaves, against the type the other side reads it as. Believe every level of
  what arrives as its arrival type says.
* The same, without stores: ask calls and returns only.
* Believe nothing that arrives, so that every pointer read out of a parameter's
  memory may name read-only storage.

## Decision Outcome

Chosen option: the first, by the owner's decision on #615's third design run.
The second leaves an out-parameter or a static as a silent route (row 6). The
third doubts every write through a pointer read out of a caller's memory
(row 4, on most correct programs).

* **Leaving.** Where ADR-0068 asks a departing value, a call's argument, a
  `return`, or a store whose memory leaves, the read-only question is asked
  of every level the departure walk reaches. A level is reported `SC0417`
  where it may hold a read-only pointer and the type the other side reads it
  as points at something not `const`. That type is the parameter's type, the
  result's type, or the stored-into type. `free` and `realloc` are left to the
  free rules, as in ADR-0120.
* **Arriving.** Memory a parameter points at, and memory a call's result
  points at, is believed read-only at a level where the type it arrived as
  points at `const`. A load out of that level carries the mark whatever it is
  cast to. This is the memory counterpart of #1122's rule for a local declared
  as a pointer to `const`.
* **A comparison carries no mark**, since its result is `0` or `1` (#1135).
  Every other value is asked whatever its type, because an integer may hold a
  pointer's bytes and the other side may read it as one.

### Confirmation

`read_only_storage_below_what_leaves_or_arrives_is_unproven` holds a function
per route. Each mutation below silences the function named after it:

* skip the levels below at a call (`handed_read_only`): `handed_below`;
* skip them at a `return` (`returned_read_only`): `returned_block`;
* skip `stored_read_only`: `out_parameter` and `into_a_static`;
* leave a parameter's memory unmarked at entry: `cast_after_arrival`;
* leave a call result's memory unmarked: `cast_after_a_result`;
* skip the mark on a pointer read through one: `three_levels_arrived`.

The same case's functions that only read, that hand the pointer to a reader
whose type keeps the `const`, and that let a writable pointer leave stay
silent. `a_value_computed_from_a_pointer_to_const_is_not_a_write`
fails where a comparison keeps the mark.

### Consequences

* Good, because every level of what leaves is asked by the doors that already
  ask its width, so a level added to that walk is asked here too.
* Bad, because a read-only pointer stored where it leaves and never written
  through is doubted. `char *G; G = "ab";` is the common case, the same cost
  ADR-0120 accepts for `w("ab")`.
* What would reverse this: a whole-unit summary of which callees write through
  which levels, which would let a callee that only reads go unreported.

## More Information

ADR-0068, ADR-0085, ADR-0120, #615, #1122, #1134, #1135.
