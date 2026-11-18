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

Rule 1, measured on #1036's and #1037's branches: the block branch of
`Known::named_alone` asked with `false`, or asked after the call's exposure,
silences `a_local_a_block_names_on_one_path_only_is_not_believed_written_by_a_call`;
every site taken as one block silences
`a_local_one_of_a_loops_blocks_names_is_not_believed_written_by_a_call_handed_another`;
every handed value counted as a whole pointer silences
`a_local_a_pointer_zeroed_by_a_fill_named_is_not_believed_written_by_a_call_handed_it`;
a zero fill's targets left as they were silences
`a_local_a_copy_of_a_zeroed_pointer_named_is_not_believed_written_by_a_call`.

Rule 2, measured on #1038's branch: `Cell::poured` never marking a null, or
`Cell::absorb` keeping its own, silences
`a_local_a_block_named_before_a_null_overwrote_it_is_not_believed_written_by_a_later_call`
and `a_local_a_block_holding_a_null_on_one_path_names_is_not_believed_written_by_a_call`.
A load takes its null from what it reads, so a copy of a pointer naming its
local alone marks none (`a_pointer_copied_into_a_block_names_its_local_alone_to_a_call`
is doubted when every load may be null) and a copy of one that may be null
does (`a_local_a_possibly_null_pointer_copied_into_a_block_names_is_not_believed_written_by_a_call`
is silent when none may be). Memory a caller handed over is never replaced, so
a null stored there before an address keeps it possibly null:
`a_local_stored_into_caller_memory_after_a_null_is_doubted_after_a_call` is
that accepted cost.

Rule 3 is settled as #938 lands.

### Consequences

* Good, because every holder is asked one question, so a holder spelled a new
  way is a doubt rather than a silence.
* Bad, because the output-parameter idiom through memory this check does not
  follow, `*slot = m; tick();`, is doubted where it was believed.

## More Information

#941 (the parent), #1036, #1037, #1038, #938, and the hunt recorded with them.
See ADR-0101.
