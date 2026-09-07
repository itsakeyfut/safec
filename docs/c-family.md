# The C Family

[concept.md](concept.md) says this project is a reference implementation for
the C language family, and the diagram in
[clang-integration.md](clang-integration.md) already reads `C / C++`. The intent
is on record. What is not on record is what reaching C++ demands of the design,
and some of it is due in Phase 2 rather than at the end.

This document answers that. It is intent, like the rest of `docs/`, and it
decides nothing that code depends on today. What it does is name the decisions
C++ forces, and the moment each becomes due.

## C++ is not reached by extending the C parser

C++ is not a superset of C, and that is the least interesting reason not to
grow one parser into the other. The parser is not where the difficulty is.

For a safety analysis, the difference between the two languages is not syntax
at all. It is three properties C does not have:

* **Operations with no syntax.** C++ runs code that nobody wrote. A destructor
  at the end of a scope, a copy or move constructor on an assignment, the
  destruction of a temporary, the construction of a base subobject. In C, every
  operation the analysis must reason about is written down somewhere.
* **Control flow no statement produced.** An exception turns nearly every call
  into a branch, to code that runs on the way out. A CFG built from `if`,
  `while` and `return` cannot express it.
* **One source, many meanings.** A template is analyzed once per instantiation,
  and a property that holds for `vector<int>` need not hold for
  `vector<unique_ptr<T>>`.

None of the three is a parsing problem. All three are questions about the Safety
IR and the CFG, which is why they matter now: the CFG is Phase 2.

## C++ arrives through Clang, and that is the destination

[Phase 7](roadmap.md) adds the Clang adapter. It should be read as the permanent
answer for C++ rather than as a stepping stone toward a C++ frontend of this
project's own.

Writing a C++ frontend would end the property [concept.md](concept.md) names as
the core philosophy: small enough that one developer can understand the major
components. Name lookup, overload resolution, and template instantiation are the
parts of C++ that cannot be written alone, and they are exactly the parts Clang
already did. The adapter buys them at the cost of a lowering, and the lowering
is a thing one person can hold in their head.

Annotations do not change this. The obvious reason to want a frontend is to add
syntax the safety model needs, in the way
[safety-model.md](safety-model.md#annotations) sketches `owner` and `borrow` for
C. Clang already has attribute mechanisms for carrying annotations it does not
itself interpret, so even that case is served by the adapter.

What would reverse this: nothing currently foreseen. It would take a safety
mechanism that cannot be expressed in anything Clang can parse, and no such
mechanism is on the roadmap.

## Why C first, given that C++ is where the code is

The usual answer is that C is smaller, so it is the easier place to start. That
answer is wrong in the way that matters, and the real one is better.

**C is the harder language for this analysis, not the easier one.** In C,
ownership is written nowhere. `int *p` says nothing about who frees it, how long
it is valid, or whether another thread can reach it, which is why
[safety-model.md](safety-model.md#annotations) has an annotations section at
all. The analysis has to infer what the language never recorded.

In C++, much of it is already written. `std::unique_ptr<T>` is an ownership
annotation, `std::shared_ptr<T>` is a different one, and `T&` is a non-null,
non-reseatable borrow. These appear in real code, written by people who never
heard of this compiler and were not thinking about static analysis when they
typed them.

So starting with C forces the safety model to work with no help from the type
system, and the C++ frontend inherits a model that already survived the harder
case. The annotation experiment is discovering, for C, what C++ already spells.

That gives a test worth applying as the model is designed: **a C annotation and
the C++ type that means the same thing should reach the same Safety IR.** If
`owner int *p` and `std::unique_ptr<int> p` cannot be made to land on one
concept, then one of the two readings is wrong, and finding that out is more
useful than either frontend.

The second observation is that C++ is where the failures are most worth
catching. It is a language with a large safe-looking vocabulary and no
enforcement behind much of it:

```cpp
std::string_view name() {
    std::string s = "hello";
    return s;          // dangles
}
```

This is a lifetime bug wearing the clothes of a safe abstraction. The same shape
appears with `std::span` over a local `vector`, and with a reference to a
container element held across a reallocation. An analysis that can see them is
worth more in C++ than the equivalent is worth in C, because in C the pointer at
least looks dangerous.

## What C++ demands of the Safety IR

Four constraints. Each is cheap while the IR does not exist and expensive once
analyses are written against it.

### An operation must be able to say it was not written

A destructor call at the end of a scope has a source location to blame but no
source text. So every IR operation needs an attribution that distinguishes
"written here" from "caused by this, and generated".

The mechanism is already anticipated. `Span`'s doc comment says it expects a
third coordinate, so that a macro expansion, and later a template instantiation,
can say both where code is written and where it came from. Implicit operations
are the third consumer of that same coordinate, which raises its priority: it is
not a preprocessor detail, it is how any frontend that generates code will point
at source.

Nothing to build now. The point is that whatever shape that coordinate takes has
to serve three cases and not only macros.

### The CFG must carry edges no statement produced

This is the one that is due soon. [Phase 2](roadmap.md) builds the CFG and the
dataflow framework, and every analysis after it walks that graph. A CFG whose
edges come only from `if`, `while`, `return` and `goto` cannot represent an
exception, and adding a new kind of edge afterwards means revisiting every
analysis that assumed the old set.

C does not force the issue, which is exactly the danger: the shape that is
natural for C alone is the shape that has to be undone. The cheap version now is
to leave room in the edge representation rather than to build unwinding, and to
say so where the edge type is defined.

C is not entirely free of this either. `longjmp` is the same shape, and so is
any analysis that wants to reason about what runs on an error path.

### The unit of analysis is the instantiation, not the written function

A template is one piece of source and many functions. The IR should be able to
hold several analyses of one source range, and a diagnostic should be able to
say which instantiation it is talking about, in the way a C++ compiler says
`in instantiation of`.

This one can wait. It costs nothing to accommodate later if the constraint below
is respected, because it is mostly a question of how functions are keyed.

### Identity is not a name

Two C++ functions can share a name and differ in signature; the mangled name is
an implementation detail of the ABI. So a function's identity in the IR should
be an opaque id, not a string.

This is cheap now and is right for C on its own merits: two `static` functions in
different translation units share a name and are different functions. The same
reasoning as `FileId` in the source map, one layer up.

## What makes the boundary real rather than intended

Everything above depends on one arrow: the analyses must not know which frontend
produced the IR. Stated as intent, that lasts until the first convenient place to
reach through it.

[repository.md](repository.md) says to split crates only when boundaries become
clear. This is the criterion for one of them: **the boundary that must not be
crossed is the boundary worth making a crate**, because cargo then enforces the
arrow at build time. A safety-IR crate that does not depend on the frontend
cannot grow a dependency on it by accident, and the reversal is a build failure
rather than a review comment.

That is the same discipline the accepted records already use:
[ADR-0003](adr/0003-pass-the-source-map-to-each-render-call.md) is confirmed by
`E0502` and [ADR-0004](adr/0004-resolve-the-strictest-level-where-the-policy-is-built.md)
by `E0451`. A guard the compiler holds does not have to be remembered.

The testable half is smaller and just as useful: **an analysis should be
runnable over IR built by hand in a test, with no frontend present.** If that
test can be written, the IR is frontend-independent by construction, and the
Clang adapter has a target to aim at years before it exists.

## Not decided here

**How much C++ the adapter accepts.** Real C++ arrives all at once, so the
adapter will need a ladder of subsets in the way `SafetyLevel` is a ladder of
strictness. What the rungs are cannot be guessed before there is an adapter to
disappoint.

**The project name.** `safec` reads as "safe C", and the positioning in
`concept.md` is already broader than that. This is worth settling before the
crate is published, because a name is cheap to change until it is a URL someone
else has written down, and the trigger is therefore the first publish rather
than any point before it.

**Whether the interpreter runs C++ lowerings.** The interpreter in
[architecture.md](architecture.md) exists to test the compiler without LLVM.
Whether it also serves as a way to test adapter output is a question for when
there is adapter output.

## When each decision is due

| Trigger | Decision | Where it will be recorded |
|---|---|---|
| Phase 2 builds the CFG | Whether an edge can exist that no statement produced | An ADR, because every later analysis is written against the answer |
| The Safety IR gains a function table | Whether identity is an opaque id or a name | The IR's doc comments, unless something else turns on it |
| `Span` grows its third coordinate | What it has to serve: macros, instantiations, and generated operations | An ADR; `Span`'s doc already says the shape is pinned by more than privacy |
| The safety IR is extracted from the `safec` crate | Which crates may depend on which | An ADR, because the crate graph is the enforcement |
| The first C++ lifetime bug is analyzed | Whether the C annotation and the C++ type reached the same IR concept | Wherever it shows that one of them was wrong |
| The crate is published | The name | Not an ADR. A decision, taken once |

No record is opened for any of this today. By the bar in
[adr/README.md](adr/README.md), a record answers what guards a decision now, and
none of these guards anything yet: there is no IR, no CFG, and no adapter. The
triggers above are what turns each one into a record, and this document is the
thing that has to be reread when one of them fires.
