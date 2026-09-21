---
status: "accepted"
date: 2026-09-21
decision-makers: itsakeyfut
---

# A pointer operand decides what pointer arithmetic reaches

## Context and Problem Statement

`built_from` folds the operands of an `Rvalue::Binary` into what the result may
point at. It followed every operand that named a local, because nothing here
read a type and so nothing could say which of them was the pointer.

A parameter is an allocation site, so `p[i]` reached the site `i` is as well as
the allocation `p` holds, and a live site stops
[ADR-0020](./0020-a-free-of-a-may-set-is-a-fact-about-the-set.md)'s proof.
Measured on `main` at 93c9f44, release build, on the two corpus cases that
differ only in whether the subscript is a constant:

```console
$ safec --emit safety-ir --target x86_64-pc-windows-msvc a_constant_subscript_of_a_freed_pointer.c
error[SC0402]: this uses a value after it was freed       # p[0]
$ safec --emit safety-ir --target x86_64-pc-windows-msvc a_subscript_of_a_freed_pointer.c
warning[SC0402]: this may use a value after it was freed  # p[i]
```

Two programs with the same defect, and the proof is lost in the more common
spelling of the most common memory defect in C. The direction is towards
`Unknown`, so it costs a proof rather than a silence, and this record is about
giving it back without giving back more than that.

## Decision Drivers

* **This narrows a may-set**, which is the first rule here that does. Widening
  one is the move everybody reaches for; RK-045 in the review knowledge bank is
  what an emptied set does on the reporting path, and the three remaining safety
  axes will each meet this question on their first expression.
* **The authority has to be C rather than a belief about a type.** A type says
  what a local was declared as; what a check may drop needs a clause saying the
  value cannot be there.
* **The cost has to be nothing where C says nothing.** An analysis that trades a
  silence for a proof is on the wrong side of `CLAUDE.md`'s list.

## Considered Options

* **Follow only the pointer operands, and every operand where none is one.**
* **Follow only the pointer operands, always.**
* **Follow the left operand.**
* **Leave it, and take the union.**

## Decision Outcome

Chosen option: **follow only the pointer operands, and every operand where none
is one**.

C17 6.5.6 p8 says the result of adding an integer to a pointer points into the
object that pointer points into. So for `p + i` the integer cannot decide what
the result reaches, whatever that integer holds, and 6.5.2.1 p2 makes `p[i]` and
`i[p]` that same addition. This is a statement about the program: it does not
rest on believing that a local declared `int` holds no address, only on the
operand of an addition whose other operand is a pointer being an integer, which
6.5.6 p2 requires.

**Where neither operand is a pointer, no clause says anything**, and this
follows both as it always did. Two integers added together are not pointer
arithmetic, and narrowing there would be narrowing on nobody's authority.
Ranked on `CLAUDE.md`'s list, which is why this half exists at all:

| Option, for an operation with no pointer operand | Its failure |
|---|---|
| follow both, as today | row 4: a suspicion about a value C does not define this way |
| follow neither | row 6: a hand-built IR whose integer holds an allocation is dereferenced and nothing is reported |

RK-045 is the mechanism of the second row: a local reaching **no** site is
silence at a dereference, while a local reaching one live site is silence too,
so emptying a set does not announce itself anywhere. No C this frontend accepts
reaches the branch, which answers `error[SC0304]: cannot compile an expression
whose type is not known` for `q + r` and for `q - r`, measured. It is kept for
what it would cost the day something does reach it, which is the same argument
[ADR-0024](./0024-a-proof-survives-while-the-set-it-is-about-does-not-grow.md)
makes for its own unreachable line, and RK-064 is the entry that asks which
upstream rule is answering rather than leaving a reader to guess.

**The rule counts contributions rather than sides.** `i[p]` is `i + p` and is
the same program as `p[i]`, so the filter reads types and never positions.

**What ADR-0024 decides is untouched.** The proof still survives while the set
does not grow, and still drops where two operands contributed; what this record
changes is which operands did. One consequence is that no C program this
frontend accepts can now grow a may-set through arithmetic, so the corpus case
that held ADR-0024's rule is gone and a hand-built one holds it instead.

### Confirmation

`error[E0061]` at both call sites if the predicate is dropped from
`built_from`'s signature, which is what stops a third caller from being written
without answering this question. Measured, by giving the parameter up and
letting the body answer `false`.

`error[E0004]` at `Allocations::is_pointer` if a fourth kind of type is added,
because it is spelled as a `match` over every kind rather than as a `matches!`.
That is not style: a kind this does not recognise is **dropped** from an
addition that has a pointer beside it, and a dropped operand is a site nothing
reports. Measured by adding a variant to `Ty`, which stops three readers from
compiling: this one, `TranslationUnit::integer`, and the printer.

Each mutation below applied on its own, the whole workspace suite run with
`--no-fail-fast`, the tree restored, the binary rebuilt so that `cargo` does not
serve the old one, and the failure read rather than predicted.

| Mutation | Named test that fails |
|---|---|
| the predicate is never consulted, so every followed operand travels again | `a_subscript_of_a_freed_pointer`, `an_offset_by_an_integer_local_keeps_the_proof` and `an_offset_by_an_integer_parameter_keeps_the_proof`, each back to a warning |
| the filter keeps the operands that are **not** pointers | `a_subscript_of_a_freed_pointer` and `an_index_written_on_the_left_still_carries_the_pointer`, whose `SC0402` goes silent altogether, and six cases beside them. Not `a_constant_subscript_of_a_freed_pointer`: ADR-0021 folds `p[0]` away where the IR is built, so it has no addition for this to be wrong about |
| the filter takes the left operand rather than the pointer one | `an_index_written_on_the_left_still_carries_the_pointer` in `crates/safec-ir/tests/freed.rs`, which is `i[p]` and which no C this frontend accepts can write |
| the fallback goes, so an operation with no pointer operand reaches nothing | `an_addition_of_two_integers_carries_what_both_hold`, which drops from `Conclusion::Unsafe` to `Unknown` |
| two pointer operands keep the proof | `an_offset_by_a_second_pointer_loses_the_proof`, which becomes `Conclusion::Unsafe` |

The three hand-built cases are in `crates/safec-ir/tests/freed.rs` because the
frontend refuses each of their shapes, which is the same reason ADR-0028 keeps
`the_address_of_a_dereference_is_not_an_edge_to_the_local` there.

**One of the two call sites is held by nothing, and this says which rule is
answering instead.** The rule is one function and the predicate is passed at two
call sites, so RK-052's shape applies; measured, the `Deref` arm can stop
filtering altogether and the whole workspace stays green, while the same
mutation at the assignment arm fails the three cases above. What covers it is
[ADR-0017](./0017-record-each-half-of-an-escape-where-its-subject-lives.md):
`Held::writes_to` is written only where a local's address was taken, so the
target of a write through a pointer has always escaped, and an escaped local
answers `Reached::Lost` wherever a report is made. `*pp = q + i;` was built by
hand with the site carried on into a copy of the target, and the answer is
`Unknown` with or without the filter. RK-064 is the entry that asks for this
paragraph rather than for the words "held by nothing": the day #188 or #196
narrows what an escaped local is reported as, this line stops being covered, and
what it would cost then is a proof about a set the index widened.

### Consequences

* Good, because the proof is back in the spelling C programs use, and the two
  subscript cases answer the same thing about the same defect.
* Good, because #202 can be written: a write through a pointer this check cannot
  follow has to unprove every escaped local, and the rule costs what it should
  only once a write whose value cannot be a pointer can be told apart.
* Bad, because a may-set is now narrowed, and every later reader of
  `built_from` has one more thing to be right about. The fallback is the half
  that will look redundant.
* Bad, because the branch nothing reaches is guarded by a hand-built test and
  not by the corpus, so what holds it is a file a C programmer does not read.
* What would reverse this: a cast in the IR. `(int)p` and `(int *)n` make a
  local's declared type stop answering what it may hold, and this rule would
  then need the cast to say what it did rather than the declaration.

## Pros and Cons of the Options

### Follow only the pointer operands, and every operand where none is one

* Good, because the narrowing rests on a clause rather than on a belief.
* Bad, because the rule has two halves and the second is reachable from nothing
  this compiler builds.

### Follow only the pointer operands, always

* Good, because it is one line and reads as one rule.
* Bad, because it narrows where C has said nothing, and the failure is a
  dereference nobody reports: row 6, the one failure
  `docs/safety-model.md` reserves for the worst thing this compiler can do.

### Follow the left operand

* Good, because it needs no types at all.
* Bad, because C17 6.5.2.1 p2 makes `i[p]` and `p[i]` one program, so it answers
  differently about two spellings of the same thing, and the spelling it gets
  wrong is silent.

### Leave it, and take the union

* Good, because a growing may-set can never make a proof out of nothing, which
  is the direction that cannot be wrong about safety.
* Bad, because the cost is the proof on `free(p); p[i] = 42;`, which is what
  `docs/concept.md` is about being able to say.

## More Information

* `built_from` and `Allocations::is_pointer` in
  [`crates/safec-ir/src/memory.rs`](../../crates/safec-ir/src/memory.rs).
* ADR-0024 is the proof's algebra, whose rule this one feeds; ADR-0020 is what
  put the proof in `Held`; ADR-0021 folds a zero offset away so that `p[0]` and
  `*p` are one shape before this is asked anything.
* Issue #143 carries the measurement this was filed with, and #202 is the issue
  it unblocks.
