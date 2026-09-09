---
status: "accepted"
date: 2026-09-07
decision-makers: itsakeyfut
---

# Carry a position rather than the text it covers

## Context and Problem Statement

The lexer produces tokens, and a token has to let a later stage learn which word
an identifier is. The obvious answer is to put the word in the token.

The same question is about to be asked again, in the same shape, several times
over. An AST node naming a variable, a symbol table entry, a Safety IR operand
that has to say which place it refers to: each one names a region of source and
each one could carry the text of that region. Answering it once for tokens and
leaving the rest to be settled case by case is how the answers stop agreeing.

There is already an answer in the codebase, stated in `crate::source` and never
generalised. Positions are byte offsets, and a line and column are worked out
from a position when something needs one, rather than recorded alongside it. A
token carrying its text is the same question at one remove, and the reason not
to is the same reason.

## Decision Drivers

* A value that carries both a span and the text at that span holds one thing
  twice. Diagnostics point with the span, so the span is what is authoritative;
  the text is a copy that every construction site is then responsible for
  keeping in step, and nothing checks that they are. ADR-0004 refused the same
  arrangement for `Policy`: an invariant nobody can enforce is not an invariant.
* A token that carries a value forces the scanner to compute one. What a numeric
  constant is worth, and what type it has, follows from its base, its suffix and
  the first type it fits in, some of it implementation defined. That is a
  question for a stage that knows the target's type sizes. A scanner is not that
  stage and should not be given a field it can only fill in by guessing.
* Interning is the right answer eventually, and the wrong answer now. It is
  worth having when identifiers are compared often, which is name resolution,
  and it costs a `&mut Interner` on the lexer from the day it is introduced.
* Reversing this is not local. It touches the type, everything that builds one
  and everything that matches on one, which by then is the whole frontend.

## Considered Options

* **A span and nothing else**, with the text read out of the source map when a
  stage needs it.
* **A span and an owned `String`**, so a token answers on its own.
* **A span and an interned `Symbol`**, with an interner threaded through the
  lexer from the start.

## Decision Outcome

Chosen option: **a span and nothing else.**

```rust
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}
```

`TokenKind::Identifier` says that a word is not a keyword, and stops.
`TokenKind::Number` says where a numeric constant begins and ends, and stops.
The text is `sources.snippet(token.span)`, and the value and the type of a
constant are worked out later by something that can.

The rule this is an instance of: **a value that names a region of source carries
the region. Text is derived from it on demand.** The AST and the Safety IR are
expected to follow it, for the reason above rather than for consistency's own
sake.

### Consequences

* Good, because there is one authority. A caret and a name cannot disagree about
  what a token is, since there is only one place either can come from.
* Good, because the scanner settles only what it can settle. Literal typing lands
  in a stage that knows the target, which is where it was always going to have to
  live.
* Good, because a token is 16 bytes and `Copy`, and scanning a file allocates
  nothing per token. This is a consequence rather than a reason, but it is the
  consequence that can be asserted.
* Bad, because a stage that wants text needs the source map. Three of them do:
  keyword recognition, which is in the lexer and already has it; identifier
  comparison, in name resolution; and literal evaluation, in semantic analysis.
  All three are stages that ought to have a source map. The one that should not
  reach for it is the parser, and in Stage 1 it has no reason to: the only place
  a C recursive descent parser has to compare an identifier is the typedef
  ambiguity in `(T)*x`, which is Stage 2.
* Bad, because comparing two identifiers is a string comparison until an interner
  exists. Nothing in Stage 1 does it often enough to notice.
* Neutral, because it fixes when to revisit this. **The parser asking for a
  `&SourceMap` is the signal to add an interner.** Not a measurement and not a
  size; the parser reaching for text is the point at which the derived form is
  being asked for often enough to be worth materialising.
* Neutral, and worth stating because the tree has since arrived: the table this
  record calls "parallel to the tokens" is keyed by span, not by token index.
  `Expr::Identifier` in `crates/safec/src/ast.rs` carries a `Span`, and name
  resolution walks the tree rather than the token stream, so an index into the
  scan is not something it holds. `Span` derives `Hash` for that reason. The
  shape the record fixes is unchanged; only what the key is was left open, and
  the tree has answered it.

  What that paragraph is *not* about is the table name resolution builds, which
  arrived later still and is keyed by `ExprId` rather than by a span. The
  reason is the one this record could not see: a macro body is one piece of
  text, so two uses expanded from one `#define` carry one span and can refer to
  two declarations, while ADR-0008's ids are unique by construction.
  `crates/safec/src/sema.rs` says so where it declares the table, and calls its
  own type `Binding` rather than `Symbol` so that the name this record spends
  below is still free.

  The shape it takes then is decided here too, because it is what makes "later"
  cheap: interning is a pass *after* the scan, over `(&SourceMap, &[Token])`,
  producing a `Vec<Symbol>` parallel to the tokens or resolving lazily. `Token`
  does not change and the lexer never learns that an interner exists. An
  interner that the lexer holds is the version of this that cannot be added
  later without rewriting the scan and its tests, so it is ruled out now rather
  than discovered then.

### Confirmation

`crates/safec/src/token.rs`:

```rust
const _: () = assert!(mem::size_of::<Token>() == 16);
```

A compile time assertion rather than a test, because this guards against a field
being added and nobody is going to remember a test about a struct's size. Adding
`text: String` takes `Token` to 40 bytes and the crate stops building; adding
`symbol: Symbol` takes it to 20 and does the same.

The number is not a budget. It is the shape of the decision written in the one
form the compiler can check: 16 bytes is a kind and a span, and anything else is
something else.

What is not confirmed is the trigger. Nothing fails when the parser reaches for
the source map, because there is no parser. That is a convention recorded here
and enforced by whoever reviews the change, which is worth saying plainly rather
than leaving a reader to assume a guard exists.

## More Information

* `crates/safec/src/token.rs`: `Token`, `TokenKind`, and the size assertion.
* `crates/safec/src/source.rs`: the module documentation stating the same rule
  for positions, which this generalises.
* [ADR-0004](0004-resolve-the-strictest-level-where-the-policy-is-built.md): the same
  argument about invariants that nothing can enforce, in a different place.
