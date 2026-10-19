---
status: "accepted"
date: 2026-10-19
decision-makers: itsakeyfut
---

# A span from an expansion is located in a copy of the macro's body

## Context and Problem Statement

`Span` is `(file, start, end)`, and its doc comment has expected a third coordinate since it was written: a macro expansion, and later a template instantiation, needs to say both where code is written and where it came from. `docs/roadmap.md` asks that it serve three consumers rather than macros alone: a macro expansion, a template instantiation, and an operation no source text produced. Object-like macros, [#546](https://github.com/itsakeyfut/safec/issues/546), are the first thing that builds it.

It is a safety question rather than a display one, because analyses reach conclusions by comparing spans:

* **Containment.** `memory/report.rs::inside` takes a read strictly inside a call's span as ordered before the call (ADR-0043), and `driver/words.rs::unordered` asks whether a span lies inside a parameter's length.
* **Identity.** Equal spans are taken for one event or one name. This is done by:
  * `memory/known.rs::noticed` and the pending-call tests beside it;
  * the nullability fold in `nullability.rs`;
  * `transfer.rs`'s returned-by test;
  * `sema.rs`'s searches for an identifier or a tag by its span;
  * the lowering's `locals: HashMap<Span, LocalId>`.
* **Text.** `snippet` gives names, numbers and what the memory check recognises as `free` or `malloc`.

A first design, kept in [#544](https://github.com/itsakeyfut/safec/issues/544), located every token of an expansion where the macro was used. A prototype showed it silencing the memory check in three ways that the same program written out is reported for:

* a call that crossed an expansion's edge contained the whole expansion, so `inside` skipped a read in another operand;
* equal spans made `Known` take distinct elements for one event;
* two declarators from one expansion collided as `locals` keys.

## Decision Drivers

* A comparison that answers differently for a macro than for the same code written out is a silence waiting to happen, and a believed silence is the worst thing this compiler can do.
* ADR-0043 chose that a mistake falls on the reporting side.
* `Span` is `Copy` and in every token, node and IR element; `Token` is held to 16 bytes (ADR-0006).

## Considered Options

* **An instance file per expansion:** every expansion is a copy of the macro's body registered as a file of its own, and its tokens are located in it.
* **The use as the location**, with the spelling in a table and a mark carried through joins so that comparisons can be made conservative where it is set.

And for a node across an expansion's edge:

* **Lift a node that holds a whole expansion, and refuse one that cuts an expansion.**
* **Lift every node across an edge.**

## Decision Outcome

Chosen: **an instance file per expansion; a node holding a whole expansion is lifted to where it was used, and a node that cuts one is refused.**

**Each expansion is a file of its own.** The source map registers, for every expansion, a file holding a copy of the macro's body, an *instance*, whose `FileName` records two spans:

* `spelled`, where the body was written in its `#define`;
* `at`, the macro's name where it was used.

For a macro used inside another macro's body, `at` lies in the outer instance, so instances form a chain to a real file. Each token the expansion produces is located in its instance at the position its text has there. `Span` keeps `(file, start, end)`, and `Token` keeps its 16 bytes: the third coordinate is the instance's record, reached through the span's file.

**So inside one expansion, every comparison answers as it does for the body written out.**

* Containment, equality and ordering between two spans of one instance are what they are between the same text written out.
* `snippet` reads the instance, which is the body's text, so names, numbers and allocator names resolve as written.
* Two expansions are two files and never compare equal or contained, including two expansions of one macro, so `#define C g(*p)` used as `C + (free(p), C)` keeps its report.
* Two declarators from one expansion have distinct name spans. Every identity test listed above therefore stays a test of `Span`'s derived equality, unchanged.

**A node across an expansion's edge.**

* **A node that holds a whole expansion** is located in the outer file, as `N + 1` and `g(N)` are. Each endpoint that lies in an instance is lifted along the chain to the span of its use first, so `N + 1` runs from `N`'s use to `1`.
* **A node that holds some of an expansion's tokens and not others** is refused, as valid C this compiler does not read yet. That is the shape the prototype silenced: `F(p)` with `#define F (x = *p) + release`. `Span::to` keeps its assertion that both spans are in one file, so a cut node that is not refused panics with "cannot join spans from different files" rather than being joined.

**Containment lifts.** `inside`, `unordered`'s test, and any comparison of the same kind lift both spans along their chains to the nearest file they share, and compare there. A read inside `g(N)`'s argument is then strictly inside the call, as it is written out. Two spans with no file in common are not inside each other. Identity tests do not lift, so each token stays itself.

**`Origin` is unchanged.**

* A macro's body was written, so an operation from it is `Written` with an instance span.
* A generated operation has no spelling and is `Generated` as it is today.
* A template instantiation is the same model as a macro: an instance of the template's text, used at the point of instantiation.

**A diagnostic** at an instance span renders the instance's text, which is the body, under the macro's name, and adds a note at `at`: expanded here.

### Confirmation

Nothing builds an instance yet, so nothing holds this decision today. It is unenforced until #546, which owes one case for each shape the prototype silenced. Each case is written out and as a macro, and the two must agree:

* A call that cuts an expansion is refused, and the same call holding the whole expansion is reported as it is written out. Building the cut call by joining across files instead fails it.
* `(x = *q) + (*slot = q, 0) + release_all()` from one expansion reports the use of `q`. Locating every token at the use fails it.
* `int *a = malloc(4), *b = 0;` from one expansion, with `a` used after its free, reports it. Keying `locals` by the use fails it.
* A read inside `g(N)` is taken as the call's argument. Comparing without lifting fails it.
* A free from a macro inside a parameter's length is worded as a length's. Comparing without lifting fails it.
* A diagnostic in an expansion shows the body and the use. Dropping the note fails it.

The prototype that showed the silences is the starting point for these cases.

### Consequences

* Good, because inside an expansion every analysis sees what it would see written out, the standard every finding here is measured against.
* Good, because the identity tests, `Origin`, `Span`'s size and every artifact that prints a position are unchanged.
* Good, because a mistake in the cut rule is a refusal or a named panic, never a join.
* Bad, because a macro whose body is not a whole operand where it is used, such as a function name and an operator with no right operand, is refused although it is C.
* Bad, because every expansion is a file in the source map, so a file with many expansions holds many small copies.
* Bad, because containment has to lift, and a comparison written as plain offsets is right for written-out code and wrong for a macro. The cases above are what catch one.
* What would reverse this: a need to accept cut expansions, which would need a rule for a node whose span is shared with its parent, since `Known` takes equal spans for one event.

## Pros and Cons of the Options

### An instance file per expansion

* Good, because positions within an expansion are as distinct as written out.
* Bad, because containment across an edge needs lifting.

### The use as the location, with a mark

* Good, because no file is registered per expansion.
* Bad, because every token of an expansion shares a location. Every comparison would then have to consult the mark, and one that did not would be silent with nothing to hold it: the prototype found three such places, in a check nobody had yet written for macros.

### Lift every node across an edge

* Good, because no C is refused.
* Bad, because a node and its parent can be lifted to one span, as `-release` used as `X(p)` makes the negation and the call, and `Known` takes equal spans for one event.

## More Information

* [ADR-0043](0043-a-call-is-ordered-after-what-its-own-arguments-read-and-a-span-inside-it-says-which.md), whose strictness this keeps.
* [ADR-0006](0006-carry-a-position-rather-than-the-text.md), which holds `Token` to 16 bytes.
* [`docs/c-family.md`](../c-family.md#an-operation-must-be-able-to-say-it-was-not-written), on what the coordinate serves.
* [#544](https://github.com/itsakeyfut/safec/issues/544), where both designs and the prototype are, and [#546](https://github.com/itsakeyfut/safec/issues/546), which builds this.
