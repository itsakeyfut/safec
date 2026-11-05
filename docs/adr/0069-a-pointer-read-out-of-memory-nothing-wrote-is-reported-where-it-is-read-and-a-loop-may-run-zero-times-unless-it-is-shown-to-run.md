---
status: "accepted"
date: 2026-11-06
decision-makers: itsakeyfut
---

# A pointer read out of memory nothing wrote is reported where it is read, and a loop may run zero times unless it is shown to run

## Context and Problem Statement

[ADR-0057](./0057-a-pointer-read-out-of-fresh-memory-is-reported-where-it-is-read.md) and [ADR-0064](./0064-a-loop-may-run-zero-times-unless-its-test-is-shown-true-on-the-way-in.md) each grew a paragraph per route found, and the first was partly replaced by the second. This record restates both in the shape the code has now and supersedes them whole. No behaviour changes. The history, the measurements and the routes stay in those records, in the doc comments the rules below name, and in the `Belief::Fresh` rows of `tests/routes.rs` (ADR-0062).

The question: when is a read of a pointer out of memory nothing wrote reported, as what, and what counts as a write, a path and a turn of a loop on the way to deciding it.

## Decision Drivers

* Silence is the conclusion this compiler must not give wrongly. Annex J.2 lists using the value of an object `malloc` made, or of a local whose initial value is indeterminate, as undefined, whatever it is used for.
* A path no test excludes is a path, and a loop's path that skips its body is one.
* One defect, one report.

## Considered Options

* Report the read itself, or report each later use that reaches memory (ADR-0057 took the first).
* At a loop's back edge, create no doubt; create one always; or create one unless the loop is shown to run (ADR-0064 took the third).

## Decision Outcome

1. **The read is reported, whatever it is read for.** `report::loaded_indeterminate` asks every element and terminator that reads a place through a dereference as a value, `read_out_of_memory_after`, where the value is a pointer by its type: an integer or a byte is not followed. A write through `*l` is not a read of it. It is `Unsafe` `SC0411` where `Allocations::reads_fresh` holds, one dereference through a pointer whose every site and local is unwritten on every path, and `Unknown` where `Allocations::reads_partly` holds, any place the read may reach being unwritten on some path. A pointer itself read out of memory, and a read below the first dereference, are asked only by `reads_partly`, since what they show is a lower bound on what they may name.
2. **What nothing wrote is one value per place.** `Known::init` holds an `Init`, `Unwritten`, `Maybe` or `Written`, per allocation and per local, one kind of place (ADR-0063). An allocation starts `Unwritten` where its library row says `Leaves::Indeterminate`, or `Leaves::TheOld` handed a pointer that is certainly null. A local that may hold a pointer starts `Unwritten` at the function's entry, never a parameter (`Allocations::unwritten_at_entry`), and again at its `StorageLive`. A local array of at most `ELEMENTS_KEPT` innermost elements keeps one per element in `Known::element_init`. A local read by name is asked here, by `report::unwritten_read`, where its address is taken, and by `memory::assigned` where it is not; `assigned::address_taken` is the one answer to which.
3. **A write makes a place `Written` only where it certainly lands there.** `Allocations::initialised_by` holds what a store and a library copy do, one arm each. The one place `Landing::certain` names is written; every other place the write may reach takes `Init::may_write`, which ends a proof and never makes a doubt `Written`. A call this check cannot read writes only the locals it may reach, `Known::exposed`, as the `may_write` answer of `Known::replaced`; a store or copy this check cannot place writes only `Known::locals_a_write_may_land_in`, which for a pointer read out of memory or written by a call adds what such code could name when that pointer was fixed, `Held::named_at_read`. A library copy also carries what it copies: bytes nothing wrote in its source leave its destination unwritten or in doubt. The two errors are not alike: a write missed is an `SC0411` the reader sees, and a may-write taken as a write is a silence, so every door takes the first.
4. **One defect is reported once.** A later dereference, `free` or `realloc` of the value is not reported as `SC0411` again. `Held::fresh_read` carries the read to its copies and decides no report: it drops only a doubt raised at a `free` or `realloc` of the local it is handed, `report::freed_after_its_read_was_reported`, and only where `Known::holds_fresh_read` says no code this check cannot read may have replaced the value since. Any store writes its slot, a store of an indeterminate pointer included. A free leaves a place's `Init` as it was, since a `realloc` that fails leaves the old block live and unwritten, so a block read after it was freed is `SC0411` beside `SC0402`; the report is keyed by the pointer read through rather than by `*l` so that neither replaces the other.
5. **Where paths meet, a place unwritten on one and written on the other is `Maybe`** (`Init::joined`). At a loop's back edge the same holds unless `Allocations::runs_once` is set for its head, which `memory::runs::shown_to_run` decides: there the two meet as `Written`, since the path that skips the body is one no execution takes. And whatever the loop, a place written on the way in and unwritten again along the back edge is `Maybe` (`Init::unwritten_again`, applied by the memory check's `join_back_beside`).
6. **A loop is shown to run only in one narrow shape**, since a wrong proof is a silence: one edge into the head from outside, one latch, no edge leaving the body for anything the head's test leaves for, and the head's test true on the way in where every value it reads is a constant from zero to `INT_MAX`, or an `int` local whose address is never taken, written at most once by the head and otherwise given such a constant on a straight line of blocks before the loop. `memory/runs.rs` gives the reason for each clause.
7. **A read inside a loop is also asked as of that loop's first turn**, whatever turn reaches it: `dataflow::solve_first_turn_within`, which leaves out the back edges into one head, solved per loop after the every-turn walk. It is a proof only where the read's block dominates every latch, and a doubt elsewhere in the loop and in every block outside it that a block of the body other than the head dominates. A proof takes the place of a doubt said at the same caret (`report::may_say`). One loop is solved at a time, and every first turn together is held to one more walk bound, past which the function is `Kind::TooLong`.
8. **`memory::assigned` asks a loop the same way, by name.** A read whose block dominates every latch is asked as of each loop's first turn and is a proof there; a read on an arm is not asked so, and stays silent where the memory check doubts it, by the user's decision. At a loop not shown to run, its head's edge out of the loop is asked as of the first turn, so a read after it that the body alone assigns is the `Unknown` doubt a branch edge gives.

### Confirmation

Each a case in `tests/cases.rs` unless it is named as a unit test; the comment beside each names its mutation too.

* Rule 1: `a_pointer_read_out_of_fresh_memory_and_only_compared_is_reported` builds when the every-turn walk asks `loaded_indeterminate` nothing for an element.
* Rule 2: `a_local_whose_address_is_taken_read_by_name_before_anything_wrote_it_is_reported` goes silent when `unwritten_read` reports nothing, and `an_element_of_a_local_array_nothing_wrote_beside_one_written_is_reported` goes silent when a write of a known element writes every element.
* Rule 3: `a_store_that_may_land_in_an_unwritten_block_or_a_written_one_leaves_the_first_in_doubt` goes silent when `Init::may_write` writes `Written`; `a_pointer_read_out_of_a_local_array_an_unrelated_call_did_not_reach_is_reported` goes silent when a call this check cannot read writes every escaped local; `a_pointer_read_out_of_memory_a_call_wrote_after_a_local_array_left_may_write_it` gains `SC0411` when a write ignores `named_at_read`.
* Rule 4: `a_pointer_read_out_of_fresh_memory_and_freed_is_reported_once` gains a doubt when the suppression is dropped; `a_pointer_read_out_of_fresh_memory_a_call_may_have_replaced_is_still_doubted_where_it_is_freed` loses its doubt when `holds_fresh_read` drops its escape clause; `a_pointer_read_out_of_fresh_memory_stored_and_read_back_is_reported_once` gains reports when a certain store leaves its block unwritten; `a_pointer_read_out_of_a_freed_table_and_returned` loses `SC0402` when the report is keyed by `*l`.
* Rule 5: the unit test `a_forward_join_of_an_unwritten_and_a_written_object_may_be_unwritten` fails when a forward join gives `Written`; `a_pointer_read_after_a_loop_that_may_run_zero_times_is_doubted` goes silent when `runs_once` is true at every head, and `a_local_array_filled_by_a_loop_is_not_doubted` gains `SC0411` when it is never true; the unit test `a_place_a_loop_makes_unwritten_again_is_in_doubt_after_its_back_edge` fails when `unwritten_again` answers `Maybe` whatever was on the way in.
* Rule 6: `a_pointer_read_after_a_loop_tested_on_a_char_is_doubted` goes silent when a `char` local is taken to hold its constant; `pointers_read_after_loops_their_constants_compare_false_at_the_bound_are_doubted` goes silent in one loop when `Lt` gains its equality; the unit tests `a_loop_with_two_latches_is_not_shown_to_run` and `a_loop_left_from_its_body_for_where_its_test_leaves_is_not_shown_to_run` fail when the latch count or the early exit is not asked.
* Rule 7: `a_pointer_read_out_of_a_local_array_in_a_loop_before_the_body_writes_it_is_reported` goes silent when no loop is asked as of its first turn; `a_pointer_read_on_an_arm_of_a_loop_before_the_body_writes_it_is_doubted` becomes certain when every first-turn read is a proof; `a_pointer_read_on_an_arm_that_leaves_a_loop_that_runs_before_its_write_is_doubted` goes silent when no block outside the loop is asked; `a_pointer_local_doubted_every_turn_is_proved_as_of_the_first` stays a doubt when a proof does not replace one; the unit test `a_function_whose_first_turns_pass_one_more_bound_is_one_unknown_finding` fails when a first turn past the bound is left unreported.
* Rule 8: `a_pointer_local_read_in_a_loop_before_the_body_assigns_it_is_reported` goes silent when `memory::assigned` asks no read as of a first turn; `a_pointer_local_read_after_a_loop_that_may_run_zero_times_is_doubted` goes silent when no exit edge is asked, and `a_pointer_local_assigned_by_a_loop_its_constants_run_is_not_doubted` gains a doubt when a loop shown to run is asked too.

Held by nothing, each said where it lives: the `inner_address` clause of `reads_fresh`, and `lost` and `foreign` among the marks it refuses; `several` in a library copy's `Whole` arm; the doubt `Known::array_writes_after` keeps; `join_back_beside` for a local and an element; the walk back in `runs` stopping at a call whose result lands in the counter; and every latch rather than any in rule 7. The rule that a store of an indeterminate pointer writes its slot has no mutation narrower than leaving every certain store's block unwritten.

### Consequences

* Good, because the first use J.2 lists is the one reported, so no later use is silent for want of being listed.
* Good, because a loop this check cannot show runs is read as one that may not, and loops counted by constants keep their answer.
* Bad, because a program that reads an unwritten slot and discards the value is refused, which the normative text makes undefined only through a trap representation (C17 6.2.6.1 p5); J.2 is the reading taken.
* Bad, because a read on an arm of a loop is a doubt where C defines it reading last turn's value, and a loop C runs whose test this cannot evaluate is doubted after it; ADR-0064 measured twelve corpus cases gaining that doubt.
* Bad, because per-element records and `named_at_read` cost memory, which `memory::per_block` counts; ADR-0057 measured both.
* What would reverse this: a reading of C17 under which reading an indeterminate pointer is defined, or a check that follows integer values far enough to show a loop runs wherever C makes it run.

## Pros and Cons of the Options

The options and their measurements are ADR-0057's and ADR-0064's, which keep them. In short: a report at a later use is silent at every use the list misses; a back edge that creates no doubt is silent after a loop that runs zero times; one that always does refuses every loop counted by constants that fills a local.

## More Information

* Supersedes [ADR-0057](./0057-a-pointer-read-out-of-fresh-memory-is-reported-where-it-is-read.md) and [ADR-0064](./0064-a-loop-may-run-zero-times-unless-its-test-is-shown-true-on-the-way-in.md).
* `crates/safec-ir/src/memory/init.rs`, what a write leaves and whether a read is of memory nothing wrote; `memory/parts.rs`, `Init` and `Inits`; `memory/known.rs`, `Known::init` and `Known::element_init`; `memory/runs.rs`, the proof a loop runs; `memory/assigned.rs`, a local by name; the first-turn pass in `crates/safec-ir/src/memory.rs`.
* [`docs/safety-model.md`](../safety-model.md), where an indeterminate pointer is level 1's, and [`docs/diagnostics.md`](../diagnostics.md), whose `SC0411` paragraph describes this rule.
