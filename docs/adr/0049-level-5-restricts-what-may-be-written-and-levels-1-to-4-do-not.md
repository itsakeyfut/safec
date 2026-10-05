---
status: "accepted"
date: 2026-10-05
decision-makers: itsakeyfut
---

# Level 5 restricts what may be written, and levels 1 to 4 do not

## Context and Problem Statement

The safety levels are one axis, 0 to 5. `docs/safety-model.md` says "a level says **which checks run**, and nothing else", and also that level 5 is "the fully safety-checked subset", which leaves nothing `Unknown`. Every level accepts the same C today, and nothing says what "subset" means or how a program gets into it ([#328](https://github.com/itsakeyfut/safec/issues/328)).

Two kinds of safety are wanted. One is the most safety C allows without restricting C: any program is accepted as written, the checks report what they find, and what they cannot prove is a policy question. The other is safety of the kind a language that forbids the hard cases gives: some of what C allows is not accepted, so that everything accepted can be proved. The design of level 5 has to say which kind it is before any check is built for it.

## Decision Drivers

* An existing C program has to be able to adopt the checks without being rewritten first; that is what levels 1 to 4 are for.
* Leaving nothing `Unknown` is not reachable for all of C: what some programs do cannot be proved without restricting how they are written.
* A refusal has to tell the reader what kind of refusal it is: a defect the analysis found, a doubt it could not settle, or a construct this level does not accept.

## Considered Options

* **Two kinds on the one level axis**: levels 1 to 4 run checks and restrict nothing; level 5 also restricts what may be written.
* **A second axis, a mode beside the levels**, choosing restricted or unrestricted independently of which checks run.
* **No restriction at any level**, and level 5 only as the policy that nothing may be `Unknown`.

## Decision Outcome

Chosen option: **two kinds on the one level axis.** Levels 1 to 4 accept any C the frontend accepts and decide only which checks run. Level 5 runs every check and restricts what may be written, so that what it accepts can be proved. A second axis was rejected because the restricted kind is meaningless without every check behind it, so most of its combinations would be ones nobody should choose; no restriction at all was rejected because some programs cannot be proved as they are written, so "nothing `Unknown`" would refuse them with nothing for the reader to change.

- **A construct level 5 does not accept is a diagnostic about the program's text, not a conclusion.** It says the construct is outside the level-5 subset, carries a remedy saying how to write it inside, and has a code of its own topic, which takes the lowest free hundred when it is first emitted ([`docs/diagnostics.md`](../diagnostics.md)). It is not `Unknown`: a doubt is about an execution the analysis could not settle, and this is about how the program is written, which the reader settles by rewriting it.
- **The level is chosen per translation unit**, as it is now, on the command line. Code at a lower level meets level-5 code only through declarations, and what such a declaration does not say is answered conservatively: ADR-0050 says how for null. Mixing levels inside one translation unit, per function or per declaration, is a question of its own and is not decided here; it is [#331](https://github.com/itsakeyfut/safec/issues/331).
- **What level 5 restricts is a list, and an entry is added by the phase whose check needs it**, with a record of its own saying what it costs a C program and what it buys, measured. A restriction is never added ahead of the check it serves, because nothing could show what it buys. The first entry is null (ADR-0050).

### Confirmation

Nothing holds this yet: no level-5 restriction is implemented, and `--safety strict` is wired in Phase 8. The decision is held by `docs/safety-model.md`, which says it, and the first check that emits a level-5 diagnostic, [#330](https://github.com/itsakeyfut/safec/issues/330), will hold it in code. This record's Confirmation is to be rewritten then.

### Consequences

* Good, because adopting the checks never requires rewriting a program first, and reaching level 5 is a rewrite the diagnostics walk the reader through.
* Good, because a refusal at level 5 says which of three things it is.
* Bad, because "a level says which checks run, and nothing else" is no longer true of level 5, and every place that reasons from it has to say which level it means.
* What would reverse this: a check strong enough to prove the cases a restriction exists for, which would make that restriction removable.

## More Information

* [`docs/safety-model.md`](../safety-model.md), *Incremental Safety Levels*, which this amends.
* [ADR-0033](./0033-a-conclusion-this-analysis-could-not-prove-does-not-build.md) and [ADR-0035](./0035-a-level-a-run-cannot-deliver-is-a-conclusion-it-could-not-prove.md) for what `Unknown` means and how a level is delivered.
* [ADR-0050](./0050-at-level-5-a-pointer-is-non-null-unless-it-is-written-nullable.md), the first restriction.
