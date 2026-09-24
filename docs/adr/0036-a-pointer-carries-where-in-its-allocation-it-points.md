---
status: "accepted"
date: 2026-09-24
decision-makers: itsakeyfut
---

# A pointer carries where in its allocation it points

## Context and Problem Statement

C17 7.22.3.3 p2 makes `free` of anything but a pointer an allocation function
returned undefined. `memory.rs` follows *which allocation* a pointer reaches and
carried nothing about where in it, so `p + 1` was the same allocation as `p`
and a free of it was a free of that allocation. Measured on `main` at 66222f0,
debug build:

```console
$ safec --emit safety-ir --target x86_64-pc-windows-msvc interior.c   # p++; free(p);
$ echo $?
0
```

No diagnostic, which is a `Safe` about something nothing proved: the bottom row
of `CLAUDE.md`'s list. It was masked for the program, not for the report: at
the default `--emit executable` the backend refuses pointer arithmetic with
`SC0801`, while the memory check's diagnostics are emitted beside that refusal
rather than behind it, so whatever this check says a reader sees today. Issue
#227 carries the rest of the evidence.

## Decision Drivers

* **Row 6 has to be emptied, and nothing may be traded into it.** An offset the
  check cannot evaluate is the common case, `free(p + i)`, and it must not be
  silence.
* **A proof where C gives one.** `free(p + 1)` is the spelling the defect is
  written in, and `docs/concept.md` asks this compiler for proofs.
* **The shape three more lattices will copy.** Whatever this adds to `Held` is a
  pattern the lifetime, ownership and thread axes will read before they write
  their own, so the identity and the guard have to be deliberate.

## Considered Options

* **An offset per local with three values, `Zero`, `NonZero` and `Unknown`,
  reported under a code of its own.**
* The same, answering `Unknown` for every offset, a constant one included.
* The same, reporting only a constant offset and staying silent on the rest.
* An offset per site rather than per local.
* An offset in the IR.

## Decision Outcome

Chosen option: **an offset per local with three values, reported under
`SC0404`**.

`Held` gains `offset: Offset`. The only door into anything but `Zero` is
`Rvalue::Binary`, through `built_from`, which answers it last in `offset_of`:
`NonZero` where the operator is `+`, or `-` with the pointer on the left
(C17 6.5.6 p3 and p8), exactly one operand was followed, the other is a
constant that is **read** and is not zero, and the followed operand was itself
at the start; `Unknown` otherwise. A call's destination and a parameter are
the start of what they stand for, a copy carries what it copied, and every other
rvalue clears the local. That list is closed by `E0004` on `Rvalue` rather than
by a comment, which is RK-061's question answered at the IR's shape.

A `free` is then asked a second question beside the double free, in
`memory.rs::interior`, out of the same `Allocations::touching` answer. A
`Reached::Lost` among what the argument reached answers nothing, which is where
the escape pays for this: ADR-0017 answers `Lost` for an escaped local that
holds sites, so the offset, a positive claim about a local, is not believed once
its address is out. `NonZero` is `Conclusion::Unsafe`; `Unknown` is
`Conclusion::Unknown` with `Unproven::Offset`.

**`Zero` is what a local holding no site answers, and the guard for that is a
signature rather than a seed.** `Held::none` and `Held::clear` answer `Zero`, so
`int *p = 0; if (c) { p = malloc(4); } free(p);` joins two `Zero`s and says
nothing. Seeding `Unknown` instead, the way `writes_elsewhere` seeds `true`,
would put a warning about an offset nobody wrote on one of the commonest shapes
in C. What replaces the seed as the guard is `Held::hold` taking the offset as
an argument, so a producer of a site written later cannot be written without
answering: dropping it is `error[E0061]` at both call sites.

**`NonZero` survives a join.** Two paths that each moved the pointer off the
start both arrive off the start whatever the distances were, so
`if (c) q = p + 1; else q = p + 2; free(q);` stays a proof.

**The constant is read rather than trusted.** ADR-0021 folds `p + 0` away where
the IR is built, and `docs/c-family.md` records that nothing enforces it. A
frontend that skipped the fold would hand `offset_of` a literal zero, and
reading it answers `Unknown` where assuming would answer a false `NonZero`.
RK-067 is leaning on an invariant nobody enforces.

**The transfer is not touched.** An interior free still marks the site freed,
so `int *q = p + 1; free(q); free(p);` keeps its `error[SC0401]` and gains an
`error[SC0404]` on the line above. Dropping those sites to `Unknown`, on the
ground that an invalid free frees nothing, would cost the proof on a program
that is already reported.

**A parameter is proved on, and that is sound.** `void f(int *p) { free(p +
1); }` frees the start of an object only if a caller passed a `p` one element
before the start of one, which 6.5.6 p8 already makes undefined, so every
conforming caller leaves the call undefined. ADR-0027 is the record that says a
conclusion is about an execution this function has.

### Confirmation

Each mutation applied on its own to the tree this record landed with, the whole
workspace run with `--no-fail-fast`, and the file restored and touched before
the next, RK-013. No counts are given where a mutation takes neighbours with it,
for RK-028's reason.

* `offset_of` always answering `Zero`: every reporting case goes silent, among
  them `a_free_of_a_pointer_past_the_start_of_an_allocation`,
  `a_free_of_a_pointer_an_increment_moved`, which is the issue's own program,
  `a_free_of_a_pointer_before_the_start_of_an_allocation` and
  `a_free_of_a_pointer_offset_by_an_integer_is_not_proved`.
* `offset_of` never answering `NonZero`: the four proved cases fail, the three
  above that are proved and `a_free_offset_on_both_arms_is_still_proved`, and
  nothing else.
* the constant test turned to `== 0`: the same four.
* the constant not read at all, every constant taken for a move:
  `a_pointer_plus_a_literal_zero_is_not_proved_to_be_off_the_start` in
  `crates/safec-ir/tests/freed.rs`, alone. No C reaches it, because ADR-0021's
  fold leaves no literal zero in the IR, so it is written by hand.
* `-` taking its constant on either side:
  `a_pointer_subtracted_from_a_constant_is_not_proved_to_be_off_the_start` in
  `crates/safec-ir/tests/freed.rs`, alone.
* any operator accepted beside a constant:
  `a_pointer_scaled_by_a_constant_is_not_proved_to_be_off_the_start`, alone.
* `+` no longer reading a constant on its left:
  `a_free_of_a_constant_plus_a_pointer`, alone, down to `may`.
* `offset_of` no longer requiring the followed operand to be at the start:
  `a_free_of_a_pointer_moved_back_to_the_start_is_not_proved`, alone, which
  becomes a proved `SC0404` about a free C defines.
* `interior` folding `made` by keeping the first site's, and separately by
  keeping the last site's:
  `a_free_past_the_start_of_either_of_two_allocations_names_neither` each
  time, on the `allocated here` it gains. Its two allocations are made before
  the branch, because made on its arms each site meets the other arm's
  `Live(None)` at the join and neither mutation has anything to act on, which
  is RK-038.
* `Held::joined` leaving the field alone:
  `a_free_offset_on_one_arm_only_is_not_proved`, alone.
* `Offset::joined` answering `Unknown` for two `NonZero`s:
  `a_free_offset_on_both_arms_is_still_proved`, alone.
* `Offset::joined` answering `Zero` for a `Zero` and a `NonZero`:
  `a_free_offset_on_one_arm_only_is_not_proved`, alone, which goes silent.
* `Held::clear` answering `Unknown`: `a_free_of_an_allocation_made_on_one_arm_only`
  among a great many, because `Held::hold` joins its argument into what the
  cleared local answers and so every call's destination turns `Unknown`.
  `Held::none` answering `Unknown` takes every parameter the same way.
* `interior` believing an offset beside a `Reached::Lost`: five corpus cases
  about a write through a pointer whose target escaped, among them
  `a_write_through_a_pointer_whose_own_address_escaped`, each of which gains an
  `SC0404` about an offset the escape says nothing here can know.
* `Held::hold` without its offset: `error[E0061]` at the parameter in
  `Analysis::on_entry` and at a call's destination in `Analysis::terminator`.
* a fifth `Unproven` variant: `error[E0004]` at `memory_finding` in
  `crates/safec/src/driver.rs`.
* the double free pushed after the interior free rather than before:
  `an_addition_of_two_integers_carries_what_both_hold` and
  `an_offset_by_a_second_pointer_loses_the_proof`, whose two findings share a
  caret and come out in the other order.

**Held by a pair, and by nothing one at a time.** `interior`'s
`Reached::SetFreed` arm and its rule that no site answers nothing each cover
the other: `Known::reached_by` clears the sites whenever it answers `SetFreed`,
and `Allocations::touching` pushes a `Reached::Lost` for an argument reaching
no site. Removing either alone leaves the workspace green; removing both fails
`a_free_after_an_offset_that_kept_the_set_is_proved`,
`an_offset_by_an_integer_local_keeps_the_proof` and
`an_offset_by_an_integer_parameter_keeps_the_proof`, which RK-039 says to name
as a pair rather than read as unguarded.

**Held by nothing, and each says which rule is answering instead.**

* `Held::hold` joining its argument rather than assigning it. Both callers hold
  on a local `Held::clear` or `Held::none` has just left at `Zero`, and `Zero`
  joined with `Zero` is `Zero`.
* `Held::accumulated` answering `Unknown`. `built_from` assigns over it, and
  the other caller is a write through a pointer, whose target has always
  escaped, so `interior` never reads it: the escape is answering, which is
  RK-064's shape.

### Consequences

* Good, because the issue's program is `error[SC0404]` at exit 1, and so is
  every free of a constant offset.
* Good, because a free of an offset this check cannot evaluate is `Unknown`
  rather than silence, so it is an error in any compilation that asked for a
  check, ADR-0033, and a warning where unknown was allowed.
* Bad, because **`Zero` is not an identity of the join, only what a local
  holding no site answers.** `int *q = 0; if (c) q = p + 1; free(q);` joins
  `Zero` with `NonZero` and answers `Unknown`. That program frees null on one
  path and an interior pointer on the other, so the warning is not false, but a
  fourth value below the other three would say the null path contributes
  nothing and prove it. Three values is what the design asked for and this is
  what they cost; the failure is row 4.
* Bad, because no magnitude is carried. `free(p + 1 - 1)` is `Unknown`, because
  the second operation works from a base this check only knows to be off the
  start.
* Bad, because two existing cases now carry a third diagnostic:
  `a_read_in_a_frees_own_argument_is_ordered_before_it` and
  `a_read_in_a_frees_argument_through_a_call_is_ordered_before_it` free
  `p + *p` and `p + g(*p)`, whose offsets this check cannot evaluate. The first
  program writes zero through `p` first, so its offset really is zero and the
  report is a suspicion about a defined program, which is row 4.
* What would reverse this: a lattice that carries a distance, which would make
  `Offset` a special case of it, or a lifetime phase that knows what storage a
  value names and can say the same thing about a free of a local's address.

## Pros and Cons of the Options

### An offset per local with three values, under `SC0404`

* Good, because it proves the constant case and suspects the rest.
* Good, because it adds no square table to `Held`, which is the condition its
  own doc comment names for a packed bitset, and #173 would otherwise have to
  be paid.
* Bad, because of the `Zero` join above.

### `Unknown` for every offset

* Good, because it has one rule fewer.
* Bad, because it gives up the proof on `free(p + 1)`, which is the spelling
  the defect is written in. Row 4, and a worse row 4 than the chosen option's.

### Only a constant offset, silent on the rest

* Bad, because `free(p + i)` stays exit 0 with nothing said, which is row 6 and
  is the row this record exists to empty. Rejected outright.

### An offset per site

* Bad, because arithmetic applies its offset to the whole may-set at once, so
  every column of the row would answer the same thing, and it would be a third
  square table in a struct that is already measured at 2.8 GB for a 489-line
  function.

### An offset in the IR

* Bad, because `Rvalue::Binary` already says everything this needs, and an IR
  that carried an offset would be one four analyses have to keep in step.

## More Information

* `crates/safec-ir/src/memory.rs`: `Offset`, `Held::offset`, `offset_of`,
  `interior`, and `reported`, which asks both questions of one call.
* `crates/safec/src/driver.rs`: `INTERIOR_FREE` and its two rows in
  `memory_finding`.
* [`docs/diagnostics.md`](../diagnostics.md) for why `SC0404` is a class of
  its own.
* Issue #227, whose design comment this record carries the reasoning of.
