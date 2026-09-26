---
status: "proposed"
date: 2026-09-26
decision-makers: itsakeyfut
---

# An allocation code this check cannot read may reach is exposed for good, and a call this check cannot read may return any exposed allocation

## Context and Problem Statement

`Callee::Opaque` in `crates/safec-ir/src/memory.rs` is what a call this check
cannot read does to what it knows. [ADR-0029](./0029-a-call-this-check-cannot-read-replaces-what-an-escaped-local-holds.md)
made it assume the worst of the allocations its arguments name and of the
locals whose address escaped, and took the call's result as a fresh
allocation. [#250](https://github.com/itsakeyfut/safec/issues/250) measured two
programs where that lets a use after free build, which is the bottom row of
`CLAUDE.md`'s list.

A first design closed those two by marking the routes one at a time. Review
found four more routes it left open, each measured to exit 0 and two run to a
read of freed memory: a pointer stored into a stack slot through a pointer, an
address as the argument, a pointer handed to an earlier call and returned by a
later one, and a call in a loop whose result may be last turn's freed
allocation. It also refused `realloc` and `memset` idioms `main` builds. That
design was withdrawn before merging; #250's comments carry what it found.

## Decision Drivers

* `CLAUDE.md`'s ranking. Every option leaves something at row 4; the ones that
  leave something at row 6 say so here.
* Measured against two sets rather than the corpus alone, because the corpus
  holds no library call that returns a pointer (RK-080 in the review knowledge
  bank): seven programs with a defect for some definition of the callees C
  permits, and eight well-defined ones using the C library the way real code
  does. The prototype and the tables are on #250's `/spec` comment.
* C17 7.1.3 reserves the library's names, which is why `malloc` and `free` are
  read by name at all.

## Considered Options

The thing a call reaches:

* one mark per allocation, *exposed*, set once code this check cannot read may
  reach a pointer to it, and never cleared while the allocation lives
* marking each route to a callee separately (the withdrawn design)

What a call may return:

* a fresh allocation, or any exposed one
* a fresh allocation, or what this call's own arguments reach
* a fresh allocation only (`main`'s rule)

A pointer stored through a pointer:

* recorded as the contents of the allocation it was stored into, and exposed
  when that allocation is
* exposed at the store

## Decision Outcome

Chosen: **an exposed mark per allocation, a call's result that may be any
exposed allocation, and a record of what each allocation may contain.**

**Exposed.** An allocation is exposed once a pointer to it may be reached by
code this check cannot read: named by an argument of an opaque call, held by a
local an argument points at, held by a local whose address escaped, contained
in an exposed allocation, or stored through a pointer this check cannot say
where it lands. The mark is a union at a join and survives everything but a
rebirth of the site. At every opaque call the call's reach is added to it,
closed over contents, and every exposed allocation still live becomes
unproven. A proved free stays proved. This covers all four routes review found
and both of #250's programs, and it costs one corpus case its wording and
nothing else.

**What a call returns.** A fresh allocation, or any exposed one, at an offset
nobody said. That catches a pointer handed to one call and returned by
another, which is ordinary registry code. It costs `show(p); q = make(); *q`:
`q` may be `p`, and is unproven. Returning only what the call's own arguments
reach keeps that program and leaves the registry silent; asked, and the row-6
direction was ranked below.

**A site reborn from an opaque call that was exposed stays unproven.** In a loop
the call writes the same site every turn, and what it returns may be last
turn's allocation, which the site number cannot tell from the new one.

**Contents.** A write through a pointer that holds sites records what it
carries as the contents of those allocations; one this check cannot place
exposes what it carries at once. So `*tab = p; log_line(); *p` builds: `tab` was
never reachable by `log_line`, so neither is `p`. Exposing at the store instead
refused that program. The record is a square table beside the two `Held`
already has, which is the condition `Held::sites` names for a packed bitset:
measured on a 456-line function, peak memory went from 485 MB to 711 MB and time
from 0.49 s to 0.71 s. The bitset is left to
[#173](https://github.com/itsakeyfut/safec/issues/173), and the number is
written where the table is.

**The library, by name.** `realloc` makes its first argument unproven rather
than freed, since C17 7.22.3.5 p4 leaves the old object alone when it fails, and
returns a fresh allocation. `memset`, `memcpy`, `memmove`, `strcpy`,
`strncpy`, `strcat` and `strncat` free nothing and return their first argument,
which C17 7.24.2 to 7.24.6 say of each; what they are handed is exposed, because
they copy bytes and a pointer is bytes. Measured, this is what builds
`memset(malloc(4), 0, 4)` and its siblings, which `main` refused already.

**What is believed, and written down.** A callee that returns an allocation it
made and freed itself is a fresh allocation to this rule, and silent: #252.
`realloc`'s failure branch, `if (q == 0) free(p);`, is unproven because nothing
ties the result's nullness to the argument: [#253](https://github.com/itsakeyfut/safec/issues/253).

### Confirmation

**Nothing guards this yet; the record is `proposed`.** #250 lands it and
rewrites this section with what it measured. Each program in the two probe sets
becomes a corpus case, and the mutation for each rule has to fail a named one:
not setting the mark for each of its five sources, not closing over contents,
not unproving at the call, unproving a proved free, dropping the mark at a join
or keeping it through a rebirth, the result holding only its fresh site or only
what its arguments reach, a reborn exposed site proved live, each library
function read as opaque, and `realloc` proving its argument freed.

### Consequences

* Good, because the six silences #250 and its review measured are reported,
  and the library idioms `main` refused build.
* Bad, because an allocation handed to any call this check cannot read makes
  every later call's result doubtful: row 4, and the one a user will meet most.
* Bad, because the memory check costs about half again in memory and time,
  until #173.
* Bad, because a callee returning what it freed itself is believed (#252), and
  `realloc`'s failure branch is refused (#253).
* What would reverse this: summaries of functions this translation unit
  defines, which would let a call's result be what its body returns rather than
  anything exposed.

## Pros and Cons of the Options

### One exposed mark

* Good, because every route a pointer takes to unread code ends in the same
  place, so a route nobody listed is not a silence.
* Bad, because it never forgets.

### Marking routes one at a time

* Good, because it costs least.
* Bad, because review found four routes it missed.

### Result: any exposed allocation

* Good, because registry code is reported.
* Bad, because every call after an exposure returns something doubtful.

### Result: what the arguments reach

* Good, because an unrelated call's result stays proved.
* Bad, because a registry's use after free builds.

### Result: fresh only

* Good, because it is `main`.
* Bad, because a call returning its argument is silent.

### Contents recorded

* Good, because a table nothing unread can reach does not expose what it holds.
* Bad, because it is a third square table.

### Exposed at the store

* Good, because it needs no table.
* Bad, because it refuses a pointer stored in a table followed by any call.

## More Information

* [#250](https://github.com/itsakeyfut/safec/issues/250), whose comments carry
  the withdrawn design, what its review found, and the probe tables.
* [ADR-0029](./0029-a-call-this-check-cannot-read-replaces-what-an-escaped-local-holds.md),
  which this extends from the locals an opaque call reaches to the allocations.
* [ADR-0038](./0038-a-hatch-is-a-function-definition-and-what-it-could-not-prove-is-listed-rather-than-reported.md),
  whose wider rule for a hatch stays.
* RK-080 in the review knowledge bank, on why the corpus alone could not cost
  this.
