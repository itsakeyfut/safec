---
status: "accepted"
date: 2026-09-20
decision-makers: itsakeyfut
---

# Replace what a target held where a write through a pointer must land in it, and say when the edge is all of it

## Context and Problem Statement

```c
int *p = malloc(4);
int *q = malloc(8);
int **pp = &p;
*pp = q;
free(p);
*q = 1;
```

`pp` names one local and the write is unconditional, so `p` **is** `q`
afterwards and `*q = 1` is a certain use after free.
[ADR-0019](./0019-follow-a-write-through-a-pointer-only-where-it-lands.md)
unions the written value into every local the pointer may reach, so `p` came out
of that line holding both allocations.
[ADR-0020](./0020-a-free-of-a-may-set-is-a-fact-about-the-set.md) then read that
two-element set as real ambiguity and gave up the proof over it, which is the
last bullet of its Consequences naming this program by name. It was
`error[SC0402]` and exit 1 when ADR-0019 landed and has been `warning[SC0402]`
and exit 0 since. That is issue #175.

ADR-0019 considered replacing rather than unioning and rejected it, on the
ground that naming one target is not writing to it: two arms of a branch leave a
pointer with one target each, and replacing on the merged value erases a value
nothing wrote over. That reason is still correct. What has changed is that the
reason is no longer about this rule.

## Decision Drivers

* `Held::writes_to` is a may-set, and one member in it means "at most one target
  this check has seen an address for". An empty set is what a pointer this check
  never followed has, so a union of an empty set with one member is one member,
  and the two readings were the same value. That is the third-emptiness shape
  RK-045 is about, one field over.
* Erasing a value nothing wrote over is a silence, which is the bottom row of
  `CLAUDE.md`'s list. Any rule here has to be checked in that direction first.
* Three more axes will each meet this. A move through a pointer and a borrow
  through a pointer ask the same question about the same edge.

## Considered Options

* **Say whether the edge is all of it, and replace where it is.**
* **Replace wherever the edge names one local**, without saying whether it is
  all of it.
* **A must-point-to lattice beside the may-point-to one**, which is what
  ADR-0019 named as the thing that would reverse it.

## Decision Outcome

Chosen option: **say whether the edge is all of it, and replace where it is**.

`Held::writes_elsewhere` is one `bool` per local: whether a write through this
local may land somewhere `writes_to` does not name. It starts `true`, and
`Rvalue::Address` is the only thing that clears it. The `Deref` arm of
`Allocations::element` replaces the target's whole row, rather than
accumulating into it, when the flag is clear and the set names one local.

**This is the must-analysis ADR-0019 asked for, at one bit.** A full must-set
buys nothing over it. Where two arms give a pointer two different targets,
neither a must-set nor this flag can replace, because the union already names
two. Where two arms give it the *same* target, the union names one and the flag
is clear on both arms, so the replacement fires and a must-set would agree.
What the flag adds over `writes_to` alone is the one case a set cannot carry:
a path that knew the target meeting a path that did not.

**It replaces the whole row**, as an assignment to the target does: the sites,
what the local had lost, the proof about the set it named, and the edge alike.
`*pp = q` gives `p` what `q` holds in the same sense that `p = q` does, `freed`
included.

**It replaces for every kind of rvalue, including the ones this check cannot
follow.** Where the target is certain, `*pp = 0;` leaves `p` holding nothing,
because the write happened and what was there is gone. Keeping the union for
the constant, unary, address and projected-read arms was written and measured:
it recovers the headline proof just as well and costs two things. `p` keeps an
allocation the program demonstrably overwrote, so a later read of it is a
suspicion about a defect that is not there. And "a certain write" would then
mean two things depending on the rvalue, inside the one arm RK-052 already
caught answering two ways.

**It does not call `Known::unproved` on the target**, which is the one thing
the direct assignment does that this does not. `unproved` writes on the
*sites*, which are shared between locals, and ADR-0019 measured that doing it
here turned `free(p); *pp = 0; *p = 1;` from a proof into a suspicion.
Certainty about which local was written says nothing about who else holds that
allocation.

**A local whose own address is taken gives up the flag.** Once something holds
`pp`'s address, anybody may put a different pointer in it, so a write through
`pp` may land where this check cannot see. `opaque(&pp)` never reaches the
`Deref` arm, so nothing else in this check says so, and without that line
`int **pp = &p; opaque(&pp); *pp = q; free(p); *q = 1;` was a proved use after
free about a program with no defect. It is RK-061's rule arriving for this
lattice: a new answer that proves something positive about a local has to
answer for that local's address escaping.

**The join is a union and the transfer is not monotone**, which is the shape
`writes_to` already has and which `Analysis::height`'s second paragraph already
answers for.

`Analysis::height` gains `locals`.

### Confirmation

Each mutation applied on its own to `crates/safec-ir/src/memory.rs`, the whole
workspace suite run with `--no-fail-fast`, the file restored from a copy rather
than from `git`, because the branch had uncommitted work.

| Mutation | Named test that fails |
|---|---|
| union always, as before | `a_free_through_a_pointer_that_reached_another_allocation`, `a_subscript_write_is_the_write_it_is_defined_as`, `a_write_through_a_pointer_minus_zero` and `a_write_through_a_pointer_plus_zero_on_the_left`, which are one program in four spellings and drop back to `warning[SC0402]` and exit 0, and the three cases whose pointer is replaced rather than widened |
| `Rvalue::Address` does not clear the flag on the destination | the same seven, because the replacement then never fires at all |
| replace wherever the set names one local, ignoring the flag | `a_write_through_a_pointer_with_one_target_on_one_arm_only` and `an_address_taken_on_one_arm_is_written_through_after_the_join`, which gain an `error[SC0402]` about a program that frees each allocation once, and `a_write_through_a_pointer_whose_own_address_escaped` |
| `Held::joined` intersects the flag rather than unioning it | the first two of those, by the same programs: a path that knows the target meeting a path that does not would answer that it knows |
| `Rvalue::Address` does not set the flag on the local whose address is taken | `a_write_through_a_pointer_whose_own_address_escaped`, and nothing else. That case exists for this row: before it was written the whole suite stayed green while the program became a proved use after free |
| a field added to `Held` | does not compile: `error[E0063]` in `Held::none` and `error[E0027]` in `Held::clear`, `Held::joined` and `Held::accumulated` |

**Four of the answers are held by nothing, and one reason covers all four.**
`Held::none` seeding `true`, `Held::clear` restoring it, and the two lines
beside `writes_to.fill(false)` in the `Rvalue::Binary` readers each leave the
whole suite passing when they are mutated or removed. The flag is read only
where `writes_to` is not empty, and every one of those four empties `writes_to`
in the same breath, so the `targets.is_empty()` return above answers before the
flag is reached. They are written anyway because the alternative is a value
that says something false about itself, and because the next reader of this
struct will copy whichever shape is there. The two `Rvalue::Binary` lines are
also RK-052's rule: one question asked in two places is answered in both or it
drifts inside the change that touches one of them.

`Analysis::height` is held by nothing either, as ADR-0016 says of the method and
as ADR-0018, ADR-0019 and ADR-0020 say of the last three fields added.

### Consequences

* Good, because the program issue #162 was filed about is a proved
  `error[SC0402]` and exit 1 again, with all three of the labels
  `docs/safety-model.md` asks for: allocated here, freed here, used here.
* Good, because the four spellings of it agree. ADR-0021 folded a zero offset
  where the IR is built so that `*pp`, `pp[0]`, `*(pp - 0)` and `*(0 + pp)` are
  one shape, and this is the first change whose answer they all move on
  together.
* Good, because three programs that replace a pointer through an alias stop
  being told they may be freeing something twice. `free(p); *pp = q; *p = 1;`
  said `may use a value after it was freed` about a pointer this check had
  watched being replaced.
* Bad, because those same three now say `this check stopped following` instead.
  That is ADR-0017's answer for a local whose address escaped, and it is honest
  but it is not the whole truth: `p` holds a fresh allocation, or null, and this
  check can see which. #188 owns the null half and #196 owns a report that names
  no free. Nothing here changes ADR-0017.
* Bad, because a pointer whose own address escaped can never reach the
  replacement, however plainly the program reads. `int **pp = &p; opaque(&pp);
  *pp = q;` is followed exactly as it was, by the union, and the proof this
  record recovers everywhere else is not available there. That is the flag
  doing its job rather than a cost that can be removed: nothing in this check
  knows what `opaque` put in `pp`.
* What would reverse this: a reason to believe `writes_to` can name one local
  without the write landing there. The three ways it could are a join, an
  arithmetic result and an escape, and each is answered by a line with a named
  test above.

## Pros and Cons of the Options

### Say whether the edge is all of it, and replace where it is

* Good, because it separates the two readings of a one-member set, which is the
  whole of what was wrong.
* Good, because it costs one `bool` per local. `size_of::<Held>()` is 72 before
  and after: the field lands in padding, so the measured peak does not move.
* Bad, because four of its seven answers are held by nothing, for the reason
  above, and a later reader has to be told that on purpose.

### Replace wherever the edge names one local

* Good, because it is one condition and no new field.
* Bad, because it is a false proof. Measured: `int **pp = other(); if (c) { pp = &p; } *pp = q; free(p); *q = 1;` becomes `error[SC0402]` on a program that frees each allocation once, and so does the same shape with `opaque(&pp)` in place of the branch. Both are C defines on every path.

### A must-point-to lattice beside the may-point-to one

* Good, because it is the general answer and ADR-0019 named it.
* Bad, because it answers nothing the flag does not. The cases it would decide
  differently are the ones where the union already names two targets, and
  neither rule may replace there.
* Bad, because it is a second square table on a value ADR-0019 already measured
  at 2.8 GB on a 489-line function, which is issue #173.

## More Information

* `Held::writes_elsewhere`, the `Rvalue::Address` arm and the `Deref` arm of
  `Allocations::element` in
  [`crates/safec-ir/src/memory.rs`](../../crates/safec-ir/src/memory.rs).
* [`docs/diagnostics.md`](../diagnostics.md) carries what a reader of `SC0402`
  is told about a write through a pointer.
* ADR-0019 has the measurement that made the union right when it was taken, and
  ADR-0020 has the rule that turned the manufactured set into a lost proof.
