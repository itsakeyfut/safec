---
status: "accepted"
date: 2026-09-14
decision-makers: itsakeyfut
---

# Record each half of an escape where its subject lives

## Context and Problem Statement

Taking a local's address means two things at once, and the memory check had a
slot for only one of them. `int **pp = &p;` says that **this check's knowledge
of `p` may be stale**, because whoever holds `pp` can put a different pointer
there, and it says that **the allocations `p` may hold may be freed through the
alias**, because whoever holds `pp` can read `p` and call `free`. The first is a
fact about one local; the second is a fact about the heap, and every other local
holding those allocations is affected by it.

Both were written into `Known::state`, the per-site table, by marking every site
the escaped local reached as `SiteState::Unknown`. That answered the second
correctly and the first only by accident: a site is shared, so the fact about
one local's knowledge was being recorded against allocations that other locals
hold, and a local was proved or not according to what its allocations had been
marked with rather than according to what was known about the local. #161 is
where that surfaced: `int **pp = &p; free(p); *pp = q; *p = 1;` reported
`error[SC0402]`, a **proof** of a use after free, about a program in which the
write through `pp` may well have replaced the pointer before the read.

## Decision Drivers

* [`docs/safety-model.md`](../safety-model.md#safe-unsafe-unknown) puts silence
  at the bottom of the list, so a change that trades a false positive for a
  silence is a bad trade however much more precise it looks.
* Reporting asymmetry: a local reaching no site is reported as
  `Reached::Lost`, while a local reaching one live site is not, so widening a
  may-set can make this check say *less*. Where a fact is kept therefore
  decides which programs go quiet.
* Three more lattices are to be written against this shape: ownership, lifetime
  and thread. Whatever rule this settles is the one their authors will copy.

## Considered Options

* **Both facts, each in the slot its subject lives in.** `escaped` stays a
  lattice field on the local and is read at the report; the site marking stays.
* **One fact, on the local.** Move the marking onto the local, read it at the
  report, and delete the writes into the site table.
* **One fact, on the site.** Leave it as it was: mark the sites and read
  nothing at the report.

## Decision Outcome

Chosen option: **both facts, each in the slot its subject lives in**.

`Known::escaped` already records that an address was taken. `Known::reached_by`
answers a local's sites plus one `Reached::Lost` when that bit is set, and both
readers, `used` and `Allocations::touching`, ask it. That is the fact about the
local, answered where a local is asked about. `Known::unproved` still marks the
sites an escaped local reaches, at the four places a local is given something
and again over the merged value in `join`. That is the fact about the heap,
answered where the heap is asked about.

**A local reaching no site answers nothing, escaped or not.** `used` already
says nothing about a place it follows no allocation for, however it came to
follow none, and an address taken is not a reason to break that: a pointer this
check never had a site for is an indeterminate pointer, a different defect with
a check of its own that does not exist yet. Answering `Reached::Lost` there
instead was written first and put `perhaps after the free` on programs that
free nothing at all, `--deny-unknown` included. This corner is the one a later
lattice is most likely to get wrong, because the distrust is real and the thing
to be distrustful about is missing, so it is stated here rather than left to be
read off the code.

Nothing is added to the lattice value, so `Analysis::height` is unchanged.

### Confirmation

Half of the rule is held by the compiler. Reporting takes `&Known` and the
transfers take `&mut Known`, so spelling `Known::reached_by` as `&mut self` is
`error[E0596]` at both call sites: a reader cannot quietly become a transfer.
It is two walls rather than one. `rustc` suggests widening
`Allocations::touching`'s parameter, and taking that suggestion is
`error[E0308]` where `reported` calls it holding a `&Known`.

The rest is one named test per rule, each of which fails under the mutation
beside it and under nothing else. Every one frees or reads through a **second**
local that shares the allocation, because a report about the escaped local
itself now stands on `reached_by` whatever the sites say, and would not notice
the site marking going.

| Mutation in `crates/safec-ir/src/memory.rs` | Named test that fails |
|---|---|
| `reached_by` answers `Reached::Lost` for an escaped local whose set is empty | `a_pointer_written_through_an_alias_the_check_does_not_follow`, which frees nothing and is silent |
| `reached_by` answers an escaped local's sites as `Reached::Lost` instead of listing them | `a_free_through_an_escaped_local_is_seen_by_a_sharer`, whose proved double free drops to two suspicions |
| `reached_by` never pushes `Reached::Lost` | `a_pointer_replaced_through_its_alias_after_a_free`, which goes back to a proved `error[SC0402]`, and `a_pointer_replaced_through_its_own_address`, which goes back to a proved `error[SC0401]`, and `an_escaped_local_read_twice_at_one_span`. The first two proofs are about a pointer a write through the alias may have replaced first |
| drop `unproved` at the `Rvalue::Address` arm | `an_allocation_shared_with_a_local_whose_address_escaped` |
| drop `unproved` after the `Element::Assign` match | `an_allocation_given_to_an_escaped_local_after_the_escape` |
| drop `unproved` at the call's destination in `terminator` | `a_call_into_a_local_whose_address_escaped` in `crates/safec-ir/tests/freed.rs` |
| drop `settle` from `join` | `an_escape_on_one_arm_and_a_shared_allocation_on_the_other` |

The corpus cases are in `crates/safec/tests/cases/` and are named in the table
in `crates/safec/tests/cases.rs`, per
[ADR-0007](./0007-list-the-test-cases-rather-than-discovering-them.md).

### Consequences

* Good, because the check no longer proves a use after free about a local whose
  address something else holds. That was a false `error` on a program with no
  defect in it.
* Good, because the two facts can now move independently. Making the heap fact
  more precise, which is what #162 and #164 are about, no longer risks the
  local fact going with it.
* Good, because what a local reaches now has exactly one producer. #164 wants a
  fact about a *set* of allocations that no member carries, and a set-level
  answer has to **override** the per-member ones, which only one producer can
  do. With the two readers computing their own answers, adding it to one and not
  the other builds cleanly and makes a real double free exit 0 in silence: that
  was measured on a working prototype during review. This shape makes it
  unwritable.
* Bad, because a local whose address has been taken can never again be the
  subject of a proved `SC0402`. Every report about it is a warning, and
  `--deny-unknown` is what turns it into a failure.
* Bad, because the site marking now looks redundant to a reader who only tries
  it against the escaped local: four of the five mutations above break nothing
  at all unless a second local shares the allocation. That is why the tests are
  shaped the way they are, and why this record says so.
* What would reverse this: a relation saying *which* local an alias may write
  to, rather than a bit saying that one exists. Then the heap fact could be
  narrowed to the allocations actually reachable through that alias, and the
  local fact would still be read where it is read now.

## Pros and Cons of the Options

### Both facts, each in the slot its subject lives in

* Good, because each fact is answered where its subject is asked about, so a
  later reader changing one does not silently change the other.
* Good, because it is seven edits in one file and no signature outside
  `memory.rs` moves.
* Bad, because two mechanisms mean two things to keep right, and the second is
  easy to mistake for dead weight.

### One fact, on the local

* Good, because it looks like the tidier rule, and the suite, the corpus and
  six review lenses all passed it.
* Bad, because it is unsound. Deleting the site marking made three programs
  this compiler reports go silent, each measured against `main`: a copied
  `points_to` row carries none of the distrust, an escaped local handed to an
  opaque call stops tainting the allocation, and a free through the alias stops
  reaching a local sharing it. All three are the worst row of `CLAUDE.md`'s
  list.

### One fact, on the site

* Good, because it is what the code did and it costs nothing to keep.
* Bad, because a local is then proved by what its allocations were marked with
  rather than by what is known about the local, which is the false `error` in
  #161.

## More Information

* `Known::reached_by`, `Known::unproved` and `Known::settle` in
  [`crates/safec-ir/src/memory.rs`](../../crates/safec-ir/src/memory.rs).
* [`docs/diagnostics.md`](../diagnostics.md) records what this means for a
  reader of `SC0402`, which is where a user meets it.
* [ADR-0016](./0016-an-analysis-is-a-trait-and-an-unreached-block-has-no-value.md)
  owns the `Analysis` trait this lattice implements.
