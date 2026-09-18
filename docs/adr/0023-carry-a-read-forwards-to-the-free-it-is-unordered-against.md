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
not is how wide the report is.

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
struct Pending {
    /// The element's span, which is where `used here` goes.
    at: Span,
    /// What was dereferenced, which is half the key `used` collapses on.
    place: Place,
    /// Which allocations it may have read, resolved here.
    sites: Vec<usize>,
}
```

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
controlling expression sat **before** that terminator. C17 6.8.4.1 p2 and 6.8 p1
make a controlling expression a full expression and 6.8 p4 puts the sequence
point at its end, which is after the evaluation the terminator performs, so the
position was wrong and had been inert: only a free asks about a marker and no
branch frees.

Carrying reads forwards is what makes it stop being inert, so `Builder::entering`
now begins each arm of a statement's branch with it, which is where
`Lowering::split` and `Lowering::second` already put the ones for `&&`, `||` and
`?:`. `a_condition_read_through_a_pointer_in_an_unsequenced_operand_is_reported`
is why this is worth the churn rather than answering `None` for a branch's
condition: a `?:` below a `+` is enclosed by something C leaves unsequenced, gets
no marker, and has to report.

**Both arms**, because both follow the condition: an `if` with no `else` still
has the edge that skips the body, and a loop's exit is as much after the
condition as its body is. `for` with no condition has no controlling expression
and no marker, which is `entering`'s `None`.

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
| nothing is recorded, at either transfer | `an_unsequenced_use_the_check_meets_first_is_reported` and `a_condition_read_through_a_pointer_in_an_unsequenced_operand_is_reported`, which go back to silent |
| the terminator's transfer records nothing | the same two: a `?:` condition is read by the branch and the first case's use is read by a call |
| the report concludes `Unsafe` | the same two, whose `.stderr` carries a warning and would carry an error |
| the sites are not compared, so every read behind a free is reported | `a_use_of_another_pointer_before_a_free_is_not_reported` |
| `Known::reborn` leaves `pending` alone | `a_read_of_a_site_handed_to_a_second_allocation_is_not_carried_to_its_free` in `crates/safec-ir/tests/freed.rs`, which no C program reaches and a frontend can build |

The rows above are this record's own. The two below are the marker's, and each
makes the compiler report a use after free in a program C defines, which is a
false positive rather than a false proof and is row 4.

| Mutation | Named test that fails |
|---|---|
| an arm does not begin with the sequence point | `a_condition_read_through_a_pointer_is_sequenced_before_the_body` and `..._before_the_loop_exits`, and thirty-one artifacts whose markers move |
| only the taken arm of an `if` begins with it | `a_condition_read_through_a_pointer_is_sequenced_before_the_other_arm`, on its `.stderr`: measured, the mutated compiler warns about `if (*p) { } else { free(p); }` |

`a_comma_that_frees_after_it_reads` is the case that says the clearing is real:
dropping `value.pending.clear()` makes it report a program C17 6.5.17 p2 defines,
and it fails together with the two condition cases and the two hand-built
programs in `freed.rs` that read before they free.

`a_use_and_a_free_on_two_arms_of_one_conditional_are_not_both_reached` is held by
no mutation of a line, and this says so rather than claiming otherwise. What
makes it silent is that the reads travel in a lattice value along the graph's
edges, so an arm is walked from what reached it and not from what the other arm
did; breaking that means moving the field out of the lattice, which is a design
rather than an edit.

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
  make. Its doc comment says the order exists so that a set of them can be
  canonical and means nothing else; a reader who sorts places for any other
  reason is reading the derive as permission.
* Bad, because thirty-two blessed artifacts gain or move a line, and the change
  is mechanical: `SAFEC_BLESS=1` makes them and a reviewer skims past them to
  find the one `.stderr` that matters.
* Bad, because the same asymmetry for a call this check cannot read is still
  there. It is issue #184, and what it costs is every dereference beside an
  opaque call rather than beside a free, which is a different decision.
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
  measured a walk that does not end. It is a sorted set rather than a slot,
  which is why it converges, and `Place: Ord` is the price.

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
  `Builder::entering` in
  [`crates/safec/src/lowering.rs`](../../crates/safec/src/lowering.rs).
* C17 Annex C is the complete list of sequence points; 6.5 p3 is what makes
  everything not on it unsequenced, and 6.5.2.2 p10 is what a call is answered
  by.
* Issue #177 carries the measurement and the two programs. Issue #184 is the
  same asymmetry for a call this check cannot read.
