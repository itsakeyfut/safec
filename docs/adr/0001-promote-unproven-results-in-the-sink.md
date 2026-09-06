---
status: "accepted"
date: 2026-09-06
decision-makers: itsakeyfut
---

# Tag a result the analysis could not prove at the check, and promote it in the sink

## Context and Problem Statement

[The safety model](../safety-model.md) requires the analyser to distinguish
`Safe` / `Unsafe` / `Unknown` rather than pretend every C program can be proven
safe, and `Options::deny_unknown`, already parsed and already documented as
"report `Unknown` analysis results as errors rather than warnings", promises
that the third case can be made fatal.

Nothing in `crates/safec/src/diagnostics.rs` can express `Unknown`: a
`Diagnostic` carries a severity and nothing about what the analysis concluded.
`DiagnosticSink::report` also settles its error count at the moment a diagnostic
is pushed, and exposes no way to revise a severity afterwards, so the promotion
cannot happen after collection either. The promise cannot be kept today, and the
failure mode is silent: an unproven result reported as an ordinary warning leaves
`has_errors()` false and the process exits successfully, which is a positive
claim that C code passed a check it never passed.

This is decided before the first analysis pass exists, because every pass will be
written against whichever shape is chosen, and retrofitting it means revisiting
every construction site in every check.

## Decision Drivers

* A missed promotion is not a cosmetic bug. It makes `safec` say code is safe
  that was never proven safe, which is the worst failure this project has. The
  mechanism must live in one place that cannot be forgotten, not at hundreds of
  call sites.
* A check must not need `Options` in order to report. Threading policy into every
  analysis makes each one able to get it wrong independently.
* `error_count()`, `has_errors()` and `diagnostics()` must not be able to
  disagree. They agree today only because severity is frozen at push time, so any
  scheme that revises severity later has to keep that property.
* Nothing produces an analysis result yet, so an interface designed now has no
  consumer to validate it. Prefer the smallest shape that cannot be silently
  wrong over the most expressive one.
* The documents in [`docs/`](../README.md) record intent, not settled design.
  Where the two disagree, the design is decided on its merits and the document
  is updated.

## Considered Options

* **The sink is the enforcement point.** Checks build a `Diagnostic` that names
  what the analysis concluded; `DiagnosticSink::report` applies the policy.
* **A separate enforcement layer.** Checks return analysis results
  (`Safe`/`Unsafe`/`Unknown` plus a span and a message); a layer between the
  analysis and the diagnostics turns them into `Diagnostic`s under the policy.
* **Defer.** Add nothing; document that `deny_unknown` is not honoured, and
  decide when the first analysis pass lands.

## Decision Outcome

Chosen option: **the sink is the enforcement point**, because it puts the
promotion in exactly one function while leaving `Diagnostic` usable by the
non-safety diagnostics (syntax errors, `no input files`) that make up most of a
compiler's output. Read as inference at the check and enforcement at the sink, it
satisfies the separation of inference from enforcement without inventing an
interface that has no consumer yet.

The shape has three parts.

**A `Certainty` on the diagnostic.** Two variants, because `Safe` produces no
diagnostic at all:

| The analysis concluded | The check calls | Severity | Under `--deny-unknown` |
|---|---|---|---|
| Safe | nothing | n/a | n/a |
| Unsafe (proven) | `Diagnostic::error(..)` | `Error` | `Error` |
| Unknown (unproven) | `Diagnostic::unproven(..)` | `Warning` | **`Error`** |

Every existing constructor is `Certainty::Proven`, so a syntax error never has to
mention certainty. `Diagnostic::unproven` is the only new constructor: a proven
unsafe result is a statement of fact, and `error` already says that.

**Constructors that name the conclusion.** A safety check chooses a conclusion,
not a severity. Writing `Diagnostic::warning(..)` for an unprovable result is
still possible and still wrong, but it is visible in review, whereas a forgotten
`with_certainty(..)` on an otherwise complete call is not.

**The safety level is metadata.** `Diagnostic` carries
`level: Option<SafetyLevel>` so a report can say which check it came from, a
promise `SafetyLevel::number()`'s doc comment already makes, but the level does
not decide severity. Which checks run is the pass manager's business.

This diverges from the level sketch the project started with, in which level 1
produced *memory warnings*: here a proven unsafe result is an error at every
level, because there is no safety argument for knowing something is wrong and
saying it quietly. [The safety model](../safety-model.md) is updated in the same
change.

`SafetyLevel::Strict` is defined as "nothing may be left `Unknown`", so
`--safety strict` implies `--deny-unknown`. That is resolved once in
`Cli::into_options`, which exists for defaults that have to be computed, and
`Options::deny_unknown` becomes the resolved value rather than the raw flag.

### Confirmation

The promotion is `DiagnosticSink::report`'s only branch on certainty, so the
guards are unit tests in `crates/safec/src/diagnostics.rs`, each named with the
mutation that makes it fail:

* Removing the promotion from `report` fails the test that a sink with
  `deny_unknown` set turns an unproven warning into an error and counts it.
* Widening the promotion to every warning, by dropping the `Certainty::Unproven`
  condition, fails the test that a *proven* warning is left alone. This is the
  negative half, and without it the promotion could be correct for the wrong
  reason.
* Counting before promoting rather than after fails the test that `error_count()`
  equals a recount over `diagnostics()` in both policies.
* Dropping the `|| safety == SafetyLevel::Strict` clause fails the `cli.rs` test
  that `--safety strict` alone resolves `deny_unknown` to true.
* Making `Diagnostic::unproven` return `Certainty::Proven` fails the test that it
  is the only constructor producing an unprovable diagnostic.

Honestly: **this record precedes the code by one change.** Until those tests
land, nothing enforces any of it. And because no driver exists yet, nothing
constructs a sink from `Options` at run time. The mechanism will be complete and
tested, but `--deny-unknown` only takes effect end to end once the driver wires
`Policy::from(&options)`, which is one line.

### Consequences

* Good, because the promotion exists in one function. A check cannot forget a
  policy it never sees.
* Good, because `error_count()` stays derived from the severity that was actually
  recorded: promotion happens before counting, and nothing can mutate a
  diagnostic after it is pushed.
* Good, because `Certainty::Unproven` is reachable only through
  `Diagnostic::unproven`, which fixes the severity at `Warning`. An unproven
  diagnostic is therefore always a warning until the sink promotes it, and
  `promote_to_error` only ever handles one direction.
* Bad, because a check author can still write `Diagnostic::warning(..)` for a
  result they could not prove, and nothing detects it. The constructor names make
  it conspicuous; they do not make it impossible.
* Bad, because `diagnostics` now depends on `options` for `SafetyLevel`.
  `SafetyLevel` is a safety-model concept that happens to live in `options.rs`
  because it is also a CLI value, and it should move when the analysis layer
  lands.
* What would reverse this: an analysis pass that needs to report a result whose
  severity depends on more than certainty, for instance if the safety level turns
  out to belong in the policy after all, would push the decision toward the
  separate enforcement layer. The sink is where that would be noticed first.

## Pros and Cons of the Options

### The sink is the enforcement point

* Good, because the policy is read in one place and checks never see it.
* Good, because it needs one new constructor and one new field, and nothing that
  exists has to move.
* Good, because it works for the diagnostics that are not safety results, which
  is most of them.
* Bad, because policy now lives in the diagnostics module rather than in an
  analysis layer, which is a looser reading of "separate inference from
  enforcement" than the principle's wording suggests.

### A separate enforcement layer

* Good, because it is the literal shape the principle describes, and `Diagnostic` stays
  purely about reporting.
* Good, because a `Safe` result is representable, so a pass can report what it
  proved as well as what it did not.
* Bad, because it means designing an interface with no consumer: the first
  analysis pass does not exist, and a wrong guess costs more than the duplication
  it saves.
* Bad, because every safety result then travels through two types, and the
  conversion is another place to get the promotion wrong.

### Defer

* Good, because it is honest YAGNI and costs nothing today.
* Bad, because `deny_unknown` is already shipped in the CLI and already
  documented, so the gap is a live false promise rather than a missing feature.
* Bad, because the cost of choosing late is paid by every check written in the
  meantime.

## More Information

* [`docs/safety-model.md`](../safety-model.md): Safe / Unsafe / Unknown, the
  incremental safety levels, and the migration model in which "warnings /
  uncertainty" is what annotations exist to remove.
* [`docs/design-principles.md`](../design-principles.md): the principle this
  serves, "separate inference from enforcement".
* `crates/safec/src/options.rs`: `deny_unknown`, `SafetyLevel`.
* `crates/safec/src/diagnostics.rs`: `Severity`, `Diagnostic`, `DiagnosticSink`.
* The multi-agent review that found the gap: `DiagnosticSink::report` settles its
  count at push time, so no later pass can promote, and nothing distinguishes an
  unprovable warning from an ordinary one.
