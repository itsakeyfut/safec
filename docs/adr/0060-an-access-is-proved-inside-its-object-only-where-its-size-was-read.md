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

For a `void *`, which carries no width, three options were weighed (#624):

* **Ask its start and no width.**
* Follow the narrowest width each store into an allocation was asked at, and
  believe a load no wider.
* Follow the type stored in each slot, and report a read through another type.

## Decision Outcome

Chosen option: **believe a pointer whole where it arrives, and ask where it
leaves**, under `SC0412` for an access and `SC0413` for a pointer that
leaves.

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
not of the local it lands in, so a `char`'s address converted to `void *` and
back, which needs no cast, believes nothing. A `void *` arrives believing
nothing. **A belief is no wider than what this check recorded**: a load is
bounded by the objects its memory is recorded to hold, and a call's result by
what it may return that this check saw, so a `char`'s address written through
a `void **` alias and read through an `int **` one, or copied in by `memcpy`,
is `Unknown` rather than four bytes.

**A pointer that leaves is asked** whether it points at the start of an object
as wide as the type it leaves as: an argument, a returned pointer, and a
pointer stored through a projection. `report::departed_at_call` asks an
argument as the callee's parameter is typed, walking the parameters as well as
the arguments so that `void g();` cannot make a call ask nothing, and an
argument past every parameter as its own type; `report::departed_in_element`
asks a `return` and a store. **A pointer to memory lets the pointers stored
there leave with it**: each local it may point at is asked at its own type, as
it stands where the address leaves, so `int *q = v; h(&q);` asks `q`, and the
source of `memcpy` and `memmove` is asked as a store into the destination. A
call this check reads by name is asked how far it reaches instead, unless the
callee is defined in this unit, and `free`, an allocator, `realloc` and a call
that does not return are asked nothing. **A `void *` that leaves is asked
whether it starts an object, and no width**: whoever converts it back believes
that much, and asking nothing let a pointer off its start stored through a
`void **` alias be read back through an `int **` one as whole (#624). What a
`void *` cannot carry is size, so **what a pointer lets leave is asked at the
width the receiver believes**: a local at the wider of its own type and the
receiver's, and memory by what its record says the pointers stored there point
at, so `use(pv)` with `pv` a `void *` local's address and `use` taking `int **`
asks that local four bytes wide. Memory the record does not hold was written
unseen or is the caller's, and is asked where it was written. A read through a
type the object was not stored as is undefined by C17 6.5 p7 as well, and is
reported by size, since effective types are not followed (#627).

**A departure is always `Unknown`**, even where every object is shown
narrower: C leaves `g(v)` with `v` a `char`'s address passed as an `int *`
defined, and only the receiver's read through it is not, while `Unsafe` says
some execution of this function is undefined. The words say narrower where it
was shown. **And a departure does not doubt an escape, a callee's write or a
lost name**, which an access does: each stands for a write asked where it
happened, a store here or in the callee, or a site made again by the same call
at the same size. What a call this check cannot read may write into an escaped
local is believed as its result is, where the local was whole before the
call.

**The question**, `report::fits`, is asked at every access `used` asks about,
of the pointer the place goes through first; a deeper level goes through a load
of exactly the type it reads, so it fits by what is believed, **except where it
may go through a local**, whose value is asked at the access as it stands
there: an assignment to a local is no door, so `v = p + 4;` after `pi = &v;`
was believed at `**pi = 1` (#629). That local's escape is not doubted, since
its address is what escapes it. The locals are the ones the walk a load takes
finds, through an allocation as through a local, and below the first level a
proof needs the pointer to be certainly one local's value, since a may-set
proves nothing from one member. Such a pointer handed on, to a call or to a
library function that takes a length, is asked the same way, and one local
asked twice at one caret is said once, the proof over the doubt. A write
through a pointer into a local starts where what it held and what was written
start, not anywhere, since the local is now asked. A pointer read out of a
local into another is believed only where the local's value fits what it is
read as, so `int *q = *pi; *q = 1;` asks `v` as `**pi` does, and an `int *`
read through an alias of a `void *` a callee filled is not believed (#627). In
order: a
pointer holding nothing is the nullability check's; a lost, foreign or escaped
one is `Unknown`, since something may have written it where this check did not
see; a pointer moved off its start holds something for this question even
where it names nothing, since arithmetic on a null or on a local's address is
not a null; a believed one fits where the belief and the record are both wide
enough;
one off its start, or perhaps off it, is `Unknown`, since no distance is carried
(ADR-0036); an edge that may be the address of something inside a local is
`Unknown` (`Held::inner_address`); and otherwise every object it may reach is
folded. Every lower bound at least as wide as the access is proved. Every one
exact and narrower is **`Unsafe`**, and so an error that `--allow-unknown`
leaves an error. Anything between is `Unknown`, because a may-set proves nothing
from one member: an access that fits one of two allocations a pointer may hold
may still overflow the other. A size is read off a constant as the parameter
receives it, converted as C17 6.5.2.2 p7 converts it.

A library function read by name reaches through the pointers it is handed as
one table, `memory::parts::LIBRARY`, says per argument: by a length, as
`memset`, `memcpy`, `memmove`, `memcmp`, `memchr` and `strncpy`'s destination
do, which is an access of that many bytes where the length is a constant and
`Unknown` otherwise; or as far as a string runs, as `strlen`, `strcmp` and the
copies and concatenations otherwise do, which nothing here reads and so is
always `Unknown`. The same row says what the function does to what it is
handed and what it allocates, so a name cannot be added for one question and
forgotten for the other. A name not in the table is believed, as a parameter
is.

`Target` gains `pointer_bits`, measured per row with `clang`, which amends
ADR-0013's "a pointer's width is not here": the memory check is now a reader.

### Confirmation

Each mutation applied on its own, the workspace run, and the file restored.

* `fits` answering every access inside:
  `a_write_through_a_pointer_moved_past_its_allocation_after_a_null_test`,
  `an_int_written_into_one_byte`, `a_pointer_written_into_four_bytes_is_outside_where_a_pointer_takes_eight` and the
  rest of `tests/cases/bounds/` that report go silent.
* `made_extent` leaving the allocator's argument unread: `an_int_written_into_one_byte`
  turns `Unknown`, `an_int_written_into_one_byte_is_an_error_where_unknown_is_allowed`
  builds, and `an_int_written_into_four_bytes_is_inside` is reported.
* `calloc` reading its first argument only:
  `a_calloc_of_two_constants_is_their_product`.
* the fold in `fits` proving a fit when any object it may reach is wide enough
  rather than every one: `two_allocations_of_different_sizes_are_neither_proved_inside_nor_outside` goes silent.
* `Extent::joined` keeping the larger bound, or dropping its arm for two equal
  sizes: `two_sizes_for_one_object_meet_as_a_bound_below_both` in
  `crates/safec-ir/src/memory/parts.rs`. No C program brings two sizes for one
  site, since a site is one call's destination or one parameter, so nothing in
  the corpus holds it.
* `Offset::NonZero` read as `Zero` in `fits`:
  `a_write_through_a_pointer_moved_past_its_allocation_after_a_null_test` goes
  silent.
* `read_through` believing every pointer read out of memory to point at eight
  bytes, whatever it was read as: `a_char_address_stored_as_void_and_read_as_int`
  goes silent.
* a local's address folded as `AtLeast(0)`:
  `a_char_address_converted_through_void_and_written_as_int` turns `Unknown`.
* a parameter's extent `AtLeast(0)` where the function starts:
  `a_parameter_written_through_is_inside` is reported; an opaque call's result
  not believed: `a_call_s_result_is_believed_whole` is reported.
* `reached_by_library` removed: `a_memset_wider_than_its_allocation` and
  `a_memset_of_a_length_nothing_bounds_is_unknown` go silent.
* `x86_64-pc-windows-msvc` given 32 pointer bits: `a_pointer_written_into_four_bytes_is_outside_where_a_pointer_takes_eight`
  goes silent, and `every_target_is_what_clang_says_it_is` fails.
* a new `Kind` with no words: `error[E0004]` at `memory_finding` in
  `crates/safec-c/src/driver/words.rs`.
* a row of `LIBRARY` without one of its three fields: `error[E0063]`.
* `departed_at_call`, or the `Return` or `Store` door of
  `departed_in_element`, removed: `a_pointer_moved_past_its_allocation_handed_to_a_call`,
  `a_pointer_moved_off_its_start_returned` or
  `a_pointer_moved_off_its_start_stored_into_memory` goes silent. The width
  taken from the argument rather than the parameter:
  `a_char_handed_to_an_int_parameter_is_shown_narrower` goes silent.
* the pointers a departure lets leave with it, not asked at a call or at
  `memcpy`: `a_pointer_in_a_local_whose_address_leaves_is_asked_at_its_own_type`,
  `a_pointer_assigned_after_its_local_s_address_was_taken_is_asked_where_the_address_leaves`
  or `a_pointer_copied_into_memory_by_memcpy_is_asked_as_a_store` goes silent.
  A callee defined here asked by its library name:
  `a_function_defined_with_a_library_name_is_asked_as_any_function` goes
  silent.
* a callee's write into an escaped local believed whatever the local held:
  `a_pointer_moved_after_its_address_was_kept_is_not_believed_after_a_call`
  loses its `SC0413`; never believed: `a_pointer_handed_by_address_twice_builds`
  and the other cases that grow a pointer by address are reported.
* a pointer read out of a local believed whatever the local holds:
  `an_int_pointer_read_through_an_alias_of_a_void_pointer_a_callee_filled_is_asked`
  and `a_pointer_read_out_of_its_local_before_it_is_used_is_asked_what_the_local_holds`
  go silent. What leaves asked at its own type only:
  `a_void_pointer_s_address_handed_as_an_int_pointer_to_pointer_is_asked_at_the_wider_type`
  goes silent; memory not asked:
  `memory_holding_a_char_s_address_handed_as_an_int_pointer_to_pointer_is_asked_what_it_holds`
  does (#627).
* review's findings, each with a case in `tests/cases/bounds/` and the
  mutation beside it in `tests/cases.rs`, each applied and seen to fail its
  case: the library reach keyed with the dereferences
  (`a_proof_at_a_call_is_not_displaced_by_a_doubt_about_its_argument`), the
  offset left out of what a pointer holds
  (`a_pointer_moved_off_null_and_tested_is_not_proved_inside`,
  `a_local_s_address_moved_past_it_and_tested_is_not_proved_inside`), a
  constant read as written (`an_allocation_s_size_is_what_its_parameter_receives`,
  `a_length_is_what_its_parameter_receives`), a row of `LIBRARY` dropped or
  narrowed (`a_memmove_wider_than_its_destination`,
  `a_memcmp_longer_than_both_objects`, `a_strlen_is_not_shown_to_fit`,
  `a_strncpy_of_its_destination_s_size_is_unknown_only_at_its_source`,
  `an_int_written_into_one_aligned_byte`), and a belief answered without its
  record (`a_char_address_stored_as_void_and_read_through_an_int_alias`,
  `a_pointer_to_a_char_copied_into_memory_by_memcpy_and_read_back`).

**Held by nothing, and each says why.** `Held::inner_address` and the
`Unknown` an `Index` in a place gets: no C reaches either, since the lowering
folds `&*p` and writes `p[i]` as `*(p + i)`, and #621 is where the first
`Index` is built and has to be guarded.

### Consequences

* Good, because the three programs of #622 are reported, and `malloc(1)`
  written as an `int` is a proof.
* Good, because an access the check cannot read is `Unknown` rather than
  silence, so #621's arrays and pointer arithmetic land under Phase 11's rule.
* Bad, because **a pointer that is lost, foreign or escaped is doubted about
  its size as well as about whether it was freed**, so most corpus cases that
  already reported such a doubt gained an `SC0412` beside it, and the output-parameter
  idiom, `get(&p); if (p) *p = 1;`, is `Unknown`. The escape is doubted because
  something may have written the local where this check could not see.
* Bad, because the record a load is bounded by is coarser than the program:
  a store through one load read back through another,
  `an_allocation_stored_through_a_load_and_freed_is_doubted_when_read_back_one_level_further`,
  gains a false `Unknown`.
* Bad, because every access off a pointer's start is `Unknown`, so `p[1]` does
  not build at `--safety memory` without `--allow-unknown`. That is what Phase
  11's rule allows until a distance is carried.
* Bad, because `strcpy` and its family are always `Unknown` at their
  destination, and a list walked through `void *` is `Unknown` at every step.
* Bad, because a `void *` store is asked its start and no size, so size
  through a `void *` is seen only where the record or a local holds what was
  stored (#627).
* Bad, because every pointer handed on off its start is `Unknown`, so a
  pointer one past the end handed to a call that only compares with it is
  refused, and corpus cases that already reported a doubt about such a
  pointer gained an `SC0413` beside it.
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
* Bad, because a pointer one past the end that is handed on is a false
  `Unknown`.

### Doubt every load

* Bad, because `**pp` and every walk of a list become a false `Unknown`, which
  a reader sees, and which is the commonest shape of pointer-holding data.

### Believe nothing

* Bad, because `void f(int *p) { *p = 1; }` would not build at `--safety
  memory`, and neither would nearly the whole corpus.

### Ask a `void *` its start and no width

* Good, because every way a pointer enters memory through a projection, a
  call or `memcpy` is then asked about its start: a typed store at its type,
  a `void *` store at zero width, a byte at one, and `memcpy`'s source
  through the pointers it reaches. An assignment to a local whose address was
  taken and has not left the function is not one of those. One existing
  corpus case changed, the one that asserted the old rule.
* Bad, because a pointer off its start passed to a `void *` parameter,
  `fwrite(buf + off, ...)` among them, is a false `Unknown`.
* Bad, because size does not survive the store; #627 asks it where the
  pointer leaves and where it is read out of a local instead.

### Follow the narrowest width stored into an allocation

* Bad, because it adds a field to the lattice and still rests on the belief
  in a callee's store across a call, which is where the open case lives.

### Follow the stored type per slot

* Bad, because the lattice does not tell an allocation's slots apart, so it
  is a redesign and wants an experiment first.

## More Information

* `crates/safec-ir/src/memory/parts.rs`: `Extent`, `Held::believed`,
  `Held::inner_address`.
* `crates/safec-ir/src/memory/report.rs`: `fits`, `may_not_fit`,
  `reached_by_library`.
* `crates/safec-ir/src/memory/transfer.rs`: `made_extent`, `pointee_size`, the
  extent where a parameter and a call's destination are made, and its join.
* `crates/safec-ir/src/target.rs`: `Target::pointer_bits`.
* Issue #622 and its design comment; #623 for the departures.
