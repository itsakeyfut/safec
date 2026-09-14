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
into every local the pointer may write to, and does nothing else to them. The
paragraph below on what it does not do is the one that was measured hardest.

**Doing nothing when the target is unknown is the decision, not an omission.**
`*pp = q` where no `&` was ever seen for `pp` leaves the silence exactly as it
was. The rule could obviously be widened to close it, the widening was written,
and what it cost is in the option below. A later reader who improves this in
good faith should read that first.

**A union, not a replacement.** A pointer that may point at one local is not a
pointer that must. Replacing erases a value nothing wrote over, which is a
silence rather than a false positive.

**The write unions and does nothing else.** #155's rule says what an escaped
local is *given* is no more proved than what it held, and aligning the two paths
by applying it here was the design until review measured what it cost.
`Known::unproved` writes on the **sites**, which are shared, so a write through
an alias reached into allocations other locals had already proved something
about: `free(p); *pp = 0; *p = 1;` went from a proved use after free to a
suspicion, and exit 1 to exit 0, with the write carrying nothing at all. Two
reviewers found it independently.

Nothing is lost by leaving it out, which is the part that makes this a decision
rather than a retreat. The target's address was taken, so ADR-0017 answers
`Reached::Lost` for it wherever a report is made, and `Known::settle` applies
the heap half over the merged value at every join. What the line was defending
against, a widened set quietening a local that reached nothing, is held by that
first answer and not by this one.

**The rvalue is read by an exhaustive `match`.** Every other reader of `Rvalue`
in this crate is, and what a missed arm would mean here is that a write silently
carries nothing, which is a silence rather than a build error.

**The edge stops at pointer arithmetic.** C17 6.5.6 p8 keeps the result of
`p + 1` inside the object `p` points into, which is why the *allocation* travels
through the arithmetic arm. A local's address plus one is not that local, so the
edge does not: `pp[1] = q;` is an out of bounds write, and following it reported
a proved use after free about an allocation nothing had freed.

An offset of zero never reaches this rule.
[ADR-0021](./0021-fold-a-zero-pointer-offset-where-the-ir-is-built.md) folds it
away when the IR is built, so `pp[0]` and `*pp` are one shape before anything
reads them. **The rule in this check is still about arithmetic and not about
which arithmetic**: every `Rvalue::Binary` drops the edge, a zero offset
included, because nothing here can tell one offset from another. What #172 found
is that the *reason* is about arithmetic that moves the pointer, and the layer
that can act on the difference is the one that builds the IR.

`Analysis::height` gains `locals * locals` for the second square table.

### Confirmation

Each mutation was applied to `crates/safec-ir/src/memory.rs` on its own, the
whole workspace suite run with `--no-fail-fast`, and the file restored.

| Mutation | Named test that fails |
|---|---|
| the `Deref` arm never fires | `a_free_through_a_pointer_that_reached_another_allocation`, which goes back to silent about its last line, and four cases beside it |
| `Rvalue::Address` records the bit and not the edge | the same five, because the target set is then always empty |
| the write replaces instead of unioning | `a_write_through_a_pointer_that_may_land_elsewhere_keeps_what_was_there`, where the pointer may land in either of two locals and one of them keeps an allocation the free then reaches |
| the write unproves the target, as a direct assignment does | `a_write_through_an_alias_leaves_a_sharer_s_proof_alone` and `a_write_through_an_alias_keeps_what_it_carried_proved`, whose proved reports drop to suspicions, and three cases whose diagnostics lose the `allocated here` label |
| a write this check cannot follow carries every allocation instead of none | `a_write_through_an_alias_that_carries_no_allocation` |
| a variant added to `Rvalue` | does not compile: `error[E0004]` here and at four other readers |
| the edge survives pointer arithmetic | `a_write_through_an_address_plus_one_is_not_a_write_to_the_local`, which gains a proved `error[SC0402]` about an allocation nothing freed |
| `Held::union` does not union the edge | `an_address_taken_on_one_arm_is_written_through_after_the_join` |
| a field added to `Held` | does not compile: `error[E0063]` in `Held::none` and `error[E0027]` in `Held::clear` and `Held::union` |

**`Analysis::height` is held by nothing here either.** Leaving the number where
it was leaves the whole suite passing, which is what ADR-0016 already says about
this method and what ADR-0018 says about the last field added.

### Consequences

* Good, because the headline silence is closed: the program in #162 is
  reported on its last line, where it used to say nothing at all. It was
  `error[SC0402]` and exit 1 when this record landed and is `warning[SC0402]`
  and exit 0 since
  [ADR-0020](./0020-a-free-of-a-may-set-is-a-fact-about-the-set.md), which is
  the last bullet of this section arriving: the proof rested on a free over a
  two-site may-set marking both members freed, and that rule was wrong.
  `--deny-unknown` still exits 1.
* Good, because two corpus programs that contain a real double free through an
  alias are now suspected at both frees rather than one.
* Bad, because `*pp = q` with no `&` in sight is still silent, deliberately.
  Nothing says so at the point of the write, which is why this record exists.
* Bad, because a local whose address was taken and which is then written
  through gets a warning on a read whether or not anything was freed.
  `a_pointer_written_through_an_alias_this_check_follows` is that program, and
  its old name said this check did not follow it. That answer comes from
  ADR-0017's rule for an escaped local, not from this one; what this record
  changed is that the local now reaches a site, so the rule applies to it.
* Bad, because a build that failed can now pass. Following more writes widens
  more may-sets, so a report that was a proof becomes a suspicion and the
  default run exits 0. Measured over four hundred generated programs: six moved
  from exit 1 to exit 0 and none moved the other way, and three of the proofs
  lost were about programs with no defect in them. `--deny-unknown` exits 1 on
  all of them.
* Bad, because the subscript spelling now costs what the dereference spelling
  costs. A file of loops writing through a pointer went from 4.5 s to 10.2 s at
  47 lines and from 75 s to 140 s at 87, which is the growth issue #173 is
  about arriving one spelling earlier rather than a new one.
* Bad, because the spelling still decides, for the spellings the fold does not
  reach. ADR-0021 owns that cost and measures it; it is not repeated here,
  because one fact written into two records is two facts to disagree.
* Bad, because the value doubled in size. It is two square tables of bytes now,
  `blocks * locals * (2 * locals + 49)`, which is 2.8 GB and six seconds on a
  489-line function against 1.5 GB and three before. A loop full of writes
  through pointers is worse than that, because the widening costs visits as
  well as bytes. Both are measured on issues rather than fixed here.
* This record meets ADR-0017's stated reversal condition, "a relation saying
  which local an alias may write to, rather than a bit saying that one exists",
  and deliberately does not take it: the bit and the edge answer different
  questions and both are kept. Narrowing the heap fact with the edge is a
  precision change with its own measurement burden and is not this.
* The headline proof leans on a rule #164 is open against. `free(p)` where `p`
  reaches two sites marks both `Freed`, which is how `*q = 1` is proved here.
  When #164 lands, this program stays reported and stops being proved, and
  three blessed expectations move with it. That is the correct direction and it
  should not read as a regression when it happens.
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
