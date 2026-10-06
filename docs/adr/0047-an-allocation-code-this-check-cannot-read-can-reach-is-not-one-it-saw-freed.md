---
status: "accepted"
date: 2026-10-05
decision-makers: itsakeyfut
---

# An allocation code this check cannot read can reach is not one it saw freed

## Context and Problem Statement

`SiteState::Unknown` meant two things. An allocation freed on one path and not on another, or handed by value to a call that may free it, is one this check saw may be gone. An allocation held by a local whose address escaped (ADR-0017), or exposed to a call (ADR-0039), is one this check saw nothing happen to: code it cannot read can reach it, and may free it later. ADR-0042 asks a call handed `&a` about what `a` may point at, and had to exempt `Unknown` reached through the address, because taking the address makes a live allocation `Unknown` and asking about it refused every `use2(&a)`. The exemption covered the first meaning too, so a pointer freed on one path, or handed to `release(a)`, and then handed by address built with exit 0 ([#305](https://github.com/itsakeyfut/safec/issues/305)).

## Decision Drivers

* A use after free in the callee that builds is row 6 of the ranking. A report about a program C defines is row 4.
* Passing a pointer by address to a function that may replace it, `grow(&a)`, is common C, and so is calling one twice.
* No answer of the report may become less than it is on `main`: a site this check stops calling `Unknown` must still be doubted everywhere `Unknown` is.

## Considered Options

* **Keep one `Unknown`**, and the exemption, as now.
* **A second state for "code this check cannot read can reach it", made only where an address is taken**, with every call, exposure and hatch still writing `Unknown`.
* **A second state made wherever the allocation is reached only through an address or an exposure**: taking an address, an exposure at a call, a call left unordered against an exposure, a hatch. A free on one path, a free of a set or of a pointer this check stopped following, a `realloc`, and a call that holds the pointer, by name, as a load, read out of memory, or inside an allocation it was handed, still write `Unknown`.

## Decision Outcome

Chosen option: **the second state where the allocation is reached only through an address or an exposure**, `SiteState::Reachable`. It is live as far as this check saw. Every reader of a state answers it as it answers `Unknown`, a doubt, except `Known::handed_below`: a call handed an address is asked about what the address reaches when it is `Unknown`, and not when it is `Reachable`, which is the one meaning the exemption was for. The join keeps `Reachable` where both sides are live or reachable, and gives `Unknown` wherever a side is freed or unknown, so a free on one path reaches the call as `Unknown`.

Measured on probes written for this record, each run on `main` and on a prototype of the option:

| probe | C defines it | `main` | one state where an address is taken | reached only through an address (chosen) |
|---|---|---|---|---|
| #305's program: freed on one arm, then `use2(&a)` | no, `use2` reads freed memory on that path | builds | refused | refused |
| `release(a); use2(&a);` | no, if `release` frees | builds | refused | refused |
| the same through a copy, `b = a; if (c) free(b); use2(&a);` | no | builds | refused | refused |
| `use2(&a)` over a live pointer | yes | builds | builds | builds |
| `grow(&a); grow(&a);` | yes | builds | **refused** | builds |
| `use2(&a); log_it(); use2(&a);` | yes | builds | **refused** | builds |
| freed on both paths, then `use2(&a)` | no | refused | refused | refused |
| `q = *t; release(q); use2(&a);` | no, if `release` frees | builds | refused | refused |
| `*d = a; release_in(d); use2(&a);` | no, if `release_in` frees `*d` | builds | refused | refused |
| `show1(q)` or `show(d)` for those calls, then `use2(&a)` | yes, if they only read | builds | refused | refused |
| the same two, then `return *a;` | yes, if they only read | **refused** | refused | refused |

The chosen option first drew its line at a call naming the allocation by value, which left the two rows after the table's first seven building ([#324](https://github.com/itsakeyfut/safec/issues/324)); it now draws it at the route, and the last two rows are what that costs: a call that only read the pointer it held makes `use2(&a)` a doubt, which a direct use of `a` after the same call already was on `main`. `Allocations::reach` cannot draw it, since `Known::reach_of` adds every escaped local's sites to every call, and writing `Unknown` on it refused the in-out rows.

Neither option changes any answer in the corpus but one: `a_may_set_freed_then_written_through_an_alias` now also reports `SC0407` at `opaque(pp)`, where `pp` is the address of a pointer whose set was freed (ADR-0020), which is true.

The route is drawn at the holder too ([#326](https://github.com/itsakeyfut/safec/issues/326)). A call this check cannot read leaves `Reachable` only where it could have replaced every local still read after it that holds the allocation, which is a local whose address code this check cannot read may hold: handed to such a call, stored in memory such a call reached, or stored in memory this check does not model, on every path to the call. Not every local whose address was taken: `int **pa = &a;` with `pa` read by nobody made `a` exempt and the first program below build again. Where a holder not handed away holds it, the call may have freed it through another route and cannot have put anything new there, so it is `Unknown`: `b = a; release_ref(&b); use2(&a);` and `*box = &b; release_deep(box); use2(&a);` are asked at `use2(&a)`, as the load and memory routes above are. Recording which address made a site `Reachable` would say the same on every probe below and needs a route through every writer of the state and the join. Only a holder live after the call counts, by the liveness ADR-0048 computes:

| probe | C defines it | before #326 | every holder | a holder live after the call (chosen) |
|---|---|---|---|---|
| `b = a; release_ref(&b); use2(&a);` | no, if `release_ref` frees and `use2` reads `*pp` | builds | refused | refused |
| `*box = &b; release_deep(box); use2(&a);` | no, if `release_deep` frees and `use2` reads `*pp` | builds | refused | refused |
| `b = a; use2(&b); use2(&a);` | yes, if `use2` only reads | builds | refused | refused |
| `b = a; grow(&b); grow(&b);` | yes | builds | **refused** | builds |
| `b = a; grow(&b); use2(&b);` | yes | builds | **refused** | builds |
| `b = a; grow(&a); use2(&a);`, `b` never read again | yes | builds | **refused** | builds |

The third row is the cost the first table already pays in its `show1(q)` row. The same cost reaches the cursor idiom wherever the original is used again by address: `p = buf; next_token(&p); release_buf(&buf);` is refused, since `next_token` may have freed what `buf` holds and cannot replace `buf`, and is defined where it only advances `p`. A hole in the liveness would take a holder for dead and leave the site `Reachable`, which is a silence; ADR-0048's unit tests, one per kind of read, are what hold that. A route `handed_away` misses leaves a holder counted, which is a report.

A slot in memory is a holder the call may not be able to replace either, and it has no liveness to say whether it is read again ([#346](https://github.com/itsakeyfut/safec/issues/346)). So it is not counted: each allocation that may contain the site is marked `stale`, as ADR-0045 marks one whose contents may name something gone, and a pointer later read out of it is asked where it is handed by address, whatever its sites' state. Measured against counting the slot as a holder:

| probe, after `*h = a;` | C defines it | before #346 | slot counted | slot marked (chosen) |
|---|---|---|---|---|
| `b = a; release_ref(&b); c = *h; use2(&c);` | no, if `release_ref` frees and `use2` reads `*pp` | builds | refused | refused |
| `b = a; grow(&b); c = *h; use2(&c);` | no, if `grow` reallocates and `use2` reads | builds | refused | refused |
| `use2(&a); log_line(); c = *h; use2(&c);` | no, if either frees | builds | refused | refused |
| `grow(&a); use2(&a);`, `*h` never read again | yes | builds | **refused** | builds |
| `use2(&a); use2(&a);`, `*h` never read again | yes | builds | **refused** | builds |
| `*h = 0; b = a; grow(&b); use2(&b);` | yes | builds | **refused** | builds |

Every allocation that may contain the site is marked, not only those the call did not reach: a load out of one it reached is doubted already, and skipping those, the ones no live local reaches, or the ones already freed moved no probe and no corpus case. A stale read for any other reason is now asked at a call by address as well, which moved no corpus case either.

The rule also turns a site `Unknown` at a call that did not reach it: every escaped local's sites are in every call's reach (ADR-0039), so after `b = a; int **pb = &b; log_line();`, which reaches nothing in C, `use2(&a)` is refused. Narrowing the sites to the ones the call reached through an address handed away would make a missed route a silence, so it stays a report, and [#345](https://github.com/itsakeyfut/safec/issues/345) is where the reach itself is to narrow.

What the chosen option still does not ask, each of which builds on `main` as well: a call handed only an address that frees what is there and does not replace it, followed by a second call handed the address, which is the callee's contract; an allocation exposed to code this check cannot read, freed by a later call through what it kept, and then handed by address.

### Confirmation

The cases are in `crates/safec/tests/cases`, and every mutation is in `crates/safec-ir/src/memory/`.

- `error[E0004]` at every `match` on `SiteState` if a variant is added, which is how every reader written as a `match` was made to answer `Reachable`. A reader written as `matches!` is not held by it, and review found one: `Known::reborn` counted `Reachable` as live, so a pointer stored last turn that a call may have freed was read as the new allocation in silence. It counts it gone, and `a_pointer_stored_last_turn_that_a_call_may_have_freed_is_doubted_after_the_loop_allocates_again` and `a_pointer_copied_last_turn_that_a_call_may_have_freed_is_doubted_when_stored_and_read_back` fail if it counts it live.
- `handed_below` exempting `Unknown` through the address again fails `a_pointer_freed_on_one_path_and_handed_by_address_is_asked`, `a_pointer_whose_address_was_taken_before_a_free_on_one_path_is_asked`, `a_pointer_handed_to_a_call_and_then_by_address_is_asked` and `a_may_set_freed_then_written_through_an_alias`, which lose their `SC0407`. The second is the two events in the other order, since a rule about two events is held only in the order a case writes them; this mutation is the only one of the four it fails under, since there the free is of an escaped local, which writes `Unknown` on its arm before any join.
- The join giving `Reachable` for a freed side fails `a_pointer_freed_on_one_path_and_handed_by_address_is_asked`, and seventeen other cases whose doubt rests on the same join.
- An opaque call writing `Reachable` on what it is handed by value fails `a_pointer_handed_to_a_call_and_then_by_address_is_asked`, beside two cases about a call between two frees.
- The opaque call leaving out what it holds, by name, as a load or through the memory it was handed, fails `a_pointer_a_call_was_handed_through_a_load_is_asked_by_address` and `a_pointer_a_call_reached_through_memory_is_asked_by_address`, which go silent; taking only what it is handed, without what that memory holds, fails the second alone. Leaving `read_out` out of what it holds fails `a_pointer_a_call_was_handed_read_out_of_memory_is_asked_by_address`, `release(*t)`, alone. Writing `Unknown` on all of `reach` instead fails the two in-out cases below and nothing else.
- Every producer but an address taken writing `Unknown`, which is the rejected option, fails `a_pointer_handed_by_address_twice_builds` and `a_pointer_handed_by_address_around_another_call_builds` and nothing else, which are refused.
- Dropping the call to `Known::held_out_of_reach` fails `a_pointer_a_call_reached_through_another_locals_address_is_asked_by_address`, `a_pointer_a_call_reached_through_a_locals_address_in_memory_is_asked_by_address`, `a_pointer_whose_address_only_a_local_holds_is_asked_after_a_call_reached_it_through_a_copy`, `a_pointer_handed_away_on_one_arm_is_asked_on_the_other_after_a_call_reached_it` and `a_copy_read_by_value_after_a_call_reached_it_through_another_address_is_asked`, which build. Its `live_after` answering `true` refuses `a_copy_grown_twice_by_address_builds`, `a_dead_copy_does_not_doubt_a_pointer_grown_by_address` and two more, and counting only a holder whose address is taken silences the last of the five above. Asking whether a holder holds any site rather than this one refuses `a_call_reached_through_a_copy_of_another_allocation_does_not_doubt_this_one`.
- Testing `escaped` in place of `handed_away` silences `a_pointer_whose_address_only_a_local_holds_is_asked_after_a_call_reached_it_through_a_copy` and the one-arm case. Each route that sets `handed_away` fails its own: dropping the argument fails `a_pointer_handed_by_address_twice_builds` and five more, dropping memory a call reached refuses `a_pointer_whose_address_is_in_exposed_memory_builds_after_a_call` and `a_pointer_grown_through_the_memory_its_address_was_stored_in_builds`, and dropping a store into memory this check does not model refuses `a_pointer_whose_address_is_stored_through_a_load_builds_after_a_call`. A join that unions it silences `a_pointer_handed_away_on_one_arm_is_asked_on_the_other_after_a_call_reached_it`.
- Dropping the marking of a slot in `held_out_of_reach`, or the `stale` test in `handed_below`, silences `a_pointer_copied_out_of_a_slot_a_call_could_not_replace_is_asked_by_address`; turning the site `Unknown` for a slot instead refuses `a_slot_nothing_reads_again_does_not_doubt_a_pointer_grown_by_address`.

### Consequences

* Good, because the doubt a call by address is asked about is a doubt about a free, and the in-out idiom keeps building where the pointer handed is the one used again; through a copy whose original is used again by address, it is refused (#326).
* Good, because `Reachable` is doubted everywhere `Unknown` was, so nothing reported on `main` stops being reported.
* Bad, because a site's state now carries why it is doubted, which is one more thing a new producer of a doubt has to choose between. It costs `Analysis::height` nothing: `Reachable` and `Freed` join to `Unknown`, so a site still walks at most two steps.
* Bad, because the choice is made per site, not per local: a site freed on one path through one local and reached through the address of another is `Unknown` for both, and so is a site a call reached through a copy's address while the original is still read.
* What would reverse this: summaries of the callees this translation unit defines, or an annotation saying what a callee does with an address, which would answer the residual case above rather than exempt it.

## More Information

* `SiteState` in [`crates/safec-ir/src/memory/parts.rs`](../../crates/safec-ir/src/memory/parts.rs), `Known::handed_below` in [`crates/safec-ir/src/memory/known.rs`](../../crates/safec-ir/src/memory/known.rs), and the call transfer in [`crates/safec-ir/src/memory/transfer.rs`](../../crates/safec-ir/src/memory/transfer.rs).
* [ADR-0042](./0042-a-pointer-handed-to-a-call-is-asked-at-the-call-as-a-dereference-of-it-would-be.md) is the question this narrows the exemption of; [ADR-0017](./0017-record-each-half-of-an-escape-where-its-subject-lives.md) is why taking an address doubts an allocation; [ADR-0039](./0039-an-allocation-code-this-check-cannot-read-may-reach-stays-exposed-while-it-lives.md) is the exposure.
* [#305](https://github.com/itsakeyfut/safec/issues/305) carries the probes and the prototype measurements.
