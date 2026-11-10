---
status: "accepted"
date: 2026-11-10
decision-makers: itsakeyfut
---

# A site made at most once an activation is written vacuously where its result is tested null, whatever holds it, replacing part of ADR-0080 rule 2

## Context and Problem Statement

ADR-0080 rule 2 writes an allocation's block vacuously on the edge where its
result is tested null, and only where `Known::held_sites` finds no other holder
of the site once the tested local and its copies are set aside. The holder test
is there because a site names every block its call made: on a loop's later
turn, an earlier turn's block may still be held, and writing the site would say
that block was written too.

A store before the test, `*out = b; if (!b) return;`, counts `out`'s memory as a
holder, so the null edge wrote nothing and the `return;` there saw the site
`Unwritten`. The function was said to let a pointer leave to memory nothing
wrote, where the block is written on every path it exists (#877). The design
first taken, recording in memory which call's result it holds, could not reach
that program soundly: memory a caller handed over is one cell that may hold
the caller's pointer too. #882 measured three answers.

## Considered Options

* Accept the false positive as a stated limit.
* Mark, on every value and in memory, the sites it names only as their current
  call's result, and set those aside (#882, mechanism 1).
* Take an assignment expression's value from what was stored rather than read
  it back, as a change to the lowering (#882, mechanism 2).
* Skip the holder test for a site whose every call is on no cycle of the graph.

## Decision Outcome

Chosen option: the fourth, by the author's decision on #882.

A call on no cycle runs at most once an activation, so its site makes at most
one block. On the edge where that call's result is null, that block does not
exist, and every local and every block of memory naming the site holds the
null the test read. `Allocations::made_once` holds it per local, which is the
site: every reachable block whose call writes the local is outside every loop
`Cfg::loops` lists. `branch_on_an_allocations_result` writes the site
vacuously where `made_once` holds or the holder test finds no holder. A site
made in a loop keeps ADR-0080's holder test unchanged.

### Confirmation

Each in `tests/cases/memory/`, with its mutation in its note in
`tests/cases.rs`:

* `a_block_stored_out_before_its_null_test_and_written_on_one_path_is_doubted`
  is "nothing wrote" and
  `a_block_stored_out_before_its_null_test_and_written_on_every_path_is_not_doubted`
  is reported when no site counts as made once.
* `a_block_kept_in_memory_from_an_earlier_turn_is_doubted_where_read_after_a_later_allocation_fails`,
  its two siblings,
  `a_block_a_local_kept_on_an_earlier_turn_is_nothing_wrote_where_a_later_allocation_fails`,
  `a_block_made_again_each_turn_and_filled_after_it_is_stored_out_is_doubted`
  and `a_block_one_turn_stores_out_and_a_later_turn_leaves_unwritten_is_reported`
  go silent or lose their `SC0411` when every site counts as made once.

### Consequences

* Good, because the answer is a property of the graph, computed once a
  function, and nothing new is kept in the state that a join or a door could
  forget.
* Bad, because a call on a cycle `Cfg::loops` misses would be a silence. It
  finds a back edge by the order blocks drain in, which covers any cycle; every
  one C reaches today comes from `while` and `for`, `goto` being refused.
* Bad, because #882 found two programs that `main` reported and this makes
  silent: `p = b; if (c) p = slot; *out = p;` with `slot` a local, and
  `p = b + 0;` where `b` may be null. `main`'s report on each was a false
  "nothing wrote" about `b`'s block standing in for a defect this check does
  not ask, a local's address that outlives it and arithmetic on a null
  pointer, filed as #883 and #884.
* Bad, because a block stored out before its null test in a loop,
  `out[i] = b; if (!b) return;`, is still doubted: the site's one record of
  what was written cannot say an earlier turn's block was.
  `a_block_stored_out_before_its_null_test_in_a_loop_is_doubted` holds it.

## More Information

* ADR-0080 rule 2 is replaced for a site made at most once an activation and
  stands for every other. Its status line says so.
* #882 records the three options measured, the adversary's programs, and what
  the rejected two cost.
