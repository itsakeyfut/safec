---
status: "accepted"
date: 2026-10-01
decision-makers: project author
---

# A store through a pointer that may hold what this check cannot name exposes what it carries, and a local a call may have written says so

## Context and Problem Statement

A write through a pointer records what it stores inside the allocations the pointer holds, and exposes it at once when it cannot place it (ADR-0039). A pointer that held a site was treated as placed even when it might also hold something this check cannot name, so the rest of what it might point at exposed nothing ([#276](https://github.com/itsakeyfut/safec/issues/276)):

- **A pointer read out of memory** (`Held::loaded`, ADR-0040). `t = c ? s : *tab; *t = a; release_all(); return a[0];` built, because the site `s` made the store placed and `*tab` may be the caller's memory.
- **A site it lost the name for** (`Held::lost`).
- **What a call this check cannot read may have written into it through its address.** For a local holding no site that set neither bit: `Known::replaced` leaves `lost` off it so that the report says nothing about an output parameter (ADR-0017, ADR-0029). So `get(&u); t = c ? s : u; *t = a;` was silent the same way.

## Decision Drivers

* A missed route is a silence, row 6 of the ranking; an extra one is a report the reader can see, row 4.
* Nothing may report less than before. A rule that is more precise somewhere is worth nothing if it is silent somewhere `main` was not.
* ADR-0017's refusal to report an output parameter, `int *p; get(&p); *p = 1;`, has to survive.

## Considered Options

* **Expose what a store carries for the part of a pointer this check cannot name.** A pointer may be loaded, lost, or written by a call it cannot read.
* **Also tell a load of the function's own memory from one it cannot model.** A bit would say a value may be memory reachable to code this check cannot read, and a bit per allocation would say it may hold such a value. A store through a load of the function's own memory would then be recorded in the allocations stored there rather than exposed.

## Decision Outcome

Chosen option: **expose for the part a pointer cannot name**, with one new bit for the third door.

**`Held::foreign`**, set by `Known::replaced` on every local a call this check cannot read may write whose address escaped, whatever it holds. It is unioned at a join and through arithmetic, and `Held::clear` resets it, as `Held::loaded` is. The report never reads it, so an output parameter is still not reported.

**A store through a pointer** that holds sites and is `loaded`, `lost` or `foreign` records what it carries in the sites, as before, and exposes it as well. That is ADR-0039's unplaced write, for that part.

The second option was implemented and rejected by its review. It treated a load as one of the function's own memory whenever it could not see otherwise, and that is false by too many routes, each a silence where `main` reported:

- a load through a pointer to a local (`pp = &u; t = *pp;`);
- a table of pointers grown with `realloc`, which carried the contents but not the new bit;
- a store through `**pp`;
- a local's address stored in the function's own memory and read back.

More than ten such programs were found across four review lenses. The closure each load computed also made a large function about seventeen times slower. Every review round found a new route, so the premise was dropped rather than patched.

### Confirmation

Every mutation below was applied on its own to the memory check (`crates/safec-ir/src/memory.rs` and `memory/`), the whole workspace was run with `--no-fail-fast`, and the file was restored. The cases are in `crates/safec/tests/cases`.

- **Dropping `loaded` from the store's condition** fails `a_store_through_a_pointer_that_may_be_a_load_exposes_what_it_stores` and `a_read_before_a_store_through_a_pointer_that_may_be_a_load_is_asked_at_a_later_call`, which go silent. It also fails `a_list_built_and_appended_to_in_one_function_is_refused`, which builds.
- **The output-parameter cases.** Dropping `foreign` from the store's condition, or not setting it in `Known::replaced`, fails each of these, which go silent:
  - `a_store_through_a_pointer_a_call_filled_in_exposes_what_it_stores`;
  - `a_store_through_a_pointer_a_call_filled_in_on_the_first_arm_exposes_what_it_stores`;
  - `a_pointer_moved_off_one_a_call_filled_in_is_still_one`.
- **Keeping one arm's bit in `Held::joined`** fails `a_store_through_a_pointer_a_call_filled_in_on_the_first_arm_exposes_what_it_stores` alone.
- **Not carrying it in `Held::accumulated`** fails `a_pointer_moved_off_one_a_call_filled_in_is_still_one` alone.
- **Dropping `lost` from the condition** fails `a_store_through_a_pointer_that_lost_its_site_exposes_what_it_stores` alone, which loses its report at the read.
- **Exposing every placed write** fails, among others, `a_store_through_a_pointer_that_may_be_either_of_two_allocations_exposes_nothing` and `a_pointer_stored_in_the_heap_is_not_exposed_until_what_holds_it_is`.
- **Setting it on every local a call may write, escaped or not,** fails `a_local_whose_address_no_call_was_given_is_not_one_a_call_filled_in` alone, which reports.
- **Held by nothing.**
  - `Held::clear` resetting the bit fails nothing: a fresh allocation reaches a local as a copy of a whole `Held`, and no assignment measured goes through `clear` with the bit set.
  - Dropping the `may_hold` narrowing in `Known::replaced` fails nothing either. It narrows only by the type a write through a pointer writes (ADR-0031), and a local of a type no write could hold a pointer in is never written through.

On the 158 probes kept from #273, #276 and the four reviews of #276, no program that `main` refuses builds. The programs that moved all moved from building to refused.

### Consequences

* Good, because a store through a pointer that may be the caller's memory, or that a call may have filled in, is no longer silent. Nothing reports less than before.
* Bad, because a load of the function's own memory is exposed through like any other. `a_list_built_and_appended_to_in_one_function_is_refused` is a list built and appended to in one function, refused with an `SC0402` and an `SC0401` about a double free that cannot happen. `*box = s; t = c ? s2 : *box; *t = a; log_line(); return a[0];` is refused the same way.
* Bad, because the lattice gains one bit per local, and `Analysis::height` one step per local for it. It now also counts the per-site set ADR-0042's pending reads carry, which it had not.
* What would reverse the cost: following a load to the allocations it may have been read from, so that a store through one is placed rather than exposed. That is [#278](https://github.com/itsakeyfut/safec/issues/278), the report's question in #256 asked of a store as well, and it has to answer the routes the second option missed.

## More Information

[#276](https://github.com/itsakeyfut/safec/issues/276) has the programs, the two designs and the reviews. ADR-0017, ADR-0029, ADR-0039 and ADR-0040 are what this extends.
