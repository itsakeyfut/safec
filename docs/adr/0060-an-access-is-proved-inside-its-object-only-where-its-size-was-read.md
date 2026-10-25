---
status: "accepted"
date: 2026-10-25
decision-makers: itsakeyfut
---

# An access is proved inside its object only where its size was read, and a pointer is believed whole where it arrives

## Context and Problem Statement

The memory check asked whether an allocation was live and whether a pointer
might be null. It never asked whether an access lands inside the object it
reaches. Measured on `main` at `32f04518`, `--safety memory`:

```c
int *p = malloc(1); if (p == 0) { return 1; } *p = 5;
```

built an executable with nothing said, and so did `int **pp = malloc(4); ...
*pp = 0;` on every target whose pointers take eight bytes. A pointer moved off
its start, `int *q = p + 5;`, was stopped only by `SC0403`, because the
nullability check does not carry "not null" through arithmetic, and a null test
on `q` removed that. Issue #622 carries the evidence. Each is saying safe about
a program C makes undefined, which [`docs/safety-model.md`](../safety-model.md)
calls the worst answer this compiler can give, and #621 was about to let the
backend write the pointer arithmetic that had kept the second shape from
becoming an executable.

## Decision Drivers

* **Read or `Unknown`, and nothing answered safe without being read**, which is
  Phase 11's rule in [`docs/roadmap.md`](../roadmap.md).
* **A proof where C gives one.** `malloc(1)` written as an `int` lands outside
  on every execution.
* **A function's body is read on its own.** Nothing here reads a caller, so what
  a parameter points at has to be either believed or doubted, and ADR-0041 and
  ADR-0042 already answer that shape for the freed question.

## Considered Options

* **Believe a pointer whole where it arrives, and ask where it leaves.**
* Believe parameters and call results, and doubt every pointer read out of
  memory.
* Believe nothing.

## Decision Outcome

Chosen option: **believe a pointer whole where it arrives, and ask where it
leaves**, under `SC0412` for an access and, in #623, `SC0413` for a pointer
handed on.

**What an object takes is read where it is made.** `Known::extent` holds, per
site, `Extent::Exactly(n)` for an allocation asked for with constants
(`malloc(n)`, `aligned_alloc(a, n)`, `calloc(m, n)` as `m * n`, `realloc(p, n)`)
and `Extent::AtLeast(n)` for what is only believed. A size this check cannot
read is `AtLeast(0)`. A local whose address a pointer holds is its own size,
exactly, which `TranslationUnit::size_of` answers from the target.

**A pointer that arrives is believed to point at the start of one whole object
of the type it arrives as**: a parameter, through its site's extent; what a
call this check cannot read returns; and a pointer read out of memory, through
`Held::believed`, which is the size of what the pointer *read* points to and
not of the local it lands in, so the `void *` round trip RK-068 is about
believes nothing. A `void *` arrives believing nothing.

**A pointer that leaves is asked** whether it points at one whole object of the
type it leaves as: an argument, a returned pointer, and a pointer stored
through a projection. That half is #623, and until it lands what is believed is
asked nowhere, which is where `main` already was.

**The question**, `report::fits`, is asked at every access `used` asks about,
of the pointer the place goes through first; a deeper level goes through a load
of exactly the type it reads, so it fits by what is believed. In order: a
pointer holding nothing is the nullability check's; a lost, foreign or escaped
one is `Unknown` (RK-061); a believed one fits where the belief is wide enough;
one off its start, or perhaps off it, is `Unknown`, since no distance is carried
(ADR-0036); an edge that may be the address of something inside a local is
`Unknown` (`Held::inner_address`); and otherwise every object it may reach is
folded. Every lower bound at least as wide as the access is proved. Every one
exact and narrower is **`Unsafe`**, and so an error that `--allow-unknown`
leaves an error. Anything between is `Unknown`, because a may-set proves nothing
from one member (RK-035).

`memset`, `memcpy` and `memmove` with a constant length are an access of that
many bytes at each pointer they touch, and with any other length `Unknown`. The
string functions write as far as a string runs, which nothing here reads, so
their destination is always `Unknown`.

`Target` gains `pointer_bits`, measured per row with `clang`, which amends
ADR-0013's "a pointer's width is not here": the memory check is now a reader.

### Confirmation

Each mutation applied on its own, the workspace run, and the file restored.

* `fits` answering every access inside:
  `a_write_through_a_pointer_moved_past_its_allocation_after_a_null_test`,
  `an_int_written_into_one_byte`, `a_pointer_written_into_four_bytes` and the
  rest of `tests/cases/bounds/` that report go silent.
* `made_extent` leaving the allocator's argument unread: `an_int_written_into_one_byte`
  turns `Unknown`, `an_int_written_into_one_byte_is_an_error_where_unknown_is_allowed`
  builds, and `an_int_written_into_four_bytes_is_inside` is reported.
* `calloc` reading its first argument only:
  `a_calloc_of_two_constants_is_their_product`.
* `Extent::joined` keeping the larger bound: `a_join_of_four_bytes_and_one_byte`
  goes silent.
* `Offset::NonZero` read as `Zero` in `fits`:
  `a_write_through_a_pointer_moved_past_its_allocation_after_a_null_test` goes
  silent.
* `read_through` believing a load by the destination's type rather than the
  read's: `a_char_address_stored_as_void_and_read_as_int` goes silent.
* a local's address folded as `AtLeast(0)`:
  `a_char_address_converted_through_void_and_written_as_int` turns `Unknown`.
* a parameter's extent `AtLeast(0)` where the function starts:
  `a_parameter_written_through_is_inside` is reported; an opaque call's result
  not believed: `a_call_s_result_is_believed_whole` is reported.
* `touched_by_length` removed: `a_memset_wider_than_its_allocation` and
  `a_memset_of_a_length_nothing_bounds` go silent.
* `x86_64-pc-windows-msvc` given 32 pointer bits: `a_pointer_written_into_four_bytes`
  goes silent, and `every_target_is_what_clang_says_it_is` fails.
* a new `Kind` with no words: `error[E0004]` at `memory_finding` in
  `crates/safec-c/src/driver/words.rs`.

### Consequences

* Good, because the three programs of #622 are reported, and `malloc(1)`
  written as an `int` is a proof.
* Good, because an access the check cannot read is `Unknown` rather than
  silence, so #621's arrays and pointer arithmetic land under Phase 11's rule.
* Bad, because **a pointer that is lost, foreign or escaped is doubted about
  its size as well as about whether it was freed**, so 192 corpus cases that
  already reported a doubt gained an `SC0412` beside it, and the output-parameter
  idiom, `get(&p); if (p) *p = 1;`, is `Unknown`. The escape is RK-061's rule:
  something may have written the local where this check could not see.
* Bad, because every access off a pointer's start is `Unknown`, so `p[1]` does
  not build at `--safety memory` without `--allow-unknown`. That is what Phase
  11's rule allows until a distance is carried.
* Bad, because `strcpy` and its family are always `Unknown` at their
  destination, and a list walked through `void *` is `Unknown` at every step.
* Bad, because what is believed is not yet asked anywhere, until #623.
* What would reverse this: a lattice that carries a distance and a length per
  allocation, which would make `Extent` and `Offset` one fact; or annotations
  that say how many objects a parameter points at.

## Pros and Cons of the Options

### Believe on arrival, ask on departure

* Good, because `void f(int *p) { *p = 1; }` builds, as every C function with a
  pointer parameter needs to.
* Good, because it is the shape ADR-0041 and ADR-0042 gave the freed question,
  so the two halves of a boundary are asked the same way.
* Bad, because code `safec` never reads can hand in a pointer that is not
  whole, and `safec` then says safe about the body and is believed. That reach
  is bounded the way a written `_Nonnull` is.
* Bad, because a pointer one past the end that is handed on will be a false
  `Unknown` once #623 asks.

### Doubt every load

* Bad, because `**pp` and every walk of a list become a false `Unknown`, which
  a reader sees, and which is the commonest shape of pointer-holding data.

### Believe nothing

* Bad, because `void f(int *p) { *p = 1; }` would not build at `--safety
  memory`, and neither would nearly the whole corpus.

## More Information

* `crates/safec-ir/src/memory/parts.rs`: `Extent`, `Held::believed`,
  `Held::inner_address`.
* `crates/safec-ir/src/memory/report.rs`: `fits`, `outside`,
  `touched_by_length`.
* `crates/safec-ir/src/memory/transfer.rs`: `made_extent`, `pointee_size`, the
  extent where a parameter and a call's destination are made, and its join.
* `crates/safec-ir/src/target.rs`: `Target::pointer_bits`.
* Issue #622 and its design comment; #623 for the departures.
