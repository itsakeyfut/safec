---
status: "accepted"
date: 2026-10-24
decision-makers: itsakeyfut
---

# The `safec` crate is a facade at the workspace root, over parts that each stand alone

## Context and Problem Statement

The workspace was three crates under a virtual root. The one named `safec` held
the lexer, the parser, sema, the types, the lowering, the diagnostics, the CLI
and the driver, so the name a user would reach for was taken by the part that
reads C. Nothing presented the compiler as one crate, and a root-level `tests/`
or `benches/` belonged to no package.

The project means to read C++ as well, through a second frontend. On the day
that frontend arrives, a crate called `safec` that is also the C frontend names
one language as the compiler, and every path that names it has to move. On
`main` at `c1bf5f81`, `docs/` alone names `crates/safec/` 167 times, and the
number only grows, so the move is cheapest now.

## Decision Drivers

* Each part stays usable on its own. A user who wants only the Safety IR
  depends on `safec-ir` and gets nothing above it, which is what ADR-0011 and
  `docs/roadmap.md` already assume for the second frontend.
* `docs/repository.md`: split a crate only where a boundary must not be
  crossed.
* `env!("CARGO_BIN_EXE_safec")` is set only for the targets of the package that
  defines the binary, and a `cargo::rustc-env` from one package's build script
  reaches no other package. Measured with a two-package workspace: the binary
  path resolved from the root package's test, and the member's build-script
  variable did not.
* A path left behind by the move must fail a check, not keep pointing at a
  directory whose contents changed.

## Considered Options

Where the facade lives:

* The root manifest becomes the `safec` package as well as the workspace.
* `crates/safec/` becomes the facade, with the root staying virtual.

What the crate that reads C is called:

* `safec-c`
* `safec-frontend`
* `safec-cc`

Where the binary lives:

* In the facade.
* In a crate of its own, such as `safec-cli`.

How the facade presents the parts:

* One module per part: `safec::c`, `safec::ir`, `safec::llvm`.
* The C part's modules at the root, beside `safec::ir` and `safec::llvm`.

And whether the re-exports sit behind Cargo features.

## Decision Outcome

**The root manifest is the `safec` package**, and the workspace beside it:

```text
Cargo.toml          [workspace] and [package] safec
src/lib.rs          pub use safec_c as c; safec_ir as ir; safec_llvm as llvm
src/main.rs         the binary
tests/              every test that runs the binary
benches/growth.rs
crates/
├── safec-c/        everything that reads C, from the preprocessor to the driver
├── safec-ir/       unchanged
└── safec-llvm/     unchanged
```

**Root rather than `crates/safec/`**, because the old directory then ceases to
exist. Every pointer the move misses is a dead link, found by anyone who
follows it and by a search for the old path. Reusing `crates/safec/` for the
facade would leave some of the old paths alive, such as `crates/safec/tests/`,
with different contents behind them, and a stale pointer that still resolves
is found by neither.

**`safec-c`**, because it names the language the part reads, which is what
`safec-ir` and `safec-llvm` are named for, and a C++ part is then `safec-cpp`
beside it. The driver and the CLI are in it today, and parts of the driver,
such as running the analyses and wording their findings, are not specific to
C. When the second frontend needs them they leave for a crate both frontends
share, and what remains is the C frontend, still correctly named.
`safec-frontend` would be the name that has to change on that day, since there
would be two. `safec-cc` describes today's contents most exactly and the
remainder after that split least. Nothing else is split now: the crate moves
whole, which is all `docs/repository.md` allows without a boundary to defend.

**The binary is in the facade**, because the tests that run it have to be in
the binary's package, and those are the tests of the whole compiler, which
belong to the crate a user installs. A `safec-cli` crate would defend no
boundary.

**One module per part**, because no language is the root of a compiler meant to
read two. The cost is a stutter, `safec::ir::ir::TranslationUnit`, which a
`use` absorbs.

**No features.** The binary uses every part, so there is nothing a feature
could leave out of the build that matters, and a user who wants one part
depends on that part. The facade adds nothing a part needs: a type a user of
`safec-ir` alone would need lives in `safec-ir`.

### Confirmation

`a_program_reaches_every_part_through_the_facade_alone` in `tests/facade.rs`
compiles C through `safec::c`, prints the result through `safec::ir` and
writes it through `safec::llvm`, naming no part's own crate. Deleting any one
of the three `pub use` lines in `src/lib.rs` is `E0432` or `E0433` at that
file's imports through it, depending on how far each path gets, and the test
target does not build.

`a_function_built_by_hand_is_printed_with_nothing_above_this_crate` in
`crates/safec-ir/tests/print.rs` builds a function from `safec-ir`'s public API
and prints it. Making `print::dump_ir` `pub(crate)` is `E0603` there. That the
crate has nothing above it is still ADR-0011's guard, now spelled
`safec-c = { path = "../safec-c" }` or `safec = { path = "../.." }` in
`crates/safec-ir/Cargo.toml`, both of which cargo refuses as a cyclic package
dependency.

Moving the binary out of the root package leaves `env!("CARGO_BIN_EXE_safec")`
undefined in every process-level test under `tests/`, which is a compile error
rather than a failing assertion.

What nothing checks is prose. A sentence calling `safec-c` "the `safec` crate"
compiles and passes every script, and it is held by review alone.

### Consequences

* Good, because the name a user reaches for is the whole compiler, and a
  second frontend arrives as a sibling rather than as a rename.
* Good, because each part's tests still build with nothing above it in the
  graph, so the position each crate would be published in is already the one
  it is tested in.
* Bad, because the repository root now holds a package's `src/`, `tests/` and
  `benches/` beside the workspace's `crates/`, and `cargo test` at the root
  without `--workspace` tests the facade alone.
* Bad, because `safec::ir::ir` stutters.
* What would reverse this: the first publish, if the project's name changes
  then. The facade's name is the one to change, and the parts' names stay.

## Pros and Cons of the Options

### The root as the facade package

* Good, because `crates/safec/` disappears and every missed pointer is dead
  rather than wrong.
* Good, because `cargo run` and `cargo install --path .` reach the compiler
  with no `-p`.
* Bad, because the root carries a package's targets as well as the workspace.

### `crates/safec/` as the facade

* Good, because the root stays a virtual manifest.
* Bad, because pointers the move misses can still resolve, to the wrong thing.

### `safec-frontend`

* Good, because it is what the documents call the part.
* Bad, because the driver and the CLI are not a frontend, and a second frontend
  makes the name ambiguous.

### `safec-cc`

* Good, because it describes a crate holding a driver and a CLI exactly.
* Bad, because it describes what remains after the driver is shared least
  well.

### The binary in a crate of its own

* Good, because the facade would be a library alone.
* Bad, because it is a crate with no boundary to defend, and every
  process-level test moves to it rather than to the crate a user installs.

### The C part's modules at the facade's root

* Good, because `safec::driver` keeps working and fewer tests change.
* Bad, because C becomes the compiler's root and C++ a module beside it, and
  the shape changes when `safec-cpp` arrives.

## More Information

[`docs/repository.md`](../repository.md) carries the layout this produces.
[ADR-0011](0011-the-ir-crate-depends-on-nothing-in-the-workspace.md) and
[ADR-0014](0014-write-llvm-ir-as-text-from-a-crate-of-its-own.md) are the
boundaries the parts keep. Issue #570 is where the questions were settled.
