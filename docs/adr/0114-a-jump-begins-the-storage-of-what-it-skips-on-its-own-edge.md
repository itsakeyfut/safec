---
status: "accepted"
date: 2026-11-20
decision-makers: itsakeyfut
---

# A jump begins the storage of what it skips, on its own edge

## Context and Problem Statement

ADR-0012 put a local's `StorageLive` at its declaration and called that an
approximation with a stated end: C17 6.2.4 p6 begins an automatic object's
lifetime "from entry into the block with which it is associated", and the two
readings come apart only when control enters a block without passing the
declaration. `goto` into a block, `goto` forward past a declaration, and a
`case` label after one (6.8.4.2) are each that, and #616 adds all three. ADR-0012
expected the change to move `StorageLive` to the top of the scope, which needs a
pre-pass over each compound before any of it is lowered; it also named the cost
of that pre-pass, a declaration's type lowered and reported out of order.

## Decision Drivers

* On the path a jump takes, the IR says what C says: every object whose block
  the jump enters is alive, and every object whose block it leaves is not.
* No pass reads a compound before it is lowered, so a bad declaration is still
  reported once, where it is.
* The analyses that read storage markers today read the new ones unchanged.

## Considered Options

* The jump's own edge: each jump that leaves or enters a scope goes through a
  block of its own holding the `StorageDead` of what it leaves and the
  `StorageLive` of what it enters, declared before its target.
* ADR-0012's stated end: every `StorageLive` at the top of its scope, found by
  a pre-pass, so that entering a block anywhere has passed its markers.
* Refuse a jump that skips a declaration, as `SC0207`.

## Decision Outcome

Chosen option: "the jump's own edge", by the owner's decision on #616.

* The lowering keeps, for each open scope, an id and the locals it has
  declared so far. A jump compares the scopes open where it is with the scopes
  open at its target. It ends what it leaves, innermost first, in reverse order
  of declaration, which is what a `return` already does (ADR-0012). It begins
  what it enters: every local of a scope open at the target and not at the
  jump, and, in the deepest scope open at both, every local the target has
  declared and the jump has not.
* A jump whose target is not lowered yet, a forward `goto` or a `switch`'s
  dispatch to a `case`, ends its block with a `Goto` to a reserved block, and
  that block is written when the target is reached, because only then are the
  locals declared before the target known. A backward jump writes the markers
  in its own block.
* The markers are `Generated` at the jump, as a `return`'s are: the jump is the
  reason the storage ends or begins, and nobody wrote a marker.
* A declaration still begins its local's storage where it is, on the path that
  reaches it. One in the body of the function itself, which ADR-0012 leaves
  unmarked because it lives as long as the frame, is marked too once a label
  has come before it in the function: a backward `goto` can then reach it a
  second time, and unmarked its last value was carried round as if the
  declaration had not been reached. It is not ended by any jump or `return`. A backward jump over a declaration in the same block leaves the
  local alive and passes the declaration again, so the marker runs again where
  C says only that the value "becomes indeterminate each time the declaration
  is reached". That keeps what is read right, an indeterminate value, and is
  wrong about a pointer kept from before the jump, which C still lets name the
  object.

### Confirmation

Each half of the rule is held by a program run through the interpreter, in
`crates/safec-c/tests/interp.rs`, which stops on a write to storage that has
not begun and on a read of storage that has ended or holds nothing:

* `a_goto_into_a_block_begins_what_it_skips` fails when `jump` begins nothing
  in a scope only the label has open.
* `a_goto_past_a_declaration_begins_only_what_it_skips` fails when it begins
  nothing the deepest shared scope declared, and when it begins that scope's
  whole list rather than what was declared past the jump.
* `a_goto_out_of_a_block_ends_what_it_leaves` fails when it ends nothing in a
  scope only the `goto` has open.
* `a_goto_between_sibling_blocks_ends_one_and_begins_the_other` fails when
  scopes are matched by depth rather than by id.
* `a_body_declaration_a_backward_goto_reaches_again_is_indeterminate`, in the
  corpus, is built in silence when a declaration in the body of the function
  is left unmarked after a label.

The memory check does not ask whether storage is alive, which is the lifetime
level's question, but it does read a `StorageLive` as a new object holding
nothing, so a pointer local a `goto` enters a block past is unassigned there.
`a_pointer_a_goto_enters_a_block_past_is_unassigned_there` holds the entering
half on that side: without it, the read is silent.

### Consequences

* Good, because every jump is exact about what it leaves and enters, with no
  pre-pass and no change to how a declaration is lowered.
* Good, because the analyses read nothing new: a jump's markers are the
  elements a `return` and a closing brace already write.
* Bad, because a pointer taken to a local before a backward jump over its
  declaration is treated as pointing at a new object once the declaration is
  passed again, which can report a program C defines, and makes the
  interpreter stop one. Reaching it takes `&x`, a backward `goto` past the
  declaration of `x`, and a use of the old pointer after.
* What would reverse this: a construct that enters a scope with no jump to put
  the markers on, which C does not have.

## More Information

ADR-0012, whose decision this keeps and whose stated end it replaces with a
different route. #616 and its sub-issues.
