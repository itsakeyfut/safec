---
status: "accepted"
date: 2026-10-05
decision-makers: itsakeyfut
---

# A byte read out of memory is a load, and carries what was stored there

## Context and Problem Statement

C17 6.5 p7 lets an lvalue of character type access any object, so copying a pointer's object representation one `char` at a time is defined, and the copy is the same pointer. The memory check did not follow such a copy: `Allocations::may_be_pointer` answers `false` for `Ty::Char`, so a load of a `char` held nothing, `*d = *s` recorded nothing inside the allocation `d` points into, and a use after free through the copy built with exit 0 ([#257](https://github.com/itsakeyfut/safec/issues/257)). ADR-0040 wrote the exclusion into that predicate and listed it as a cost; this record decides it.

## Decision Drivers

* A use after free that builds is row 6 of the ranking. A report about a program C defines is row 4, and a proof that is wrong is still row 4 but is believed more.
* String code reads characters out of memory and hands them to calls this check cannot read more than any other C there is. A rule that turns every such call into a doubt about every stored pointer refuses the commonest programs.
* A byte holds part of a pointer at most. Nothing about it is a proof that it names an allocation.

## Considered Options

* **Leave a `char` load holding nothing**, as before.
* **Answer `true` for `Ty::Char` in `may_be_pointer`**, so a `char` is read as a pointer everywhere a place's type is asked.
* **Let a `char` load hold what was stored where it was read from, as an ordinary set of sites**, without the marker ADR-0045 gives a load.
* **Let a `char` load be a load**: it holds what was stored where it was read from, marked as possibly incomplete, as ADR-0045 has a pointer read out of memory do, while `may_be_pointer` keeps answering `false` for a `char` local.

## Decision Outcome

Chosen option: **a `char` load is a load**. `Allocations::read_through` marks a read of a `char` as `loaded`, as it does a read of a pointer, so it holds what `Known::inside` records for where it was read from and every rule ADR-0045 gives a load applies: a store of it records those sites where it lands, a dereference of what it was copied into is asked about them, and a free of it is a doubt. The lowering also hands a load straight to what takes it, as an operand of arithmetic or as an argument, so the same answer is given there: `built_from` lets a byte contribute what was stored where it was read, as a load does, though not as the pointer operand that narrows what is beside it, and `Allocations::read_out` answers a byte handed to a call with what was stored where it was read, and nothing more. `may_be_pointer` is unchanged, so a byte handed to a call is never read out as everything stored anywhere, which is what separates this from the second option. All three ask one predicate, `Allocations::reads_a_byte`.

The three rejected options, measured on twelve probes written for this record, each run on `main` and on a prototype of the option. The chosen option was first written in `read_through` alone, which built the two rows marked †; review found them, and the rule now reaches an operand and an argument too.

| probe | C defines it | `main` | `may_be_pointer` | sites, no marker | a load (chosen) |
|---|---|---|---|---|---|
| #257's program, copied into a table | no, a use after free | builds | refused | refused | refused |
| #257's program, copied into an escaped local | no | builds | refused | refused | refused |
| the same through a `char` temporary | no | builds | refused | refused | refused |
| the same with `free` of the copy | no | refused | refused | refused | refused |
| the copy into a local, read back named | no | builds | refused | refused | refused |
| a character handed to a call while a table holds a pointer | yes | builds | **refused** | builds | builds |
| a string length loop beside a table | yes | builds | builds | builds | builds |
| a string copied a byte at a time through a temporary, each byte handed to a call | yes | builds | **refused** | builds | builds |
| `free(c)` of a byte read out of a table | no, a constraint violation the frontend accepts | a doubt | a doubt | **a proved double free** | a doubt |
| #257's program with the copy `*d = *s + 0` † | no | builds | refused | builds | refused |
| bytes handed straight to a call that may keep them, `stash(*s)` † | no | builds | refused | builds | refused |
| a byte handed straight to a call, also through a pointer read out of a table, beside a table | yes | builds | **refused** | builds | builds |

- Holding nothing leaves the use after free silent.
- `may_be_pointer` answering `true` reads a `char` handed to any call as a pointer read out of memory, which `read_out` answers with everything stored anywhere, so a correct string routine beside a table of pointers is refused at the next use of the table. It also follows the two † rows, which is why the chosen option has to answer them in `built_from` and `read_out` as well rather than leave them to this.
- Sites without the marker give a byte the standing of a named pointer, so `free(c)` proved a double free of an allocation a byte cannot name.

The extra `SC0407` the escaped-local probe reports at `release(slot)`, beside the `SC0402` at the use, is reported by every option that follows the copy, and is ADR-0031's escaped local being written through a pointer this check cannot place; it is not this record's.

### Confirmation

The cases are in `crates/safec/tests/cases`, and every mutation is in `crates/safec-ir/src/memory/transfer.rs` or `built.rs`.

- Dropping the `char` term from `read_through`'s marker fails `a_use_after_free_through_a_pointer_copied_a_byte_at_a_time_is_reported`, `a_use_after_free_through_a_pointer_copied_a_byte_at_a_time_into_a_local_is_reported` and `a_use_after_free_through_a_pointer_copied_a_byte_at_a_time_through_a_temporary_is_reported`, which go silent, and `a_free_of_a_byte_read_out_of_memory_proves_nothing`, whose free of the allocation is then not asked about at all.
- Answering `true` for `Ty::Char` in `may_be_pointer`, the rejected option, fails `a_character_handed_to_a_call_beside_a_table_builds` and `a_string_copied_a_byte_at_a_time_builds`, which doubt a use after free and a double free. Both run with `--allow-unknown`, since [#333](https://github.com/itsakeyfut/safec/issues/333) doubts the pointer each reads out of the table in place, so the doubts are warnings rather than refusals.
- Holding the sites without the marker fails `a_free_of_a_byte_read_out_of_memory_proves_nothing` in `crates/safec-ir/tests/freed.rs`, which becomes a proof.
- Leaving the bytes out of `built_from`'s loads fails `a_use_after_free_through_a_pointer_copied_a_byte_at_a_time_by_arithmetic_is_reported`, and leaving them out of `read_out` fails `a_use_after_free_through_bytes_handed_to_a_call_is_reported`; each goes silent.
- Answering a byte in `read_out` with `Known::stored` fails `a_byte_handed_straight_to_a_call_beside_a_table_builds`, which doubts a use after free and a double free, as warnings for the same reason.

### Consequences

* Good, because a pointer copied a byte at a time is followed whether the byte is assigned, stored, added to something or handed to a call, and nothing a correct string routine does was refused on the probes.
* Bad, because a byte passed through a unary operator, `~~c`, carries nothing, as the `Rvalue::Unary` arm gives nothing for any value; it is a copy C defines and this does not follow.
* Good, because a byte is never a proof: everything a load of one holds is read beside the marker.
* Bad, because a byte holds every site that may be stored where it was read from, not the one whose bytes it is, so a copy of part of a table holds all of it. That is the direction a may-set is allowed to be wrong in.
* Bad, because a byte read out of memory that holds nothing stored is still a load, and a free of it is the doubt a free of any load is.
* What would reverse this: a frontend that rejects passing a `char` where a pointer is expected (C17 6.5.2.2 p2) removes the `free(c)` case; it does not remove the reason for the marker, which is that a byte names no allocation.

## More Information

* `Allocations::read_through` and `Allocations::may_be_pointer` in [`crates/safec-ir/src/memory/transfer.rs`](../../crates/safec-ir/src/memory/transfer.rs).
* [ADR-0045](./0045-a-pointer-read-out-of-memory-holds-what-was-stored-there-and-proves-nothing-with-it.md) is the rule a load follows; [ADR-0040](./0040-a-pointer-read-out-of-memory-reaches-what-was-stored-and-a-parameter-is-exposed-where-the-function-starts.md) wrote the exclusion this decides; [ADR-0031](./0031-a-write-this-check-cannot-pin-down-replaces-what-an-escaped-local-holds.md) is the precedent that a write through a `char` pointer reaches everything.
* [#257](https://github.com/itsakeyfut/safec/issues/257) carries the probes and the prototype measurements.
