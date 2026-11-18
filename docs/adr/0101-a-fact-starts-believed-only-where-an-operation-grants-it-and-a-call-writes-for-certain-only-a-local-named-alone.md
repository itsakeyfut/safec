---
status: "accepted"
date: 2026-11-18
decision-makers: itsakeyfut
---

# A fact starts believed only where an operation grants it, and a call writes for certain only a local named alone

## Context and Problem Statement

`tests/routes.rs` lists every field of `Known` and `Held` with where it starts:
in doubt, or believed (`Start::Quiet`) behind a route-matrix belief or a reason.
Eighteen start believed with a reason and no belief. And one belief that is not
a start, that a call this check cannot read writes for certain every local it
may reach, is held the other way round from ADR-0074's rule for a store or a
copy: it writes every local except those `Known::reached_only_beside_another`
finds reached only beside another place, so a local reached by a route that
function does not walk is believed written. #1000 measured six past fixes that
each added such a route.

#1000 also measured that a field's start is the wrong unit: a field with no
operation that grants its safe value, started in doubt, refuses its whole
class. The question per fact is what grants it.

## Decision Drivers

* A precise start no belief exercises silences whatever route reaches it
  unseen ([`docs/safety-model.md`](../safety-model.md)).
* Starting a fact in doubt that nothing can grant refuses every program that
  needs it, which is a report nobody can act on.

## Considered Options

* For the call: believe it writes every local except those found beside
  another (as before); believe it writes none (#1000's I-B, 14 deliberate
  quiets refused on the output-parameter idiom); believe it writes only a
  local some value it is handed or finds names alone.
* For the fields: invert all eighteen and measure; keep each with its reason;
  name a granting operation or a belief per field, and start the rest in
  doubt.

## Decision Outcome

Chosen, by the owner's decisions on #941:

1. **A call this check cannot read writes for certain only a local some value
   it is handed or finds names alone**, the output-parameter idiom; it may have
   written every other local it may reach. This is ADR-0074's rule for a store
   and a copy, applied to the call. Measured on a prototype: no report moves
   over the corpus and the route matrix, and a local a call may have missed,
   then reached by a later call through nothing that names it, is `SC0411`
   where it was believed written (#1028). A pointer that may be null, or a
   holder nothing may have assigned on some path, names no local alone,
   since on that path it hands no address over; a block that may hold a null
   still counts as naming what it holds alone, since a block records no null
   (#938).
2. **A field starts believed only where an operation grants its safe value or
   a route-matrix belief exercises it**; one with neither starts in doubt, and
   the reports that moves are listed, a correct program refused being put to
   the owner. A field no report reads is bookkeeping, held so by a test
   (#1029).
3. **`docs/safety-model.md` carries the table** of where each field starts and
   what grants it, and a test fails where it and `tests/routes.rs` differ
   (#1030).

### Confirmation

Rule 1, measured on #1028's branch by applying each change and restoring
it:

* A call believing every local not named beside another written, as before,
  silences the `SC0411` of
  `a_local_a_call_may_have_missed_is_not_believed_written_by_a_later_call`
  and `a_local_kept_on_one_path_is_not_believed_written_by_a_later_call`.
* A call believing every local it may write written fails those two and the
  six `a_call_handed_*` and `a_local_array_handed_*` cases that hold each
  route a pointer may name another place by.
* A holder nothing may have assigned on some path counted as naming its local
  alone silences
  `a_local_reached_through_a_holder_assigned_on_one_path_is_not_believed_written_by_a_call`.

Rules 2 and 3 are settled as #1029 and #1030 land.

### Consequences

* Good, because a route this check does not walk lands in doubt, which the
  reader sees, rather than in silence.
* Bad, because a field nothing can grant and nothing reads into a report must
  be shown so, which is work a reason in prose did not ask for.

## More Information

#941 (the parent), #1000 (the measurement), #1028, #1029, #1030. See ADR-0074,
which this extends to the call.
