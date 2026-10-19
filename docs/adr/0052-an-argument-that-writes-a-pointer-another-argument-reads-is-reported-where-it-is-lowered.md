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

Chosen option: **report the conflict**, found by the lowering and carried by `Element::UnsequencedAccess` (named `UnsequencedArguments` until #525), which the memory check concludes `Unsafe` as `SC0409`.

The lowering has the tree, and the conflict is a fact about the tree: which pointer locals each argument writes by name, and which it reads. It is found where the call is lowered, for every argument against every other, and an element is written before the call's terminator for each local in conflict. The memory check reads nothing else about it: wherever its walk reaches the element, it is a finding, proved, because 6.5 p2 makes the call undefined on every execution that evaluates both accesses, and `Unsafe` says that some execution is undefined (`docs/safety-model.md`).

**A write that may not run is still a proof.** Where one of the two accesses is behind `&&`, `||` or `?:`, the walk counts it all the same: `g(q, c && (q = &x))` is undefined on every execution where `c` holds, which is what `Unsafe` means for a parameter. The cost is a pair no execution evaluates both of: a condition none satisfies, `g(q, 0 && (q = &x))`, and, found reviewing #525, conditions each satisfiable but never together, `x = (c ? (q = s, 0) : 0) + (c ? 0 : *q)`. C defines both and this refuses both; `a_write_that_never_runs_is_still_a_conflict` pins the first, and nothing about either can be silent.

**The store is a write too** ([#525](https://github.com/itsakeyfut/safec/issues/525)). C17 6.5.16 p3 sequences an assignment's store after the value computations of both operands and nothing more, so a write to the same local in the value is unsequenced against it unless a sequence point inside the value puts that write first: in the left operand of `,`, `&&` or `||`, the condition of `?:`, or a call's arguments. `q = q++`, `q = (q = s)` and `q = c ? (q = s) : s` are reported; `q = (q = r, q)` and `q = g(q++)` are not. A compound assignment reads its place as well (6.5.16.2 p3), so its place is walked as a value.

**Only pointer locals.** `k(i, i++)` and `i + i++` are undefined by the same paragraph and stay silent: neither is something a memory check is believed about. They are a cost this record names rather than a gap it hides.

**And an operator's operands, since [#525](https://github.com/itsakeyfut/safec/issues/525).** This record first asked only a call's arguments, calling every unsequenced operand pair a larger feature. Then `*q = (free(q), q = r, 0)` and `x = *q + (q = r, 0)` turned out to be the same silence, and the same walk answers them. So the search runs at an assignment, a binary operator that does not sequence, and a subscript, as well as at a call. The element's `Between` field says which, and the words follow it. An assignment's place is walked as a place: the local it names is the store's, sequenced after the value computations of both operands (C17 6.5.16 p3), so `q = (q = r, q)` is no conflict, while what computing the place reads is. A pair of accesses is found at the lowest node whose operands it lies across; an outer node can still report the same local over another pair, which is a second report rather than a missed one.

**And two parameters' lengths, since [#529](https://github.com/itsakeyfut/safec/issues/529).** C17 6.9.1 p10 evaluates the lengths on entry and gives them no order. A parameter's declarator is not a full declarator, so its end is no sequence point (6.7.6 p3). So `int a[g(p, 1)], int b[(p = 0, 1)]` is the same conflict as two arguments of one call, and it exited 0 because each length was walked on its own. `Lowering::lengths` now asks every length it walks against the others, with the same pairing a call's arguments get, and `Between::Lengths` words it as a parameter's length. A conflict inside one length is an operator's and was already found; its remedy, a statement before the expression, is one a declarator cannot hold, which is #531.

**Only what is named.** A write through a pointer, `*pp = 0`, is not found, and neither is one into a member or an element of an array. Missing one is the silence that is already there, not a new one.

### Confirmation

`error[E0004]` at every match over `Element` that names the kinds if this one goes, and at every match over `memory::Kind` if its variant goes; `words.rs` is one of those, so a conclusion with no code does not build.

Mutations, each failing named cases:

* The lowering finding nothing fails `an_argument_read_while_another_argument_writes_it_is_reported`, `a_dereference_in_an_argument_another_argument_writes_is_reported`, `a_freed_pointer_read_in_an_argument_another_argument_writes_is_reported`, `a_write_inside_a_comma_is_unsequenced_against_another_argument` and `two_arguments_writing_one_pointer_are_reported`, which go silent.
* The memory check's arm for the element doing nothing fails the same five.
* Leaving out an increment's write fails `an_argument_incremented_while_another_reads_it_is_reported`, and walking nothing below an assignment's place that is not a name fails `a_write_through_a_pointer_another_argument_writes_is_reported`.
* Counting an address, `&q`, as a read fails `an_address_taken_while_another_argument_writes_it_is_not_reported`.
* Comparing an argument with itself fails `a_write_and_a_read_in_one_argument_are_not_reported`.
* Counting only a read as the other access fails `two_arguments_writing_one_pointer_are_reported`.
* Not asking whether the local holds a pointer fails `an_integer_another_argument_writes_is_not_reported`.
* An assignment, a binary operator or a subscript pushing no conflict task fails `a_store_through_a_pointer_the_value_writes_is_reported` and the two beside it, `a_sum_reading_a_pointer_its_other_operand_writes_is_reported`, or `a_subscript_whose_index_writes_its_base_is_reported`, which go silent.
* Walking an assignment's place as a value fails `a_store_into_a_pointer_its_value_replaced_first_is_not_reported`, which reports a program C defines.
* Wording `Between::Operands` as a call's arguments fails those five on their `.stderr`.
* `Lowering::lengths` not asking its lengths for conflicts fails `a_pointer_one_length_reads_and_another_writes_is_reported` and `two_lengths_writing_one_pointer_are_reported`, which go silent, and passing `Between::Operands` there fails both on `.stdout` and `.stderr`.
* Counting a write in a binary operator or a subscript as ordered before the value fails `a_pointer_assigned_a_sum_that_increments_it_is_reported` or `a_pointer_assigned_an_element_whose_index_increments_it_is_reported`, and counting a comma's right operand as ordered fails `a_pointer_assigned_a_comma_that_increments_it_last_is_reported`, each going silent.

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
