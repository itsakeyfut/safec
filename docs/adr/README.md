# Architecture Decision Records

A design decision's rationale lives here and nowhere else. The documents in
[`docs/`](../README.md) say what the project wants to build; the doc comments in
the code say what each type is and how to use it. Neither repeats the reasoning:
they link here.

These records are written in **English**, like the code they constrain, because
they are read by contributors and tooling alike.

Format: [MADR 4.0](https://adr.github.io/madr/), the de facto Markdown ADR
standard. Copy [`adr-template.md`](./adr-template.md) to start one.

## Index

| # | Decision | Status | Confirmed by |
|---|---|---|---|
| [0001](./0001-promote-unproven-results-in-the-sink.md) | Tag a result the analysis could not prove at the check, and promote it in the sink | accepted | unit tests in `crates/safec/src/diagnostics.rs` (promotion on, promotion off, a proven warning left alone, `error_count` equals a recount) and the `--safety strict` resolution test in `crates/safec/src/cli.rs`; each verified by the mutation that makes it fail |
| [0002](./0002-severity-runs-from-least-to-most-severe.md) | Declare `Severity` from least to most severe, the same direction as `SafetyLevel` | accepted | `severities_are_ordered_from_least_to_most_serious` and `the_worst_severity_in_a_set_is_its_maximum` in `crates/safec/src/diagnostics.rs`; both fail if the variants are put back in descending order |

**By status**: accepted: 0001, 0002 · proposed: none · superseded: none

Records are numbered consecutively from `0001`.

## Where each kind of writing belongs

| Location | Holds | Does not hold |
|---|---|---|
| [`docs/*.md`](../README.md) | what the project wants to build, roughly: the concept, the safety model, the architecture, the phases | how a decision was reached. These are intent documents, not specifications, and a design may knowingly diverge from one. Say so in the record and update the document |
| doc comments in the code | what a type is, how to use it, and the local reason it is shaped that way | cross-cutting rationale; link here instead |
| `docs/adr/**` | why a cross-cutting decision was chosen, when, and what would reverse it | type or signature detail; link to the code |

## When to write one

* Two or more implementations are possible and one is chosen, especially when the
  choice shapes an interface that later phases will be written against.
* A choice affects whether the compiler can be *trusted*: anything that decides
  when a result is reported, suppressed, or promoted. Getting one of these wrong
  makes `safec` claim code is safe that was never proven, which is the worst
  failure this project has.
* An existing decision is reversed: write a new record, mark the old one
  `superseded by ADR-NNNN`, and note what changed.
* A design knowingly diverges from a document in `docs/`: record why, and update
  that document in the same change.
* You are about to write "undecided" into a doc comment: open one as `proposed`.

**Not worth an ADR:** formatting, or anything affecting a single call site.
Naming usually is not worth one either, but it is when the name shapes an API
that the rest of the crate compares against or matches on, because those are the
names that go silently wrong.

## Conventions

* Filename `NNNN-short-slug.md`, numbers consecutive.
* MADR statuses: `proposed`, `accepted`, `rejected`, `deprecated`,
  `superseded by ADR-NNNN`.
* Every record fills in **Confirmation**: which test or guard fails if the
  decision is violated, **and the change to the code that makes it fail**. This
  repository verifies guards by breaking the code and watching the suite go red,
  so a record should name that mutation. If nothing would fail, say so.
* A `proposed` status while the codebase already relies on the decision is itself
  a defect; say so in *Context and Problem Statement*.
* Keep the status in sync between an ADR's front matter and its row in this index.
* Prose here uses no em dashes.

## More Information

* [MADR 4.0](https://adr.github.io/madr/): the template this follows.
* [`adr-template.md`](./adr-template.md): copy this to start a new record.
