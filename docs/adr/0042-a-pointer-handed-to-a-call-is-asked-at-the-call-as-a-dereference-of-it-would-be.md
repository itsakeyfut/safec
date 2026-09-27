---
status: "accepted"
date: 2026-09-27
decision-makers: itsakeyfut
---

# A pointer handed to a call is asked at the call, as a dereference of it would be

## Context and Problem Statement

`int use(int *p) { return *p; }` and `free(a); return use(a);` build. The
caller's call is a bare read of `a`, which the memory check does not ask about,
and the callee believes its parameter live, because
[ADR-0040](./0040-a-pointer-read-out-of-memory-reaches-what-was-stored-and-a-parameter-is-exposed-where-the-function-starts.md)
makes a parameter's allocation unproven only after a call this check cannot
read, and none ran before the read.
[#263](https://github.com/itsakeyfut/safec/issues/263) filed that shape for a
callee defined in the same file.

It is wider than the file. Measured on `main` at `381262f`, every one of these
exits 0: the callee only declared (`free(a); return use(a);`), a library
function this check reads by name (`free(d); memcpy(d, s, 8);`), a callee that
frees what it is handed (`free(a); drop(a);`), a parameter freed and handed on
(`void f(int *p) { free(p); run(p); }`), and a free on one arm only (`if (c) {
free(a); } return use(a);`). So does `init(a); return use(a);`, which is a
defect only if `init` freed what it was handed, and which a dereference of `a`
in its place already doubts. The cause is one: nothing asks what a call's
arguments hold.

This is the belief
[ADR-0041](./0041-a-pointer-a-function-returns-is-asked-at-its-return-as-a-dereference-of-it-would-be.md)
checked at a `return`, arriving by the other door. A function's body believes
its pointer parameters live at entry, in this translation unit and in any other
`safec` checks, so the one place the belief can be checked is where a pointer
is handed over. That is
[ADR-0037](./0037-a-nonnull-parameter-is-believed-by-its-body-and-checked-at-every-call-in-its-translation-unit.md)'s
shape, a body that believes and a call that is checked, for liveness rather than
for null.

## Decision Drivers

* A program with a use after free that builds is the worst answer this compiler
  gives, and every shape above is silent in both functions.
* C17 6.2.4 p2 makes a pointer's value indeterminate when what it points at
  reaches the end of its lifetime, and 7.22.3 p1 ends an allocated object's
  lifetime at its deallocation, which is what `free` does (7.22.3.3 p2).
  Annex J.2, which is informative, lists using such a value as undefined, so on
  that reading handing it to a function is undefined whether or not the callee
  reads through it.
* `docs/diagnostics.md` says a bare read, `free(p); p;`, is not something this
  check reads. An argument is that shape, so reporting it needs a reason the
  other bare reads do not have, as a `return` did.
* An unproven conclusion fails the build
  ([ADR-0033](./0033-a-conclusion-this-analysis-could-not-prove-does-not-build.md)),
  so a doubt reported at a call is a refusal, and calls that take a pointer are
  far more common than dereferences after a call.
* The corpus holds almost no program a call's arguments can be costed on, so the
  cost was measured on probes, the library idioms among them.

## Considered Options

Which calls:

* **every call except `free` and the allocators**
* only calls to a function this translation unit defines

What is asked:

* **every allocation the argument may hold, as a dereference asks**
* proofs only
* proofs, and a doubt only where this function's own paths disagree about a
  free

How it is reported:

* **a code of its own, `SC0407`**
* `SC0402`, with the words changed

## Decision Outcome

Chosen: **at every call except `free` and the allocators, every allocation a
pointer argument may hold is asked about as a dereference asks, and reported as
`SC0407`.**

**Why an argument is read when other bare reads are not.** A value copied into
another local stays in this check's sight, and a later dereference of the copy
is asked. A value handed to a call leaves it for a body that believes it live,
so the call is the last point anything can be said about it, which is the
reason ADR-0041 gives for a `return`.

**Which calls.** Every argument of `Callee::Opaque` and `Callee::ReturnsFirst`,
none of `Callee::Frees` or `Callee::Allocates`, and the arguments after the
first of `Callee::Reallocates`. A `free` and `realloc`'s first argument are
asked already, as a double free; an allocator is handed sizes. Chosen as a
`match` over `Callee`, so a new kind of callee is `error[E0004]` here. A
function this translation unit defines is not told apart from one it only
declares: a body in another translation unit believes its parameters live on
the same terms, and narrowing to defined callees leaves `free(a); use(a);`
silent whenever `use` is in another file.

**What is asked.** What `Known::reached_by` answers for the argument's local,
through `verdict`, exactly as `used` asks a dereference, so an escaped local is
distrusted as ADR-0017 distrusts it and a local that reaches no site says
nothing (the dereference reader's answer, not the free's). Only an argument
that is a local read with no projection: a pointer read out of memory holds no
site, and this check says nothing about a dereference of one either (ADR-0017,
#256). Only an argument of pointer type: an addition of integers keeps its
operands' sites (ADR-0030), so `h(f() + g())` was refused on a program with no
pointer in it, measured, which is the reason ADR-0041 gives for its own
condition. One finding per local per call, so `g(p, p)` is one report.

**The order at the call is asked as a dereference asks it.** `Kind::ArgumentAfterFree`
answers `earliest.is_some_and(|freed| freed.sequenced)` in `verdict`'s
`ordered`. Answering `true`, as a `return` does, proved `(free(a), 0) +
use(a)`, where C17 6.5.2.2 p10 lets `use` run before `free` and the program is
defined on that order, measured. What the rule costs is `two(a, (free(a), 0))`,
where every order hands `two` a freed pointer and this check says only that it
may: a proof reported as a doubt, which fails the build unless
`--allow-unknown` is given, and under it is a warning on a run that exits 0.

**Asked at the call and nowhere after it.** A dereference is also recorded as a
pending read and asked again at a free the same full expression leaves
unsequenced with it (ADR-0023). What a call is handed is not, so
`(memset(a, 0, 4) != 0) + (free(a), 0)` builds: C lets the free run first. A
callee this check cannot read hides the gap, because the later free becomes a
doubted `SC0401`; `memset`, `memcpy` and the rest read by name do not. Carrying
it forward changes what a pending read records, and is
[#270](https://github.com/itsakeyfut/safec/issues/270).

**No null exemption.** `memory::asked` exempts a pointer established null from
a free, and nothing here does. An exemption is the half of a rule that can go
quiet, and the one program measured that it would change, `free(a); if (a == 0)
{ use(a); }`, reads `a` inside an arm no execution reaches.

**A code of its own, `SC0407`.** `SC0402` is a dereference by
`docs/diagnostics.md`'s own definition, and the fix is at the call or at the
free rather than at a read, which is ADR-0041's reason for `SC0406`. A new
`Kind` is `error[E0004]` in `verdict`'s `ordered` and in `driver.rs`'s
`memory_finding`.

### Confirmation

Every mutation below was applied on its own, the whole workspace was run with
`--no-fail-fast`, and the file was restored. The tests named are the ones that
failed. The cases are in `crates/safec/tests/cases`, and every mutation is in
`crates/safec-ir/src/memory.rs` unless it says otherwise.

* Dropping the call to `handed` from `findings` fails
  `a_freed_pointer_handed_to_a_function_defined_in_the_file`, which is #263's
  program, `a_freed_pointer_handed_to_a_function_only_declared`,
  `a_freed_pointer_handed_to_a_function_that_frees_it`,
  `a_parameter_freed_and_handed_on`, `a_freed_pointer_handed_to_memcpy`, the
  two doubts below, and every case this record moved.
* Reporting only proofs in `handed` fails
  `a_pointer_freed_on_one_arm_and_handed_to_a_call` and
  `an_allocation_a_call_was_handed_is_doubted_when_handed_on`, which go
  silent, and the cases whose report here is a doubt.
* Answering `Callee::ReturnsFirst` with no arguments fails
  `a_freed_pointer_handed_to_memcpy` alone.
* Asking every argument of `realloc` rather than those after the first fails
  `a_freed_pointer_handed_to_realloc_is_freed_twice` and
  `a_pointer_freed_before_realloc_stays_freed_after_it`, which gain a second
  report at the call their `SC0401` stands at.
* Dropping the pointer-type condition fails
  `an_integer_built_from_two_calls_is_not_asked_about_as_an_argument` alone.
* Answering `true` for `Kind::ArgumentAfterFree` in `verdict`'s `ordered` fails
  `a_call_unsequenced_with_a_free_is_not_proved_to_be_handed_it` alone, which
  becomes a proof.
* Dropping the repeated-local test fails
  `a_freed_pointer_handed_twice_to_one_call_is_one_report` alone.
* Asking only the first argument, `arguments.iter().take(1)`, or turning the
  loop's `continue`s into `break`s, each fails
  `a_freed_pointer_handed_after_other_arguments_is_asked` alone, which goes
  silent. Found by review: every other case hands the freed pointer first.
* Asking an argument read through a projection fails
  `a_pointer_read_out_of_a_freed_table_and_handed_to_a_call`,
  `a_freed_pointer_read_in_an_argument` and
  `a_comma_inside_a_call_argument_orders_nothing_outside_it`, each gaining an
  `SC0407` about the pointer the argument was read through.
* In `crates/safec/src/driver.rs`, changing the words of the `Lost` row fails
  `a_table_allocated_again_by_a_loop_still_holds_what_it_held` alone, and giving
  the `Unsequenced` row the disagreement remedy fails
  `a_call_unsequenced_with_a_free_is_not_proved_to_be_handed_it` alone.

A new `Kind` is `error[E0004]` in `verdict`'s `ordered` and in `driver.rs`'s
`memory_finding`, and a new `Callee` is `error[E0004]` in `handed`, which are
the guards the compiler holds. That `realloc`'s size and an allocator's
arguments are never asked is held by the pointer-type condition as much as by
the `match`, and nothing tells the two apart.

### Consequences

* Good, because a freed pointer handed to any call is reported, in the function
  that handed it, whatever the callee is and wherever it is defined.
* Good, because `a_call_this_check_cannot_read_between_two_frees` and
  `a_parameter_freed_then_handed_to_a_call_and_returned` gain the report they
  were missing, measured on a prototype.
* Bad, because a pointer handed to a call after anything this check reads as a
  doubt is refused, with no free in the function required. Measured on a
  prototype, of eleven probes without a defect, four that built are refused: a
  parameter handed to two opaque calls (`init(p); run(p);`), a parameter handed
  on after an unrelated call (`log_line(); run(p);`), a wrapper defined in the
  same file called twice, and the free-and-null idiom. Each was already refused
  where the pointer is dereferenced rather than handed on. A false refusal the
  reader can see, and the answer ADR-0033 names for it is the hatch and an
  annotation.
* Bad, because the free-and-null idiom `if (c) { free(p); p = 0; } run(p);` is
  refused through the join that does not tell paths apart, which is #264.
* Bad, because `two(a, (free(a), 0))` is reported as a doubt where it is a
  proof, so `--allow-unknown` builds it with a warning.
* Bad, because two more shapes C defines are refused, found by review rather
  than by the prototype: an allocation handed to one call this check cannot
  read and then to another, `fill(p); return use(p);`, with no free anywhere;
  and `realloc`'s failure branch handing the old pointer to a function,
  `if (b == 0) { release(a); }`, which C17 7.22.3.5 p3 leaves allocated. Both
  are the doubt the first bullet describes, and a free in place of `release`
  was already refused as `SC0401`.
* Bad, because the join is not path-sensitive, so a free followed by a call in
  an arm no execution reaches, `free(a); if (0) { use(a); }`, is reported as a
  proof. `SC0402` already did the same with `*a` in that arm; a call there is
  the commoner shape.
* Bad, because a pointer handed to a call and freed later in the same full
  expression, which C may run first, is silent. That is #270, above.
* Bad, because the address of a freed pointer handed to a call, `use2(&a)`
  where the callee reads `*pp` and dereferences it, is silent in both
  functions, as it was before this record. That is
  [#271](https://github.com/itsakeyfut/safec/issues/271).
* Bad, because a pointer read out of memory and handed on, `use(*tab)`, is
  silent, as its dereference is. That is #256.
* What would reverse this: summaries of the functions this translation unit
  defines, or an annotation saying what a callee does with a parameter, either
  of which would let a call that provably reads nothing through its argument
  leave a doubt unasked.

## Pros and Cons of the Options

Measured on a throwaway prototype against seven probes, six with a defect and
one, `init(a); return use(a);`, that is a defect only if `init` freed, and
eleven without a defect, and against the corpus.

### Every call

* Good, because the declared, library and defined callee are one rule.
* Bad, because a callee that reads nothing through what it is handed is asked
  all the same, and nothing in this check can tell one apart.

### Only calls to a defined function

* Good, because it is what #263 was filed as, and a summary could later narrow
  it.
* Bad, because `free(a); use(a);` with `use` in another file is silent.

### Every allocation, as a dereference

* Good, because all seven probes are reported.
* Bad, because four of the eleven others are refused, listed above.

### Proofs only

* Good, because none of the eleven is newly refused.
* Bad, because a free on one arm only and an allocation an opaque call was
  handed stay silent in every function. It is the narrowing ADR-0041 chose
  first and withdrew.

### Proofs, and a doubt from this function's own paths

* Good, because the one-arm free is reported.
* Bad, because an allocation an opaque call was handed stays silent, and
  telling the two doubts apart needs `SiteState::Unknown` split in the lattice.

### `SC0402`

* Good, because no code is added.
* Bad, because `SC0402` is defined as a dereference, and sharing its `Kind`
  would share the order rule by accident rather than by decision.

## More Information

* [#263](https://github.com/itsakeyfut/safec/issues/263), whose design comment
  carries the probes.
* [ADR-0041](./0041-a-pointer-a-function-returns-is-asked-at-its-return-as-a-dereference-of-it-would-be.md),
  whose reason for reading a bare `return` this extends to an argument, and whose
  withdrawn option this does not repeat.
* [ADR-0037](./0037-a-nonnull-parameter-is-believed-by-its-body-and-checked-at-every-call-in-its-translation-unit.md),
  the same boundary for null.
