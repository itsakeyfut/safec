---
status: "accepted"
date: 2026-11-17
decision-makers: itsakeyfut
---

# A library copy exposes what it carries where it lands, as a store does, replacing what ADR-0066 rule 6 says of what it is handed

## Context and Problem Statement

ADR-0066 rule 6 has `Callee::ReturnsFirst` and `Callee::Copies` "free
nothing, return their first argument, and expose what they are handed".
ADR-0039 gave the reason, that they copy bytes and a pointer is bytes. A
store exposes less: what it carries, and only where it lands where this check
cannot place or name it, and it tells the reads still pending what it put
into memory such code already reaches (ADR-0044).

So one write answered two ways. `p = malloc(4); memset(p, 0, 4); forget();
return *p;` was reported as a use after a free nothing could have made:
`memset`, `memcpy` and `memmove` write the bytes their clauses name and do
nothing else (C17 7.24.6.1, 7.24.2.1, 7.24.2.2), and `p` reached nothing a
call can free except through the copy. Experiment #1013 placed about thirty
four of the doubts a copy raised beyond the direct store `*d = v;` in this
exposure.

## Decision Drivers

* A copy and a store over the same program should answer alike where C makes
  them alike (#992).
* No report may be lost at a point whose execution is undefined.

## Considered Options

* Keep ADR-0066 rule 6: a library copy exposes everything it is handed.
* A library copy exposes what it carries where it lands, as a store does.
* A library copy exposes nothing.

## Decision Outcome

Chosen: a library copy exposes what it carries where it lands, as a store
does, by the owner's decision on #1014.

1. **What a library copy carries is exposed where it lands where this check
   cannot place or name it**, the store's rule (ADR-0044): `Pours::Copy`
   carries the sites the copy reads, and `land_contents` exposes them through
   the helper the store's arm calls, `Allocations::expose_carried`.
2. **And the reads still pending are told what it put into memory code this
   check cannot read already reaches**, as a store's are, through
   `Allocations::noticed_where_reachable`.
3. **It exposes nothing else it is handed.** ADR-0066 rule 6's "and expose
   what they are handed" is replaced by rules 1 and 2. The rest of rule 6
   stands: they free nothing and return their first argument.

### Confirmation

Measured on the #1014 branch by applying each change and restoring it:

* The copy exposing everything it is handed again, in `Wrote::Copy`, fails
  `what_memset_was_handed_is_not_freed_by_a_later_call` and
  `what_memset_is_handed_out_of_a_table_is_not_freed_by_a_later_call`, which
  report `SC0402`, and many cases besides, since the exposure also believes
  every exposed block written.
* `Pours::Copy` exposing nothing fails the route rows
  `returned_memcpy_d2_both`, `returned_memcpy_d2_one`,
  `returned_memmove_d2_both`, `returned_memmove_d2_one`,
  `returned_memcpy_block_d2_both` and `returned_memcpy_block_d2_one`.
* The copy's arm not telling the pending reads fails
  `a_pointer_copied_into_a_parameters_memory_is_asked_of_a_read_beside_a_later_call`
  alone: a copy of a pointer into a parameter's pointee, beside a later call.
  It went silent while this was being implemented, before rule 2 was written.
* **Held by nothing**: eleven cases that held how an exposure is ordered,
  joined and closed over used `memset` as a call that exposes without freeing;
  they report nothing now, and the mutations they named are #1016's to hold
  again, listed there with which are still held by other cases.

### Consequences

* Good, because eighteen reports in fifteen corpus cases, each false at the
  point it names, go, and ten route rows of `COPY_DIFFERS` leave it, every row
  in which the copy doubted more than its store twin among them.
* Bad, because no spelling is left that is a call, exposes, and frees
  nothing, so the ordering of an exposure is held by fewer cases (#1016).

## Pros and Cons of the Options

### Keep ADR-0066 rule 6

* Good, because the ordering cases keep their exposure.
* Bad, because a later call is taken to free what only a library copy saw.

### A library copy exposes nothing

* Bad, because a copy of a pointer into memory such code reaches, a
  parameter's pointee, hides it from a later call: measured, the case in the
  Confirmation goes silent, and the six `returned_*_d2_*` rows move.

## More Information

#1013 (where the copy and the store differ), #1014 (the issue), #1016 (the
guards left without a case). See ADR-0044, ADR-0066 and ADR-0077.
