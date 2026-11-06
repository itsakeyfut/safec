---
status: "accepted"
date: 2026-11-06
decision-makers: itsakeyfut
---

# A pointer is asked where it leaves, by a call, a return or a store, what the other side will believe of it

## Context and Problem Statement

Nothing here reads a callee from its caller or a caller from its callee (the
module comment of `crates/safec-ir/src/memory.rs`), so a pointer that leaves
the function is believed on the other side: a callee believes its parameter
live and whole, a caller the result, and a later load what was stored. The
door is the last point this check can say anything about it. ADR-0041 (the
return and the store), ADR-0042 (the call), ADR-0060's half on the doors
(`Departure`, `SC0413`) and ADR-0065 (a store nothing outside reads) each
answer part of that, and grew a paragraph per route found. This record
restates them in the shape the code has now and supersedes them; ADR-0060's
half on an access proved inside its object is ADR-0070's. No behaviour
changes.

## Decision Drivers

* A program that builds with a use after free or a narrow pointer handed on is
  the worst answer this compiler gives (`docs/safety-model.md`).
* One question asked one way at every door, so that a route does not answer
  differently by the door it leaves through.

## Considered Options

* Restate the four records as one, stating the rules as the code holds them.
* Keep them, and add a paragraph per route as before.

## Decision Outcome

Chosen option: the first. The rules, each with the code that holds it:

1. **Three doors, asked before the transfer.** `memory::findings` asks each
   element and terminator what leaves as it stands there: `report::returned`,
   `report::stored_away` and `report::departed_in_element` at an element,
   `report::departed_at_call`, `report::copied_away`, `report::handed` and
   `report::handed_stored_out` at a call. Every reader of a store takes one
   `store::Store`, and `Store::leaves` is the one answer to whether code
   outside may read what it writes.
2. **Whether it is live is asked as a dereference asks it**, by `verdict` over
   `Known::handed_reached`, of a local of pointer type or a place of
   dereferences: `Kind::ReturnAfterFree` (`SC0406`) at the write into the
   return place, only where the function returns a pointer;
   `Kind::StoreAfterFree` (`SC0414`) at a store that leaves; and
   `Kind::ArgumentAfterFree` (`SC0407`) at a call, for the arguments
   `allocations::handed_places` names by a `match` over `Callee` (none of
   `Frees`, `Allocates` or `Ends`, all but `realloc`'s first). `ordered`
   answers `true` for a return and a store, and for a call only where the free
   is sequenced before it. What a call is handed is carried as a
   `Read::Argument` pending read to a later free, `realloc` or opaque call in
   the same expression (`report::used_before`), and an opaque call is asked
   beyond its arguments.
3. **A store leaves** where it is a direct write of a local
   `Known::locals_named_by_unknown_code` names, or a write through at least
   one dereference that `Known::leaves` says may land in memory or a local such
   code can name, or anywhere this check did not follow. A store into memory
   only this function reads is not asked whether it is live; its load is, and
   the door that memory later leaves by. A library copy that leaves is asked by
   `copied_away`, only of what was certainly freed and only as a doubt, in
   place of the doubt `handed` would raise about its source.
4. **What leaves holding a freed pointer behind it is doubted**, at every
   level, by `report::freed_behind` over `Known::handed_below`, at all three
   doors (`FreedBehindReturn`, `FreedBehindStore`, `FreedBehindArgument`), and
   only where the pointer itself drew nothing. Never a proof:
   `Reached::Partial` is always beside what it finds, since the receiver may
   only write there. An allocation reached only through the address handed is
   exempt, and a library copy's destination is not asked.
5. **A pointer stored out and then freed is doubted at the free.**
   `Known::stored_out` marks, per site, what a store or copy that leaves
   carries and everything it holds; a union at a join, cleared where the site
   is made again, never by a later store. A free of a marked site that is no
   double free is `FreedAfterStore`, and an opaque call that may free one,
   `report::freed_by_opaque_call`, is `HandedAfterStore`, both `SC0414`
   doubts.
6. **Whether it is whole is asked at every door that leaves**, as
   `Kind::HandedOnNarrow` (`SC0413`) with its `Departure`, always `Unknown`
   (`say_leaving`): a call at its callee's parameter types, an argument past
   them at its own type, a body defined here whatever its name, a library
   function by its reach instead, and a copy at its source's and its
   destination's type; a return; and a store that leaves. What the pointer
   lets leave with it is asked too, `report::contents_leaving`.
7. **A store nothing outside reads asks only whether its pointer may be off
   its start**, `report::at_its_start`. Its width is asked by the load that
   reads it back (ADR-0067) and by the door the memory leaves by.
8. **A `void *` leaves believing no width**, so it is asked only whether it
   starts an object (`fit::pointee_width` answers `0` for it), at every door.
9. **What a call this check cannot read is handed is believed no wider than
   it was handed as, at every level, at that call and every later one**:
   `Known::handed_at_most` bounds a local's own level and
   `Known::handed_contents_at_most` the levels below, both read back where the
   local is read and where it leaves. Handed untyped, through a `void *` or as
   bytes, it is believed nothing, and `Known::handed_untyped` keeps the mark,
   at the local or at the memory it reached, so that every later such call
   walks below it again and a pointer put there after is doubted where it
   leaves, `slots_from`.

A new `Kind` is `error[E0004]` in `verdict`'s `ordered` and in
`memory_finding` (`crates/safec-c/src/driver/words.rs`), and a new `Callee`
in `handed_places`, `departed_at_call`, `handed_stored_out` and `store_at`.

### Confirmation

Per rule, a named case in `tests/cases.rs` unless it says otherwise:

1. `a_free_after_the_write_into_the_return_place_is_not_asked_about` in
   `crates/safec-ir/tests/freed.rs` pins the boundary of asking at the write.
2. Disabling `returned` fails `a_function_that_returns_what_it_freed`, and so
   does answering `false` for it in `ordered`; dropping its pointer-type
   condition fails `an_integer_built_from_two_calls_is_not_asked_about_at_its_return`.
   Asking no argument in `handed` fails
   `a_freed_pointer_handed_to_a_function_defined_in_the_file`; answering `true`
   for a call in `ordered` fails
   `a_call_unsequenced_with_a_free_is_not_proved_to_be_handed_it`; asking every
   argument of `realloc` fails `a_freed_pointer_handed_to_realloc_is_freed_twice`.
   Not carrying `Read::Argument` fails
   `a_pointer_handed_to_a_call_the_check_meets_first_is_reported`, and asking
   an opaque call only its arguments fails
   `a_read_carried_to_an_opaque_call_that_reaches_it_by_exposure_is_reported`.
3. Disabling `stored_away` fails
   `a_freed_pointer_a_callee_stores_through_its_parameter_is_reported`; every
   store leaving fails
   `a_freed_pointer_stored_in_memory_only_this_function_reads_is_not_reported_where_it_is_stored`;
   dropping the reachable allocations from `Known::leaves` fails
   `a_freed_pointer_stored_in_memory_handed_to_a_call_is_reported`. A copy
   asked nothing fails `a_freed_pointer_copied_out_through_a_parameter_by_memcpy_is_reported`,
   and so does its doubt made the verdict's proof.
4. `freed_behind` asking nothing fails `a_box_holding_a_freed_pointer_returned_is_doubted`;
   `handed_below` stopping at the first level fails
   `a_box_of_a_box_holding_a_freed_pointer_handed_to_a_call_is_doubted`;
   dropping its `Reached::Partial` makes the first a proof; asking through the
   address handed fails `the_address_of_a_live_pointer_handed_to_a_call_is_not_asked`;
   asking a copy's destination fails
   `a_copy_into_a_block_that_held_a_freed_pointer_is_not_asked_what_it_held`.
5. The free not asking `stored_out` fails
   `an_allocation_stored_where_the_caller_reads_it_and_then_freed_is_doubted`;
   intersecting it at a join fails
   `an_allocation_stored_where_the_caller_reads_it_on_one_arm_and_then_freed_is_doubted`;
   every store marking fails `an_allocation_stored_in_a_local_and_then_freed_is_not_doubted`;
   keeping it where the site is made again fails
   `an_allocation_made_again_in_a_loop_and_freed_is_not_doubted_for_an_earlier_turns_store`;
   a copy marking nothing fails `a_pointer_copied_out_of_memory_by_memcpy_and_freed_is_doubted`;
   the call not asking it fails
   `an_allocation_stored_where_the_caller_reads_it_and_then_handed_to_a_call_is_doubted`.
6. `departed_at_call` asking nothing fails
   `a_pointer_moved_past_its_allocation_handed_to_a_call`; no return door fails
   `a_pointer_moved_off_its_start_returned`; no direct store door fails
   `a_pointer_stored_by_name_into_a_local_whose_address_was_kept_is_asked_how_wide`;
   a copy asked nothing fails `a_pointer_copied_into_memory_by_memcpy_is_asked_as_a_store`;
   a defined callee asked by its library name fails
   `a_function_defined_here_under_a_library_name_is_read_as_any_call` in
   `crates/safec-ir/tests/freed.rs`, since C defining one is refused first
   (ADR-0071); the
   locals a pointer lets leave not asked fails
   `a_pointer_in_a_local_whose_address_leaves_is_asked_at_its_own_type`; a
   narrower one proved fails `a_char_handed_to_an_int_parameter_is_shown_narrower`.
7. Every store through a pointer asked as leaving fails
   `a_narrow_pointer_stored_into_a_block_nothing_outside_reads_is_not_asked_where_it_is_stored`;
   such a store asked nothing fails `a_pointer_moved_off_its_start_stored_into_memory`.
   In `tests/routes.rs`, `every_pair_kept_apart_still_differs` holds the pairs
   `ASKED_WHERE_STORED` lists, and `a_route_answers_alike_at_depth_zero_and_one`
   the rest.
8. `pointee_width` answering nothing for `void` fails
   `a_void_pointer_off_its_start_leaving_is_asked_whether_it_starts_an_object`.
9. In `tests/routes.rs`: `handed` recording no `handed_at_most` fails
   `width_call_stored_d0_both`, and bounding no `handed_contents_at_most` fails
   `width_call_deep_d0_both`; `handed` recording no read-as for `void` fails
   `width_kept_void_d0_both`, nothing untyped below fails
   `width_kept_bytes_d0_both`, and `handed_site` recording nothing untyped fails
   `width_kept_block_d0_both`.

### Consequences

* Good, because a freed or narrow pointer is reported in the function that let
  it leave, whatever the other side does with it.
* Bad, because a return or a store doubts what it cannot prove live, with no
  free in the function, as ADR-0041 measured; and a free-and-null on one arm is
  refused at the return.
* Bad, because what memory holds only gains, so a slot freed and overwritten
  is still doubted where it leaves (#656), and a store out followed by a call
  that may keep what it is handed is doubted, as ADR-0041 measured.
* Bad, because every pointer off its start that leaves is `Unknown`, and a
  departure doubts what was only believed, as ADR-0060 measured.
* Bad, because a pointer read out of memory and returned is believed by the
  caller, the boundary ADR-0017 draws for a dereference.
* What would reverse this: summaries of the functions this unit defines, which
  would let one side read what the other believes instead of believing it.

## More Information

* Supersedes [ADR-0041](./0041-a-pointer-a-function-returns-is-asked-at-its-return-as-a-dereference-of-it-would-be.md),
  [ADR-0042](./0042-a-pointer-handed-to-a-call-is-asked-at-the-call-as-a-dereference-of-it-would-be.md),
  [ADR-0065](./0065-a-store-nothing-outside-reads-asks-only-whether-its-pointer-is-off-its-start.md),
  and the door half of [ADR-0060](./0060-an-access-is-proved-inside-its-object-only-where-its-size-was-read.md);
  their history and measurements stay there.
* `crates/safec-ir/src/memory/report.rs`, `memory/store.rs`, `memory/known.rs`
  (`leaves`, `handed_below`, `stored_out`) and `memory/allocations.rs`
  (`handed_places`); the per-route detail is in `tests/routes.rs` (ADR-0062)
  and in their doc comments.
