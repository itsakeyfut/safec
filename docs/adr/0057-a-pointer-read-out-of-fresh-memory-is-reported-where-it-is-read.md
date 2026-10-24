---
status: "accepted"
date: 2026-10-24
decision-makers: itsakeyfut
---

# A pointer read out of fresh memory is reported where it is read

## Context and Problem Statement

[#581](https://github.com/itsakeyfut/safec/issues/581) made a pointer read out of an allocation `malloc` or `aligned_alloc` made, before anything wrote it, `SC0411`, and only at a use that reaches memory through it: a dereference, a `free`, a `realloc`. The reason it gave was that a load or a comparison is undefined only with a trap representation (C17 6.2.6.1 p5). Review found that is narrower than the text: 7.22.3.4 p2 and 7.22.3.1 p2 leave the contents indeterminate, and Annex J.2 lists "the value of the object allocated by the malloc function is used" as undefined, which covers the load and a comparison as well. So `int *t = *box; return t == q;` built in silence ([#587](https://github.com/itsakeyfut/safec/issues/587)), where `clang --analyze` warns at the load.

The memory check carries a mark, `Held::fresh_read`, from the load to every later use of the value, through copies between locals and through fresh memory it is stored into and read back from (#588), so that the dereference or the free can be reported. This record decides where the report stands, what it says, and what the mark is for once it no longer decides a report.

#581 pinned the comparison as silent, in the case that is now `a_pointer_read_out_of_fresh_memory_and_only_compared_is_reported`, inverted.

## Decision Drivers

* Silence is the conclusion this compiler must not give wrongly. J.2 lists the load, so a rule that waits for a later use is silent about every program whose only use is one it did not list.
* Level 1 already reports a pointer local read before anything was assigned to it, "whatever the value is read for", a plain copy included. The same class of value read out of memory should not need more to be reported.
* One defect, one report. A reader handed three diagnostics for one uninitialised slot reads none of them.

## Considered Options

* **Report the read out of memory itself**, every time a value is read out of a slot nothing wrote, whatever it is read for.
* **Report each use other than a plain copy**: a comparison, an argument, a `return`, a conversion, a dereference, a free.
* **Keep #581's rule**, a use that reaches memory, and write the narrower policy into the documents.

## Decision Outcome

Chosen option: **report the read out of memory itself.**

- **Where**: at every element or terminator that reads `*l` as a value, where one dereference of `l` reaches only allocations, every one of them fresh on every path, as `Held::fresh_read` asks today. That is an operand `Copy` of a place whose projection is exactly `Deref`, in any rvalue and in a call's arguments or a branch's condition; an `Evaluate` of such a place, since C17 6.3.2.1 p2 converts it; and any place, read or written, whose projection begins with two dereferences, since reaching through `*l` reads it. A write to `*l` is not a read of it.
- **As what**: `Unsafe`, `SC0411`, at the element's span. Fresh is an intersection over paths, so at that point the read is certain. The note keeps citing 7.22.3.1 p2, 7.22.3.4 p2 and Annex J.2.
- **Nothing more is said at a later use of the value.** A dereference, a `free` or a `realloc` of it is not reported as `SC0411` again, because the read that made it was.
- **A store of the value back into fresh memory writes the slot**, as any store does. #588 kept such a slot fresh, so that reading it back was reported; with the report at the first read that is a second report of one defect, and it reported the copy the lowering takes of an assignment's value (C17 6.5.16 p3) as "read before anything wrote it" at the store that wrote it. Found while implementing.
- **The mark stays, and only suppresses.** `Held::fresh_read`, its propagation through copies, and the `escaped` clause of `Known::reads_indeterminate` remain. They decide no report. They decide one suppression: a doubt this check would raise at a `free` or `realloc` of the value, because it knows no allocation the value names, is dropped where the value is known to have been read out of fresh memory and so was reported at that read. Only a doubt is dropped, never a proof, and only for the local the free is handed.
- **A site that may have been freed is no longer fresh.** Reading it is a use after free, which `SC0402` reports; calling it indeterminate as well is a second report for one read.

Reporting each use other than a copy was rejected because its failure is silence: every position the list misses is a use J.2 makes undefined that builds, and an argument is one it cannot reach without a summary of what the callee does ([#596](https://github.com/itsakeyfut/safec/issues/596)). Keeping #581's rule was rejected for the same failure at a known place, the comparison. The chosen option's failure is a report about a program that reads a slot and throws the value away, which the reader can see and fix by writing the slot first.

Keeping #588's rule for a store, with a second per-site bit saying the slot holds a value already reported, was rejected: it would carry the suppression through the store as well, at the price of a lattice field whose error drops a doubt that should stand, which is a silence. Without it, freeing a pointer read back out of such a slot raises a doubt beside the report, which the reader can see.

Dropping the mark along with the reports it used to make was rejected because a free of a value read out of fresh memory then raises a second diagnostic, a doubt saying this check knows no allocation the value names, beside the proof at the read. The mark is the only thing that knows the two are one defect.

### Confirmation

Each in `crates/safec/tests/cases/memory/` unless named otherwise, with the mutation that fails it in its comment in `crates/safec/tests/cases.rs`:

* **The read is reported, whatever it is read for.** `a_pointer_read_out_of_fresh_memory_and_only_compared_is_reported` and `a_pointer_read_out_of_fresh_memory_and_discarded_is_reported`: drop the call to `loaded_indeterminate` for an element, and both build. `..._in_a_comparison_is_reported`, `..._and_handed_to_a_call_is_reported`, `..._by_an_expression_statement_is_reported`, `..._through_two_dereferences_is_reported` and `..._by_a_condition_is_reported` each fail when the matching arm of `read_out_of_memory_in_element` or `read_out_of_memory_in_terminator` reads nothing. `a_pointer_written_into_fresh_memory_is_not_reported` fails when a write through one dereference is read as a read.
* **Not again at a later use.** `a_pointer_read_out_of_fresh_memory_read_through_and_freed_is_reported_once`: report a read through a local `Known::reads_indeterminate` marks, and `SC0411` joins at the read through.
* **The suppression at a free.** `..._and_freed_is_reported_once` fails when the suppression is dropped, `..._and_reallocated_is_reported_once` when it is asked of `free` alone, and `a_copy_of_a_pointer_read_out_of_fresh_memory_freed_is_reported_once` when a plain copy clears the mark.
* **What the suppression must not drop.** `..._a_call_may_have_replaced_is_still_doubted_where_it_is_freed` and `..._a_call_replaced_is_still_doubted_where_a_copy_is_freed` fail when the `escaped` clause is dropped, and `..._on_one_arm_only_is_still_doubted_where_it_is_freed` when the mark is joined with `||`.
* **A store writes the slot.** `..._stored_and_read_back_is_reported_once`: keep the slot fresh for a store of an indeterminate pointer, and `SC0411` joins at the store and at the read back.
* **A freed site is not fresh.** `returns/a_pointer_read_out_of_a_freed_table_and_returned`: drop the clearing in the transfer of a `free`, and `SC0411` joins its `SC0402`.

### Consequences

* Bad, because a pointer read back out of a slot it was stored into, and then freed, raises a doubt beside the report at the first read.
* Good, because the first use J.2 lists is the one reported, so no later use can be silent for want of being listed, and the program in #596 is reported at its load without a callee summary.
* Good, because the rule matches level 1's for an unassigned local, and `clang --analyze`'s caret.
* Bad, because a program that reads an unwritten slot and discards the value is refused, which C17's normative text makes undefined only where the bits read are a trap representation (6.2.6.1 p5). Annex J.2, which is informative, is the reading taken here.
* Bad, because the mark is kept for a suppression alone, which looks redundant beside a report that no longer reads it. This record is why it is not.
* What would reverse this: a target or a reading of C17 under which reading an indeterminate pointer is defined, which would move the report back to the uses that reach memory.

## More Information

* [`docs/safety-model.md`](../safety-model.md), where an indeterminate pointer is level 1's.
* [`docs/diagnostics.md`](../diagnostics.md), whose `SC0411` paragraph describes #581's rule and is rewritten by the change that implements this one.
* [ADR-0040](./0040-a-pointer-read-out-of-memory-reaches-what-was-stored-and-a-parameter-is-exposed-where-the-function-starts.md), what a pointer read out of memory holds.
