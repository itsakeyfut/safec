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
*freed = match (*freed, other.freed) {
    (Some(here), Some(there)) if !grows && !shrinks => Some(here.joined(there)),
    (Some(here), _) if !grows => Some(here),
    (_, Some(there)) if !shrinks => Some(there),
    _ => None,
};
```

**A union of the present ones invents a proof, and it is the reading a reader
arrives at first.** What `Held::freed` is worth is not that *some* member of the
set was freed; it is that freeing the local again takes the same member. A set
that has gained a site nothing freed no longer supports that. Measured, with the
accumulator taking the proof from whichever side has one:

```console
$ cat d4.c
void *malloc(int n);
void free(void *p);
int f(int c, int i) {
    int *q = malloc(4);
    if (c) { q = malloc(8); }
    free(q);
    int *p = q + i;
    free(p);
    return 0;
}

$ safec ... d4.c          # main, and this decision
warning[SC0401]: this may free a value that was freed already
$ safec ... d4.c          # the union of the present ones
error[SC0401]: this frees a value that was freed already
```

`i` is a parameter and a parameter is a site, so the accumulator reaches `i`'s
site as well as `q`'s two. `p` may hold the one nothing freed. That report is a
proof this check cannot make, which is row 6 of `CLAUDE.md`'s list, and it is
the reason the rule reads `sites` at all.

**Symmetric, and read before the union.** `grows` and `shrinks` are computed
before anything is merged, because afterwards every site is `self`'s and the
answer is that nothing ever grows. Both directions are asked so that
`p = i + q` and `p = q + i` are one expression; `shrinks` is also what carries
the seed, since a value built from `Held::none()` holds no site at all, so the
first operand's set is the whole of the result so far and its proof is still
about it.

**A field-wise algebra cannot be the answer, and that is why there is no trait.**
The rule reads one field to decide another. A type per field, each carrying its
own `join` and `accumulate`, is the shape this looked like it wanted and it
cannot see across fields. What the compiler holds instead is that both methods
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

`error[E0027]` at `Held::joined` and `Held::accumulated` if a field is added to
`Held`, which is what asks a new fact to say what it means in each role. The
same addition is `error[E0063]` at `Held::none` and `error[E0027]` at
`Held::clear`, which ADR-0018 already records.

Each mutation below applied on its own, the whole workspace suite run with
`--no-fail-fast`, the tree restored, and the failure read rather than predicted.

| Mutation | Named test that fails |
|---|---|
| `accumulated` joins the proof the way `joined` does | `a_free_after_an_offset_that_kept_the_set_is_proved`, which drops to a warning |
| `accumulated` takes the proof from whichever side has one | `an_offset_that_grew_the_set_is_not_proved`, which becomes a proof about a set that grew |
| `accumulated` reads `grows` and `shrinks` after the union rather than before | the same, because afterwards nothing ever grows |
| `joined` accumulates the proof rather than intersecting it | `a_free_of_a_may_set_on_one_arm_only`, a proved error on a path that never freed |

The second and third rows are the ones that matter: each makes this compiler
**certain** about a program it cannot prove.

**Dropping the proof at the write through a pointer is held by nothing, and this
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
* Bad, because it is measured row 6: `free(q); p = q + i; free(p);` becomes a
  proved double free about a set that has grown by a site nothing freed.

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
  and why; ADR-0017 is the escape answer the unguarded line rests on.
* Issue #174 carries the measurement the body was filed with, and the note that
  ADR-0021 has since folded the program it used.
