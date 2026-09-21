---
status: "accepted"
date: 2026-09-19
decision-makers: itsakeyfut
---

# A proof survives while the set it is about does not grow

## Context and Problem Statement

`Held` holds what one local may point at: three may-facts and one proof.
[ADR-0020](./0020-a-free-of-a-may-set-is-a-fact-about-the-set.md) put the proof
there, `Held::freed`, saying that the set this local named had one member freed
and that freeing the local again takes the same member.

One method combined two `Held`s, and it was called with two different meanings.
In `Allocations::join` it is the lattice's join, where two paths meet. In the
arms of `Allocations::element` that build a value out of its operands it is an
accumulator, starting from `Held::none()` and unioning things in. The three
may-facts union correctly in both roles. The proof is joined by **intersection**,
which is right where two paths meet and clears the field every time it is
intersected against nothing: `Held::none()` is the identity for three of these
fields and the zero for the fourth.

So a local that had a proved free lost it by being written into an expression.
Measured on `main` at 9fc9c5f:

```console
$ cat d1.c
void *malloc(int n);
void free(void *p);
int f(int c) {
    int *p = malloc(4);
    if (c) { p = malloc(8); }
    free(p);
    p = p + 1;
    free(p);
    return 0;
}

$ safec --emit safety-ir --target x86_64-pc-windows-msvc d1.c
warning[SC0401]: this may free a value that was freed already
```

Without the `p = p + 1;` line the same program is `error[SC0401]`. The direction
is towards `Unknown`, so it costs a proof rather than a silence, and this record
is about giving it back without giving back more than that.

## Decision Drivers

* **The obvious accumulate is not sound**, which is the whole of why this needed
  deciding rather than repairing. See below.
* **Three more lattices are coming.** `docs/safety-model.md` has four axes and
  one is checked; each of the others will have a may half and a proof half and
  will meet this question on its first expression.
* **The compiler should be able to ask** which algebra a new fact belongs to,
  rather than the answer living only in prose.

## Considered Options

* **Two methods, and the proof survives while the set does not grow.**
* **Two methods, and the proof accumulates as a union of the present ones.**
* **Two methods, and the accumulator keeps clearing the proof**, so the split is
  a naming change and the behaviour stands.
* **One method, and the asymmetry is documented.**

## Decision Outcome

Chosen option: **two methods, and the proof survives while the set does not
grow**. `Held::union` becomes `Held::joined` and `Held::accumulated`, and every
caller says which it means.

| method | callers | what the proof does |
|---|---|---|
| `joined` | `Allocations::join` | intersection: both sides, or nothing. Unchanged |
| `accumulated` | the two `Rvalue::Binary` arms of `Allocations::element` | survives while the set does not grow |

```rust
// `Held::accumulated`
*freed = None;

// `built_from`, where the operands of a binary operation are folded
if let [one] = followed[..] {
    reached.freed = value.points_to[one].freed;
}
```

**The proof is carried by the fold and not by the combining method**, which is
where this record was first wrong. Written as a rule inside `accumulated` the
condition was "the set does not grow", and a review found that it cannot tell
the fold's empty seed from a real operand that holds no site. When this was
written the check did not read types, so a local holding nothing was an `int`
and a pointer whose allocation it lost at the same time, and
`int *slot = base + ok;` with `ok` an `int` and `base` a pointer this check
follows no allocation for was a **proved** use after free about an allocation
nothing had freed.

[ADR-0030](./0030-a-pointer-operand-decides-what-pointer-arithmetic-reaches.md)
has since made that shape answer differently: `ok` is an integer, C17 6.5.6 p8
keeps `base + ok` inside the object `base` points into, and so `ok` contributes
nothing and the expression has one contributing operand rather than two. **What
that record changes is which operands contributed, and not what happens when two
of them did.** Two contributing operands are two pointers, which is `q + r`, and
the reason the rule here still stands is unchanged: the result may be either of
them, and a proof about one of two sets is not a proof.

So the proof survives only where **nothing else contributed**: one contributing
operand and a constant, which the clause above keeps inside the same object.

**A union of the present ones invents a proof, and it is the reading a reader
arrives at first.** What `Held::freed` is worth is not that *some* member of the
set was freed; it is that freeing the local again takes the same member. A set
that has gained a site nothing freed no longer supports that. The shape is

```text
q = malloc(); if (c) { q = malloc(); }   two sites
free(q)                                  the proof: freeing q again takes the same member
p = q + r                                r is another pointer, holding a site nothing freed
free(p)                                  a proof this check cannot make
```

**Written as IR rather than as C, because it is not C.** `q + r` with two
pointers violates C17 6.5.6 p2, which requires one operand of an addition to
have integer type; `clang -std=c17 -pedantic-errors` answers `invalid operands
to binary expression ('int *' and 'int *')`, measured. The one conforming
spelling with two pointer operands is `q - r`, whose result is a `ptrdiff_t` and
which this frontend refuses for want of that type. So `p` may hold the one
nothing freed, and no C program reaches it. That report is a proof this check
cannot make. `CLAUDE.md` puts a false positive on row 4, not on row 6: the
reader can see it and `docs/safety-model.md` reserves row 6 for a silence. What
makes it worth a decision anyway is that `Unsafe` is what that document reserves
for something established, so a wrong one spends the word.

**There is no console transcript here and there used to be.** It was measured on
`int *p = q + i;` with `i` a parameter, which ADR-0030 has since made a program
with one contributing operand; and `q + r` is
`error[SC0304]: cannot compile an expression whose type is not known` in this
frontend, so the shape the rule is now about cannot be written in C here at all.
`an_offset_by_a_second_pointer_loses_the_proof` in
`crates/safec-ir/tests/freed.rs` is where it is measured instead.

**Symmetric, because it counts contributions rather than folding them.**
`p = i + q` and `p = q + i` are one expression and answer the same. Under
ADR-0030 that is one contribution each and both keep the proof; it was two each
and neither kept it, and what mattered then and now is that the two spellings
are never told apart.

**A field-wise algebra is not the answer, and that is why there is no trait.**
The proof's rule is not about the two values being combined at all; it is about
how many operands the expression had, which only the fold knows. A type per
field, each carrying its own `join` and `accumulate`, cannot see that. What the compiler holds instead is that both methods
destructure every field with no `..`, so a fact added to `Held` is
`error[E0027]` in each and has to say what it means in both. RK-018 in the
review knowledge bank is that spelling.

**The write through a pointer is neither, and drops the proof on its own line.**
`Allocations::element`'s `Deref` arm accumulates a written value into every
target the write may land in. Letting the rule above decide there would keep the
target's proof whenever the written value adds no site, which is every
unfollowable write: `free(p); *pp = 0; free(p);` would be proved, although that
write may have replaced the pointer and then the second free is not a second
free of anything. The sites are kept because keeping them is the conservative
direction for a use after free; the proof is dropped because keeping *it* is the
confident one.

### Confirmation

`error[E0027]` at `Held::joined`, at `Held::accumulated` and at the `Deref` arm
of `Allocations::element` if a field is added to `Held`, which is what asks a new
fact to say what it means in **each of the three roles**. The same addition is
`error[E0063]` at `Held::none` and `error[E0027]` at `Held::clear`, which
ADR-0018 already records.

The third of those is spelled as a destructure rather than as an assignment, and
it is worth saying why: written as `value.points_to[target].freed = None;` the
same addition asked four questions rather than five, and the site this record
names as the one the accumulator's rule is *wrong* for was the site that answered
none. Measured both ways.

Each mutation below applied on its own, the whole workspace suite run with
`--no-fail-fast`, the tree restored, and the failure read rather than predicted.
**Re-measured after ADR-0030 narrowed what counts as an operand here**, which is
what RK-066 in the review knowledge bank asks for: the second row used to be
held by two corpus cases and is held by a hand-built one now, because the
programs those cases were written from no longer reach the rule.

| Mutation | Named test that fails |
|---|---|
| `built_from` does not restore the proof for a sole operand | `a_free_after_an_offset_that_kept_the_set_is_proved`, `an_offset_by_an_integer_local_keeps_the_proof` and `an_offset_by_an_integer_parameter_keeps_the_proof`, each of which drops to a warning. The last two are the cases ADR-0030 moved onto this row |
| `built_from` restores the proof whenever any operand carries one | `an_offset_by_a_second_pointer_loses_the_proof` in `crates/safec-ir/tests/freed.rs`, which becomes a certainty about a value that may be the other operand's |
| `accumulated` does not union `lost` | `a_pointer_built_by_arithmetic_from_a_local_that_lost_its_allocation`, which goes silent. A free cannot hold it, because an argument reaching nothing answers `Reached::Lost` anyway; a dereference can, because that one says nothing about a place it follows no allocation for |
| `accumulated` does not union `sites` | eleven, across the corpus |
| `joined` accumulates the proof rather than intersecting it, at the method or at its one caller | `a_free_of_a_may_set_on_one_arm_only`, a proved error on a path that never freed |

The second row is the one that matters: it makes this compiler **certain** about
a program it cannot prove, which is what the first shape of this rule did and
what a review measured.

**Two things are held by nothing and this says which.**

The union of `writes_to` in `accumulated` is dead for both callers that build a
value out of operands, because each empties that field on the next line under
ADR-0019. Only the write through a pointer reads it.

**Dropping the proof at the write through a pointer is the third, and this
says so rather than implying otherwise.** Measured: removing that line leaves
the whole suite green. `Held::writes_to` is written only where an address is
taken, so a target of such a write has always escaped, and
[ADR-0017](./0017-record-each-half-of-an-escape-where-its-subject-lives.md)
answers `Reached::Lost` for an escaped local wherever a report is made, so the
answer is `Unknown` whatever the field says. Confirmed from the other end:
`int **pp = &p;` with no write at all already turns that program's proof into a
warning. The line is kept for the reason the `free` transfer keeps its own
unreachable projection guard: what it would cost if the escape stopped covering
it is a proof about a pointer the write may have replaced.

### Consequences

* Good, because a local that had a proved free keeps it through an expression
  that does not widen what it may hold, which is what the fact was about.
* Good, because the two algebras have two names, so the next lattice copies a
  shape that has already been asked which one it means.
* Bad, because the condition is cross-field, so `accumulated` reads `sites` to
  decide `freed` and cannot be split into anything smaller. A reader who wants
  the fields to be independent will find that they are not, and the measurement
  above is why.
* Bad, because one line of it is guarded by nothing and is kept on an argument
  about a rule in another record. If ADR-0017's escape answer changes, that line
  becomes load-bearing with no test underneath it.
* What would reverse this: a must-set beside the may-set. The whole difficulty
  is that `Held::sites` is a may-set, so "the same member" is the strongest
  thing a proof about it can say. A local that must point at one allocation
  needs none of this.

## Pros and Cons of the Options

### Two methods, and the proof survives while the set does not grow

* Good, because the failure when the condition is got wrong is losing a proof,
  which is row 4, while the alternative's failure is inventing one.
* Bad, because the condition is not a property of the field it governs.

### Two methods, and the proof accumulates as a union of the present ones

* Good, because it reads like the may-facts beside it and needs no cross-field
  condition.
* Bad, because it is measured to fabricate a proof: `free(q); p = q + i; free(p);`
  becomes a proved double free about a set that has grown by a site nothing
  freed. Row 4 rather than row 6, and still the word `Unsafe` spent on something
  nothing established.

### Two methods, and the accumulator keeps clearing the proof

* Good, because no behaviour changes and the split is pure shape.
* Bad, because it leaves `free(p); p = p + 1; free(p);` a suspicion for no
  reason anybody could state: nothing about that expression widened what `p`
  may hold.

### One method, and the asymmetry is documented

* Good, because it costs nothing today.
* Bad, because the thing to be got wrong is which of two algebras a new fact
  belongs to, and a doc comment is what the next lattice's author will not be
  asked to read. Three of them are coming.

## More Information

* `Held::joined` and `Held::accumulated` in
  [`crates/safec-ir/src/memory.rs`](../../crates/safec-ir/src/memory.rs), and
  the `Deref` arm of `Allocations::element` beside them.
* ADR-0020 is what put a proof in this struct; ADR-0018 is what the struct holds
  and why; ADR-0017 is the escape answer the unguarded line rests on;
  [ADR-0030](./0030-a-pointer-operand-decides-what-pointer-arithmetic-reaches.md)
  is which operands the fold counts, which this record takes as given.
* Issue #174 carries the measurement the body was filed with, and the note that
  ADR-0021 has since folded the program it used.
