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
`x86_64` targets, where LLVM lowers it to the register save area; whether
that lowering reads what `clang`'s does is measured on CI's Linux and
macOS runners, which run `tests/run`. `aarch64-unknown-linux-gnu` stays
refused, as ADR-0126 decided.

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

### Consequences

* Good, because each target reads what its callers wrote, as `clang` does.
* Bad, because the System V `x86_64` read rests on LLVM's lowering, which is
  checked by running it rather than by a line in this compiler.
* What would reverse this: a target where LLVM's instruction and `clang`'s
  expansion disagree, measured.

## More Information

#617, #1214; ADR-0126.
