---
status: "accepted"
date: 2026-09-26
decision-makers: itsakeyfut
---

# An allocation code this check cannot read may reach stays exposed while it lives, and a call this check cannot read may return any exposed allocation

## Context and Problem Statement

`Callee::Opaque` in `crates/safec-ir/src/memory.rs` is what a call this check
cannot read does to what it knows. [ADR-0029](./0029-a-call-this-check-cannot-read-replaces-what-an-escaped-local-holds.md)
made it assume the worst of the allocations its arguments name and of the
locals whose address escaped, and took the call's result as a fresh
allocation. [#250](https://github.com/itsakeyfut/safec/issues/250) measured two
programs where that lets a use after free build, which is the bottom row of
`CLAUDE.md`'s list.

A first design closed those two by marking the routes one at a time. Review
found four more routes it left open, each measured to exit 0 and two run to a
read of freed memory: a pointer stored into a stack slot through a pointer, an
address as the argument, a pointer handed to an earlier call and returned by a
later one, and a call in a loop whose result may be last turn's freed
allocation. It also refused `realloc` and `memset` idioms `main` builds. That
design was withdrawn before merging; #250's comments carry what it found.

## Decision Drivers

* `CLAUDE.md`'s ranking. Every option leaves something at row 4; the ones that
  leave something at row 6 say so here.
* Measured against two sets rather than the corpus alone, because the corpus
  holds no library call that returns a pointer (RK-080 in the review knowledge
  bank): seven programs with a defect for some definition of the callees C
  permits, and eight well-defined ones using the C library the way real code
  does. The prototype and the tables are on #250's `/spec` comment.
* C17 7.1.3 reserves the library's names, which is why `malloc` and `free` are
  read by name at all.

## Considered Options

The thing a call reaches:

* one mark per allocation, *exposed*, set once code this check cannot read may
  reach a pointer to it, and never cleared while the allocation lives
* marking each route to a callee separately (the withdrawn design)

What a call may return:

* a fresh allocation, or any exposed one
* a fresh allocation, or what this call's own arguments reach
* a fresh allocation only (`main`'s rule)

A pointer stored through a pointer:

* recorded as the contents of the allocation it was stored into, and exposed
  when that allocation is
* exposed at the store

## Decision Outcome

Chosen: **an exposed mark per allocation, a call's result that may be any
exposed allocation, and a record of what each allocation may contain.**

**Exposed.** An allocation is exposed once a pointer to it may be reached by
code this check cannot read: named by an argument of an opaque call, returned by
one, held by a local whose address escaped (a local an argument points at is
one), contained in an exposed allocation, or stored through a pointer this
check cannot say where it lands. The mark is a union at a join and survives
everything but a rebirth of the site. At every opaque call the call's reach is
added to it, closed over what each exposed allocation holds, and every exposed
allocation still live becomes unproven. A proved free stays proved. This covers all four routes review found
and both of #250's programs, and it costs one corpus case its wording and
nothing else.

**What a call returns.** A fresh allocation, or any exposed one, at an offset
nobody said; and it is exposed itself, since the callee had it. That catches a
pointer handed to one call and returned by another, which is ordinary registry
code. It costs `show(p); q = make(); *q`: `q` may be `p`, and is unproven; and
`q = make(); log_line(); *q`, since `log_line` may free what `make` kept; and a
loop that frees what a call returns each turn, since the next turn's call may
return it again. Returning only what the call's own arguments
reach keeps that program and leaves the registry silent; asked, and the row-6
direction was ranked below.

**A site reborn from an opaque call that was exposed stays unproven.** In a loop
the call writes the same site every turn, and what it returns may be last
turn's allocation, which the site number cannot tell from the new one.

**Contents.** A write through a pointer that holds sites records what it
carries as the contents of those allocations; one this check cannot place
exposes what it carries at once. So `*tab = p; log_line(); *p` builds: `tab` was
never reachable by `log_line`, so neither is `p`. Exposing at the store instead
refused that program. The record is a square table beside the two `Held`
already has, which is the condition `Held::sites` names for a packed bitset:
measured on a 456-line function, peak memory went from 485 MB to 711 MB and time
from 0.49 s to 0.71 s. The bitset is left to
[#173](https://github.com/itsakeyfut/safec/issues/173), and the number is
written where the table is.

**The library, by name.** `calloc` and `aligned_alloc` are read as `malloc` is,
since C17 7.22.3 p1 gives all three the same guarantee: the start of an object
disjoint from any other. `realloc` makes its first argument unproven rather than
freed, since 7.22.3.5 p3 leaves the old object alone when it fails with a
nonzero size, and returns a fresh allocation holding what the old one held
(p2). `memset`, `memcpy`, `memmove`, `strcpy`,
`strncpy`, `strcat` and `strncat` free nothing and return their first argument,
which C17 7.24.2 to 7.24.6 say of each; what they are handed is exposed, because
they copy bytes and a pointer is bytes. Measured, this is what builds
`memset(malloc(4), 0, 4)` and its siblings, which `main` refused already.

**What is believed, and written down.** A callee that returns an allocation it
made and freed itself is a fresh allocation to this rule, and silent: #252.
`realloc`'s failure branch, `if (q == 0) free(p);`, is unproven because nothing
ties the result's nullness to the argument: [#253](https://github.com/itsakeyfut/safec/issues/253).

**What this does not reach, found by review, and written down.** A pointer read
out of memory, `q = *tab`, holds no site, which is ADR-0017's belief about it;
this rule reads "holds no site" as "reaches nothing", so handing such a pointer
to a call exposes nothing it points at, and neither does copying it from one
allocation to another. An allocation a parameter holds is not exposed at entry,
though the caller had it. Both were silent on `main` too, and both need a
decision this record does not take:
[#254](https://github.com/itsakeyfut/safec/issues/254). And a store into a local
aggregate, which Phase 9 brings as a projection other than one `Deref`, has no
targets and exposes what it carries at the store; recording it inside the local
instead is the fix, and its shape waits for how fields are lowered.

### Confirmation

Every mutation below was applied on its own to the tree as committed, the whole
workspace was run with `--no-fail-fast`, and the file was restored from git. The
tests named are the ones that failed; how many there were is not written down,
for RK-028's reason. The cases are in `crates/safec/tests/cases` and every
mutation is in `crates/safec-ir/src/memory.rs`.

**What a call reaches.** Leaving the arguments out of `Known::reach_of` fails
`what_a_callee_frees_through_a_pointer_stored_in_the_heap_is_unproven_after_it`
and `a_call_may_return_what_it_was_handed`, #250's two programs, among others.
Leaving the escaped locals out fails
`what_a_callee_frees_through_a_pointer_stored_in_a_local_is_unproven_after_it`,
`a_call_handed_an_address_may_return_what_is_behind_it` and
`a_pointer_in_a_local_whose_address_an_earlier_call_kept_is_unproven_after_a_later_call`.

**Exposure.** No closure over what an allocation holds, and never recording it
at a write, each fail
`a_pointer_stored_on_one_arm_is_reached_through_what_holds_it` and the heap
case. A closure that stops after one step fails
`a_pointer_two_tables_deep_is_reached_through_both` alone, and recording into
only the first allocation a pointer may hold fails
`a_pointer_stored_through_either_of_two_tables_is_inside_both` alone. Closing only from the sites a call has just marked fails
`a_pointer_in_a_table_exposed_on_the_other_arm_is_unproven_after_a_call` alone,
which is the join RK-044 describes, and which mutating the first version of
this change found as a silence. An unplaced write exposing nothing fails
`a_pointer_stored_two_levels_down_is_reached_through_what_holds_it` alone.
Dropping either half of the join fails a one-arm case alone. Keeping the mark
through a rebirth fails
`an_allocation_made_again_at_a_site_is_not_the_one_exposed_before` alone, by a
false positive.

**At the call.** Not unproving the exposed allocations fails every case that
reads one after a call. Unproving a proved free as well fails
`a_free_proved_before_a_call_stays_proved_after_it` alone.

**What a call returns.** Not holding the exposed sites fails
`a_call_may_return_what_an_earlier_call_was_handed`,
`a_call_may_return_what_it_was_handed` and
`a_call_after_an_allocation_was_exposed_may_return_it`, the last being the cost
the rule accepts. Holding them at `Offset::Zero` loses the `SC0404` of
`a_call_may_return_what_an_earlier_call_was_handed` and of the loop case.
Proving a reborn exposed site live fails
`a_call_in_a_loop_may_hand_back_what_was_freed_last_turn` alone. Not exposing
the result itself fails `what_a_call_returned_is_unproven_after_a_later_call`.

**The library.** Reading none of the seven functions that return their first
argument by name fails `what_memset_returns_is_the_allocation_it_was_handed`,
`memcpy_frees_neither_of_its_arguments` and
`what_strcpy_returns_is_the_allocation_it_was_handed`, and leaving out any one
of the other four fails
`what_the_other_library_copies_return_is_what_they_were_handed`. Their arm not
exposing
what it is handed fails
`what_memset_was_handed_is_unproven_after_a_later_call`; not replacing what an
escaped local holds fails
`a_local_memcpy_is_handed_the_address_of_may_hold_something_else_after_it`
alone; not returning the first argument fails the `memset` and `strcpy` cases.
Not reading `realloc` by name fails its cases; proving its argument freed
rather than unproven fails
`a_free_on_reallocs_failure_branch_is_not_proved`, whose `SC0401` would claim
what C17 7.22.3.5 p3 contradicts; not asking its argument whether it was freed
fails `a_freed_pointer_handed_to_realloc_is_freed_twice`; asking its size too
fails `the_size_realloc_is_handed_is_not_asked_whether_it_was_freed`; not
carrying what the old object held fails
`a_table_realloc_grew_still_holds_what_it_held`, which reading `realloc` by name
had made silent before review; and `what_realloc_returns_is_named_where_it_was_allocated`,
`a_pointer_freed_before_realloc_stays_freed_after_it` and
`a_pointer_into_an_allocation_handed_to_realloc_is_not_its_start` pin its label,
a proved free across it, and its offset. Not reading `calloc` or `aligned_alloc`
by name fails the case named for each.

**ADR-0029 still holds.** Dropping its `Known::replaced` from the opaque arm
fails the four cases its Confirmation names, measured after this change, so the
exposed mark did not make them vacuous (RK-048).

**What nothing holds.** Clearing what a reborn site holds, for the reason its
comment gives; `realloc` being asked about its first argument only in its
transfer, which no C program can show since its size holds no allocation; and `Analysis::height`, as for every other term in it.

### Consequences

* Good, because the six silences #250 and its review measured are reported,
  and the library idioms `main` refused build.
* Bad, because an allocation handed to any call this check cannot read makes
  every later call's result doubtful, and a call's result is doubtful after the
  next call: row 4, and the one a user will meet most.
* Bad, because the memory check costs about half again in memory and time,
  until #173.
* Bad, because a callee returning what it freed itself is believed (#252), and
  `realloc`'s failure branch is refused (#253).
* Bad, because a pointer read out of memory, and a parameter's allocation, are
  not exposed, so a use after free through either still builds (#254).
* What would reverse this: summaries of functions this translation unit
  defines, which would let a call's result be what its body returns rather than
  anything exposed.

## Pros and Cons of the Options

### One exposed mark

* Good, because every route this check can name ends in the same place, so a
  route nobody listed is not a silence. A pointer it cannot name, one read out
  of memory, is the exception, and #254.
* Bad, because it never forgets.

### Marking routes one at a time

* Good, because it costs least.
* Bad, because review found four routes it missed.

### Result: any exposed allocation

* Good, because registry code is reported.
* Bad, because every call after an exposure returns something doubtful.

### Result: what the arguments reach

* Good, because an unrelated call's result stays proved.
* Bad, because a registry's use after free builds.

### Result: fresh only

* Good, because it is `main`.
* Bad, because a call returning its argument is silent.

### Contents recorded

* Good, because a table nothing unread can reach does not expose what it holds.
* Bad, because it is a third square table.

### Exposed at the store

* Good, because it needs no table.
* Bad, because it refuses a pointer stored in a table followed by any call.

## More Information

* [#250](https://github.com/itsakeyfut/safec/issues/250), whose comments carry
  the withdrawn design, what its review found, and the probe tables.
* [ADR-0029](./0029-a-call-this-check-cannot-read-replaces-what-an-escaped-local-holds.md),
  which this extends from the locals an opaque call reaches to the allocations.
* [ADR-0038](./0038-a-hatch-is-a-function-definition-and-what-it-could-not-prove-is-listed-rather-than-reported.md),
  whose wider rule for a hatch stays.
* RK-080 in the review knowledge bank, on why the corpus alone could not cost
  this.
