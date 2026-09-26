---
status: "accepted"
date: 2026-09-26
decision-makers: itsakeyfut
---

# A `_Nonnull` parameter is believed by its body and checked at every call in its translation unit

## Context and Problem Statement

[`docs/roadmap.md`](../roadmap.md)'s Phase 5 asks for the first annotation, and
for as much machinery as nullability needs, and names the sharpest question in
the phase beside it: whether an annotation is trusted or checked. A trusted
annotation that is wrong turns `Unknown` into `Safe` with nothing said, which is
the bottom row of `CLAUDE.md`'s failure list.

Before this record, the nullability check answered `Unknown` for every
parameter, because a parameter's nullness is a caller's fact and nothing carried
one. Under
[ADR-0033](./0033-a-conclusion-this-analysis-could-not-prove-does-not-build.md)
that is an error by default, so `void f(int *p) { *p = 1; }` does not build, and
there is nothing a user can write to say what they know.

[ADR-0032](./0032-bound-what-is-unchecked-inside-a-declared-hatch.md) decided
the stance for a region: a promise at a boundary, read by the checked side, and
said that a trusted annotation is that boundary at the scale of one declaration.
This record is that sentence made concrete.
[#134](https://github.com/itsakeyfut/safec/issues/134) landed it, and the status
moved with it.

**The title was narrowed by review.** It said "every call this compiler sees",
and a run handed two files sees a call in one to a function defined in the
other without checking it. The boundary is the translation unit, and the record
now says so wherever it used to say the wider thing.

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

* `_Nonnull`, what `clang` calls a type nullability specifier, written after the
  `*` it applies to
* `__attribute__((nonnull))`, which GCC and `clang` both read
* a keyword of this compiler's own, `nonnull`, in the style
  [`safety-model.md`](../safety-model.md#annotations) sketches for `owner` and
  `borrow`

What it means:

* believed by the body and checked at every call in its translation unit
* believed by the body only
* checked at the calls only, and believed nowhere

## Decision Outcome

Chosen: **`_Nonnull`, believed by the function's body at its entry and checked
at every call in the translation unit that declares it.**

**The body believes it.** A parameter declared `_Nonnull` enters the nullability
lattice as not null. Nothing else changes: it is read through
`Nullability::known` like every other local, so a parameter whose address is
taken answers `Unknown` whatever it was declared, which is RK-061 and is the
same mask that keeps the rest of the lattice honest; and it is a fact at entry
only, so `p = 0; *p = 1;` in the body is a proved null dereference.

**Every call is checked.** An argument passed to a `_Nonnull` parameter is asked
the same question a dereference is. Proved null is `Unsafe`, not established is
`Unknown`, and not null is nothing. A `_Nonnull` parameter a call passes no
argument for, which a call through `void g();` can do, is not established. So
the obligation the body stopped carrying moves to the caller, where it can be
discharged by a test, by an address, or by passing on a parameter that was
itself declared `_Nonnull`.

**What is believed is what no call in the translation unit can reach**: a caller
in another translation unit, including another file of the same `safec` run, or
one compiled by something else. That is the whole of what the promise rests on,
it is written at a named place in the source, and it is the property ADR-0032
asks of a hatch. It is also the reason every prototype of one function has to
agree about it: a header that omits the annotation is how a caller in another
translation unit goes unchecked, so a disagreement between two prototypes in one
unit is refused rather than resolved. `void g();` is not a prototype and says
nothing to agree with.

**The refusals hold at every safety level, `--safety off` included.** Where
`_Nonnull` may be written, and whether two prototypes agree, is the frontend
reading the language, and [`safety-model.md`](../safety-model.md) makes a level
decide which checks run and nothing else. So `--safety off` ignores what the
annotation means, since no check runs to believe or ask about it, and still
refuses it where it cannot apply. What that costs is that a file `clang` builds
with `_Nonnull` on a local is refused at every level, which is row 4.

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

Every mutation below was applied on its own, the whole workspace was run with
`--no-fail-fast`, and the file was restored from a copy rather than from git.
The tests named are the ones that failed; how many there were is not written
down, for RK-028's reason. The cases are in `crates/safec/tests/cases`.

**Believed by the body.** `Nullability::on_entry` in
`crates/safec-ir/src/nullability.rs` never reading `Function::nonnull` fails
`a_parameter_declared_nonnull_is_dereferenced_in_silence` and
`a_nonnull_parameter_passed_on_to_another_is_silent`. The unannotated half of
the pair is every `SC0403` on a parameter the corpus already had.

**Checked at every call.** This is the mutation the decision is about, because
it is the one that makes the compiler believe something nobody proved. Deleting
the `report_arguments` call from `nullability::findings` fails
`a_null_constant_passed_to_a_nonnull_parameter_is_proved`,
`a_local_proved_null_passed_to_a_nonnull_parameter_is_proved`,
`a_pointer_nothing_established_passed_to_a_nonnull_parameter_is_not_proved`,
its `--allow-unknown` sibling, and
`each_argument_to_a_nonnull_parameter_is_asked_about_on_its_own`. Reporting
only a proved null there, and dropping the unproven one, fails the last three of
those. Keying `findings`' `dedup_by` on the caret alone fails the last one
alone, which is two promises at one call collapsing into one report.

Three more rows of the same rule were found by review, each a way a call went
unasked with the whole suite green. Walking the arguments rather than the
parameters, so that one with no argument is skipped, fails
`a_nonnull_parameter_a_call_passes_no_argument_for_is_not_proved` alone.
Skipping an argument read through a pointer fails
`an_argument_read_through_a_pointer_is_asked_about_as_a_dereference_and_as_an_argument`
alone. And `report_arguments` names every terminator, so adding a second kind of
call to `Terminator` is `error[E0004]` there as well as at the eight places it
already was, measured by adding one; that is the guard on *What would reverse
this* below.

**Believed at the entry and nowhere else.** Having `Nullability::known` answer
`NonNull` for any parameter declared `_Nonnull`, ahead of the escape mask, is
the edit that believes the annotation over what the body did, and it fails
`a_nonnull_parameter_whose_address_escaped_is_not_proved` and
`a_nonnull_parameter_given_a_null_in_the_body_is_proved_null`.

**Carried to the IR.** `Lowering::body` building with `Function::new` rather
than `Function::with_parameters` fails the definition's cases, and
`Lowering::declare_one` building with `Function::declaration` rather than
`Function::declaration_with_parameters` fails every call case, among them
`a_nonnull_parameter_of_a_declaration_reaches_the_ir`. The parser keeping the
`_Nonnull` token and dropping its span fails every case above and
`a_nonnull_parameter_is_read_into_the_tree`. Not printing it in the tree fails
that case alone; not printing it in the IR fails every case that emits the IR.
Reading a parameter's annotation from the first derivation of its declarator
rather than the last fails `a_nonnull_after_the_last_star_of_a_parameter_is_read`
alone. `the_annotation_table_is_the_spellings_this_compiler_reads` in
`crates/safec/src/token.rs` holds the spelling, and
`a_word_spelled_as_an_annotation_is_one_and_its_neighbours_are_not` in
`crates/safec/src/lexer.rs` fails when `scan_word` never asks for one.

**Refused where it cannot apply.** Deleting the call to `Parser::placed` fails
the six `a_nonnull_on_..._is_refused` cases, one per reason that function gives.
Choosing the label for a block-scope function on whether the function type is
the declarator's own alone fails
`a_nonnull_on_a_parameter_of_a_parameter_that_is_a_function_is_refused` alone.
`a_nonnull_not_after_a_star_is_refused` and
`a_second_nonnull_on_one_pointer_is_refused` are held by `Parser::core`
instead, which is the other place `SC0204` is reported, and giving the second
the first one's label fails it alone.

**Every prototype agrees.** Replacing the call to `Lowering::agree` with nothing
fails `declarations_that_disagree_about_nonnull_are_refused` and
`a_definition_that_disagrees_with_a_later_declaration_about_nonnull_is_refused`,
which put the promise on the definition in each order, and
`a_declaration_that_says_nonnull_where_its_definition_does_not_is_refused`,
which puts it on the declaration. Comparing only the first parameter fails
`declarations_that_disagree_about_a_later_parameter_are_refused` alone, and
asking `agree` about `void g();` as well, which is how this shipped to review,
fails `an_unprototyped_declaration_does_not_stand_in_for_the_first_prototype`
alone.

**What nothing holds.** That `--safety off` ignores what the annotation means is
held by the level gate every check already sits behind rather than by anything
of this record's: `a_null_passed_to_a_nonnull_parameter_is_silent_at_safety_off`
documents the row and there is no mutation of this change that it alone fails.
A call through a function pointer is not checked, and cannot be reached, because
the lowering refuses it with `SC0304`; *What would reverse this* below is the day
it can.

### Consequences

* Good, because the guarantee stays statable: proved within the translation
  unit, given the promises written at its function boundaries.
* Good, because every promise is a `_Nonnull` in the source, so an audit can
  count them with a search.
* Good, because a program `clang` compiles keeps compiling under `clang`, and
  means the same thing.
* Bad, because a caller in another translation unit is believed. That is row 6
  arrived at deliberately, and what bounds it is that a person wrote the
  promise at a place with a name. It holds for two files handed to one `safec`
  run as well: each is lowered and checked on its own, and a call in one to a
  function defined in the other is not asked about. Measured, `a.c` calling
  `g(0)` through a prototype without the annotation and `b.c` defining `g` with
  it builds, and the program faults. Checking across the files of one run would
  need each unit's external signatures kept until the run ends; nothing decided
  that here.
* Bad, because a promise on a declaration and not on its definition is refused
  and also reported inside the body, which is built from the definition and so
  believes nothing: one mistake, two diagnostics. The refusal comes first.
* Bad, because the argument check inherits the dereference check's reading of
  a call: an argument is asked about as it stands once every other argument has
  been evaluated, so `g(q, q = &x)` with `q` null is silent. C17 6.5 p2 makes
  that call undefined, since one argument writes what another reads with nothing
  ordering them, and `h(*q, q = &x)` is silent on the dereference side in the
  same way. Found by review and not answered here; it is
  [#247](https://github.com/itsakeyfut/safec/issues/247).
* Bad, because it is stricter than `clang` in two places a user will meet:
  `clang` inherits `_Nonnull` from one declaration to another in silence, and
  accepts it on a local or a return type. Both are refused here, at every level.
  Both are row 4.
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
  boundary promise this is at the scale of one declaration.
  [#210](https://github.com/itsakeyfut/safec/issues/210) built its hatch as a
  function definition, and
  [ADR-0038](./0038-a-hatch-is-a-function-definition-and-what-it-could-not-prove-is-listed-rather-than-reported.md)
  is that decision.
* [ADR-0033](./0033-a-conclusion-this-analysis-could-not-prove-does-not-build.md),
  which is why an unannotated parameter costs a build.
* [`docs/safety-model.md`](../safety-model.md#annotations), whose sketch this
  departs from in spelling, and which is updated in the change that lands it.
