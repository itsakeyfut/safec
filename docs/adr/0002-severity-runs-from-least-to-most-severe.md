---
status: "accepted"
date: 2026-09-06
decision-makers: itsakeyfut
---

# Declare `Severity` from least to most severe, the same direction as `SafetyLevel`

## Context and Problem Statement

`Severity` was declared `Error, Warning, Note, Help` with a derived `Ord`, so
`Error < Warning < Note < Help`. Its own doc comment offered
`severity <= Severity::Warning` as the way to filter, which in English reads as
"no more severe than a warning" while selecting the opposite.

The same crate declares `SafetyLevel` the other way round, `Off` through
`Strict`, and documents `level >= SafetyLevel::Lifetime` as "at least this
strict". Two ordinal enums in one crate that disagree about which way is up is a
trap, and a reader who has internalised one will write the wrong comparison
against the other. That is not hypothetical: during the review that produced this
record, one reviewer asserted that `Severity` "mirrors `SafetyLevel`'s deliberate
ordering" when the two were exactly opposed.

Nothing in the crate compares severities yet, so this is the cheapest it will
ever be to settle.

## Decision Drivers

* A filter written the wrong way round silently selects the wrong set. In a
  compiler whose job is to report unsafety, "warning or worse" quietly becoming
  "note and help only" means the user reads a clean report on code that failed a
  check.
* Internal consistency beats any external convention here, because a reader
  learns the rule once and applies it to every ordinal type in the crate.
* The blast radius today is one test, one doc line, and the order of four
  variants. `Severity: Ord` is not used anywhere else in the crate, in
  production code or in tests.
* The trap is the declaration order, not the `Ord` derive. Removing the derive
  and leaving `Error` first would leave the next person to re-derive it with a
  silently reversed order.

## Considered Options

* **Declare from least to most severe and keep `Ord`.**
* **Keep the current order, drop the `<=` example, and add a named predicate**
  such as `at_least(floor)` so nobody writes a raw comparison.
* **Reorder and drop `Ord` entirely**, so a raw comparison does not compile.

## Decision Outcome

Chosen option: **declare from least to most severe and keep `Ord`**:

```rust
pub enum Severity {
    Help,
    Note,
    Warning,
    Error,
}
```

`severity >= Severity::Warning` now means "warning or worse", the worst severity
in a set is its `max()`, and the direction matches `SafetyLevel`. A variant more
serious than `Error`, for a problem that has to stop the compilation where an
error would let it continue, has a place above it rather than below.

The doc comment states the direction and why, rather than only demonstrating an
idiom.

### Confirmation

Two unit tests in `crates/safec/src/diagnostics.rs`, and the mutation that fails
them is the same one: putting the variants back in descending order.

* `severities_are_ordered_from_least_to_most_serious` spells the chain out
  variant by variant. It is written as an explicit chain rather than a loop over
  the variants on purpose: a loop comparing each variant to the next holds for
  *any* declaration order, because the iteration order and the derived `Ord` come
  from the same source. That mistake was made once already, on `SafetyLevel`, and
  the tautological test it produced passed while the invariant was broken.
* `the_worst_severity_in_a_set_is_its_maximum` pins what the order is *for*:
  `max()` is the worst, `min()` is the least serious, and
  `filter(|s| *s >= Severity::Warning)` selects the warning and the error. This
  is the test that would catch a future reordering done for some other reason,
  because it fails on meaning rather than on declaration.

Verified: reordering the variants back to `Error, Warning, Note, Help` fails both
and nothing else.

### Consequences

* Good, because the crate now has one rule for ordinal enums: later variants are
  greater, and `>=` means "at least".
* Good, because `max()` over a set of severities is the worst one, which is the
  natural way to summarise a compilation.
* Good, because a `Fatal` or `Bug` variant has an obvious home above `Error`.
* Bad, because `Severity::Error` is no longer the first variant, so it is no
  longer what `#[derive(Default)]` would pick if `Severity` ever gains a default.
  It should not gain one: a diagnostic with an unstated severity is a bug in the
  caller, not something to default.
* What would reverse this: a need to sort diagnostics most-serious-first often
  enough that `sort_by(|a, b| b.cmp(a))` becomes noise. That is a weak argument
  against a reversal, and rendering order is report order rather than severity
  order anyway.

## Pros and Cons of the Options

### Declare from least to most severe and keep `Ord`

* Good, because it agrees with `SafetyLevel`, which is the comparison a reader of
  this crate will actually have in mind.
* Good, because it agrees with `codespan-reporting`, the closest analogue, which
  declares `Help, Note, Warning, Error, Bug` and puts
  `assert!(Severity::Error > Severity::Warning)` in the type's own doctest.
* Good, because it costs one test and one doc line today.
* Bad, because a raw comparison is still a raw comparison: the direction is now
  the intuitive one, but nothing stops someone writing `<=` and meaning `>=`.

### Keep the order and add a named predicate

* Good, because a named predicate reads unambiguously at the call site whichever
  way the variants are declared.
* Bad, because it leaves two opposed conventions in one crate, which is the
  actual defect. A helper only helps the callers who use it.
* Bad, because it adds an API with no caller, to work around an order that could
  simply be correct.

### Reorder and drop `Ord`

* Good, because a comparison written the wrong way round would not compile.
* Good, because `Ord` is unused today, so removing it costs nothing now and
  adding it back later is not a breaking change.
* Bad, because severities genuinely are ordered, and refusing to model that
  pushes every future filter into an explicit `matches!` over four variants.
* Bad, because once the declaration order is fixed, the trap this avoids is
  mostly gone, and the remaining benefit does not pay for the lost capability.

## More Information

* `crates/safec/src/diagnostics.rs`: `Severity`.
* `crates/safec/src/safety.rs`: `SafetyLevel`, the other ordinal enum, and the
  doc comment that states the convention this now matches.
* `codespan-reporting` 0.13.1, `src/diagnostic.rs`: the ordering doctest.
* [ADR-0001](0001-promote-unproven-results-in-the-sink.md), which separates what
  a check concluded from how severely it is reported. `Certainty` deliberately
  has no ordering: its two variants are not more and less severe, they are a
  different question.
