---
status: "accepted"
date: 2026-09-27
decision-makers: itsakeyfut
---

# A pointer read out of memory reaches what was stored, for what a call reaches, and a pointer parameter's allocation is exposed where the function starts

## Context and Problem Statement

[ADR-0039](./0039-an-allocation-code-this-check-cannot-read-may-reach-stays-exposed-while-it-lives.md)
marks an allocation *exposed* once code this check cannot read may reach a
pointer to it, and names two routes it does not reach.
[#254](https://github.com/itsakeyfut/safec/issues/254) is those two routes.

A pointer read out of memory, `q = *tab`, holds no site, which is
[ADR-0017](./0017-record-each-half-of-an-escape-where-its-subject-lives.md)'s
belief about it. That belief was written for a reader that reports a
dereference, for which "no site" means "say nothing". ADR-0039 added readers
that ask what a call reaches and what a write carries into an allocation, and
each read the same answer as "reaches nothing". So `*tab = p; release(*tab);
return *p;` builds, and so do `*b = *a; drop_inner(b);`, a table read back out
of a holder and handed to a call, and a pointer whose site a loop's rebirth
took away handed to a call. An answer that was safe to be wrong about for the
reader it was written for is a silence for the new ones.

An allocation a pointer parameter holds is not exposed at entry, though the
caller, which this check cannot read, had the pointer. So `int f(int *p) {
release_all(); return *p; }` builds, and so does `q = lookup(); free(p); return
*q;`.

Every one of these is saying safe wrongly, the answer
[`docs/safety-model.md`](../safety-model.md) calls the worst this compiler can
give, measured to exit 0 on `main` at `642261c`.

## Decision Drivers

* The ranking of failures, saying safe wrongly worst and a false report the
  reader can see far better: every option below fails one of those two ways,
  and the chosen ones fail only as false reports the reader can see.
* Giving a local a site it did not have can make the reporting path quieter,
  because an empty set is the loud one for a `free`. A rule that widens what a
  call reaches must not widen what a report reads.
* [`docs/safety-model.md`](../safety-model.md): a parameter ranges over every
  value a caller may pass, so the caller having stashed it where an unread
  callee can free it is one of those values.
* The corpus holds few programs a call's transfer can be costed on, so the
  costs below were measured on probes written for this record.

## Considered Options

What a pointer read out of memory means:

* **to what a call reaches and what a write stores only**: the report still
  reads it as holding no site
* to the report as well: the load holds the sites the allocation it was read
  from contains

How precisely a loaded pointer held in a local is followed:

* **one bit per local, "may hold a pointer read out of memory", and every
  stored pointer where one reaches a call**
* a fourth square table naming the allocations it was read from

A pointer parameter:

* **exposed where the function starts**
* not exposed, and recorded as the boundary of a check that reads one function

## Decision Outcome

Chosen: **a load reaches what was stored, for exposure and for contents only;
one bit per local; and a pointer parameter exposed at entry.**

**What a load reaches.** Where an argument of a call this check cannot read, or
of a library function that returns its first argument, may be a pointer read
out of memory, the call reaches what that memory may hold: for a read through
one `Deref` of a local holding sites and nothing it lost, the union of
`Known::inside` over those sites; otherwise every site any allocation's
`inside` row holds, which this record calls *what is stored*. The same answer is
recorded as the contents of an allocation a write stores such a pointer into,
and is exposed at once by a write this check cannot place. The report is not
told: `q = *tab; *q` is read exactly as before, and so is `free(q)`.

**What was stored stays stored.** `Known::reborn` used to clear what an
allocation held when a loop made a new one at the same site, which nothing
read for a pointer the check had lost. This record reads it for exactly that
pointer, so the row now keeps what the old allocation held; it is a may-set,
and a stale entry costs a report rather than a proof.

**The bit.** `Held` gains a bit saying the local may hold a pointer read out of
memory. A direct assignment from a place with a projection sets it on a
pointer-typed destination, a write that carries such a value carries the bit,
an operation unions it, and a fresh value clears it: it is a fact that an
assignment destroys, which is ADR-0018's rule for what `Held` holds. A local
with the bit, or with `Held::lost`, reaches what is stored when it is handed to
a call, when its address has escaped and any call runs, and when it is stored.
A local that merely holds no site does not: `int *z = 0; log_ptr(z);` exposed
every stored pointer when emptiness was the test, measured.

**A pointer parameter.** Its site is exposed where the function starts. Only a
pointer: an integer parameter can hold an allocation only through a conversion
this frontend does not accept yet, a cast, which does not parse, or an implicit
one, which C17 6.5.16.1 p1 forbids and #154 is about; exposing one moved
`a_write_through_a_pointer_with_one_target_on_one_arm_only`, measured. The day
casts parse, the type is no longer enough. And not `main`'s: its caller is the
host, and C17 5.1.2.2.1 p2 has `argv` and its strings keep their values until
the program ends, so no call frees them without undefined behaviour. A program
that calls `main` itself hands it arguments that are not the host's, which this
does not see.

### Confirmation

Every mutation below was applied on its own to the tree as committed, the whole
workspace was run with `--no-fail-fast`, and the file was restored. The tests
named are the ones that failed. The cases are in `crates/safec/tests/cases`
and every mutation is in `crates/safec-ir/src/memory.rs`.

**What a load reaches at a call.** `Allocations::read_out` answering nothing
for a read through one `Deref` fails
`what_a_callee_frees_through_a_pointer_read_out_of_a_table_is_unproven_after_it`
and `a_pointer_copied_from_one_table_to_another_is_inside_both`; answering what
is stored there instead fails
`a_pointer_read_out_of_one_table_exposes_only_what_that_table_holds` alone.
Reading a container that may itself hold a load as though it did not fails
`a_pointer_read_out_of_memory_on_one_arm_is_still_one_after_the_join` alone.
Leaving `memset`'s arguments out fails
`what_memset_is_handed_out_of_a_table_is_unproven_after_a_later_call` alone.

**Contents.** The block that records into `Known::inside` ignoring `read_out`
fails `a_pointer_copied_from_one_table_to_another_is_inside_both` alone.

**The bit.** Never setting it in the direct assignment fails
`a_table_read_out_of_a_holder_into_a_local_is_reached_through_it` and
`a_pointer_read_into_a_local_whose_address_a_call_is_handed_is_reached_through_it`;
never setting it in `Allocations::carried` fails
`a_pointer_read_out_of_a_table_through_an_address_is_reached_through_it` alone;
`Known::reach_of` ignoring it for an escaped local fails that case and
`a_pointer_read_into_a_local_whose_address_a_call_is_handed_is_reached_through_it`.
Testing in `read_out` for a local holding no site, in place of the bit or
beside it, fails `a_null_pointer_handed_to_a_call_exposes_nothing_stored`,
`a_null_pointer_whose_address_a_call_is_handed_exposes_nothing_stored` and
`a_local_given_something_else_after_a_load_is_no_longer_one`; testing so in
`reach_of` fails `a_null_pointer_whose_address_a_call_is_handed_exposes_nothing_stored`
alone. Dropping it at
a join fails the one-arm case; keeping it through `Held::clear` fails
`a_local_given_something_else_after_a_load_is_no_longer_one`; dropping it in
`Held::accumulated`, and not setting it in `built_from` for an operand read
through a projection, each fail `a_pointer_moved_off_a_load_is_still_a_load`.
The second of those was a silence the first version of this change shipped to
itself, found by mutating the first. Setting it whatever the type, in
`Allocations::may_be_pointer` or in `built_from`, fails
`an_integer_read_out_of_memory_is_not_a_load`.

**What was stored.** Clearing the row in `Known::reborn` again fails
`a_table_allocated_again_by_a_loop_still_holds_what_it_held` and
`a_table_read_back_after_a_loop_allocated_it_again_holds_what_it_held`; the
first is also the case that `read_out` not reading `Held::lost` fails alone,
since the table it releases is last turn's. Two review lenses found the
clearing independently, each with a program that built.

**A pointer parameter.** Exposing nothing in `on_entry` fails
`an_allocation_a_parameter_holds_is_unproven_after_any_call`,
`a_call_may_return_what_a_parameter_holds` and
`a_call_whose_destination_it_dereferences`. Exposing every parameter whatever
its type fails `a_write_through_a_pointer_with_one_target_on_one_arm_only`, and
exposing `main`'s fails `the_arguments_the_host_hands_main_are_not_exposed`
alone.

**What nothing holds.** `Known::reach_of` reading `Held::lost` for an escaped
local: every call that gives an escaped local the bit also reaches it, and a
lost bit from ADR-0029 is on a local whose new contents an unread callee
wrote, which are exposed already, so there it is wider than it needs to be.
What a library copy returns carrying the bit, since the same call has just
exposed what its argument reaches (`Allocations::read_through` says so where
it is used). `Analysis::height`, as for every other term in it.

### Consequences

* Good, because every route #254 and ADR-0039's review measured, by which an
  unread call reaches an allocation, is reported.
* Good, because no report about a dereference or a free changes, so a wider
  may-set making a report quieter is closed by construction rather than by a
  test.
* Bad, because a pointer parameter read after **any** call this check cannot
  read is unproven, and so is the result of any such call in a function that
  takes a pointer, since it may be the parameter's allocation. Under ADR-0033
  those do not build: `fill(int *out) { log_line(); *out = n; }`, and `q =
  make(); return *q;` or `lookup(); free(p);` in a function taking a pointer,
  were each measured to be refused. A false report the reader can see, and the
  one a user meets first; the answer ADR-0033 names for it is the hatch and an
  annotation.
* Bad, because a loaded pointer held in a local exposes every stored pointer
  when it reaches a call, not only those in the allocation it came from, and so
  does a local an unread callee may have written into.
* Bad, because a use through the loaded pointer itself still builds, with a
  call between or without one: `free(p); q = *tab; return *q;`, and `q = *tab;
  release(tab); return *q;`, and `int f(int **pp) { q = *pp; release_all();
  return *q; }`. The call reaches the allocation and makes it unproven; the
  report reads `q` as holding no site, as ADR-0017 does, so nothing asks. That
  is [#256](https://github.com/itsakeyfut/safec/issues/256), and
  [ADR-0045](./0045-a-pointer-read-out-of-memory-holds-what-was-stored-there-and-proves-nothing-with-it.md)
  answers the first two by telling the report too. The third is
  [#281](https://github.com/itsakeyfut/safec/issues/281).
* Bad, because what `memset` is handed out of a table is unproven after a
  later call, although C17 7.24.6.1 has it copy no pointer. That is ADR-0039's
  rule for the family, that what it is handed is exposed, applied to a load;
  narrowing it for `memset` is a question about that rule.
* Bad, because a pointer copied a byte at a time through `char` is not a load,
  and a use after free through the copy builds, as it did before this record.
  That is [#257](https://github.com/itsakeyfut/safec/issues/257).
* What would reverse this: summaries of the functions this translation unit
  defines, or an annotation saying what a callee may free, either of which
  would let a parameter's allocation stay proved across a call that provably
  cannot reach it.

## Pros and Cons of the Options

### A load reaches what was stored, for exposure only

* Good, because the report is untouched.
* Bad, because a use after free through a loaded pointer with no call between
  is still silent.

### A load reaches what was stored, for the report too

* Good, because `free(p); q = *tab; *q` is reported.
* Bad, because `inside` is per allocation and only ever grows, so a table with
  one freed slot warns on every read of the others, and `free(q)` of a loaded
  pointer, now `SC0401` for losing it, becomes a proof over a may-set nobody
  said was complete: a wider may-set making a report quieter.

### One bit per local

* Good, because it costs a byte per local against a value already square in
  them.
* Bad, because a loaded pointer handed to a call exposes every stored pointer.

### A fourth square table

* Good, because only what that allocation held is exposed.
* Bad, because the third one cost half again in memory and time (ADR-0039), and
  #173's bitset would be needed first.

### A parameter exposed at entry

* Good, because a caller that stashed the pointer is one of the callers C
  permits, and the safety model already reads a parameter that way.
* Bad, because it is the cost above, a false report the reader can see.

### A parameter as the boundary

* Good, because nothing an ordinary function does changes.
* Bad, because `release_all(); return *p;` builds, which is saying safe wrongly
  written down rather than moved.

## More Information

* [#254](https://github.com/itsakeyfut/safec/issues/254), whose design comment
  carries the probes and the prototype measurements.
* [ADR-0039](./0039-an-allocation-code-this-check-cannot-read-may-reach-stays-exposed-while-it-lives.md),
  which this extends and whose "What this does not reach" paragraph it answers.
* [ADR-0017](./0017-record-each-half-of-an-escape-where-its-subject-lives.md),
  whose belief about a pointer this check never had a site for stands for the
  report.
* What earlier reviews of this check learned: adding a site to a may-set can
  turn a report into silence, because a local reaching no site is reported and
  one reaching a single live site is not; a `free` and a dereference need
  opposite answers for a local reaching no site; an answer that was safe to be
  wrong about for one reader can be a silence for the next; and the corpus has
  no program to cost a rule about what a library call returns.
