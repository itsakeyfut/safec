---
status: "accepted"
date: 2026-09-24
decision-makers: itsakeyfut
---

# A level a run cannot deliver is a conclusion it could not prove

## Context and Problem Statement

`--safety` selects a level, and two different things can leave that level
undelivered: the checks it selects may not be implemented, which is true of
levels 2 to 5, or the artifact may stop before the Safety IR the checks read,
which is true of `--emit tokens` and `--emit ast`. Today both exit 0 with an
empty stderr, so a run that asked for the level defined as leaving nothing
`Unknown` and got nothing at all reports success.
[ADR-0033](./0033-a-conclusion-this-analysis-could-not-prove-does-not-build.md)
already named this as
[#62](https://github.com/itsakeyfut/safec/issues/62) and as the bottom row of
`CLAUDE.md`'s failure list while deciding something else.

It has to be decided before the answer is built on. Every level above 1 lands on
top of it, and the shape chosen here decides whether `--safety lifetime` can be
written in a build file today.

The tree relies on this decision as of the change that implemented it, and the
status moved with it.

## Decision Drivers

* The level ladder exists for migrating existing C.
  [`safety-model.md`](../safety-model.md#incremental-safety-levels) opens with
  "avoid requiring every existing C program to become fully safe immediately",
  and ADR-0033 says the ladder "exists precisely so that" a plain run over
  unconverted C does not reject at the first line. So a level is a declaration
  about a codebase as well as an instruction to the compiler, and refusing the
  declaration until the compiler catches up costs the thing the ladder is for.
* `--allow-unknown` is already the migration knob, and ADR-0001 puts the
  decision it drives in one place, the sink. A second place that reads the
  policy is the defect that record exists to prevent.
* [`diagnostics.md`](../diagnostics.md) splits diagnostics three ways. A fact
  about the invocation carries no code and is built with `Diagnostic::error`; a
  conclusion is built with `Diagnostic::concluded`, which is the only way to
  reach `Certainty::Unproven` from outside that module. The two constructors
  decide different things, so the choice between them is the decision rather
  than a detail of it.
* The corpus is 271 cases and what each option costs it was measured rather than
  estimated. None of the 76 `--emit ast` and `--emit tokens` cases passes
  `--safety`; three cases pass `--safety off`; no case anywhere passes a level
  above `memory`.

## Considered Options

* An unproven conclusion, reported through the sink
* A codeless refusal about the invocation, always an error
* A conflict raised by the argument parser
* Run the checks for every artifact, so that no level is undeliverable
* Report only when `--safety` was typed rather than defaulted

## Decision Outcome

Chosen option: **an unproven conclusion, reported through the sink**, because it
is the only option that lets a level be declared before the compiler can honour
it, and because the migration behaviour then falls out of machinery that is
already recorded instead of being invented beside it.

A run delivers the lowest of three things: the level it asked for, the highest
level with checks behind it, and what its artifact can carry. Where that is
below what was asked for, the run reports what it did not establish, with no code
and no span, and the sink decides the severity. Under the default that is an
error and the build fails. Under `--allow-unknown` it is a warning and the build
proceeds, which is what a program still being migrated asks for and is the same
sentence ADR-0033 wrote about a conclusion the analysis reached but could not
prove.

The cost is that `Conclusion` is documented as what a check concluded, and this
is a check that did not run. That stretch is deliberate and the same doc comment
licenses it: a check that cannot name a remedy "says instead what it failed to
establish, which claims nothing about the program". Nothing was established
about lifetime safety by a run with no lifetime check in it, and that is a true
sentence in the vocabulary the sink already speaks.

**The level a run defaults to is derived from its artifact**, `memory` where the
artifact reaches the IR and `off` where it does not, rather than being the
constant `memory` for all six kinds. Without this the decision above reports on
every `--emit ast` run in the world, because `--safety` defaults to `memory` and
an AST dump can never deliver it. With it, a dump claims nothing and says
nothing, an explicit `--safety` above what the run can deliver is reported, and
no corpus expectation changes at all.

### Confirmation

Every mutation below was applied on its own and measured by running the whole
workspace with `--no-fail-fast`. No total is given: this repository has had a
count in prose about code elsewhere go wrong four times, twice in this change,
and one of these mutations is measured against two tests. **Four of the sections
are here because a tier-2 review found what the first pass did not reach**, and
they are the four whose subject is what the report says rather than what the
gate decides.

**The two axes, twice: the gate and what it says.** The second was got wrong and
shipped to review. In
`driver.rs::undelivered`, compare `options.safety` against
`SafetyLevel::IMPLEMENTED` rather than against `Options::delivered`, which keeps
the level axis and drops the artifact one. Exactly one test fails,
`an_artifact_that_stops_before_the_ir_delivers_no_checks`, while
`a_level_with_no_checks_behind_it_is_not_delivered` stays green. That asymmetry
is the guard rather than a side effect of it: a gate answering one axis leaves
the other answered by nothing, and these two cases are what make the halves
separable at all.

**The combination.** Dropping the `min` from `Options::delivered` fails
`options.rs::a_run_is_delivered_no_more_than_the_levels_with_checks_behind_it`
and both reporting corpus cases.

**The artifact question.** Answering `true` for `Ast` from
`EmitKind::reaches_the_ir` fails that predicate's table, the delivered table and
the artifact corpus case. Answering `false` for `Executable` fails
`options.rs::a_kind_that_is_a_program_reaches_the_ir`, the roster invariant that
covers the kind nobody has written yet: `E0004` makes somebody write an arm and
nothing whatsoever makes the arm they write correct, which is RK-015 in the
review knowledge bank.

**The constructor, which is the decision this record took.** Answering the
conclusion `Unsafe` rather than `Unknown` is how `Diagnostic::concluded` builds
an error instead, and `a_migrating_run_is_told_what_was_not_established_and_builds`
goes from exit 0 to exit 1. Without that case the choice between the two
constructors is invisible to the suite, because both produce the same text.

**The derived default.** Resolving an unnamed `--safety` to
`SafetyLevel::Memory` unconditionally in `Cli::into_options` fails 84 tests, of
which the only unit test is
`cli.rs::the_default_level_is_what_the_artifact_can_carry`. The loudness is the
answer rather than noise: it is every dump in the corpus reporting that the old
default was false for it.

**The reporting of those axes.** Choosing the note and the remedy with one `if
options.emit.reaches_the_ir()`, which is how this shipped, fails
`both_reasons_a_level_can_go_undelivered_are_said_at_once` while the two
single-cause cases stay green. A run whose level and artifact both fall short was
told one of the two, and its remedy sent the reader to `--emit safety-ir`, which
is refused again for the other reason. ADR-0034 makes a remedy the change that
would make the program compile, so half of it was a promise the run breaks.
Offering `--allow-unknown` whatever the level fails the same case: `Cli::check`
refuses that pair at `strict`, so following the advice was an argument conflict.

**The level this compiler says it implements.** Moving
`SafetyLevel::IMPLEMENTED` up one without wiring a check for the level it moves to
fails five tests, and only
`driver.rs::the_implemented_level_runs_a_check_the_level_below_it_does_not` says
something is absent. The other four are three `.stderr` files and a table row,
every one of which reads as an expectation to re-bless, and re-blessing them ships
a run that exits 0 with an empty stderr on `int *foo(void) { int x = 42; return
&x; }` at `--safety lifetime`, where the silence now means the level was
delivered. That is the bottom row of `CLAUDE.md`'s list reached through
housekeeping, and it is what this record claimed the expiring corpus case
prevented. It did not; it only asked for an edit.

**The level a user reads.** Special-casing one variant inside
`SafetyLevel::spelling` fails
`every_safety_level_is_spelled_the_way_the_command_line_takes_it`. Before that
table it failed nothing: `ownership` and `thread` reach `spelling` from nowhere,
because no case asks for them, so a diagnostic naming a flag the user never typed
was one edit away.

**The consequence this record leads with.** Building the report with
`Diagnostic::error` rather than `Diagnostic::concluded` fails
`object.rs::a_migrating_build_still_makes_an_object`, which is `--safety lifetime
--allow-unknown --emit object` exiting 0 with a real object beside the warning.
Every corpus case here asks for a dump, so "can go into a build file today" was
inferred from two separately tested properties rather than held by anything.

**What nothing holds.** Resolving an unnamed level to `SafetyLevel::IMPLEMENTED`
instead of `Memory` fails nothing, because the two are equal today. They are not
the same thing: `IMPLEMENTED` rises when a level lands, and a default that rose
with it would reject programs that built the day before, which is what the ladder
exists to prevent. The comment at the resolution says so and no test can, until a
second level exists, which is the change that has to read it.

**What is weaker than it looks, measured rather than assumed.** The corpus case
`a_level_that_was_not_asked_for_is_not_reported` is the only place a satisfiable
level sits beside an artifact that carries no check, so nothing else pins the
combination. No mutation isolates it: every one that reaches it fails the 76 bare
dump cases as well, because the derived default already makes a bare dump resolve
to `off`, so "report above `off`" and "report where delivered differs" agree on
every input the corpus has. It documents the row rather than discriminating a
defect, and its table entry says so.

### Consequences

* Good, because `--safety lifetime` can go into a build file today and starts
  enforcing on the day the lifetime checks land, with no edit to that build file.
* Good, because the migration behaviour needs no new policy: `--allow-unknown`
  governs it through the sink, so there is still one place that turns a request
  into what the build does.
* Good, because no corpus expectation changes. Measured, and it is the reason
  the default is derived rather than reported against: the alternative rewrites
  76 expected outputs with the same line.
* Bad, because `Conclusion` now covers a check that did not run as well as one
  that ran and could not prove something. A reader who takes the type's name
  literally will find one case that does not fit it.
* Bad, because `--safety` no longer has one default a reader can hold in their
  head, and `--help` has to state a rule instead of printing
  `[default: memory]`. That line is currently false for two of the six
  artifacts, which is why the trade is worth taking, but a rule is still harder
  to read than a value.
* Bad, because a run that dumps a tree is silently unchecked. It says so in
  `--help` and in
  [`safety-model.md`](../safety-model.md#incremental-safety-levels), and prose is
  not a guard. What makes it defensible rather than a hole is that the level such
  a run resolves to is `off`, and `off` is defined as compiling ordinary C with
  nothing said about it, so the exit code claims only that the dump was produced.
* What would reverse this: a level whose checks exist but which some artifact
  still cannot carry. Today the two reasons a level goes undelivered are
  independent, and the rule takes the minimum of both. If an artifact ever
  carries some checks and not others, "what this artifact can carry" stops being
  a level on this ladder and the minimum stops being the right combination.
  Also, a `--warn <LINT>` system: `allow_unknown`'s own doc comment says
  `unknown` becomes its first lint, and at that point what governs this report
  is a lint level rather than one flag.

## Pros and Cons of the Options

### An unproven conclusion, reported through the sink

* Good, because a level can be declared before it can be honoured, which is what
  the ladder is for.
* Good, because `--allow-unknown` governs it with nothing added, and ADR-0001's
  single place survives.
* Good, because it adds no `Diagnostic::error`, so the count in
  [`diagnostics.md`](../diagnostics.md) stays at twelve. That count has been
  wrong three times and every one of them was a change that added a refusal
  without coming back to the document.
* Bad, because it widens what a `Conclusion` is.

### A codeless refusal about the invocation, always an error

* Good, because it is the category
  [`diagnostics.md`](../diagnostics.md) already defines for a fact about the
  invocation, and it leaves `Conclusion` alone.
* Good, because the precedent is in the history: `-o` was reported as
  unsupported by a corpus case until the commit that honoured the flag deleted
  both.
* Bad, because `--allow-unknown` cannot reach it, so `--safety lifetime` fails
  every run until Phase 8 and cannot be written down in advance. That is the
  ladder's purpose, spent to keep a type's name narrow.
* Bad, because deciding its severity in the driver would put a second reader of
  the policy beside the sink, so it has to be an error unconditionally rather
  than by choice.

### A conflict raised by the argument parser

* Good, because the run never starts, which is higher on `CLAUDE.md`'s list than
  anything reported.
* Good, because it needs no diagnostic and no change to the count.
* Bad, because `--safety` defaults to `memory`, so every `--emit ast` run is
  refused until the user types `--safety off`. All 76 dump cases in the corpus
  would carry that flag, and so would every user reading a tree.
* Bad, because the Clang adapter builds an `Options` without the parser and
  would walk past the refusal entirely.

### Run the checks for every artifact, so that no level is undeliverable

* Good, because it keeps
  [`safety-model.md`](../safety-model.md#incremental-safety-levels)'s sentence
  that a level decides which checks run, with no exception for the artifact.
* Bad, because it buys nothing measurable. None of the 76 dump cases contains
  `free` or `malloc`, so zero safety findings appear.
* Bad, because six of them gain a lowering refusal and exit 1 where they have no
  expected stderr today, which makes the frontend's debugging artifact fail on
  exactly the programs it exists to debug.
* Bad, because `--emit tokens` has no tree to lower, so it needs a second answer
  and the rule stops being one rule.

### Report only when `--safety` was typed rather than defaulted

* Good, because the corpus stays green with no derived default.
* Bad, because the default run is the silent one. A bare `safec --emit ast` opts
  out of checking without anything being written anywhere, which is the opposite
  of how the only opt-out this project has is meant to work: something typed, in
  a place a reader can find.
* Bad, because the same command means two things depending on whether a default
  was spelled out, and the compiler cannot tell the user which one they got.

## More Information

* [#62](https://github.com/itsakeyfut/safec/issues/62), where the two halves are
  measured and the design comment is.
* [ADR-0001](./0001-promote-unproven-results-in-the-sink.md) for why the sink is
  the one place a policy is applied.
* [ADR-0033](./0033-a-conclusion-this-analysis-could-not-prove-does-not-build.md)
  for what asking to be checked means, and for the sentence that names this
  issue.
* [ADR-0034](./0034-a-diagnostic-that-rejects-a-program-carries-the-change-that-would-make-it-compile.md)
  for why the remedy is a parameter, which is what stops this report shipping
  with nothing to change.
* [`safety-model.md`](../safety-model.md#incremental-safety-levels), which gains
  the rule in the same change, and
  [`diagnostics.md`](../diagnostics.md) for the three kinds of diagnostic.
* `crates/safec/src/cli.rs`, `crates/safec/src/options.rs`,
  `crates/safec/src/safety.rs` and `crates/safec/src/driver.rs` carry the
  behaviour. `crates/safec/src/diagnostics/render.rs` and
  `crates/safec-ir/src/analysis.rs` carry comments this decision made false: the
  first counted the producers of an unproven conclusion, the second said what the
  value means.
