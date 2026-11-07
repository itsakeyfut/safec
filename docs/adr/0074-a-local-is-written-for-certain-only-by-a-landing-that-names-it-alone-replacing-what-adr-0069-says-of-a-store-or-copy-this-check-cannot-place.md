---
status: "accepted"
date: 2026-11-07
decision-makers: itsakeyfut
---

# A local is written for certain only by a landing that names it alone, replacing what ADR-0069 says of a store or copy this check cannot place

## Context and Problem Statement

A write to an allocation is certain only through a `Certain`, which only
`Landing::certain` makes (ADR-0063). A write to a local has no such type:
`Known::written_local` takes a bare `LocalIdx`, and what keeps a write that may
have missed a local from calling it is a guard each door writes by hand. Where a
door cannot prove one local, it hands the write to `Known::replaced`, the belief
for a call this check cannot read, that it writes every escaped local it may
reach (#782, as ADR-0069 carries it from ADR-0057), and that belief writes the
local for certain.

The architecture review of 2026-11-07 measured two programs that build silent
for it, each a read of a pointer nothing wrote:

- a library copy that may land in either of two locals writes the one it may
  have missed: with `g` either `&m` or `&n`, `memcpy(g, &px, 8); r = m;`
  prints nothing, where `*g = &x; r = m;` is `SC0411` (2047e8b1 fixed the
  store, not the copy);
- a store through a pointer read out of memory code this check cannot read
  writes the one local it may name: `*b = m; t = *pp; *t = 0; q = m[0];` with
  `b` and `pp` parameters prints nothing at any policy.

## Decision Drivers

* Each is a silence about a program C leaves undefined.
* A write that may have missed a local is a may-write; that a store or a copy
  is a certain write of the one local it names is a belief, and an unstated
  one.
* A block already has one answer to "certain", `Landing::certain`, which
  refuses a landing reached through memory this check did not record (#726).

## Considered Options

* A token for a certain write of a local, made only by a landing that names it
  alone, and a may-write elsewhere, the call belief named apart.
* The same token, with the call belief kept for a store or a copy.
* `Strong` as the token.

## Decision Outcome

1. **`WrittenLocal`**, in `memory::store` with a private field, is the only
   thing `Known::written_local` takes. Two places make one: a `Strong`, the
   local an assignment or a call's result replaces whole; and
   `store::written_alone`, handed the pointer, how many dereferences down the
   write lands and its targets, which asks for itself that there is exactly one
   local target and that every pointer the write goes through last names no
   allocation and holds nothing this check did not follow. Handed those
   premises by each door, the store two dereferences down was asked of the
   pointer it starts from and wrote a local for certain through one read out of
   a caller's memory; found by review.
2. **A store or a library copy that has no `WrittenLocal` is a may-write** of
   every local it may land in: `Init::may_write`, and every element of an array
   kept per element likewise. It no longer goes through the call's belief.
3. **The call's belief is named apart**, as a site's is
   (`Known::believed_written_by_unknown_code`), and only a call this check
   cannot read takes it. This is what replaces ADR-0069's reading that a store
   or copy this check cannot place writes an escaped local it may reach as a
   call does; for a call it stands.
4. **"Holds nothing this check did not follow" is five marks**: `lost`,
   `foreign`, `from_caller`, `lost_read` and `stale_read`, asked as one
   question on `Beyond`. Not `loaded`: a pointer read out of memory this
   function recorded all of is believed to reach what the record says, as a
   block's one container is (#726). Not `unreplaced_read`: it says a call may
   have left an old value in the slot read, which `Known::handed_below` asks,
   not that the address is one this check did not follow.
5. **`Known::written_all`**, which writes every allocation for certain where a
   call's result is stored through a pointer this does not place, writes them
   as a may-write. No C program reaches it: the lowering calls into a temporary.

### Confirmation

Each in `tests/cases/memory/`, with its mutation in its note in
`tests/cases.rs`:

1. `Known::written_local` handed a bare `LocalIdx` is `error[E0308]`, and a
   `WrittenLocal` built outside `memory::store` does not compile: the field is
   private there. `written_alone` taking any one-local landing, `[local, ..]`,
   fails `a_copy_that_may_land_in_either_of_two_locals_may_have_missed_each`
   and `a_store_through_a_pointer_to_either_of_two_locals_may_have_missed_each`;
   ignoring a block beside the one local fails
   `a_store_through_a_pointer_to_a_local_or_a_block_may_have_missed_the_local`;
   asking a write two dereferences down of the pointer it starts from fails
   `a_store_two_dereferences_through_a_pointer_read_out_of_a_callers_memory_may_have_missed_the_local`
   and `a_store_two_dereferences_through_a_pointer_to_a_local_or_a_block_may_have_missed_the_local`.
2. `WrittenBy::StoreOrCopy` writing a local for certain fails
   `a_copy_that_may_land_in_either_of_two_locals_may_have_missed_each` and
   `a_local_array_copied_into_through_a_dereference_of_a_pointer_read_out_of_memory_is_doubted`.
3. `WrittenBy::Call` made a may-write fails forty-four cases of the call belief
   (#782).
4. Leaving `from_caller` out of `Beyond::believed_to_reach_only_its_edges` fails
   `a_store_through_a_pointer_read_out_of_a_callers_memory_may_have_missed_the_local_it_names`;
   counting `loaded` fails
   `a_store_through_a_pointer_read_out_of_recorded_memory_writes_the_local_it_names`.
   Of rule 4's five marks only `from_caller` is held by a case; leaving out any of the
other four fails nothing, since where each is set the belief for a call this
check cannot read has already written the local. They are kept by the
author's decision, a mark kept being at worst a doubt too many. Rule 5 has no
case, since no C program reaches it.

### Consequences

* Good, because both programs above are `SC0411`, and a may-write calling
  `written_local` does not compile.
* Bad, because a store or a copy through a pointer a call wrote, or one read
  out of a caller's memory, that names one local array is now a doubt where it
  was believed written: eight corpus cases gain the doubt, each true, measured
  on a prototype, with no route-matrix row moving.
* What would reverse rule 4's exclusion of `loaded`: a measurement that a
  pointer read out of recorded memory reaches a local it does not name, which
  would doubt the block's belief (#726) as well.

## Pros and Cons of the Options

### The call's belief kept for a store or a copy

* Good, because no case moves.
* Bad, because both silences stay, behind a token that only names them.

### `Strong` as the token

* Good, because no type is added.
* Bad, because `Strong` means a replacement of the whole value: a write of one
  element of an array, `m[0] = &x`, is no `Strong`, and would become a
  may-write, doubting the commonest way to fill an array.

### Seven marks, `loaded` among them

* Good, because a pointer read out of memory is doubted too.
* Bad, because three proofs become doubts, measured, and a local is then asked
  differently from a block reached the same way (#726).

## More Information

* [ADR-0069](./0069-a-pointer-read-out-of-memory-nothing-wrote-is-reported-where-it-is-read-and-a-loop-may-run-zero-times-unless-it-is-shown-to-run.md),
  whose belief for a call this keeps and whose reading of a store or copy this
  check cannot place it replaces.
* [ADR-0063](./0063-the-memory-check-has-one-kind-of-place-one-kind-of-cell-and-one-description-of-a-write.md), the
  `Certain` and `Strong` this follows.
