---
status: "accepted"
date: 2026-09-10
decision-makers: itsakeyfut
---

# The IR crate depends on nothing in the workspace, and the frontend depends on it

## Context and Problem Statement

`docs/c-family.md` rests everything it says about reaching C++ on one arrow: the
analyses must not know which frontend produced the IR. Stated as intent, that
arrow lasts until the first convenient place to reach through it, and there is
already one, because the IR, the lowering and the frontend are modules of a
single crate that can all see each other.

The IR, the lowering and the interpreter now exist, so where the boundary falls
is a question about code rather than a guess. `docs/c-family.md`'s table names
this record's trigger: *the safety IR is extracted from the `safec` crate*, and
the decision due is *which crates may depend on which*.

## Decision Drivers

* `docs/repository.md`: **the boundary that must not be crossed is the boundary
  worth making a crate**, because cargo enforces the arrow at build time.
* Everything else stays in one crate until it hurts. Nothing else hurts.
* The testable half of `docs/c-family.md`: an analysis must be runnable over IR
  built by hand in a test, with no frontend present.
* A crate whose dependency is a manifest line is reversed by deleting the line.
  A guard is only worth taking if a reversal fails to build.

## Considered Options

* Two crates: `safec-ir` holding `source`, `ir`, `interp` and the IR printer,
  and `safec` holding everything else and depending on it.
* Two crates with `source` left in `safec`, so the IR crate holds only the IR.
* The full split `docs/repository.md` sketches: `lexer`, `parser`, `ast`,
  `sema`, `types`, `safety-ir`, and the rest, one crate each.
* One crate, with the arrow left as intent and a review comment enforcing it.

## Decision Outcome

Chosen option: two crates, with `source` on the IR side.

The workspace graph is one arrow and it is the whole decision:

```text
safec  ->  safec-ir
```

`safec-ir` depends on no other crate in this workspace, and nothing is allowed
to change that. `safec` depends on it. There is no third crate and no second
arrow, so any dependency question this repository can currently ask has one
answer.

`source` goes to the IR side because `Span` is in the IR: every statement,
terminator and diagnostic the interpreter produces carries one. Leaving
`source` in `safec` would make the IR crate depend on the frontend to say where
an instruction came from, which is the arrow this record exists to forbid,
pointing the wrong way for the most ordinary reason available.

The IR printer moves with the IR for the same reason the IR's `name()` methods
did: what an `--emit safety-ir` artifact looks like is a fact about the IR, and
a printer on the other side of the arrow is a second place that has to be
taught every time a variant is added. `safec`'s AST and token dumps import the
shared line helpers from `safec-ir` rather than the other way round.

Diagnostics stay in `safec`. `docs/c-family.md` says a record is opened when a
decision guards something now, and nothing in `safec-ir` reports: the
interpreter answers a `Trap` and the printer answers text. When an analysis
lands in the IR crate it will have to report, and that is the trigger to decide
where `Diagnostic` lives, not this change.

### Confirmation

Adding `safec = { path = "../safec" }` to `crates/safec-ir/Cargo.toml` is the
mutation, and cargo refuses the workspace before compiling anything:

```text
error: cyclic package dependency: package `safec v0.1.0 (...)` depends on itself. Cycle:
package `safec v0.1.0 (...)`
    ... which satisfies path dependency `safec` (locked to 0.1.0) of package `safec-ir v0.1.0 (...)`
    ... which satisfies path dependency `safec-ir` (locked to 0.1.0) of package `safec v0.1.0 (...)`
```

The manifest is the guard because the manifest is where the reversal would be
written. A `use safec::` with no such line does not resolve either, which
catches the same reversal one step later. Which code that is depends on how far
the path gets: `use safec::driver;` is `error[E0432]: unresolved import safec`
and `use safec::lexer::lex;` is `error[E0433]: cannot find module or crate
safec`. Both were measured; neither is worth relying on, which is why the
manifest is the guard this record names.

The testable half is `a_function_built_by_hand_runs` and the five other
hand-built interpreter tests, which live in `crates/safec-ir/src/interp.rs` and
compile with no frontend in the dependency graph at all. `cargo tree -p
safec-ir` prints one line, which is the same claim from the other side. The
eighteen interpreter tests that compile a C program before running it moved to
`crates/safec/tests/interp.rs`, because a test that lexes C is a test of the
pair rather than of the IR.

The printer's five hand-built tests moved with the printer for the same reason,
and `--emit safety-ir`'s corpus cases did not: they run the driver, so they
stayed where the driver is, and they are what says this change moved code
without changing a byte of what it prints.

### Consequences

* Good, because the arrow `docs/c-family.md` depends on is now held by cargo
  rather than by a reviewer noticing.
* Good, because the Clang adapter it describes has a target that exists: a
  crate it can depend on without pulling in a C frontend.
* Bad, because a change spanning both crates is two manifests and one more
  place to keep versions aligned. Both are workspace-inherited, so the cost is
  bounded at the manifest rather than at every dependency.
* Bad, because `source` is now in a crate named for the IR. `Span` is used by
  the lexer, the parser and the diagnostics, none of which are IR, so the name
  is narrower than the contents.
* What would reverse this: an analysis that genuinely needs a frontend type,
  which would mean the type belongs on the IR side rather than that the arrow
  is wrong. Merging the crates back is one manifest deletion and a rename of
  `safec_ir::` to `crate::`.

## Pros and Cons of the Options

### Two crates, `source` on the IR side

* Good, because the arrow is a manifest line and reversing it fails to build.
* Good, because the IR crate compiles and runs with nothing else present, which
  is exactly the property `docs/c-family.md` asks to be testable.
* Bad, because `safec-ir` holds `source`, which is not the IR.

### Two crates, `source` left in `safec`

* Good, because each crate is named for what it holds.
* Bad, because it does not work: `Span` is in the IR, so the arrow would have
  to point from the IR crate at the frontend, which is the thing being
  forbidden. Duplicating `Span` on both sides trades the arrow for two types
  that must agree and nothing to check that they do.

### The full split from `docs/repository.md`

* Good, because every boundary is enforced rather than one.
* Bad, because `docs/repository.md` says not to: split crates when boundaries
  become clear, and the lexer, parser and sema boundary is not being crossed by
  anything today. Enforcing a boundary nobody is pushing on costs the crates
  and buys nothing.

### One crate

* Good, because it is what exists and costs nothing to keep.
* Bad, because the arrow stays intent. `docs/c-family.md` is explicit that this
  is the failure mode: intent lasts until the first convenient place to reach
  through it, and inside one crate every place is convenient.

## More Information

* [`docs/c-family.md`](../c-family.md), *What makes the boundary real rather
  than intended*, and the trigger row in *When each decision is due*.
* [`docs/repository.md`](../repository.md) for the criterion this applies.
* [ADR-0003](./0003-pass-the-source-map-to-each-render-call.md) and
  [ADR-0004](./0004-resolve-the-strictest-level-where-the-policy-is-built.md)
  are the other decisions here confirmed by a build failure rather than an
  assertion.
