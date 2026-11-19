---
status: "accepted"
date: 2026-11-19
decision-makers: itsakeyfut
---

# A call certainly keeps only what it was certainly handed, narrowing ADR-0107

## Context and Problem Statement

ADR-0107 has a later call find a holder an earlier call certainly kept, and
`Known::certainly_exposed` was set for every local the earlier call named
alone: the whole chain. So `keep(&h)` with `h = &m` marked `h` and `m`. But
`keep` holds `&h`, and reaches `m` only through what `h` holds, which an
assignment can change. After `h = 0`, a later call found `m` as certainly kept,
`m` certainly named `a`, and `a` was believed written with nothing reaching it
(#1069), on main since before ADR-0107.

## Decision Drivers

* What a call certainly keeps is what it was certainly handed, not what that
  pointed at when it was handed.
* The output-parameter idiom through a holder an earlier call kept,
  `stash(&p); p = m; tick();`, must stay believed.

## Considered Options

* Mark only the head of each chain: the locals a handed value certainly
  names.
* Mark the whole chain, as ADR-0107 did.

## Decision Outcome

Chosen option: "Mark only the head of each chain". The design of #1069 under
#1056 settled it, and no option with a measured cost was in question. A later
call that finds the head follows its fact as it stands then, through
`Known::named_alone`'s chain, so the idiom is kept and a holder retargeted
since is not believed.

### Confirmation

* Marking every local `named_alone` returned silences
  `a_local_a_call_reached_through_a_holder_that_changed_since_is_not_believed_written_by_a_later_call`
  and
  `a_local_a_call_reached_through_a_holder_retargeted_on_one_path_is_not_believed_written_by_a_later_call`.

No other case moved, measured when this was written.

### Consequences

* Good, because a fact about what an earlier call keeps no longer outlives
  the pointer that gave it.
* Bad, because nothing measured.
* What would reverse this: an idiom where a call keeps what it reached
  through a holder rather than the holder, which C cannot tell apart from the
  caller's side.

## More Information

ADR-0107, #1056, #1063, #1069.
