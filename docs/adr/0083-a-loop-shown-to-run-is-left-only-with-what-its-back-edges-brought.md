---
status: "accepted"
date: 2026-11-10
decision-makers: itsakeyfut
---

# A loop shown to run is left only with what its back edges brought

## Context and Problem Statement

A loop head joins what arrives from before the loop with what its back edges
bring, and every edge out of the head carries that join. For a loop that may
run zero times that is right: the path in is a path out. For a loop
`memory::runs::shown_to_run` proves runs its body and leaves only by its head's
test after a whole turn (ADR-0069 rule 6), the path in is no path out, and
joining it in carried facts no execution has past the loop.

`init` already answers such a loop's back edge as written (ADR-0069 rule 5).
Points-to did not: `for (int j = 0; j < 2; j++) { b = 0; }` before a `return
b;` left the site in `b` through the zero-turn path, and the block was doubted
where it is returned, though `b` is null wherever it is (#874).

## Considered Options

* Accept the false positive as a stated limit.
* Answer the `return` from nullability's established null, which joins the
  head the same way and does not establish `b` null after the loop, measured
  on a prototype.
* Let an analysis say, per head, that its loop is left only after a whole
  turn, and send along the head's edges out of the loop the head's transfer
  of what its back edges brought alone.
* The same, for every analysis.

## Decision Outcome

Chosen option: the third, by the author's decision on #871.

`Analysis::leaves_after_a_turn` defaults to `false`; the memory check answers
`runs_once` for the head. At such a head, not one a first-turn walk names,
`dataflow::solved` runs the head's elements and terminator over the join of
what its back edges last sent and sends that along each edge whose target is
outside the loop `Cfg::loops` lists for the head, through `Analysis::edge` as
any edge; until a back edge has sent, the exit sends nothing. A back edge into
such a head walks the head again when what it sends moves, since the head's
own value, which still holds the path in, may not. The body still sees the
path in on every walk, and nullability keeps the join.

### Confirmation

Each in `tests/cases/memory/`, with its mutation in its note in
`tests/cases.rs`:

* `a_block_nulled_by_a_loop_shown_to_run_before_the_one_that_returns_it_is_not_doubted`
  is doubted when no head answers `true`.
* `a_block_nulled_by_a_loop_that_may_not_run_before_the_one_that_returns_it_is_doubted`
  goes silent when every head answers `true`.

### Consequences

* Good, because every field of the memory check leaves such a loop as `init`
  already did, by one rule in the solver rather than one per field.
* Bad, because a wrong `shown_to_run` proof, already a silence for `init`, is
  now one for every field of the memory check. ADR-0069 rule 6 keeps the proof
  narrow for that reason.
* Neutral, because `a_heap_block_written_before_an_inner_loop_makes_it_again_is_doubted_where_read_after`
  moves from "may be read" to "is read before anything was assigned": the
  inner loop, shown to run, is left only after a turn that made the block
  again, which nothing wrote.
* Bad, because that case no longer holds what ADR-0081's Confirmation names
  it for: with the per-element back edge of `join_back_beside` disabled
  outright it still reports, found by review, and no respelling tried, a
  loop that may run zero times or a read inside either loop among them,
  tells the rule apart. #886 is to find a guard or show the rule redundant.
* Bad, because `a_block_nulled_by_a_loop_inside_the_one_that_returns_it_is_doubted`
  stays a false positive: its loop may run zero times, so the path that
  skips it is a path out, and on that path the block was written only under
  the `d` the return is also under. Seeing the two agree is a correlation
  this check does not keep.
* Good, because the first of the two loops ADR-0080 names as its cost, a
  loop shown to run that nulls the pointer, is no longer one; the second
  stays, for the reason above.

## More Information

* ADR-0069 rules 5 and 6 are what this rests on, and are unchanged.
* #871 records the decision and the prototype that ruled out nullability's
  answer.
