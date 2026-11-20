---
status: "accepted"
date: 2026-11-20
decision-makers: itsakeyfut
---

# A reader of a place matches its shape

## Context and Problem Statement

`Place.projection` was a public `Vec<Projection>` that every analysis read
directly: 106 sites tested `projection.is_empty()`, 11 compared it with
`[Projection::Deref]`, and others pushed, popped and counted steps. A new kind
of step reached none of them as a compile error: adding `Projection::Field`
gave four `E0004` sites, none in `memory`, `nullability` or `reads`, and
`reads::derefs` answered 0 for a place with a step that is not a dereference,
so `p->f->g` would have counted as no dereference. The same class shipped a
silence once (#949). Phase 11 adds struct members (#614), which is that step.

## Decision Drivers

* A new kind of step is a compile error at every reader, not a branch nobody
  revisits.
* No behaviour changes now; every reader keeps today's answer.

## Considered Options

* `Place::shape()` returning an enum every reader matches with every arm.
* Named boolean questions on `Place` (`is_local`, `is_one_deref`, ...).

## Decision Outcome

Chosen option: "an enum every reader matches", by the owner's decision on
#1092.

* `Place.projection` is private to `ir`. A place is built with `Place::local`,
  `Place::new`, `Place::deref`, `Place::prefix` and
  `Place::without_last_deref`, and read with `Place::shape`, `Place::derefs`
  and, for walks that match every `Projection` themselves (printing, the
  interpreter, liveness, the backend, and the few readers of an address's
  steps), `Place::steps`.
* `Shape` is `Local`, `Deref(n)` or `Indexed`, with no `PartialEq`, so it is
  matched rather than compared. A new step is a new variant: `E0004` at every
  reader that has not said what it means.
* A `matches!` or an `if let` over a shape or a step names one arm and lets a
  new one through, which the compiler cannot see; a test reads the sources
  for one.

Named questions were rejected because a new step's meaning would live in
`ir.rs` alone: every caller would go on answering without being asked, and a
misread member would be a false report or a silence.

### Confirmation

* `every_reader_of_a_place_answers_every_shape` fails on a `matches!` whose
  subject is a place's shape or that names a `Projection`, on an `if let`,
  `while let` or `let ... else` naming a `Shape` or a `Projection`, and on a
  comparison of a place's steps outside a test module, in `safec-ir`,
  `safec-c` or `safec-llvm`. Mutation: write `matches!(place.shape(),
  Shape::Local)`, or `if let Shape::Local = place.shape()`, in
  `Strong::assigned`; it fails and names `store.rs`. It cannot see a shape
  bound to a name and then matched with `matches!` by that name; review
  does.
* A reader that asks a prefix of the steps, not their shape, reads the steps:
  `reads::first_of_two_derefs` and the lowering's array decay, each found
  answering differently for a place with an `Index` after the rewrite.
  `the_first_of_two_dereferences_is_read_whatever_follows` holds the first;
  the second is not reachable from C and is held by nothing.
* Adding a variant to `Shape` was measured at 121 `E0004` sites in `safec-ir`
  alone, where a new `Projection` had made none in `memory`.
* A direct read of `projection` outside `ir` is `E0616`.

### Consequences

* Good, because struct members (#614) will be a compile error at every reader
  of a place, each of which then says what a member means to it.
* Bad, because a site that tests a shape spells out every arm, and the
  conversion kept today's answer at each rather than deciding now whether
  `Indexed` means "the local itself" or "no memory reached"; that choice is
  made where a new variant forces it.
* What would reverse this: a place representation with no kinds of step to
  tell apart, which C's aggregates rule out.

## More Information

ADR-0045, #614, #621, #949, #1092.
