---
status: "accepted"
date: 2026-09-28
decision-makers: project author
---

# A call is ordered after what its own arguments read, and a span strictly inside the call's says which reads those are

## Context and Problem Statement

C17 6.5.2.2 p10's first sentence puts a sequence point after a call's arguments
are evaluated and before the call, so everything its arguments read is ordered
before it. ADR-0026 expresses that point as `Element::ArgumentsEvaluated`, which
clears the whole carried set, and so emits it only where no unsequenced operator
encloses the call: below one, the set also holds the other operand's reads,
which C does not order. That record names what it leaves: `x = (free(p + *p),
0);` is still reported, with the note citing the clause that orders it.
Clearing only the reads this call's own arguments produced, it says, needs an
IR whose call carries its argument evaluations.

#270 made that residue common. It carries a pointer handed to a call forwards
as a pending read (ADR-0042), and a call nested in another call's argument is
the commonest shape in C: `if (strlen(strcpy(s, t)) > 0)`, `x = h(memset(a, 0,
4));`, `x = (free(memset(a, 0, 4)), 0);` and `b = realloc(memset(a, 0, 4), 8);`
each built on `main` and were refused on the branch that fixed #270, the first
with no free anywhere in it. The dereference spelling of the same residue,
`x = h(a + a[0]);`, was already refused on `main`.

## Decision Drivers

* A read C orders before a call must not be asked about at that call. That is
  a report about a program C defines, and the reader's only fix is to rewrite an
  ordinary expression.
* A read C leaves unordered against a call must still be asked. Excluding it is
  a use after free that builds, which is the worst answer this compiler can give.
* The two have to be told apart without an IR change this question is not worth.
  ADR-0026 turned down the IR whose call carries its arguments for that reason,
  and ADR-0022 turned down the expression-tree version of it.
* Whatever tells them apart is read by a future frontend. When that frontend is
  wrong, the failure has to be the one the reader sees.

## Considered Options

* A read whose span lies strictly inside the call's span is one of its arguments
* A marker the lowering emits where a call's arguments begin, and a stack in the
  lattice of the calls a read is inside
* Ask a carried argument only at a later `free`
* Leave it, and record the refusals as a cost

## Decision Outcome

Chosen option: **a read whose span lies strictly inside the call's span**,
because it answers the question with what the IR already carries. The answer
is checked where a carried read is asked, in `memory.rs::used_before`, and
nowhere else. The read is not removed from the set, because C orders it before
this call and before nothing else: `strlen(strcpy(s, t)) + (free(s), 0)` still
carries `strcpy`'s argument to the free.

**Why a span can say this.** In C source, a call's function designator and its
arguments are written within the call, and nothing else is, and 6.5.2.2 p10
orders both before the call; the designator is a name today and reads nothing.
The lowering gives each read the span of the element that performs it, and a
call's span covers its name and its parentheses. So a read inside the call's
arguments, including one inside a call nested there, has a span strictly inside
the call's, and a read in another operand has a span that is not: it lies
outside the call, or encloses it, or starts where the call does and reaches past
it, as the right operand of `h(x) || *a` does, whose read the lowering places at
the whole expression.

**Why strictly.** A span equal to the call's is not taken for an argument. The
only reads that carry exactly the call's span are the call's own operand reads
and what the call is handed, which `Allocations::terminator` records after this
call has been asked, so nothing is lost at this call. What strictness buys is
the direction of a future mistake. If macro expansion ever attributes a call and
a sibling operand to one span, which `Span`'s third coordinate has yet to
decide, the sibling is still asked and reported, rather than excluded and
silent.

**What it does not close.** A `,`, `&&`, `||` or `?:` below any node other
than one of those four gets no `Element::Sequenced`, which includes an
assignment, a `!`, an arithmetic operator and a call's argument list. So
`x = (memset(a, 0, 4) != 0) && h(a);`, `if (!((memset(a, 0, 4) != 0) && h(a)))`,
`h2(0, (memset(a, 0, 4) != 0) && h(a));` and `x = (memset(a, 0, 4), free(a), 0);`
are refused, as each one's dereference spelling is on `main`, while the same
operators at the root of a full expression build. The missing marker is the one #178 is about. #178 records the
proof its absence costs, and this is the report it costs; this record does not
reach either.

### Confirmation

Each mutation below was applied on its own to `crates/safec-ir/src/memory.rs`,
the whole workspace was run with `--no-fail-fast`, and the file was restored.
The cases are in `crates/safec/tests/cases` unless named otherwise.

* Dropping the `inside` test in `used_before` fails
  `a_call_nested_in_an_argument_is_ordered_before_the_call_around_it`,
  `a_free_of_what_a_nested_call_returns_is_ordered_after_it`,
  `a_read_in_an_argument_below_an_assignment_is_ordered_before_its_call` and
  `a_read_in_a_frees_own_argument_below_an_assignment_is_ordered_before_it`,
  which are refused again, and `a_nested_call_is_still_carried_to_a_free_beside_it`,
  which gains reports.
* Letting `inside` answer `true` for an equal span, by dropping its
  `inner != outer`, fails
  `a_read_with_the_span_of_a_later_free_is_still_carried_to_it` in
  `crates/safec-ir/tests/freed.rs` alone, which goes silent. No C program
  reaches an equal span, and that test is IR a frontend can build.
* Dropping `inner.end() <= outer.end()` from `inside` fails
  `a_read_reaching_past_a_later_free_is_still_carried_to_it` in
  `crates/safec-ir/tests/freed.rs` alone, which goes silent. The C lowering
  does make such spans, the right operand of `h(x) || *a` among them, but never
  has one pending at the call it starts inside, because that read runs after
  the call; IR can.
* Dropping the reads inside a call's span from `pending` at that call's
  transfer, as well as skipping them where `used_before` asks, fails
  `a_nested_call_is_still_carried_to_a_free_beside_it` alone, which loses its
  `SC0407` at `strcpy`. Dropping them there *instead of* skipping them fails
  the four nested-call cases as well, because `used_before` asks a call before
  its transfer runs.

Held by nothing: that `inside` compares the file. Every case is one file, and
two spans in two files cannot be nested by offsets that happen to agree unless a
case is written across two files.

### Consequences

* Good, because the nested-call idioms the carry refused build again. So does
  ADR-0026's recorded residue, `x = (free(p + *p), 0);`, which keeps its
  interior-free `SC0404` and loses the `SC0402`. So does `x = h(a + a[0]);`.
  Measured on a prototype first and then on the implementation, no other
  corpus case moved.
* Bad, because a span now carries a fact an analysis relies on, and not only a
  place to point. `docs/c-family.md` has to say it for the next frontend: a read
  that is part of a call's argument has a span strictly inside the call's, and a
  read that is not has no such span. The third coordinate `Span` is expected to
  grow has to keep that true.
* Bad, because a frontend that breaks the second half, giving a read in another
  operand a span inside a call's, makes a use after free build. Strictness
  covers the one way that is foreseeable, an equal span. It does not cover a
  frontend that hands out spans nested wrongly.
* Bad, because what #178's missing marker costs stays: a read and a call
  ordered by a `,`, `&&`, `||` or `?:` below any other node are still
  reported, and `if (!(p && q))` is an ordinary shape of that.
  With #270's carry that holds in the argument spelling as well as the
  dereference one.
* What would reverse this: an IR whose call carries its argument evaluations,
  where the point needs neither an element nor a span. That is the reversal
  ADR-0026 already names.

## Pros and Cons of the Options

### A read strictly inside the call's span

* Good, because it is one condition at the one place a carried read is asked,
  and it needs no change to the IR, the lowering or any other consumer.
* Good, because it closes the dereference half of ADR-0026's residue as well as
  the argument half.
* Bad, because it makes the span semantic, which is the obligation above.

### A marker where a call's arguments begin

* Good, because a frontend that omits the marker produces a report, not a
  silence.
* Bad, because it adds an element kind to every consumer of the IR. That is
  nine matches in three crates, by ADR-0026's count.
* Bad, because the lattice has to carry which calls a read is inside, and has to
  decide what a join does when two arms disagree about that. The dataflow here
  has already been caught hanging on a value that was not canonical (ADR-0016).

### Ask a carried argument only at a later `free`

* Good, because it undoes the refusals that have no free in them.
* Bad, because `x = (free(memset(a, 0, 4)), 0);` stays refused.
* Bad, because a pointer handed to `memset` and then to a call this check
  cannot read, or to `realloc`, builds in silence again, as it does on `main`.

### Leave it

* Bad, because `strlen(strcpy(s, t))` in a condition is refused, and the note
  cites the clause that orders it.

## More Information

* ADR-0026, whose residue this closes, and whose Consequences name the IR that
  would make both records unnecessary.
* ADR-0042 and #270, which made the residue common.
* #178, whose missing marker is what is left.
