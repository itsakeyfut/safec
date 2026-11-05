---
status: "accepted"
date: 2026-11-05
decision-makers: itsakeyfut
---

# A loop may run zero times unless its test is shown true on the way in, replacing what ADR-0057 says of a loop's back edge

## Context and Problem Statement

ADR-0057 decided that a loop's back edge creates no doubt: `Init::joined`
met `Unwritten` from before the loop with `Written` from the back edge as
`Written`, because joining them as paths meet doubted every loop that fills an
array, for the path that skips the body. It said what that costs: "a read
after a loop that runs zero times is silent". The architecture review of
2026-11-03 found the program: `int x; int *m[1]; int i = 0; while (i < n) {
m[0] = &x; i = i + 1; } int *q = m[0];` built in silence, and with `n <= 0` it
reads a local nothing wrote, where `clang --analyze` warns (#783).

This record replaces what ADR-0057 says of a loop's back edge, and nothing
else of it. What a forward join, a first turn and a read on an arm answer is
still that record's.

## Decision Drivers

* **A path no test excludes is a path.** Believing a loop runs is saying safe
  about the execution where it does not, the failure this project exists to
  avoid.
* **The doubt costs most where it is least needed.** Measured, joining the back
  edge as paths meet moved 20 cases, and the three it refused outright were
  each `for (int i = 0; i < 2; i++)` filling a local, a loop C runs twice.

## Considered Options

* Keep ADR-0057's rule, every back edge creating no doubt.
* Join every back edge as paths meet.
* Join every back edge as paths meet, except at a loop shown to run its body
  at least once by its own test of constants on the way in.

## Decision Outcome

Chosen option: the third, by the user's decision on #772 and #783. At a loop's
head the back edge joins as a forward join does, `Unwritten` with `Written`
as `Maybe`, unless `memory::runs::shown_to_run` shows the loop runs its body at
least once and leaves only by its head's test after a whole turn. There it
joins as ADR-0057 decided, so `for (int i = 0; i < 2; i++) m[0] = &x;` is not
doubted after it.

The proof is narrow on purpose, since a wrong one is a silence: one edge into
the head from outside the loop; one latch; no edge leaving the body early that
reaches anything the head's test leaves for; and the head's test, run over its
own elements, true where every value it reads is a constant from zero to
`INT_MAX` or an `int` local whose address is never taken, written once at
most by the head, and otherwise last given such a constant by a straight line
of blocks before the loop. The range and the type are there because the IR
carries no conversion: `char c = 200;` holds 200 there and not what C stores.
The address because a write through it is a write the head's own walk does
not see, `(i = 0, *p = 5, i < 1)`; the single write because two that C leaves
unsequenced are lowered in an order C did not choose, `(i = 0) < (i = 3)`.
Review found both shown to run.

**A loop shown to run hides its first turn from what leaves the body early**,
so the first-turn reading ADR-0057 gives a loop's body is given as well to
every block outside the loop that a block of its body other than the head
dominates: an arm that returns, `if (c()) return *m[0] == 0;` before the
body's write. That is a doubt, since such an arm may not run on the first
turn, by the user's decision on #783: kept out, the read was silent in a loop
shown to run, as on `main`.

### Confirmation

In `tests/cases/memory/`:

* `a_pointer_read_after_a_loop_that_may_run_zero_times_is_doubted` goes silent
  if `Allocations::runs_once` is true at every head, which is ADR-0057's rule.
* `a_local_array_filled_by_a_loop_is_not_doubted`,
  `a_pointer_local_assigned_by_a_loop_is_not_doubted` and the two outer-loop
  forms of them gain `SC0411` if `runs_once` is never true.
* `a_pointer_read_after_a_loop_its_constants_do_not_run_is_doubted` goes
  silent if `memory::runs::shown_to_run` takes a test it can evaluate as true
  whatever it answers.
* `pointers_read_after_loops_their_constants_compare_false_at_the_bound_are_doubted`
  goes silent, in one of its five loops, if `Lt`, `Gt` or `Eq` in
  `runs::compared` gains or loses its equality or turns its direction, and
  `pointers_read_after_loops_their_constants_compare_true_at_the_bound_are_not_doubted`
  gains `SC0411` if `Le`, `Ge` or `Ne` does.
* `a_pointer_read_after_a_loop_tested_on_a_char_is_doubted` goes silent if a
  `char` local is taken to hold the constant written to it.
* `a_pointer_read_after_a_loop_whose_counter_was_given_another_constant_is_doubted`
  goes silent if the walk back takes the first write it finds rather than the
  last.
* `a_pointer_read_after_a_loop_whose_counter_one_arm_set_is_doubted` goes
  silent if the walk back goes on past a block with two predecessors by its
  last, and `a_pointer_read_after_a_loop_whose_counter_each_arm_set_apart_is_doubted`
  if it goes on by its first.
* `a_pointer_read_after_a_loop_whose_counter_a_call_returned_is_doubted` goes
  silent if the walk back passes over a write of what is not a constant.
* `a_pointer_read_after_a_loop_whose_counter_was_written_through_its_address_is_doubted`
  goes silent if a local whose address is taken is taken to hold a constant.
* `a_pointer_read_after_a_loop_whose_test_assigns_a_char_is_doubted` goes
  silent if a `char` the head reads is taken to hold what the head wrote.
* `a_pointer_read_after_a_loop_whose_test_writes_its_counter_through_an_alias_is_doubted`
  goes silent if a local whose address is taken is asked only where the head
  has not written it.
* `a_pointer_read_after_a_loop_whose_test_writes_its_counter_twice_unsequenced_is_doubted`
  goes silent if a local the head writes twice keeps its second value, and
  `a_pointer_read_after_a_loop_whose_test_writes_its_counter_twice_in_sequence_is_doubted`
  if the head reads the first of its writes rather than the last.
* `a_pointer_read_on_an_arm_that_leaves_a_loop_that_runs_before_its_write_is_doubted`
  goes silent if no block outside the loop is read as of its first turn.
* `a_pointer_local_read_after_a_loop_that_may_run_zero_times_is_doubted` goes
  silent if `memory::assigned` asks no loop's exit edge as of its first turn,
  and `a_pointer_local_assigned_by_a_loop_its_constants_run_is_not_doubted`
  gains `SC0411` if it asks one of a loop shown to run.

The frontend reads neither `continue`, `break` nor `goto`, so no C program
reaches the latch's refusal or the early exit's, and IR built by hand in
`memory::runs`'s tests holds them:
`a_loop_with_two_latches_is_not_shown_to_run` fails if the latch count is not
asked, `a_loop_left_from_its_body_for_where_its_test_leaves_is_not_shown_to_run`
if an early exit is not, `a_loop_left_from_its_body_for_what_follows_it_is_not_shown_to_run`
if only one that reaches where the test leaves is refused, and
`a_loop_left_from_its_body_by_a_return_is_shown_to_run` if every early exit is.
The walk back stopping at a call whose result lands in the local itself is
held by nothing, since the lowering puts a call's result in a temporary and
copies it.

### Consequences

* Good, because a loop the check cannot show runs is read as one that may not.
* Good, because the loops the corpus fills locals and blocks with, counted by
  constants, keep their answer.
* Bad, because a loop C runs but whose test this cannot evaluate, `n` being a
  parameter shown positive by an earlier test, is doubted after it.
* Bad, because a read on an arm that returns from inside a loop is doubted
  where the arm's own test excludes the first turn, `if (k == 1)`: twelve corpus
  cases gained that doubt.
* What would reverse this: a check that follows integer values far enough to
  show a loop runs wherever C makes it run.

## Pros and Cons of the Options

### Keep ADR-0057's rule

* Good, because it moves nothing.
* Bad, because the zero-turn read is silent, which is the failure this record
  exists for.

### Join every back edge as paths meet

* Good, because it is one rule, and nothing has to be shown.
* Bad, because a loop counted by constants that fills a local is refused: three
  corpus cases, and every such loop a reader writes.

### Except where its test is shown true on the way in

* Good, because it keeps those loops and doubts the rest.
* Bad, because the proof is code whose mistake is a silence, which is why it
  refuses every shape it does not need.

## More Information

* `crates/safec-ir/src/memory/runs.rs`, the proof.
* `dataflow::Analysis::join_back`, which is handed the head so that an analysis
  can ask which loop it is joining.
* [ADR-0057](./0057-a-pointer-read-out-of-fresh-memory-is-reported-where-it-is-read.md),
  whose back-edge rule this replaces.
* `memory::assigned`, which asks a pointer local whose address is never taken
  by name, and reports a read some paths assigned only after a branch edge that
  certainly leads to it unassigned. A loop's test is such a branch as of its
  first turn, at the edge that leaves the loop, unless the loop is shown to
  run, by the user's decision on #783: kept to the every-turn state, which the
  back edge assigns, `while (i < n) q = &x; r = q;` was silent there.
