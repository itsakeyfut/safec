---
status: "accepted"
date: 2026-10-01
decision-makers: project author
---

# A pointer says whether it may point into memory this check does not model, and an allocation says whether it may hold one

## Context and Problem Statement

A write through a pointer records what it stores inside the allocations the pointer holds, and exposes it at once when it cannot place it (ADR-0039). A pointer read out of memory holds no site (ADR-0017) and carries `Held::loaded` (ADR-0040), which says that it was read out of memory and not from where.

That loses the one distinction a store needs. A pointer read out of a parameter's memory may point into memory the caller owns, which code this check cannot read can reach. A pointer read out of an allocation this function made and never exposed points at one of the allocations stored there, every one of which this check names. Treating the two alike was wrong in both directions ([#276](https://github.com/itsakeyfut/safec/issues/276)):

- **A silence.** `t = c ? s : *tab; *t = a; release_all(); return a[0];` built, because the site `s` made the store placed and the load exposed nothing. So did the same with `t = c ? s : u` after `get(&u)`, since a pointer a call writes into a local that holds no site sets neither `loaded` nor `lost` (ADR-0017, ADR-0029).
- **A false refusal.** Exposing what is stored through every possible load refused a function appending to a list it built itself, with a double free that cannot happen.

## Decision Drivers

* A missed route is a silence, row 6 of the ranking; an extra one is a report the reader can see, row 4.
* ADR-0017's refusal to report an output parameter, `int *p; get(&p); *p = 1;`, has to survive: nothing the report reads may change for it.
* Lattice size: #173 tracks the cost of square fields, and a third already exists.

## Considered Options

* **Expose what a store carries through any pointer that may be a load.** This was the first implementation of #276, and was rejected by its review for the list program.
* **A bit on the value alone.** The bit is set where a load reads memory reachable to code this check cannot read. It is lost when such a pointer is stored into the function's own memory and read back, so that route is a silence.
* **A bit on the value, and a bit per allocation saying it may hold such a pointer.**

## Decision Outcome

Chosen option: **a bit on the value, and a bit per allocation**.

**The value's bit is `Held::foreign`**: the local may point into memory reachable to code this check cannot read, which no site names. It is set:

- by a load through one `Deref` when the pointer read through is itself foreign or lost, or when any allocation it may point at is reachable to such code (`Known::reachable_now`) or may hold such a pointer. A load through a pointer that may itself be a load asks the same of every allocation `Known::stored` names. A load through any deeper projection is foreign.
- by a call this check cannot read, for every local whose address escaped and whose type it may write, whether or not the local holds a site. This is `Known::replaced`. A local holding no site gets `foreign` and not `lost`, so the report, which reads `lost` and not `foreign`, is unchanged for an output parameter.
- by arithmetic on such a load, as `loaded` is (ADR-0040).

It is cleared by an assignment and unioned at a join, as `loaded` is. The report never reads it.

**The allocation's bit is `Known::holds_foreign`**, one per site. It is set where a store of a foreign value is recorded into an allocation, unioned at a join, and kept through a rebirth for `inside`'s reason (ADR-0040).

**A store through a pointer** then:

- exposes what it carries when the pointer is foreign or lost (ADR-0039's unplaced write, for that part);
- otherwise, when the pointer may be a load, records what it carries inside every allocation `Known::stored` names, since the load points at one of them, rather than exposing it;
- records into the sites it holds as before.

### Confirmation

Every mutation below was applied on its own to `crates/safec-ir/src/memory.rs`, the whole workspace was run with `--no-fail-fast`, and the file was restored. The cases are in `crates/safec/tests/cases`.

- **`Known::foreign_load` answering not foreign** fails, among others:
  - `a_store_through_a_pointer_that_may_be_a_load_exposes_what_it_stores`;
  - `a_read_before_a_store_through_a_pointer_that_may_be_a_load_is_asked_at_a_later_call`;
  - `a_foreign_pointer_stored_in_the_function_s_own_memory_stays_foreign`;
  - `a_pointer_moved_off_an_unmodeled_load_is_still_unmodeled`.

  All of them go silent.
- **It answering foreign** fails `a_list_built_and_appended_to_in_one_function_is_not_refused` and `a_pointer_read_out_of_the_function_s_own_memory_is_not_unmodeled`, which report.
- **One case alone per route.** Each of these fails only the case named, which goes silent unless it says otherwise:

| Mutation | The case that fails |
|---|---|
| not setting the bit in `Allocations::read_through` | the first three above, together |
| not setting `Known::holds_foreign` at a store | `a_foreign_pointer_stored_in_the_function_s_own_memory_stays_foreign` |
| not carrying the bit in `built_from` | `a_pointer_moved_off_an_unmodeled_load_is_still_unmodeled` |
| not carrying it in `Held::accumulated` | `a_pointer_moved_off_a_local_holding_an_unmodeled_load_is_still_unmodeled` |
| not setting it in `Known::replaced` | `a_store_through_a_pointer_a_call_filled_in_exposes_what_it_stores` |
| keeping one arm's bit in `Held::joined` | `a_store_through_a_pointer_that_is_a_load_on_the_first_arm_exposes_what_it_stores` |
| keeping one arm's `Known::holds_foreign` at the join | `a_foreign_pointer_stored_on_one_arm_is_foreign_after_the_join` |
| adding nothing from `Known::stored` to a store's containers | `a_store_through_a_modeled_load_lands_in_what_it_may_point_at`, which loses its report at the read |
| exposing a store through a modeled load that has no site of its own | `a_pointer_read_out_of_the_function_s_own_memory_is_not_unmodeled`, which reports |
| dropping `lost` from the store's condition | `a_store_through_a_pointer_that_lost_its_site_exposes_what_it_stores`, which loses its report at the read |

- **Held by nothing.** `Held::clear` resetting the bit fails nothing. A fresh allocation reaches a local as a copy of a whole `Held`, and no assignment measured goes through `clear` with the bit set. `Known::reborn` keeping `Known::holds_foreign` is unguarded for `Known::inside`'s reason.

### Consequences

* Good, because the two silences in #276 report, and a store through a pointer read out of the function's own memory no longer exposes anything. On a prototype, `*s2 = s; t = *s2; *t = a; release_all(); return a[0];`, which `main` refused, built.
* Bad, because a write by this function through a pointer it cannot pin down makes every escaped local foreign, though what it wrote may not have been. That is ADR-0031's writer reusing a rule written for a call, and costs a report rather than a silence.
* Bad, because the lattice gains one bit per local and one per site, and `Analysis::height` grows by two steps per local for them.
* What would reverse this: following a load to the allocations it may have been read from, as #256 asks of the report, which would make both bits derivable.

## More Information

[#276](https://github.com/itsakeyfut/safec/issues/276) has the programs and the review that found them. ADR-0017, ADR-0029, ADR-0039 and ADR-0040 are what this narrows or extends.
