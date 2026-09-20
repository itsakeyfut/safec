---
status: "accepted"
date: 2026-09-20
decision-makers: itsakeyfut
---

# An `Unsafe` conclusion is about an execution this function has, and only a pointer established null exempts a free

## Context and Problem Statement

C17 7.22.3.3 p2 says that if the argument to `free` is a null pointer, no action
occurs. So `int f(int *p) { free(p); free(p); return 0; }` is well defined for
the caller that passes a null pointer, and the memory check calls it a proved
double free at every safety level. The same clause exempts
`int *p = malloc(4); free(p); free(p);` on the execution where the allocation
failed, because C17 7.22.3.4 p3
lets `malloc` return a null pointer, and `crates/safec-ir/src/nullability.rs`
answers `Unknown` for a `malloc` result for exactly that reason.

What has to be decided is what an `Unsafe` conclusion asserts: that every
execution is undefined, or that one is. Nothing in
[`docs/safety-model.md`](../safety-model.md) said, and the two answers report
these two programs differently.

## Decision Drivers

* The clause does not distinguish the two programs, so a rule that exempts one
  and keeps the other has to justify the line on something other than C.
* `docs/safety-model.md` puts the compiler's worst failure at saying safe about
  something it did not prove; turning proofs into warnings is not that failure,
  but it is how much of this check's reporting survives.
* This check reads one function at a time. Which callers a translation unit
  happens to contain is not something it asks, and asking would be a different
  analysis rather than a stricter one.

## Considered Options

* **Only a pointer this check established is null exempts a free.** Nothing else
  weakens a proof.
* **An `Unsafe` conclusion requires the freed pointer to be known not null.**
* **A site whose allocation this check did not see loses the proof**, and one it
  watched keeps it.

## Decision Outcome

Chosen option: **only a pointer this check established is null exempts a free**.

An `Unsafe` conclusion from the memory check means *there is an execution of this
function that is undefined*. The check is intraprocedural, so a parameter ranges
over every value a caller may pass rather than over the calls this translation
unit contains: `int f(int *p) { free(p); free(p); return 0; }` is a proved
double free however `f` is called here, and stays one in the program whose
`main` passes `0`.
A pointer the analysis established *is* null is the one thing that removes a free
from the question, because then no execution frees anything. That is the rule
`Allocations::touching` already applies to `free(0)` with this clause quoted,
and extending it to a local the nullability analysis proved null is the seam
#188 builds. That arm is wider than the clause and says so: it skips every
constant, `free(17)` included, which is #154's to answer rather than this
rule's, so what #188 extends is the principle here and not that arm's reach.
Until #188 lands, such a free is reported as a pointer this check stopped
following, which is #188's subject and not this record's.

The second option was rejected on a measurement rather than on taste: because a
`malloc` result is `Nullness::Unknown`, it turns `a_value_freed_twice` into a
warning along with `a_parameter_freed_twice`, leaving this check able to prove a
double free only where a guard has already tested the pointer. The third draws a
line between a null a caller supplies and a null a failed allocation supplies,
which is a distinction the clause does not make.

What this costs is a false positive on a program `clang` accepts, measured with
`clang -target x86_64-pc-windows-msvc -std=c17 -pedantic-errors -Wall
-fsyntax-only`, which exits 0 on the corpus case named below. That is row 4 of
`CLAUDE.md`'s failure list: this compiler reports something wrong and the reader
can see it. The options above move nothing off row 6 in exchange, because neither
of them makes this check quiet about anything: what they turn a proof into is a
warning that `--deny-unknown` turns back into an error.

**The exemption is the half of this rule that can go quiet, and that is where
it lands on `CLAUDE.md`'s list.** Everything else here only refuses to weaken a
proof, which leaves a false positive on row 4. A free that is exempted is
reported by nobody, so a `Nullness::Null` this compiler is wrong about is a
real double free nothing says anything about, which is row 6. Three things
keep it off that row and an implementation of the seam has to hold all three,
which is why they are written here rather than left to be rediscovered:

* the nullness is read through the mask that answers `Unknown` for a local
  whose address escaped, which is `Nullability::known` and ADR-0017's shape on
  the other axis. A raw read of the lattice loses it;
* the nullness is asked at the free rather than for the function, because
  `int *p = 0; p = q; free(p); free(p);` is null at one of those points and not
  the other. Neither `Allocations::touching` nor `Analysis::terminator` is
  given a position today, so this is work rather than a lookup;
* the exemption skips the report, rather than clearing the local's sites. A
  cleared local reaches no site, and reaching no site is how this check spells
  having lost a pointer, which is a warning about the wrong thing.

### Confirmation

`a_double_free_in_a_function_the_only_caller_passes_null_to_is_proved` in
`crates/safec/tests/cases`, whose `main` passes `0` to the function that frees
its parameter twice, and whose `.stderr` holds the error. The mutation is the
third option, written as requiring `made.is_some()` in the `Unsafe` arm of
`verdict` in `crates/safec-ir/src/memory.rs`: measured, that case fails along
with `a_parameter_freed_twice` and every other case whose proof stands on a site
this check did not watch being allocated.

**No mutation fails that case alone**, and the record says so rather than
implying otherwise. What it holds over `a_parameter_freed_twice` is the caller,
in view, passing a null pointer; the change that would fail it on its own is a
rule that reads call sites, which is interprocedural and exists in no phase of
[`docs/roadmap.md`](../roadmap.md). Until one does, the case's job is that
anybody who narrows this rule has to rewrite an expectation whose name states
what is being given up, and finds this record from there.

The first option is held by nothing today, because nothing in
`crates/safec-ir/src/memory.rs` reads `crates/safec-ir/src/nullability.rs`: the
seam does not exist yet. #188 is where it is built, and the case that will hold
this half is the one that issue names.

### Consequences

* Good, because every proof this check makes survives, `a_value_freed_twice`
  included, and the milestone's headline double free stays an error.
* Good, because the one exemption is the one C names, and it is already written
  for a constant argument, so there is one rule rather than two.
* Bad, because `safec` rejects a program that is well defined as written, and a
  reader who checks the whole translation unit can see that the only caller
  passes a null pointer.
* What would reverse this: an interprocedural phase that can say what a function
  is called with. At that point a parameter stops ranging over every value and
  the quantifier this record fixes is the wrong one, and the case named above is
  the expectation that has to be rewritten.

## Pros and Cons of the Options

### Only a pointer established null exempts a free

* Good, because the exemption follows the clause rather than the source of the
  null, so `free(0)`, a local assigned `0` and a pointer proved null on a branch
  are one rule.
* Good, because nothing this check proves today becomes weaker.
* Bad, because it keeps a false positive that a reader with the whole unit in
  front of them can refute.

### `Unsafe` requires the pointer to be known not null

* Good, because then every error this check reports is undefined on every
  execution, which is the strongest thing an error could mean.
* Bad, because `malloc` returns a pointer that is not known to be anything, so
  the programs this check exists to report become warnings: measured,
  `int *p = malloc(4); free(p); free(p);` reports `error[SC0401]` today, and its
  pointer is `Nullness::Unknown`.
* Bad, because what would restore those errors is an assumption that `malloc`
  succeeded, which is the compiler naming a safety it has not established.

### A site this check did not watch being allocated loses the proof

* Good, because it removes exactly the false positive this record is about, and
  `a_value_freed_twice` keeps its error.
* Bad, because the line is drawn where this check's knowledge stops rather than
  where C's rule does, and C exempts the failed allocation identically.
* Bad, because it is measured to take `a_parameter_freed_twice` and the cases
  whose proof stands on a site with no allocation span with it, and a parameter
  freed twice is the commonest double free there is.

## More Information

* [`docs/safety-model.md`](../safety-model.md), *Safe, Unsafe, Unknown*, which
  this record is the reasoning behind.
* `Allocations::touching` and `verdict` in `crates/safec-ir/src/memory.rs`, and
  `Nullness` in `crates/safec-ir/src/nullability.rs`.
* [ADR-0020](./0020-a-free-of-a-may-set-is-a-fact-about-the-set.md), which is the
  other rule about when a free is a proof, and answers a different question: what
  a set of sites says, rather than what an execution is.
* Issues #137, where the measurements above were made, and #188, which builds the
  seam this record's exemption is applied through.
