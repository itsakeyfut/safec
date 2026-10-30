---
status: "accepted"
date: 2026-09-27
decision-makers: itsakeyfut
---

# A pointer a function returns is asked at its return, as a dereference of it would be

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

Chosen option: "every allocation the returned value may hold, as a dereference
is asked", at the write into the return place, because it is the only option
measured that leaves no function silent about a free that reaches its return.

The second option was chosen first, implemented, and withdrawn after review.
Its reason was that a caller already treats as unproven everything it handed a
call it cannot read, so a doubt about a parameter's allocation at the return
would add nothing the caller does not say at its own next use. That holds only
where the caller's next use is a dereference or a free. A caller that hands the
result on as an argument asks nothing, because a bare read is not asked, and
the function it hands it to believes its parameter live: a parameter freed on
one arm and returned was silent in every function, and so was
`free(p); log_ptr(p); return p;`, where the call then turned the proved free
into a doubt, which it no longer does
([#262](https://github.com/itsakeyfut/safec/issues/262)). Review demonstrated both, and restoring the narrowing silenced them
again until
[ADR-0042](./0042-a-pointer-handed-to-a-call-is-asked-at-the-call-as-a-dereference-of-it-would-be.md)
asked what a call is handed, which reports each at the caller's call.

**Why a return is read when other bare reads are not.** A value copied into
another local stays in this check's sight, and a later dereference of the copy
is asked. A value returned leaves it for a caller that believes it live, so the
return is the last point anything can be said about it, and the check is of
what the caller was promised rather than of the read itself.

**What the return asks, exactly.** Where the lowering writes a `return`'s value
into the return place, the local being written is asked what `Known::reached_by`
answers for it, as a dereference asks, so an escaped local is distrusted as
ADR-0017 distrusts it. Only in a function whose return type is a pointer,
because every call's result is a site and exposed, a later call this check
cannot read unproves it, and an addition of integers keeps its operands' sites
(ADR-0030), so `return f() + g();` reached `f`'s result after `g` ran and was
refused, in six corpus cases. A value read through a projection is not asked,
because a pointer read out of memory holds no site and this check says nothing
about a dereference of one either (ADR-0017, #256). The order is not asked: a
`return` leaves the function after its whole expression, so any free in it has
run, and `memory::report::verdict` answers `true` for this question as it does
for a double free.

**A code of its own, `SC0406`.** `SC0402` is a dereference by
`docs/diagnostics.md`'s own definition, and the fix here is at the return or
the free rather than at a read.

**A store that leaves is asked as a return is, under `SC0414`** (#641). A
pointer stored where code outside the function may read it leaves this
check's sight by the other door: `void put(int **pp) { ... free(x); *pp = x;
}` built in silence, and the caller read the freed pointer back as one nobody
freed. `report::stored_away` asks such a store what `returned` asks, of the
value stored, at the same point. A store leaves where it may land in one of
three places:

- an allocation code this check cannot read can reach
  (`Known::sites_named_by_unknown_code`): one exposed to it, a parameter's
  memory included (ADR-0040), or one that a local whose address such code
  holds points at, at any depth, `keepp(&slot); slot = b; *b = x;` (#655);
- a local such code can name (`Known::locals_named_by_unknown_code`): one
  whose address a call was handed, or one whose address such a local holds,
  `keepp(&slot); slot = &t; t = x;`, before any later call marks it (#655);
- anywhere, where the pointer or a level on the way down may hold an
  address this function never recorded (`Known::through_unnamed`), or where
  the pointer names nothing this check followed, as one moved off a local
  that is not an array by a non-zero constant does (ADR-0019).

A write of such a local without a pointer, `keep(&slot); slot = x;`, is the
same store and is asked the same way. Every route is the same proof, which
`--allow-unknown` does not lower: through a kept local as through a
parameter, a freed pointer stored there is a use of an indeterminate value
(C17 6.2.4p2, Annex J.2), so `free(x); *b = x; *b = 0;` exits 1 even though
nothing reads `*b` before it is overwritten. A store into memory only this
function reads is not asked there, since a load of it carries what it held;
where that memory leaves, it is asked what it holds, as the next paragraph
says. A library copy out of the function is the doubt `SC0407` raises rather
than this proof, which is #652. It has a code of its own because what
to change is what is stored, and because code that is not a caller may read
it. A live pointer stored and freed after is not this rule, since the store is
of what was live as it ran; the next paragraph but one is.

**What memory that leaves holds is asked, as a call asks it**
(#651). A pointer that leaves live, by a return or a store, may point at memory
holding a freed pointer: `free(x); *b = x; return b;` built in silence, and so
did `*ppp = b;` in its place. Where the pointer itself draws nothing,
`report::freed_behind` asks `Known::handed_below` of it, the question ADR-0042
asks of what a call is handed, and says a doubt under the door's code,
`FreedBehindReturn` under `SC0406` and `FreedBehindStore` under `SC0414`.
Never a proof, since whoever receives the memory may only write there:
`*b = x; free(x); return b;` is correct when the caller overwrites `*b` before
reading it, and a proof would refuse it with nothing `--allow-unknown` can
lower. At every level, as a call asks (ADR-0042, #654), so the three doors
agree. `Known::handed_below`
also reads what a loop made again: a box filled on one turn and freed through on
the next holds an allocation whose site reads live for the new one, and is
doubted at all three doors, where it was silent at each.

That shape, a live pointer stored and freed after and the box returned, is now
asked here.

**A live pointer stored out and freed after is asked at the free** (#650).
`*pp = x; free(x);` handed the caller a freed pointer in its own memory in
silence: the store door asked a live pointer, and a parameter's memory is no
site this check models, so at the return nothing can be asked what it holds.
`Known::stored_out` records, per site, that a pointer to it may have been
stored by a store `Known::leaves` says leaves, the store door's own rule, a
union at every join and cleared where the site is made again; a free that
reaches such a site, where it is no double free, is `FreedAfterStore` under
`SC0414`. Asked at the free rather than at the return, which would have had
the function's name for a caret and could clear nothing either. A doubt, for
the reason the paragraph above gives, by the user's decision. It costs what
the paragraph above's shape costs: `*pp = x; free(x); *pp = 0;` is correct and
doubted, since this check does not follow what one place in such memory holds,
which is #656; in the corpus, 28 programs that already had a report each gained
this one at their free, and none was newly refused. A library copy out of the
function marks nothing, as the store door asks nothing of one, which is
#652.

**What this costs.** What memory holds only gains, so a slot freed and then
overwritten with null or a new allocation is still doubted at the door,
`*b = x; free(x); *b = 0; return b;` among them, which built before; the read
back and a call were already doubted the same way, and #656 is the strong update
that would stop all four. And a box holding what a call this check cannot read
returned, returned after a second such call, is doubted as the pointer itself
would be. Both are doubts `--allow-unknown` lowers.

**What it costs is what a return costs, and the store door has more
callers.** Asked as a return is, a store doubts what it cannot prove live, so
a pointer a call this check cannot read returned, stored out after a second
such call, is refused as unproven with no free in the function at all:
`keep(a) + (holder = a, 0)` with `holder`'s address kept, which built before,
now does not without `--allow-unknown`. That was accepted rather than taking
proofs only, for the reason this record gives for a return: proofs only leaves
silent every pointer that may have been freed. The proof itself rests on Annex
J.2, as ADR-0057 does, which lists using a pointer to an object whose
lifetime has ended (C17 6.2.4 p2) as undefined: the store uses the value
whether or not anything reads it back.

### Confirmation

Each rule has a mutation in the memory check (`crates/safec-ir/src/memory.rs`
and `memory/`), applied and measured, and a named case in
`tests/cases` that it fails:

* Not calling `stored_away` fails
  `a_freed_pointer_a_callee_stores_through_its_parameter_is_reported` and
  `a_freed_pointer_a_callee_stores_through_a_pointer_to_a_local_is_reported`.
  Asking `reachable_now()` in `leaves` in place of
  `sites_named_by_unknown_code()` fails
  `a_freed_pointer_stored_into_a_box_a_local_a_call_kept_points_at_is_reported`,
  `a_freed_pointer_stored_into_a_box_two_levels_below_a_local_a_call_kept_is_reported`
  and `a_box_holding_a_freed_pointer_stored_into_a_box_a_local_a_call_kept_points_at_is_doubted`,
  and taking one step from the exposed locals and stopping fails the second
  alone. Asking `exposed_locals` in place of
  `locals_named_by_unknown_code()` fails
  `a_freed_pointer_written_into_a_local_a_kept_local_points_at_is_reported`
  at the direct write and
  `a_freed_pointer_stored_through_a_pointer_to_a_local_a_kept_local_points_at_is_reported`
  among the locals a store lands in. Taking the walk's sites for its locals
  fails both and ten others (#655).
  Leaving `sites_named_by_unknown_code()` or `locals_named_by_unknown_code()` out of `leaves` fails
  `a_freed_pointer_stored_in_memory_handed_to_a_call_is_reported` or
  `a_freed_pointer_stored_through_a_pointer_to_a_local_a_call_kept_is_reported`,
  and every store leaving fails
  `a_freed_pointer_stored_in_memory_only_this_function_reads_is_not_reported_where_it_is_stored`.
  Taking a pointer that names nothing to land nowhere fails
  `a_freed_pointer_stored_through_a_pointer_moved_off_a_local_array_is_reported`,
  and doing that and leaving `through_unnamed` out together fails
  `a_freed_pointer_stored_two_levels_below_a_parameter_is_reported` and
  `a_freed_pointer_stored_through_a_pointer_read_out_of_a_parameter_is_reported`,
  which each of the two holds alone;
  and not asking a write of an exposed local fails
  `a_freed_pointer_written_into_a_local_a_call_kept_is_reported`.
* The free not asking `Known::stored_out` fails
  `an_allocation_stored_where_the_caller_reads_it_and_then_freed_is_doubted`,
  intersecting it at a join fails
  `an_allocation_stored_where_the_caller_reads_it_on_one_arm_and_then_freed_is_doubted`,
  every store marking fails
  `an_allocation_stored_in_a_local_and_then_freed_is_not_doubted`, and
  `Known::reborn` leaving it fails
  `an_allocation_made_again_in_a_loop_and_freed_is_not_doubted_for_an_earlier_turns_store`
  (#650).
* Not asking `freed_behind` at a return fails
  `a_box_holding_a_freed_pointer_returned_is_doubted` and, for a return of a
  dereference, `a_box_holding_a_freed_pointer_returned_through_a_dereference_is_doubted`;
  at a leaving store, `a_box_holding_a_freed_pointer_stored_through_a_parameter_is_doubted`
  and, for a direct write of an exposed local,
  `a_box_holding_a_freed_pointer_written_into_a_local_a_call_kept_is_doubted`.
  Doubting every allocation behind it fails
  `a_box_holding_a_live_pointer_returned_is_not_reported` and
  `a_box_holding_a_live_pointer_stored_through_a_parameter_is_not_reported`.
  `handed_below` not reading `stale` fails the three
  `a_box_filled_on_one_turn_and_freed_through_on_the_next_..._is_doubted`.
* Not calling `returned` fails `a_function_that_returns_what_it_freed`,
  `a_return_after_a_free_on_one_arm_only`, `a_parameter_freed_and_returned`,
  `an_escaped_local_freed_and_returned` and
  `an_allocation_handed_to_a_call_and_returned`, and the reported half of
  `a_free_after_the_write_into_the_return_place_is_not_asked_about` in
  `crates/safec-ir/tests/freed.rs`.
* Dropping a parameter's site from what is asked unless it is
  `SiteState::Freed`, which is the withdrawn option, fails
  `a_parameter_freed_on_one_arm_and_handed_on_by_its_caller` and
  `a_parameter_freed_then_handed_to_a_call_and_returned`, which lose the
  report at the return, and three more cases about a parameter. Neither goes
  silent since ADR-0042: each keeps an `SC0407` at a call.
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

A new `Kind` is `error[E0004]` in `verdict`'s `ordered` and in
`driver/words.rs`'s `memory_finding`, which is the guard the compiler holds.

**`ordered` answering `true` is held only against `false`.** Answering
`false` fails `a_function_that_returns_what_it_freed` and
`a_parameter_freed_and_returned`; answering as a dereference does,
`earliest.is_some_and(|freed| freed.sequenced)`, changes nothing, measured,
because a `free` returns `void` and so is always followed by a sequence point
before the write into the return place. `realloc` can sit in the returned
expression unsequenced, but it only ever makes a site unproven, never freed.
The arm is right for the reason given above and no C program shows it.

**The boundary is held too.** A free placed between the write and the return,
which no C program builds, is silent, and
`a_free_after_the_write_into_the_return_place_is_not_asked_about` pins it so
that closing it fails a named test. `docs/c-family.md` carries the requirement.

### Consequences

* Good, because a function that frees what it returns is reported, proved
  where it is proved, in the function that did it, whatever its caller does
  with the result.
* Bad, because a pointer returned after anything this check reads as a doubt
  is refused, with no free in the function required: after a call this check
  cannot read was handed it or its address (`enroll(p); return p;`,
  `grow(&b); return b;`), after any such call where the pointer is a parameter
  (`log_line(); return p;`, `return dup(p);`), after its address was taken
  (`int **pp = &p; return p;`), and on `realloc`'s failure branch. Each was
  already refused as a dereference; a return is far more common. A false
  refusal, measured.
* Bad, because the join does not tell paths apart, so the free-and-null idiom
  `if (c) { free(p); p = 0; } return p;` is refused: the site is freed on one
  arm and held on the other, and the two meet as unproven. The dereference of
  the same pointer was already refused.
* Bad, because a pointer read out of memory and returned, `return *box;`, is
  silent in the callee and believed by the caller. That is the boundary
  ADR-0017 draws for a dereference, now drawn for a return too. `return *&p;`
  was the same until the lowering folded `*&p` to `p`, which C17 6.5.3.2
  defines it as.
* What would reverse this: summaries of functions this translation unit
  defines, which would let a caller read what a callee returns instead of
  believing it.

## Pros and Cons of the Options

Measured on a throwaway prototype against seven programs with a defect and ten
without, all returning a pointer, and against the corpus; the review of the
first implementation added the programs named in the Decision Outcome and in
Consequences.

### Every allocation, as a dereference

* Good, because nothing reaching the return is left silent in either
  function.
* Bad, because it also refuses returning a parameter after any call this check
  cannot read, returning `p` on `realloc`'s failure branch, and a wrapper
  returning another call's result while a pointer parameter exists: five of the
  ten, where the narrowed option refused two.

### A parameter's allocation only where proved freed

* Good, because it builds the three parameter idioms above.
* Bad, because a caller that hands the result on as an argument asks nothing,
  so a parameter freed on one arm and returned is silent in every function.
  Chosen first, and withdrawn for this.

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
* `crates/safec-c/src/lowering.rs`, whose `Stmt::Return` arm writes the return
  place, marks the end of the `return`'s full expression with an
  `Element::Sequenced`, ends the storage of every scope it leaves (#114), and
  ends the block with `Terminator::Return`. Nothing that frees an allocation
  runs between the write and the return, which is what makes asking at the
  write the same as asking at the return for C. `docs/c-family.md` is
  where that becomes a requirement on another frontend.
