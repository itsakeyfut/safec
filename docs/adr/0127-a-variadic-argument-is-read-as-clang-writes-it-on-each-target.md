---
status: "accepted"
date: 2026-11-25
decision-makers: itsakeyfut
---

# A variadic argument is read as `clang` writes it on each target

## Context and Problem Statement

ADR-0126 refused `va_arg` on `aarch64-unknown-linux-gnu` and wrote LLVM's
`va_arg` instruction everywhere else, on the driver that the instruction
"is right on the other five targets". Implementing #1214 showed that driver
false: on `x86_64-pc-windows-msvc`, `int x = va_arg(a, int); int y =
va_arg(a, int);` in `first(2, 7, 5)` read `y` as 0, where two `long long`s
read right. Every argument there fills an 8-byte slot, and the instruction
moved past an `int` by 4. `clang 22.1.8 -S -emit-llvm -O0` writes the
instruction on `aarch64-apple-darwin` alone; on Windows, `i686`, `armv7`
and `wasm32` it reads the list's pointer and moves it itself, and on the
System V `x86_64` targets it writes the register save area out.

## Decision Drivers

* a variadic argument read wrong is a miscompile, which nothing reports
* where `clang` writes the read itself, it is because the instruction is
  not the target's convention
* the hosts this project builds and runs on include Windows

## Considered Options

* Walk the list's pointer where the list is one, Windows, `i686`, `armv7`
  and `wasm32`, as `clang` does; keep the instruction on
  `aarch64-apple-darwin` and the System V `x86_64` targets; refuse
  `aarch64-unknown-linux-gnu` as before.
* Write every target's read out, the register save areas of System V
  `x86_64` and AAPCS64 too.
* Refuse `va_arg` everywhere but `aarch64-apple-darwin`.

## Decision Outcome

**The list's pointer is walked where the list is one**: read it, load the
value where it points, and move it past as many pointer-wide slots as the
value fills, an 8-byte value first aligned to 8 on `armv7` and `wasm32`,
which is what `clang` writes there. The instruction stays on
`aarch64-apple-darwin`, where `clang` writes it too, and on the System V
`x86_64` targets, where LLVM lowers it to the register save area, **but
only out of a list that is the function's own plain local**: read through a
pointer to a list, a member or a static, that lowering reloads the list's
address at `-O0` from a stack slot it never wrote, and the program crashed
(measured under WSL, exit 139, where `clang`'s expansion ran). That read is
refused with `SC0801` until the expansion is written (#1333).
`aarch64-unknown-linux-gnu` stays refused, as ADR-0126 decided.

**What the memory check reads of a list** is whether it was started: a
read before `__builtin_va_start` is `SC0411`, as any read of what nothing
wrote. Whether it was ended, and how many arguments the caller passed, are
not read (#1334): the second needs the caller, which ADR-0126 says this
check does not read.

Writing every target out was rejected for this issue: the register save
areas are the larger part of the work, and the one target where the
instruction is known wrong is already refused. Refusing all but one target
would leave the Windows host unable to run a variadic definition at all.

This replaces ADR-0126's driver that LLVM's `va_arg` is right off
`aarch64-unknown-linux-gnu`, and nothing else of that record.

### Confirmation

* Writing LLVM's `va_arg` on Windows rather than walking the pointer:
  `a_variadic_function_reads_its_arguments` in `tests/run` runs to the wrong
  result there, and `a_variadic_argument_is_read_as_each_target_writes_it`
  in `tests/cases` loses its walk.
* Aligning nothing on `armv7`:
  `a_variadic_long_long_is_aligned_to_eight_on_armv7` loses its `and`.
* Writing LLVM's `va_arg` through a list that is not a plain local on
  System V `x86_64`:
  `a_variadic_argument_is_read_only_out_of_a_local_list_on_x86_64_linux`
  builds.
* Answering nothing for a list `__builtin_va_start` writes:
  `a_list_read_before_it_was_started_is_reported`'s `started` reports.

### Consequences

* Good, because each target reads what its callers wrote, as `clang` does.
* Bad, because the System V `x86_64` read rests on LLVM's lowering, which is
  checked by running it rather than by a line in this compiler.
* What would reverse this: a target where LLVM's instruction and `clang`'s
  expansion disagree, measured.

## More Information

#617, #1214, #1333, #1334; ADR-0126.
