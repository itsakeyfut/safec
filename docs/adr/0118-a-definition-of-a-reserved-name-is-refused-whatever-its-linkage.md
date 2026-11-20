---
status: "accepted"
date: 2026-11-20
decision-makers: itsakeyfut
---

# A definition of a reserved name is refused whatever its linkage, replacing what ADR-0051 says of a `static` one

## Context and Problem Statement

ADR-0051 refuses a definition of a name this compiler believes by its name,
`abort`, `exit` and the functions the memory check reads, as `SC0319`, and
says of the day `static` is read: "a definition with internal linkage is not
one 7.1.3 p1 reserves, so the refusal has to ask linkage, and
`Lowering::does_not_return` with it". #85 reads `static`, and the sentence is
half of C17 7.1.3 p1. The other half reserves "each identifier with file
scope listed in any of the following subclauses ... as an identifier with
file scope in the same name space if any of its associated headers is
included". A `static int free(int x)` is C where `<stdlib.h>` is not
included and undefined where it is.

## Decision Drivers

* This compiler does not read `#include <...>` and does not ask which
  header a unit includes, so it cannot tell the two halves apart.
* A `static exit` that returns, believed not to return, is a function body
  the checks never reach: the worst answer this compiler can give.
* The memory check reads `free` and `memcpy` by name, and a `static` one
  that does something else would be read as the library's.

## Considered Options

* Refuse a definition of a reserved name whatever its linkage.
* Ask linkage, as ADR-0051 said: accept a `static` one, and make every
  reader of a name by its spelling, the lowering's `does_not_return` and the
  memory check's library table, ask linkage too.

## Decision Outcome

Chosen option: "refuse whatever its linkage", by the owner's decision in
review of #85, because it cannot believe a `static exit` that returns, and
the other option would have every reader of a name by its spelling ask
linkage before this compiler reads a header.

* `Resolver::define` refuses a `static` definition as it refuses one with
  external linkage, under `SC0319`, and its note says what is true of a
  `static` one: 7.1.3 p1 reserves it wherever its header is included, which
  this compiler does not ask.
* ADR-0051's bullet on `static` is replaced by this. The rest of ADR-0051
  stands.

### Confirmation

`a_static_function_named_exit_is_refused` in `tests/cases/frontend` builds
when `Resolver::define` is not called for a `static` definition.

### Consequences

* Good, because no definition can make a name's belief false, whatever its
  linkage.
* Bad, because a program that defines a `static free` and includes no
  header is valid C and refused; the fix is a rename.
* What would reverse this: reading `#include <...>`, which would let the
  refusal ask whether the header is there.

## More Information

ADR-0051, #85, #343.
