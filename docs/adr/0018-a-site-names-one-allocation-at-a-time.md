---
status: "accepted"
date: 2026-09-14
decision-makers: itsakeyfut
---

# A site names one allocation at a time, and the locals that held the last one say so

## Context and Problem Statement

An allocation site in this check is a local: `Known::points_to` is square in the
locals, and the site a call's allocation is filed under is the local the call
writes into. A loop that allocates every turn therefore hands **one** site to one
allocation after another, and `Allocations::terminator` resets that site to
`SiteState::Live` on each turn. The comment there is right about why it resets
rather than joining: the previous turn's `Freed` is about a different allocation,
and carrying it across would report a double free for code that allocates each
time round.

What it did not account for is that the site is a **shared name**. A local that
copied out of the call on an earlier turn still points at the site, and the reset
hands it a proof about an allocation it is not holding. The result is worse than
a silence, because it is both directions at once:

```c
while (c) {
    p = malloc(8);
    if (saved) { free(keep); }   /* the real double free: nothing is said */
    keep = p;
    saved = 1;
    free(p);                     /* correct, and this is what was reported */
    c = c - 1;
}
```

Before this record, the only diagnostic on that function was a
`warning[SC0401]` on the `free(p)` that is correct, and the `free(keep)` that is
a genuine double free produced nothing at all. The same shape with a *read*
through `keep` is silent even at `--safety strict --deny-unknown`, which is the
bottom row of `CLAUDE.md`'s list.

## Decision Drivers

* [`docs/safety-model.md`](../safety-model.md#safe-unsafe-unknown) puts silence
  at the bottom, and this is a silence that also spends a caret on innocent code.
* The check has to stay quiet on `while (c) { p = malloc(8); free(p); }`. A
  repair that reports the commonest loop in C is not a repair.
* Three more lattices will be written against the same `Analysis` trait, and
  each will have a value that can stop being nameable. What is settled here is
  what they will copy.

## Considered Options

* **Mark the local.** The reborn site is taken away from every other holder,
  and each of them records that it holds something this check can no longer
  name.
* **Mark the site.** Start the new allocation at `SiteState::Unknown` whenever
  another local still points at the site.
* **Give each allocation its own site.** Stop naming a site after the local and
  number the allocations instead.

## Decision Outcome

Chosen option: **mark the local**.

At the reset, every local other than the destination is told the site is gone:
`Held::lose` takes the bit away and sets the row's `lost` flag. `reached_by`
answers `Reached::Lost` for a local whose row is `lost`, which is the vocabulary
that already existed for exactly this and had no producer until now.

That is a fact about one local, so it lives in the local's slot and is read at
the report, which is the rule
[ADR-0017](./0017-record-each-half-of-an-escape-where-its-subject-lives.md)
settled. This record is the second instance of it and does not restate it.

**A row of `points_to` is a type, not a bit vector.** The two halves travel
together through every path a local's contents take: a copy writes both,
arithmetic unions both, anything else clears both. Kept as a parallel `Vec<bool>`
they have to be held in step by hand in four arms of `Allocations::element`, and
the fifth arm somebody adds later is a silence rather than a build error.

**An empty set answers `Reached::Lost` here, unlike the escape.** The two are
different facts and ADR-0017's empty-set rule does not reach this one. A local
that never had a site is an indeterminate pointer, a defect on an axis this
compiler has no check for. A local that *had* a site and lost the name for it is
what `Reached::Lost` is for.

`Analysis::height` gains one monotone bit per local.

### Confirmation

Each mutation below was applied to `crates/safec-ir/src/memory.rs` on its own,
the whole workspace suite was run with `--no-fail-fast`, and the named test
failed and nothing else did.

| Mutation | Named test that fails |
|---|---|
| the reset does not call `Held::lose` on the site's other holders | `a_pointer_saved_across_a_loop_that_allocates_again`, `a_copy_of_a_local_that_lost_its_allocation_lost_it_too`, `a_double_free_across_a_loop_names_the_free_that_is_wrong` and `a_free_of_the_previous_turns_pointer_leaves_the_new_one_proved` |
| `Held::lose` sets `lost` and keeps the bit | `a_free_of_the_previous_turns_pointer_leaves_the_new_one_proved`, whose last `free` gains a warning because the old holder's free wrote `Freed` onto the new allocation's site, and `a_double_free_across_a_loop_names_the_free_that_is_wrong` |
| `Held::clear` leaves `lost` set | `a_local_given_something_fresh_forgets_what_it_lost` |
| `Known::reached_by` does not read `lost` | `a_pointer_saved_across_a_loop_that_allocates_again` and `a_copy_of_a_local_that_lost_its_allocation_lost_it_too` |
| `Held::union` does not union `lost` | the same two, which is the back edge rather than the assignment |
| **the rejected option in place of this one**, marking the site rather than the local | all six, and `a_loop_that_allocates_and_frees_each_turn_is_proved` is the one that matters: it gains a `warning[SC0401]` on a loop with no defect in it |

A field added to `Held` is `error[E0063]` in `Held::none` and `error[E0027]` in
`Held::clear` and `Held::union`, which both destructure it. That is what holds
the two halves together rather than a test.

**`Analysis::height` is not held by anything, and this record says so rather
than implying otherwise.** Leaving the number at what it was before the new bit
leaves the whole suite passing. That is the state
[ADR-0016](./0016-an-analysis-is-a-trait-and-an-unreached-block-has-no-value.md)
already describes for this method, and what a too-low answer costs is a panic
naming it.

### Consequences

* Good, because a double free across a loop is now reported on the `free` that
  is wrong rather than on the one that is right.
* Good, because the check stays silent on a loop that allocates and frees each
  turn. That program was checked against every option before one was chosen.
* Bad, because a local that held a reused site is unprovable from that point
  until it is given something fresh, whatever it actually holds. The loop above
  reports its read as a suspicion, not as the proof it could be.
* Bad, because the number of allocations a loop makes is still invisible. This
  records that one site may have named several allocations; it does not let the
  check say anything about how many or in what order.
* What would reverse this: naming a site after something other than a local, so
  that two allocations from one call are two sites. That is a bigger change than
  it looks, because the sites are what makes the value's height finite, and an
  unbounded supply of them turns a wrong answer into a build that does not stop.

## Pros and Cons of the Options

### Mark the local

* Good, because it is the honest answer: the local does hold something, and
  what is gone is this check's name for it.
* Good, because it costs nothing on a loop whose allocation does not outlive
  its turn, which is most of them.
* Bad, because it adds a field to the lattice value and so a term to
  `Analysis::height`, which nothing checks.

### Mark the site

* Good, because it is eight lines and adds no field.
* Bad, because it puts a `warning[SC0401]` on
  `while (c) { p = malloc(8); free(p); }`. The local a call is copied into
  still points at the previous turn's site when the call is reached, so the
  condition is true on the plainest loop there is. Measured on a working
  implementation, and the whole suite passed it.

### Give each allocation its own site

* Good, because it is the only option that can tell two allocations from one
  call apart, which is what the second Bad above gives up.
* Bad, because a loop makes allocations without bound while the lattice value
  has to stay finite. The failure is a build that does not terminate, which is
  row 5 of `CLAUDE.md`'s list and the one row above the worst.

## More Information

* `Held`, `Known::reached_by` and the reset in `Allocations::terminator` in
  [`crates/safec-ir/src/memory.rs`](../../crates/safec-ir/src/memory.rs).
* [`docs/diagnostics.md`](../diagnostics.md) carries what this means for a
  reader of `SC0401` and `SC0402`.
* Issue #168 has the measurements taken before the design was chosen.
