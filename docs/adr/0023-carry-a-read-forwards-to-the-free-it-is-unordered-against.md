---
status: "accepted"
date: 2026-09-18
decision-makers: itsakeyfut
---

# Carry a read forwards to the free it is unordered against

## Context and Problem Statement

[ADR-0022](./0022-say-where-c-sequences-one-evaluation-before-another.md) gave
the IR an `Element::Sequenced` and the memory check the half of the question a
forward walk can use: a free only counts as ordered before what follows it once
a marker has passed. Its own Consequences said the other half was open. A marker
says what *is* ordered, and a walk that only ever looks back can compare a free
with the reads behind it and never with the ones ahead, so the two spellings of
one unsequenced program still answered differently:

```c
int x = g(*p) + (free(p), 0);   /* nothing at all, exit 0 under --deny-unknown */
int x = (free(p), 0) + g(*p);   /* warning[SC0402], exit 1 under --deny-unknown */
```

Both are one program under C17 6.5 p3, which leaves the operands of `+`
unsequenced, and 6.5.2.2 p10, which makes the callee's execution indeterminately
sequenced with everything in the caller that is not otherwise sequenced. Which
order an implementation picks is unspecified, and the answer this compiler gave
turned on which side of the `+` the free was written: not on C, but on the order
ADR-0010 makes the lowering emit. The silence is the failure `docs/safety-model.md`
is written to prevent, arriving on the half of the question ADR-0022 left.

The same asymmetry exists for a call this check cannot read, measured, and it is
issue #184 rather than this record: `h(p) + g(*p)` reports and `g(*p) + h(p)`
does not. What the two have in common is the direction of the walk; what they do
not is how wide the report is. (Issue #184 has since closed by widening the
mechanism below to an opaque callee; the consequence that left it open says what
that cost.)

## Decision Drivers

* **The answer cannot be a proof.** The read and the free are in one full
  expression with nothing sequencing them, so one allowed order reads freed
  storage and another does not. Whatever shape this takes, `Unsafe` is never
  right, which puts a ceiling on what the change can cost: row 4 of
  `CLAUDE.md`'s list, never row 6.
* **The framework answers forwards.** `Analysis::element` is handed no block and
  no position, so a backward transfer cannot ask the forward solution what a
  free's argument reaches. ADR-0016 lists a backward direction as one of the
  three things nothing has asked for, and this is not the thing that asks.
* **Phase 6 and Phase 7 meet the same question.** A reference unsequenced with
  whatever ends the lifetime it points into, and a move unsequenced with a read,
  are the same shape on two more axes. Whichever way this is written is the way
  three more will be.

## Considered Options

* **A field on the memory lattice holding what has been read since the last
  marker**, compared against the sites at each free.
* **A backward analysis**, saying at each point which sites may be freed later
  in the same region.
* **A hand-written backward reachability in the reporting walk**, from each free
  to the points that reach it without crossing a marker.
* **Leave it**, and keep answering one spelling and not the other.

## Decision Outcome

Chosen option: **a field on the memory lattice**.

```rust
/// Where the read is, and what it read through.
type ReadKey = (usize, u32, u32, Place);

struct PendingRead {
    /// The element's span, which is where `used here` goes.
    at: Span,
    /// Which allocations it may have read, resolved here.
    sites: BTreeSet<usize>,
}
```

`Known::pending` is a `BTreeMap<ReadKey, PendingRead>`. **Ordered containers because a
lattice value has to be canonical, and because the type is a better place for
that than a rule somebody maintains.** It was written first as a sorted `Vec`
with four rules kept in step by hand: the merge sorts, the merge deduplicates,
an insertion goes at its ordered position, and the key carries the end of a span
as well as its start. A mutation of each of the first three left the whole suite
green, because reaching them means building a program for the bound rather than
for the check. A map and a set have no arrangement to get wrong, so three of the
four stopped being rules. The fourth is a rule about which two reads are one
read and is discussed under Confirmation.

`Known::pending` holds them, `Known::met` records one at every element and every
terminator, the `Element::Sequenced` arm empties it, and `used_before` reports
the ones a free is about to take the sites of. **One marker, one meaning, read
from each side**: the element that says a free behind it is ordered is the
element that says a read behind it is ordered, and the same arm does both.

**The sites are resolved where the read is and not where the free is.**
`x = *p + (p = q, free(p), 0)` reads one allocation and frees another, and asking
at the free would answer about the wrong one. That is a silence rather than a
false positive, which is why the entry carries the answer rather than the
question.

**Every report from this is `Unknown` with `unsequenced` set.** There is no
program it can be right to call `Unsafe` about, so this cannot fabricate a proof
whatever it gets wrong. That is the ceiling the drivers name, and it is a
property of the shape rather than of the code.

**It does not go through `verdict`.** That answers what a set of sites is worth
*now*, and now is before the free, where every one of them is still live: it
answers `None` here, correctly, to a different question. RK-046 in the review
knowledge bank is a slot asked what it does not hold, and RK-055 is one
judgement point inheriting a rule written for the other question; this is the
second, avoided by not asking.

**The words are the mirror's words.** Two spellings of one program answer the
same thing, including the note citing 6.5.2.2 p10, which is as true of a free
written after the read as of one written before it. Nothing in `driver.rs`
changed and no string was added.

### The marker moves to the head of each arm

`if (*p)` needs no temporary, so the dereference is carried by
`Terminator::Branch` itself, and the marker the lowering emitted for the
controlling expression sat **before** that terminator. C17 6.8 p4 lists a
controlling expression among the full expressions and puts a sequence point at
the end of one, and the end of an expression is after the evaluation the
terminator performs, so the position was wrong and had been inert: only a free
asks about a marker and no branch frees.

Carrying reads forwards is what makes it stop being inert, so `Builder::enter`
now begins each arm of a statement's branch with it, which is where
`Lowering::split` and `Lowering::second` already put the ones for `&&`, `||` and
`?:`. `a_condition_read_through_a_pointer_in_an_unsequenced_operand_is_reported`
is why this is worth the churn rather than answering `None` for a branch's
condition: a `?:` below a `+` is enclosed by something C leaves unsequenced, gets
no marker, and has to report.

**Both arms**, because both follow the condition: an `if` with no `else` still
has the edge that skips the body, and a loop's exit is as much after the
condition as its body is. `for` with no condition has no controlling expression
and no marker, which is `enter`'s `None`.

### Confirmation

`error[E0027]` at `Allocations::join` and `Known::reborn`, which destructure
every field of `Known` rather than writing `..`: a field added to that value has
to say what a merge and a rebirth do to it. RK-018 in the review knowledge bank
is that spelling and why. `error[E0004]` at `Allocations::element` and
`dereferenced_in_element` if a kind is added to `Element`, which is ADR-0022's
guard and is unchanged.

Each mutation below applied on its own, the whole workspace suite run with
`--no-fail-fast`, the tree restored, and the failures read rather than predicted.

| Mutation | Named test that fails |
|---|---|
| the element transfer records nothing | `a_write_through_a_pointer_the_check_meets_first_is_reported` and `a_discarded_read_the_check_meets_first_is_reported` |
| the terminator transfer records nothing | `an_unsequenced_use_the_check_meets_first_is_reported`, `a_condition_read_through_a_pointer_in_an_unsequenced_operand_is_reported` and `a_read_of_either_of_two_allocations_before_a_free_names_neither` |
| a report from this concludes `Unsafe` | the same three, whose `.stderr` carries a warning and would carry an error |
| the sites are not compared, so every read behind a free is reported | `a_use_of_another_pointer_before_a_free_is_not_reported` |
| `made` keeps the first rather than folding it with `same` | `a_read_of_either_of_two_allocations_before_a_free_names_neither`, which gains an `allocated here` naming one of two |
| `Known::reborn` leaves `pending` alone | `a_read_of_a_site_handed_to_a_second_allocation_is_not_carried_to_its_free` in `crates/safec-ir/tests/freed.rs`, which no C program reaches and a frontend can build |
| the key drops the place, so one span is one read | `a_proof_replaces_the_suspicion_at_one_caret` |
| `say` keys on the span alone | `two_pointers_used_after_a_free_on_one_line` |

**The two transfers needed a case each and nobody would have guessed which.**
Deleting both recordings at once fails two cases, so the pair looked guarded;
deleting only the element half left the whole suite green, because every read the
corpus reported on until then was carried by a call's argument or a branch's
condition. RK-039 in the review knowledge bank is a mutation that measures at
both ends, and this is the shape it warns about: a mutation of the pair says
nothing about either half.

The rows above are this record's own. The two below are the marker's, and each
makes the compiler report a use after free in a program C defines, which is a
false positive rather than a false proof and is row 4.

| Mutation | Named test that fails |
|---|---|
| an arm does not begin with the sequence point | `a_condition_read_through_a_pointer_is_sequenced_before_the_body` and `..._before_the_loop_exits`, and the artifacts whose markers move |
| only the taken arm of an `if` begins with it | `a_condition_read_through_a_pointer_is_sequenced_before_the_other_arm`, on its `.stderr`: measured, the mutated compiler warns about `if (*p) { } else { free(p); }` |
| a `for` with no condition asks for a marker anyway | `lowering::tests::a_for_loop_asks_before_each_turn_and_steps_after_each_body`, which is where that `None` is reached: the corpus cases with such a loop stop at `--emit ast` |

`a_comma_that_frees_after_it_reads` is the case that says the clearing is real:
dropping `value.pending.clear()` makes it report a program C17 6.5.17 p2 defines,
and it fails together with the three condition cases and the two hand-built
programs in `freed.rs` that read before they free.

**Three things here are held by nothing, and saying which is the point of this
section.**

`a_use_and_a_free_on_two_arms_of_one_conditional_are_not_both_reached` is held by
no mutation of a line. What makes it silent is that the reads travel in a lattice
value along the graph's edges, so an arm is walked from what reached it and not
from what the other arm did; breaking that means moving the field out of the
lattice, which is a design rather than an edit.

The end of a span in the key is what keeps two reads at two elements that begin
at one column from collapsing into one report. Dropping it from the key leaves
the whole suite green: no program in the corpus has such a pair reaching a free
unordered. What it would cost is one of the two reports, which is a suspicion
lost rather than a proof invented.

Recording the read **before** the transfer's arms, so that no early return can
skip it, is guarded by nothing either. Measured: moving it below the match
changes no answer, because the one arm that returns early is the write through a
pointer and this frontend reads an assignment's value back into a temporary, so
the read is recorded by that second element instead. It is written first because
the order is free and the alternative rests on a property of one lowering.

`Analysis::height` is held by nothing, as ADR-0016, ADR-0018, ADR-0019, ADR-0020
and ADR-0022 each say. Answering too low is a panic naming the method.

### Consequences

* Good, because the two spellings of one unsequenced program answer the same
  thing, and the answer is the one C licenses: a suspicion, and an error under
  `--deny-unknown`.
* Good, because the sequence point now bounds a walk in both directions, so a
  frontend that emits too few is louder rather than quieter whichever way the
  question is asked. `docs/c-family.md` says so where it states the requirement.
* Good, because the marker for a controlling expression is where C17 6.8 p4 puts
  it rather than one terminator early.
* Bad, because the lattice value is bigger and is cloned per edge per visit. It
  is emptied at every sequence point, so what it holds is one full expression's
  worth of reads rather than a function's, but #173 is already open about what
  this analysis costs on a loop and this does not help it.
* Bad, because `Place` now derives `Ord`, which is a claim the domain does not
  make. Its doc comment says the order exists so that a map can be keyed on one
  and means nothing else; a reader who sorts places for any other reason is
  reading the derive as permission.
* Bad, because thirty-two blessed artifacts gain or move a line, and the change
  is mechanical: `SAFEC_BLESS=1` makes them and a reviewer skims past them to
  find the one `.stderr` that matters.
* Bad, because the same asymmetry for a call this check cannot read is still
  there. It is issue #184, and what it costs is every dereference beside an
  opaque call rather than beside a free, which is a different decision.

  **No longer true.** #184 took the decision and it was to widen this mechanism
  rather than to build a second one: `used_before` asks the same question of an
  opaque callee, and answers it without a `freed here` caret because nothing
  established a free.

  **What that costs, measured rather than inferred from the corpus.** No corpus
  case moved, and a corpus that does not move is not evidence about ordinary C.
  What newly reports is an unsequenced read beside a call in a program with no
  free in it at all: `int f(int *p) { int x = g(*p) + h(p); return x; }` is an
  `SC0402` where it was silent. It stays `Unknown`, so it is row 4 and
  `--deny-unknown` is what turns it into a refusal, and that is the ground on
  which it was accepted rather than a claim that nothing was paid. What bounds
  it is that a read waits in `Known::pending` only where an earlier element or
  terminator registered it, and ADR-0026's `Element::ArgumentsEvaluated`
  empties it at every call's own arguments.
* **Bad, because #178 got wider and this is where that is written down.**
  ADR-0022's enclosure rule suppresses the marker for a `,`, `&&`, `||` or `?:`
  below anything C leaves unsequenced, and `=` is such a parent under C17
  6.5.16 p3. Until now that cost a proof in one direction only. Now it costs a
  suspicion in the other as well: measured, `int x = (*p, free(p), 0);` is
  silent and `x = (*p, free(p), 0);` is `error[SC0402]` under `--deny-unknown`,
  which is one expression written two ways and answered two ways, and C17
  6.5.17 p2 settles the order in both. Every one of these is row 4. The record
  that owns the rule is ADR-0022 and the issue is #178; what is new is the
  surface, not the rule.
* **Bad, because the sequence point between a call's arguments and the call is
  not expressed to this direction at all.** C17 6.5.2.2 p10's first sentence
  orders a call's own argument evaluation before the call, and ADR-0022 says
  the IR expresses that by position, which was true while only a free looked
  backwards. A read carried forwards is not stopped by a position, so
  `void f(int *p) { free(p + *p); }` is reported although C defines it, and the
  note the report prints cites the very clause that refutes it. Measured, and
  `clang -std=c17 -pedantic-errors` accepts the program. It is row 4 and it is
  issue #185.
* What would reverse this: a value per program point and a backward direction in
  `dataflow.rs`, which ADR-0016 lists as unasked-for and which would let the
  question be answered where it is asked rather than carried to where it can be.
  It is a larger framework for one question, and the question is answered here in
  one field.

## Pros and Cons of the Options

### A field on the memory lattice

* Good, because the solver answers the diamond and the back edge, which is what
  it is for: an arm's reads do not reach the other arm's free.
* Good, because nothing in `dataflow.rs` or the IR changes.
* Bad, because the value grows, and a value carrying spans is where ADR-0016
  measured a walk that does not end. What converges is a set rather than a slot
  that keeps whichever arrived, and `Place: Ord` is the price of the container
  that holds the arrangement for us.

### A backward analysis

* Good, because it says what it means: which sites may be freed later in this
  region.
* Bad, because it cannot be written against `Analysis` as it stands. The
  transfer is handed no position, so it cannot ask the forward solution what a
  free's argument reaches, and carrying the place instead and resolving it at
  the free answers about the wrong allocation whenever the region reassigns the
  pointer. That is a silence, which is the direction this check cannot afford.

### A hand-written backward reachability in the reporting walk

* Good, because the lattice does not grow and `Analysis::height` is untouched.
* Bad, because what a region is would be written twice, once in the transfer
  that clears the marker and once in the walk that stops at it. RK-052 in the
  review knowledge bank is one rule in two places drifting apart inside the
  change that touches one of them, and it is an entry about this file.

### Leave it

* Good, because it costs nothing and the corpus blesses the silence.
* Bad, because the answer depends on which side of a `+` the free is written,
  which is a fact about the lowering and not about the program. A reader who
  swaps two operands and watches a diagnostic disappear has learnt that the
  compiler is not answering about C.

## More Information

* `Known::pending` and `used_before` in
  [`crates/safec-ir/src/memory.rs`](../../crates/safec-ir/src/memory.rs), and
  `Builder::enter` in
  [`crates/safec/src/lowering.rs`](../../crates/safec/src/lowering.rs).
* C17 Annex C is the complete list of sequence points; 6.5 p3 is what makes
  everything not on it unsequenced, and 6.5.2.2 p10 is what a call is answered
  by.
* Issue #177 carries the measurement and the two programs. Issue #184 was the
  same asymmetry for a call this check cannot read, and closed by widening this
  record's mechanism rather than by deciding anything else.
