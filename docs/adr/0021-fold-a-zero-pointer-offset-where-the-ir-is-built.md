---
status: "accepted"
date: 2026-09-14
decision-makers: itsakeyfut
---

# Fold a zero pointer offset where the IR is built, so no analysis sees two shapes for one expression

## Context and Problem Statement

`pp[0] = q;` was silent where `*pp = q;` reported a use after free. C17 6.5.2.1
p2 defines `E1[E2]` as `(*((E1)+(E2)))`, so those are one program spelled two
ways, and the spelling decided whether this compiler said anything.

The lowering builds the subscript as the standard defines it, an addition and a
dereference, and says in that arm why:

> one C expression is one shape in the IR, so an analysis asking what an access
> reaches has one thing to read rather than two spellings of it

The addition of zero is the exception the sentence does not survive. `*pp` is a
dereference of a local; `pp[0]` is a dereference of a temporary holding
`pp + 0`. They are two shapes, and
[ADR-0019](./0019-follow-a-write-through-a-pointer-only-where-it-lands.md)'s
rule for what an edge survives is read against the second one.

A first implementation taught the memory check to read the constant zero. It
worked, and review measured that it is the more expensive half of a choice
nobody had made deliberately.

## Decision Drivers

* The lowering already states the principle, in the function this is about. A
  check that reconciles two shapes is a check working around a guarantee the
  layer below meant to give.
* Three more lattices are to be written against this IR. Each would meet the
  same question, and what they would copy is a condition rather than a rule.
* An integer operand is promoted before an arithmetic operation. Folding one
  away would skip a conversion the IR models, and what that costs is a **value**
  rather than a report.

## Considered Options

* **Fold in the lowering, where the result is a pointer.**
* **Fold in the lowering, whatever the type.**
* **Read the constant in the memory check**, which is what was built first.

## Decision Outcome

Chosen option: **fold in the lowering, where the result is a pointer**.

`Lowering::unmoved` answers, for an additive operation whose result is a
pointer, the operand it yields when the offset is a literal zero. `E + 0`,
`0 + E` and `E - 0`, and nothing else. `Expr::Subscript` and `Expr::Binary` ask
it and push that operand rather than emitting an `Rvalue::Binary`, so `pp[0]`
takes the same path `*pp` takes and arrives as the same IR.

**C17 6.5.6 p8 is the paragraph.** Adding an integer to a pointer yields a
pointer to the element that far along, `(P)+N` and `N+(P)` alike, so at zero it
is a pointer to the same element. That is Semantics, which is what a fold needs.
The Constraints beside it answer who may write what, and are cited where that
question is asked: p3 allows the pointer only on the left of a `-`, which is why
`0 - E` is not folded and why `types.rs` declines to give `1 - p` a type. RK-042
in the review knowledge bank is the entry about reading one of those for the
other, and it was written after a reviewer did.

**Only where the result is a pointer.** A pointer is not promoted, so the
conversion question does not arise for the case this is for. An integer's does:
`char c; c + 0;` is an addition at `int` after a conversion, and folding it to
`c` would hand a `char` to whoever expected the promoted value. Nothing in the
suite would catch that, which is the reason it is excluded by construction
rather than by test.

**The IR print changes and that is the point.** `--emit safety-ir` is an
interface, and it now prints one shape where C has one expression. Six blessed
expectations move, none of them a diagnostic.

### Confirmation

Each mutation applied on its own to `crates/safec/src/lowering.rs`, the whole
workspace suite run with `--no-fail-fast`, the file restored.

| Mutation | Named test that fails |
|---|---|
| `unmoved` always answers `None` | `a_subscript_write_is_the_write_it_is_defined_as`, `a_write_through_a_pointer_plus_zero_on_the_left` and `a_write_through_a_pointer_minus_zero` on their diagnostics, and `a_constant_subscript_of_a_freed_pointer`, `a_discarded_subscript_after_a_free` and `a_subscript_in_a_condition_after_a_free` on their IR |
| `unmoved` stops asking whether the result is a pointer | `a_zero_added_to_an_integer_keeps_its_operation`, whose `char` addition loses the operation it is defined to perform. That is the one guard here whose absence costs a value rather than a report |
| `Expr::Subscript` asks about its own type rather than the base's | the three subscript cases above, whose IR keeps the addition, because a subscript's type is what the address reaches and not the address |
| `Add` reads the zero on one side only | `a_subscript_write_is_the_write_it_is_defined_as` or `a_write_through_a_pointer_plus_zero_on_the_left`, depending which side |
| `Sub` is not answered for, or takes its zero on the left | `a_write_through_a_pointer_minus_zero` |

**Every diagnostic in the corpus is byte-identical to what the rejected option
produced.** That is the measurement this record rests on: the fold and the check
answer the same, and only one of them leaves the next three lattices something
to copy.

### Consequences

* Good, because four spellings of one write now agree, and the rule that makes
  them agree is one sentence in the layer that already promised it.
* Good, because `crates/safec-ir/src/memory.rs` gets smaller rather than larger.
  The condition it had, and the function that condition had to become when
  review found it written twice, are both gone.
* Bad, because a zero offset that is not the token `0` is not folded.
  `*(pp + -0)` and `*(pp + (1 - 1))` keep their addition and are read as moving
  the pointer. This reads the operand, not the value, and a constant folder that
  read values would be a different promise.
* Bad, because the lowering now normalises, which it did not before. What it
  folds is three shapes and the list is in one function; `E * 1`, `E - E` and
  constant propagation are not on it and each would be its own decision.
* What would reverse this: an analysis that needs to see the addition a
  subscript performs. None does today, and the one that might, #143's reading of
  operand types to tell a pointer from an index, wants the types rather than the
  operation.

## Pros and Cons of the Options

### Fold in the lowering, where the result is a pointer

* Good, because it makes the layer that states the principle keep it.
* Good, because every analysis, present and future, gets the answer for free.
* Bad, because it moves what `--emit safety-ir` prints, which is an interface
  somebody may be reading.

### Fold in the lowering, whatever the type

* Good, because it is four lines shorter and needs no type at hand.
* Bad, because it folds `c + 0` for a `char`, where C converts before it adds.
  Nothing in the suite holds the conversion, so the failure would be a wrong
  value with no test to say so, which is worse than anything this issue was
  about.

### Read the constant in the memory check

* Good, because it touches one crate and moves no IR.
* Bad, because it puts the reconciliation of two shapes inside one reader of
  them, and there are four readers coming. Review found the rule already written
  twice inside `memory.rs` and the two copies already disagreeing, which is the
  shape of what the other three would inherit.

## More Information

* `Lowering::unmoved` and its two callers in
  [`crates/safec/src/lowering.rs`](../../crates/safec/src/lowering.rs).
* Issue #172 carries both implementations and the measurement that chose
  between them.
