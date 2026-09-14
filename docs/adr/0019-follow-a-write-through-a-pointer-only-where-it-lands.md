---
status: "accepted"
date: 2026-09-14
decision-makers: itsakeyfut
---

# Follow a write through a pointer where this check knows where it lands, and do nothing where it does not

## Context and Problem Statement

`int **pp = &p; *pp = q; free(p); *q = 1;` said nothing at all about the last
line. The write through `pp` is what made `p` hold `q`'s allocation, and it was
invisible, so the `free` was read against whatever `p` held before it and `q`'s
allocation stayed proved live. A use after free, at `--safety strict
--deny-unknown`, exit 0. That is the bottom row of `CLAUDE.md`'s list and it is
issue #162.

An attempt to close it went the other way and was withdrawn. Its rule was that a
`free` this check cannot follow could have freed anything an escaped local
holds, so every such site becomes unproved. `Known::escaped` is a bit rather
than an edge, so the rule reached a local no write could touch, and widening a
may-set that way made a **different** program go silent: a local reaching no
site answers `Reached::Lost` and is reported, and giving it a live site quietens
it.

What changed since is that
[ADR-0018](./0018-a-site-names-one-allocation-at-a-time.md) made a row of
`points_to` a type whose fields a new one has to answer for, and
[ADR-0017](./0017-record-each-half-of-an-escape-where-its-subject-lives.md) said
where a fact about a local belongs. The edge the withdrawn rule needed now has a
home the compiler polices.

## Decision Drivers

* Silence is the worst answer, and the first attempt traded one silence for
  another. Any rule here has to be checked in the quietening direction, not only
  the loud one.
* A write through a pointer is a **may** write. The pointer that names one local
  today names two after a join, and neither is certain.
* The other three axes will each meet this: a move through a pointer, a borrow
  through a pointer. The shape settled here is the one they will copy.

## Considered Options

* **Follow the write where the target is known, and do nothing otherwise.**
* **Taint instead of follow**: a `free` this check cannot follow makes every
  escaped local's sites unproved.
* **Follow, and replace rather than union** where the pointer names one local.

## Decision Outcome

Chosen option: **follow the write where the target is known, and do nothing
otherwise**.

`Held::writes_to` records, per local, which locals a write through it may land
in. `Rvalue::Address(taken)` writes the edge on the destination beside the bit
it already sets on `taken`; the two are different facts and both are needed. The
bit outlives everything done to the pointer that took the address, which is why
it stays on `Known`; the edge dies with an assignment to that pointer, which is
why ADR-0018's rule puts it in `Held`.

`Element::Assign` whose place is exactly one `Deref` unions the written value
into every local the pointer may write to, and marks each of them unproved.

**Doing nothing when the target is unknown is the decision, not an omission.**
`*pp = q` where no `&` was ever seen for `pp` leaves the silence exactly as it
was. The rule could obviously be widened to close it, the widening was written,
and what it cost is in the option below. A later reader who improves this in
good faith should read that first.

**A union, not a replacement.** A pointer that may point at one local is not a
pointer that must. Replacing erases a value nothing wrote over, which is a
silence rather than a false positive.

**Marking the target unproved is what keeps the widening from quietening.**
Every target is a local whose address was taken, so what it is given is unproved
by #155's rule, and the sites the union just added are unproved with it. Without
that the union can hand a live site to a local that reached none, and reaching
none is the loud answer.

**The edge stops at pointer arithmetic.** C17 6.5.6 p8 keeps the result of
`p + 1` inside the object `p` points into, which is why the *allocation* travels
through the arithmetic arm. A local's address plus one is not that local, so the
edge does not: `pp[1] = q;` is an out of bounds write, and following it reported
a proved use after free about an allocation nothing had freed.

`Analysis::height` gains `locals * locals` for the second square table.

### Confirmation

Each mutation was applied to `crates/safec-ir/src/memory.rs` on its own, the
whole workspace suite run with `--no-fail-fast`, and the file restored.

| Mutation | Named test that fails |
|---|---|
| the `Deref` arm never fires | `a_free_through_a_pointer_that_reached_another_allocation`, which goes back to silent about its last line, and four cases beside it |
| `Rvalue::Address` records the bit and not the edge | the same five, because the target set is then always empty |
| the write replaces instead of unioning | `a_write_through_a_pointer_that_may_land_elsewhere_keeps_what_was_there`, where the pointer may land in either of two locals and one of them keeps an allocation the free then reaches |
| the write leaves the target proved | `a_pointer_whose_address_escaped` and two others |
| the edge survives pointer arithmetic | `a_write_through_an_address_plus_one_is_not_a_write_to_the_local`, which gains a proved `error[SC0402]` about an allocation nothing freed |
| `Held::union` does not union the edge | `an_address_taken_on_one_arm_is_written_through_after_the_join` |
| a field added to `Held` | does not compile: `error[E0063]` in `Held::none` and `error[E0027]` in `Held::clear` and `Held::union` |

**`Analysis::height` is held by nothing here either.** Leaving the number where
it was leaves the whole suite passing, which is what ADR-0016 already says about
this method and what ADR-0018 says about the last field added.

### Consequences

* Good, because the headline silence is closed: the program in #162 is
  `error[SC0402]` and exit 1.
* Good, because two corpus programs that contain a real double free through an
  alias are now suspected at both frees rather than one.
* Bad, because `*pp = q` with no `&` in sight is still silent, deliberately.
  Nothing says so at the point of the write, which is why this record exists.
* Bad, because a write through a pointer makes the written-to local unprovable
  from that point, so a program that writes through an alias and then reads gets
  a warning whether or not anything was freed.
  `a_pointer_written_through_an_alias_this_check_follows` is that program, and
  its old name said this check did not follow it.
* What would reverse this: a `Deref` arm that knows which of several targets a
  write must land in. That is a must-analysis beside this may-analysis, and the
  measurement to take first is whether the precision is worth a second lattice.

## Pros and Cons of the Options

### Follow the write where the target is known

* Good, because it closes the silence with the rules that already exist: the
  free marks the site, the read reports it.
* Good, because the edge is recorded where an assignment destroys it, so a
  reassigned pointer stops being trusted without anybody remembering to say so.
* Bad, because the silence survives wherever the edge was never recorded, and a
  reader has to be told that on purpose.

### Taint instead of follow

* Good, because it needs no new field and no edge.
* Bad, because it is unsound in the quietening direction. `escaped` is a bit, so
  the rule reaches locals no write could, and a local given a live site it does
  not hold stops being reported. Written, measured, withdrawn; the programs are
  on issue #162.

### Follow, and replace rather than union

* Good, because it is more precise where the pointer names one local, and would
  let a write through an alias produce a proof rather than a suspicion.
* Bad, because naming one target is not writing to it. Two arms of a branch
  leave a pointer with one target each, and replacing on the merged value erases
  an allocation nothing wrote over. The failure is a silence.

## More Information

* `Held::writes_to`, `Known::written_through` and the `Deref` arm of
  `Allocations::element` in
  [`crates/safec-ir/src/memory.rs`](../../crates/safec-ir/src/memory.rs).
* [`docs/diagnostics.md`](../diagnostics.md) carries which half of the `SC0402`
  boundary this moved and which half stayed.
* Issue #162 has the withdrawn implementation and its measurements.
