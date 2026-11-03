---
status: "proposed"
date: 2026-11-03
decision-makers: itsakeyfut
---

# A pointer read out of memory is believed only as wide as was recorded there

## Context and Problem Statement

ADR-0060 believes a pointer read out of memory to point at one whole object of
the type it is read as, no wider than what the memory is recorded to hold. Where
nothing is recorded, nothing narrows the type: `Allocations::believed_read`
answers `bound.map_or(as_read, ...)`, and `Known::memory_levels` gives a local
neither loaded nor written by a call no bound. So every route that reaches
memory without recording a width is believed whole, and seventeen fixes from
#622 to #724 were such a route found by review after it was silent: #622, #623,
#624, #627, #629, #632, #634, #636, #675, #677, #679, #690, #711, #713, #715,
#717, #719.

This record replaces ADR-0060's belief about a load and nothing else. A
parameter, a call's result and what a call writes into memory it was handed are
still believed as the type they arrive as, and a pointer that leaves is still
asked as ADR-0060 says. It is `proposed` while #735 is built.

## Decision Drivers

* **A forgotten width is a doubt, not a belief.** The failure this project
  exists to avoid is saying safe without having read it, and a route that
  records nothing has read nothing.
* **The common case still proves.** `*box = &q; int *t = *box; *t = 1;` is
  correct and has to stay silent.
* **What arrives is still believed.** Nothing here reads a caller or a callee,
  and ADR-0017 and ADR-0060 believe what arrives from them.

## Considered Options

* **Record the width of what a store names, then believe a load no wider than
  what was recorded, and nothing where nothing was.**
* Believe nothing where nothing was recorded, without recording more.
* Record `0` at each writer that cannot say a width, and keep believing the
  type where nothing was recorded.

## Decision Outcome

Chosen option: **record, then believe no wider than the record.**

**What a store names is recorded at its width.** A value that names sites or
locals, at its start and not inside a local, is recorded at the least of their
widths: each site's `Extent` lower bound, and each local at its size. A value
off its start or inside a local records `0`. A value naming nothing, a null,
records nothing. The store into memory records it in `Contents::stored_at_most`,
and a local reached through its address on the way of a read gives it from its
own value in `Known::memory_levels`.

**What arrives is recorded as the type it arrives as.** A local or a site a
call this check cannot read was handed is believed written as it was handed,
and where it was handed nothing that bounds a width, as the type it is read as.

**A load believes no wider than what its memory records, and nothing where
nothing is recorded**: `believed_read` answers `0` where no level is bounded.
**Except a read already reported as indeterminate** (ADR-0057), which is
believed as its type, since the defect is reported once, where it is read.

### Confirmation

To be filled by #735 with the cases and the mutation each holds: believe the
type again where nothing is recorded, and the case named for it goes silent;
record nothing for a store of a local's address, and `*box = &q; int *t = *box;
*t = 1;` is reported.

### Consequences

* Good, because a route that reaches memory with no record at all is a doubt.
  Measured on a prototype at `0586f663`: reversing six past width fixes one at
  a time left 61 cases silent on `main` and 33 on the prototype.
* Bad, because **a record is a bound and is never replaced**, so a route that
  forgets its width after another store recorded one is still believed by that
  record. The 33 are that shape, and this record does not close them.
* Bad, because memory nothing was stored into is believed nothing too:
  `calloc`'s nulls read back and tested are doubted, a false `Unknown`, by the
  user's decision; and a value lost with no site named is doubted.

## Pros and Cons of the Options

### Believe nothing where nothing was recorded, without recording more

* Good, because it is one line.
* Bad, because no store of a local's address or an allocation records a width
  today, so the prototype moved 237 cases and newly reported 22 correct ones,
  `*box = &q; int *t = *box; *t = 1;` among them.

### Record `0` at each writer that cannot say a width

* Good, because a fresh or `calloc` block is believed as before.
* Bad, because a writer nobody has written yet records nothing and is believed
  whole, which is the shape of all seventeen.

## More Information

* `crates/safec-ir/src/memory/transfer.rs`: `Allocations::believed_read`, the
  store arm that records `Contents::stored_at_most`.
* `crates/safec-ir/src/memory/known.rs`: `Known::memory_levels`.
* Issue #735 and its design comment.
