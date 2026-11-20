---
status: "accepted"
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
* `Rank` and `Sign` hold only what a program can write, `Char` and `Int`,
  `Plain` and `Signed` since #1111, and grow with each sub-issue that reads
  one more. A rank declared before anything builds it is answered at every
  site already, so the sub-issue that makes it reachable is pointed at none
  of them; found in review of #1111, where the three ranks #1112 adds were
  first declared ahead of it.
* `_Bool` is not a rank here. It is Phase 9's unless a sub-issue needs it.

### Confirmation

A rank added later is `E0004` at every site that answers for an integer: a
third one was, at 25 sites in `safec-ir` and `safec-llvm` before the
frontend compiled, measured in a scratch build during #1111.
`every_target_is_what_clang_says_it_is` writes every rank's width out per
triple, measured with `clang 22.1.8 -dM -E`, and fails on any width wrong.
`minus_one_is_not_below_one_unsigned`,
`a_char_is_promoted_before_it_is_added`,
`a_long_compared_with_an_unsigned_int_depends_on_the_target` and
`unsigned_arithmetic_is_modulo_and_sees_no_sign` in
`crates/safec-c/tests/interp.rs` fail when a conversion of 6.3.1.1 or
6.3.1.8 is answered by rank alone, and `integer_arithmetic_is_what_clang_computes`
builds the same program with `clang` and compares.

### Consequences

* Good, because a width is asked once, of the target, whatever the type.
* Bad, because the representation change touches every integer site before a
  new width can be written, so it lands as a pull request of its own that
  changes nothing a program does.
* What would reverse this: an integer kind that is not a rank, which C17 has
  only in `_Bool` and the extended integer types.

## More Information

ADR-0013, #615.
