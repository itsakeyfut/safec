---
status: "accepted"
date: 2026-09-22
decision-makers: itsakeyfut
---

# A write this check cannot pin down replaces what an escaped local of the written type holds

## Context and Problem Statement

```c
int *p = malloc(4);
int *r = p;
*outer = &p;
int **alias = *outer;
*alias = q;
free(p);
*r = 1;
```

`alias` is read back through a projection, so `Held::writes_to` names nothing
for it and the write through it is dropped. `*alias = q` may therefore have put
a different allocation in `p`, `free(p)` need not free what `r` holds, and
`*r = 1` need not touch anything freed. The program is defined on that
execution, and it was `error[SC0402]` at exit 1, which `Conclusion::Unsafe`
makes an error at every safety level with no flag to suppress it. That is issue
#202.

[ADR-0029](./0029-a-call-this-check-cannot-read-replaces-what-an-escaped-local-holds.md)
closed the same false proof where the writer is a callee, and named this door in
its own Decision Drivers as one it did not close. Nothing about the defect needs
a call: the program above has none, and neither does the one in #202's comment,
where the address never leaves the function at all.

What held it back was cost rather than doubt. The honest rule is that such a
write may land in **any** escaped local, and applying it as written unproves
every escaped local at every dereferencing write, `*p = 1` into an `int`
included.

## Decision Drivers

* The rule ADR-0029 wrote for a callee is true word for word of a write this
  check cannot pin down. Leaving one door open leaves the false proof
  reachable, and a reader who finds the rule at one door and not the other has
  no way to tell which is the decision.
* The sites are shared. A `free` of a local whose contents this check has not
  kept up with writes `SiteState::Freed` on sites that every other local
  holding them reads, which is what turns one stale row into a proof about a
  sharer. RK-048 is that asymmetry from the other side.
* A rule whose cost is unmeasured is not a rule anybody can rank.
  `CLAUDE.md` puts a false positive on row 4 and this decision has to say which
  row *its own* failure lands on, in both directions.
* `docs/safety-model.md` ranks a silence below a false positive. Whatever this
  does, a use after free of the same shape has to stay reported.

## Considered Options

* Call `Known::replaced` wherever the write is not ADR-0028's certain one,
  narrowed to the escaped locals declared with the type the write writes.
* The same, unnarrowed: every escaped local that holds a site.
* The same, narrowed to writes whose pointee is a pointer type.
* Key the door on an empty target set, which is the shape #202's body
  describes, rather than on ADR-0028's certainty condition.
* Clear the escaped local's row instead of marking it.

## Decision Outcome

Chosen option: the first. The fact and the field are ADR-0029's, and the method
that writes them is the same one; what a write has and a call does not is a
type, and that is the whole of the difference between the two call sites.

`Known::replaced` takes a predicate saying which locals the writer could have
reached. `Callee::Opaque` passes one that answers `true` for every local,
because nothing here reads a callee's body and so nothing here knows what type
it writes. The write passes one that answers for the locals declared with the
type the write writes, which `TranslationUnit::place_ty` gives for the place
being written through.

**The narrowing is type identity and not an interpretation of a type.** It
compares two `TyId`s, which
[ADR-0013](./0013-the-translation-unit-carries-the-target-and-answers-what-a-type-is-worth.md)
interns so that equality of ids is equality of types. `Allocations::is_pointer`
is not called and no arm of `Ty` is read, which matters because #202 was filed
saying the fix waited on the check learning to read types: it does not. What
makes the narrowing sound about a *program* is C17 6.5 p7, which gives an object
an effective type and permits an lvalue of another type to access it only for a
character type. This frontend parses no cast, so no conforming program it
accepts can make a `char *` that aliases a pointer object.

**Which row each direction lands on, because this decision both distrusts and
exempts.** RK-063 is the entry that says the exempting half is the one to rank.
Distrusting too much costs a proof: a report drops from `error` to `warning` and
is still made, and `--deny-unknown` still fails the build on it. Exempting too
much *keeps* a marking off a local, a local that is not marked keeps whatever
proof it had, and a proof is a report. So the exemption's failure is the false
positive this record removes, which is row 4, and neither direction can reach
row 6. That is what makes reading a declared type affordable here where RK-067
warns against it: `docs/c-family.md`'s fourth requirement costs a silence when a
frontend breaks it and this fifth one costs a suspicion, so #204 and #205, which
are the two shapes that break the fourth today, do not block this.

### Confirmation

Six cases in `crates/safec/tests/cases`. Six mutations in
`crates/safec-ir/src/memory.rs`, each measured against the whole workspace, each
failing named cases and nothing else.

Dropping the `Known::replaced` call from the `Projection::Deref` arm, and the
predicate answering `false` for every local, each fail
`a_write_through_a_pointer_this_check_cannot_follow_may_have_replaced_what_an_escaped_local_holds`,
`a_write_that_replaces_what_an_escaped_local_holds_needs_no_call_and_no_parameter`
and
`a_write_that_may_land_beside_its_target_may_have_replaced_what_an_escaped_local_holds`,
each back to an `error` at exit 1, and
`a_use_after_free_a_write_took_the_proof_of_still_fails_a_build_that_denies_unknown`,
which is the first of those programs with the flag on. That case is what makes
the downgrade acceptable rather than merely cheap: the report this record turns
from an `error` into a `warning` still fails a build that asks for it.

The predicate answering `true` for every local fails
`a_write_through_a_pointer_to_an_int_leaves_what_an_escaped_local_holds_alone`
and
`a_write_this_check_is_certain_about_leaves_what_another_escaped_local_holds_alone`,
whose proved double frees drop to warnings and whose exit 1 drops to exit 0.
Comparing the types with `!=` rather than `==` fails those two and two more; it
is written down because a narrowing has more than one wrong version and a table
that measures one of them holds one of them, which is RK-038.

Keying the door on an empty target set fails
`a_write_that_may_land_beside_its_target_may_have_replaced_what_an_escaped_local_holds`
alone, and ignoring ADR-0028's certainty condition so that the rule fires at
every write fails
`a_write_this_check_is_certain_about_leaves_what_another_escaped_local_holds_alone`
alone. Those two cases are why the condition is what it is rather than either
thing next to it.

`error[E0061]` is the compiler's half: `Known::replaced` takes a parameter, so
the `Callee::Opaque` call site cannot go on compiling without saying what a
callee may reach.

Every case reports through a **second local sharing the allocation**, because
ADR-0017 already takes away the report about the escaped local itself and a case
that frees or reads through that local alone observes nothing whatever this
record says. RK-048 is that hole.

**No mutation fails
`a_write_through_a_pointer_this_check_cannot_follow_may_have_replaced_what_an_escaped_local_holds`
alone**, and it is kept anyway: it is #202's own program, and it and the case
beside it are the two ways a pointer leaves this check's sight, through a
parameter and through a chain of locals with no call and no parameter in it. A
fix keyed on either shape alone passes the other.

**Held by nothing**: that `Callee::Opaque` passes an unnarrowed predicate.
Narrowing it would need the type a callee writes, which nothing here reads, so
there is no implementation of the other answer to measure against. #134's
annotation is what would change that.

**Held by ADR-0029's cases rather than by new ones**: that a write *before* the
escape marks nothing. `Known::replaced` reads `Known::escaped` for both
producers, and `an_opaque_call_before_the_escape_leaves_the_proof_alone` is the
case that fails if it stops. RK-065 is a guard that held only the order it was
written in, and the order is covered here because the two doors share the
method, not because a second case was written.

### Consequences

* Good, because three programs C defines stop being an error no flag
  suppresses, which is what `docs/concept.md` promises anyone adopting this
  compiler incrementally.
* Good, because no existing case moves. All 757 corpus cases and the whole
  workspace pass unchanged, measured, which is the cost #202's body called
  unknown.
* Good, because the fact, the field and the method are the ones ADR-0029 left,
  so there is no second slot for a later reader to forget to test.
* Bad, because a real double free or use after free through a sharer, where the
  write did nothing of the kind, is a warning rather than an error. It is still
  reported and `--deny-unknown` still fails the build on it. What would recover
  it is following a pointer read out of a projection, which nothing here does.
* Bad, because the analysis now depends on the IR's types being honest about
  more than ADR-0030 asked. `docs/c-family.md` carries the requirement and what
  breaking it costs.
* What would reverse this: a points-to analysis that follows what a pointer
  points at rather than which local holds what, which would make "this check
  cannot pin the write down" a much smaller set of programs.

## Pros and Cons of the Options

### Narrowed to the written type

* Good, because it keeps the proofs a write of an `int` cannot have touched,
  which is every `*p = 1` in an ordinary function.
* Good, because the narrowing can only keep a proof, so getting it wrong is the
  row-4 direction rather than the row-6 one.
* Bad, because it reads a declared type, which RK-067 says is an invariant
  nobody enforces. The requirement is written into `docs/c-family.md` for that
  reason.

### Unnarrowed, every escaped local

* Good, because it asks nothing of the IR's types at all.
* Bad, because it takes proofs a write cannot have touched. Measured: it drops
  both `..._leaves_..._alone` cases from a proved double free to a warning, and
  `*r = 1` into an `int` is what does it in the first of them.

### Narrowed to a write whose pointee is a pointer

* Good, because it exempts `*r = 1`, which is the write the cost lands on most.
* Bad, because it is the same as the option above wherever the program writes a
  pointer at all: measured, it drops both `..._leaves_..._alone` cases exactly
  as the unnarrowed rule does, because the write that reaches them writes an
  `int **`. It reads a type and buys nothing that identity does not.

### Keyed on an empty target set

* Good, because it is the shape #202's body describes and touches ADR-0028's
  condition not at all.
* Bad, because a write with one named target and `writes_elsewhere` set may
  land beside that target, and the union into the target says nothing about the
  escaped local it landed in.
  `a_write_that_may_land_beside_its_target_may_have_replaced_what_an_escaped_local_holds`
  is that program and stays an `error[SC0402]` at exit 1 under this option.

### Clear the escaped local's row

* Good, because it is the strongest statement of what the write may have done.
* Bad, because a local reaching no site says nothing at all, which is RK-049.
  ADR-0029 measured it at the other door: the `SC0402` goes and a silence is
  the bottom row of `CLAUDE.md`'s list while a false positive is row 4.

## More Information

Issue #202, whose comment corrected its own body: the defect is reachable with
no type question in sight, and the type is what makes the fix affordable rather
than what makes it possible. #143, which the body named as a dependency, is not
what answers it.
