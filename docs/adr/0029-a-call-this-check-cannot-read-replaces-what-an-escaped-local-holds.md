---
status: "accepted"
date: 2026-09-21
decision-makers: itsakeyfut
---

# A call this check cannot read replaces what an escaped local holds, and a free of it proves nothing

## Context and Problem Statement

```c
int *p = malloc(4);
int *r = p;
stash(&p);
other();
free(p);
*r = 1;
```

`stash` may keep `&p` and `other` may write a fresh pointer through it, so
`free(p)` need not free what `r` holds and `*r = 1` need not touch anything
freed. The program is defined on that execution, and it was `error[SC0402]` at
exit 1, which `Conclusion::Unsafe` makes an error at every safety level with no
flag to suppress it. That is issue #200.

[ADR-0017](./0017-record-each-half-of-an-escape-where-its-subject-lives.md)
answers `Reached::Lost` beside an escaped local's sites, which stops a report
**about that local**. It does not reach this program, because what is reported
here is `r`, which never escaped. The transfer folds the argument's answer
through `named`, which keeps the sites and drops everything else, and writes
`SiteState::Freed` on them. A site is shared, so the proof the escape had
withheld from `p` is handed to every other local holding `p`'s allocation.

[ADR-0028](./0028-replace-what-a-target-held-where-a-write-must-land-in-it.md)
widened it. A write through a pointer whose target is certain now replaces what
that target held, so the free is handed a set of one where it used to be handed
two and [ADR-0020](./0020-a-free-of-a-may-set-is-a-fact-about-the-set.md)
refused to prove over it.

## Decision Drivers

* An escaped local's contents are stale only once something can have written
  through the address. Inside this function, a write through a pointer is
  followed, which is ADR-0019 and ADR-0028. Outside it, the writer is a call,
  and this check reads no callee's body.
* The sites are shared and the local's own answer is not. A fact recorded
  against the local is invisible to the sharer that gets reported, which is
  what RK-048 is about from the other side.
* A free of a local this check has stopped following cannot say which
  allocation went. What it writes on a site is a claim about every holder of
  that site.
* `docs/safety-model.md` ranks a silence below a false positive. Whatever this
  does, a use after free of the same shape has to stay reported.

## Considered Options

* Set `Held::lost` on every escaped local at a call this check cannot read, and
  have a free of a local carrying it write `SiteState::Unknown` on the sites it
  reached rather than `SiteState::Freed`.
* Have a free write `Unknown` whenever anything it reached was lost, with no
  new producer, which is the same rule keyed on the fold `Reached::Lost`
  already produces.
* Clear an escaped local's row at the call, so that it reaches no site at all.
* Call `Known::unproved` on the target of a certain write, taking back what
  ADR-0028 recovered.

## Decision Outcome

Chosen option: the first. A callee writing through a stashed address leaves the
local holding an allocation this check cannot name, which is exactly what
`Held::lost` already says for the other door ADR-0018 gave it, so the fact needs
no field of its own. The rule that reads it lives in the `Callee::Frees` arm of
the transfer, above the two rules that say which member of a set went, because
neither applies once the set is not known to be what was freed.

`Callee::Frees` and `Callee::Allocates` do not produce it. C17 7.22.3.3 and
7.22.3.4 say what those two do and neither writes through an address a caller
stashed earlier, which is the whole reason this check reads a callee's name.

### Confirmation

`a_call_this_check_cannot_read_may_have_replaced_what_an_escaped_local_holds`
and `a_certain_write_does_not_survive_a_call_this_check_cannot_read` in the
corpus are the two programs, each reported and at exit 0. Dropping the
`Known::replaced` call from the `Callee::Opaque` arm, or letting `Callee::Frees`
write `Freed` although the argument's local is `lost`, takes both back to
`error[SC0402]` at exit 1.

Every case that observes the rule does so through a **second local sharing the
allocation**, because the escape has already taken away the report about the
escaped local itself: a case that frees or reads through that local alone
observes nothing whatever this decision says. RK-048 is that hole, found after
ADR-0017 made four guards vacuous at once.

`a_free_through_an_escaped_local_is_seen_by_a_sharer` is what holds the rule to
calls: a free refusing to prove whenever anything was lost, rather than when the
argument's own row says so, drops that proved double free to a warning.
`an_opaque_call_before_the_escape_leaves_the_proof_alone` is the same rule from
the other end, a call that ran before the address escaped, and it fails for any
implementation that reads the escape without reading where it happened. RK-065
is a guard that held only the order it was written in.

`a_double_free_through_a_sharer_after_an_opaque_call_is_not_proved` holds what
this costs rather than what it fixes, so that the cost cannot be taken away
without a case moving.
`a_use_after_free_through_an_escaped_local_is_still_reported_after_an_opaque_call`
holds the direction that is worse than the cost: clearing the local's row at the
call instead leaves that program saying nothing about the read at all, measured,
and a silence is the bottom of `CLAUDE.md`'s list.

### Consequences

* Good, because a program C defines stops being an error no flag suppresses,
  which is what `docs/concept.md` promises anyone adopting this compiler
  incrementally.
* Good, because the fact is one the lattice already carried, so there is no
  second slot for a later reader to forget to test.
* Bad, because a real double free or use after free through a sharer, where the
  opaque call did nothing, is a warning rather than an error. It is still
  reported, and `--deny-unknown` still fails the build on it. What would recover
  it is knowing what a callee does to what it is passed, which is #134.
* Bad, because a dereference of a pointer whose own address escaped and which
  has survived such a call now reports `this uses a pointer this check stopped
  following`. It is true, and it is a report where there was none.
* What would reverse this: an annotation saying a callee writes through nothing,
  which is the phase's last issue. Until then the callee's body is not read and
  the conservative answer is the only one available.

## Pros and Cons of the Options

### Set `lost` at the call, and read it at the free

* Good, because it says where the doubt comes from: the call, not the escape.
  A local whose address escaped and which no call has run past is still
  followed, and the proofs that rest on that survive.
* Good, because it reuses the fact and the field that already mean this.
* Bad, because the bit is read where a report is made as well as where a free
  is, so it changes two answers and not one.

### Key the free on the `Reached::Lost` the fold already produces

* Good, because it adds nothing at all: the fold says the pointer was lost and
  the free stops proving.
* Bad, because ADR-0017 produces that same answer for every escaped local,
  including one nothing can have written to yet. Measured: it drops
  `a_free_through_an_escaped_local_is_seen_by_a_sharer` from a proved double
  free to a warning. The two facts are not interchangeable and this option
  spends the second to buy the first.

### Clear the escaped local's row at the call

* Good, because it is the strongest statement of what the callee may have done.
* Bad, because a local reaching no site says nothing at all, which is RK-049.
  Measured on `stash(&p); other(); free(p); *p = 1;`: the `SC0402` goes, and
  what is left is a warning about the free. A silence is the bottom row of
  `CLAUDE.md`'s list while a false positive is row 4.

### Take back what ADR-0028 recovered

* Good, because the second program in #200 is one ADR-0028 made worse.
* Bad, because it removes a proof rather than a false one, and it answers
  nothing about the first program, which ADR-0028 never touched.

## More Information

* Issue #200, which carries both programs and the measurements on `main`.
* Issue #202, the same false proof through a write this check cannot follow,
  which this does not close: telling a write into an `int` apart from a write
  into a pointer needs #143.
* `Known::replaced` and the `Callee::Frees` arm of `Allocations::terminator` in
  `crates/safec-ir/src/memory.rs`.
