---
status: "accepted"
date: 2026-11-06
decision-makers: itsakeyfut
---

# What code this check cannot read may reach is exposed while it lives, and every such call may have freed any of it

## Context and Problem Statement

[ADR-0039](./0039-an-allocation-code-this-check-cannot-read-may-reach-stays-exposed-while-it-lives.md) and the half of [ADR-0040](./0040-a-pointer-read-out-of-memory-reaches-what-was-stored-and-a-parameter-is-exposed-where-the-function-starts.md) on a parameter's memory grew a paragraph per route found, and read as a log. This record restates them as the memory check holds them now, and supersedes ADR-0039 whole and that half of ADR-0040. No behaviour changes. The other half of ADR-0040, what a load reaches, is ADR-0067's.

The question: a call to a function this check cannot read, and the caller of the function being checked, are code this check does not see. What may that code reach, and what may it have done to it?

## Decision Drivers

* Saying safe wrongly is the worst answer this compiler gives ([`docs/safety-model.md`](../safety-model.md)); a false report the reader can see is far better. Every rule below fails only in the second way.
* A wider may-set must not make a report quieter, so what a call reaches is widened without widening what a report reads.

## Considered Options

* one record per question, stating the rules in the shape the code has now
* keep appending to ADR-0039 and ADR-0040

The options each rule was chosen from, and their measurements, stay in the superseded records.

## Decision Outcome

Chosen: one record per question. The rules, in [`crates/safec-ir/src/memory/`](../../crates/safec-ir/src/memory):

1. **Exposed.** `Known::exposed` marks a site once code this check cannot read may reach a pointer to it. `Known::expose` closes the mark over what every exposed allocation holds, `Known::reachable_now`, not only over the sites just marked, because a join can leave a pointer stored on one arm and its holder exposed on the other. A union at a join, and cleared only by `Known::reborn`, since the new allocation has been handed to nobody. Its per-local half is ADR-0069's.
2. **What a call reaches.** At a `Callee::Opaque` call, `Allocations::reach`: the sites its arguments name and every escaped local holds, `Known::reach_of`, and what an argument read out of memory may be, which is ADR-0067's. All of it is exposed.
3. **What it may have done.** Then `Known::unproved_exposed`: every exposed allocation still live becomes `SiteState::Reachable`, which ADR-0047 says how to read, as it says what becomes of the memory the call was handed. A freed one stays freed, since no call un-frees an allocation.
4. **What it returns.** A new site at its start, every other exposed site at an offset nobody said, and every escaped local's address, `Held::name_local`; the result is exposed itself. A site that was exposed before its rebirth is `SiteState::Unknown`, because in a loop the call may return last turn's allocation. Otherwise the result is believed to be an allocation, `SiteState::Live(None)`, with no label claiming one was made.
5. **A store.** A store this check can place records what it carries in `Known::contents` and exposes nothing until its holder is exposed, in `Allocations::land_contents`. One it cannot place, or that may also land where it cannot name, exposes at once: ADR-0044.
6. **The library, by name.** [`crate::library::LIBRARY`](../../crates/safec-ir/src/library.rs), under C17 7.1.3. `Callee::Allocates` (`malloc`, `calloc`, `aligned_alloc`) makes a fresh allocation and touches its arguments not at all. `Callee::Reallocates` leaves its first argument maybe freed rather than freed, and its result holds what the old object held, `Known::pour`. `Callee::ReturnsFirst` and `Callee::Copies` free nothing, return their first argument, and expose what they are handed, in `Allocations::initialised_by`. Every other name is `Callee::Opaque`.
7. **`realloc`'s failure branch.** `Known::realloced` remembers what a `realloc` with a non-zero constant size was handed while all of it is live. `Allocations::branch_on_reallocs_result` gives it back live on the arm where the result is null, and freed on the other, only where the tested local holds exactly that result, `Held::returned_by`. The fact goes at a call handed one of them, at `unproved_exposed`, at a rebirth, and where a join's arms disagree.
8. **A pointer parameter.** `Allocations::exposed_parameters` is every pointer-typed parameter of every function but `main`, whose arguments C17 5.1.2.2.1 p2 keeps for the whole program. Each is exposed where the function starts, in `Allocations`' `on_entry`, and its cell marked `from_caller`.
9. **The caller's memory, and a callee's.** A cell marked `from_caller` is memory a caller or callee may still hold: a parameter's from entry, and an opaque call's result once the call returns, never before, or the call would stale its own result. A pointer read out of one carries `Held::from_caller`, `Allocations::reads_caller_memory`, and a store of it marks where it lands, `Cell::absorb`. At every opaque call, and at a `free` or `realloc` that may free the caller's, `Allocations::frees_callers`, before the free's own arm, `Known::callers_memory_may_be_freed` makes each such pointer lost, each marked cell `stale_read`, and every exposed allocation unproven.

### Confirmation

Each mutation below, applied alone to the memory check, fails the test named; the cases are in `tests/cases.rs`, the route row in `tests/routes.rs`.

1. `Known::expose` closing only from the sites just marked: `a_pointer_in_a_table_exposed_on_the_other_arm_is_unproven_after_a_call`. `Known::reborn` keeping the mark: `an_allocation_made_again_at_a_site_is_not_the_one_exposed_before`, by a false report.
2. `Known::reach_of` leaving out what escaped locals hold: `a_call_handed_an_address_may_return_what_is_behind_it`.
3. `unproved_exposed` doing nothing: `an_allocation_a_parameter_holds_is_unproven_after_any_call`. Doing it to a freed site too: `a_free_proved_before_a_call_stays_proved_after_it`.
4. The result holding no other exposed site: `a_call_may_return_what_an_earlier_call_was_handed`. No escaped local's address: `a_call_handed_a_locals_address_may_return_it`. Not exposed itself: `what_a_call_returned_is_unproven_after_a_later_call`. A reborn exposed site left live: `a_call_in_a_loop_may_hand_back_what_was_freed_last_turn`.
5. A placed store exposing what it carries: `a_pointer_stored_in_the_heap_is_not_exposed_until_what_holds_it_is`.
6. The library copies exposing nothing: `what_memset_was_handed_is_unproven_after_a_later_call`. Returning nothing: `what_memset_returns_is_the_allocation_it_was_handed`. `realloc` pouring nothing: `a_table_realloc_grew_still_holds_what_it_held`.
7. The null arm restoring nothing: `a_free_on_reallocs_failure_branch_builds`. Any size read: `a_free_on_reallocs_failure_branch_with_size_zero_is_not_proved`.
8. No parameter exposed: `an_allocation_a_parameter_holds_is_unproven_after_any_call`. `main`'s exposed: `the_arguments_the_host_hands_main_are_not_exposed`.
9. A parameter's cell unmarked: `a_pointer_read_again_out_of_a_parameter_after_a_free_is_doubted`. A result's cell unmarked: `a_pointer_read_again_out_of_what_a_call_returned_after_a_free_is_doubted`. A call not making `from_caller` lost: `a_pointer_read_out_of_a_parameter_is_doubted_after_a_call`. `Cell::absorb` dropping the mark: `caller_store_d1_both`. The free's rule never firing: `a_read_through_one_parameter_after_another_is_freed_is_doubted`; firing for any free: `a_free_of_this_functions_own_allocation_leaves_a_parameter_alone`.

`Analysis::height` is held by nothing, as for every other term in it. The unit tests `a_lost_local_holding_a_parameters_site_reads_caller_memory` and `a_local_holding_what_an_opaque_call_returned_reads_callee_memory` in `memory/transfer.rs` hold the two clauses of `reads_caller_memory` no program shows alone.

### Consequences

* Good, because every route by which an unread call reaches an allocation ADR-0039's and ADR-0040's reviews measured is reported, and the library idioms build.
* Bad, because a call's result is doubtful after any exposure and after the next call, which ADR-0039 measured as the false report a user meets most.
* Bad, because a pointer parameter read after any opaque call is unproven and, under [ADR-0033](./0033-a-conclusion-this-analysis-could-not-prove-does-not-build.md), does not build; so is a read through one parameter after another is freed, and a load out of a call's result in a loop from the second turn. ADR-0040 measured each.
* Bad, because a callee returning what it freed itself (#252), and a call returning what an earlier one freed (#379), are believed, and `realloc`'s failure branch is refused where the size may be zero.
* Bad, because the contents table is square in the locals; what it costs packed is measured on `Held::names`.
* What would reverse this: summaries of the functions this translation unit defines, or an annotation saying what a callee may free.

## More Information

* Superseded: [ADR-0039](./0039-an-allocation-code-this-check-cannot-read-may-reach-stays-exposed-while-it-lives.md) whole; [ADR-0040](./0040-a-pointer-read-out-of-memory-reaches-what-was-stored-and-a-parameter-is-exposed-where-the-function-starts.md)'s rules on a pointer parameter, what the caller stored behind it, what a callee handed over, and a free of what the caller owns.
* Beside it: [ADR-0029](./0029-a-call-this-check-cannot-read-replaces-what-an-escaped-local-holds.md) for the locals such a call writes, [ADR-0044](./0044-a-store-through-a-pointer-that-may-hold-what-this-check-cannot-name-exposes-what-it-carries.md) for the store that exposes, [ADR-0047](./0047-an-allocation-code-this-check-cannot-read-can-reach-is-not-one-it-saw-freed.md) for what `Reachable` means, [ADR-0038](./0038-a-hatch-is-a-function-definition-and-what-it-could-not-prove-is-listed-rather-than-reported.md) for a hatch, and [ADR-0062](./0062-each-belief-s-routes-are-generated-as-a-matrix-and-asked-against-a-control.md) for the route matrix, whose `Caller` rows ask rule 9 of every route.
* The code: `terminator` and `on_entry` of `Allocations` in [`memory/transfer.rs`](../../crates/safec-ir/src/memory/transfer.rs), `Known` in [`memory/known.rs`](../../crates/safec-ir/src/memory/known.rs), `Cell` in [`memory/parts.rs`](../../crates/safec-ir/src/memory/parts.rs).
