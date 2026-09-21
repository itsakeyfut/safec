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

[Phase 10](roadmap.md) adds the Clang adapter. It should be read as the permanent
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

This is the one that is due soon. [Phase 2](roadmap.md) builds the CFG, as part
of a Safety IR that is block-structured from the start, and every analysis after
it walks that graph. A CFG whose
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

### The arrow points at the IR, so the IR is where a normalisation belongs

An analysis not knowing which frontend produced the IR is a claim about the
**IR**, not only about the crate graph: whatever a frontend does, what arrives
has to be the same thing. So where two C spellings would otherwise arrive as two
shapes, the normalisation is the frontend's to perform and the IR's to require.

There are three such requirements today.

[ADR-0022](adr/0022-say-where-c-sequences-one-evaluation-before-another.md) asks
a frontend to say **where C sequences one evaluation before another**, as an
`Element::Sequenced`. A block's element list is a total order and C gives a
partial one, so without this an analysis reading the list as sequencing believes
whichever order the frontend happened to emit. C17 Annex C is the complete list
of sequence points and the requirement is to emit one for each that no
unsequenced operator encloses, which is the whole of the difficulty: `*p +
(free(p), 0)` contains a comma, and recording it would tell an analysis that the
free happens before the read of `*p`, which C has not decided.

**What an omission costs here is a proof and not a silence**, which is the
opposite of the requirement below and is deliberate. A free the memory check has
not seen sequenced is never a proof, so an adapter that emits none gets
suspicions where it would have had certainties: annoying, and never wrong about
safety. `crates/safec-ir/tests/freed.rs` models the boundary its programs have,
in `after_the_statement`, and says there what an IR without them answers.

**The same element bounds the question asked in the other direction**, which
[ADR-0023](adr/0023-carry-a-read-forwards-to-the-free-it-is-unordered-against.md)
added: a read is carried forwards until one of these, so that a free later in the
same full expression is compared with it. An omission costs a suspicion here too,
because a read nothing has ordered is carried further than it should be rather
than dropped. So the bias holds whichever way the question is asked, and a
frontend that emits too few markers is louder rather than quieter.

**It also costs time, which is the half a producer will not guess.** What the
marker bounds is a set carried in the lattice value, so without one the set is
per function rather than per full expression and is cloned into every block the
solver visits. Measured, on generated straight-line functions with the C
frontend's markers disabled: 1.2 s becomes 9.5 s at 908 lines and 8.0 s becomes
38 s at 1508 lines. That moves the failure from a report somebody can read to a
wait with nothing to read, which
[`CLAUDE.md`](../CLAUDE.md) ranks lower, so "annoying and never wrong about
safety" is true of the answer and not of the run.

[ADR-0026](adr/0026-say-that-a-call-s-arguments-have-been-evaluated.md) asks for
the one point the paragraphs above do not cover, as an
`Element::ArgumentsEvaluated`: C17 6.5.2.2 p10's first sentence puts a sequence
point after a call's arguments and before the call, and the IR expressed that by
position until a consumer arrived that travels forwards. The enclosure rule is
the same one, asked about the call rather than about the point.

**It says less than the element above, and a producer owes only the less.** A
read behind it is ordered before what follows; nothing is concluded about a free
behind it, because a call keeps argument reads in its own operands and a
consumer judges those after an element written before the terminator. A frontend
that emits none gets `free(p + *p)` reported, which is a suspicion about a
program C defines, so the bias is the same as above: too few markers is louder
rather than quieter.

There is one more such requirement today.
[ADR-0021](adr/0021-fold-a-zero-pointer-offset-where-the-ir-is-built.md) folds a
zero pointer offset away, so **no `Rvalue::Binary` in a well-formed safety IR
adds or subtracts a literal zero from a pointer**. C17 6.5.2.1 p2 makes `E[0]`
and `*E` one expression and this keeps them one shape.

A frontend that does not fold gets silence rather than a diagnostic, and this is
measured rather than feared: hand-built IR for
`t = pp + 0; *t = q; free(q); *p = 1;` produces no finding at all, where the
folded shape of the same program produces one. Nothing enforces the requirement
at the boundary, which is why it is written here, in the document a frontend
author reads, rather than only in the record of the change that introduced it.
`an_unfolded_zero_offset_is_a_shape_this_check_does_not_follow` in
`crates/safec-ir/tests/freed.rs` holds the boundary so that closing it later
fails a named test rather than passing quietly.

**A fourth arrived with**
[ADR-0030](adr/0030-a-pointer-operand-decides-what-pointer-arithmetic-reaches.md),
and it is the first that this compiler's own frontend does not yet satisfy. The
memory check reads a local's declared type to tell the pointer operand of an
addition from the integer beside it, and drops the integer, so **a local that
may hold an allocation has to be declared as a pointer**. C17 6.5.6 p8 is what
makes that sound for a program whose types are what C requires.

Two shapes break it today and neither involves a cast. `Lowering::promoted`
gives a compound assignment's temporary `Ty::Int` whatever the left operand is,
so `p += i` writes pointer arithmetic into a local declared `int`; nothing is
lost, because no expression puts that temporary beside a pointer operand, and
the requirement is broken all the same. And an initializer is never checked
against the assignment constraint, so `int n = p;` is accepted in silence where
`n = p;` is `error[SC0302]`.

**What an omission costs here is a silence, which is the worst of the four.**
Measured on hand-built IR: a `malloc` into a local declared `int`, added to a
pointer that holds nothing, produces no finding at all where the same program
with the local declared as a pointer produces a proved use after free.
`an_allocation_in_a_local_declared_int_is_dropped_beside_a_pointer` in
`crates/safec-ir/tests/freed.rs` holds that boundary, so that closing it later
fails a named test rather than passing quietly, and #204 and #205 are the two
shapes above.

**A fifth arrived with**
[ADR-0031](adr/0031-a-write-this-check-cannot-pin-down-replaces-what-an-escaped-local-holds.md),
and it is the other half of the same reading. The memory check compares the type
a write writes with the type each escaped local is declared with, and distrusts
only the locals that match, so **a write through a pointer to `T` must not land
in an object declared as anything but `T`, unless `T` is a character type**.
C17 6.5 p7 is what makes that sound for a program whose types are what C
requires: an object has an effective type and an lvalue of another type may
access it only in the cases that clause lists.

The character type is the case of those that a conforming program reaches, and
it needs no cast to get there: C17 6.3.2.3 p1 and 6.5.16.1 p1 make the `void *`
round trip implicit both ways, so `void *v = &p; char *c = v;` is a program
this frontend accepts and copying a pointer's object representation through `c`
is defined. The check answers for it by distrusting every escaped local at a
write of a character type, which is why the requirement is written with an
exception rather than without one. An earlier draft of ADR-0031 claimed the
case needed a cast; review wrote the C, compiled it with `clang -std=c17
-pedantic-errors` and ran it under AddressSanitizer to show otherwise.

**What an omission costs here is a false positive, which is the cheapest of the
five.** The comparison can only keep a local out of the distrusted set, a local
that is not distrusted keeps whatever this check had proved about it, and a
proof is a report: an adapter whose types are dishonest gets the `error` about a
program C defines that ADR-0031 removed, and never a silence. That is the
opposite of the requirement above it, which is why the two are written as two:
they read the same `Ty` and they fail in opposite directions.

This compiler's own frontend breaks it today, by the same shape that breaks the
fourth. `char **alias = *outer;` is accepted in silence where `alias = *outer;`
is `error[SC0302]`, which is #205, and a write through that `alias` is then
narrowed away from an `int *` local that it may have reached. Measured, and the
answer is the `error[SC0402]` this compiler gave before ADR-0031 rather than
anything quieter.

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
| Phase 2 builds the CFG | Whether an edge can exist that no statement produced | [ADR-0010](adr/0010-give-the-graph-an-edge-no-statement-produced.md), written when the trigger fired |
| The Safety IR gains a function table | Whether identity is an opaque id or a name | `FuncId`'s doc comment in the IR, where it was recorded when the table landed |
| `Span` grows its third coordinate | What it has to serve: macros, instantiations, and generated operations | An ADR; `Span`'s doc already says the shape is pinned by more than privacy |
| The safety IR is extracted from the `safec` crate | Which crates may depend on which | [ADR-0011](adr/0011-the-ir-crate-depends-on-nothing-in-the-workspace.md), written when the trigger fired |
| The first safety check is written | Where `Diagnostic` lives, and therefore which crate an analysis can be written in | An ADR. [ADR-0011](adr/0011-the-ir-crate-depends-on-nothing-in-the-workspace.md) says why this is the trigger and why waiting costs nothing |
| The first C++ lifetime bug is analyzed | Whether the C annotation and the C++ type reached the same IR concept | Wherever it shows that one of them was wrong |
| The crate is published | The name | Not an ADR. A decision, taken once |

Three of those triggers have since fired, and their rows say where each answer
went. Nothing is opened for the rest. By the bar in
[adr/README.md](adr/README.md), a record answers what guards a decision now, and
none of the remaining four guards anything yet: `Span` has two coordinates,
there is no check, there is no adapter, and nothing is published. The triggers are what turns each
one into a record, and this document is the thing that has to be reread when one
of them fires.
