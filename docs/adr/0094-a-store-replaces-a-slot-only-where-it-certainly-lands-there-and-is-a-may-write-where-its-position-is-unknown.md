---
status: "proposed"
date: 2026-11-11
decision-makers: itsakeyfut
---

# A store replaces a slot only where it certainly lands there, and is a may-write where its position is unknown

## Context and Problem Statement

What a block holds is a union that nothing replaces (`Cell::absorb`),
because the slots of a block were not told apart. So two cases are
answered wrongly, in opposite directions:

- `free(*b); *b = 0;` still believes the freed pointer is in the slot, and
  the program is doubted (#656, row 4).
- A store at an element this check cannot name writes the whole block
  (ADR-0081 rule 2), and a block written at one element is believed written
  everywhere (#893, row 6).

ADR-0092 tells the slots apart.

## Decision Drivers

* A replacement is a claim that the old value is gone. Made where it is not
  certain, it is a silence (row 6).
* A site stands for every block one allocation call makes. Inside a loop,
  that is several blocks.

## Considered Options

* Replace a slot where the store certainly lands in it, and treat a store at
  an unknown position as a may-write.
* Never replace, and keep the may-write alone.

## Decision Outcome

Chosen: replace where certain, by the author.

1. **A store replaces a slot's contents and its written record** where all
   of these hold:
   - it lands in exactly one site;
   - at a known `At::Bytes` covering the whole slot;
   - and the site certainly stands for one block: `Allocations::made_once`,
     and not a parameter's site, an opaque call's result, or a `realloc`'s
     result.

   `Known::contents` is kept per slot for this.
2. **Anywhere else a store adds to what the slot may hold**, as today.
3. **A store at an unknown position is a may-write** of every slot it may
   reach: it makes no slot written that was not. This replaces ADR-0081
   rule 2, and retires `Held::array_edge` and `Known::stored_in_part`, whose
   only job was to make some such stores may-writes.
4. **Precision for a loop that fills a block comes back only through a
   proof.** It does not come back through the store believing itself whole.
   Until such a proof exists, a block filled in a loop is doubted where it is
   read, a doubt the reader sees.

### Confirmation

To be filled by the change for #918:

- a route-matrix row for "certainly one block", whose mutation (`made_once`
  answered `true` for a site in a loop) fails it by name;
- the cases of #656 and #893.

### Consequences

* Good, because a slot overwritten for certain is no longer doubted, and an
  element nothing named is no longer believed written.
* Bad, because a block filled in a loop is doubted until a loop proof is
  written. The prototype on #893 measured three cases moving and no exit
  code.

## Pros and Cons of the Options

### Never replace

* Good, because no claim of certainty is made.
* Bad, because #656's false report stays.

## More Information

* [ADR-0081](./0081-a-heap-block-of-pointers-of-a-constant-size-is-written-per-element-as-a-local-array-is.md),
  whose rule 2 this replaces.
* [ADR-0092](./0092-a-position-inside-an-object-is-one-byte-offset-and-a-places-written-record-is-kept-per-pointer-sized-slot.md),
  which tells the slots apart.
