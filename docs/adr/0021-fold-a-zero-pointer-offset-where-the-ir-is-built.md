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
* Nothing asks for the integer case. `E + 0` on an `int` is dead arithmetic
  and folding it would be an optimisation, which this compiler does not do and
  has no reason to start doing in the lowering.

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
`0 - E` is not folded and why `types.rs` declines to give `1 - p` a type.
Reading one of those for the other is a habit rather than an accident, and it
happened twice while this change was being made: RK-042 in the review knowledge
bank is the same mistake caught in 6.5.3.2, about a different program.

**Only where the result is a pointer, and not because the value would break.**
The obvious reason is the wrong one and it was measured rather than assumed:
`int f(char c) { return c + 0; }` built with the type test removed emits
`sext i8 %t0 to i32` and stores that, because `crates/safec-llvm/src/emit.rs`
converts every operand at the point it is consumed rather than relying on an
`Rvalue::Binary` having promoted it first. Compiled and run, `f(-5)` exits `-5`
either way. So folding the integer case would **not** hand anyone a `char`
today.

What the restriction buys is a rule with one subject. The fold exists because
`E[0]` and `*E` are one C expression, which is a statement about pointers; an
integer `c + 0` is not two spellings of anything, so folding it would be an
optimisation with no motivation and a promotion this IR models for the reader
to lose. The line is drawn where the reason runs out, which is also the only
place it can be drawn without a value-level argument nobody has.

**The IR print changes and that is the point.** `--emit safety-ir` is an
interface, and it now prints one shape where C has one expression. Six blessed
expectations move, none of them a diagnostic.

**The backend gained a spelling, which the corpus could not see.** `pp[0] = 0;`
under `--emit llvm-ir` was `error[SC0801]`, because the backend cannot write
pointer arithmetic and the subscript built some. It now emits, because there is
no arithmetic left to refuse, and that is correct rather than hidden: `pp[0]` is
`*pp` and `*pp` always emitted. It is a real change and no corpus case reached
it, so `a_zero_subscript_reaches_the_backend` is added to hold it. `pp[1]` is
still `SC0801`, which is the gap this does not close.

### Confirmation

Each mutation applied on its own to `crates/safec/src/lowering.rs`, the whole
workspace suite run with `--no-fail-fast`, the file restored.

| Mutation | Named test that fails |
|---|---|
| `unmoved` always answers `None` | `a_subscript_write_is_the_write_it_is_defined_as`, `a_write_through_a_pointer_plus_zero_on_the_left` and `a_write_through_a_pointer_minus_zero` on their diagnostics, and `a_constant_subscript_of_a_freed_pointer`, `a_discarded_subscript_after_a_free` and `a_subscript_in_a_condition_after_a_free` on their IR, and `a_zero_subscript_reaches_the_backend`, which goes back to `error[SC0801]` |
| `unmoved` stops asking whether the result is a pointer | `a_zero_added_to_an_integer_keeps_its_operation`, whose `char` addition loses the operation it is defined to perform. It guards the IR's shape and not a computed value: the same program compiled and run answers the same under the mutation |
| `Expr::Subscript` asks about its own type rather than the base's | the three subscript cases above, whose IR keeps the addition, because a subscript's type is what the address reaches and not the address |
| `Add` reads the zero on one side only | `a_subscript_write_is_the_write_it_is_defined_as` or `a_write_through_a_pointer_plus_zero_on_the_left`, depending which side |
| `Sub` is not answered for, or takes its zero on the left | `a_write_through_a_pointer_minus_zero` |

**Every diagnostic in the corpus is byte-identical to what the rejected option
produced.** That is the measurement this record rests on: the fold and the check
answer the same, and only one of them leaves the next three lattices something
to copy. It is a comparison against the other implementation of this change and
not against what shipped before it, which is the next section.

### Consequences

* Bad, because a build that failed now passes, and this is the headline cost.
  `int *p = malloc(8); int **pp = &p; free(p); p[0] = 42;` was `error[SC0402]`
  and exit 1 and is `warning[SC0402]` and exit 0.
  `a_subscript_of_an_escaped_pointer_is_suspected_not_proved` is that program
  and `a_constant_subscript_of_a_freed_pointer` is the same one without the
  `&p`, which still proves. Two of six hundred generated programs moved, both
  this way and none the other. **The direction is right even though it is
  down.** `*p` in that program was never proved, because
  [ADR-0017](./0017-record-each-half-of-an-escape-where-its-subject-lives.md) keeps a
  local whose address was taken out of a proof, and the subscript reached one
  only by arriving as a shape that rule did not meet. Aligning two spellings has
  to pick one answer, and the answer a rule already gives beats the one that
  came from evading it. `--deny-unknown` exits 1 on it.
* Good, because four spellings of one write now agree, and the rule that makes
  them agree is one sentence in the layer that already promised it.
* Good, because `crates/safec-ir/src/memory.rs` gets smaller rather than larger.
  The condition it had, and the function that condition had to become when
  review found it written twice, are both gone.
* Bad, because a zero offset that is not the token `0` is not folded.
  `*(pp + -0)` and `*(pp + (1 - 1))` keep their addition and are read as moving
  the pointer, and the cost is **a silence under every flag**: measured, such a
  program is exit 0 with no output at `--safety strict --deny-unknown` where its
  `pp[0]` twin exits 1. This reads the operand, not the value, and a constant
  folder that read values would be a different promise. What bounds the class is
  that the frontend refuses most of the other spellings: a hexadecimal or long
  constant, a character constant and a cast are each `SC0304` or `SC0201`
  today, and an array type is refused outright, so what is left is `-0`,
  `n - n` and `0 * k`. **The boundary is the operand and not the value**, and
  crossing it needs a constant evaluator, which the frontend does not have:
  `types.rs` and `ast.rs` each say in as many words that they do not evaluate a
  constant expression. So the trigger for reopening this is not a second
  complaint about a spelling, it is Phase 9's `#if`, which brings the evaluator
  in for its own reasons.
* Bad, because a check now rests on a normalisation rather than on something it
  verified. `docs/c-family.md` carries what that costs and who has to keep it.
* Bad, because the lowering normalises more than it did. It already dropped
  unary `+` outright, where the same promotion argument applies, so the first
  fold is not this one; what changes is that there is now a list. What it holds
  is three shapes and it is in one function; `E * 1`, `E - E` and
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
* Bad, because it folds `c + 0` for a `char`, where C converts before it adds,
  and the IR stops showing the conversion. Measured, the emitted value is
  unchanged, so what is lost is the reader's view of the promotion rather than
  the answer. It is also a fold with no reason behind it: nothing spells an
  integer addition of zero two ways.

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
