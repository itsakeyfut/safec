---
status: "accepted"
date: 2026-10-03
decision-makers: itsakeyfut
---

# A pointer read out of memory holds what was stored there, and proves nothing with it

## Context and Problem Statement

A pointer read out of memory holds no site. That is ADR-0017's belief about a pointer this check never followed, and a dereference of such a pointer says nothing. ADR-0040 let a load reach what was stored for what a call reaches, and left the report untouched. So `*tab = p; free(p); q = *tab; return *q;` built with exit 0, a use after free the report never saw ([#256](https://github.com/itsakeyfut/safec/issues/256)). So did:

- the same with `release(tab)` between the load and the read;
- `*tab = p; release(*tab); return **tab;`, which AddressSanitizer reports as a heap use after free.

`Known::inside` records what each allocation may contain. It is a may-set kept per allocation, not per slot, and a store this check cannot place exposes what it carries instead of recording it there. So what it says about a load is a lower bound on what the load may be, and not all of it.

## Decision Drivers

* A use after free that builds is row 6 of the ranking. A report about a read that was safe is row 4.
* A proof about a set that may be missing members is a proof this check is not entitled to (ADR-0020, ADR-0024).
* `free(q)` of a loaded pointer is refused today, as a doubt about a pointer this check stopped following. Giving `q` sites must not turn that into silence or into a proof.

## Considered Options

* **Leave a load holding no site**, as now.
* **Let a load hold what `inside` records, as an ordinary may-set.**
* **Let a load hold what `inside` records, marked as possibly incomplete.**

## Decision Outcome

Chosen option: **a load holds what `inside` records, marked as possibly incomplete**.

- **The load.** A pointer read through one dereference of a local that holds sites, and is not lost, holds every site those allocations may contain, at an offset nobody said. A local that is itself a load is followed too, and so is a place of several dereferences, `q = **t3`, one level per dereference, so a chain of loads is followed however long, and however the read is written: what it holds is a lower bound read beside the marker below, and since what an allocation may contain only grows, the chain reads a finite set. Following it cost nothing measured ([#282](https://github.com/itsakeyfut/safec/issues/282), [#288](https://github.com/itsakeyfut/safec/issues/288)). A load moved by arithmetic, `t3[i][i]` or `**t3 + i`, holds what it held at an offset nobody said, as a named pointer moved by arithmetic does under ADR-0036, and is the pointer operand of that arithmetic under ADR-0030 ([#290](https://github.com/itsakeyfut/safec/issues/290)).
- **The report.** It reads those sites, with a marker saying the set may be missing members. A place of dereferences handed to a call or returned, `release(*tab)` or `return *tab;`, is asked what its deepest level holds in the same way, so that it is answered as the same load through a local is ([#284](https://github.com/itsakeyfut/safec/issues/284)). A dereference of a load whose sites are freed or unknown is reported as unproven, never proved. A dereference through several levels, `**tab` or `***t3`, is asked about each level in the order C reads them, the table's own sites first, and a deeper level only when every level above it says nothing. Asking the second level instead of the first turned a proved `free(tab); return **tab;` into silence.
- **The free.** A free of a loaded pointer still answers that this check stopped following it, beside the sites. It stays the doubt it was, and the transfer is ADR-0029's for a free of something this check cannot name: the sites become unknown rather than freed, and a site already proved freed stays so. Writing `Freed` there made `free(q); return *p;` a proof where `q` was null.
- **A call handed a load.** A site a load may hold that is already proved freed is not blanked by an opaque call it is handed, since the call is not known to have been handed that allocation. What an argument names outright is blanked as before. Blanking it turned a proved use after free in a hatch, where a doubt is only listed, into a build.

The two rejected options:

- Holding no site leaves the use after free silent.
- An ordinary may-set would let a free of a load write a proof onto sites the load may not name. It would also let a dereference prove a use after free about an allocation the pointer may not hold, since what `inside` records is a lower bound.

A load out of what a parameter points at is outside this rule. The caller stored it, so `inside` records nothing and the load holds no site. That is [#281](https://github.com/itsakeyfut/safec/issues/281).

### Confirmation

Every mutation below was applied on its own to `crates/safec-ir/src/memory.rs`, the whole workspace was run with `--no-fail-fast`, and the file was restored. Each list is every test that failed. The cases are in `crates/safec/tests/cases`.

**The load.**

- **Holding no site again**, by dropping what `Allocations::read_through` holds from `Known::stored_below`, fails `a_use_after_free_through_a_pointer_read_out_of_memory_is_reported`, `a_use_after_free_through_a_chain_of_two_loads_is_reported`, `a_use_after_free_through_a_chain_written_as_one_place_is_reported`, `a_use_after_free_through_a_chain_of_three_loads_written_as_one_place_is_reported`, `a_read_two_levels_down_through_a_load_after_a_free_is_reported`, `a_pointer_read_out_of_memory_is_unproven_after_a_call_that_reaches_it`, `a_pointer_read_out_of_memory_after_a_free_is_not_proved_freed_when_returned`, `a_pointer_read_out_of_memory_after_a_free_is_not_proved_freed_when_handed_on`, `a_free_of_a_pointer_read_out_of_memory_proves_nothing_about_what_it_holds`, `a_live_slot_of_a_table_with_a_freed_one_is_doubted` and `a_slot_stored_again_after_what_it_held_was_freed_is_doubted`.
- **Given sites for a source of one dereference only**, as before #288, fails `a_use_after_free_through_a_chain_written_as_one_place_is_reported` and `a_use_after_free_through_a_chain_of_three_loads_written_as_one_place_is_reported`, which go silent, and **for no more than two** fails the second alone. **Given sites whatever its projection**, so that a place with an `Index` counts its dereferences too, changes no case: an array of pointers is refused as `SC0304`, measured. And `a_pointer_read_two_levels_down_holds_nothing_from_the_level_above` holds that `q = **t3` reads what one level down holds and not the level above: **`Known::stored_below` stopping after one level** fails it, `a_use_after_free_through_a_chain_written_as_one_place_is_reported`, `a_use_after_free_through_a_chain_of_three_loads_written_as_one_place_is_reported`, `a_read_three_levels_down_after_a_free_is_reported` and `a_read_four_levels_down_after_a_free_is_reported`.
- **`Known::stored_in` not reading through a load**, by restoring `loaded` to its test, fails `a_use_after_free_through_a_chain_of_two_loads_is_reported` and `a_read_two_levels_down_through_a_load_after_a_free_is_reported`, which go silent.
- **`Known::reached_below` answering nothing for a load** fails `a_read_two_levels_down_through_a_load_after_a_free_is_reported` alone, which goes silent.

**The marker.**

- **Not pushed for a load** in `Known::reached_by` fails `a_use_after_free_through_a_pointer_read_out_of_memory_is_reported`, `a_use_after_free_through_a_chain_of_two_loads_is_reported`, `a_use_after_free_through_a_chain_written_as_one_place_is_reported`, `a_use_after_free_through_a_chain_of_three_loads_written_as_one_place_is_reported`, `a_pointer_read_out_of_memory_after_a_free_is_not_proved_freed_when_returned`, `a_pointer_read_out_of_memory_after_a_free_is_not_proved_freed_when_handed_on` and `a_proved_use_after_free_in_a_hatch_stays_proved_after_a_load_is_handed_to_a_call`.
- **Not pushed for a level below the first** in `Known::reached_below` fails `a_read_two_levels_down_after_a_free_is_not_proved`, `a_read_two_levels_down_through_a_load_after_a_free_is_reported`, `a_read_three_levels_down_after_a_free_is_reported`, `a_read_three_levels_down_through_a_freed_middle_level_is_reported`, `a_read_four_levels_down_after_a_free_is_reported` and `a_pointer_read_two_levels_down_holds_nothing_from_the_level_above`.
- **`verdict` settling with it present** fails the thirteen cases the two rows above name.
- **Counted as a doubt**, as `Reached::Lost` is, fails `a_live_pointer_read_out_of_memory_is_read_in_silence`, `a_live_value_read_through_a_chain_of_two_loads_is_read_in_silence` and `a_live_value_read_three_levels_down_is_read_in_silence`, the reported cases of the chains above, and six earlier cases that build or stay silent today, among them `a_list_built_and_appended_to_in_one_function_is_refused` and `a_table_read_out_of_a_holder_into_a_local_is_reached_through_it`.

**The read several levels down.**

- **`used` asking no level below the first** fails `a_read_two_levels_down_after_its_allocation_was_released_is_reported`, `a_read_two_levels_down_after_a_free_is_not_proved`, `a_read_two_levels_down_through_a_load_after_a_free_is_reported`, `a_read_three_levels_down_after_a_free_is_reported`, `a_read_three_levels_down_through_a_freed_middle_level_is_reported`, `a_read_four_levels_down_after_a_free_is_reported` and `a_pointer_read_two_levels_down_holds_nothing_from_the_level_above`. **Asking no level past the second** fails `a_read_three_levels_down_after_a_free_is_reported` and `a_read_four_levels_down_after_a_free_is_reported`, **asking no level past the third** fails `a_read_four_levels_down_after_a_free_is_reported` alone, and **asking the deepest level only** fails `a_read_three_levels_down_through_a_freed_middle_level_is_reported` alone. The order among the levels below the first is not visible in what is printed, measured: each is unproven and says the same words.
- **`used` skipping the table's own sites** for a place of more than one dereference fails `a_read_two_levels_down_through_a_freed_table_is_proved` alone, which goes silent.

**The load moved by arithmetic.**

- **`built_from` holding no sites for a load** fails `a_load_moved_by_a_subscript_after_a_free_is_unproven`, `a_load_moved_by_arithmetic_and_read_after_a_free_is_unproven` and `a_load_moved_by_arithmetic_and_handed_on_after_a_free_is_unproven`, which go silent about the free; **reading it one level down whatever its depth** fails the second and third.
- **The integers beside a load still contributing** changes no case: an integer holding an allocation takes `int i = p;`, refused as `SC0302`, measured.
- `a_live_load_moved_by_a_subscript_is_read_in_silence` is among those **the marker counted as a doubt** fails.

**The free and the call.**

- **`Allocations::touching` not answering `Reached::Lost` for a load** fails `a_free_of_a_pointer_read_out_of_memory_stays_a_doubt` and `a_free_of_a_pointer_read_out_of_memory_proves_nothing_about_what_it_holds`.
- **`holds_something_unnameable` not answering for a load** fails `a_free_of_a_pointer_read_out_of_memory_proves_nothing_about_what_it_holds`, which becomes a proof, and `a_proved_use_after_free_in_a_hatch_stays_proved_after_a_free_of_a_load`, which builds.
- **An opaque call blanking every site it reaches**, or `named_outright` counting a load, fails `a_proved_use_after_free_in_a_hatch_stays_proved_after_a_load_is_handed_to_a_call` alone, which builds.

`a_live_slot_of_a_table_with_a_freed_one_is_doubted` and `a_slot_stored_again_after_what_it_held_was_freed_is_doubted` hold the cost below. Both are reported as unproven today and move when slots are told apart.

### Consequences

* Good, because a use after free through a pointer this check saw stored in the function's own memory, and read into a local, is reported, with or without a call between.
* Bad, because `inside` does not tell slots apart and a store adds to it rather than replacing what was there. `tab[0] = p; tab[1] = r; free(p); q = tab[1]; *q` is reported as unproven though `q` is `r`, and so is `*tab = p; free(p); *tab = malloc(4); q = *tab; *q`. Telling slots apart is a question for how fields and indices are lowered in Phase 9, and the case that holds this answer moves then.
* Bad, because a load out of a parameter's memory is still silent after a call, which is #281.
* Bad, because a load out of an allocation a store this check cannot place has written into holds only what was recorded there, so `memcpy(tab, src, 8); free(r); q = *tab; *q` is silent when what was recorded is live. That is [#283](https://github.com/itsakeyfut/safec/issues/283).
* Bad, because what a chain reads is what was recorded where each level was stored, and a store written as one place, `**t3 = r`, is exposed rather than recorded. So `**t3 = r; free(r); return ***t3;` is silent where the same store through a local is reported. That is [#283](https://github.com/itsakeyfut/safec/issues/283), the same gap as a store `memcpy` makes.
* Bad, because a free of a load written as one place, `free(*t3)`, frees none of what the load holds; it is reported as a free of a pointer this check stopped following, where `m = *t3; free(m);` also reports the read through `t3` after it.
* What would reverse this: slots told apart, so that a load names exactly what was stored where it read, and a record of every store, so that the set is complete and a proof is earned.

## More Information

[#256](https://github.com/itsakeyfut/safec/issues/256) carries the programs and the design. ADR-0017, ADR-0020, ADR-0024, ADR-0029 and ADR-0040 are what this narrows or extends.
