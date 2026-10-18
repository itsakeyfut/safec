---
status: "accepted"
date: 2026-10-18
decision-makers: itsakeyfut
---

# An argument that writes a pointer another argument of the same call reads is found where it is lowered, and reported by the memory check

## Context and Problem Statement

A call's arguments are lowered into the block before its terminator, and the terminator reads every argument that is a place where it stands once all of them have been evaluated. So an argument that another argument of the same call writes is read as the write left it: `g(q, q = &x)` with `q` null and `g`'s first parameter `_Nonnull` is silent, and so is `h(*q, q = &x)`, and so is `h(*q, q = &x)` after `free(q)` ([#247](https://github.com/itsakeyfut/safec/issues/247)). ADR-0037 records the first as a consequence.

C17 6.5 p2 makes each of those calls undefined, whatever `q` holds: one argument modifies `q` and another reads it, and nothing orders the two, since C17 6.5.2.2 p10 sequences the arguments only against the call and 6.5 p3 leaves them unsequenced against each other. The silence is about a program C does not define, at an order the IR chose.

## Decision Drivers

* The defect is the conflict, not the value. Any order this compiler picks for the read orders two arguments, which ADR-0026 already calls a proof C has not licensed, and asking about both orders misses the middle values when two arguments write the same local.
* What is reported has to obey what every other memory conclusion obeys: `--safety off`, the `safec_unchecked` hatch (ADR-0038), and the policy that decides what a conclusion costs.
* Every reader of the IR has to answer for whatever is added, and the one that does not catch up should land on a compile error rather than on a silence.

## Considered Options

* Report the conflict: the lowering finds it, a new `Element` kind carries it, and the memory check concludes it
* Report the conflict from the lowering directly
* Ask about the argument both before and after the other arguments' writes
* Treat such an argument as not established, in each analysis that reads it

## Decision Outcome

Chosen option: **report the conflict**, found by the lowering and carried by `Element::UnsequencedArguments`, which the memory check concludes `Unsafe` as `SC0409`.

The lowering has the tree, and the conflict is a fact about the tree: which pointer locals each argument writes by name, and which it reads. It is found where the call is lowered, for every argument against every other, and an element is written before the call's terminator for each local in conflict. The memory check reads nothing else about it: wherever its walk reaches the element, it is a finding, proved, because 6.5 p2 makes the call undefined on every execution that reaches it.

**Only pointer locals, and only a call's arguments.** `k(i, i++)` and `i + i++` are undefined by the same paragraph and stay silent: neither is something a memory check is believed about, and a check for every unsequenced operand pair is a frontend feature larger than this issue. They are a cost this record names rather than a gap it hides.

**Only what is named.** A write through a pointer, `*pp = 0`, is not found, and neither is one into a member or an element of an array. Missing one is the silence that is already there, not a new one.

### Confirmation

`error[E0004]` at every match over `Element` that names the kinds if this one goes, and at every match over `memory::Kind` if its variant goes; `words.rs` is one of those, so a conclusion with no code does not build.

Mutations, each failing named cases:

* The lowering finding nothing fails `an_argument_read_while_another_argument_writes_it_is_reported`, `a_dereference_in_an_argument_another_argument_writes_is_reported`, `a_freed_pointer_read_in_an_argument_another_argument_writes_is_reported` and `a_write_inside_a_comma_is_unsequenced_against_another_argument`, which go silent.
* The memory check's arm for the element doing nothing fails the same four.
* Counting an address, `&q`, as a read fails `an_address_taken_while_another_argument_writes_it_is_not_reported`.
* Comparing an argument with itself fails `a_write_and_a_read_in_one_argument_are_not_reported`.
* Counting only a read as the other access fails `two_arguments_writing_one_pointer_are_reported`.
* Not asking whether the local holds a pointer fails `an_integer_another_argument_writes_is_not_reported`.

`a_local_proved_null_passed_to_a_nonnull_parameter_is_proved` is the control: the same null, not overwritten, keeps its `SC0405`.

### Consequences

* Good, because the three programs in #247 are reported, and the report says what C says is wrong with them rather than a value one order would give.
* Good, because the conclusion goes through the same policy, levels and hatch as every other memory conclusion.
* Bad, because a kind is added that only the memory check reads, and every other reader writes an arm that does nothing.
* Bad, because the same paragraph's integer conflicts stay silent, and a reader who sees `g(q, q = 0)` refused may expect `k(i, i++)` to be too.
* What would reverse this: a frontend check for 6.5 p2 over every unsequenced operand, which would subsume this and could report from the frontend for every type; or an IR whose call carries its argument evaluations, as ADR-0026 names, where the order would not be chosen at all.

## Pros and Cons of the Options

### Report the conflict, through an element

* Good, because it does not depend on which order is read, and it reaches the dereference side and the `_Nonnull` side alike.
* Good, because a reader of `Element` that has not caught up does not compile.
* Bad, because it is a new kind in the IR for one question.

### Report it from the lowering directly

* Good, because it is the smallest change.
* Bad, because it is a memory conclusion that `--safety off` and the hatch do not reach, so it behaves unlike every other `SC04xx`, and a reader sees a refusal they turned off.

### Ask about the argument before and after the other arguments' writes

* Good, because it keeps the codes the issue asked for.
* Bad, because it needs a new kind for the `_Nonnull` question as well, since a read before the writes is not a parameter being passed.
* Bad, because two arguments writing the same local leave a value neither point sees, which is a silence.

### Treat such an argument as not established

* Good, because it is narrow.
* Bad, because every analysis that reads an argument has to do it separately, and the one that does not is silent.

## More Information

[#247](https://github.com/itsakeyfut/safec/issues/247); ADR-0022 and ADR-0026 for how a call's evaluations are ordered in the IR; ADR-0037 for the consequence this closes; [`docs/diagnostics.md`](../diagnostics.md) for `SC0409`.
