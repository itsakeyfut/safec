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

## When a file becomes a directory

Inside a crate the question is not a boundary cargo enforces but whether a file
can still be read, and four rules answer it.

**A file splits when it holds two or more concerns that can be read apart, and
its code, tests excluded, is over about 2,000 lines.** The number is when to
look; the concerns decide. A file of one concern stays whole however long:
`crates/safec/src/lowering.rs` is almost all one `impl Lowering`, and cut by
size its pieces would not read apart. A small file of two concerns stays whole
too, because every file a reader has to hold costs them more than a section
does.

**`foo.rs` stays the module root, and the parts go in `foo/`.** The root keeps
the public surface and the `mod` declarations, so a reference to the module's
public API by its file still names the right one. This is the shape
`crates/safec/src/diagnostics.rs` and `diagnostics/render.rs` already have, and
no `mod.rs` is used.

**Inline tests over about 500 lines move to `foo/tests.rs`**, declared
`#[cfg(test)] mod tests;`, whether or not the code splits. The code a reader is
in stops sharing a file with them, and none of its lines move.

**A split is a pure move, and lands with every reference.** No behaviour
changes and nothing is renamed on the way, so the diff is moved lines and the
suite and every blessed corpus output say so. Every path in `docs/` and in code
comments that names a moved item is updated in the same change: the records
name files by path, and nothing checks prose for one.

What the rules say of the files they apply to today, measured on `main` at
`d3406e7`. Each split updates its row when it lands.

| file | code, tests excluded | concerns | inline tests | the rules say |
|---|---|---|---|---|
| `crates/safec-ir/src/memory.rs` | 5,497 | five | 0 | split |
| `crates/safec/src/driver.rs` | 2,466 | four | 1,509 | split, and its tests move |
| `crates/safec/src/lowering.rs` | 2,250 | one | 1,464 | its tests move |
| `crates/safec/src/parser.rs` | 1,888 | one | 948 | its tests move |
| `crates/safec/src/types.rs` | 1,617 | one | 1,476 | its tests move |
| `crates/safec-ir/src/ir.rs` | 1,279 | one | 635 | its tests move |
| `crates/safec/src/ast.rs` | 1,084 | one | 552 | its tests move |
| `crates/safec-llvm/src/emit.rs` | 947 | one | 619 | its tests move |
| `crates/safec/src/diagnostics.rs` | 678 | one | 521 | its tests move |
| `crates/safec/src/diagnostics/render.rs` | 574 | one | 690 | its tests move |
