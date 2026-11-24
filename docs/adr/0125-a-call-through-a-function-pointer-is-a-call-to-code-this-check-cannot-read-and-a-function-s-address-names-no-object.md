---
status: "accepted"
date: 2026-11-24
decision-makers: itsakeyfut
---

# A call through a function pointer is a call to code this check cannot read, and a function's address names no object

## Context and Problem Statement

Phase 11 compiles function pointers (#617, #1212): storing one, calling
through one, comparing two, and holding one in an array or a member. The IR
has no function value. `Terminator::Call` names its callee by `FuncId`, no
`Ty` is a function, and no operand is a function's address, so the lowering
refuses every one of these with `SC0304`. Three questions decide every
sub-issue under #617, so they are settled once: how the IR names what a call
calls, what the memory check records for a function's address, and what
becomes of ADR-0037's `_Nonnull` parameter once a call can reach a function
without naming it.

## Decision Drivers

* Phase 11's rule: the memory check reads each construct or concludes
  `Unknown` about it, never safe without reading it
* a call this check cannot read is already answered soundly: it may write,
  keep or free anything it is handed (ADR-0066), which is what a call
  through an unknown pointer can do
* ADR-0049: no construct is refused at levels 1 to 4 for how it is written
* ADR-0037 names this day as the one that reverses it: "the annotation has to
  become part of the pointer's type or the call has to be refused"

## Considered Options

For the call: the callee as an enum, `Direct(FuncId)` or `Through(Operand)`,
in the one `Call` terminator; or a second terminator for a call through a
pointer.

For a function's address in the memory check: a new reason a value names
what the check does not model, `Why::Function`; or a value naming nothing,
the shape `int *z = 0;` already has.

For a `_Nonnull` parameter: `Unknown` where the function's address is taken;
the annotation carried in the function pointer's type; or the address
refused.

## Decision Outcome

**The callee is an enum in the one `Call` terminator**, `Called::Direct` or
`Called::Through`, because what a call does with its arguments, its result
and its continuation is one body whichever way the callee is named. A second
terminator would split that body in two, and a rule changed in one copy and
not the other is a call through a pointer answered safe. Every reader of a
callee answers both arms, which the compiler holds. The memory check reads
`Through` as `Callee::Opaque`; the lowering never writes `Through` for a
function it can name.

**A function's address names `Referent::Unknown(Why::Function)`.** A
function is not an object, so a read or a write through its address as data
is undefined, and a value naming an unknown referent is doubted where it is
used so. The empty set was rejected because it is what a null pointer holds,
and any reader that takes "names nothing" as "reaches nothing" would let a
function's address be read through in silence. Every reader of a reason
destructures every reason, so a reader that has not answered `Function`
does not compile.

**A `_Nonnull` parameter's function has its address taken under `Unknown`**,
reported by the nullability check where the address is taken, because a
call through the pointer cannot be checked against an annotation the
pointer's type does not carry, while the body goes on believing it. This
replaces ADR-0037's sentence on a call through a function pointer, which it
named as what would reverse it, and nothing else of that record. Carrying the
annotation in the type was rejected for now: a conversion that drops it is a
call answered safe, and C gives the conversion no constraint to stop it.

**And two rules follow from the first three**, without a choice between
options:

* a call through a pointer is a dereference of it: the nullability check asks
  it as it asks `*p`, and the assigned check reads the callee operand as a
  read, so a call through a pointer nothing assigned is `SC0411`;
* a conversion from one function pointer type to another, or between a
  function pointer and any other pointer or an integer, concludes `Unknown`
  where it is written, because a call through a pointer to a function of
  another type is undefined (C17 6.3.2.3 p8) and the check does not follow
  which function a pointer holds.

### Confirmation

Each mutation names the function of
`a_call_through_a_function_pointer_is_read_as_a_call_this_check_cannot_read`
in `tests/cases/memory/` that it fails:

* `Allocations::callee` answering a call through a pointer as
  `Callee::Reads`: `freed_through_a_pointer` goes silent;
* `assigned::terminator_reads` reading the arguments alone:
  `through_an_unassigned_pointer` loses its `SC0411`;
* the nullability report skipping `called_through`:
  `through_a_pointer_that_may_be_null` goes silent;
* the `_Nonnull` pass skipped: `nonnull_address_taken` goes silent;
* `converted_function` skipped: `converted` goes silent.

The program `a_function_pointer_is_stored_called_and_compared` in
`tests/run/` builds and runs each spelling. `Called` gaining a variant is
`error[E0004]` at every reader of a callee, and `Why` gaining a reason is
`error[E0004]` in `Held::beyond` and `carried_through_an_address`.

### Consequences

* Good, because a call through a pointer inherits every rule an opaque call
  already has, and a later change to those rules reaches both.
* Bad, because a callback is never read: a `free` called through a pointer
  is a possible free, and what any call through a pointer is handed is
  doubted after it.
* Bad, because a `_Nonnull` function used as a callback is `Unknown` where
  its address is taken, at every level that runs nullability.
* What would reverse this: the check following which functions a pointer may
  hold, so that a call through one it can name is read as the call it is.

## More Information

#617, #1212, #1213; ADR-0037, ADR-0049, ADR-0066.
