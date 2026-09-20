---
status: "accepted"
date: 2026-09-20
decision-makers: project author
---

# Say that a call's arguments have been evaluated, as an element of its own

## Context and Problem Statement

ADR-0022 emits an `Element::Sequenced` for four of the sequence points C17
Annex C lists and leaves the fifth to position: the point between a call's
arguments and the call, 6.5.2.2 p10's first sentence, is expressed by the
argument operations sitting in the block before the call terminator. That
record says so and says what makes it acceptable, which was that every consumer
of the IR looked backwards from a free.

ADR-0023 added one that does not. `Known::pending` carries every read met since
the last marker forwards to meet whatever frees ahead of it, and a position does
not stop a walk that travels that way. So `void f(int *p) { free(p + *p); }` is
reported, although C orders that read before the call unconditionally, and the
note the report prints cites the very clause that refutes it. That is issue
#185, and it is row 4 of `CLAUDE.md`'s list: a report about a program with no
defect, at a caret a reader can see.

## Decision Drivers

* The point is real and unconditional. 6.5.2.2 p10's first sentence does not
  ask what encloses the call; what asks that is whether a *block-wide* marker
  may be written for it, which is ADR-0022's enclosure rule.
* A call's arguments are unsequenced against **each other**, by the same
  paragraph's second sentence. Anything that orders them is a proof C has not
  licensed, which `docs/safety-model.md` calls the worst thing this compiler
  can do.
* The IR linearises those unsequenced arguments into one block, and a call
  carries some of its argument reads in its own operands rather than in
  elements, so any marker written before the terminator has some argument
  evaluations on each side of it.
* Whatever is chosen is read by three analyses today and by the three
  `docs/safety-model.md` still asks for, so the failure mode of a consumer that
  has not caught up is part of the choice.

## Considered Options

* A new element kind, `Element::ArgumentsEvaluated`, saying only that the reads
  behind it are ordered before what follows
* An `Element::Sequenced` before the call terminator
* A field on `Element::Sequenced` naming which of the two meanings it carries
* A field on `Terminator::Call`
* Leave it

## Decision Outcome

Chosen option: **a new element kind**, emitted where no unsequenced operator
encloses the call, and concluding half of what `Element::Sequenced` concludes:
the reads behind it are ordered before everything that follows, and nothing is
established about the frees behind it.

The half is not modesty, it is the measurement. `Element::Sequenced` also marks
every free reaching it as ordered, and a consumer judges a call's own operand
reads after the element although C puts them before it. Emitting a full
`Element::Sequenced` here turned `int x = g((free(p), 0), *p);` from a warning
into an **error**, about two arguments C leaves unsequenced;
`a_comma_inside_a_call_argument_orders_nothing_outside_it` is the case, and it
exists to say exactly that. Nothing is lost by omitting the half, because a
call this element is emitted for is the root of its full expression and nothing
in that expression follows it.

The kind rather than a field, because the three options cost the same 116
artifacts and close the same programs, and what separates them is the reader
who has not caught up. A kind is `error[E0004]` at every consumer, and one that
answers it by doing nothing is this compiler before the element existed: a
suspicion where C licensed silence. A `covers` field on `Element::Sequenced`
fails the other way, because a reader that ignores it treats the weaker marker
as a full barrier, which is the false proof above. A field on `Terminator::Call`
is RK-018 in the review knowledge bank: `Terminator::Call { .. }` is how several
consumers already spell it, and a field walks past an exhaustive match.

**What this does not close.** A call below something C leaves unsequenced gets
no element, so `x = (free(p + *p), 0) + g(0);` is still reported. The enclosure
rule is why, it is ADR-0022's rule asked about the call rather than about the
point, and the reports it leaves standing are #178's class rather than a second
one: a proof C settles and this check does not give. The same holds for
`k(p, *p)`, whose read never waits at all, and for the stronger half of the
marker, which is left unconcluded for the reason above.

### Confirmation

`error[E0004]` at every reader of `Element` if the kind goes, which is nine
matches in three crates, and `error[E0027]` at the six that walk the IR if a
field is added to it, because each of those names the field rather than writing
`..`. The three in test helpers do write `..`, because what they filter on is
which kind it is.

Three mutations, each measured, each failing named cases and nothing else:

* The arm in `memory.rs` that reads the element doing nothing:
  `a_read_in_a_frees_own_argument_is_ordered_before_it` and
  `a_read_in_a_frees_argument_through_a_call_is_ordered_before_it` fail on their
  `.stderr`, with the `SC0402` back. That is the consumer half.
* The lowering emitting it whatever encloses the call: four cases lose their
  `SC0402` outright, which is silence about a read C leaves unordered:
  `an_unsequenced_use_the_check_meets_first_is_reported`,
  `a_write_through_a_pointer_the_check_meets_first_is_reported`,
  `a_discarded_read_the_check_meets_first_is_reported` and
  `a_condition_read_through_a_pointer_in_an_unsequenced_operand_is_reported`.
* The element concluding what `Element::Sequenced` concludes:
  `a_comma_inside_a_call_argument_orders_nothing_outside_it` fails on its
  `.stderr`, its warning having become an error.

The two halves are mutated apart on purpose. Mutating the lowering alone fails
the two new cases on their `.stdout`, along with every other artifact that holds
a call, which says nothing about whether any consumer reads the element: RK-039
in the review knowledge bank is that hazard.

Held by nothing: that the element is emitted for a call and not for something
else. Every call is one, so no case distinguishes a rule keyed on the node from
a rule keyed on anything a call happens to be.

### Consequences

* Good, because a program C defines stops being reported, and the report that
  went was one whose own note cited the clause refuting it.
* Good, because the fifth of Annex C's points is now expressed the way the other
  four are, and ADR-0022's "one rule with two spellings" consequence is spent.
* Bad, because it is a line in a hundred and sixteen artifacts, and two markers
  in one block can print at the same span: `free(p + *p);` has an
  `ArgumentsEvaluated` and a `Sequenced` at 5:5, meaning different points and
  licensing different things. The kind's name is what tells them apart.
* Bad, because the element says less than C does, and a reader who takes it for
  a sequence point will over-read it. Its doc comment is where that is answered.
* What would reverse this: an IR whose call carries its argument evaluations
  rather than linearising them into the block, where the point would need no
  element because nothing would be on the wrong side of it. That is a larger
  change to the IR than this question is worth, and ADR-0022 turned down the
  expression-tree version of the same argument.

## Pros and Cons of the Options

### A new element kind

* Good, because `error[E0004]` makes every consumer answer, and the answer that
  costs nothing to write is the conservative one.
* Good, because its doc comment is the one place the weaker meaning is stated,
  beside the kind it is weaker than.
* Bad, because the element list now has two markers that look alike in the
  artifact.

### An `Element::Sequenced` before the call terminator

* Good, because it needs no new kind and no consumer changes at all.
* Bad, because it concludes the half that cannot be concluded here, measured:
  `g((free(p), 0), *p)` becomes a proved use after free about an order C has not
  chosen.

### A field on `Element::Sequenced`

* Good, because one concept stays one kind, and `error[E0027]` still makes every
  reader mention the field.
* Bad, because mentioning a field is not reading it. A consumer that binds it
  and does not branch treats the weaker marker as a full barrier, which is the
  false proof above arriving quietly.

### A field on `Terminator::Call`

* Good, because it is where C puts the point, on the call itself.
* Bad, because RK-018: `Terminator::Call { .. }` is how several walks spell it
  today and a field added to a variant walks past an exhaustive match. ADR-0022
  turned this position down for a different proposal and the reason survives the
  change of proposal.

### Leave it

* Good, because it costs nothing and the failure is row 4 rather than row 6.
* Bad, because the report is about a program C defines, the note under it cites
  the clause that says so, and `--deny-unknown` makes it an error. A diagnostic
  that is wrong in a way the reader can check is how a check stops being read.

## More Information

* Issue #185, which carries the measurement this rests on.
* [ADR-0022](./0022-say-where-c-sequences-one-evaluation-before-another.md), whose
  fifth point this takes over, and whose enclosure rule this reuses.
* [ADR-0023](./0023-carry-a-read-forwards-to-the-free-it-is-unordered-against.md),
  the consumer that travels forwards and made the position insufficient.
* `docs/c-family.md`, which asks that two spellings of one C construct arrive as
  one shape.
