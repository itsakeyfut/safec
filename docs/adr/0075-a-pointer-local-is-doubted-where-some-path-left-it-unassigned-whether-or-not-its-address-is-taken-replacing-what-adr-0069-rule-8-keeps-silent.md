---
status: "accepted"
date: 2026-11-07
decision-makers: itsakeyfut
---

# A pointer local is doubted where some path left it unassigned, whether or not its address is taken, replacing what ADR-0069 rule 8 keeps silent

## Context and Problem Statement

A pointer local read where some path to the read left it unassigned gets two
answers today, chosen by whether its address is taken anywhere in the function.
A local whose address is taken is asked by the memory check's `Known::init`,
and a forward join with one unwritten side is `Init::Maybe`, an `SC0411` doubt
(ADR-0069 rule 5). A local whose address is never taken is asked by
`memory::assigned`, which reports a read no path assigned and, of the reads
some path assigned, only one after a branch edge that certainly leads to it
unassigned. Everything else is silent, by the decision ADR-0069 rule 8 records
(#582).

So one defect builds silent or not by a line that does not touch it:

```c
void use(int *p);
int f(int c, int d, int * _Nonnull q) {
    int *p;
    if (c) p = q;
    if (d) use(p);
    return 0;
}
```

prints nothing and exits 0; with `int **pp = &p;` added it is `SC0411` "`p`
may be read before anything was assigned to it". With `c` zero and `d`
non-zero, C17 6.3.2.1 p2 leaves the read undefined.

## Decision Drivers

* A silence about a program C leaves undefined is the failure this compiler
  ranks worst.
* One defect should not be answered two ways by a fact about the program that
  does not decide it.
* What a doubt costs is visible: the reader sees the report and can judge it.

## Considered Options

* One rule, the doubting one: a local whose address is never taken is doubted
  where `Known::init` would doubt it if its address were taken.
* Two rules, kept, with ADR-0069 rule 8's reason.
* One rule, the silent one: a local whose address is taken reported only after
  an edge that certainly leads to the read unassigned.

## Decision Outcome

1. **A pointer local whose address is never taken is doubted where it is
   `Init::Maybe` at the read**, by `report::unwritten_read`, as a local whose
   address is taken is: `SC0411`, `Unknown`, `Kind::IndeterminateLocal`. So the
   joins, the loops shown to run (ADR-0069 rules 5 and 6) and the first turn of
   a loop (rule 7) answer both kinds of local by one implementation.
2. **`memory::assigned` keeps what it says first**: a read no path assigned is
   `Kind::Unassigned`, `Unsafe`, and a read after a branch edge that certainly
   leads to it unassigned is `Kind::SometimesUnassigned`, `Unknown`, with the
   edge. `unwritten_read` asks a local whose address is never taken only where
   neither said anything at that caret, so no read is reported twice.
3. **What this replaces**: ADR-0069 rule 8's "a read on an arm is not asked
   so, and stays silent where the memory check doubts it". The rest of rule 8,
   the first-turn proof and the doubt at a loop's exit edge, stands.

### Confirmation

Each in `tests/cases/memory/`, with its mutation in its note in
`tests/cases.rs`:

* The program above is
  `a_pointer_assigned_on_one_arm_and_read_under_another_test_is_doubted`,
  `SC0411` "may be read". Returning `report::unwritten_read` to asking only a
  local whose address is taken makes it silent, and the seven below.
* Asking a local whose address is never taken where `memory::assigned`
  already reported at the caret reports the read twice:
  `a_pointer_assigned_on_one_arm_is_doubted_where_the_other_arm_leads_to_its_read`
  fails, and seven more.

### Consequences

* Good, because the program above and every read some path left unassigned
  are doubted, whatever happens to the local's address.
* Bad, because seven correct programs in the corpus are now doubted, each an
  idiom #582 kept silent and a local whose address is taken is already doubted
  for: a pointer assigned beside a flag and read under it (two cases), a
  pointer assigned and read under one parameter tested twice, assigned on both
  arms of a test and its negation, read in a loop only after its first turn,
  and a pointer whose unassigned path ends in `abort` or in `exit`. Each is a
  false `SC0411` the reader sees, measured on a prototype of exactly this rule
  (since removed); no other case and no unit test moved.
* What would reverse it: a check that follows the condition a path took, which
  would answer the seven for both kinds of local at once.

## Pros and Cons of the Options

### Two rules, kept

* Good, because no case moves.
* Bad, because the program above stays silent, and whether a defect is
  reported turns on whether `&p` appears anywhere in the function.

### One rule, the silent one

* Good, because the seven idioms are silent for both kinds of local.
* Bad, because every read some path left unassigned behind a second test is
  silent for both kinds, a silence added where one is now reported.

## More Information

* [ADR-0069](./0069-a-pointer-read-out-of-memory-nothing-wrote-is-reported-where-it-is-read-and-a-loop-may-run-zero-times-unless-it-is-shown-to-run.md),
  whose rule 8 this replaces in part.
* #582, the decision rule 8 recorded; #730, where it was reopened.
