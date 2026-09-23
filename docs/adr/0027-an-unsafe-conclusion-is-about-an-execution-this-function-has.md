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
#188 built. That arm is wider than the clause and says so: it skips every
constant, `free(17)` included, which is #154's to answer rather than this
rule's, so what #188 extended is the principle here and not that arm's reach.

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
keep it off that row and an implementation of the seam has to hold all four,
which is why they are written here rather than left to be rediscovered. The
fourth was not here when the seam was first built, and an implementation
holding the other three is what found it:

* the nullness is read through the mask that answers `Unknown` for a local
  whose address escaped, which is `Nullability::known` and ADR-0017's shape on
  the other axis. A raw read of the lattice loses it;
* the nullness is asked at the free rather than for the function, because
  `int *p = 0; p = q; free(p); free(p);` is null at one of those points and not
  the other. Neither `Allocations::touching` nor `Analysis::terminator` is
  given a position today, so this is work rather than a lookup;
* the nullness is one C has ordered before the free. This lattice has no notion
  of order and says so, while the check that reads its answer is built on one:
  ADR-0022 and ADR-0023 exist because the lowering's order inside a full
  expression is not C's. `int x = (p = 0, 1) + (free(p), 0);` writes the null in
  an operand C leaves unsequenced against the free's read of the same pointer,
  which C17 6.5 p2 makes undefined outright rather than undefined on one of the
  orders; exempting it is this compiler going silent about a program C does not
  define. What says whether C ordered it is `Element::ArgumentsEvaluated`, which
  ADR-0026 emits only where no unsequenced operator encloses the call. **What
  that marker answers is narrower than the condition**, and the Consequences say
  what the difference costs;
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

**The exemption is held by three corpus cases**, one per condition above, and
each of them is exit 1 becoming exit 0 under its own mutation, which is the
direction that matters. `a_free_of_a_pointer_proved_null` is the rule itself:
dropping the filter in `memory.rs::reported` puts `this frees a pointer this
check stopped following` back on a program C defines, and fails it along with
`a_pointer_set_to_nothing_after_a_free_holds_nothing` and
`a_local_given_nothing_forgets_the_set_it_freed`, which are the two spellings
of `free(p); p = 0; free(p);` the corpus keeps.
`a_pointer_proved_null_before_its_address_escaped_is_not_exempt` holds the
mask: reading the lattice rather than `Nullability::known` in
`nullability.rs::null_at_terminators` exempts both frees of a local a store
this check cannot follow has given an allocation, and that case goes silent.
`a_pointer_that_stopped_being_null_before_the_free_is_not_exempt` holds the
position: recording the row before a block's elements rather than after exempts
a **proved** double free, and that case goes silent too. The position mutation
is not discriminating: it takes the three exemption cases down with it, because
a row read before the block's elements is not the row any of them needs either.
The mask mutation is the one that fails a single case.

A fourth case holds something the three conditions above do not say and an
implementation has to know anyway: the nullness is about a pointer.
`Nullability::nullness_of` calls a constant zero null without asking what it is
assigned to, which costs nothing where the answer is read about a dereference
and costs a diagnostic when it is read to exempt a free, so `int x = 0;
free(x);` went silent. `a_free_of_an_int_that_holds_zero_is_not_exempt` is the
case, and dropping the `is_pointer` call in `nullability.rs::null_at_terminators`
is the mutation that fails it. C17 6.3.2.3 p3 is why: a null pointer constant is
an integer constant expression converted to a pointer type, and an `int` lvalue
holding zero is neither.

`a_free_in_an_unsequenced_operand_is_not_exempt` and
`a_free_in_an_unsequenced_operand_across_a_call_is_not_exempt` hold the order,
and it takes both: dropping the `ordered` term in `null_at_terminators` fails
the two of them together, because the first reaches the term through a block
whose last element is an ordinary operation and the second through a block with
no elements at all, which a call between the two operands produces. The
narrower mutation, answering `true` for that empty block, fails the second
alone.

**Every row of this table was re-measured after that term was added**, because
narrowing a rule makes a table that was true before it false in silence. All of
them still fail what they name, and no row was found to have stopped reaching
what it is about.

**No row here says how many tests it breaks, and that is deliberate.** RK-028
in the review knowledge bank is a count of exactly this kind: it is a fact about
a suite that grows every week, and this record carried one for a fortnight
before three separate measurements of the same mutation answered 22, 23 and 24.
A row names the case it is about, and where a mutation takes neighbours down
with it the row says which neighbours and why, which is the part a later reader
can check.

The report-not-the-transfer condition is held by where the exemption is called
from rather than by a case. `memory.rs::asked` is reached only from the walk that reports;
`Analysis::terminator` has no access to the nullness, so a free of a pointer
established null still marks its sites freed and still leaves the local holding
them. What that costs is in the Consequences below.

### Consequences

* Good, because every proof this check makes survives, `a_value_freed_twice`
  included, and the milestone's headline double free stays an error.
* Good, because the one exemption is the one C names, and it is already written
  for a constant argument, so there is one rule rather than two.
* Bad, because `safec` rejects a program that is well defined as written, and a
  reader who checks the whole translation unit can see that the only caller
  passes a null pointer.
* Bad, because the exemption is applied to the report and not to the transfer,
  so a free of a pointer established null still marks the sites it reaches
  freed. On the arm of `if (p == 0)` that freed nothing, the sites are freed
  all the same, and a later use of one of them is reported. That is a false
  positive on row 4, taken deliberately: the alternative is a transfer that
  believes a nullness, and a nullness this compiler is wrong about would then
  be a real free nobody recorded, which is the bottom row.
* Bad, for the same reason, in the other reader of what a call frees.
  `memory.rs::used_before` carries a read forwards to a `free` it is unordered
  against and asks `Allocations::touching` without the filter, so such a read
  is reported unsequenced against a free that does nothing. It takes a branch
  that establishes the null to reach at all, and it is row 4 again. #219 is
  where that is written down.
* Bad, because the marker the fourth condition reads answers a narrower question
  than the condition states. `Element::ArgumentsEvaluated` is emitted where no
  unsequenced operator encloses the *call*, so the exemption reaches a free at
  the root of its full expression and no other, whatever C has ordered. A null
  established in an earlier full expression, which C17 6.8 p4 sequences
  unconditionally, buys nothing if the free sits under an unsequenced operator:
  `p = 0; y = (free(p), 0);` and `p = 0; y = g(0) + (free(p), 0);` are both well
  defined, are exit 0 under `clang -target x86_64-pc-windows-msvc -std=c17
  -pedantic-errors -Wall -fsyntax-only`, and are `error[SC0401]` here. So is
  `int x = (p = 0, free(p), 1) + 2;`, where a comma has ordered the assignment
  before the free inside the operand and the enclosing `+` is what removes the
  marker. All three are row 4. Asking the question per local, rather than
  zeroing the row for the block, would close the class and needs the ordering to
  cross a block edge, which is a lattice dimension; no program in the corpus
  asks for it.
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
* Issues #137, where the measurements above were made, and #188, which built the
  seam this record's exemption is applied through.
