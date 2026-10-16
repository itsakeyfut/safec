---
status: "accepted"
date: 2026-10-05
decision-makers: itsakeyfut
---

# A site nothing live holds on one path takes its state from the other

## Context and Problem Statement

`int *p = malloc(4); if (c) { free(p); p = 0; } return p;` is the free-and-null idiom, and C defines it on every path: where `p` was freed it is null. The memory check refused it with `SC0406`, and the read after `if (p != 0)` with `SC0402` ([#264](https://github.com/itsakeyfut/safec/issues/264)). The join is per site: the allocation is `Freed` on one arm and `Live` on the other, so it is `Unknown` after them, although on the arm where it was freed nothing that is read again still names it. The temporary the `malloc` call landed in still holds the site on both arms, which is why a join that only asked whether any local holds it changed nothing, measured.

## Decision Drivers

* A refusal of a program C defines is row 4; the idiom is one of the commonest ways C frees.
* Forgetting a site something can still reach is row 6: whatever decides that nothing holds it must over-count readers, never under-count them.
* The join must stay monotone and the height bounded, since a non-terminating walk is row 5. The join first written for this was not monotone: it asked whether each side held a site, and the side stored at a block held whatever every earlier arrival had, so an early value could decide what a later one kept and the answer depended on the order the solver walked in ([#478](https://github.com/itsakeyfut/safec/issues/478)). The state below is what made it monotone.

## Considered Options

* **Keep the per-site join.**
* **A state per local and site**, what each local's view of an allocation is. Not prototyped: it squares the state table and every reader of a state would change.
* **Liveness, and a join that ignores a side where nothing holds the site**: a backward liveness pass per function; at each edge, a local the successor does not read before writing, and whose address has not escaped, holds nothing; at a join, a site nothing holds on one side takes the other side's state.

## Decision Outcome

Chosen option: **liveness, and a join that ignores a side where nothing holds the site.**

- **Liveness** is computed once per function, backwards to a fixpoint. A local is read wherever it appears in a place other than as the whole destination of an assignment or a call: the base of a projection, an `Index` operand, an operand of an rvalue, a call's argument, a branch's condition, and the return place at `Return`. A local whose address is taken anywhere in the function is live everywhere, since a pointer may read it.
- **At each edge** (`Analysis::edge`), after what the edge already does, every local the successor does not read is cleared. A local whose address was taken is never among them, since liveness counts it live everywhere, and `Rvalue::Address` is the only way this IR takes an address.
- **At the join**, a site is held on a side if a local holds it, an allocation contains it, it is exposed or reachable to a pending call, a pending read names it, or a `realloc` fact remembers it; and every site is held on a side where a local is lost, loaded or foreign, since such a local may name anything. A site held on one side only takes that side's state; held on both, or on neither, the states join as before. Since #478 that is not a test the join makes: `Allocations::edge`, after liveness has cleared what the successor does not read, marks every site nothing holds `SiteState::Unheld`, which is the identity of `SiteState::joined`, and the join joins every state the same way. A side that holds nothing then says nothing, whatever arrived at the block before it.

Measured on probes written for this record, on `main` and on a prototype:

| probe | C defines it | `main` | chosen |
|---|---|---|---|
| `if (c) { free(p); p = 0; } return p;` | yes | refused | builds |
| the same, then `if (p != 0) return *p;` | yes | refused | builds |
| free and null inside a loop, then `if (p != 0) free(p);` | yes | refused | builds |
| `if (c) free(p); return p;`, no null | no | refused | refused |
| a copy kept, `q = p; if (c) { free(p); p = 0; } return q;` | no | refused | refused |
| stored first, `*t = p; if (c) { free(p); p = 0; } q = *t; return *q;` | no | refused | refused |
| a copy freed after, `if (c) { free(p); p = 0; } free(q);` | no | refused | refused |
| a parameter freed and nulled, then returned | yes | refused | refused |

The suite changes in one case, `a_free_on_a_branch_on_reallocs_result_nulled_on_one_arm_is_not_proved`, which loses a doubt at a `free(q)` only the arm where `q` was not nulled reaches; the doubt at the other free stays.

The last row is what this does not reach: a parameter's allocation is exposed where the function starts (ADR-0040), so it is held on both arms.

### Confirmation

The cases are in `crates/safec/tests/cases/memory`, and every mutation is in `crates/safec-ir/src/memory/`.

- `Allocations::edge` not marking an unheld site `Unheld` fails `a_free_on_a_branch_on_reallocs_result_nulled_on_one_arm_is_not_proved`, and so do `a_pointer_freed_and_set_to_null_on_one_arm_is_returned`, `a_pointer_freed_and_set_to_null_on_one_arm_is_read_after_a_null_test` and `a_pointer_freed_and_set_to_null_in_a_loop_builds`, which are refused.
- `Known::held_sites` leaving out `inside` fails `a_pointer_stored_on_the_arm_that_freed_it_is_doubted`, and leaving out what locals hold fails `a_copy_made_on_the_arm_that_freed_it_is_doubted`; each builds, among many other cases that fail with them. Before #478 only those two did, because a holder left out mattered only where one side held the site and the other did not; since `Allocations::edge` marks every unheld site `Unheld`, a holder left out makes a site that is still read lose its state at every edge, and the symmetric controls in the table fail too, measured. Of the other holders, leaving out a `realloc` fact's fails `a_pointer_reallocated_on_one_arm_is_doubted_after_its_null_test`, in the direction of a false report it costs, which [#494](https://github.com/itsakeyfut/safec/issues/494) is removing; leaving out the early return for a lost, loaded or foreign local, `exposed_after_call`, or a pending read's sites or `reachable` fails nothing, measured, and `Known::held_sites` says beside each what covers it and what was tried ([#481](https://github.com/itsakeyfut/safec/issues/481)).
- `live_in` leaving out a local whose address is taken fails `a_pointer_freed_on_one_arm_and_read_back_through_its_address_is_doubted`, which builds, `an_escape_on_one_arm_and_a_shared_allocation_on_the_other`, every case that reads a local back through its address after a join, and the unit test `a_local_whose_address_is_taken_is_live_everywhere`. The edge does not test `escaped` as well: measured, that test was answered by liveness every time, and a second guard nothing can fail is not one.
- `live_in` leaving out one kind of read fails that kind's unit test in `memory/transfer.rs`: a copy, a dereference, an `Index` operand, a write through the local, arithmetic, an `Evaluate`, a call's argument, a branch's condition.
- The edge clearing before its own `realloc` logic fails three `realloc` cases in `cases/exposure`, which lose the answer a branch on the result gives.

### Consequences

* Good, because the idiom builds, and every control that frees and keeps a reader is refused as before.
* Bad, because the check now rests on liveness: a read liveness misses clears a local that is read later, and a dereference of a local holding nothing says nothing. The rule above over-counts reads for that reason, and each kind of read has a unit test.
* Bad, as first written, because the join was no longer a union for a site's state: where the entry did not hold the site and the arriving value did, the state was replaced, and could move down, and what it moved to depended on what had arrived before. `SiteState::Unheld` replaced that with an identity (#478), and `a_site_nothing_holds_is_the_identity_of_the_join` in `crates/safec-ir/src/memory/parts.rs` holds it.
* Bad, because a pointer that holds a site beside a local's address is never a proof now (`Known::reached_by` adds `Reached::Partial`): on the path that gave it the address it reads the local. That was a false proof on `main` for `if (c) q = p; else q = &x; free(p); return *q;`, and this join made a second shape of it one; `a_pointer_to_an_allocation_freed_on_one_arm_or_to_a_local_is_doubted` and `a_pointer_to_an_allocation_or_a_local_freed_after_the_join_is_doubted` become proofs without it.
* What would reverse this: a state per local and site, which would make liveness unnecessary for this question.

## More Information

* `Allocations::join` and `Allocations::edge` in [`crates/safec-ir/src/memory/transfer.rs`](../../crates/safec-ir/src/memory/transfer.rs).
* [ADR-0016](./0016-an-analysis-is-a-trait-and-an-unreached-block-has-no-value.md) is the edge hook and the height.
* [#264](https://github.com/itsakeyfut/safec/issues/264) carries the probes and the prototype measurements.
