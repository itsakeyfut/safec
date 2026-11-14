---
status: "accepted"
date: 2026-11-14
decision-makers: itsakeyfut
---

# A library copy writes a local for certain only where its length is the local's size, replacing what ADR-0073 rule 2 says of a length it cannot read

## Context and Problem Statement

ADR-0073 rule 2 has a store at an index this check cannot read, and a
library copy of a length it cannot read, write a local whole, "so a loop
that fills it is not doubted". ADR-0094 rule 3 has since made the store a
may-write, and rule 4 left a loop's precision to a proof. The copy kept the
old answer: in `Allocations::initialised_by`, a copy or a fill that lands in
one local alone wrote it for certain whatever its length.

A length this check cannot read may be zero, so the local may be as it was.
`int *p; memcpy(&p, &src, n); return p;` returned a pointer nothing may have
written, and `memset(&q, 0, n)` and `memset(v, 0, n)` for `int *v[1]` did the
same, with nothing said at the read (#942). An `SC0412` at the call kept such
a program from building, but that is a report about the call's reach, and it
says nothing about the read.

## Decision Drivers

* A certain write is a claim that what was there before is gone. Made where
  it is not certain, it is a silence, and silence about safety is the worst
  thing this compiler can do.
* One length should get one answer whether it is written by a store or by a
  copy, so that the two routes cannot drift apart.

## Considered Options

* A copy or a fill writes its one local for certain only where its length is
  a constant equal to the local's size, the width `Allocations::replaces_whole`
  asks of a store. Any other length is a may-write.
* Keep ADR-0073 rule 2's copy clause and let the call's `SC0412` stand in for
  the read.

## Decision Outcome

Chosen: the width asked of a store, by the author (#942's design, step 5 of
#937).

1. **A library copy or fill writes the one local it lands in for certain
   only where its length is a constant equal to that local's size.** Asked by
   `Allocations::covers_exactly`, the same function `replaces_whole` asks for a
   store, so the two answer one length alike.
2. **A length this check cannot read, or a shorter one, is a may-write** of
   that local, as every other local the copy may land in already is. A longer
   one is the bounds rule's to report, as before.
3. **Its length is the one the row says it writes from its destination's
   start**: the argument `reaches` gives the first argument,
   `Library::written_length`, and not the third argument whatever the row.
   `strncat`'s third caps what it appends after the string already there
   (C17 7.24.3.2), so it names none, and `strncat(&r, &src, 8)` does not
   write `r` whole. Found by review.
4. ADR-0073 rule 2's sentence "a copy of a length it cannot read, still write
   it whole" is replaced by rule 1 here. The rest of ADR-0073 rule 2 stands,
   and its store clause had already been replaced by ADR-0094 rule 3.

### Confirmation

Measured on the #942 branch by applying each change and restoring it:

* `Allocations::covers_exactly` answering `true` for a length it cannot read
  fails `a_local_a_library_copy_or_fill_of_a_length_not_read_wrote_is_doubted`,
  whose three `SC0411` go. No other case or route row moves.
* A copy never writing its one local for certain fails seven named cases,
  `a_char_buffer_a_copy_of_its_whole_length_wrote_read_as_a_pointer_is_not_doubted`
  among them, and 34 route rows, so a copy of exactly the local's size still
  writes it.
* `Allocations::written_bytes` answering the third argument for every row
  again fails the same case, whose `SC0411` at `return r` after `strncat`
  goes.
* **Held by nothing**: a shorter constant length counted as covering the
  local. `built::write_effects` already leaves a local written in part
  possibly unwritten (ADR-0073 rule 2), so a copy or a fill of 4 bytes into a
  pointer local, through a local pointer or one read back out of memory, and
  of 8 into an array of two, printed the same with and without that half,
  measured.

### Consequences

* Good, because a copy or a fill of a length this check cannot read is
  doubted at the read, as a store at an index it cannot read already is.
* Bad, because a loop that copies into a local piece by piece, at lengths
  this check cannot read, is doubted where it is read. No case in the corpus
  moved, measured.
* What would reverse this: a proof that the length is the local's size, from
  a test on the path or a range for the length.

## Pros and Cons of the Options

### Keep the copy clause

* Good, because nothing more is doubted.
* Bad, because the call's `SC0412` is an unproven result, so where unproven
  results are allowed the program builds and the read is silent; and it
  answers one length differently from a store.

## More Information

* [ADR-0073](./0073-what-leaves-a-function-is-asked-by-how-it-was-declared-and-what-wrote-it.md),
  whose rule 2 copy clause this replaces.
* [ADR-0094](./0094-a-store-replaces-a-slot-only-where-it-certainly-lands-there-and-is-a-may-write-where-its-position-is-unknown.md),
  whose rule 3 is the store's answer this matches.
