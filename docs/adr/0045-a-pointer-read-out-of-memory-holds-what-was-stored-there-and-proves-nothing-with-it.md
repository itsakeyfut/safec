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

- **The load.** A pointer read through one dereference of a local that holds sites, and is not lost, holds every site those allocations may contain, at an offset nobody said. A local that is itself a load is followed too, and so is a place of several dereferences, `q = **t3`, one level per dereference, so a chain of loads through allocations is followed however long, and however the read is written: what it holds is a lower bound read beside the marker below, and since every level is a set of sites, the chain reads a finite set. Following it cost nothing measured ([#282](https://github.com/itsakeyfut/safec/issues/282), [#288](https://github.com/itsakeyfut/safec/issues/288)). A load moved by arithmetic, `t3[i][i]` or `**t3 + i`, holds what it held at an offset nobody said, as a named pointer moved by arithmetic does under ADR-0036, and is the pointer operand of that arithmetic under ADR-0030 ([#290](https://github.com/itsakeyfut/safec/issues/290)).
- **The report.** It reads those sites, with a marker saying the set may be missing members. A place of dereferences handed to a call or returned, `release(*tab)` or `return *tab;`, is asked what its deepest level holds in the same way, so that it is answered as the same load through a local is ([#284](https://github.com/itsakeyfut/safec/issues/284)). So is what it points at one level in, asked of a pointer that said nothing about itself, and what a call it is handed to may now replace: `use2(*k)` was asked neither, and a freed pointer whose address `*k` held built in silence where `q = *k; use2(q);` was refused ([#348](https://github.com/itsakeyfut/safec/issues/348)). A dereference of a load whose sites are freed or unknown is reported as unproven, never proved. A dereference through several levels, `**tab` or `***t3`, is asked about each level in the order C reads them, the table's own sites first, and a deeper level only when every level above it says nothing. Asking the second level instead of the first turned a proved `free(tab); return **tab;` into silence.
- **A copy or a deep store.** What `memcpy` or `memmove` copies is recorded in what the destination may contain (ADR-0039), and a store of more than one dereference, `**t3 = r`, is recorded in what the level above may be, and exposed as well since that set is a lower bound, as ADR-0044 does for a store through a load ([#283](https://github.com/itsakeyfut/safec/issues/283)).
- **A pointer to a local.** A load through a pointer whose edge names a local (ADR-0019), `*t2` after `t2 = &slot`, holds what the local holds, beside the marker, and is lost where the local is, for a load into a local, `**t2`, `*t2` handed on and `*t2` moved by arithmetic alike; a direct read of the escaped local is unproven too, so the spellings agree. The walk that answers what a chain of dereferences reads, `Known::level_below`, steps through a local as through an allocation, at any level: a local's own edges are followed below the first ([#298](https://github.com/itsakeyfut/safec/issues/298)), and an allocation records the locals whose address it may hold, `Known::inside_locals`, so `*t3 = &slot; **t3 = r;` puts `r` in `slot` and `***t3` reads it ([#300](https://github.com/itsakeyfut/safec/issues/300)). A load carries the locals it reaches as edges, so a write through it lands in them. The table is joined, kept at a rebirth, and copied by `realloc` and `memcpy` as `inside` is.
- **A loop's rebirth.** Every allocation one `malloc` in a loop makes is one site, and when the site is handed to a new allocation while the one it named is gone, what other allocations recorded containing it would name the new, live one. Each allocation that held it is marked (`Known::stale`), and a load out of a marked allocation, at any level of its chain, is a pointer this check stopped following: a dereference of it is a doubt, never a proof. Only when the old allocation is gone, since a read of a live one is no use after free; marking whatever its state doubted a loop that keeps last turn's allocation. The entry is kept: what a call handed the container reaches is read off it, and dropping it left what the old allocation held live across a call that may free it. A load out of a marked allocation stored into another marks that one too, carried by `Held::stale_read` across copies, arithmetic and joins; not any lost value, since a list built with nothing freed loses its head every turn, and marking for that doubted every walk of it ([#293](https://github.com/itsakeyfut/safec/issues/293)). A local that loses a site whose allocation is not live carries the same bit, and so does a load through a pointer to such a local and what a call this check cannot read makes lost of the caller's (ADR-0040), so a store of any of them marks where it lands: each was recorded as nothing and read back in silence where read directly it was doubted ([#295](https://github.com/itsakeyfut/safec/issues/295)). One that loses a live allocation carries nothing, for the list's reason.
- **The free.** A free of a loaded pointer still answers that this check stopped following it, beside the sites. It stays the doubt it was, and the transfer is ADR-0029's for a free of something this check cannot name: the sites become unknown rather than freed, and a site already proved freed stays so. Writing `Freed` there made `free(q); return *p;` a proof where `q` was null.
- **A call handed a load.** A site a load may hold that is already proved freed is not blanked by an opaque call it is handed, since the call is not known to have been handed that allocation. What an argument names outright is blanked as before. Blanking it turned a proved use after free in a hatch, where a doubt is only listed, into a build.

The two rejected options:

- Holding no site leaves the use after free silent.
- An ordinary may-set would let a free of a load write a proof onto sites the load may not name. It would also let a dereference prove a use after free about an allocation the pointer may not hold, since what `inside` records is a lower bound.

A load out of what a parameter points at is outside this rule: the caller stored it, so `inside` records nothing and the load holds no site. ADR-0040 answers it instead, making such a load a pointer this check stopped following at a call it cannot read ([#281](https://github.com/itsakeyfut/safec/issues/281)).

### Confirmation

Every mutation below was applied on its own to the memory check (`crates/safec-ir/src/memory.rs` and `memory/`), the whole workspace was run with `--no-fail-fast`, and the file was restored. Each list is every test that failed. The cases are in `crates/safec/tests/cases`.

**The load.**

- **`report::handed` asking only a plain local one level in** silences `a_freed_pointer_whose_address_is_read_out_of_memory_and_handed_on_is_asked`; **asking a place of dereferences whatever reached it**, in `handed_below`'s `Reachable` arm, refuses `a_live_pointer_whose_address_is_read_out_of_memory_and_handed_on_builds`; **keying its report `*k` rather than `**k`** adds a second `SC0407` to `a_place_and_what_it_points_at_handed_to_one_call_are_one_report`. What the call may then replace is ADR-0047's, which hands a place of dereferences nothing.
- **Holding no site again**, by dropping what `Allocations::read_through` holds from `Known::stored_below`, fails `a_use_after_free_through_a_pointer_read_out_of_memory_is_reported`, `a_use_after_free_through_a_chain_of_two_loads_is_reported`, `a_use_after_free_through_a_chain_written_as_one_place_is_reported`, `a_use_after_free_through_a_chain_of_three_loads_written_as_one_place_is_reported`, `a_read_two_levels_down_through_a_load_after_a_free_is_reported`, `a_pointer_read_out_of_memory_is_unproven_after_a_call_that_reaches_it`, `a_pointer_read_out_of_memory_after_a_free_is_not_proved_freed_when_returned`, `a_pointer_read_out_of_memory_after_a_free_is_not_proved_freed_when_handed_on`, `a_free_of_a_pointer_read_out_of_memory_proves_nothing_about_what_it_holds`, `a_live_slot_of_a_table_with_a_freed_one_is_doubted` and `a_slot_stored_again_after_what_it_held_was_freed_is_doubted`.
- **Given sites for a source of one dereference only**, as before #288, fails `a_use_after_free_through_a_chain_written_as_one_place_is_reported` and `a_use_after_free_through_a_chain_of_three_loads_written_as_one_place_is_reported`, which go silent, and **for no more than two** fails the second alone. **Given sites whatever its projection**, so that a place with an `Index` counts its dereferences too, changes no case: an array of pointers is refused as `SC0304`, measured. And `a_pointer_read_two_levels_down_holds_nothing_from_the_level_above` holds that `q = **t3` reads what one level down holds and not the level above: **`Known::stored_below` stopping after one level** fails it, `a_use_after_free_through_a_chain_written_as_one_place_is_reported`, `a_use_after_free_through_a_chain_of_three_loads_written_as_one_place_is_reported`, `a_read_three_levels_down_after_a_free_is_reported` and `a_read_four_levels_down_after_a_free_is_reported`.
- **`Known::level_zero` not reading through a load**, by answering nothing for a `loaded` local, fails `a_use_after_free_through_a_chain_of_two_loads_is_reported`, `a_read_two_levels_down_through_a_load_after_a_free_is_reported`, `a_load_moved_by_a_subscript_after_a_free_is_unproven` and `a_load_through_two_locals_addresses_reads_what_the_last_holds`, which go silent.
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
- **The integers beside a load still contributing** fails `an_integer_beside_a_load_does_not_decide_where_it_points` alone, which reports `SC0402` about what an opaque call returned as an integer.
- `a_live_load_moved_by_a_subscript_is_read_in_silence` is among those **the marker counted as a doubt** fails.

**A loop's rebirth.**

- **`reborn` marking nothing** fails `a_pointer_stored_and_freed_last_turn_is_doubted_after_the_loop_allocates_again`, `a_load_moved_by_arithmetic_and_stored_last_turn_is_doubted_after_the_loop_allocates_again`, `a_pointer_two_levels_down_freed_last_turn_is_doubted_after_the_loop_allocates_again` and `a_pointer_freed_on_one_arm_last_turn_is_doubted_after_the_loop_allocates_again`, which go silent, and the cost's case. **Marking whatever the old allocation's state** fails `a_pointer_kept_alive_last_turn_is_read_in_silence_after_the_loop_allocates_again` and `a_pointer_stored_and_read_in_one_turn_with_nothing_freed_is_silent`, which report.
- **`read_through`**, **`built_from`** and **`reached_below`** each ignoring the mark fail a case named for their reader: the first through a local, the second through arithmetic, the third `a_read_two_levels_down_through_a_pointer_freed_last_turn_is_doubted`. **`stale_below` reading one level** fails the two-level case, and **the join dropping the mark** fails the one-arm case among others.

**A lost value stored.**

- **The rebirth never setting `Held::stale_read`** fails `a_pointer_lost_to_a_free_and_stored_is_doubted_when_read_back`, `a_pointer_lost_to_a_free_read_through_its_address_and_stored_is_doubted_when_read_back`, `a_pointer_lost_to_a_free_read_through_its_address_moved_and_stored_is_doubted_when_read_back` and `a_pointer_lost_to_a_free_read_through_one_of_two_addresses_and_stored_is_doubted_when_read_back`, which go silent; **setting it whether or not the allocation is gone** fails `a_list_built_in_a_loop_with_nothing_freed_is_walked_without_a_doubt`.
- **A load through a lost local carrying nothing**, in `read_through`, fails `a_pointer_lost_to_a_free_read_through_its_address_and_stored_is_doubted_when_read_back` alone, and in `built_from` fails `a_pointer_lost_to_a_free_read_through_its_address_moved_and_stored_is_doubted_when_read_back` alone; **a call this check cannot read not setting it** fails `a_pointer_read_out_of_a_parameter_and_stored_after_a_call_is_doubted_when_read_back` alone; **`Known::stale_through` asking the first local it points at only** fails `a_pointer_lost_to_a_free_read_through_one_of_two_addresses_and_stored_is_doubted_when_read_back` alone.
- **Held by nothing**: `stale_through` reading `lost` rather than `stale_read`. A local read through its address has escaped, so its allocation is unproven and a rebirth of it counts as gone, and one lost to a call writing through its address is doubted at the read by its allocation's state; no program was found that tells the two apart.

**A local's address stored, and a local's own edges.**

- **The store never marking `Known::inside_locals`**, or **`Known::level_below` ignoring it**, fails `a_locals_address_stored_in_an_allocation_is_followed_by_a_read_through_it`, `a_locals_address_read_back_out_of_an_allocation_is_written_through` and `a_write_through_a_locals_address_stored_in_an_allocation_lands_in_the_local`; **a load carrying no edges** fails the second and `a_load_through_two_locals_addresses_reads_what_the_last_holds`; **a store two levels down landing in no local** fails the first and third.
- **`Known::level_below` ignoring a local's own edges** fails `a_load_through_two_locals_addresses_reads_what_the_last_holds` alone.
- **`Known::lost_through` asking the first level only**, as it did before the walk stepped through locals at every level, fails `a_local_this_check_lost_read_two_levels_down_through_a_stored_address_is_doubted` alone, which goes silent where `u = *t3; *u` was doubted.
- **The join keeping only what both arms hold**, **`realloc` copying no row** and **`memcpy` copying no locals** fail `a_locals_address_stored_on_one_arm_is_followed_after_the_join`, `a_locals_address_stored_in_an_allocation_is_followed_after_realloc` and `a_locals_address_copied_by_memcpy_is_followed` respectively.

**A copy or a deep store.**

- **`Callee::Copies` recording nothing**, or `memcpy` mapped back to `Callee::ReturnsFirst`, fails `a_pointer_copied_by_memcpy_into_a_table_and_freed_is_unproven_when_read_back` and `a_pointer_copied_by_memcpy_from_a_locals_address_is_read_back`; **reading only plain locals as its arguments** fails `a_copy_into_a_destination_read_out_of_memory_is_recorded_there` and `a_copy_from_a_source_read_out_of_memory_is_recorded`; **marking nothing** fails `a_copy_out_of_an_allocation_a_loop_allocated_again_stays_doubted`. **The store arm finding no containers below one dereference** fails `a_store_two_levels_down_is_recorded_where_it_lands`, and **such a store not counted as unnamed** fails `a_store_two_levels_down_through_a_set_this_check_may_not_know_whole_is_exposed`, which goes silent after the call.

**A pointer to a local.**

- **`Known::reached_below` ignoring `lost_through`** fails `a_read_two_levels_down_through_a_pointer_to_a_local_this_check_lost_is_doubted` and `a_pointer_to_a_local_this_check_lost_handed_on_through_it_is_doubted`; **`built_from` ignoring it** fails `a_load_moved_by_arithmetic_through_a_pointer_to_a_local_this_check_lost_is_doubted`; **`reached_below` leaving out what the edge's local holds** fails `a_read_two_levels_down_through_a_pointer_to_a_local_after_a_free_is_unproven` and `a_load_through_a_pointer_to_a_local_handed_on_after_a_free_is_unproven`.
- **`Known::level_zero` reading no edge to a local** fails `a_pointer_written_into_a_local_through_its_address_and_read_back_after_a_free_is_unproven`, `a_pointer_to_either_of_two_locals_reads_what_both_hold`, and six more that read through a local's address, which go silent about the free; **`Known::level_below` not stepping into what a local holds** fails the same and the first two cases below that store a local's address; **`read_through` ignoring `Known::lost_through`** fails `a_local_this_check_lost_read_through_its_address_is_doubted` alone; and `a_live_pointer_written_into_a_local_through_its_address_is_read_back_in_silence` is among those the marker counted as a doubt fails.

**The free and the call.**

- **`Allocations::touching` not answering `Reached::Lost` for a load** fails `a_free_of_a_pointer_read_out_of_memory_stays_a_doubt` and `a_free_of_a_pointer_read_out_of_memory_proves_nothing_about_what_it_holds`.
- **`holds_something_unnameable` not answering for a load** fails `a_free_of_a_pointer_read_out_of_memory_proves_nothing_about_what_it_holds`, which becomes a proof, and `a_proved_use_after_free_in_a_hatch_stays_proved_after_a_free_of_a_load`, which builds.
- **An opaque call blanking every site it reaches**, or `named_outright` counting a load, fails `a_proved_use_after_free_in_a_hatch_stays_proved_after_a_load_is_handed_to_a_call` alone, which builds.

`a_live_slot_of_a_table_with_a_freed_one_is_doubted` and `a_slot_stored_again_after_what_it_held_was_freed_is_doubted` hold the cost below. Both are reported as unproven today and move when slots are told apart.

### Consequences

* Good, because a use after free through a pointer this check saw stored in the function's own memory, and read into a local, is reported, with or without a call between.
* Bad, because `inside` does not tell slots apart and a store adds to it rather than replacing what was there. `tab[0] = p; tab[1] = r; free(p); q = tab[1]; *q` is reported as unproven though `q` is `r`, and so is `*tab = p; free(p); *tab = malloc(4); q = *tab; *q`. Telling slots apart is a question for how fields and indices are lowered in Phase 9, and the case that holds this answer moves then.
* Good, because a local's address stored in an allocation is remembered, so a write or a read through the allocation's copy of it reaches the local: `*t3 = &slot; **t3 = r; free(r); ***t3` is reported (#300).
* Bad, because a write that now reaches an escaped local through a stored or loaded address makes that local hold what was written, so the local handed on is a pointer this check stopped following: `t = *bb; *t = a; release(slot);` is `SC0407`, as `t = &slot; *t = a; release(slot);` was already. Measured, every program it moved was refused already.
* Bad, because a copy adds to what its destination may contain and replaces nothing, so a freed pointer the destination held before `memcpy` is still doubted after: `a_copy_over_a_freed_pointer_is_doubted_though_it_replaced_it` holds the answer, and it moves when slots are told apart.
* Good, because a chain through two locals' addresses, `u = *t3; *u` with `t3 = &t2` and `t2 = &slot`, reads what `slot` holds (#298). With the value given to `slot` by assignment after its address escaped, the live program is doubted at the read, as it is read directly; written through the alias it is not.
* Good, because a pointer this check lost to an allocation that may be gone, stored into memory and read back, is doubted as it is read directly (#295). One lost by a call writing through its address is doubted on `main` already, since the allocation it held is left unproven.
* Bad, because a local whose address escaped counts as losing an allocation that may be gone at every rebirth, since the escape leaves the allocation unproven, so stored and read back it is doubted with nothing freed. Read directly it was doubted already, so this moves a doubt from one spelling to the other rather than adding one where the program built.
* Bad, because a local lost to a live allocation that is freed afterwards, `o = p;` and next turn `free(o); *t4 = o; q = *t4; *q`, is not doubted at the read of `q`: the bit is decided where the loss is. The free itself is reported, since `o` is a pointer this check stopped following, so the program does not build in silence.
* Bad, because a pointer to a local moved by an offset this check cannot evaluate, `po[k - 1]`, loses the edge and reads nothing of the lost local. That is [#308](https://github.com/itsakeyfut/safec/issues/308).
* Bad, because what an allocation contains is not told apart by slot, so a store into one does not clear the mark the rebirth below sets. A loop that stores, reads and frees within one turn is doubted on the next: `a_pointer_stored_and_read_in_one_turn_is_doubted_on_the_next` holds the answer, and it moves when slots are told apart.
* Bad, because a free of a load written as one place, `free(*t3)`, frees none of what the load holds; it is reported as a free of a pointer this check stopped following, where `m = *t3; free(m);` also reports the read through `t3` after it.
* What would reverse this: slots told apart, so that a load names exactly what was stored where it read, and a record of every store, so that the set is complete and a proof is earned.

## More Information

[#256](https://github.com/itsakeyfut/safec/issues/256) carries the programs and the design. ADR-0017, ADR-0020, ADR-0024, ADR-0029 and ADR-0040 are what this narrows or extends.
