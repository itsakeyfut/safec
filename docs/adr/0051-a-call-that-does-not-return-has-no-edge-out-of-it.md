---
status: "accepted"
date: 2026-10-05
decision-makers: itsakeyfut
---

# A call that does not return has no edge out of it

## Context and Problem Statement

`Terminator::Call` always names a block to continue in, so the graph says every call returns. C says some do not, among them `abort` "does not return to its caller" (C17 7.22.4.1), and `exit`, `_Exit` and `quick_exit` "cannot return to" it (7.22.4.4, 7.22.4.5, 7.22.4.7). Every analysis therefore walks the code after such a call as reachable, and the nullability check tells a function that promised its result and ends with `abort();` that it may reach the end of its body (`SC0408`) ([#341](https://github.com/itsakeyfut/safec/issues/341)). At level 5 every function defined here that returns a pointer has made that promise (ADR-0050), so the pattern is refused wherever it is written.

The graph's edges are an interface every analysis is written against (ADR-0010), so how the IR says a call does not return is decided here rather than inside the change that first needs it.

## Decision Drivers

* An edge the program never takes, kept in the graph, is a false report the reader can see. An edge it does take, dropped from the graph, is code no analysis asks about, which is the failure this project exists to avoid.
* `Cfg::of` and `dataflow::solve` read one function and its terminators, and nothing else.
* Whatever represents it is something the Clang adapter will produce too, from a callee `clang` already knows does not return.

## Considered Options

* **The call's continuation is optional**: `then: Option<BlockId>`, `None` where the call does not return.
* **A terminator of its own** for a call that does not return.
* **A fact on the callee**, `Function` saying it does not return, read wherever the graph is built.

## Decision Outcome

Chosen option: **the call's continuation is optional.** `Terminator::successors` names no edge for a `Call` whose `then` is `None`, so the graph, the solver and every analysis read the code after it as reached by no execution without a line of them changing, and every place that reads a continuation is a type error until it says what it does without one. A terminator of its own was rejected because each analysis would answer for a call twice, what it reads and what it exposes in both, and two copies of one rule drift. A fact on the callee was rejected because the graph would have to look the callee up, which it cannot from one function.

What decides it is the name, for now: a call is made with no continuation when its callee is `abort`, `exit`, `_Exit` or `quick_exit`, this translation unit does not define it, and the call is the whole of its full expression. C17 7.1.3 reserves those identifiers with external linkage, so a program that defines one so has no behaviour C defines, and one defined here is lowered as any other function with its continuation kept: its body is what the checks read, and believing it never returns would be believing a name over the code beside it. The memory check reads library functions by name already (ADR-0039).

- Only the whole of a full expression, because inside a larger one the lowering reads a place operand where its operator is lowered, after the call: `*p + (exit(1), 0)` would put the read of `*p` where nothing reaches it, though C17 6.5 p3 lets it happen first and clang 20.1.6 does it first. Such a call keeps its edge, and the cost is a report about code that may not run.
- A definition in another file of the same run is not seen, since each file is lowered as its own unit, and the call is believed by its name. The program has no behaviour C defines, and nothing reports that either: [#343](https://github.com/itsakeyfut/safec/issues/343) is collecting the names a whole run defines.
- This is a hosted implementation's reading. A freestanding one (C17 4 p6) need not provide these functions, and clang 20.1.6 stops treating `abort`, `exit` and `_Exit` as not returning under `-ffreestanding` or `-fno-builtin`, measured. This compiler has no such mode.

- `_Noreturn` (C17 6.7.4) is not read. Believing it of a declaration would be a promise written down and believed, which this compiler checks where it can: a function defined `_Noreturn` that returns would need refusing first. It is a decision of its own.
- The backend writes `unreachable` after a call with no continuation, which is what C says of the point after it.

### Confirmation

Every reader of a call's continuation is `error[E0308]` until it answers for `None`, which is how the interpreter, the printer and the backend came to say what they do without one. The cases are in `crates/safec/tests/cases`, and the mutations in `crates/safec/src/lowering.rs` unless named.

- The lowering giving every call its continuation fails `a_function_that_ends_in_abort_does_not_reach_the_end`, `each_function_that_cannot_return_ends_a_body` and `code_after_abort_is_not_asked`, which are refused.
- Dropping any one name from `DOES_NOT_RETURN` fails `each_function_that_cannot_return_ends_a_body`, at that name's function.
- `Lowering::does_not_return` ignoring `defined` fails `code_after_a_function_defined_here_named_abort_is_asked` and `code_after_a_function_defined_below_named_abort_is_asked`, which go silent, and asking the IR whether the callee has a body fails the second.
- Dropping the test that the call is the root of its full expression fails `a_read_written_before_a_call_that_does_not_return_inside_an_expression_is_asked`, which goes silent.
- `Terminator::successors` pushing a block for `None` fails `every_terminator_says_where_control_can_go` in `crates/safec-ir/src/ir/tests.rs`.
- The backend writing nothing after a call with no continuation fails `llvm_ir_of_a_call_that_does_not_return` and, where `clang` is present, `tests/llvm.rs`, which hands it the module.

### Consequences

* Good, because a body that ends in `abort();` or `exit(1);` is no longer told it may reach its end, and code after such a call is no longer asked about.
* Good, because the graph says it, so every analysis, the interpreter and the backend agree without each knowing a name.
* Bad, because a function of the program's own that never returns, `die()` written around `exit`, still has a continuation, and code after a call to it is still asked about: a false report the reader can see, until `_Noreturn` is read.
* Bad, because `quick_exit` is treated as not returning where `clang` 20.1.6 does not: measured, `clang -Wreturn-type` is silent after `abort()`, `exit(1)` and `_Exit(1)` and warns after `quick_exit(1)`. C17 7.22.4.7 is what this follows.
* What would reverse this: an IR that has to say why a call does not return, or how it leaves instead, which `None` cannot carry. A call that unwinds is not that case: it keeps `then`, or has none, and gains an edge beside it.

## More Information

* [ADR-0010](./0010-give-the-graph-an-edge-no-statement-produced.md), the edges as an interface.
* [ADR-0016](./0016-an-analysis-is-a-trait-and-an-unreached-block-has-no-value.md), what a block nothing reaches is to an analysis.
* [#338](https://github.com/itsakeyfut/safec/issues/338), the constant controlling expression, which is the other half of the same report.
