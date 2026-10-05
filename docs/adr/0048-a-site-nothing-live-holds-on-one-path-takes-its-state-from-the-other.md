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
* The join must stay monotone and the height bounded, since a non-terminating walk is row 5.

## Considered Options

* **Keep the per-site join.**
* **A state per local and site**, what each local's view of an allocation is. Not prototyped: it squares the state table and every reader of a state would change.
* **Liveness, and a join that ignores a side where nothing holds the site**: a backward liveness pass per function; at each edge, a local the successor does not read before writing, and whose address has not escaped, holds nothing; at a join, a site nothing holds on one side takes the other side's state.

## Decision Outcome

Chosen option: **liveness, and a join that ignores a side where nothing holds the site.**

- **Liveness** is computed once per function, backwards to a fixpoint. A local is read wherever it appears in a place other than as the whole destination of an assignment or a call: the base of a projection, an `Index` operand, an operand of an rvalue, a call's argument, a branch's condition, and the return place at `Return`. A local whose address is taken anywhere in the function is live everywhere, since a pointer may read it.
- **At each edge** (`Analysis::edge`), after what the edge already does, every local the successor does not read and whose address has not escaped is cleared.
- **At the join**, a site is held on a side if a local holds it, an allocation contains it, it is exposed or reachable to a pending call, a pending read names it, or a `realloc` fact remembers it; and every site is held on a side where a local is lost, loaded or foreign, since such a local may name anything. A site held on one side only takes that side's state; held on both, or on neither, the states join as before.

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

- The join joining every site as before fails `a_pointer_freed_and_set_to_null_on_one_arm_is_returned`, `a_pointer_freed_and_set_to_null_on_one_arm_is_read_after_a_null_test` and `a_pointer_freed_and_set_to_null_in_a_loop_builds`, which are refused.
- `Known::held_sites` leaving out `inside` fails `a_pointer_stored_on_the_arm_that_freed_it_is_doubted`, and leaving out what locals hold fails `a_copy_made_on_the_arm_that_freed_it_is_doubted`; each builds. Only an asymmetric case can hold one kind of holder: where both sides hold the site by the same kind, leaving that kind out makes it held on neither, and the states join as they always did, so the symmetric controls in the table do not fail under these mutations, measured.
- `live_in` leaving out a local whose address is taken fails `a_pointer_freed_on_one_arm_and_read_back_through_its_address_is_doubted`, which builds, and the unit test `a_local_whose_address_is_taken_is_live_everywhere`.
- `live_in` leaving out one kind of read fails that kind's unit test in `memory/transfer.rs`: a copy, a dereference, an `Index` operand, a write through the local, arithmetic, an `Evaluate`, a call's argument, a branch's condition.
- The edge clearing before its own `realloc` logic fails four `realloc` cases in `cases/exposure`.

### Consequences

* Good, because the idiom builds, and every control that frees and keeps a reader is refused as before.
* Bad, because the check now rests on liveness: a read liveness misses clears a local that is read later, and a dereference of a local holding nothing says nothing. The rule above over-counts reads for that reason, and each kind of read has a unit test.
* Bad, because the height gains one step per site: a site held on neither side is below every state.
* What would reverse this: a state per local and site, which would make liveness unnecessary for this question.

## More Information

* `Allocations::join` and `Allocations::edge` in [`crates/safec-ir/src/memory/transfer.rs`](../../crates/safec-ir/src/memory/transfer.rs).
* [ADR-0016](./0016-an-analysis-is-a-trait-and-an-unreached-block-has-no-value.md) is the edge hook and the height.
* [#264](https://github.com/itsakeyfut/safec/issues/264) carries the probes and the prototype measurements.
