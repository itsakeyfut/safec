---
status: "accepted"
date: 2026-10-19
decision-makers: itsakeyfut
---

# A span from an expansion is located in a copy of the macro's body

## Context and Problem Statement

`Span` is `(file, start, end)`, and its doc comment has expected a third coordinate since it was written: a macro expansion, and later a template instantiation, needs to say both where code is written and where it came from. `docs/roadmap.md` asks that it serve three consumers rather than macros alone: a macro expansion, a template instantiation, and an operation no source text produced. Object-like macros, [#546](https://github.com/itsakeyfut/safec/issues/546), are the first thing that builds it.

It is a safety question rather than a display one, because analyses reach conclusions by comparing spans:

* **Containment.** `memory/report.rs::inside` takes a read strictly inside a call's span as ordered before the call (ADR-0043), and `driver/words.rs::unordered` asks whether a span lies inside a parameter's length.
* **Identity.** Equal spans are taken for one event or one name. This is done by:
  * `memory/known.rs::noticed` and the pending-call tests beside it;
  * `memory/parts.rs::same` and `read_key`;
  * the calls key in `transfer.rs`;
  * the `said` deduplication in `report.rs`;
  * the nullability fold in `nullability.rs`;
  * `sema.rs`'s searches for an identifier or a tag by its span;
  * the lowering's `locals: HashMap<Span, LocalId>`.
* **Order.** `memory/parts.rs::earlier` orders two spans by file, then start, to decide which free a joined state names and which of two proofs stands at one caret.
* **Text.** `snippet` gives names, numbers and what the memory check recognises as `free` or `malloc`.

A first design, kept in [#544](https://github.com/itsakeyfut/safec/issues/544), located every token of an expansion where the macro was used. A prototype showed it silencing the memory check in three ways that the same program written out is reported for:

* a call that crossed an expansion's edge contained the whole expansion, so `inside` skipped a read in another operand;
* equal spans made `Known` take distinct elements for one event;
* two declarators from one expansion collided as `locals` keys.

## Decision Drivers

* A comparison that answers differently for a macro than for the same code written out is a silence waiting to happen, and a believed silence is the worst thing this compiler can do.
* ADR-0043 chose that a mistake falls on the reporting side.
* `Span` is `Copy` and in every token, node and IR element; `Token` is held to 16 bytes (ADR-0006).

## Considered Options

* **An instance file per expansion:** every expansion is a copy of the macro's body registered as a file of its own, and its tokens are located in it.
* **The use as the location**, with the spelling in a table and a mark carried through joins so that comparisons can be made conservative where it is set.

And for a node across an expansion's edge:

* **Lift a node that holds a whole expansion, and refuse one that cuts an expansion.**
* **Lift every node across an edge.**

## Decision Outcome

Chosen: **an instance file per expansion; a node holding a whole expansion is lifted to where it was used, and a node that cuts one is refused.**

**Each expansion is a file of its own.** The source map registers, for every expansion, a file holding the expansion's replacement text, an *instance*, whose `FileName` records two spans:

* `spelled`, where the body was written in its `#define`;
* `at`, the macro's name where it was used.

For a macro used inside another macro's body, `at` lies in the outer instance, so instances form a chain to a real file. Each token the expansion produces is located in its instance at the position its text has there. `Span` keeps `(file, start, end)`, and `Token` keeps its 16 bytes: the third coordinate is the instance's record, reached through the span's file.

**So inside one expansion, every comparison answers as it does for the body written out.**

* Containment, equality and ordering between two spans of one instance are what they are between the same text written out.
* `snippet` reads the instance, which is the body's text, so names, numbers and allocator names resolve as written.
* Two expansions are two files and never compare equal or contained, including two expansions of one macro, so `#define C g(*p)` used as `C + (free(p), C)` keeps its report.
* Two declarators from one expansion have distinct name spans. Every identity test listed above therefore stays a test of `Span`'s derived equality, unchanged.

**A node across an expansion's edge.**

* **A node that holds a whole expansion** is located in the outer file, as `N + 1` and `g(N)` are. Each endpoint that lies in an instance is lifted along the chain to the span of its use first, so `N + 1` runs from `N`'s use to `1`.
* **A node that holds some of an expansion's tokens and not others** is refused, as valid C this compiler does not read yet. That is the shape the prototype silenced: `F(p)` with `#define F (x = *p) + release`.

**Wholeness is decided on the tokens a production consumed, not on spans.** A parenthesised expression makes no node of its own, and a declarator's span starts at its declaration's specifiers, so a test on node spans would refuse `#define N (expr)`, the usual way to write a macro, as a cut. An expansion's tokens are contiguous in the token stream, so a production holds a whole expansion exactly when the range of tokens it consumed contains the expansion's range, lies inside it, or misses it. The parser does not record that range for expressions today: its expression joins use the spans of the nodes they join. So #546 owes a change to every expression production, each recording the token it started at, and joins made from those tokens rather than from node spans.

**Two operations, one for joins and one for comparisons.**

* **A join lifts with a side.** `SourceMap::lift_start(span, into)` and `lift_end(span, into)` move a span along its chain to the use in `into`. The first succeeds only where, at every level of the chain, the span starts at the instance's first token; the second only where it ends at the instance's last. On a contiguous token stream that is exactly the wholeness test, so a test that answered "whole" for a cut still cannot be joined: `F(p)` starts its call at `release`, which is not the first token of `F`'s instance, and the start lift fails. A join is made by `SourceMap::join(from, to)`, which lifts `from` by its start and `to` by its end into the nearest file both reach and answers either the span or the cut it met. The parser reports a cut as `SC0207`, valid C not read yet, and the lowering, which builds spans the parser never joined, as `SC0304`; `lift_start` and `lift_end` are that same check where a cut panics, naming it, for a caller that cannot meet one. A cut is C, so it is refused rather than panicked on, and it is still never joined. Amended by #553, which built the joins.
* **A comparison locates, without a check.** `SourceMap::locate(span, into)` moves any span, an interior one included, to the use in `into`, and fails no span. Containment and order use it.

Every join that can cross files goes through `SourceMap::join`: the parser's joins, and the two joins the lowering makes outside the parser, the span over a definition's parameter lengths and a declarator's store span. `Span::to` keeps its assertion that both spans are in one file, and `Span::new` is never given offsets from two files.

**Containment and order locate.** `inside`, `unordered`'s test, `earlier`, and any comparison of the same kind locate both spans in the nearest file they share and compare there. A read inside `g(N)`'s argument is then strictly inside the call, as it is written out, and a free from a macro used above a written free is the earlier one, as written out, rather than ordered by when its instance was registered. Two spans with no file in common are not inside each other, and `earlier` orders them by the files' indices, as it does today, so it stays a total order. Identity tests do not locate, so each token stays itself.

**An `#include` is an instance too.** Each inclusion is a file of its own whose `at` is the `#include` directive, so a node holding an included file, `{` then `#include "body.inc"` then `}`, is joined by the same lifts, and a file included twice is two files that never compare equal. Its text is the included file's, read once from disk. This amends [#545](https://github.com/itsakeyfut/safec/issues/545), which planned to keep an included token in the included file's own `FileId`.

**The translation unit is preprocessed whole, then parsed.** The preprocessor registers instances, so it needs the source map mutably; the parser lifts, so it needs it immutably; the two cannot be interleaved. So preprocessing ends before parsing begins, as lexing does now (ADR-0003, ADR-0005). The parser reads only the chain of `at` spans, not any text, so this does not fire ADR-0006's trigger for an interner, which is the parser asking for text.

**`Origin` is unchanged.**

* A macro's body was written, so an operation from it is `Written` with an instance span.
* A generated operation has no spelling and is `Generated` as it is today.
* A template instantiation is the same model as a macro: an instance of the template's text, used at the point of instantiation.

**A diagnostic** at an instance span renders the instance's text, the replacement text, under the macro's name, and adds a note at `at`: expanded here.

### Confirmation

Inclusions and expansions are built by #545 and #546, and the corpus cases in `crates/safec-c/tests/cases/preprocessor/` hold this decision: one for each shape the prototype silenced, each run as a macro beside the same program written out, the two giving the same errors:

* A call that cuts an expansion is refused, and the same call holding the whole expansion is reported as it is written out. Making the wholeness test answer "whole" for the cut fails it, since the cut call is then the use-location design's span and the read inside it is skipped. So does testing only the end of the chain's innermost level, which `#define B release` inside `#define A (x = *p) + B` used as `A(p)` holds.
* A parenthesised body used as a whole operand, `#define N (*p)`, is not refused. Deciding wholeness on node spans fails it.
* `(x = *q) + (*slot = q, 0) + release_all()` from one expansion reports the use of `q`. Locating every token at the use fails it.
* `int *a = malloc(4), *b = 0;` from one expansion, with `a` used after its free, reports it. Keying `locals` by the use fails it.
* A read inside `g(N)` is taken as the call's argument. Comparing without lifting fails it.
* A free from a macro inside a parameter's length is worded as a length's. Comparing without lifting fails it.
* A diagnostic in an expansion shows the body and the use. Dropping the note fails it.
* A function body made of an `#include` between its braces is read. Keeping an included token in the included file's own `FileId` fails it, at `Span::to`'s assertion.
* A file included twice where both inclusions are read does not merge their names or events. Giving every inclusion of one file one `FileId` fails it.

The prototype that showed the silences is the starting point for these cases.

### Consequences

* Good, because inside an expansion every analysis sees what it would see written out, the standard every finding here is measured against.
* Good, because the identity tests, `Origin`, `Span`'s size and every artifact that prints a position are unchanged.
* Good, because a mistake in the cut rule is a refusal or a named panic, never a join, since the lift a join needs is what checks the edge and its side.
* Bad, because a macro whose expansion is not a whole operand where it is used is refused although it is C: a function name with an operator and no right operand, `#define TWO 1 + 1` used as `TWO * 3`, and a type such as `#define PTR int *` used as `PTR a;`, whose declarator holds the `*` and not the `int`.
* Bad, because every expansion is a file in the source map, so a file with many expansions holds many small copies.
* Bad, because containment has to lift, and a comparison written as plain offsets is right for written-out code and wrong for a macro. The cases above are what catch one.
* What would reverse this: a need to accept cut expansions, which would need a rule for a node whose span is shared with its parent, since `Known` takes equal spans for one event. Function-like macros will test the instance's definition too: an argument used twice in a body must be two copies in the replacement text, not two uses of one spelling, and `#` and `##` make text no source file holds, so `spelled` cannot be an offset into the `#define`.

## Pros and Cons of the Options

### An instance file per expansion

* Good, because positions within an expansion are as distinct as written out.
* Bad, because containment across an edge needs lifting.

### The use as the location, with a mark

* Good, because no file is registered per expansion.
* Bad, because every token of an expansion shares a location. Every comparison would then have to consult the mark, and one that did not would be silent with nothing to hold it: the prototype found three such places, in a check nobody had yet written for macros.

### Lift every node across an edge

* Good, because no C is refused.
* Bad, because a node and its parent can be lifted to one span, as `-release` used as `X(p)` makes the negation and the call, and `Known` takes equal spans for one event.

## More Information

* [ADR-0043](0043-a-call-is-ordered-after-what-its-own-arguments-read-and-a-span-inside-it-says-which.md), whose strictness this keeps.
* [ADR-0006](0006-carry-a-position-rather-than-the-text.md), which holds `Token` to 16 bytes.
* [`docs/c-family.md`](../c-family.md#an-operation-must-be-able-to-say-it-was-not-written), on what the coordinate serves.
* [#544](https://github.com/itsakeyfut/safec/issues/544), where both designs and the prototype are, and [#546](https://github.com/itsakeyfut/safec/issues/546), which builds this.
