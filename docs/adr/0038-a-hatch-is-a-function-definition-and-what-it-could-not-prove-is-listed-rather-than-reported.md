---
status: "accepted"
date: 2026-09-26
decision-makers: itsakeyfut
---

# A hatch is a function definition, and what it could not prove is listed rather than reported

## Context and Problem Statement

[ADR-0032](./0032-bound-what-is-unchecked-inside-a-declared-hatch.md) decided
the stance: a declared hatch claims nothing about its body, carries a promise at
its boundary, and the checked side reads the promise. It left the spelling and
the form open until one annotation existed, and
[ADR-0037](./0037-a-nonnull-parameter-is-believed-by-its-body-and-checked-at-every-call-in-its-translation-unit.md)
has since made `_Nonnull` that annotation.

[ADR-0033](./0033-a-conclusion-this-analysis-could-not-prove-does-not-build.md)
makes every unproven conclusion an error wherever a check runs, so a program
whose unprovable part is confined to one function has nowhere to put it today.
[#210](https://github.com/itsakeyfut/safec/issues/210) is the work, and its
`/spec` narrowed it to the form decided here.

Three things are decided together because each constrains the others: what a
hatch is (a region or a declaration), how it is spelled, and where a conclusion
drawn inside one goes.

## Decision Drivers

* ADR-0032's own guard: a hatch changes what a conclusion is about and not what
  it concluded, and a mutation that turns the hatch into a suppression has to
  fail a named test. An implementation that *is* a filter leaves that mutation
  identical to the code.
* `CLAUDE.md`'s ranking. A proved defect that builds because it sits inside a
  hatch is the shape of row 6 even though somebody wrote the hatch.
* Replacing `clang` with `safec` must not change what a program does, which is
  the driver ADR-0037 already applied to a spelling.
* What the analysis already does at a boundary. Every callee other than `free`
  and `malloc` is `Callee::Opaque` in `crates/safec-ir/src/memory.rs`, a
  function defined in the same translation unit included, so a caller already
  assumes the worst of a call. A declaration hatch needs no new transfer.
* Measured on `main` at `ff53a50`: a local whose address is taken to hand to a
  wrapper stays escaped for the rest of the function, so a later, unrelated
  opaque call takes its proof away. `int **pp = &p; other(); free(p);` is
  `SC0401`, and the same program without the first statement builds. That is
  the cost of a declaration over a region, and it arises only where the hatch
  has to write a caller's local.

## Considered Options

The form:

* a function definition
* a region of statements inside a function
* both

The spelling:

* `__attribute__((annotate("safec_unchecked")))`
* `[[clang::annotate("safec_unchecked")]]`
* `_Pragma("safec unchecked")`

Where a conclusion drawn inside a hatch goes:

* an unproven one is listed by `--emit hatches` and not reported; a proved one is
  reported as it is anywhere else, and listed too
* every one is reported at a severity that does not fail the build
* none is kept; the hatch is found by searching for its spelling

## Decision Outcome

Chosen: **a function definition whose declaration specifiers begin with
`__attribute__((annotate("safec_unchecked")))`, whose unproven conclusions are
listed by `--emit hatches` rather than reported, and whose proved ones are
errors as they are everywhere else.**

**A definition, not a region, for now.** The boundary of a function is already
the place this analysis stops reading and assumes the worst, so the conservative
default ADR-0032 requires is built rather than to be built, and the boundary
promise is the prototype, which `_Nonnull` already lets a person write and every
call in the translation unit is already held to. A region needs the analysis to
decide what a stretch of statements could reach on the way out, and that is new
work in the transfer. The measurement above is what a definition costs, and it
is paid only by a hatch that writes a caller's local; a region stays open as the
answer to that, together with the vocabulary for narrowing what either form may
do, and both are [#249](https://github.com/itsakeyfut/safec/issues/249).

**`__attribute__((annotate(...)))`.** Measured with `clang 20.1.6 -std=c17`: it
is accepted with `-Wall -Wextra` and with `-pedantic-errors`, and the body's
code is unchanged; the one difference in the emitted IR is that the function
loses `local_unnamed_addr`, because the annotation table takes its address.
`__attribute__` begins with two underscores, so C17 7.1.3 p1 reserves it and
reading it takes no name from a conforming program. `[[clang::annotate]]` was
also accepted silently in C17 mode, but it is C23's syntax and nothing here was
measured about what `-pedantic-errors` or another compiler says of it.
`_Pragma` warns under `-Wall`, and reading it properly belongs to a preprocessor
this compiler does not have.

Only that one form is read. Any other attribute, or this one anywhere but at the
start of a function definition at file scope, is refused rather than skipped,
because a skipped attribute is one whose meaning this compiler silently
changed.

**Listed rather than reported, and only what was not proved.** The checks run
over a hatch's body exactly as over any other, and what they conclude is
attributed to the function it was concluded in. An `Unknown` inside a hatch is
not reported: it is written by `--emit hatches` under the hatch it belongs to,
which is what makes it a statement about the hatch rather than about the
program. An `Unsafe` inside a hatch is reported as an error, as it would be
anywhere, and listed as well. A hatch is for what cannot be proved; a program
this analysis proved undefined on some execution is not that, and letting it
build would be a proved defect shipped in silence. What that costs is ADR-0027's
row: an execution no caller makes is still a reason to refuse, inside a hatch as
outside one.

`--emit hatches` is also how the hatches are enumerated. A search for the
spelling finds them as well, as it finds every `_Nonnull`, and the listing is
what adds what each hatch is standing in for.

Reporting at a severity that does not fail is rejected because a hatch that
prints on every build is one people learn to stop reading, and because it
changes the acceptance criterion that a check inside a hatch does not report.
Keeping nothing is rejected because it is the suppression ADR-0032 rules out:
the implementation would be the filter the record's guard is written against.

**Believed and checked, as ADR-0037 answers it.** A hatch's boundary is its
prototype. A `_Nonnull` parameter of a hatch is checked at every call in the
translation unit, as any other is; what is believed is what the body does, and
it is believed by nobody, because callers already assume the worst of every call
they cannot read. #134's answer and this one are the same answer.

### Confirmation

Every mutation below was applied on its own to the tree as committed, the
whole workspace was run with `--no-fail-fast`, and the file was restored from
git. The tests named are the ones that failed; how many there were is not
written down, for RK-028's reason. The cases are in `crates/safec/tests/cases`.

**Where a conclusion goes.** This is the one the record is about. Having
`driver.rs::route` keep nothing in `hatched`, which is the hatch as a
suppression, fails `an_unproven_dereference_inside_a_hatch_is_listed_and_not_reported`
and `a_proved_double_free_inside_a_hatch_is_still_reported`. Reporting an
`Unknown` inside a hatch as well fails the first of those and
`an_unproven_dereference_inside_a_hatch_is_not_reported`. Answering `Unsafe` the
way `Unknown` is answered, so that a proved defect inside a hatch builds, fails
`a_proved_double_free_inside_a_hatch_is_still_reported` alone. `route` matches
every conclusion, so a fourth is `error[E0004]` there.

**Which function a conclusion is about.** Having either check's
`Finding::function` name the unit's first function, for the memory check's
findings, the nullability check's dereferences, or its arguments, fails
`a_function_after_a_hatch_is_still_answered_for`, whose hatch is that first
function: it is the silent direction, a conclusion about an ordinary function
credited to a hatch and not reported. Before that case was written, the
nullability half of this passed the whole suite.

**Carried to the IR.** `Lowering::body` never calling `Function::unchecked`
fails every case above that has a hatch in it and
`driver::tests::every_emit_kind_makes_something_of_a_program`. Not printing
`unchecked` fails the cases that emit the IR of a hatch, and not printing the
attribute in the tree fails `a_hatch_is_read_into_the_tree`.

**The listing.** Listing every function rather than every hatch fails
`a_program_with_no_hatch_lists_none` and
`every_hatch_is_listed_at_safety_off_with_nothing_under_it`. Dropping the sort
fails `what_a_hatch_concluded_is_listed_in_the_order_it_was_written` alone, and
calling a proof unproven fails the double-free case alone. File names in it are
written by `print.rs::dump_node`, whose escaping is held by that file's own test.

**The boundary.** Deleting the `report_arguments` call fails
`a_null_passed_to_a_nonnull_parameter_of_a_hatch_is_proved` beside ADR-0037's
own cases, so a hatch's prototype is held to the same rule as any other.
Having `Allocations::callee` answer that a call to a hatch touches nothing fails
`freeing_what_was_handed_to_a_hatch_is_not_proved`, which is ADR-0032's
conservative default at a hatch's boundary.

**The spelling and where it is read.** Each refusal fails the case named for
it and nothing else: on a declaration, a list of two, an argument that is not
a string, before a specifier (a second attribute, a parameter, a block), after
a specifier, and a block sending it to the expressions. `sema::resolve` never
comparing fails `an_attribute_other_than_annotate_is_refused` and
`an_annotation_other_than_the_hatch_is_refused`, and comparing the name alone
fails the second. Misspelling `__attribute__` in the annotation table fails
the table test, the lexer's neighbour test, and every case above.

**What nothing holds.** That `--emit hatches` is written on a run that failed
is held by nothing, as for `--emit ast`, and
`options.rs::EmitKind::survives_an_error` says so. A test that the name is an
identifier was measured to be one no mutation could break and was removed; the
parser says where such a name is refused instead.

### Consequences

* Good, because every hatch is a named function with a signature, which is
  what an audit wants to count and read.
* Good, because the conservative boundary and the promise it reads were already
  built and guarded, by ADR-0029, ADR-0031 and ADR-0037.
* Good, because a program `clang` compiles keeps compiling under `clang`,
  `-pedantic-errors` included, and means the same thing.
* Bad, because a hatch that must write a caller's local costs that local its
  escape for the rest of the function, measured above.
* Bad, because a hatch cannot yet say what it does not do, so anything handed to
  it is unproven afterwards. That is ADR-0032's default, and the narrowing
  vocabulary is [#249](https://github.com/itsakeyfut/safec/issues/249).
* Bad, because a wrong promise is believed. A caller in another translation
  unit is not checked against a `_Nonnull`, as ADR-0037 says, and a hatch's body
  is not checked against anything.
* Bad, because GCC's reading of `annotate` was not measured here.
* What would reverse this: a region hatch landing and making the definition
  form redundant, or a second compiler whose reading of the attribute changes
  what the program does.

## Pros and Cons of the Options

### A function definition

* Good, because the boundary is one callers already treat conservatively.
* Bad, because a hatch that writes a caller's local escapes it.

### A region of statements

* Good, because nothing has to escape to reach it.
* Bad, because deciding what the region could reach is new analysis.

### `__attribute__((annotate("safec_unchecked")))`

* Good, because `clang` accepts it under `-pedantic-errors` and emits the same
  code for the body.
* Bad, because it needs an attribute grammar, even one this narrow.

### `[[clang::annotate("safec_unchecked")]]`

* Good, because it is the standard attribute syntax from C23.
* Bad, because it is not C17's, and its treatment elsewhere was not measured.

### `_Pragma("safec unchecked")`

* Good, because it is standard C99 and would suit a region as well.
* Bad, because `clang -Wall` warns on it, and it wants a preprocessor.

### Unproven listed, proved reported

* Good, because the suppression mutation fails a test.
* Bad, because it is a new `--emit` kind to answer ADR-0035 for.

### Reported at a severity that does not fail

* Good, because nobody can miss it.
* Bad, because a hatch that is never quiet is not one.

### Kept nowhere

* Good, because it is the least to build.
* Bad, because it is a suppression.

## More Information

* [#210](https://github.com/itsakeyfut/safec/issues/210), whose design comment
  carries the detail this record leaves out.
* [ADR-0032](./0032-bound-what-is-unchecked-inside-a-declared-hatch.md), the
  stance this implements, which moves to `accepted` in the same change.
* [ADR-0037](./0037-a-nonnull-parameter-is-believed-by-its-body-and-checked-at-every-call-in-its-translation-unit.md),
  the promise a hatch's prototype carries.
* [`docs/safety-model.md`](../safety-model.md#annotations) and
  [`docs/frontend.md`](../frontend.md), which gain the hatch in the change that
  lands it.
