---
status: "proposed"
date: 2026-09-26
decision-makers: itsakeyfut
---

# A `_Nonnull` parameter is believed by its body and checked at every call this compiler sees

## Context and Problem Statement

[`docs/roadmap.md`](../roadmap.md)'s Phase 5 asks for the first annotation, and
for as much machinery as nullability needs, and names the sharpest question in
the phase beside it: whether an annotation is trusted or checked. A trusted
annotation that is wrong turns `Unknown` into `Safe` with nothing said, which is
the bottom row of `CLAUDE.md`'s failure list.

The nullability check answers `Unknown` for every parameter today, because a
parameter's nullness is a caller's fact and nothing carries one:
`Nullability::on_entry` in `crates/safec-ir/src/nullability.rs` says so and
names [#134](https://github.com/itsakeyfut/safec/issues/134). Under
[ADR-0033](./0033-a-conclusion-this-analysis-could-not-prove-does-not-build.md)
that is an error by default, so `void f(int *p) { *p = 1; }` does not build, and
there is nothing a user can write to say what they know.

[ADR-0032](./0032-bound-what-is-unchecked-inside-a-declared-hatch.md) decided
the stance for a region: a promise at a boundary, read by the checked side, and
said that a trusted annotation is that boundary at the scale of one declaration.
This record is that sentence made concrete, and it is `proposed` because nothing
in the tree reads it yet.

## Decision Drivers

* `CLAUDE.md`'s ranking. Whatever is believed has to be believed where a person
  wrote it, and nowhere else.
* [`concept.md`](../concept.md)'s incremental migration. An annotation that
  removes nothing is one nobody writes.
* Replacing `clang` with `safec` must not change what a program does, which the
  `review-c-conformance` lens holds this project to. Measured with
  `clang 20.1.6 --target=x86_64-unknown-linux-gnu -O2`:
  `__attribute__((nonnull))` puts `nonnull` on the parameter and deletes the
  body's own `p ? *p : 7` test, and `_Nonnull` leaves the IR exactly as it is
  without it.
* C17 7.1.3 p1 reserves every identifier that begins with an underscore and an
  uppercase letter for the implementation, so a spelling there cannot take a
  name from a conforming program.

## Considered Options

The spelling:

* `_Nonnull`, `clang`'s nullability qualifier, written after the `*` it
  qualifies
* `__attribute__((nonnull))`, which GCC and `clang` both read
* a keyword of this compiler's own, `nonnull`, in the style
  [`safety-model.md`](../safety-model.md#annotations) sketches for `owner` and
  `borrow`

What it means:

* believed by the body and checked at every call this compiler sees
* believed by the body only
* checked at the calls only, and believed nowhere

## Decision Outcome

Chosen: **`_Nonnull`, believed by the function's body at its entry and checked
at every call this compiler lowers.**

**The body believes it.** A parameter declared `_Nonnull` enters the nullability
lattice as not null. Nothing else changes: it is read through
`Nullability::known` like every other local, so a parameter whose address is
taken answers `Unknown` whatever it was declared, which is RK-061 and is the
same mask that keeps the rest of the lattice honest; and it is a fact at entry
only, so `p = 0; *p = 1;` in the body is a proved null dereference.

**Every call is checked.** An argument passed to a `_Nonnull` parameter is asked
the same question a dereference is. Proved null is `Unsafe`, not established is
`Unknown`, and not null is nothing. So the obligation the body stopped carrying
moves to the caller, where it can be discharged by a test, by an address, or by
passing on a parameter that was itself declared `_Nonnull`.

**What is believed is what no call this compiler saw can reach**: a caller in
another translation unit, or one compiled by something else. That is the whole
of what the promise rests on, it is written at a named place in the source, and
it is the property ADR-0032 asks of a hatch. It is also the reason every
declaration of one function has to agree about it: a header that omits the
annotation is how a caller in another translation unit goes unchecked, so a
disagreement between two declarations in one unit is refused rather than
resolved.

`__attribute__((nonnull))` is rejected on the measurement above. A program that
violates it means one thing under `clang` and another under `safec`, and the
difference is `clang` deleting a test the author wrote.

A keyword of this compiler's own is rejected because it takes an identifier C
gives to the program, so it refuses valid C (row 4), and because no other
compiler reads the file afterwards.

Believed by the body only is rejected because `g(0)` in the same file as `g`
would be silent, which is row 6 about a program this compiler can see all of.
`clang` is that option and warns only on a literal null; `int *q = 0; g(q);` is
silent there, measured.

Checked at the calls only is rejected because it removes nothing. It is the
option with no row 6 in it, and the one nobody would write an annotation for.

### Confirmation

**Nothing guards this yet, and the record is `proposed` for that reason.**
[#134](https://github.com/itsakeyfut/safec/issues/134) lands it, and this
section is to be rewritten in that change rather than after it. RK-017 in the
review knowledge bank is what happens otherwise.

What has to guard it, from the design on that issue:

* a call passing a null to a `_Nonnull` parameter is reported. Mutation: never
  ask about a call's arguments. This is the mutation the decision is about,
  because it is the one that makes the compiler believe something nobody
  proved.
* a `_Nonnull` parameter dereferenced in the body is not reported, beside the
  same function without it, which is. Mutation: accept the annotation and never
  read it at entry.
* a `_Nonnull` parameter whose address is taken is still reported. Mutation:
  read the entry fact past the escape mask.
* two declarations that disagree about `_Nonnull` are refused. Mutation: skip
  the comparison.

### Consequences

* Good, because the guarantee stays statable: proved within the translation
  unit, given the promises written at its function boundaries.
* Good, because every promise is a `_Nonnull` in the source, so an audit can
  count them with a search.
* Good, because a program `clang` compiles keeps compiling under `clang`, and
  means the same thing.
* Bad, because a caller in another translation unit is believed. That is row 6
  arrived at deliberately, and what bounds it is that a person wrote the
  promise at a place with a name.
* Bad, because it is stricter than `clang` in two places a user will meet:
  `clang` inherits `_Nonnull` from one declaration to another in silence, and
  accepts it on a local or a return type. Both are refused here. Both are row
  4.
* Bad, because `_Nonnull` is a `clang` extension and GCC does not read it.
  Mitigated by nothing yet; a macro is the usual answer and there is no
  preprocessor to define one.
* What would reverse this: a call this compiler lowers whose argument it cannot
  ask about, such as a call through a function pointer, which is refused today
  with `SC0304`. The day one is lowered, the annotation has to become part of
  the pointer's type or the call has to be refused, and this record has to say
  which.

## Pros and Cons of the Options

### `_Nonnull`

* Good, because it is reserved to the implementation by C17 7.1.3.
* Good, because `clang` reads it and emits the same code with or without it.
* Bad, because GCC does not read it.

### `__attribute__((nonnull))`

* Good, because GCC and `clang` both read it.
* Bad, because `clang` optimises on it, so a violation changes what the program
  does depending on which compiler built it.
* Bad, because it needs an attribute grammar this parser does not have.

### A keyword of this compiler's own

* Good, because it matches the sketch in `safety-model.md`.
* Bad, because it refuses a conforming program that uses the word as a name.
* Bad, because no other compiler can read the file.

### Believed by the body only

* Good, because it is the least to build.
* Bad, because a null passed in the same file is silent.

### Checked at the calls only

* Good, because nothing is believed.
* Bad, because nothing is removed, so nothing is gained by writing it.

## More Information

* [#134](https://github.com/itsakeyfut/safec/issues/134), where the design
  comment carries the detail this record leaves out.
* [ADR-0032](./0032-bound-what-is-unchecked-inside-a-declared-hatch.md), whose
  boundary promise this is at the scale of one declaration, and which stays
  `proposed` until [#210](https://github.com/itsakeyfut/safec/issues/210) lands a
  region.
* [ADR-0033](./0033-a-conclusion-this-analysis-could-not-prove-does-not-build.md),
  which is why an unannotated parameter costs a build.
* [`docs/safety-model.md`](../safety-model.md#annotations), whose sketch this
  departs from in spelling, and which is updated in the change that lands it.
