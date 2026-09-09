---
status: "accepted"
date: 2026-09-08
decision-makers: the author
---

# Hold the tree in flat vectors and address a node by index

## Context and Problem Statement

The parser needs somewhere to put the tree, and every phase after it walks that
tree: semantic analysis attaches a type to each node, the Safety IR is lowered
from it, and each analysis after that reads what the lowering produced.

The shape is settled once. Changing it later means touching every node, so it is
decided before there is a tree rather than after there is a large one.

## Decision Drivers

* Semantic analysis has to record a type for each node. Where it records it
  depends entirely on whether a node can be named without holding a reference
  to it.
* `docs/architecture.md` gives arena-based data structures as one of the reasons
  the compiler is written in Rust.
* `docs/repository.md` asks for a deliberately small dependency list. The crate
  depends on `clap` and `ariadne`.

## Considered Options

* Flat `Vec`s with newtype indices, written here
* `Box`, an ordinary owning tree
* `la-arena`, which `docs/repository.md` lists as a candidate

## Decision Outcome

Chosen option: **flat vectors and newtype indices**, because an index is a name
for a node that costs nothing to copy, keep, or put in a side table, and that is
what the phase after this one needs.

`Box` makes a tree that is pleasant to build and impossible to annotate. There
is no way to say "the type of that node" without either holding a reference to
it, which the borrow checker will not allow across a mutation, or giving every
node an id afterwards, which is this decision arrived at late and with the tree
already written.

`la-arena` would give the same thing this does. What it costs is a third
dependency in a crate that has two, for about twenty lines. `docs/repository.md`
says a small number of well-understood dependencies is preferable to
reimplementing everything, and also that the principle is to minimise accidental
complexity rather than dependency count; twenty lines of `Vec` and a newtype is
not accidental complexity.

### Confirmation

`an_id_still_names_its_node_after_more_are_pushed` in `crates/safec/src/ast.rs`.

The mutation: take the id from `len()` **after** the push rather than before. It
fails, because every id then names the node written after the one it was handed
out for. That is the property `Box` cannot offer and the whole reason for this
shape, so it is the thing worth guarding.

The other half the compiler holds on its own. `Ast`'s vectors are private and
every accessor takes `&self`, so holding a `&Expr` across a push is
`error[E0502]`: pushing needs `&mut self`. Nothing has to remember that rule
because nothing can break it and still build.

### Consequences

* Good, because a node can be named, so semantic analysis puts types in a side
  table keyed by id rather than rebuilding the tree. What such a table needs
  from an id, an `index()` or a `Hash`, and a length per arena, was not here
  when this was written and was deliberately not invented before its first
  caller. `FileId::index` in `crates/safec/src/source.rs` is the same pattern
  one layer up and is the shape to copy. Adding them breaks nothing, which is
  the point: the choice being made here is the one that cannot be added later.

  The caller arrived with `crates/safec/src/sema.rs`, which keys a table by
  `ExprId`, so that id derives `Hash` and `Ast::expr_ids` gives the length. The
  other three ids still have no caller and still derive neither, which is the
  same rule applied rather than an oversight. `index()` has no caller either:
  the table that wants it is #58's.
* Good, because an id is `Copy` and carries no lifetime, so a phase can hold one
  for as long as it likes.
* Bad, because reading a child is `ast.stmt(id)` rather than following a field,
  which is one indirection more to write and to read.
* Bad, because an index into one arena means nothing in another, and only the
  types keep them apart. `ExprId`, `StmtId` and `ItemId` are separate newtypes
  for that reason, and `each_kind_of_node_is_numbered_on_its_own` is what says
  so.
* Neutral, because nothing stops an arena holding a node no root reaches. The
  parser leaves one behind when it abandons a statement whose expression it had
  already pushed. Nothing walks the expression or statement arenas directly, so
  such an orphan costs its own bytes and nothing else. `Ast::items` is the one
  exception and is walked, by the `--emit ast` dump, because the roots are the
  items; so an item nobody meant to keep would be printed, and the parser
  therefore pushes an item exactly once per attempt at one.
* Bad, because a rule arrives with it that nothing enforces: whoever pushes a
  node owns its id, and a function that has pushed must hand the id back rather
  than the node. Returning the node lets the caller push a second copy, and one
  construct with two ids is this decision defeated while looking like it holds.
  `a_nested_block_is_one_node_in_the_arena` in `crates/safec/src/parser.rs` is
  what says so, and it reads the arena through `Debug` because the duplicate is
  unreachable from the root and so invisible to every walk of the tree.
* What would reverse this: giving a node a reference to another node. That is
  the shape this exists to prevent, and it arrives disguised as a convenience.

## Pros and Cons of the Options

### Flat vectors and newtype indices

* Good, because ids survive every later push.
* Good, because no dependency.
* Good, because a wrong-arena index is a type error rather than a wrong answer.
* Bad, because every read of a child goes through the tree.

### `Box`

* Good, because it is the shape a Rust programmer expects and needs no
  explanation.
* Bad, because a node cannot be named, so the phase that annotates the tree has
  to change it.
* Bad, because the change lands after the tree is large, which is the expensive
  moment.

### `la-arena`

* Good, because it is a tested implementation of exactly this.
* Good, because its `Idx<T>` is typed, so the wrong-arena mistake is a type
  error there too.
* Bad, because it is a dependency taken for twenty lines.

## More Information

* [`crates/safec/src/ast.rs`](../../crates/safec/src/ast.rs), where the tree and
  both guards live.
* [`docs/repository.md`](../repository.md) for the dependency principle, and
  [`docs/architecture.md`](../architecture.md) for arenas as a reason to write
  this in Rust.
* ADR-0006 decided that a token carries a position rather than text, which is
  why a name in this tree is a `Span` and why the interner it names as a later
  addition has still not arrived.
