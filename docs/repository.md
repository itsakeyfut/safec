# Repository Layout and Dependencies

## Library Strategy

Keep dependencies deliberately small.

Possible initial dependencies:

```toml
[dependencies]

clap
ariadne
thiserror
inkwell
```

Potential later additions:

```text
indexmap
smallvec
bitflags
la-arena
```

These are candidates, not mandatory dependencies.

The principle is:

> Minimize accidental complexity, not necessarily dependency count.

A small number of well-understood dependencies is preferable to reimplementing everything solely for the sake of fewer crates.

## Initial Repository Structure

A Cargo workspace is appropriate once component boundaries become stable.

Target structure:

```text
safe-c/
│
├── Cargo.toml
├── README.md
├── LICENSE
├── CONTRIBUTING.md
│
├── crates/
│   ├── safec/
│   │   └── src/
│   │       └── main.rs
│   │
│   ├── lexer/
│   ├── parser/
│   ├── ast/
│   ├── sema/
│   ├── types/
│   ├── safety-ir/
│   ├── safety-analysis/
│   ├── codegen/
│   ├── codegen-llvm/
│   ├── runtime/
│   └── diagnostics/
│
├── tests/
│   ├── lexer/
│   ├── parser/
│   ├── sema/
│   ├── safety/
│   └── codegen/
│
├── examples/
│   ├── hello.c
│   ├── ownership.c
│   ├── lifetime.c
│   └── threads.c
│
└── docs/
    ├── architecture/
    ├── safety-model/
    └── language/
```

However, **do not split everything into separate crates immediately**.

The initial implementation may be better as:

```text
crates/
└── safec/
    └── src/
        ├── lexer.rs
        ├── parser.rs
        ├── ast.rs
        ├── types.rs
        ├── sema.rs
        ├── safety.rs
        ├── ir.rs
        └── codegen.rs
```

Split crates only when boundaries become clear.

This keeps the initial codebase small and easy to navigate.
