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
* **Two spellings of one program have to answer the same way.** The door this
  rule is written for is the write this check cannot follow; the shape it *can*
  follow is the narrower thing. Keying the rule on that shape is what made
  `**ppp = q` and the same program written through a temporary disagree.

## Considered Options

* Call `Known::replaced` wherever a write through a projection is not
  ADR-0028's certain one, narrowed to the escaped locals declared with the type
  the write writes, and to every local where that type is a character type.
* The same, unnarrowed: every escaped local that holds a site.
* The same, narrowed to writes whose pointee is a pointer type.
* The same, but only for a place whose projection is exactly one `Deref`, which
  is the shape the arm around it can follow.
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
it writes. A write passes `replaced_by`, which answers for the locals declared
with the type the write writes, from `TranslationUnit::place_ty` of the place
being written through.

**It fires for every projection that is not the certain write, and not for the
one shape the arm can follow.** This check follows a write through exactly one
`Deref`, because the edge `Rvalue::Address` records is one step. `**ppp = q` is
a place with two, and keying the rule on the followable shape left it saying
nothing at all about the write that needs it most: measured, `**outer = q` was
`error[SC0402]` at exit 1 while the same program with the intermediate pointer
read into a temporary was a warning at exit 0. Two review lenses found that
independently. A deeper projection needs no extra clause to be uncertain,
because `written_through` answers about the local the place starts at.

**The narrowing is type identity and not an interpretation of a type.** It
compares two `TyId`s, which
[ADR-0013](./0013-the-translation-unit-carries-the-target-and-answers-what-a-type-is-worth.md)
interns so that equality of ids is equality of types. `Allocations::is_pointer`
is not called, which matters because #202 was filed saying the fix waited on
the check learning to read types: it does not. What makes the narrowing sound
about a *program* is C17 6.5 p7, which gives an object an effective type and
permits an lvalue of another type to access it only in the cases that clause
lists.

**One of those cases is a character type, and it is reachable here.** An
earlier draft of this record said a `char *` that aliases a pointer object
needs a cast and that this frontend parses none. That is false: C17 6.3.2.3 p1
and 6.5.16.1 p1 make the `void *` round trip implicit in both directions, so
`void *v = &p; char *c = v;` is a conforming program this frontend accepts, and
copying one pointer's object representation through `c` is defined. Review
found it, compiled the program with `clang -std=c17 -pedantic-errors` and ran
it under AddressSanitizer to show it has no use after free in it, and this
compiler answered `error[SC0402]` at exit 1. So a write whose type is
`Ty::Char` reaches every escaped local and the narrowing applies to the rest.

**Which row each direction lands on, because this decision both distrusts and
exempts.** RK-063 is the entry that says the exempting half is the one to rank.
Distrusting too much costs a proof: a report drops from `error` to `warning` and
is still made, and `--deny-unknown` still fails the build on it. Exempting too
much *keeps* a marking off a local, a local that is not marked keeps whatever
proof it had, and a proof is a report. So the exemption's failure is the false
positive this record removes, which is row 4, and neither direction can reach
row 6. That is what makes reading a declared type affordable here where RK-067
warns against it: `docs/c-family.md`'s fourth requirement costs a silence when a
frontend breaks it and this fifth one costs a suspicion.

### Confirmation

Eight cases in `crates/safec/tests/cases` and one hand-built unit in
`crates/safec-ir/tests/freed.rs`. Nine mutations in
`crates/safec-ir/src/memory.rs`, each measured against the whole workspace with
`--no-fail-fast`, each failing named tests and nothing else.

Dropping the `replaced_by` call fails seven at once:
`a_write_through_a_pointer_this_check_cannot_follow_may_have_replaced_what_an_escaped_local_holds`,
`a_write_that_replaces_what_an_escaped_local_holds_needs_no_call_and_no_parameter`,
`a_write_through_more_than_one_deref_may_have_replaced_what_an_escaped_local_holds`,
`a_write_through_a_character_pointer_may_have_replaced_what_any_escaped_local_holds`,
`a_write_that_may_land_beside_its_target_may_have_replaced_what_an_escaped_local_holds`,
the hand-built
`a_write_whose_type_this_check_cannot_name_distrusts_every_escaped_local`, and
`a_use_after_free_a_write_took_the_proof_of_still_fails_a_build_that_denies_unknown`,
which is the first of them with the flag on. That last case is what makes the
downgrade acceptable rather than merely cheap: the report this record turns from
an `error` into a `warning` still fails a build that asks for it.

The predicate answering `true` for every local fails
`a_write_through_a_pointer_to_an_int_leaves_what_an_escaped_local_holds_alone`
and
`a_write_this_check_is_certain_about_leaves_what_another_escaped_local_holds_alone`,
whose proved double frees drop to warnings and whose exit 1 drops to exit 0.
Comparing the types with `!=` rather than `==` fails those two and two more; it
is written down because a narrowing has more than one wrong version and a table
that measures one of them holds one of them, which is RK-038.

Four mutations fail one case each, which is why the condition is what it is
rather than any of the things next to it. Keying the door on the shape this
check can follow, `one_step && !certain`, fails
`a_write_through_more_than_one_deref_may_have_replaced_what_an_escaped_local_holds`.
Requiring an empty target set as well fails
`a_write_that_may_land_beside_its_target_may_have_replaced_what_an_escaped_local_holds`.
Ignoring ADR-0028's certainty condition, so that the rule fires at every write,
fails
`a_write_this_check_is_certain_about_leaves_what_another_escaped_local_holds_alone`.
Dropping the character type from the exception fails
`a_write_through_a_character_pointer_may_have_replaced_what_any_escaped_local_holds`.

Spelling the fallback `is_some_and` rather than `is_none_or`, so that a write
this check cannot name the type of narrows to nothing rather than to
everything, fails
`a_write_whose_type_this_check_cannot_name_distrusts_every_escaped_local` and
nothing else. It is hand-built because `TranslationUnit::place_ty` answers
`None` only for a `Deref` of something that is not a pointer, which the lowering
does not build; a coverage pass found the arm unreached by every case above.

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
method, not because a second case was written. That giving the method a second
caller has made no earlier record's mutation vacuous was measured: the
mutations ADR-0017, ADR-0028 and ADR-0029 name still fail the tests those
records name.

### What the rule does not take

The IR's types are not enough on their own, and this rule is only as good as
they are. `docs/c-family.md`'s fifth requirement says what it asks of a
frontend and what breaking it costs. #205, an initializer never checked against
the assignment constraint, is a shape that breaks it today: `char **alias =
*outer;` is accepted where `alias = *outer;` is `error[SC0302]`, and a write
through that `alias` is narrowed away from an `int *` local. What that costs is
the false positive this record is about rather than a silence, which is the
whole reason it does not block the decision.

### Consequences

* Good, because five programs C defines stop being an error no flag
  suppresses, which is what `docs/concept.md` promises anyone adopting this
  compiler incrementally.
* Good, because no existing case moves. The whole workspace passes unchanged,
  measured.
* Good, because the fact, the field and the method are the ones ADR-0029 left,
  so there is no second slot for a later reader to forget to test.
* Bad, because a real double free or use after free through a sharer, where the
  write did nothing of the kind, is a warning rather than an error. It is still
  reported and `--deny-unknown` still fails the build on it. What would recover
  it is following a pointer read out of a projection, which nothing here does.
* Bad, because the analysis now depends on the IR's types being honest about
  more than ADR-0030 asked.
* What would reverse this: a points-to analysis that follows what a pointer
  points at rather than which local holds what, which would make "this check
  cannot pin the write down" a much smaller set of programs.

## Pros and Cons of the Options

### Narrowed to the written type, with a character type reaching everything

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

### Only for a place with exactly one `Deref`

* Good, because it is the shape the arm around the rule can follow, so the rule
  sits beside the code that reads the same condition.
* Bad, because it answers two spellings of one program differently.
  `a_write_through_more_than_one_deref_may_have_replaced_what_an_escaped_local_holds`
  is that program and is an `error[SC0402]` at exit 1 under this option, while
  the case above it, which differs only in reading the intermediate pointer
  into a temporary, is a warning at exit 0.

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

Two of the doors this rule covers were found by review rather than by the
design: the place with more than one `Deref`, and the character type. Both were
one mistake, which is a rule stated over the shape the code could already see
rather than over the thing the rule is about.
