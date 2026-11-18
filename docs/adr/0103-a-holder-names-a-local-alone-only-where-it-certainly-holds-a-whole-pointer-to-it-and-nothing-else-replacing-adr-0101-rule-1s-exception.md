---
status: "accepted"
date: 2026-11-18
decision-makers: itsakeyfut
---

# A holder names a local alone only where it certainly holds a whole pointer to it and nothing else, replacing ADR-0101 rule 1's exception

## Context and Problem Statement

ADR-0101 rule 1 has a call this check cannot read write for certain only a
local some pointer it can find names alone. Which holders name a local alone
was decided route by route: a pointer that may be null does not (#950), a local
holder nothing may have assigned on some path does not (#1028), and a block
that may hold a null still does, written into the rule as an exception (#938).

The hunt before closing #941 found three more holders counted as naming a local
alone while on some path they hand no address of it over, each a read of an
indeterminate pointer in silence on `main` since before #941:

* a block a store wrote on one path only (#1036): the write state that would
  say so is asked of a local holder and not of a block, and asked after the
  call has exposed the block, which marks it written;
* a pointer a fill of zeros wrote (#1037): the fill notes that it wrote no
  pointer and pours no value, so the holder still names what it named;
* a block whose address of the local was overwritten by a certain null (#1038):
  a block records no null, and a store into one a store does not replace
  leaves its earlier names.

And #938: a block that may hold the local's address or what this check does
not follow counts as naming the local alone.

## Decision Drivers

* A holder the rule counts wrongly is a silence ([`docs/safety-model.md`](../safety-model.md)).
* One answer to "does this holder hand the local over on every path", asked of
  every holder alike, instead of one spelling at a time.

## Considered Options

* Fix each spelling where it was found.
* One rule for every holder, a local, a value handed, or a block, keeping the
  belief that a block naming the local beside what this check does not follow
  names it alone.
* One rule for every holder, without that belief.

## Decision Outcome

Chosen: the last, by the owner's decisions on #941.

1. **A holder names a local alone only where, before the call, it certainly
   holds a whole pointer** (its write state `Init::Written(Whole::Pointer)`),
   **names that local and no other storage, may not be null, and names nothing
   this check does not model.** Asked of a local holder, of a value the call is
   handed, and of a block alike, and of the write state as it was before the
   call exposed what it reaches; and of a block only where its site stands for
   one block, made on no cycle of the graph, since the write state and the
   record are kept per site. Found by review of #1036.
2. **A block records whether a null may have been stored in it**, and one that
   may hold a null names nothing alone. This replaces ADR-0101 rule 1's
   exception for a block that may hold a null.
3. **The belief that a block naming a local beside what this check does not
   follow names it alone is given up** (#938). Two corpus programs that relied
   on it, a local array stored into memory a call returned or a parameter points
   at and then written by a later call, are doubted where they read it, and
   renamed to say so.

### Confirmation

Settled as #1036, #1037, #1038 and #938 land: each rule's mutation and the case
it fails are named here then.

### Consequences

* Good, because every holder is asked one question, so a holder spelled a new
  way is a doubt rather than a silence.
* Bad, because the output-parameter idiom through memory this check does not
  follow, `*slot = m; tick();`, is doubted where it was believed.

## More Information

#941 (the parent), #1036, #1037, #1038, #938, and the hunt recorded with them.
See ADR-0101.
