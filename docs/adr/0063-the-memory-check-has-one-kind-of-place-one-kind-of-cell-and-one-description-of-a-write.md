---
status: "proposed"
date: 2026-11-03
decision-makers: itsakeyfut
---

# The memory check has one kind of place, one kind of cell and one description of a write

## Context and Problem Statement

Three architecture reviews in three days (2026-11-01, 11-02, 11-03) scored the
memory check 2 of 5, and the third found every lens at 2. `fix(memory)` issues
rose to 39 in W44 and 23 in the first two days of W45, the longest chain of
fixes is ten links, and 13 of 417 issues were found by a measurement rather
than by review. The five lenses reached one shape from five sides:

* **Two kinds of place.** An allocation site and a local share one `usize`
  index, so most questions are asked twice (`sites`/`writes_to`,
  `inside`/`inside_locals`, `block_init`/`local_init`), and swapping the two
  compiles.
* **Two kinds of cell.** A local holds a `Held` (21 fields), a block a
  `Contents` (7), translated by hand at every store and load; each `Held` mark
  without a twin was a route patched one at a time (#705, #706, #503, #715).
* **Four descriptions of a write.** `Store`, `Landing`, `Pours` and `Wrote`;
  the unwritten belief works out its own landing, and a library copy rebuilds a
  load by hand and drops a mark the load sets.
* **Doors start where they prove.** `Held::none` is the join's identity and
  where five doors start, and a may-write door calls the certain
  `Known::written`.
* **What this check cannot name is seven marks**, asked by four predicates
  with different members.

It is `proposed` while #772 is built, stage by stage.

## Decision Drivers

* **A forgotten route fails higher on the list**: a build error, then a named
  test, then a doubt the reader sees, before ever a silence.
* **Each stage leaves the tree green** and is measured against `tests/cases`
  and `tests/routes` before and after, so a regression is read one case at a
  time.
* **Nothing is removed on the strength of a design alone**: where a step's
  premise is a measurement not yet taken, the step starts by taking it.

## Considered Options

* **Rebuild toward one place type, one cell type and one write description,
  in stages, each a pull request.**
* Keep fixing one route at a time.
* Rebuild in one pull request.

## Decision Outcome

Chosen option: **rebuild in stages**, in this order, the design of each in
#772.

0. **The net first.** The route matrix (ADR-0062) is made to fail on the
   mutations it now passes (#770) before any step that rewrites how a write
   is applied.
1. **Moves the compiler holds.** The library table at crate level with every
   reader of a callee's kind exhaustive (#773); the facts about a function at
   crate level and the report reading the walk without feeding it (#774);
   `Site` and `LocalIdx` as types (#775); one exhaustive answer for whether a
   value may hold what this check did not follow (#776), which a later
   representation re-implements without its readers changing.
2. **One description of a write.** A library copy is a store of a load
   (#777); the unwritten belief applies the write's `Landing`, and a certain
   write is a value only a landing proving one place can make (#778); every
   door builds a value naming every mark (#732).
3. **One representation.** One set of places, `Loc = Heap(Site) |
   Local(LocalIdx)` (#779); a spike asks whether what cannot be named is a
   summary place (#781); a block holds a `Held` (#780) once the spike says
   whether the list-walk doubts it removes come back. Summary places join
   `Loc` only if the spike holds, so that a failed spike leaves #779 standing.
4. **Beliefs, each its own pull request.** Freshness asked at every depth
   (#768); the opaque callee's write stated as a boundary belief and shown by
   a matrix row (#782); a loop may run zero times unless a first turn is
   shown, superseding that part of ADR-0057 (#783).
5. **Records.** The records written as logs are superseded by short decisions
   once stage 3 has fixed what they describe (#784).

### Confirmation

Each stage's pull request names, for the steps it lands, the mutation that
fails a named test; until then this record is held by its plan alone. The
redesign is done when the next architecture review measures, against
2026-11-03, the fixes per week under `memory/`, the share of new
`fix(memory)` issues found by the matrix, and the longest open chain.

### Consequences

* Good, because a mark added to a value, a kind added to a callee, or a door
  added to a write is a build error at every place that must answer it.
* Bad, because stages 2 and 3 rewrite the two functions that took most fixes,
  and a branch lost in the move is a silence; the net of stage 0 is what makes
  that a named failure instead.
* Bad, because a call this check cannot read stays believed to write what it
  is handed, now said where the boundary beliefs are said, by the user's
  decision; inverting it refused 17 correct programs, measured.

## Pros and Cons of the Options

### Keep fixing one route at a time

* Good, because each change is small.
* Bad, because the reviews showed the fixes are the same few shapes one route
  over, and their rate rose rather than fell.

### Rebuild in one pull request

* Good, because the design is applied once.
* Bad, because hundreds of corpus and matrix cases move in one diff, and a
  regression among them cannot be read; a stage that fails stops everything.

## More Information

* Issue #772 and its design comment; the architecture review of 2026-11-03
  (https://claude.ai/artifact/FE7gYXrdKiN2txNxmxMCnh).
* ADR-0062 for the route matrix the stages are measured against.
