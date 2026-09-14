---
status: "accepted"
date: 2026-09-14
decision-makers: itsakeyfut
---

# A free of a may-set is a fact about the set, and it replaces what the members say

## Context and Problem Statement

`Known::points_to` is a may-set: a local reaching two sites may hold either. The
report has applied that rule since the check landed, and `verdict`'s doc comment
says why. The transfer did not. `Callee::Frees` wrote `SiteState::Freed` into
every site the argument reached, as though the set were a must-set.

```c
int *a = malloc(4);
int *b = malloc(4);
int *p;
if (c) { p = a; } else { p = b; }
free(p);
free(a);
```

Both allocations were marked freed, so `free(a)` was a proved double free:
`error[SC0401]`, exit 1, on a program that frees each allocation exactly once
when `c` is false. `Conclusion::Unsafe` is an error at every safety level, so no
flag suppressed it. That is issue #164 and it had been true since the check
landed.

Writing `SiteState::Unknown` instead is not the fix, because it loses proofs that
are real. `if (c) { p = malloc(8); } free(p); free(p);` reaches two sites and the
second free is a double free whichever one it holds.

## Decision Drivers

* The difference between the two programs is not in the states. It is that one
  frees the same set twice and the other frees a set and then one member by
  name. Nothing in `SiteState` can say that, because it holds a state per site.
* A false proof is the defect this check has been wrong about before, and
  `Conclusion::Unsafe` is the one conclusion no flag can suppress.
* Three more lattices will be written against this trait and each will have a
  may-set whose members a transfer wants to write on.

## Considered Options

* **Record it on the local that named the set.**
* **Record it on a table keyed by the set itself**, `Vec<(Vec<bool>, Span)>` on
  `Known`.
* **Write `Unknown` on every member and record nothing.**

## Decision Outcome

Chosen option: **record it on the local that named the set**.

A free whose argument reaches more than one site writes nothing on the members:
they become `SiteState::Unknown`, and the local gets `Held::freed`, the span
where the free was. `Reached::SetFreed(Span)` is how the report reads it, and
`verdict` treats it as a freed site with no `made` span, because naming one of
several allocations as *the* one is the same may-set mistake about a label.

A free reaching exactly one site keeps the rule it had. That is why every
single-allocation program in the corpus is unchanged.

**The set fact replaces what the members say, and answering it is not the end of the question.** This is
the part worth the record. The members were just marked `Unknown` by the same
rule, `verdict` proves nothing while anything is unknown, so answering both
folds that `Unknown` in beside the proof and the proof goes. Written the other
way first and measured: `a_branch_that_allocates_either_way` dropped to a
warning, which is the rule eating the marking it had made one step earlier.

Replacing is not returning, and the first version did both. The rules after it
are about this local too: a set it freed says nothing about whether something
holding its address has put a different pointer there since. Skipping them made
`free(p); opaque(&p); free(p);` over a two-site `p` a proved double free, which
is the false proof this record exists to stop, arriving through the door it had
just opened.

**It is joined by intersection, where every other field beside it is joined by
union.** The rest of `Held` holds may-facts, which grow where paths meet. This
one is a proof, and a path that did not free proves nothing. Joined like its
neighbours it reports a proved double free on `if (c) { free(p); } free(p);`.

**It lives in `Held` rather than on `Known`** because an assignment destroys it:
what a local is given now was not freed. That is ADR-0018's rule for what that
struct holds, and `Held::clear` answers for the field because it destructures.

`Analysis::height` gains `locals`.

### Confirmation

Each mutation applied on its own to `crates/safec-ir/src/memory.rs`, the whole
workspace suite run with `--no-fail-fast`, the file restored.

| Mutation | Named test that fails |
|---|---|
| a may-set free marks every member `Freed` again | `a_free_of_one_of_two_allocations_by_name`, on the severity, and three cases that gain back a proof they should not have |
| the members stop being provable and nothing is recorded on the local | `a_branch_that_allocates_either_way` and `a_free_of_either_of_two_locals_names_no_allocation`, both on the severity: this is the "give up" answer the option below rejects |
| `Known::reached_by` answers the set fact **beside** the members rather than instead of them | the same two, for the reason above |
| `Known::reached_by` returns as soon as it has answered the set fact | `a_may_set_freed_then_written_through_an_alias`, which becomes a proved double free although something holding the local's address may have put a fresh pointer there between the two frees |
| `Held::union` joins the proof by union rather than intersection | `a_free_of_a_may_set_on_one_arm_only`, which becomes a proved error on a path that never freed |
| `Held::clear` keeps the fact | `a_local_given_nothing_forgets_the_set_it_freed`. It has to be a local given a *constant*: `p = malloc(8);` is a copy out of a temporary, and the copy replaces the whole row rather than clearing it, so it hides this |
| the rule fires on a single site as well | thirty-eight cases, which is every proved double free and use after free in the corpus |
| a variant added to `Reached`, or a field to `Held` | does not compile: `error[E0004]` at `verdict`, `error[E0063]` and `error[E0027]` at `Held` |

`Analysis::height` is held by nothing here either, as ADR-0016 says of the method
and ADR-0018 and ADR-0019 say of the last two fields.

### Consequences

* Good, because a program that frees each allocation exactly once on one of its
  paths is no longer refused.
* Good, because `Reached` now has a name for a proof about a set, which is the
  thing the three later lattices will each need.
* Bad, because a copy taken **before** the free does not carry the fact.
  `int *q = p; free(p); free(q);` over a two-site `p` was a proved error and is
  now a warning. The table keyed by the set would keep it; it needs a canonical
  form or `PartialEq` is unstable and the fixpoint may not settle, which is
  ADR-0016's row 5 against this option's row 4. An issue names the shape.
* Bad, because the three cases ADR-0019 added drop from `error[SC0402]` to
  `warning[SC0402]`. That record predicted it and called it the right
  direction: the proof they carried rested on the rule this one calls wrong.

## Pros and Cons of the Options

### Record it on the local that named the set

* Good, because the local is the only thing in this lattice that names a set,
  and the fact is about a set.
* Good, because an assignment ends it for free: `Held::clear` already answers
  for every field.
* Bad, because two locals naming the same set do not share it.

### Record it on a table keyed by the set

* Good, because the set is the key, so a copy taken before the free is covered.
* Bad, because a list of sets needs a canonical order or the value's `PartialEq`
  is unstable, and an unstable equality in a fixpoint is a build that does not
  stop rather than an answer somebody can read.

### Write `Unknown` on every member and record nothing

* Good, because it is three lines and needs no new field.
* Bad, because it gives up a proof that is real: freeing the same may-set twice
  is a double free whichever member it holds, and two corpus cases hold exactly
  that.

## More Information

* `Held::freed`, `Reached::SetFreed` and the `Callee::Frees` arm of
  `Allocations::terminator` in
  [`crates/safec-ir/src/memory.rs`](../../crates/safec-ir/src/memory.rs).
* [`docs/diagnostics.md`](../diagnostics.md) carries what it means for a reader
  of `SC0401`.
* RK-035 in the review knowledge bank is the entry that recorded the report side
  of this rule, one level down.
