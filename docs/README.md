# Documentation

What this project wants to build, and why each decision was taken.

These documents are **intent, not specification**. They say roughly what is
wanted; they are not a contract the code has to satisfy. A design may knowingly
diverge from them, and when it does, the divergence is recorded in an
[ADR](adr/) and the document here is updated in the same change.

Written in English, like the code they describe.

## Reading order

| Document | The question it answers |
|---|---|
| [concept.md](concept.md) | Why this project exists, why the target is C, and why it is written in Rust |
| [safety-model.md](safety-model.md) | What "safe" means here: what is tracked, what a check can conclude, how much is demanded at each level, and where annotations come in |
| [architecture.md](architecture.md) | The shape of the pipeline, why the Safety IR is a boundary rather than a stage, and what hangs off it |
| [frontend.md](frontend.md) | How the lexer and parser are built up, in stages |
| [diagnostics.md](diagnostics.md) | Why a diagnostic is a first-class value |
| [clang-integration.md](clang-integration.md) | The second frontend, and the analysis both share |
| [repository.md](repository.md) | Crate layout and the dependency policy |
| [roadmap.md](roadmap.md) | What to build first, and in what order |
| [design-principles.md](design-principles.md) | The short list to return to while implementing |

## Decisions

[`adr/`](adr/) holds the Architecture Decision Records: why a cross-cutting
choice was made, when, what would reverse it, and which test fails if it is
violated. The documents above state the outcome and link to the record; they do
not repeat the reasoning.
