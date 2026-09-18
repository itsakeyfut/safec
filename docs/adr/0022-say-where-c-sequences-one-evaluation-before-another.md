---
status: "accepted"
date: 2026-09-15
decision-makers: itsakeyfut
---

# Say where C sequences one evaluation before another, as an element of a block

## Context and Problem Statement

`int x = *p + (free(p), 0);` is reported as a **proved** use after free, and so
is its mirror `int x = (free(p), 0) + *p;`. `clang -std=c17 -pedantic-errors`
accepts both, and C17 6.5 p3 leaves the operands of `+` unsequenced: one order
reads `*p` before the free and the program is defined, the other does not.
6.5.2.2 p10 makes the callee's execution indeterminately sequenced with
everything in the caller that is not otherwise sequenced. Which order an
implementation picks is unspecified, and this compiler does not get to pick one.

The tell is in the caret. The `used here` label starts to the *left* of
`freed here`:

```
 5 │     int x = *p + (free(p), 0);
   │             ────────┬┬──────
   │                     ╰───────── used here
   │                      ╰──────── freed here
```

The check is not reading an order. It is reading the order the lowering emitted,
and ADR-0010 makes a call end a block, so the free lands in an earlier block than
the rest of the expression whatever the source says.

**A block's element list is a total order. C gives a partial one.** Nothing in
the IR says which parts of the total order the standard actually licenses, so a
consumer that reads it as sequencing is believing an arbitrary choice the
lowering made. That is a proof the standard does not license, and
[`docs/safety-model.md`](../safety-model.md) calls a wrong proof the worst thing
this compiler can do.

## Decision Drivers

* The lowering knows the answer while it is walking the tree, and **nothing
  downstream can recover it**. By the time an analysis runs, the tree is gone
  and the block order is all that is left.
* Every analysis after this one meets the same question. A move unsequenced with
  a read and a borrow unsequenced with a write are both Phase 7's version of it;
  Phase 6's is a reference unsequenced with whatever ends the lifetime it points
  into. Answering it once in the IR is the difference between one rule and four
  copies of it.
* The failure to avoid is a **proof**, so the bias has to be towards saying
  less. A missing sequence point is a report that is a warning where it could
  have been an error, which is row 4. A sequence point claimed where C gives
  none is row 6.

## Considered Options

* **An `Element::Sequenced` in the block's element list.**
* **A region id on every element**, where two elements sharing one are
  unsequenced.
* **The unsequenced region's span on `Terminator::Call`**, compared against the
  use's span.
* **Leave it**, and keep reporting the proof.

## Decision Outcome

Chosen option: **an `Element::Sequenced` in the block's element list**.

```rust
/// Everything before this is sequenced before everything after it.
Sequenced {
    /// Where the construct that sequences is written.
    origin: Origin,
},
```

It is emitted **at a sequence point that no unsequenced operator encloses**.
C17 Annex C is the complete list of sequence points, and the ones this frontend
can reach are the end of a full expression (6.8 p4) and the operators `,`
(6.5.17 p2), `&&` (6.5.13 p4), `||` (6.5.14 p4) and `?:` (6.5.15 p4). Annex C's
first entry, between a call's arguments and the call itself (6.5.2.2 p10), needs
nothing: the IR already places the argument operations before the call
terminator. What it does **not** say is that a call's arguments are ordered
against each other, and they are not, which is why a sequencing operator inside
one of them records nothing.

**The enclosure test is the whole of the difficulty.** `*p + (free(p), 0)`
contains a comma, and the comma is a sequence point; emitting one for it would
put a marker between the free and the read of `*p` and make the program proved
again, which is the bug this record is about. The comma sequences its own two
operands and says nothing about the other operand of the `+`. So a sequence
point counts only where the path from the full expression's root to it passes
through sequencing constructs alone.

The lowering holds that as one flag while it walks. Descending into the operand
of anything else pushes a task that puts the flag back, before the node's own
tasks, so it is popped after all of them.

**A node with one operand is on the false side of that list**, even though a
sequence point inside `-(free(p), *p)` really does order those two: there is
nothing else in the expression for them to be unordered against. Answering false
there costs a proof, which is row 4. What it buys is that `sequences` is C17
Annex C's list and nothing beside it, so a reader checks it against the standard
rather than against an argument, and every addition to it is a chance to say
`true` once too often.

**A new `Element` kind rather than a field.** RK-018 in the review knowledge
bank is the reason: `error[E0004]` makes every reader of `Element` answer for a
kind, and a field is what `..` walks past without a word. `print.rs`,
`interp.rs`, `emit.rs`, `memory.rs`, the hand-built analysis in
`crates/safec-ir/tests/written.rs` and two helpers in `lowering.rs`'s own tests
each stopped compiling until they said what this means to them, which is the
property ADR-0012 bought for the storage markers and is the same argument.

**A double free does not ask for one.** Two frees of one allocation are a
double free whichever order runs, so there is nothing for a sequence point to
settle, and asking for one turned `(free(p), 0) + (free(p), 0)` from an error
into a warning. #142's own body said so and the first implementation did it
anyway; `verdict` now takes the `Kind` and only a use asks about the order.

**A free already ordered before here is the one to keep.** A second free at a
site replaced the first, and with it the fact that the first was sequenced, so
`free(p); x = (free(p), 0) + *p;` lost a proof it had before this change. What
survives is the earlier, sequenced free, which is also what `SiteState::Freed`'s
doc comment has always said it holds and what the diagnostic has to point at for
the proof to be readable.

**What the check does with it.** A free stops being a span and becomes a
`Freeing`: the span, and whether C has ordered it before what follows. A use
asks for the second half. `Freeing::joined` takes the earlier span and the
**conjunction** of the flags, because a path that arrived with the order still
open is a path on which this is not a proof.

One type rather than a flag beside each span, because a free is recorded in two
places: on the site, and on the local that named a may-set, which ADR-0020 put
there. Two carriers of one fact is how the second gets the span and not the half
that says what the span is worth, and RK-046 is the entry about a marking that
turns out to carry two things.

**An unproven report that still names its free.** The existing `Unknown` drops
both spans because what makes it unknown is that the paths or the sites
disagree, so there is no one free to point at. Here there is exactly one, and
the reason is different in kind: nothing this check can see orders the free
first. RK-034 is the entry about a match arm that means "proved" and "gave up"
at once, and collapsing these two kinds of doubt into one is that mistake at the
report. So the finding keeps its `freed here` caret and carries a note saying
what leaves it open.

**Exactly one**, and the fold counts. Several frees meeting at one report give
the earliest span and the conjunction of their orders, which can come from two
different frees: the caret would then point at a free this check *has* seen
sequenced while the note says the order is open. Where more than one was folded
the span is dropped and the report is the ordinary spanless `Unknown`.

**The note says what this check found, not what C decided.** They are not the
same: in `x = (free(p), *p)` C17 6.5.17 p2 settles the order and this check
still records nothing, which is #178. It cites 6.5.2.2 p10 rather than 6.5 p3,
because p3 leaves subexpressions unsequenced and unsequenced evaluations may
interleave; what gives a call one order or the other is p10's indeterminate
sequencing, and a free is a call.

### Confirmation

`error[E0004]` at every reader of `Element` if the kind is removed:
`print.rs`, `interp.rs`, `emit.rs`, `memory.rs`, the hand-built analysis in
`crates/safec-ir/tests/written.rs` and two helpers in `lowering.rs`'s own
tests. `error[E0063]` at `Freeing::new` and `Freeing::joined` if a field is
added to the pair a free carries, and `error[E0004]` at `verdict` if a variant
is added to `Kind`, which is what says a third check has to answer whether its
question turns on the order.

Each mutation applied on its own, the whole workspace suite run with
`--no-fail-fast`, the tree restored.

| Mutation | Named test that fails |
|---|---|
| `Builder::sequenced` builds nothing | every proved double free and use after free this check makes, in the corpus and in `freed.rs` alike |
| the check does not read the kind | the same set, from the other side |
| `sequences` answers `false` for a comma | `a_comma_at_the_top_of_a_full_expression_is_a_sequence_point` and the corpus cases whose markers move |
| the marker for `,`, for `&&` and `\|\|`, or for `?:` is dropped | `a_comma_sequences_a_free_before_a_use`, `a_logical_and_sequences_a_free_before_a_use` with `a_logical_or_...`, and `a_conditional_sequences_a_free_before_a_use` |

Those hold that the markers are emitted. The rows below are the ones that
matter, because each mutation makes this compiler **certain** about an order C
has not chosen rather than quiet about one it has.

| Mutation | Named test that fails |
|---|---|
| the comma's marker stops asking what encloses it | `an_unsequenced_free_and_use_is_not_proved`, `the_same_program_with_the_operands_swapped_is_not_proved_either` and `a_comma_inside_an_unsequenced_operand_is_not_one` |
| `sequences` answers `true` for a call | `a_comma_inside_a_call_argument_orders_nothing_outside_it` |
| `&&` and `\|\|`'s marker stops asking | `a_logical_and_inside_an_unsequenced_operand_orders_nothing` |
| `?:`'s markers stop asking | `a_conditional_inside_an_unsequenced_operand_orders_nothing` |
| `Freeing::new` starts sequenced | the four above and three beside them |
| `Freeing::joined` takes the stronger flag | `a_free_sequenced_on_one_arm_only_is_not_a_proof` and `a_may_set_where_one_free_is_sequenced_and_one_is_not` |
| `Freeing::joined` keeps its own span rather than the earlier | the first of those, and `a_use_after_two_allocations_names_no_allocation` |

And three where asking about the order was the wrong question, each of which
this change got wrong until review measured it against the binary it replaced.

| Mutation | Named test that fails |
|---|---|
| a double free asks for a sequence point too | `a_double_free_in_one_expression_does_not_turn_on_the_order`, which goes from an error to a warning |
| a later free replaces one already sequenced | `a_free_already_sequenced_is_the_one_a_later_free_keeps`, which loses a proof it had before this change |
| the fold keeps the `freed here` caret where several frees met | `a_may_set_where_one_free_is_sequenced_and_one_is_not`, which points a caret at a free this check did see sequenced |

`a_value_used_after_it_was_freed` is the two-statement program that has to stay
proved, and it was in the corpus before this. No case that was in the corpus
before changed its answer: every one of them frees in a statement of its own.
### Consequences

* Good, because the IR now says which parts of its own order are C's and which
  are the lowering's, which is a question every later analysis asks.
* Good, because the reader of `--emit safety-ir` can see where a sequence point
  is. A fact nothing prints is a fact nothing checks.
* Bad, because ninety-nine blessed artifacts gain lines. The change is
  mechanical and `SAFEC_BLESS=1` makes it, and every one of them is a diff a
  reviewer has to skim past to find the ones that matter.
* Bad, because each site carries one more bit. `Analysis::height` is not
  changed for it: the bit is set by a transfer and taken back only by a join,
  which can happen once per site, and the number already had that much slack.
  That is slack being spent rather than a bound re-derived, and the number is
  still held by nothing. ADR-0016, ADR-0018, ADR-0019 and ADR-0020 each say the
  last part and it is still true.
* Bad, because the enclosure rule gives up a proof wherever a sequencing
  operator sits below something C leaves unsequenced, which is **much wider
  than one operand**. `int x = (free(p), *p);` is an error and
  `x = (free(p), *p);` is a warning, because an initializer is a full
  expression and an assignment's operands are unsequenced by 6.5.16 p3. Two
  spellings of one statement, two answers, which is the thing
  `docs/c-family.md` asks a frontend to normalise one section above where this
  is written down. `a_logical_and_inside_an_unsequenced_operand_orders_nothing`
  and the two cases beside it hold the current answer, and #178 is the issue.
  Every one of these is row 4 and `--deny-unknown` reports all of them.
* This answers the forward half of the question and the record should not be
  read as answering the whole of it. A marker says that what came before it is
  sequenced before what comes after; it cannot say that two things are
  unsequenced, because the check meets them one at a time and only ever looks
  back. So a use the walk met **before** the free was not compared with it at
  all: `int x = g(*p) + (free(p), 0);` was silent under every flag where
  `int x = (free(p), 0) + g(*p);` reported. That was #177, and
  [ADR-0023](./0023-carry-a-read-forwards-to-the-free-it-is-unordered-against.md)
  is the other half: the read is carried forwards to meet the free instead of
  the free looking back for it, and this element is what bounds how far. The
  marker's meaning is unchanged and is now read from both sides.
* Bad, because a program whose free and use really are unsequenced loses its
  error at the default level. That is the point, and `--deny-unknown` is what
  gets it back.
* The `()` sequence point is expressed by position rather than by a marker,
  which is one rule with two spellings. What makes it acceptable is that the
  argument operations and the call terminator are in one block in that order and
  cannot be otherwise; a frontend that emitted them apart would be building
  something else.
* What would reverse this: an IR that carries the expression tree, where the
  partial order could be read off rather than recorded. That is a larger IR for
  one question, and the question is answered here in one element.

## Pros and Cons of the Options

### An `Element::Sequenced` in the block's element list

* Good, because `error[E0004]` makes every consumer answer for it.
* Good, because it is a position, and a dataflow analysis is a walk over
  positions.
* Bad, because it is a line in ninety-nine artifacts.

### A region id on every element

* Good, because it needs no new kind and the test is an equality.
* Bad, because a field is what `..` walks past, which is RK-018 exactly, and
  every element grows it including the storage markers, which have no
  expression to belong to.

### The unsequenced region's span on `Terminator::Call`

* Good, because it is one field on one variant and touches the fewest
  artifacts.
* Bad, because the check would decide a question about C by asking whether one
  source position is inside another. `Span` is due a third coordinate for
  macros, which `docs/c-family.md` already names as a decision waiting, and
  containment is exactly what that would change underneath.

### Leave it

* Good, because it costs nothing.
* Bad, because the compiler says **proved** about a program C defines under one
  of its allowed orders, and no flag makes that quieter. It is row 6 of
  `CLAUDE.md`'s list read from the other side: not a missed defect, but a
  fabricated one stated with certainty.

## More Information

* `Element::Sequenced` in
  [`crates/safec-ir/src/ir.rs`](../../crates/safec-ir/src/ir.rs), and the flag
  the walk carries in
  [`crates/safec/src/lowering.rs`](../../crates/safec/src/lowering.rs).
* C17 Annex C is the complete list of sequence points, and 6.5 p3 is what makes
  everything not on it unsequenced.
* Issue #142 carries the measurement and the four programs.
