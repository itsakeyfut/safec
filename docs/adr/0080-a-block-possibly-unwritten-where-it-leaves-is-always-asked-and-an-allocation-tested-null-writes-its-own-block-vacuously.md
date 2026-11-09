---
status: "accepted"
date: 2026-11-10
decision-makers: itsakeyfut
---

# A block possibly unwritten where it leaves is always asked, and an allocation tested null writes its own block vacuously, replacing the null gates of ADR-0073 rule 3, ADR-0078 and ADR-0079

## Context and Problem Statement

ADR-0073 rule 3 doubted a `return` of a block possibly unwritten only where
the pointer returned was established not null there, and ADR-0078 and
ADR-0079 asked a block a store let leave the same way. The gate was there for
one program:

```c
int **b = malloc(8);
if (b) *b = x;
return b;
```

On the path where the allocation failed the site stays `Unwritten`, the join
with the path where the block was written makes it `Maybe`, and doubting every
`Maybe` refused this, the most common way C hands out a filled allocation.

Every place the not-null answer could not reach was a silence: a write under a
test that does not settle the pointer (#834), a pointer null on an outer
loop's first turn (#865), a loop left through its head (#867), a chained
assignment or a block stored twice (#868), each patched or held one at a time.
#870 measured that switching the gate off costs the idiom above, and that the
fact the gate stands in for, that this call's block does not exist on the edge
where its result is tested null, is about the call and not the site: given to
the site, a block an earlier turn made and memory still held was believed
written, and programs `main` reported went silent.

## Decision Outcome

1. **The gates come off.** A block possibly unwritten where it leaves, by a
   `return` or by a store that let it leave, is doubted wherever it is
   (`report_returned_unwritten`, `report_left_unwritten`).
2. **An allocation tested null writes its own block vacuously, where nothing
   else can name the site.** An allocation of indeterminate contents marks its
   result's `returned_by`, as a `realloc` does. On the edge where a local
   holding exactly that one site with that `returned_by` is tested null, the
   site is set `Written` if `Known::held_sites`, which over-counts by design
   (ADR-0048), finds no holder once that local, every local holding the same
   site with the same `returned_by`, and the branch's own condition are set
   aside (`branch_on_an_allocations_result`). The tested local then holds no
   block, so `if (!b) *out = b;` lets nothing leave.
3. **A join keeps `returned_by`** where the other side's local holds no block
   and nothing there holds the site, so a conditional allocation,
   `p = 0; if (c) p = malloc(8);`, still tests the call's result.
4. **A `realloc`'s site keeps ADR-0039's rule**, and the join rule does not
   reach it.

What ADR-0073 rule 3 says about which `return` is asked, and that a block
handed to a call is not, stands; only its not-null condition is replaced.
ADR-0078 and ADR-0079 are replaced whole.

### Confirmation

Each in `tests/cases/memory/`, with its mutation in its note in
`tests/cases.rs`:

* `a_block_left_unwritten_inside_a_test_of_its_pointer_is_doubted` (#834),
  `a_block_made_while_a_pointer_was_null_and_returned_on_a_later_turn_is_doubted`
  (#865), `a_block_stored_out_in_a_loop_left_through_its_head_is_doubted`
  (#867), `a_block_stored_out_by_a_chained_assignment_is_doubted` and
  `a_block_stored_out_twice_is_doubted_where_the_first_store_was_of_a_copy_taken_before_its_null_test`
  (#868) go silent when a block possibly unwritten is not asked.
* `a_block_written_on_the_path_its_allocation_succeeded_is_not_doubted`,
  `a_block_written_under_a_comparison_with_zero_and_returned_is_not_doubted`
  and `a_block_written_before_it_is_stored_out_is_not_reported_beside_its_failed_allocation`
  are reported when the null edge neither writes the site nor clears the
  tested local. Clearing alone answers these, since a site nothing holds on
  one side of a join takes the other side's state (ADR-0048); the vacuous
  write answers where a copy still names the site:
  `a_block_written_through_a_copy_tested_for_null_and_returned_is_not_doubted`
  is reported without it, and
  `a_block_written_through_a_copy_compared_with_zero_and_returned_is_not_doubted`
  is reported where the branch's condition counts as a holder.
* `a_block_kept_in_memory_from_an_earlier_turn_is_doubted_where_read_after_a_later_allocation_fails`
  and its two siblings go silent when the site is written whatever else holds
  it, and
  `a_block_a_local_kept_from_an_earlier_turn_is_doubted_where_a_later_turn_allocates_nothing`
  when the join keeps `returned_by` whatever holds the site.
* `an_allocation_made_on_one_arm_written_under_its_null_test_and_returned_is_not_doubted`
  is reported when the join keeps no `returned_by`.
* `a_pointer_stored_out_where_it_is_known_null_is_not_doubted` is reported
  when the tested local keeps its sites on its null edge.
* `a_free_on_a_branch_on_reallocs_result_nulled_on_one_arm_is_not_proved`
  loses its `SC0401` when the join rule reaches a `realloc`'s site.

### Consequences

* Good, because #834, #865, #867 and #868 are `SC0411`, and two doubts the
  corpus pinned on defined C go:
  `a_block_written_before_the_test_that_shows_its_pointer_not_null_is_not_doubted`
  and `a_pointer_read_after_a_null_test_that_guarded_its_write_is_not_doubted`.
* Good, because the not-null answers that patched the gate one turn and one
  store at a time (#856, #861, #863) go with it.
* Bad, because three doubts on defined C stay: two loops shown to run that
  null the pointer before it is returned (#874), and a null test kept in a
  flag (#875).
* Neutral, because a holder `held_sites` misses would be a silence, which is
  why the test asks the one answer ADR-0048 builds to over-count.

## More Information

* [ADR-0073](./0073-what-leaves-a-function-is-asked-by-how-it-was-declared-and-what-wrote-it.md),
  whose rule 3's not-null condition this replaces; its Confirmation for that
  condition is overtaken by this one.
* [ADR-0078](./0078-a-block-a-store-lets-leave-possibly-unwritten-is-doubted-where-the-stored-pointer-was-not-null-replacing-what-adr-0076-rule-3-leaves-silent.md)
  and [ADR-0079](./0079-a-stored-pointer-is-asked-whether-it-was-not-null-at-the-store-itself-beside-the-block-end-test-adr-0078-rule-1-reads.md),
  which this replaces.
* [ADR-0039](./0039-an-allocation-code-this-check-cannot-read-may-reach-stays-exposed-while-it-lives.md),
  whose trust in `realloc`'s result on a branch this widens to every
  allocation, under the holder test.
* [ADR-0048](./0048-a-site-nothing-live-holds-on-one-path-takes-its-state-from-the-other.md),
  why `held_sites` over-counts.
* #870, the experiment that measured the gates, and the programs its adversary
  wrote.
