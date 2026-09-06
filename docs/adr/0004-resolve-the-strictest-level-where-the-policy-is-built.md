---
status: "accepted"
date: 2026-09-06
decision-makers: itsakeyfut
---

# Resolve the strictest safety level where the policy is built, not only where the options are

## Context and Problem Statement

`--safety strict` is defined as leaving nothing `Unknown`, so it implies
`--deny-unknown`. That implication was resolved in one place,
`Cli::into_options`, which sets

```rust
deny_unknown: deny_unknown || safety == SafetyLevel::Strict,
```

and `Options::deny_unknown` documents itself as "the resolved answer, not the
raw flag". `Policy::from(&Options)` then read the field and believed it.

`Options` has public fields and no constructor. Nothing makes the documented
invariant true for an `Options` that was not built by the parser, and the
roadmap already has a caller that will not use the parser: the Clang adapter in
[`docs/clang-integration.md`](../clang-integration.md) is a second frontend into
the same safety infrastructure, and it reaches that infrastructure without
passing through this crate's command line. A library embedding is the same case.

Such a run asks for the strictest safety level and gets `deny_unknown: false`.
Every result the analysis could not prove is then reported as a warning rather
than promoted to an error (see
[ADR-0001](0001-promote-unproven-results-in-the-sink.md)), `has_errors` is false,
and the compilation exits successfully. The user asked for the level that proves
everything and was told the code passed. That is the worst failure this project
can have, and it is quiet: the diagnostics are all still printed, and only their
severity and the exit code are wrong.

The first attempt at this record put the guard in `Policy::from(&Options)` and
left `Policy` a public struct with a public field. That is worth recording,
because it is the reason the decision reads as it does. It guarded nothing:

```rust
// From outside the crate, with no `Options` anywhere in sight.
let mut sink = DiagnosticSink::with_policy(Policy { deny_unknown: false });
sink.report(Diagnostic::unproven("`p` may escape"));
assert!(!sink.has_errors()); // passed
```

A conversion into an open type is not a choke point. Any caller who can write
the destination by hand can write the answer the guard exists to compute, and
the caller most likely to do so is the one this record is about: an adapter
holding a safety level and a boolean, wiring up a sink.

So the question is not only where the implication belongs. It is what has to be
true of `Policy` for an answer to that question to hold at all.

## Decision Drivers

* A safety invariant that decides whether a build passes must not depend on a
  caller having remembered something. Public fields cannot enforce it.
* A guard has to sit where it cannot be walked around. Placing one on a
  conversion, while the type it produces stays constructible by hand, buys the
  appearance of enforcement and none of it.
* The failure is silent, and it errs in the direction of telling the user their
  code is fine. That is the combination least likely to be noticed, in review or
  in a test that only reads the printed output.
* `Options` is the interface the Clang adapter and any library embedding will
  build. It should be hard to hold wrong, and it is a plain record today.
* Duplicated logic is a real cost, and two copies of a rule can drift.

## Considered Options

* **Resolve it in `Policy::new`, and make the field private**, so the resolution
  is the only way to arrive at a policy.
* **Resolve it in `Policy::from(&Options)` and leave `Policy` open**, which is
  what was tried first.
* **Make `Options::deny_unknown` private behind a resolving constructor**, so
  the invariant holds at its source instead.
* **Leave it in `Cli::into_options` alone**, and document the obligation on
  `Options`.

## Decision Outcome

Chosen option: **resolve it in `Policy::new`, and make the field private**.

```rust
pub struct Policy {
    deny_unknown: bool,
}

impl Policy {
    pub fn new(safety: SafetyLevel, deny_unknown: bool) -> Self {
        Self {
            deny_unknown: deny_unknown || safety >= SafetyLevel::Strict,
        }
    }

    pub fn deny_unknown(self) -> bool {
        self.deny_unknown
    }
}

impl From<&Options> for Policy {
    fn from(options: &Options) -> Self {
        Self::new(options.safety, options.deny_unknown)
    }
}
```

The two arguments to `new` are the whole question, so a caller holding both is a
caller who could answer it wrong, and now does not get to. `From<&Options>`
becomes one line that delegates, which keeps the conversion as the mapping from
the command line while putting the rule itself where it cannot be routed around.

`safety` is read and not kept. A policy says what to do with a result that
arrives, and which checks ran to produce it is a different question; a field no
caller reads is a field invented ahead of its use. Adding it later is additive,
because the constructor already takes the level.

The comparison is `>=` rather than `==`. `SafetyLevel` derives `Ord`, and its
own doc comment asks passes to write `level >= SafetyLevel::Lifetime` rather
than matching each variant. The two spellings are identical today, `Strict`
being the last variant, and `==` would fail in the unsafe direction the moment
one is added after it.

The duplication with `Cli::into_options` is real and is accepted on purpose. The
two copies are not one rule stated twice for convenience. `Cli::into_options`
resolves the field so that `Options::deny_unknown` reads truthfully to anyone
who inspects it, and `Policy::new` resolves it again so that the sink does not
depend on anyone having done so. Both sites name the other. If the two ever
disagree they disagree in the safe direction, because `||` can only add denial
and never remove it.

`Policy::default()` survives and is not a hole. `Default` names no safety level
and therefore asks for nothing: it is the lenient policy that
`DiagnosticSink::new` documents itself as having. The invariant is that a caller
who asks for the strictest level gets it, and a caller who asks for nothing has
not asked.

Closing `Options` the same way was considered and not done, for a reason about
timing rather than merit. `Options` is still growing, its shape is settled by
neither the adapter nor a library embedding, and a constructor written before
either exists would be guessing at its argument list. `Policy` had none of those
problems: one field, a settled shape, and closing it cost eight edits that the
compiler found and that were all in tests.

### Confirmation

The stronger half is a compile error rather than an assertion. The bypass above
no longer builds, inside the crate or outside it:

```
error[E0451]: field `deny_unknown` of struct `Policy` is private
```

A reversal is therefore caught by the compiler, the way ADR-0003's borrow is,
rather than by remembering to run a test.

The resolution itself is guarded by three tests, one at each level it can be
reached from:

* `the_strictest_safety_level_denies_unknown_however_the_policy_is_built` in
  `crates/safec/src/diagnostics.rs` calls `Policy::new(SafetyLevel::Strict,
  false)` directly, which is the adapter's path, and asserts both the resolved
  answer and that an unproven diagnostic reaching the sink becomes an error.
* `the_strictest_safety_level_denies_unknown_even_unresolved` in the same file
  goes through an `Options` built by hand with `safety: Strict` and
  `deny_unknown: false`.
* `the_strictest_safety_level_denies_unknown_in_the_sink` in `crates/safec/src/driver.rs`
  goes through `compile`, so the guard covers the path an actual run takes.

Verified by mutation. Replacing the body of `Policy::new` with

```rust
Self { deny_unknown }
```

fails exactly those three and nothing else. The parser's own resolution keeps
its own guards in `crates/safec/src/cli.rs`
(`the_strictest_safety_level_denies_unknown_on_its_own` and
`a_lower_safety_level_leaves_deny_unknown_to_its_flag`), and those still pass
under that mutation. That is the point of the record: they never covered this.

### Consequences

* Good, because the strictest safety level cannot be asked for and silently not
  applied, by any caller, through any path.
* Good, because the rule now lives where it is acted on, so a later policy field
  that a safety level implies has an obvious place to go, and `Policy::new`
  already takes the level such a field would need.
* Good, because a reversal is a compile error rather than a test somebody has to
  remember to keep.
* Bad, because the implication is written twice, and a reader who finds only one
  copy may take it for the only one. Both sites name the other.
* Bad, because it closes one invariant and leaves `Options` open. Other
  inconsistent combinations are still constructible, `--emit tokens --safety
  strict` among them, and nothing here helps with those.
* Bad, because `Policy` gained a constructor and an accessor that a one-field
  struct would otherwise not need.
* What would reverse this: nothing currently foreseen. Giving `Options` a
  constructor as well would strengthen the same invariant at its source rather
  than replace this, and the guard here should stay either way, because the
  tests that pin it are about what the sink does rather than about how an
  `Options` was made.

## Pros and Cons of the Options

### Resolve it in `Policy::new`, with a private field

* Good, because the invariant holds by construction, for every caller, and the
  compiler enforces it.
* Good, because it puts the rule next to the sink that acts on it.
* Good, because it cost eight edits, all in tests, all found by the compiler.
* Bad, because the rule appears in two files and can be read as an oversight.

### Resolve it in `Policy::from(&Options)`, leaving `Policy` open

* Good, because it is one `||` and changes no interface.
* Bad, and disqualifying, because it does not hold. `Policy { deny_unknown:
  false }` reaches `DiagnosticSink::with_policy` from anywhere and reproduces
  the exact failure this record was written to prevent.
* Bad, because it reads as a guard, so the next reader stops looking.

### A private field on `Options` behind a resolving constructor

* Good, because the invariant would hold at its source, and it would also cover
  the other inconsistent combinations that appear as `Options` grows.
* Bad, because the constructor's argument list would be a guess right now, and
  changing it later breaks the interface the adapter is written against.
* Bad, because it would not remove the need for the guard here. `Policy` is
  reachable without an `Options` at all.

### Leave it in `Cli::into_options` and document the obligation

* Good, because there is one copy of the rule.
* Bad, because the documentation sits on the field, and reading a field's doc is
  not something a caller filling in a struct literal is obliged to do.
* Bad, because the failure it permits is the one this project treats as
  unacceptable, and it fails quietly.

## More Information

* `crates/safec/src/diagnostics.rs`: `Policy`, `Policy::new`,
  `impl From<&Options> for Policy`.
* `crates/safec/src/cli.rs`: `Cli::into_options`, the other resolution site.
* [ADR-0001](0001-promote-unproven-results-in-the-sink.md): why the sink is the
  enforcement point, and what `deny_unknown` decides once a result gets there.
* [ADR-0003](0003-pass-the-source-map-to-each-render-call.md): the other place a
  reversal in this crate is caught by the compiler rather than by an assertion.
* [`docs/safety-model.md`](../safety-model.md): what the strictest level claims.
* [`docs/clang-integration.md`](../clang-integration.md): the Clang adapter, the
  second frontend that reaches the safety infrastructure without this crate's
  command line.
