# Safe C

**Small, Hackable, Safety-Oriented Compiler**

A small experimental C compiler for memory, lifetime, ownership, and thread safety.

## Goal

Safe C explores a single question:

> How much of Rust's memory, lifetime, ownership, and thread safety can be
> brought into the C ecosystem while preserving C's familiarity,
> interoperability, and incremental migration path?

This is not a Rust clone, and not just another tiny C compiler.
It aims to be a small, understandable, hackable reference implementation for experimenting with safety mechanisms in the C language family.

Smallness is a means, not the goal: the compiler stays small enough that one developer can hold its major components in their head, which is what makes experimentation practocal.

## Design principles

- **Safety IR is the central abstraction.** The AST is never lowered directly to LLVM IR. Safety analysis operates on a dedicated semantic IR that can express ownership, lifetimes, borrowing, regions, and thread relationships.
- **Inference is separated from enforcement** Results are `Safe`, `Unsafe`, or `Unknown`. The analyzer does not pretend every C program can be proven safe.
- **Gradual adoption.** Existing C code can enter the system without a rewrite.
- **Hackability over abstraction purity.** The architecture optimizes for making experiments easy.

## Architecture

```text
C source
   |
   v
Lexer -> Parser -> AST -> Semantic analysis -> Typed AST
   |
   v
Safety IR <-- central boundary
   |
   +--> Safety analysis (memory / lifetime / ownership / thread)
   |
   +--> Backend (LLVM IR, interpreter)
```

Longer term, a Clang adapter feeds the same Safety IR, so the safety model can be validated against real-world C and C++ code.

## Build

```sh
cargo build
cargo run -p safec
```

Requires a Rust toolchain with edition 2024 support (1.85 or later).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICESE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.