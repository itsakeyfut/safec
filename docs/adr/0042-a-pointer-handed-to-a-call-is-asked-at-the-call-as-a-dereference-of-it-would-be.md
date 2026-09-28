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

**Asked at the call, and carried from it as a dereference is.** A
dereference is also recorded as a pending read and asked again at a free the
same full expression leaves unsequenced with it (ADR-0023). What a call is
handed is recorded the same way, where the call's terminator is and before its
transfer, and asked again by `used_before` at a later free, opaque call or
`realloc`: `(memset(a, 0, 4) != 0) + (free(a), 0)` is `SC0407` at `memset`,
because C lets the free run first. Asked only at the call, as this record first
had it, that program built, and so did the same program with an opaque call or
`realloc` in place of the free, while each one's dereference spelling was
`SC0402` ([#270](https://github.com/itsakeyfut/safec/issues/270)).

A later call that encloses the one it was handed to is not asked, because C
orders a call's arguments before it: `free(memset(a, 0, 4))` and
`strlen(strcpy(s, t))` build. That is ADR-0043's, which the carry made
necessary, since it refused both on a first implementation.

**A later opaque call is asked about what it may free beyond what it is
handed.** This record first asked the arguments alone, and `(memset(p, 0, 4)
!= 0) + (release_all(), 0)` over a parameter built while its swapped spelling
was `SC0407`, as did the dereference spelling
([#273](https://github.com/itsakeyfut/safec/issues/273)). A call this check
cannot read may free any allocation it can reach (ADR-0039), so a read carried
to one is asked about three things:

* what the call reaches itself through what it is handed, closed over what
  those allocations hold, which `Allocations::reach` answers for the transfer
  as well;
* what something other than the read's own call had made reachable to code
  this check cannot read, which the read remembers in `PendingRead::reachable`:
  a parameter from entry (ADR-0040), a store into a reachable allocation, or
  another call in the expression;
* after a hatch, anything (ADR-0038).

**The read's own call is left out**, and so is a call enclosing it, because C17
6.5.2.2 p10 orders a call's arguments before its body: `keep(a) +
release_all()` over a local builds, since `release_all` cannot reach `a` before
`keep` has run. Two rules were implemented first and rejected on that ground.
Counting every site unknown after the call refused `keep(a) + release_all()`,
whose swapped spelling builds. Telling each read everything exposed by the time
of each later event refused `(keep(a) != 0) + release_all()` the same way. A
read is therefore told what each event makes reachable, when it happens,
whether or not the site was reachable already, so that a second route to what
the own call exposed is still told: `keep(a) + (*t = a, 0) + release_all()` is
reported.

What it newly refuses, measured on the library idioms and on the shapes review
found (`strlen`, `memcpy`, `memset`, `realloc`, `strdup`, the free-and-null
idiom, a pointer whose address was taken, a read whose own call exposes it, a
sequencing operator under one that does not sequence): `(x = p[0]) +
strlen(s)` and `(memcpy(d, s, 4) != 0) + g()` over parameters, a read beside a
call that reaches it through what it is handed or after another call or a store
exposed it, and a read beside a hatch. The swapped spelling of each was refused
already. The one that is not a mirror is below, in Consequences. A free and
`realloc` are still asked about their arguments only, because each frees only
what it is handed, and a free's report would also name it as the free.

A pending read carries which of the two it is, `Read::Dereference` or
`Read::Argument`, and is reported under the code it would have had where it
ran. The kind is a field of the value and not a part of the key, because the
key's place already keeps the two apart: a dereference is read through a
projection and an argument is a local read with none, so a kind in the key
would be a part no mutation can break. `handed_places` is the one function
that says which arguments are asked, for `handed` at the call and for the
transfer that carries them, and `handed` reports through `say`, because
`used_before` can reach the same caret about the same local.

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
* Answering `Callee::ReturnsFirst` with no arguments in `handed_places` fails
  `a_freed_pointer_handed_to_memcpy` and the five cases below that carry a
  pointer handed to `memset`.
* Asking every argument of `realloc` in `handed_places`, rather than those
  after the first, fails
  `a_freed_pointer_handed_to_realloc_is_freed_twice` and
  `a_pointer_freed_before_realloc_stays_freed_after_it`, which gain a second
  report at the call their `SC0401` stands at.
* Dropping the pointer-type condition in `handed_places` fails
  `an_integer_built_from_two_calls_is_not_asked_about_as_an_argument` alone.
* Answering `true` for `Kind::ArgumentAfterFree` in `verdict`'s `ordered` fails
  `a_call_unsequenced_with_a_free_is_not_proved_to_be_handed_it` alone, which
  becomes a proof.
* Dropping the repeated-local test in `handed_places` fails nothing on its
  own, because `handed` reports through `say` and the second report lands on
  the first one's key. Dropping it and pushing in `handed` fails
  `a_freed_pointer_handed_twice_to_one_call_is_one_report` and
  `an_escaped_pointer_handed_to_a_call_before_a_free_is_one_report`.
* Asking only the first argument in `handed_places`,
  `arguments.iter().take(1)`, or turning the loop's `continue`s into `break`s,
  each fails
  `a_freed_pointer_handed_after_other_arguments_is_asked` alone, which goes
  silent. Found by review: every other case hands the freed pointer first.
* Asking an argument read through a projection in `handed_places` fails
  nothing on its own: the dereference of the same place at the same call
  already holds `say`'s key, because `used` runs before `handed`. Asking it and
  pushing in `handed` fails
  `a_pointer_read_out_of_a_freed_table_and_handed_to_a_call`,
  `a_freed_pointer_read_in_an_argument` and
  `a_comma_inside_a_call_argument_orders_nothing_outside_it`, each gaining an
  `SC0407` about the pointer the argument was read through, and
  `an_escaped_pointer_handed_to_a_call_before_a_free_is_one_report`.
* Dropping the loop in `Allocations::terminator` that records `Read::Argument`
  fails `a_pointer_handed_to_a_call_the_check_meets_first_is_reported`,
  `a_pointer_handed_to_a_call_before_an_opaque_call_is_reported`,
  `a_pointer_handed_to_a_call_before_realloc_is_reported` and
  `a_pointer_handed_to_a_call_inside_one_arm_before_a_free_survives_the_join`,
  which go silent, and
  `an_argument_of_a_call_this_check_cannot_read_is_carried_to_a_later_free`
  and `a_nested_call_is_still_carried_to_a_free_beside_it`, which lose their
  `SC0407` at the call that was handed the pointer. Answering
  `Kind::UseAfterFree` for `Read::Argument` in `used_before` fails the first
  four, which become `SC0402`, and the other two.
* Answering `Kind::ArgumentAfterFree` for `Read::Dereference` in `used_before`
  fails eight cases whose dereference is carried to a later call, among them
  `a_write_through_a_pointer_the_check_meets_first_is_reported` and
  `an_unsequenced_use_the_check_meets_first_is_reported`, which become
  `SC0407`.
* Answering `Read::Dereference` in the `or_insert` of `Allocations`'s `join`
  fails `a_pointer_handed_to_a_call_inside_one_arm_before_a_free_survives_the_join`
  alone, which becomes `SC0402`.
* Pushing in `handed` rather than going through `say` fails
  `an_escaped_pointer_handed_to_a_call_before_a_free_is_one_report` alone,
  which gains a second `SC0407` about `a` at the `memset` caret.
* Recording `Read::Argument` only for `Callee::ReturnsFirst` fails
  `an_argument_of_a_call_this_check_cannot_read_is_carried_to_a_later_free`
  alone, which loses its `SC0407` and keeps its exit code: after a call this
  check cannot read, the later free is already a doubted `SC0401`, so the
  carry adds the report about the other order and nothing that fails a build.
* Recording `Read::Argument` after the call's whole transfer rather than
  before it fails `what_a_call_was_handed_is_carried_as_it_was_before_the_call`
  in `crates/safec-ir/tests/freed.rs` alone, where the call writes into the
  local it was handed and the entry would name the call's own allocation. No
  C program reaches that shape. The mutation has to run after every exit of
  the transfer, which is a wrapper around `Allocations::terminator` rather
  than the loop moved within it: the transfer returns early for `free` and
  for the callees that return their first argument, so a loop moved to its
  end also stops recording for `memset` and `strcpy` and fails the corpus
  cases that hand a pointer to either.
* Answering `false` for `beyond_its_arguments` in `used_before`'s filter, so
  that a later opaque call is asked about its arguments only, fails eleven
  cases, among them
  `a_read_carried_to_an_opaque_call_that_reaches_it_by_exposure_is_reported`,
  `a_read_of_a_local_stored_where_a_parameter_points_is_asked_at_a_later_opaque_call`,
  `a_pointer_handed_to_a_call_before_an_opaque_call_that_reaches_it_by_exposure_is_reported`
  and `a_read_carried_to_a_hatch_is_reported`, which go silent.
* Dropping `read.reachable` from that filter fails nine, among them the first
  three above and
  `each_of_two_calls_in_one_expression_is_asked_about_what_the_other_may_free`,
  which loses its report at `strlen(s)`. Dropping `own_reach` fails
  `a_later_call_is_asked_about_what_it_reaches_through_what_it_is_handed`
  alone, and dropping `anything` fails `a_read_carried_to_a_hatch_is_reported`
  alone.
* Starting `PendingRead::reachable` empty in `Known::meeting` fails seven, the
  parameter and stored-local cases among them. Reading `exposed` there as it
  stands rather than through `Known::reachable_now` fails
  `a_read_of_a_local_stored_where_a_parameter_points_is_asked_at_a_later_opaque_call`
  alone, because `*t = a` marks nothing exposed until the next call.
* Telling a read its own call's routes in `Known::noticed` fails six, among
  them `a_call_is_not_asked_about_an_allocation_only_the_read_s_own_call_exposed`
  and `a_dereference_in_a_call_s_arguments_is_not_asked_about_what_that_call_exposed`.
  Owning a read only by equal spans fails
  `a_call_enclosing_the_read_s_own_call_does_not_make_it_reachable_to_a_sibling`
  alone; owning it only by `inside` fails six, the pointer each call was handed
  no longer being its own.
* Telling a read everything reachable at an event, rather than what the event
  exposes, fails
  `a_call_exposing_another_allocation_does_not_make_the_read_s_reachable`,
  `a_read_is_asked_about_what_another_call_in_the_expression_exposed` and
  `an_allocation_exposed_on_one_arm_is_reachable_to_a_read_after_the_join`.
  Dropping the notice in `Known::expose` fails the last two; dropping the one
  after a store into a reachable allocation fails
  `a_read_is_asked_about_what_a_store_in_the_expression_exposed` alone; keeping
  one arm's `reachable` at the join fails
  `an_allocation_exposed_on_one_arm_is_reachable_to_a_read_after_the_join`
  alone.
* Counting every site as taken fails eleven, among them
  `a_read_of_an_allocation_nothing_exposed_is_not_asked_at_a_later_opaque_call`.
* Answering `true` for `beyond_its_arguments` for `realloc` fails
  `a_read_is_not_asked_at_realloc_of_another_allocation` alone, and for a free
  fails `a_read_is_not_asked_at_a_free_of_another_allocation` alone, which
  gains an `SC0402` about `a` with `freed here` on `free(b)`.
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
* Bad, because a program this check already refuses can gain a second report
  about one full expression, at a different caret: `g(a) + (free(a), 0)` is
  `SC0401` at the free, since `g` may free `a`, and now `SC0407` at `g` as
  well, since the free may run first. The free-first order is undefined
  whatever `g` does, and the other order is undefined when `g` frees `a`,
  which is what the first report already doubts, so neither report is wrong;
  it is one more line for the reader. Measured on the programs of #270's
  design comment, no exit code moved except on the three programs that were
  silent.
* Bad, because a pointer handed to a call is refused where a `,`, `&&`, `||`
  or `?:` below any node other than one of those four orders it before a later
  free or opaque call: `x = (memset(a, 0, 4) != 0) && h(a);` and
  `if (!((memset(a, 0, 4) != 0) && h(a)))` both are, as their dereference
  spellings are on `main`. The operator gets no `Element::Sequenced` there,
  which is #178's, and ADR-0043 records it as what it leaves.
* Bad, because since #273 that surface reaches every opaque call, not only
  one handed the same pointer: `x = p[0] ? g() : 0;`, `x = (p[0] != 0) && g();`
  and `x = (p[0], g());` over a parameter are refused where they built, though
  C17 6.5.15 p4, 6.5.13 p4 and 6.5.17 p2 order the read before `g`. A parameter
  is reachable from entry, so the read, still pending at `g`, is asked there.
  It was taken as a cost rather than left silent because it is bounded: any
  read of a pointer parameter after any opaque call in the function was
  refused already (`h(); x = p[0];`), so what is new is only a read before the
  function's first opaque call, in one expression, on the left of one of those
  operators under one that does not sequence. The declaration spelling, `int
  y = (p[0] != 0) && g();`, puts the operator at the root and builds.
  `a_read_sequenced_before_an_opaque_call_under_an_assignment_is_still_asked`
  holds today's answer, so closing #178 moves a named case.
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
