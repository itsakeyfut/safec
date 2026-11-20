---
status: "proposed"
date: 2026-11-20
decision-makers: itsakeyfut
---

# An integer type is a rank and a sign, and the target answers its width

## Context and Problem Statement

The AST has `Type::Int` and `Type::Char`, the IR `Ty::Int` and `Ty::Char`, and
the target answers the width and signedness of those two (ADR-0013). #615 adds
`short`, `long`, `long long`, `signed` and `unsigned`, and `long` is 32 bits
on Windows and 64 on Linux, so a width cannot be a property of the type's
name.

## Decision Drivers

* A kind of integer added later is a compile error wherever an integer is
  answered for, as a kind of type is today.
* What a type is worth stays the target's to say (ADR-0013).
* C17 6.3.1.1 orders the integer types by rank, and the usual arithmetic
  conversions (6.3.1.8) are written in ranks and signs.

## Considered Options

* One variant, `Integer { rank, sign }`, replacing `Int` and `Char` in both
  the AST and the IR, with `Rank` one of `Char`, `Short`, `Int`, `Long`,
  `LongLong` and `Sign` one of `Plain` (only for `char`), `Signed` and
  `Unsigned`.
* A variant per type beside the two there are.

## Decision Outcome

Chosen option: "one variant", by the owner's decision on #615.

* The target answers `Integer` (bits and signedness) for a rank and a sign,
  measured per triple with `clang 22.1.8 -dM -E` as the existing rows are.
* Replacing the two variants is `E0004` at every site that answers for an
  integer, which is the point: each says again what an integer of any width
  is to it.
* `_Bool` is not a rank here. It is Phase 9's unless a sub-issue needs it.

### Confirmation

Proposed until #1112 lands the rest. Since #1111, a sixth `Rank` is
`E0004` at 25 sites in `safec-ir` and `safec-llvm` before the frontend is
compiled, measured in a scratch build, and
`every_target_is_what_clang_says_it_is` reads `int` and `char` through
`Target::integer`. #1112 adds the target table against measurements
written out for every rank, and a case per conversion 6.3.1.8 makes.

### Consequences

* Good, because a width is asked once, of the target, whatever the type.
* Bad, because the representation change touches every integer site before a
  new width can be written, so it lands as a pull request of its own that
  changes nothing a program does.
* What would reverse this: an integer kind that is not a rank, which C17 has
  only in `_Bool` and the extended integer types.

## More Information

ADR-0013, #615.
