---
status: "proposed"
date: 2026-11-11
decision-makers: itsakeyfut
---

# A width and a null are one value per level

## Context and Problem Statement

How wide a pointer is believed to point is held in several places, each
with its own spelling of "nothing known":

- `Held::believed` (an `Option<u64>` whose `None` means three things);
- `Held::contents_at_most` (with `None` as identity, the reverse of
  `believed`);
- `Known::handed_at_most` and `handed_contents_at_most`;
- the constant `AS_TYPED`;
- `Cell::unrecorded`.

A null is never said. It is inferred from every other mark being absent
(`Held::names_nothing`). So a pointer null on one path only names what the
other path names after a join, and a `realloc` of it is read as copying an
old object in silence (#708, row 6). The width at a load is answered by two
copies of one rule that differ (#804).

## Decision Drivers

* A null that survives a join, so that a door can ask it.
* One spelling of "no width recorded" that absorbs, and one of "nothing
  stored" that is the identity, so that no join confuses the two.

## Considered Options

* One value per level, with a may-be-null value in it.
* The nullability check's answer read during the memory walk, replacing
  ADR-0027's split.

## Decision Outcome

Chosen: one value per level, by the author.

1. **A level's width is one value**: `Unbounded` (the identity), `AtMost(n)`,
   `AsTyped`, and `Nothing` (absorbing, today's `unrecorded`). `believed` is
   level 0 of it, kept apart where it means "ask the recorded width
   instead".
2. **The two handed bounds are one**, the first at level 0.
3. **A pointer that may be null says so.**
   - A value carries `may_be_null`, set by a null and kept by a join (a
     union).
   - `names_nothing` and the `(bool, bool)` argument to `Held::joined` go.
   - `Known::names_nothing`'s reading of an indeterminate value as null is
     kept until its own reader asks otherwise.
4. **One function reads a width at a load**, called by the transfer and by
   `Known` alike (#804). A `void *` read out of memory is believed as wide as
   its cell records (ADR-0061), which answers #631 where the record is exact.
5. **A `realloc` of a pointer that may be null reads no old object on that
   path** (#708).

ADR-0061 already made "nothing recorded" believe nothing. So a mark this
change misses leaves a doubt the reader sees (row 4), not a belief.

### Confirmation

To be filled by the change for #918: the join-law tests of the value, and
the cases of #708 and #631.

### Consequences

* Good, because #804, #734, #708 and #631 are one change, and a null joins
  like every other fact.
* Bad, because every reader of `believed` and `contents_at_most` changes
  type, about 135 references.

## Pros and Cons of the Options

### Read the nullability check's answer during the walk

* Good, because no new mark.
* Bad, because the memory walk would depend on another analysis's order,
  which ADR-0027 keeps apart.

## More Information

* [ADR-0061](./0061-a-pointer-read-out-of-memory-is-believed-only-as-wide-as-was-recorded.md),
  which makes a missed mark a doubt.
* [ADR-0027](./0027-an-unsafe-conclusion-is-about-an-execution-this-function-has.md),
  whose split this keeps.
