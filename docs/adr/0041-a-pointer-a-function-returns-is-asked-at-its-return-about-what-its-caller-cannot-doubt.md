---
status: "accepted"
date: 2026-09-27
decision-makers: itsakeyfut
---

# A pointer a function returns is asked at its return, about what its caller cannot already doubt

## Context and Problem Statement

`int *stale(void) { int *p = malloc(4); free(p); return p; }` builds, and so
does a caller that reads through what it returns. The memory check asks about a
freed allocation where a place is read or written through it and where it is
freed again, and a `return` is neither. The caller reads `stale()` as a call it
cannot read, whose result
[ADR-0039](./0039-an-allocation-code-this-check-cannot-read-may-reach-stays-exposed-while-it-lives.md)
makes a fresh allocation or any exposed one, and a fresh one is live. ADR-0039
recorded the program as believed and left it to
[#252](https://github.com/itsakeyfut/safec/issues/252).

The belief is the caller's and the evidence is the callee's. Nothing reads a
callee's body from its caller, which the module comment of
`crates/safec-ir/src/memory.rs` calls the boundary, so the one place the belief
can be checked is the callee's own `return`. That is
[ADR-0037](./0037-a-nonnull-parameter-is-believed-by-its-body-and-checked-at-every-call-in-its-translation-unit.md)'s
shape turned round: there the body believes and every call is checked, and here
every caller believes and the body is checked.

## Decision Drivers

* A program with a use after free that builds is the worst answer this
  compiler gives, and this one is silent in both functions.
* `docs/diagnostics.md` says a name read without a dereference, `free(p); p;`,
  is not something this check reads. A `return` is that shape, so reporting it
  needs a reason the other bare reads do not have.
* An unproven conclusion fails the build
  ([ADR-0033](./0033-a-conclusion-this-analysis-could-not-prove-does-not-build.md)),
  so a doubt reported at a return is a refusal of an ordinary function, and
  functions return pointers far more often than they dereference one they were
  handed after a call.
* The C library idioms are the programs that cost a rule about calls, and the
  corpus holds none of them, so the cost has to be measured on probes.

## Considered Options

* Ask at the return about every allocation the returned value may hold, as a
  dereference is asked.
* The same, except that an allocation a parameter holds is reported only where
  it was proved freed.
* Ask at the return about proofs only.
* Ask at `Terminator::Return` about the return place, rather than at the write
  into it about the local being returned.

## Decision Outcome

Chosen option: "the same, except that an allocation a parameter holds is
reported only where it was proved freed", asked at the write into the return
place, because it is the narrowest rule that leaves no program silent in both
the callee and its caller.

**Why a return is read when other bare reads are not.** A value copied into
another local stays in this check's sight, and a later dereference of the copy
is asked. A value returned leaves it for a caller that believes it live, so the
return is the last point anything can be said about it, and the check is of
what the caller was promised rather than of the read itself.

**Why a parameter's doubt is left to the caller.** A caller reads this
function as a call it cannot read, so by ADR-0039 every allocation it handed
over is exposed and unproven after the call, and so is anything the result may
be. A doubt about a parameter's allocation at the return therefore adds nothing
the caller does not already say at its own next use, measured: freeing a
parameter on one arm and returning it is reported at the caller's dereference.
A proof adds something, and is kept: `free(p); return p;` is reported in the
function that did it. An allocation the function made itself, or was handed
back by a call it made, is the one the caller believes fresh, and every doubt
about that one is reported.

**What the return asks, exactly.** Where the lowering writes a `return`'s value
into the return place, the local being written is asked what
`Known::reached_by` answers for it, as a dereference asks, so an escaped local
is distrusted as ADR-0017 distrusts it, with a parameter's site dropped unless
it is `SiteState::Freed`. Only in a function whose return type is a pointer,
because every call's result is a site and exposed, a later call this check
cannot read unproves it, and an addition of integers keeps its operands'
sites (ADR-0030), so `return f() + g();` reached `f`'s result after `g` ran
and was refused, in six corpus cases. The order is not asked: a `return` leaves the
function after its whole expression, so any free in it has run, and
`memory::verdict` answers `true` for this question as it does for a double
free.

**A code of its own, `SC0406`.** `SC0402` is a dereference by
`docs/diagnostics.md`'s own definition, and the fix here is at the return or
the free rather than at a read.

### Confirmation

Each rule has a mutation in `crates/safec-ir/src/memory.rs`, applied and
measured, and a named case in `crates/safec/tests/cases` that it fails:

* Not calling `returned` fails `a_function_that_returns_what_it_freed`,
  `a_return_after_a_free_on_one_arm_only`, `a_parameter_freed_and_returned`,
  `an_escaped_local_freed_and_returned` and
  `an_allocation_handed_to_a_call_and_returned`, and the reported half of
  `a_free_after_the_write_into_the_return_place_is_not_asked_about` in
  `crates/safec-ir/tests/freed.rs`.
* Keeping every parameter's site fails
  `a_parameter_returned_after_a_call_this_check_cannot_read` and
  `a_parameter_freed_on_one_arm_is_doubted_by_its_caller`.
* Dropping every parameter's site, freed or not, fails
  `a_parameter_freed_and_returned`.
* Dropping the pointer-type condition fails
  `an_integer_built_from_two_calls_is_not_asked_about_at_its_return` and six
  older cases that return an `int` built from calls.
* Asking the return place after the write instead of the local being written
  fails `an_escaped_local_freed_and_returned`, which becomes a proof.
* Reporting only proofs fails `a_return_after_a_free_on_one_arm_only`,
  `an_allocation_handed_to_a_call_and_returned` and
  `an_escaped_local_freed_and_returned`.

* Dropping `Reached::SetFreed` from what is asked fails
  `a_free_of_either_of_two_allocations_then_returned`, which goes silent.
* Asking any copy rather than the write into the return place fails
  `a_freed_pointer_copied_but_not_returned`.
* Asking a source read through a projection fails
  `a_pointer_read_out_of_a_freed_table_and_returned`, which gains a report
  about the table where what leaves is a pointer read out of it.
* Changing the words of the `Lost` row in `memory_finding` fails
  `a_pointer_kept_from_the_last_turn_of_a_loop_and_returned`, the one case
  that reaches it.

A new `Kind` is `error[E0004]` in `verdict`'s `ordered` and in `driver.rs`'s
`memory_finding`, which is the guard the compiler holds.

**`ordered` answering `true` is held only against `false`.** Answering
`false` fails `a_function_that_returns_what_it_freed` and
`a_parameter_freed_and_returned`; answering as a dereference does,
`earliest.is_some_and(|freed| freed.sequenced)`, changes nothing, measured,
because every free the C frontend emits is followed by a sequence point before
the write into the return place. The arm is right for the reason given above
and no C program shows it.

**The boundary is held too.** A free placed between the write and the return,
which no C program builds, is silent, and
`a_free_after_the_write_into_the_return_place_is_not_asked_about` pins it so
that closing it fails a named test. `docs/c-family.md` carries the requirement.

### Consequences

* Good, because a function that frees what it returns is reported, proved
  where it is proved, in the function that did it.
* Bad, because a function that hands its own allocation to a call this check
  cannot read and then returns it is refused, and so is one that returns a
  pointer whose address it handed out, `grow(&b); return b;`: a false refusal, measured.
* Bad, because the rule leans on ADR-0039's treatment of a call's arguments in
  the caller. A caller that stopped doubting what it handed a call, which
  summaries of defined functions would do, would leave a parameter freed on one
  arm silent in both functions, so that change has to widen this one.
* Bad, because a returned value nobody dereferences is only reported where the
  callee's own allocation is involved, which is the bare-read boundary
  `docs/diagnostics.md` already states.
* What would reverse this: summaries of functions this translation unit
  defines, which would let a caller read what a callee returns instead of
  believing it.

## Pros and Cons of the Options

Measured on a throwaway prototype against seven programs with a defect and ten
without, all returning a pointer, and against the corpus.

### Every allocation, as a dereference

* Good, because nothing is left silent in either function.
* Bad, because it also refuses returning a parameter after any call this check
  cannot read, returning `p` on `realloc`'s failure branch, and a wrapper
  returning another call's result while a pointer parameter exists: five of the
  ten, where the chosen rule refuses two.

### Proofs only

* Good, because it refuses nothing that builds today.
* Bad, because a free on one arm only is silent in the callee and the caller
  believes the result live: the silence this record exists to remove.

### At `Terminator::Return`, about the return place

* Good, because it asks about the value that really leaves, whatever a
  frontend wrote between.
* Bad, because the return place is never escaped, so a returned local whose
  address escaped is proved where a dereference of it is only doubted, which
  is ADR-0017's rule skipped. Measured.
* Bad, because `Terminator::Return` carries no span, so the caret needs either
  a change to the IR or a guess at the write.

## More Information

* #252, and #250 where it was split off.
* `crates/safec/src/lowering.rs`, whose `Stmt::Return` arm writes the return
  place and ends the block with `Terminator::Return` and nothing between, which
  is what makes asking at the write the same as asking at the return for C.
  `docs/c-family.md` is where that becomes a requirement on another frontend.
