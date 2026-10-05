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
* **A second state made wherever the allocation is reached only through an address or an exposure**: taking an address, an exposure at a call, a call that names it through an address and not by value, a call left unordered against an exposure, a hatch. A free on one path, a free of a set or of a pointer this check stopped following, a `realloc`, and a call handed the pointer itself still write `Unknown`.

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

Neither option changes any answer in the corpus but one: `a_may_set_freed_then_written_through_an_alias` now also reports `SC0407` at `opaque(pp)`, where `pp` is the address of a pointer whose set was freed (ADR-0020), which is true.

What the chosen option still does not ask, each of which builds on `main` as well: a call handed only an address that frees what is there and does not replace it, followed by a second call handed the address, which is the callee's contract; an allocation exposed to code this check cannot read, freed by a later call through what it kept, and then handed by address; a call handed a pointer read out of memory, `q = *t; release(q); use2(&a);`, since `Allocations::named_outright` leaves a load out of what is named by value; and a call reaching the allocation through memory it was handed, `*d = a; release_in(d); use2(&a);`, which can free it and cannot replace `a`. The last two would need the doubt to say whether the route was an address or memory, which is a finer choice than this record makes.

### Confirmation

The cases are in `crates/safec/tests/cases`, and every mutation is in `crates/safec-ir/src/memory/`.

- `error[E0004]` at every `match` on `SiteState` if a variant is added, which is how every reader written as a `match` was made to answer `Reachable`. A reader written as `matches!` is not held by it, and review found one: `Known::reborn` counted `Reachable` as live, so a pointer stored last turn that a call may have freed was read as the new allocation in silence. It counts it gone, and `a_pointer_stored_last_turn_that_a_call_may_have_freed_is_doubted_after_the_loop_allocates_again` and `a_pointer_copied_last_turn_that_a_call_may_have_freed_is_doubted_when_stored_and_read_back` fail if it counts it live.
- `handed_below` exempting `Unknown` through the address again fails `a_pointer_freed_on_one_path_and_handed_by_address_is_asked`, `a_pointer_whose_address_was_taken_before_a_free_on_one_path_is_asked`, `a_pointer_handed_to_a_call_and_then_by_address_is_asked` and `a_may_set_freed_then_written_through_an_alias`, which lose their `SC0407`. The second is the two events in the other order, since a rule about two events is held only in the order a case writes them; this mutation is the only one of the four it fails under, since there the free is of an escaped local, which writes `Unknown` on its arm before any join.
- The join giving `Reachable` for a freed side fails `a_pointer_freed_on_one_path_and_handed_by_address_is_asked`, and seventeen other cases whose doubt rests on the same join.
- An opaque call writing `Reachable` on what it is handed by value fails `a_pointer_handed_to_a_call_and_then_by_address_is_asked`, beside two cases about a call between two frees.
- Every producer but an address taken writing `Unknown`, which is the rejected option, fails `a_pointer_handed_by_address_twice_builds` and `a_pointer_handed_by_address_around_another_call_builds` and nothing else, which are refused.

### Consequences

* Good, because the doubt a call by address is asked about is a doubt about a free, and the in-out idiom keeps building.
* Good, because `Reachable` is doubted everywhere `Unknown` was, so nothing reported on `main` stops being reported.
* Bad, because a site's state now carries why it is doubted, which is one more thing a new producer of a doubt has to choose between. It costs `Analysis::height` nothing: `Reachable` and `Freed` join to `Unknown`, so a site still walks at most two steps.
* Bad, because the choice is made per site, not per local: a site freed on one path through one local and reached through the address of another is `Unknown` for both.
* What would reverse this: summaries of the callees this translation unit defines, or an annotation saying what a callee does with an address, which would answer the residual case above rather than exempt it.

## More Information

* `SiteState` in [`crates/safec-ir/src/memory/parts.rs`](../../crates/safec-ir/src/memory/parts.rs), `Known::handed_below` in [`crates/safec-ir/src/memory/known.rs`](../../crates/safec-ir/src/memory/known.rs), and the call transfer in [`crates/safec-ir/src/memory/transfer.rs`](../../crates/safec-ir/src/memory/transfer.rs).
* [ADR-0042](./0042-a-pointer-handed-to-a-call-is-asked-at-the-call-as-a-dereference-of-it-would-be.md) is the question this narrows the exemption of; [ADR-0017](./0017-record-each-half-of-an-escape-where-its-subject-lives.md) is why taking an address doubts an allocation; [ADR-0039](./0039-an-allocation-code-this-check-cannot-read-may-reach-stays-exposed-while-it-lives.md) is the exposure.
* [#305](https://github.com/itsakeyfut/safec/issues/305) carries the probes and the prototype measurements.
