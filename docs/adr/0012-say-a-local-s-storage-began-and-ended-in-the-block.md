---
status: "accepted"
date: 2026-09-11
decision-makers: itsakeyfut
---

# Say a local's storage began and ended as elements of a block, not as a tree beside it

## Context and Problem Statement

Two programs that differ in one pair of braces lower to the same IR:

```c
int f(void) { int *p; int x; x = 42; p = &x; return *p; }        /* fine */
int g(void) { int *p; { int x; x = 42; p = &x; } return *p; }    /* dangling */
```

Measured with `--emit safety-ir`: the two artifacts differ in the function's
name and in the spans, and in nothing else. An analysis handed either has the
same material, so it must answer both the same way: silent about `g`, or wrong
about `f`. `docs/safety-model.md`'s lifetime axis is about exactly this
difference.

The IR's own module comment says so and names this record's issue. What has to
be decided is where "this local's storage is gone" is written down, because
Phase 4 builds the dataflow framework and Phases 5 and 6 are written against
whatever shape that takes.

## Decision Drivers

* **A dataflow analysis asks a question at a program point.** `docs/roadmap.md`
  Phase 4 is lattices, transfer functions and a fixpoint over the graph. Its
  input is what happens as control moves through a block.
* **Control can leave a scope somewhere the syntax does not.** `goto`, `break`
  and `continue` are Phase 1's remaining statements, and each one leaves a
  scope at a point that exists in the CFG and not in the source's nesting.
* **C++ ends a scope by running code.** `docs/c-family.md`'s first constraint
  is that an operation must be able to say it was not written, and names a
  destructor at the end of a scope as the case. Those are calls, and they
  happen where the scope ends.
* **A loop reuses the local.** `while (n < 3) { int x; ... }` lowers today, and
  `x` is one `LocalId` across every iteration.
* **A reversal has to fail to build.** ADR-0010 is confirmed by `E0004` on
  `Terminator::successors`, and RK-018 records why the arms there name every
  field: `E0004` answers for a variant and `..` lets a field walk past it.

## Considered Options

* An element of a block becomes an enum: an assignment beside a statement that
  says a local's storage began or ended.
* `Function` carries a scope tree, and locals and blocks belong to scopes.
* `Block` carries a list of locals whose storage ends as the block does.

## Decision Outcome

Chosen option: an enum element.

```rust
pub enum Element {
    Assign(Operation),
    StorageLive { local: LocalId, origin: Origin },
    StorageDead { local: LocalId, origin: Origin },
}

pub struct Block {
    pub elements: Vec<Element>,
    pub terminator: Terminator,
}
```

Two things follow from the drivers rather than from taste.

**Live as well as Dead.** A loop reuses the local, so a `StorageDead` at the end
of the body with nothing to answer it would leave the second iteration writing
to storage the IR says is gone. C17 6.2.4 p6: an automatic object's lifetime
"extends from entry into the block with which it is associated until execution
of that block ends in any way". Each iteration of a loop enters the block and
ends it, so each iteration is a fresh lifetime, and the IR should be able to
say so. The paragraph's other sentence, that "if the block is entered
recursively, a new instance of the object is created each time", is about
recursion and is answered already: a recursive call gets its own frame.

**`StorageLive` may sit anywhere in the block, and belongs at the top.** C17
6.2.4 p6 read literally begins the lifetime at entry into the block; reaching
the declaration does something else, which is that "the value becomes
indeterminate each time the declaration is reached". An element can be put
wherever the block wants it, so the IR expresses either, and that is what this
record fixes.

**The lowering emits it at the declaration, and that is an approximation with a
stated end.** For every program that parses today the two placements are
indistinguishable, because C17 6.2.1 p7 starts a name's scope at its
declarator, so nothing can name the object earlier. Emitting at the top would
mean a pre-pass over the compound to find its declarations before lowering any
of them, and a pre-pass that calls the type lowering either reports a bad
declaration twice or reports it before statements written above it. Neither is
worth paying for a difference nothing can observe. The two come apart with
`goto` into a block, which is legal C and skips the declaration while the
object is already alive; `goto` is also what forces the pre-pass to exist for
its own reasons, so that is the change that moves this.

**Markers only where a scope is narrower than the function.** A parameter and a
local declared in the function's own body live exactly as long as the frame,
which every consumer already models: the interpreter pops the frame, and a
pointer into a returned one is caught by its generation. Marking them would add
a pair per local to every artifact and say nothing that the frame does not.

Temporaries get no markers either, and that is not a convenience. C17 6.5.3.2
p1 constrains the operand of unary `&` to a function designator, the result of
`[]` or unary `*`, or an lvalue designating an object. A temporary in this IR
is none of those, so no pointer can name one and no analysis can be asked what
a temporary outlived.

### Confirmation

`E0004`, and it was measured rather than argued. Adding a fourth `Element` kind
stops three places compiling at once: `Element::name` in `crates/safec-ir/src/ir.rs`,
the element walk in `crates/safec-ir/src/interp.rs`, and `dump_ir`'s in
`crates/safec-ir/src/print.rs`.

`E0027` for a field, which is the narrower and worse failure RK-018 records.
Adding a field to `Element::StorageDead` and fixing nothing is `error[E0027]:
pattern does not mention field` in both of those walks, and once they are fixed
it is `error[E0063]` where the lowering builds the element and `E0027` again in
the test helper that reads one. Every arm names its fields; none writes `..`,
except `Element::name`, where the name does not depend on what the kind carries.

The two programs are `the_same_program_in_a_nested_scope_is_not_the_same_ir` in
`crates/safec/src/lowering.rs`: it lowers both and fails if the `StorageDead`
loop goes from the `Compound` arm. `a_local_the_function_declares_has_no_marker`
holds the other half, that a local whose scope is the function's gets nothing,
and fails if the depth test goes.

The claim by running rather than by inspection is
`a_read_through_a_pointer_to_dead_storage_stops_the_run` in
`crates/safec/tests/interp.rs`, which fails if the interpreter treats a
storage-end as a no-op, and `a_loop_body_that_declares_something_runs_more_than_once`,
which fails if `StorageLive` goes and the second iteration finds no storage.
`dead_storage_and_uninitialised_storage_are_not_one_sentence` is what keeps the
two states from collapsing back into an `Option`.

### Consequences

* Good, because the question Phase 5 asks, "is this place still allocated at
  this point", is answered by walking a block, which is what a transfer
  function does.
* Good, because a C++ destructor at the end of a scope is an element in the
  same list, carrying the `Origin::Generated` that `docs/c-family.md` asks for,
  rather than something a tree has no room for.
* Good, because `goto` leaving a scope is expressible: the element sits on the
  path control takes, and the CFG already carries that path.
* Bad, because every walk changes. The lowering, the printer, the interpreter
  and about thirty tests that match a block's elements positionally are
  rewritten in one commit, and `Block::operations` is gone.
* Bad, because the artifact grows two lines per nested local, and every corpus
  expectation holding a nested scope is re-blessed.
* What would reverse this: an analysis that wants the nesting rather than the
  path. That is a question about declarations, which a frontend can answer
  without the IR, so it would argue for adding a tree beside this rather than
  for replacing it.

## Pros and Cons of the Options

### An enum element

* Good, because it is a program point, which is what a dataflow analysis reads.
* Good, because `E0004` makes every consumer answer for a new kind.
* Bad, because it is the only option that is not additive.

### A scope tree on `Function`

* Good, because nothing that exists changes.
* Bad, because a tree says where a local was declared and not where control
  left its scope. `goto` and `break` leave at a point the tree cannot name, and
  a block would have to belong to exactly one scope, which constrains where the
  lowering may split one.
* Bad, because acceptance criterion 3 of the issue asks that every walk over a
  block's elements answer for the new thing, and under this option no walk
  changes, so nothing has to answer for anything.

### A list of dead locals on `Block`

* Good, because it is additive and the element type does not change.
* Bad, because a scope can end in the middle of straight-line code, so the
  lowering would have to end a block at every closing brace. Blocks stop being
  about control flow and start being about syntax.
* Bad, because the order in which storage ends is lost, which a destructor's
  reverse order of construction needs.

## More Information

* [`docs/safety-model.md`](../safety-model.md) for the lifetime axis this
  serves, and [`docs/roadmap.md`](../roadmap.md) for Phase 4's dataflow, which
  is the first consumer.
* [`docs/c-family.md`](../c-family.md), *An operation must be able to say it
  was not written*, for what a scope end has to carry once C++ arrives.
* [ADR-0010](./0010-give-the-graph-an-edge-no-statement-produced.md) made the
  same argument for the edge set and deliberately decided nothing else.
* C17 6.2.4 p6 for when an automatic object's lifetime begins and ends and for
  a fresh instance per entry, 6.2.1 p7 for where a name's scope starts, and
  6.5.3.2 p1 for why a temporary needs no marker. Quoted from N2310.
