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

One criterion for clear: **the boundary that must not be crossed is the boundary
worth making a crate.** The dependency arrows of a workspace are enforced by
cargo, so a safety-IR crate that does not depend on the frontend cannot grow a
dependency on it by accident, and a reversal is a build failure rather than a
review comment. That is the same shape as the guards in
[ADR-0003](adr/0003-pass-the-source-map-to-each-render-call.md) and
[ADR-0004](adr/0004-resolve-the-strictest-level-where-the-policy-is-built.md),
which are held by the compiler rather than by remembering to run a test.

Everything else can stay in one crate until it hurts.

This keeps the initial codebase small and easy to navigate.

## What the split actually is, now that one has happened

That criterion has fired twice, and the workspace is three crates:

```text
crates/
├── safec/        the lexer, the parser, sema, the types, the lowering, the
│                 diagnostics, the CLI and the driver
├── safec-ir/     the Safety IR, its printer, its interpreter, and the source
│                 map a span is an offset into
└── safec-llvm/   the LLVM backend, which writes textual LLVM IR
```

The arrows are `safec -> safec-ir`, `safec -> safec-llvm` and
`safec-llvm -> safec-ir`, and nothing points the other way.
[ADR-0011](adr/0011-the-ir-crate-depends-on-nothing-in-the-workspace.md) has the
reasoning for the first and the options it rejected, including why `source` is
on the IR side rather than the frontend's.
[ADR-0014](adr/0014-write-llvm-ir-as-text-from-a-crate-of-its-own.md) has the
second: `docs/architecture.md` asks that LLVM not leak into the frontend and the
analyses, and a backend that cannot see a token is that sentence held by cargo.
One crate per backend rather than one `codegen` crate, because a WASM backend
sharing a crate with an LLVM one shares its dependencies the day one arrives.

Nothing else has been split, and the paragraphs above still hold for the rest:
the lexer, the parser and sema are boundaries nothing is pushing on, so
enforcing them would cost the crates and buy nothing.
