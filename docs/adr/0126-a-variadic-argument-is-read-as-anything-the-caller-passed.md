---
status: "accepted (its driver that LLVM's `va_arg` is right off `aarch64-unknown-linux-gnu` superseded by ADR-0127)"
date: 2026-11-24
decision-makers: itsakeyfut
---

# A variadic argument is read as anything the caller passed

## Context and Problem Statement

Phase 11 compiles variadic functions (#617): calling one (#1213) and defining
one with `va_list`, `va_start`, `va_arg`, `va_end` and `va_copy` (#1214).
Three questions decide the second: what the memory check records for a value
`va_arg` reads, how `va_arg` is written on each target, and where a
definition gets its `va_list` when no header is read yet.

## Decision Drivers

* Phase 11's rule, which #617 states for this case: a variadic argument the
  check cannot type is `Unknown`, never safe
* `va_arg` with a type the caller did not pass is undefined (C17 7.16.1.1
  p2), and the check reads one function at a time, so it cannot see the
  caller
* LLVM's `va_arg` instruction reads the wrong value on
  `aarch64-unknown-linux-gnu`: it reads the caller's stack and not the
  register save area, measured with `clang` 22.1.8 on 2026-11-24. It is right
  on the other five targets
* `#include <...>` is not read yet (#1223)

## Considered Options

For a pointer `va_arg` reads: what the caller may have stored, as a value read
through a pointer parameter is (`Why::FromCaller`); or a reason of its own,
doubted at every use.

For `aarch64-unknown-linux-gnu`: refuse `va_arg` there; or write the AAPCS64
expansion in the backend.

For `<stdarg.h>`: the builtins `__builtin_va_list`, `__builtin_va_start`,
`__builtin_va_arg`, `__builtin_va_end` and `__builtin_va_copy` now, with the
header a set of macros over them later; or wait for the header.

## Decision Outcome

**A pointer `va_arg` reads names `Referent::Unknown(Why::Variadic)`**,
doubted at every use as a pointer built from bytes is (`Why::Forged`).
`FromCaller` was rejected because it is believed where it is dereferenced,
and a `va_arg` of `int *` where the caller passed an `int` is a pointer to
nowhere read through in silence. The cost is a doubt at every dereference of
a pointer a variadic function reads, which is a report the reader can see.

**`va_arg` is refused on `aarch64-unknown-linux-gnu`** by the backend, with
`SC0801`, rather than written as an instruction that reads the wrong value.
The expansion is its own issue, because it can be run only on that target.

**The builtins come first**, so that defining a variadic function does not
wait for `#include <...>`; `<stdarg.h>`, when headers are read, defines its
names as the builtins.

### Confirmation

Each mutation names the case in `tests/cases` that fails under it:

* a pointer `va_arg` read named nothing in `transfer.rs`:
  `a_pointer_va_arg_reads_is_doubted_wherever_it_is_used`'s `dereferenced`
  goes silent;
* `Why::Variadic` read as `from_caller` in `parts.rs`: the same case's
  `before_the_end`, read before `__builtin_va_end`, goes silent;
* the target refusal skipped:
  `a_variadic_argument_is_not_read_on_aarch64_linux_yet` builds.

`Rvalue::VaArg`, `Called::Intrinsic` and `Why::Variadic` are each
`error[E0004]` at every reader.

### Consequences

* Good, because the one fact the check cannot see, what the caller passed,
  is doubted rather than believed.
* Bad, because a variadic function that only forwards its pointers is doubted
  wherever it reads through them.
* What would reverse this: the check reading every caller in the translation
  unit and finding the types each passes.

## More Information

#617, #1213, #1214, #1223; ADR-0125.
