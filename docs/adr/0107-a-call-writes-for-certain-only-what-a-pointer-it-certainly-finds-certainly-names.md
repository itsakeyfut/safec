---
status: "accepted"
date: 2026-11-19
decision-makers: itsakeyfut
---

# A call writes for certain only what a pointer it certainly finds certainly names

## Context and Problem Statement

ADR-0101 rule 1 has a call this check cannot read write for certain only a
local some value it is handed or finds names alone, and ADR-0105 made "names
alone" a must-fact. The holders it "finds" were every local the walk
`Known::unknown_code_reaches` reaches, and that walk runs after the call has
exposed what it was handed, following every pointer that may lead somewhere.
So a holder that certainly names a local, behind a pointer that may be null,
counted as found: `p` either `&h` or a null, with `h = m`, made `fill(p)` write
`m` for certain, and `m[0]` was read indeterminate in silence (#1063), on main
since before ADR-0105.

## Decision Drivers

* "Writes for certain" needs both halves certain: the pointer is found, and it
  names the place.
* The output-parameter idiom through a holder an earlier call kept,
  `stash(&p); p = m; tick();` (#782), must stay believed.

## Considered Options

* A holder counts where a handed value certainly names it, followed as a
  chain, or where it was exposed before this call.
* A holder counts where it was exposed before this call, `Known::exposed`
  taken as found.
* Keep every holder the walk reaches.

## Decision Outcome

Chosen option: "A handed value's chain, or a holder an earlier call certainly
kept", by the owner's decision on #1056 and, for the second half, on review of
#1063.

`Known::named_alone` names a local alone where:

* a value the call is handed certainly names it;
* or a local already named alone certainly holds it, as a chain: `fill(&h)`
  with `h = m` names `h` and `m`;
* or a holder an earlier call certainly kept certainly names it.
  `Known::certainly_exposed` says which locals that is: set after a call for
  every local it named alone, and joined by intersection.

A holder this call exposes for the first time is found only through the
chain. `Known::exposed` would not do for the second half: it is a union, and
the walk that sets it is a may-walk, so `if (c) keep(&h); h = m; tick();`
found `h` where `c` is zero, which review showed was silent on main too.

### Confirmation

* Asking every holder the walk reaches, as before, silences
  `a_local_a_holder_a_call_may_reach_only_through_a_possibly_null_pointer_names_is_not_believed_written`,
  `a_local_a_holder_an_earlier_call_may_have_kept_names_is_not_believed_written_by_a_later_call`,
  `a_local_a_holder_assigned_after_an_earlier_call_may_have_kept_it_names_is_not_believed_written_by_a_later_call`
  and `a_local_array_put_in_a_local_a_call_kept_on_one_arm_is_doubted_where_read`.
* Joining `Known::certainly_exposed` by union silences
  `a_local_array_put_in_a_local_a_call_kept_on_one_arm_is_doubted_where_read`.
* Following no chain gives
  `a_local_array_reached_through_a_local_a_call_was_handed_is_not_reported_as_indeterminate`
  an `SC0411`.

One other case moved, measured when this was written: a call that kept the
local on one arm only was believed to have handed it to the later call, and
`m[0]` is now doubted there; the belief is given up.

### Consequences

* Good, because what a call may find no longer counts as what it certainly
  finds.
* Bad, because `Known` gains a field, a must-fact beside the may-fact
  `exposed`, and the output-parameter idiom through a holder an earlier
  call kept on one path only is doubted.
* What would reverse this: the idiom through a holder this call exposes
  itself, refused where a corpus program relies on it.

## More Information

ADR-0101, ADR-0105, #782, #1056, #1063.
