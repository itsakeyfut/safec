---
status: "proposed"
date: 2026-11-06
decision-makers: itsakeyfut
---

# A write spelled as a copy or through a loaded pointer is asked at its doors as the store it is, replacing what ADR-0068 says of rules 3, 5 and 6 for it

## Context and Problem Statement

ADR-0068 asks a pointer at the door it leaves by, and its rules name the code
that holds each one: `Known::leaves` answers whether a store leaves (rule 3), a
free of a site stored out is doubted only where it is no double free (rule 5),
and a library copy is asked at its source's and its destination's type
(rule 6). The architecture review of 2026-11-06 found the same write answered
differently at those doors by how it was spelled, on programs:

- `**t = x;` and `int **q = *t; *q = x;` with `t = &p`: the first does not
  leave and the second does, because a load marks what it reads `loaded`
  while `Known::leaves` reads `p`'s value directly; and the transfer's
  `Landing::unnamed` takes the first as unplaced anyway (#802).
- `*pp = x; free(x);` is `SC0414` and `memcpy(pp, &x, 8); free(x);` is
  `SC0401`: the copy takes `&x`, which escapes (ADR-0017), and the double-free
  doubt that escape raises hides the stored-out doubt (#684).
- A copy of four bytes of a pointer is `SC0413` through a typed operand and
  silent between two `void *` operands (#746).
- `*out = a[0];` with `a` unwritten is `SC0411` at the read, and
  `memcpy(out, a, 8);` is silent (#764).
- A function defined here under a library copy's name takes the copy's
  transfer and an ordinary call's doors, and a program leaving the caller a
  freed pointer builds silent (#800).

This record replaces what ADR-0068 says of rules 3, 5 and 6 for these writes,
and nothing else of it.

## Decision Drivers

* A write answers alike at every door however it is spelled, since the doors
  are where a reader learns what this check concluded.
* Where two spellings disagree, the disagreement is settled on the side that
  doubts: a change here removes no report.
* A body this unit defines is what runs, whatever its name (ADR-0068 rule 6),
  and the transfer reads the same answer as the doors.

## Considered Options

* Settle each disagreement on the side that doubts, each in the code every
  door already shares.
* Make the precise side precise everywhere: a load out of a local whose
  address only this function holds marks nothing `loaded`, and a copy's
  source address does not escape.
* Lower a library copy to the store's own code first, and let the
  disagreements fall out of it.

## Decision Outcome

Chosen option: the first, by the user's decisions on #686. The second removes
false reports, but every route that breaks its premise is a silence, and the
copy's escape was tried once under #652 and taken back for that. The third is
not chosen now: whether it is needed is measured once the route matrix asks
the door questions (#801) and these are fixed.

1. **Whether a function is a library function is answered once,**
   `Allocations::library`, which answers nothing for a body this unit
   defines, so `Allocations::callee` answers `Callee::Opaque` for it and the
   transfer and every door read the same kind (#800).
2. **A store leaves where `Known::leaves` says so or its `Landing` is
   unnamed**, and `Landing::unnamed` and `Landing::unplaced` answer a store and
   a copy at one depth alike, by the rule that doubts: a write more than one
   dereference below the pointer it goes through is unnamed, for both kinds
   (#802).
3. **A free of a site stored out is doubted as stored out beside any
   double-free finding**, `FreedAfterStore` with `DoubleFree` on one caret, as
   `SC0401` and `SC0404` already share one (#684).
4. **A copy that writes part of a pointer is asked where what its operands
   point at holds a pointer**, by the declared type of a local they name and
   the record of an allocation they name, where an operand's own type is
   `void *`; an operand that names nothing this check followed asks nothing
   more than today (#746).
5. **A copy into memory that leaves, of bytes that may be unwritten, over a
   slot believed to hold a pointer, is doubted at the copy**, by a `Kind` of
   its own reported under `SC0411` with the words of a copy rather than a
   read: code outside reads that slot as a pointer, and this check reads one
   function at a time (#764).

### Confirmation

Proposed until the last of #800, #802, #684, #746 and #764 lands; each pull
request adds its rule's case and the mutation that fails it here, as ADR-0063
did per stage. Each rule's guard is a case in `tests/cases.rs` that fails
under the reversal its rule names, and the route matrix's door-question
beliefs (#801) list the pairs that still differ until each lands.

1. `Allocations::library` answering by name whether or not the function is
   defined makes
   `a_pointer_a_memcpy_defined_here_leaves_its_caller_and_freed_after_is_doubted`
   and `a_pointer_copied_out_by_a_memcpy_defined_here_is_not_asked_as_a_library_copy`
   read the body as a library copy, and fails both, with
   `a_function_defined_with_a_library_name_is_asked_as_any_function` and
   `a_defined_memcpy_s_destination_is_asked_what_it_held`.

### Consequences

* Good, because a reader is told the same thing about one write whichever
  way it was spelled, and a door added later reads one answer.
* Bad, because the false reports a precise side would remove stay: `**t = x;`
  into memory nothing outside reads is now `SC0414` at a later free, and
  `memcpy(pp, &x, 8); free(x);` keeps its `SC0401` beside the `SC0414`.
* What would reverse this: a points-to representation that says who holds a
  local's address exactly, so that the escape and the `loaded` mark stop
  standing in for it (#810).

## More Information

* [ADR-0068](./0068-a-pointer-is-asked-where-it-leaves-by-a-call-a-return-or-a-store-what-the-other-side-will-believe-of-it.md),
  whose rules 3, 5 and 6 this narrows for these writes.
* [ADR-0017](./0017-record-each-half-of-an-escape-where-its-subject-lives.md),
  the escape rule 3 leaves standing.
