---
status: "proposed"
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

- **The load.** A pointer read through one dereference of a local that holds sites, and is neither loaded nor lost, holds every site those allocations may contain, at an offset nobody said.
- **The report.** It reads those sites, with a marker saying the set may be missing members. A dereference of a load whose sites are freed or unknown is reported as unproven, never proved. A dereference through two levels, `**tab`, reads the same sites through the table's.
- **The free.** A free of a loaded pointer still answers that this check stopped following it, beside the sites. It stays the doubt it was, and the sites become unknown rather than freed.

The two rejected options:

- Holding no site leaves the use after free silent.
- An ordinary may-set would let a free of a load write a proof onto sites the load may not name. It would also let a dereference prove a use after free about an allocation the pointer may not hold, since what `inside` records is a lower bound.

A load out of what a parameter points at is outside this rule. The caller stored it, so `inside` records nothing and the load holds no site. That is [#281](https://github.com/itsakeyfut/safec/issues/281).

### Confirmation

Not yet held: #256's implementation adds a case for each part and names it here. Planned:

- the load answering no site again silences the call-free program, the one with `release(tab)`, and the two-level read;
- the marker dropped makes a dereference of a freed load a proof;
- the free's doubt dropped turns `free(q)` of a load silent;
- a two-slot table reading the live slot holds today's answer as the cost.

### Consequences

* Good, because a use after free through a pointer stored in the function's own memory is reported, with or without a call between.
* Bad, because `inside` does not tell slots apart. `tab[0] = p; tab[1] = r; free(p); q = tab[1]; *q` is reported as unproven though `q` is `r`. Telling slots apart is a question for how fields and indices are lowered in Phase 9, and the case that holds this answer moves then.
* Bad, because a load out of a parameter's memory is still silent after a call, which is #281.
* What would reverse this: slots told apart, so that a load names exactly what was stored where it read, and a record of every store, so that the set is complete and a proof is earned.

## More Information

[#256](https://github.com/itsakeyfut/safec/issues/256) carries the programs and the design. ADR-0017, ADR-0020, ADR-0024 and ADR-0040 are what this narrows or extends.
