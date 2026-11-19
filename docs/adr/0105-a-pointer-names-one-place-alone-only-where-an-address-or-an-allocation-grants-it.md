---
status: "accepted"
date: 2026-11-19
decision-makers: itsakeyfut
---

# A pointer names one place alone only where an address or an allocation grants it, superseding ADR-0103 and ADR-0104

## Context and Problem Statement

ADR-0101 rule 1 has a call this check cannot read write for certain only a
local some pointer it can find names alone, and ADR-0104 the same of a block
an argument names. "Alone" was read off the may-set: a value naming one local
and nothing else. A local nothing assigned holds the empty set, and a null is
the empty set beside a mark, so a join of either with `{&a}` read as naming
`a` alone. Each door that left the set small while handing nothing over was
found one at a time and closed with an exclusion: a null (#950), an element
(#951), a holder unassigned on a path (#1028), a block written on one path or
made by a loop (#1036), a fill (#1037), a block holding a null (#1038) or what
this check does not follow (#938), and a block reached through a pointer that
may be null (#1046). ADR-0103 gathered the first of these into rules, and the
hunt and the architecture review of 2026-11-19 found five more doors still
open (#1049, #1050 in two forms, #1052, and an element of a block).

## Decision Drivers

* "Alone" is a must-question, and a may-set cannot answer it without an
  exclusion for every way the set can be small and incomplete.
* A fact granted in few places fails towards a doubt where a grant is
  forgotten, and towards a silence only where a door forgets to drop it.
* The output-parameter idiom through a local, `int *p; get(&p);`, must stay
  believed (#782).

## Considered Options

* A must-fact on a value, granted by taking an address and by an allocation.
* Null and "nothing assigned" as members of the may-set, so a join keeps them.
* Keep an exclusion per door.

## Decision Outcome

Chosen option: "A must-fact on a value", by the owner's decision.
`Held::alone` is the one place a value certainly names:

1. **Granted** only by taking the address of a whole local or of a one-level
   array's start (`&arr[k]` and `&m[0][0]` grant nothing), and by an
   allocation whose site stands for one block (`Allocations::made_once`).
   Nothing else a call returns grants it.
2. **Kept** by a copy, and by a join only where both sides agree, which drops
   it at a join with a null, a path that assigned nothing, another place, or
   a may-write.
3. **Dropped** by every other operation: a load, arithmetic, a fill, a call's
   result other than an allocation's, an assignment of anything else. A null
   test grants nothing.
4. `Known::named_alone` and `Known::believed_written_only_where_named_alone`
   ask this fact and nothing else, so their exclusions are gone.

Members of the set were rejected because they remove only the null and
unassigned exclusions, and the element, the one-block site and the write
state would still need theirs. Granting at a null test was rejected because
it reads "alone" off the may-set again, at one place.

This supersedes ADR-0103 and ADR-0104. ADR-0101 rule 1 stands, with "named
alone" meaning this fact.

### Confirmation

* Keeping one side's fact at a join silences
  `a_call_handed_a_pointer_to_a_local_or_null_may_have_missed_the_local`.
* Arithmetic keeping its pointer operand's fact silences
  `a_local_a_pointer_one_past_it_names_is_not_believed_written_by_a_call` and
  `a_block_a_call_was_handed_an_element_of_is_doubted_where_read_at_another`.
* A fill keeping the fact silences
  `a_local_a_pointer_zeroed_by_a_fill_named_is_not_believed_written_by_a_call_handed_it`.

No case holds the address's projection condition, nor an allocation's two
conditions (its callee allocates, and its site stands for one block),
measured when this was written: the lowering takes an address only of a whole
local or at index zero, an opaque call's block reads as written by code this
check cannot read either way, and a block a loop makes again is one this
check stops following by the next turn. Each is kept because it can only add
a doubt.

The cases that held ADR-0103 and ADR-0104 pass unchanged, measured when this
was written. Those that held a rule about a block holder are held, until
#1061, by a block's record naming nothing alone.

### Consequences

* Good, because a door added later that hands a call a pointer it has not
  shown to name one place is a doubt without a rule for it.
* Bad, because the output-parameter idiom through a block, `*box = &a;
  fill(box);`, and through `*pp`, is doubted until a block's record carries
  the fact (#1061): three corpus cases and two route rows
  (`fresh_call_d2_both`, `fresh_call_d2_one`) moved to a doubt. Two of
  those cases held mutations of the walk that finds what a call reaches;
  other cases still fail under each, measured.
* Bad, because `if (p) fill(p);` after a join with a null, and a pointer
  moved off a local and back, are doubted.
* What would reverse this: #1061 restores the block idiom; a measurement
  showing the doubts refuse correct programs the corpus relies on would
  reopen the null test.

## More Information

ADR-0101, ADR-0103, ADR-0104, #1056, #1060, #1061.
