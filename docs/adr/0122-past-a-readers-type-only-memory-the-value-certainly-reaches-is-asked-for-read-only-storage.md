---
status: "accepted"
date: 2026-11-22
decision-makers: itsakeyfut
---

# Past a reader's type, only memory the value certainly reaches is asked for read-only storage

## Context and Problem Statement

ADR-0121 asks every level below a pointer that leaves, by its reader's type. Past the levels that type has, it walks on to the deepest pointer the unit declares, since a reader handed a `void *` or a `char *` may convert it back and write. That walk read each level as a load reads it. A load through a pointer whose memory this check cannot place adds everything code it cannot read may reach.

So `const char *G; void f(const char **out) { *out = G; }` was reported `SC0417` (#1145). `G` may point anywhere reachable, which includes the memory `out` arrived pointing at. That memory holds pointers to `const`. The `char` a `const char *` reader reaches was taken to hold one of them.

## Decision Drivers

* A level the reader's type has is the program's own claim. A level past it is this check's guess at a conversion back.
* A guess that hits every escaped block reports most programs that store a pointer read out of a global.

## Considered Options

* Past the reader's type, ask only memory the value certainly names. That is the sites and locals its points-to and the cells under them record, without what unplaced memory may hold.
* Stop where the reader's last level points at `const`.
* Keep the walk, and record #1145 as a cost.

## Decision Outcome

Chosen option: the first, by the owner's decision on #615's fourth design run. The second would also stop at a `const void *` reader that converts back and writes (row 6), which is the shape `bsearch` takes. The third leaves a false report on const-correct code (row 4).

* Levels the reader's type has are asked as ADR-0121 says, with what unplaced memory may hold.
* Levels past it are asked of what the value certainly names, level by level. Nothing a load through memory this check cannot place adds is counted there.
* ADR-0121's rule for levels past the reader's type is narrowed by this.
* **Memory a parameter or a call's result points at is no longer marked by its type.** The pointer itself takes its type's levels instead: a parameter at entry, as #1152 made it, and a call's result where the call writes it, which the review of #1145 found was needed for a result converted to `void *` without a cast. Every load through either pointer shifts them. The mark on a parameter's memory was found a level past a reader's type, by a pointer recorded as possibly naming that memory. ADR-0121's Confirmation names mutations of those two marks; what they guarded is held by these levels now.
* ADR-0121's other rules stand.

The cost is a laundering route whose value was read out of memory this check cannot place. A literal's address held in a global, read back and handed on as a `void *` to a function that converts it back and writes, is believed (row 6). The value certainly reaches nothing there.

### Confirmation

`a_pointer_read_out_of_a_global_stored_through_a_const_out_parameter_is_not_a_write` holds both halves:
* `stored_beside_a_table_handed_on` is reported where the walk past the reader's type counts what unplaced memory may hold;
* `stored_a_call_result` is reported where a parameter's memory is marked by its type at entry again.

`read_only_storage_below_what_leaves_or_arrives_is_unproven`'s `returned_as_void`, `copied_out`, `arrival_through_void` and `implicit_conversion_of_a_result` stay `SC0417`; the last goes silent where a call's result is not given its type's levels.

### Consequences

* Good, because a pointer read out of a global no longer stands for every escaped block below a reader that keeps its `const`.
* Bad, because of the laundering route above.

## More Information

ADR-0121, #615, #1145.
