---
status: "accepted"
date: 2026-11-22
decision-makers: itsakeyfut
---

# Read-only storage is carried by what a pointer names and by grants, not by marks that move

## Context and Problem Statement

ADR-0120 to ADR-0122 carry "this storage may not be modified" as a mark: on values (`Held::read_only`), on blocks (`Cell::may_hold_read_only`), and on IR locals (`const_levels`). The four hunts before #615 closed found 23 read-only issues. Each round's fixes moved the mark and opened the next round's holes, alternating between silences and false reports (#1122 → #1134 → #1144/#1145 → #1152 → #1157).

Architecture review 9 traced the chain to three gaps:

- a static is not a referent, so its read-only property can only travel as a mark;
- points-to cannot keep names the program showed apart from names an opaque call is guessed to have, so "certainly names" (ADR-0122) has no type;
- a may-fact's empty value means "certainly not", so every door that forgets the mark is a silence.

On top of these, the leaving doors compute the reader's levels five different ways.

## Decision Drivers

* Phase 11's rule: a construct the check cannot read is `Unknown`, never safe.
* The owner's stop rule after review 9: route fixes on the read-only axis wait for its carriers.
* Every intermediate state between pull requests leans to row 4, not row 6.

## Considered Options

* Four carrier changes in this order: shown and guessed names, a doubting default where arrival is not shown, static storage as a referent, and one reader record per storage.
* The same four, with the default first.
* The same four, with static storage first.
* Keep fixing routes on the current marks.

## Decision Outcome

Chosen option: the first, by the owner's decisions on #1161's design run.

1. **Shown and guessed names (#1163).** `Held` keeps `guessed: Bits<Referent>` beside `names`. An opaque call's result puts the exposed sites it may return into `guessed`. "May name" is the union of the two sets; "certainly names" (ADR-0122) is `names` alone. A guess filed as shown would over-report (row 4). A shown name filed as guessed would under-ask past a reader's type only, which is ADR-0122's accepted route.
2. **Doubt where arrival is not shown (#1164).**
   * Memory a parameter or a call's result points at is marked read-only at the levels its type says `const` again. The mark is on the cell, so a pointer to that memory read any other way also finds it.
   * An opaque call's result takes the read-only levels of every pointer it was handed. A parameter itself still believes its type (ADR-0121).
   * Unplaced loads keep today's answer. Doubting them too was rejected: it moved 221 cases in review 9's prototype.
3. **Static storage as a referent (#1162).** `Referent::Static(StaticId)` and `Storage::Static`. Whether a static is read-only becomes a query on the referent. Every reader of the unknown marks first sees a static as "not recorded", exactly as today, and is opted in rule by rule. A reader that is never opted in stays at row 4.
4. **One reader record per storage (#1165).** `Known` keeps the reader's `const` levels per storage beside `receiver_width`. Every leaving door asks that record, not the lvalue, the temporary or a zero. The IR keeps a local's declared reader levels apart from the levels a temporary carries. The walk past a reader's type is bounded by the deepest pointer in the unit, as ADR-0121 says, not the deepest in the function.

ADR-0120's mark on values stays until #1162 makes it unnecessary for statics. ADR-0121 and ADR-0122 stand, except where this record changes what carries their facts.

### Confirmation

Named per sub-issue as each lands. The programs of #1147, #1148, #1150, #1156, #1157, #1158 and #1159 become `SC0417`. #1145's case stays silent. Each carrier change is guarded by a mutation that silences one of them.

### Consequences

* Good, because each fact has one home, so a new door asks it rather than carrying it.
* Bad, because `Held` grows by one set, and `Referent` by one variant whose index enlarges every per-referent table.
* Bad, because until the last of the four lands, some routes are still answered by the old marks.

## More Information

ADR-0091, ADR-0112, ADR-0120, ADR-0121, ADR-0122, #615, #1161 to #1165.
