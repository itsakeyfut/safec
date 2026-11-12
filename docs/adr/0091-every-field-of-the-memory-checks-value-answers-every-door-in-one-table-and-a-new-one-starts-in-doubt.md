---
status: "accepted"
date: 2026-11-11
decision-makers: itsakeyfut
---

# Every field of the memory check's value answers every door in one table, and a new one starts in doubt

## Context and Problem Statement

The memory check's value, `Known`, grew from 20 to 26 fields in ten days.
Each field has to say what happens to it at every door:

- a store, a library copy and a `realloc`;
- a load;
- the forward join and the back edge;
- a site made again, and a local's lifetime beginning and ending.

Nothing lists those answers per field. A field that leaves one out is
silent, and review finds the gap one pull request at a time. Over that
period:

- the longest chain of issues each found by the review of the one before
  reached eleven links (687 to 908);
- none of 226 such issues was found by a failing test.

Eight architecture reviews in a row scored the check's change dynamics and
soundness strategy 2.

The route matrix (ADR-0062) exists to turn a missed route into a failing
named row. It has ten beliefs against 26 fields.

## Decision Drivers

* A field's missing answer at a door has been a silence (row 6). It should
  fail to compile (row 1) or fail a named test (row 2).
* Inverting every default so that a missed route is a doubt was considered
  and not taken now. Its cost in false reports is unmeasured, and it would
  move most of the corpus at once.

## Considered Options

* One exhaustive table of every field's answer per door, plus a route-matrix
  row per belief, plus a rule that a new field starts in doubt.
* Invert every field's default to the value that doubts, and win precision
  back by proofs.
* Leave the shape alone and fix routes as they are found.

## Decision Outcome

Chosen: the table, the matrix rows and the rule for new fields, by the
author.

1. **Each door destructures `Known` with no `..`** and says, per field, what
   it does with it. The doors are a copy, a `realloc`, a load, the forward
   join and the back edge (one body), a site made again, the function's
   entry, and a local's lifetime, its end and its beginning. A field added
   without its answer at every door is `error[E0027]` at each, and
   `error[E0063]` at the entry, which builds the value. Where a
   door's body acts on the fields it binds, as the join, the rebirth and the
   lifetime do, the table is what runs; where it acts through helpers, as a
   load and a copy do, each answer names the helper. What a field is keyed
   by is its type's to say, `Table<Loc, _>`, `Table<Site, _>` or
   `Table<LocalIdx, _>`.

   One destructure per door, and not one function every door calls, by the
   author's decision on #918: one function over seven doors would rewrite
   the join, the rebirth and the transfer at once, for fields that #918's
   slot record changes again.
2. **Every belief a field carries has a route-matrix row**, or is named in
   the matrix as an exemption with its reason. A test fails when a field of
   `Known` is in neither. A new field lands with its row in the same pull
   request.
3. **A new field starts at the value that doubts.** Where it starts quiet,
   its doc comment says why and names the matrix row that holds the quiet
   start. A field kept from before this record keeps its start until its own
   change asks.
4. **A decision about what a value holds beyond its sites reads a question
   that matches every mark** (`Beyond`), not a reader of one mark (#853). A
   new mark is then `error[E0027]` wherever a decision is made.

### Confirmation

Each measured on the #918 branch by applying the change and restoring it:

* **Rule 1.** A field added to `Known` (`pub(super) zz: bool`) is
  `error[E0027]` at seven doors and `error[E0063]` at the entry, which builds
  the value: the load (`Allocations::read_through`), the copy and `realloc`
  arms of the transfer, the join and back edge, `Known::reborn`,
  `Known::clear` and `Known::storage_live`.
* **Rule 2.** `every_field_of_the_memory_checks_value_is_a_belief_or_an_exemption`
  in `tests/routes.rs` fails, naming the field, when a field's row is dropped
  from `KNOWN_FIELDS` (measured on `held_levels`'s). The beliefs added to the
  matrix fail their rows by name: `Allocations::copied_pointers` answering
  `Whole` for every length fails 63 rows and 72 corpus cases.
* **Rule 3** is held by nothing but review. The registry makes a new field
  name a belief or an exemption, and does not say whether its start doubts.
* **Rule 4.** A mark added to `Unrecorded` is `error[E0027]` at eleven
  matches over its marks, every question `Beyond` answers among them, and
  `error[E0063]` where it is built (#853).

### Consequences

* Good, because a field's missing door is a compile error or a failing row
  rather than a review finding.
* Bad, because every field now carries a row in the table and one in the
  matrix, and the matrix needs a hazard program and a control per belief.

## Pros and Cons of the Options

### Invert every default

* Good, because a route nobody listed would be a doubt the reader sees.
* Bad, because its cost in false reports is unknown. In #698, an inversion
  of one rule failed 21 correct cases, and this would move most of the
  corpus at once. If this record does not move the architecture review's
  scores, that is the next step, measured first by an experiment issue.

### Fix routes as they are found

* Good, because nothing moves.
* Bad, because it is what eight reviews scored 2.

## More Information

* [ADR-0062](./0062-each-belief-s-routes-are-generated-as-a-matrix-and-asked-against-a-control.md),
  the route matrix this extends.
* [ADR-0063](./0063-the-memory-check-has-one-kind-of-place-one-kind-of-cell-and-one-description-of-a-write.md),
  whose one kind of place the table keys by.
