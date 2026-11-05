---
status: "accepted"
date: 2026-11-06
decision-makers: itsakeyfut
---

# A pointer read out of memory holds what was recorded there, proves nothing with it, and is believed no wider than was recorded

## Context and Problem Statement

[ADR-0040](./0040-a-pointer-read-out-of-memory-reaches-what-was-stored-and-a-parameter-is-exposed-where-the-function-starts.md)'s
half on what a load reaches,
[ADR-0045](./0045-a-pointer-read-out-of-memory-holds-what-was-stored-there-and-proves-nothing-with-it.md)
and [ADR-0061](./0061-a-pointer-read-out-of-memory-is-believed-only-as-wide-as-was-recorded.md)
each answered part of one question, one route at a time, and read as logs of
those routes. This record restates them as the memory check holds them after
the rebuild of ADR-0063, and changes no behaviour. The question: what does a
pointer read out of memory hold, what may a report conclude from it, and how
wide is it believed to point?

ADR-0040's other half, what a parameter's memory and a callee's hold
(`Held::from_caller`), is ADR-0066's. What is asked of a load where it leaves,
`Known::handed_below` among it, is ADR-0068's.

## Decision Drivers

* Saying safe wrongly is the worst answer, and a report the reader can see is
  far better ([`docs/safety-model.md`](../safety-model.md)).
* What a cell records is a may-set kept per allocation, not per slot, and a
  store this check cannot place is exposed rather than recorded: a lower bound
  (ADR-0045). A proof from it is one this check is not entitled to.
* A route that reaches memory without recording a width has read none (ADR-0061).

## Considered Options

* **One record stating the rules as the code holds them now.**
* Leave the three records as they are.

## Decision Outcome

Chosen option: **one record**, because the code has one rule for a load and
three records describe it in names that no longer exist.

1. **A load holds what memory records, and is marked as a load.** A place with
   a dereference that may be a pointer (`Allocations::may_be_pointer`), or is a
   byte (ADR-0046), sets `Held::loaded`, in `Allocations::read_through` where it
   is assigned and in `built::built_from` where it is an operand. Both take what
   it holds from `Known::load_into`: the sites `Known::levels_below` finds, at
   `Offset::Unknown`, and the locals as address edges. The walk steps one level
   per dereference through a cell's names and a local's own value alike
   (`Known::level_below`), and adds what is reachable to code this check cannot
   read (`Known::reachable_now`) at every level below the first, and at the
   first where the local is itself `loaded` or `foreign`
   (`Known::exposed_through`). A lost local is followed no further, except into
   that memory (`Known::level_zero`).
2. **It proves nothing.** `Known::reached_by` and `Known::reached_below` put
   `Reached::Partial` beside a load's sites, and `verdict` settles nothing with
   it present: freed or unknown sites are a doubt, live ones say nothing. A
   free of a load answers `LostReason::ReadOutOfMemory` in
   `Allocations::touching`, and `holds_something_unnameable` keeps its transfer
   ADR-0029's, so the sites become unknown rather than freed.
3. **A call reaches what was stored.** `Allocations::read_out`: an argument read
   through one `Deref` of a local holding sites, with nothing
   `Beyond::unrecorded_address` names, reaches what those cells hold; any other
   load, or a local that is `lost`, `loaded` or `foreign`, reaches
   `Known::stored`. A local merely holding no site reaches nothing.
   `Known::reach_of` asks the same of an escaped local.
4. **A load takes the marks of every cell it reads through.**
   `Cell::taken_by_a_load`, for each site of `Known::cells_on_the_way`: a cell's
   `stale_read` makes the value `lost` and `stale_read`, and a cell's `lost`
   makes it `lost_read`, made lost only when a lost value may have gone
   (`Known::lost_may_be_gone`). A walk through a local this check lost does the
   same (`Known::lost_through`). At a loop's rebirth `Known::reborn` keeps the
   site's own cell (`Cell::kept_when_reborn`) and, only where the old allocation
   is not live, marks every cell and every other local holding the site
   `stale_read`.
5. **A load is believed no wider than was recorded, and nothing where nothing
   was.** `Allocations::believed_read` is the type read as, bounded by level 0 of
   `Known::memory_levels`, and `0` where that is unbounded; as its type at no
   dereference or where the read is reported indeterminate (ADR-0069). The store
   arm of `Allocations::element` records `Held::recorded_width` (the least
   extent named; `0` off its start or for an unrecorded address, a lost value
   included; nothing for a null), a load's own `believed`, or what a call wrote
   at what it was handed as, else `AS_TYPED`. `memory_levels` reads a local on
   the way by the same three arms, forgets level 0 where a cell is
   `Cell::unrecorded`, and `Allocations::handed_site` bounds a cell a call was
   handed as it was handed.
6. **A lost value handed to a call this check cannot read is handed to every
   later one.** `Known::lost_exposed` is set where such a call is handed a
   value this check lost, a union at a join and never cleared, and what any
   later such call returns is lost, since the call may hand that value back.

### Confirmation

Each mutation is in `crates/safec-ir/src/memory/`, applied alone with the file
restored after; the test named is in `tests/cases.rs`.

1. `load_into` holding no site fails
   `a_use_after_free_through_a_pointer_read_out_of_memory_is_reported`;
   `read_through` setting `loaded` for a byte only fails
   `a_table_read_out_of_a_holder_into_a_local_is_reached_through_it`;
   `level_below` ignoring a local's edges fails
   `a_load_through_two_locals_addresses_reads_what_the_last_holds`;
   `exposed_through` asking `foreign` only fails
   `an_allocation_stored_through_a_load_and_freed_is_doubted_when_read_back`;
   `levels_below` walking one level fails
   `a_use_after_free_through_a_chain_written_as_one_place_is_reported`.
2. `reached_by` pushing no `Partial` for a load fails
   `a_use_after_free_through_a_pointer_read_out_of_memory_is_reported`;
   `verdict` counting `Partial` as a doubt fails
   `a_live_pointer_read_out_of_memory_is_read_in_silence`; `touching` ignoring
   `loaded` fails `a_free_of_a_pointer_read_out_of_memory_stays_a_doubt`;
   `holds_something_unnameable` asking only for a local's address fails
   `a_free_of_a_pointer_read_out_of_memory_proves_nothing_about_what_it_holds`.
3. `read_out` answering nothing for one `Deref` fails
   `what_a_callee_frees_through_a_pointer_read_out_of_a_table_is_unproven_after_it`.
4. `taken_by_a_load` ignoring `stale_read` fails
   `a_pointer_stored_and_freed_last_turn_is_doubted_after_the_loop_allocates_again`;
   giving `lost` as `lost` fails
   `a_list_built_in_a_loop_with_nothing_freed_is_walked_without_a_doubt`;
   `reborn` marking whatever the state fails
   `a_pointer_kept_alive_last_turn_is_not_reported_freed_after_the_loop_allocates_again`;
   `reborn` clearing the site's cell fails
   `a_table_allocated_again_by_a_loop_still_holds_what_it_held`.
5. `believed_read` believing the type where nothing is recorded fails
   `a_pointer_read_out_of_memory_calloc_made_is_not_reported_as_indeterminate`;
   asking a fresh read its width fails
   `a_byte_copied_out_of_a_local_is_doubted_where_read_back_as_a_pointer`; the
   store arm recording no width fails
   `a_copy_from_a_source_read_out_of_memory_is_recorded`; `memory_levels` giving
   a local's value no width fails
   `a_live_pointer_written_into_a_local_through_its_address_is_read_back_in_silence`,
   and answering `0` for `AS_TYPED` fails
   `a_live_pointer_through_a_locals_address_stored_in_an_allocation_is_read_in_silence`;
   not forgetting level 0 fails
   `a_pointer_stored_on_one_arm_beside_an_int_on_the_other_is_doubted_how_wide`;
   `recorded_width` ignoring an offset fails
   `a_pointer_off_its_start_stored_as_void_and_read_through_an_int_alias`;
   `handed_site` recording no bound fails
   `a_pointer_read_out_of_memory_a_call_was_handed_is_not_reported_as_indeterminate`.
6. A later call's result not made lost fails
   `a_lost_pointer_handed_to_one_call_and_returned_by_another_is_doubted_after_its_free`;
   `lost_exposed` joined as an intersection fails
   `a_lost_pointer_handed_to_a_call_on_one_arm_and_returned_by_another_is_doubted_after_its_free`.

Per-route detail is the route matrix in `tests/routes.rs` (ADR-0062) and the
doc comments of the functions named above.

### Consequences

* Good, because a use after free through a pointer this function stored and read
  back is reported, through any chain of loads, locals or memory a callee can
  reach, and never proved from a set that may be missing members.
* Bad, because slots are not told apart and a record is never replaced: a live
  slot beside a freed one is doubted (ADR-0045 measured the cases), and a route
  that forgets a width after another store recorded one is still believed by
  that record (ADR-0061 measured 33).
* Bad, because a loaded pointer handed to a call exposes every stored pointer
  when it is not read through one `Deref` of a named local (ADR-0040).
* Bad, because memory nothing recorded a width in is believed nothing:
  `calloc`'s nulls and a lost value read back are doubted (ADR-0061), and so is
  `*ib = 7; *box = malloc(4);` read back through `*box` (`Cell::unrecorded`).
* What would reverse this: slots told apart and every store recorded, so that a
  load names exactly what was stored where it read.

## More Information

* Superseded: ADR-0040 (its half on what a load reaches), ADR-0045, ADR-0061.
* Related: ADR-0046 (a byte is a load), ADR-0070 (an access) and ADR-0068 (a departure),
  ADR-0062 (the route matrix), ADR-0063 (one cell type).
* Code: `crates/safec-ir/src/memory/known.rs` (`load_into`, `levels_below`,
  `level_below`, `reached_by`, `memory_levels`, `reborn`),
  `memory/transfer.rs` (`read_through`, `read_out`, `believed_read`,
  `touching`), `memory/parts.rs` (`Cell`, `Held::loaded`,
  `Held::recorded_width`, `AS_TYPED`), `memory/built.rs` (`built_from`),
  `memory/report.rs` (`verdict`).
