---
status: "accepted"
date: 2026-11-06
decision-makers: itsakeyfut
---

# An access is proved inside its object only where every object it may reach was read wide enough

## Context and Problem Statement

[ADR-0060](./0060-an-access-is-proved-inside-its-object-only-where-its-size-was-read.md)
decided two things: whether an access lands inside the object it reaches
(`SC0412`), and what a pointer is asked where it leaves (`SC0413`). Its access
half grew one paragraph per route over many edits, and its code has since moved
into `memory/fit.rs` (ADR-0063). This record restates that half in the shape the
code has now, and changes no behaviour. The doors are ADR-0068's, and what a
pointer read out of memory is believed to point at is ADR-0067's; `fit::fits`
answers for both, and they say when they ask it.

The question: given an access of a known width through a pointer, when is it
proved inside, when proved outside, and when neither?

## Decision Drivers

* **Read or `Unknown`, and nothing answered safe without being read**, which is
  Phase 11's rule in [`docs/roadmap.md`](../roadmap.md).
* **A proof where C gives one**: `malloc(1)` written as an `int` lands outside on
  every execution.
* **A may-set proves nothing from one member.**

## Considered Options

* Prove a fit only from sizes read; prove outside only where every size is exact
  and too small; otherwise `Unknown`.
* Prove a fit where any object the pointer may reach is wide enough.
* Carry a distance and a length per allocation.

## Decision Outcome

Chosen option: the first, because the second says safe about the other member,
and the third is a lattice this check does not have (ADR-0036 carries no
distance).

1. **An object's size is read where it is made**, into `Known::extent` as an
   `Extent`. `Allocations::made_extent` gives `Exactly(n)` for an allocation
   asked for with constants, as the `makes` field of its row in
   `crate::library::LIBRARY` says (`calloc` as the product), and `AtLeast(0)`
   for a size it cannot read. A constant is read as its parameter receives it,
   converted first (`Allocations::constant_argument`, C17 6.5.2.2 p7). A
   parameter is `AtLeast` the size its type points to, set where the function
   starts, and so is what a call this check cannot read returns; a `void *` is
   `AtLeast(0)`. A local whose address a pointer holds is its own size,
   exactly. `Extent::joined` only descends.
2. **`fit::fits` answers in a fixed order**, and its doc comment lists it: a
   pointer holding nothing at its start is `None`, the nullability check's;
   where `doubt_unseen_writes`, one that is lost, foreign or an escaped local is
   `Unknown`; a call's write bounded by `Known::handed_at_most` narrower than
   the access is `Unknown`; a belief (`Held::believed`) proves a fit only where
   every object the pointer is recorded to name is wide enough too; a load
   believing nothing, or a pointer off its start or perhaps off it, is
   `Unknown`; an edge that may be inside a local (`Held::inner_address`) is
   `Unknown`.
3. **Otherwise every object is folded**, by `fit::folded`: every lower bound at
   least the width proves a fit; every one `Exactly` and narrower is
   `Fit::Outside`, naming a size only where every object agrees; anything
   between is `Unknown`.
4. **An access is asked at every level of its place**, by
   `report::may_not_fit`. The first level asks the place's own local and doubts
   unseen writes. Each deeper level asks the memory it goes through by
   `Allocations::believed_read`, and each local the pointer may have been read
   from (`fit::read_from_locals`) without doubting its escape; a proof about
   one such local is `Unknown` unless the pointer is certainly that local's
   value (`fit::only_if_certain`). A place with an `Index` is `Unknown`. One
   local asked twice at one caret is said once, and a proof stands over a
   doubt (`say_fit`).
5. **A library call is an access as far as it reaches**, by
   `report::reached_by_library` over each row's `reaches`: `Reach::Length` an
   access of that many bytes where the length is a constant and `Unknown`
   otherwise, `Reach::String` `Unknown` wherever the pointer holds anything. A
   pointer read out of memory and handed straight over asks the locals it may
   have been read from (`fit::through_locals`).
6. **`Fit::Outside` is `Unsafe`**, so an error that `--allow-unknown` leaves an
   error; `Fit::Unknown` is `Unknown` with `Unproven::Extent`. Both are
   `Kind::OutsideItsObject`.
7. **A pointer's own width is the target's**, `Target::pointer_bits`, measured
   per row with `clang`, which amended ADR-0013.

### Confirmation

Each mutation applied on its own, the workspace run, and the file restored; the
test named fails.

1. `made_extent` leaving the argument unread: `an_int_written_into_one_byte`.
   Reading only `m` of `calloc`: `a_calloc_of_two_constants_is_their_product`.
   A parameter's extent `AtLeast(0)`: `a_parameter_written_through_is_inside`.
   An opaque result's extent `AtLeast(0)`: `a_call_s_result_is_believed_whole`.
   `Extent::joined` keeping the larger bound:
   `two_sizes_for_one_object_meet_as_a_bound_below_both` in
   `crates/safec-ir/src/memory/parts.rs`. A local's address folded as
   `AtLeast(0)`: `a_char_address_converted_through_void_and_written_as_int`.
   A constant read as written:
   `an_allocation_s_size_is_what_its_parameter_receives`.
2. The offset left out of `holds_something`:
   `a_pointer_moved_off_null_and_tested_is_not_proved_inside`. The unseen
   writes never doubted:
   `a_pointer_a_call_wrote_through_an_address_is_not_doubted_freed_with_no_later_call`.
   The `handed_at_most` question dropped:
   `a_local_a_callee_may_have_written_beside_a_whole_object_is_believed_no_wider_than_it_was_handed`.
   A belief answered without its record:
   `a_belief_is_no_wider_than_what_the_pointer_is_recorded_to_name` in
   `crates/safec-ir/src/memory/fit.rs`. An offset not doubted:
   `a_write_through_a_pointer_moved_past_its_allocation_after_a_null_test`.
   The `inner_address` test dropped:
   `an_edge_inside_a_local_is_not_read_at_the_local_s_size` in the same file.
3. A fit proved where any object is wide enough:
   `two_allocations_of_different_sizes_are_neither_proved_inside_nor_outside`.
   A size named where the objects disagree:
   `an_access_wider_than_two_allocations_of_different_sizes_names_neither`.
4. Only the first level asked:
   `a_pointer_moved_after_its_local_s_address_was_taken_is_asked_through_it`.
   The memory a deeper level goes through not asked:
   `a_write_two_levels_through_a_void_parameter_is_asked_of_the_memory`.
   `only_if_certain` returning its answer unchanged:
   `a_pointer_that_may_be_either_of_two_locals_proves_nothing_from_one`. The
   first answer kept in `say_fit`:
   `a_proof_through_a_local_s_address_stands_beside_a_doubt_about_the_local_itself`.
5. A length reach answered inside: `a_memset_wider_than_its_allocation`. A
   string reach asked nothing: `a_strlen_is_not_shown_to_fit`.
6. `Fit::Outside` said as `Unknown`:
   `an_int_written_into_one_byte_is_an_error_where_unknown_is_allowed` builds.
7. `x86_64-pc-windows-msvc` given 32 pointer bits:
   `a_pointer_written_into_four_bytes_is_outside_where_a_pointer_takes_eight`.

**Held by nothing**: a place with an `Index` answered `Unknown`, since no C
reaches one (the lowering writes `p[i]` as `*(p + i)`), and the walk in
`may_not_fit` stopping at a `void` level.

### Consequences

* Good, because `malloc(1)` written as an `int` is a proof, and an access this
  check cannot read is `Unknown` rather than silence.
* Bad, because every access off a pointer's start is `Unknown`, so `p[1]` needs
  `--allow-unknown`, which ADR-0060 records as what no distance costs.
* Bad, because a lost, foreign or escaped pointer is doubted about its size as
  well, so the output-parameter idiom `get(&p); if (p) *p = 1;` is `Unknown`,
  as ADR-0060 measured on the corpus.
* Bad, because `strcpy` and its family are always `Unknown` at their
  destination.
* What would reverse this: a lattice that carries a distance and a length per
  allocation, making `Extent` and `Offset` one fact; or annotations that say how
  many objects a parameter points at.

## Pros and Cons of the Options

* Any object wide enough: bad, because an access that fits one of two
  allocations may overflow the other, and this check would say safe and be
  believed.
* A distance and a length: good, because `p[1]` would be read; bad, because it
  is a redesign of `Held` and `Extent` rather than a rule.

## More Information

* Supersedes the access half of
  [ADR-0060](./0060-an-access-is-proved-inside-its-object-only-where-its-size-was-read.md),
  whose history and measurements stay there.
* Code: `memory/fit.rs`, `memory/report.rs` (`may_not_fit`, `say_fit`,
  `reached_by_library`), `memory/parts.rs` (`Extent`), `memory/transfer.rs`
  (`made_extent`, `constant_argument`, the parameter's extent at entry), all
  under `crates/safec-ir/src/`, with `library.rs` and `target.rs` beside them.
* Per-route detail: `tests/cases.rs` under `"bounds"`, and the width routes in
  `tests/routes.rs` (ADR-0062).
