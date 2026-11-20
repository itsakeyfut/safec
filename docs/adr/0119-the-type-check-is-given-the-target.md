---
status: "accepted"
date: 2026-11-21
decision-makers: itsakeyfut
---

# The type check is given the target, replacing what ADR-0013 says it is handed

## Context and Problem Statement

ADR-0013's section *Only what something reads* says that `types::check` is
handed one `Integer`, the range of `int`, "and not a `Target`", because a
constant's type was the only question it asked of the machine. #1112 reads
`short`, `long`, `long long` and `unsigned`, and the type check now asks the
machine three things: which type of C17 6.4.4.1 p5's list holds a constant,
whether a signed type holds every value of an unsigned one in 6.3.1.8, which
is where `long` and `unsigned int` differ between a 64-bit and a 32-bit
`long`, and what a `char` constant's value is. #1113's `sizeof` will ask a
pointer's width too.

## Decision Drivers

* Every width the type check asks for is the target's to answer (ADR-0013).
* An argument per width is a signature that changes with every type added,
  and each change touches every caller.

## Considered Options

* Hand `check` the `Target`.
* Hand it a table of every integer type's range, keeping "only what it
  reads", and add an argument when `sizeof` asks for a pointer's width.

## Decision Outcome

Chosen option: "hand `check` the `Target`", by the owner's decision on #1112,
because what the type check reads of the machine is now most of what the
target says, and the table would be the target again under another name.

* `types::check` takes a `Target` and asks `Target::integer(rank, sign)`. It
  still builds no type from a width: an integer type in the tree is a rank and
  a sign (ADR-0116), so the cost ADR-0013 priced against *`Ty` carries the
  width* is still not paid.
* ADR-0013's sentence that `check` is handed one `Integer` and not a `Target`
  is replaced by this. The rest of ADR-0013 stands.

### Confirmation

`a_constant_takes_the_first_type_that_holds_it` in `types/tests.rs` fails
when a constant's type is taken from its list without asking the target
which type holds it, and the interpreter test
`a_long_compared_with_an_unsigned_int_depends_on_the_target` answers
differently on `x86_64-unknown-linux-gnu` and `x86_64-pc-windows-msvc`, and
fails if the usual arithmetic conversions do not ask the target's widths.

### Consequences

* Good, because a type added later needs no new argument.
* Bad, because the type check can now read anything the target says, and
  nothing but review keeps it to widths.

## More Information

ADR-0013, ADR-0116, #1112.
