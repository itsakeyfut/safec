---
status: "accepted"
date: 2026-11-10
decision-makers: itsakeyfut
---

# A pointer's value is asked beyond a dereference: a freed one where it is certainly freed, one that may be null where it is moved

## Context and Problem Statement

A freed pointer's value is indeterminate (C17 6.2.4 p2), and Annex J.2 makes
using it undefined, whatever it is used for. A pointer that is null points at
no array object, so moving it by `+` or `-` is undefined (C17 6.5.6 p8). The
memory check asked a freed pointer only where it is dereferenced, returned,
stored or handed to a call, and the nullability check asked a null one only
where it is dereferenced. `free(p); return p != 0;` built in silence, and so
did `int *e = a + n;` with `a` a parameter nothing shows is not null (#891).

The documents disagreed about whose question the first is:
`docs/safety-model.md` says an indeterminate pointer is level 1's, and
`docs/diagnostics.md` called reading a freed value an axis with no check.

## Considered Options

* Leave both unasked, and make the documents agree.
* Ask every use of a value that may have been freed, or may be null.
* Ask a freed value where it is certainly freed, and arithmetic on a value
  that may be null.
* Ask both only where certain.

## Decision Outcome

Chosen option: the third, by the author's decision on #891, from a prototype
that measured the second.

* **A freed value used by a comparison, another operator whose result is no
  pointer, or a branch is `SC0402` where it was certainly freed**
  (`report::used_by_value`, reading `reads::operated_on_in_element` and
  `operated_on_in_terminator`). A move by `+` or `-` carries the pointer to
  where it is used, as a copy does, and is asked there: asked at the move
  too, `p[i]` after a free was said twice. Asking a value that may have been
  freed doubted 233 cases of the corpus, mostly at comparisons of pointers
  this check stopped following, against 17 certain.
* **A move by `+` or `-` of a pointer the nullability check has not shown is
  not null is to be doubted, and reported where it is null**, as a dereference
  is. #884 implements it, and its cases name their mutations in their notes;
  until it lands, `docs/safety-model.md` lists the arithmetic as not asked.
  Measured on the prototype, it moves 74 cases.

### Confirmation

Each in `tests/cases/memory/`, with its mutation in its note in
`tests/cases.rs`:

* `a_freed_pointer_compared_with_null_is_reported` and
  `a_freed_pointer_branched_on_is_reported` go silent when no value use is
  asked.
* `a_pointer_that_may_have_been_freed_compared_with_null_is_not_asked` is
  reported when a doubt is asked too, and
  `a_freed_pointer_copied_and_never_used_is_not_asked` when a copy is.

### Consequences

* Good, because the two documents now agree, and a certain use of a freed
  value is reported wherever it is used.
* Bad, because a pointer not shown to be certainly freed, compared or
  branched on, is a silence: one freed on some path only, one read back out of
  memory, and one held by a local whose address was taken.
  `docs/safety-model.md` lists it.
* Bad, because a pointer difference, `p - q`, of a freed pointer would not be
  asked: its result is no pointer to carry the state on, and `-` is not asked
  as a use. The frontend refuses a pointer difference today (`SC0304`), so no
  program reaches it yet.
* Neutral, because a certain use is said at each caret it is used at: `if (c)
  return *c;` says it at the branch and at the dereference, and `if (p == 0)`
  once, though it compares `p` and branches on what the comparison made.
* A relational comparison of null pointers (C17 6.5.8 p5) is not asked.
