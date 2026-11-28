---
status: "proposed"
date: 2026-11-28
decision-makers: itsakeyfut
---

# A static's initializer is checked as the stores of a function that runs before `main`

## Context and Problem Statement

Since #1196 a static can start holding addresses (`StaticInit::Address`, ADR-0137), and ADR-0137 calls such an initializer the store made before the program begins. The memory check asked it one thing a store is asked, whether the address names a read-only static where the pointer may write (`read_only_statics`). A store written as an assignment is also asked how wide what it points at is (`SC0413`) and whether read-only storage is reachable at any level below what leaves (`SC0417`, ADR-0121, ADR-0123). The hunt before #1196 closed found both silent through the initializer (#1430): `char **pp = (char **)&p;` with `p` a static holding a literal, and `int *ip = (int *)&c;` with `c` a `char`. The rule was written twice, and the second copy was the smaller.

## Decision Drivers

* A rule this check asks at a store has one implementation, so that a later change to it reaches every route a pointer leaves by.
* What leaves through a static's initializer is the same departure an assignment to the static makes, and C17 5.1.2 p1 makes every static's initialization happen before the program starts.
* The backend writes a static's starting value as a constant (ADR-0137); the check's question is about the program, not about how the value is written.

## Considered Options

* **Analysis-only functions, never emitted, each holding the store of one address in a static's initializer, which the memory check analyses as it does every function.**
* Mark each `StaticInit::Address` in the lowering with what the store route would conclude, the const levels it drops and whether what it names is narrower than its type, and report the marks.
* Teach `read_only_statics` the other questions, walking each static's initializer with its type.

## Decision Outcome

Chosen option: "analysis-only functions", because the store route's rules then answer the initializer as they answer an assignment, with no second copy to fall behind. The other two copy the width rule and the levels rule into a second place, which is the shape #1430 came from.

Each function is named by its initializer and spelled `.statics`, with a `.` so no C identifier holds it, one per store because no store reads what another wrote and one function holding them all made the check cubic in a table's length; marked so that the backend writes no code for it. Each store writes the static's own place, reached through its members and elements as an assignment's place is, so the levels a reader of that place keeps are the ones an assignment's would be. Writing a `const` static there is its initialization, not a write C forbids (`Origin::Initialized`), while what the store carries is asked as any store's is. `read_only_statics` and `StaticInit::Address`'s `writable` go: what they asked is the store route's now. The one question the function's stores are not asked is whether the place they write fits its object, `SC0412`: each is a static's own place, built from its type, and the check, which follows no static (ADR-0115), would otherwise doubt every member and element of every table of pointers.

### Confirmation

`a_static_initializer_is_asked_as_the_store_it_stands_for` holds each program #1430 names and goes silent when the lowering makes no such function, and is `SC0412` at `names` when its stores' places are asked whether they fit. `llvm_ir_of_a_static_aggregate_and_an_address` changes when the backend writes the function. `checking_a_static_table_grows_with_its_length_and_no_faster` fails when one function holds every store.

### Consequences

* Good, because a static's initializer is asked everything an assignment is, now and after the rules change.
* Good, because a union's readers, which #1432 makes the store route ask, reach a static union's initializer with no work of their own (#1431).
* Bad, because every corpus case that dumps the IR of a unit with an address in a static's initializer shows the function, and is blessed again once.
* Bad, because a doubt the store route raises wrongly is now raised at a static's initializer too.
* What would reverse this: the check following what a static holds into the functions that read it (ADR-0115), which would make each initializer a fact rather than a store.
