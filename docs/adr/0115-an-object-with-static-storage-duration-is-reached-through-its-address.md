---
status: "accepted"
date: 2026-11-20
decision-makers: itsakeyfut
---

# An object with static storage duration is reached through its address

## Context and Problem Statement

`Place` is a local and a list of steps, and `TranslationUnit` holds functions
and types, so a global, a `static` local or a string literal cannot be named
in the IR. #85 proposed a second kind of place root, `Root::Static`, which
every reader of a place would answer for. `place.local` is read at several
hundred sites, most of them in the memory check, and each would have to say
what a static object means to it before anything else in #615 could land.

## Decision Drivers

* Phase 11's rule: the memory check reads a construct or concludes `Unknown`
  about it, and never calls it safe unread.
* Globals are the most common construct #615 adds, and #615 lands breadth
  first, by the owner's decision on it.
* What the memory check does with storage it cannot follow is measured: a
  write through a pointer of unknown origin and a read back through another
  are doubted, `SC0401` and `SC0402` "a pointer this check stopped
  following", not silent.

## Considered Options

* A new rvalue, the address of a static object: a pointer that is not null
  and names storage this function does not own. `g = 1` lowers to
  `t = &g; *t = 1`.
* A second kind of place root, as #85 proposed.

## Decision Outcome

Chosen option: "a new rvalue", by the owner's decision on #615.

* `TranslationUnit` gains a table of statics, each with a name, a type, a
  linkage and an initial value, and `Rvalue::Static(StaticId)` is the
  address of one. Every reader of an `Rvalue` matches it with every arm, so
  it is `E0004` at each until it says what the address is.
* The memory check reads the address as a pointer into storage outside this
  function, as it reads a parameter's, so what it cannot follow is a doubt.
  The nullability check reads it as not null, as it reads `&x`.
* #85's root is not built. A static object read precisely, which a later
  phase may want, is a second record that supersedes this one.

### Confirmation

A new reader of an `Rvalue` is `E0004` until it answers `Rvalue::Static`.
`a_pointer_kept_in_a_global_and_freed_is_reported` and
`a_pointer_kept_in_a_global_may_be_freed_by_any_call` change when a use of
a static is lowered as the address of a fresh local, which is the check
following the address to something it owns; `a_global_read_is_not_null`
reports `SC0403` when the address is not believed non-null.
`a_static_and_a_local_of_one_type_are_distinct` in `interp.rs`, and
`a_global_keeps_its_value_between_calls` and
`a_static_local_keeps_its_value_between_calls` in
`crates/safec-c/tests/interp.rs`, fail when a static is not one object for
the whole run.

### Consequences

* Good, because globals, `static` locals and string literals land without
  changing how any analysis reads a place.
* Bad, because every use of a pointer kept in a global is a doubt: the check
  does not know two reads of one global read one value.
* What would reverse this: an analysis that has to tell one static object
  from another, which the thread axis will.

## More Information

#85, #615, ADR-0019 for what a write through a pointer the check cannot
follow does.
