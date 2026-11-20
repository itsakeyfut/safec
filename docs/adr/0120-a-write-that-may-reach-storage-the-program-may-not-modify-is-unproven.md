---
status: "accepted"
date: 2026-11-21
decision-makers: itsakeyfut
---

# A write that may reach storage the program may not modify is unproven

## Context and Problem Statement

C17 6.7.3 p6 makes a write to an object defined `const` undefined, and 6.4.5
p7 a write to a string literal. ADR-0117 answered the first by reporting a
cast that drops `const` from what a pointer points at, `SC0416`. The hunt
before #615 closed showed that rule asks one spelling of many: the same
pointer is reached through `&a` of a `const` array, one level down
(`*(int **)&c`), through `void *`, through `memcpy` of its bytes, and as what
`memchr` returns, each silent (#1121). A string literal reaches `char *` with
no conversion at all. And the rule itself made `free((void *)p)` of a `const
int *` a pointer the check stops following, hiding a use after it (#1122).

## Decision Drivers

* Phase 11's rule: a construct the check cannot read is `Unknown`, never safe.
* A rule asked of a spelling is a rule a new spelling walks around; the
  object, not the cast, is what may not be written.

## Considered Options

* Follow the storage: mark what may name read-only storage, and ask every
  write that may land there.
* Keep asking the cast, and add each spelling the hunt found.
* Leave writes to read-only storage outside the memory check's claim, and
  say so in `docs/safety-model.md`.

## Decision Outcome

Chosen option: "follow the storage", by the owner's decisions on #615's
second design run, including `const` locals, because it answers every
spelling with one rule where the second option answers each with its own.

* An object is **read-only** where its type is `const` at its innermost
  element: a `const` static, a `const` local, and a string literal.
  `ir::Static` and `ir::Function` say which.
* A pointer that may name a read-only static carries a mark that says so,
  joined by union and carried through every door a mark is (memory, library
  copies, library results into their argument), each door answering for it
  by `error[E0027]`. A pointer to a read-only local names the local.
* A write that may land in read-only storage is reported `Unknown`, `SC0417`,
  where it is written: a store, a library write's destination, and a call that
  hands such a pointer to a parameter whose pointee is not `const`, since the
  callee may write through it. **And a `return` of one from a function whose
  result is no pointer to `const`**, since the caller may write through what
  it is given; found by the review of #1121. `free` and `realloc` are left to
  the free rules.
* ADR-0117's rule that a cast dropping `const` is `Rvalue::Reinterpret` is
  superseded by this, and #1122 removes it: such a cast is then followed as
  `Rvalue::Use`, as any other pointer cast is, so a free through it is
  followed. Until #1122 lands both are said at such a cast.

The second option fails where a spelling nobody listed is written: row 6.
The third fails nowhere it claims, and leaves every write to a literal
believed, which is a named silence rather than a fixed one.

### Confirmation

`a_write_that_may_reach_a_const_object_is_unproven` holds a function per
spelling #1121 found, each `SC0417`, and goes silent where the mark is not set
at `Rvalue::Static`, where a load does not take it, where a read-only local is
not asked, and where a call is not; `a_write_to_a_string_literal_is_unproven`
goes silent where a literal is not read-only; and
`a_const_object_read_and_a_writable_one_written_are_not_reported` fails where
every static is. ADR-0117's rule for a cast that drops `const` is removed by
#1122, whose case `free((void *)p); return *p;` is then `SC0402`.

### Consequences

* Good, because the rule is about the object, so a new spelling of the
  pointer is asked by the doors it already passes.
* Bad, because a literal handed to a parameter declared without `const`,
  `void w(char *); w("ab");`, is doubted though the callee may only read it.
* Bad, because the mark is one more every door carries.
* Bad, because a variadic call, once `...` is read, hands every argument past
  the last parameter to no pointer to `const`, so `printf("%s", "x")` will be
  doubted unless the library is told which variadic arguments it only reads.

## More Information

ADR-0115, ADR-0117, #615, #1121, #1122.
