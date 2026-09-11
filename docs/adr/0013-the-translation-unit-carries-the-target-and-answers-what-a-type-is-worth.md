---
status: "accepted"
date: 2026-09-11
decision-makers: itsakeyfut
---

# The translation unit carries the target, and the IR answers what a type is worth

## Context and Problem Statement

`ir::Ty` is `Int`, `Char`, `Void` and `Pointer`, and nothing says what any of
them is worth. Its doc comment names this record's phase as where that changes,
and `docs/architecture.md` says an artifact depends on the target "once type
widths reach them".

Nothing has reached it, and the cost is already written down.
`docs/frontend.md` records three answers the interpreter gives that no C
implementation gives, measured against `clang 20.1.6
--target=x86_64-pc-windows-msvc`:

| Written | This interpreter | `clang` |
|---|---|---|
| `int x = 2147483647; return x + 1;` | `2147483648` | `-2147483648` |
| `int x = 1; return x << 31;` | `2147483648` | `-2147483648` |
| `char c = 300; return c;` | `300` | `44` |

The document says why: "it computes in `i128` because `ir::Ty` holds no widths".

Something has to hold the widths, and what holds them decides what every
analysis from Phase 4 onwards reads, because they are written against
`safec-ir`.

## Decision Drivers

* **`docs/architecture.md`'s table.** "tokens, AST, Safety IR: depends on the
  target, once type widths reach them" is a claim about the IR, so an answer
  that lives only in the backend cannot make it true.
* **The interpreter is a consumer today.** An answer nothing reads until the
  LLVM backend exists is an interface with no caller, which `CLAUDE.md` calls
  invented rather than designed.
* **`--emit safety-ir` is an interface** that corpus cases pin byte for byte,
  and `docs/architecture.md` asks that output depend on the target and never on
  the host. Two artifacts that differ in meaning must not be identical in text.
* **`Ty::Int` and `Ty::Char` are printed as `int` and `char`** and greppable as
  such. A shape that dissolves the two into one integer kind changes what an
  artifact says a program is made of.

## Considered Options

* `TranslationUnit` carries a `Target`, and the IR answers what a type is worth.
* `Ty` carries the width and the signedness itself.
* The IR carries a triple and nothing else; the backend and the interpreter each
  keep a table.

## Decision Outcome

Chosen option: the unit carries the target.

```rust
/// What an integer type is worth: a width and a signedness, with the C rules
/// about range and conversion on it.
pub struct Integer { /* private */ }

/// A machine, and what C's types are worth on it. Private fields and a private
/// constructor, so the only targets that exist are the measured rows of `ALL`.
pub struct Target { /* private */ }

impl TranslationUnit {
    pub fn target(&self) -> Target;
    /// What this type is worth here, or `None` for `void` and for a pointer,
    /// which have no width to answer with.
    pub fn integer(&self, ty: TyId) -> Option<Integer>;
}
```

One function rather than a `bits` and a `signed` that each panic on the two
types that have neither: a caller has to say what it does about `void` and
about a pointer, and `Option` is where it says it.

`Ty` does not change, so `--emit safety-ir` still says `int` and `char` and the
corpus still reads as a program's shape rather than as a machine's.

**The artifact names the target.** `dump_ir` prints it first, because the IR now
means something different for a different one and an artifact that hides what it
depends on is what `docs/architecture.md`'s table is about. Corpus cases that
emit the IR name a target explicitly, so that the three CI runners compare the
same bytes.

**The default target is the host's triple**, which is what `cc` and `rustc` do.
That does not contradict "output depends on the target, never on the host":
the host chooses the default, and the output then depends on that target and on
nothing else. A run that names a target gets the same answer everywhere.

**Only what something reads.** The frontend parses `int`, `char`, `void` and
pointers, so `long` and the rest have nothing to say yet. `int`'s width is the
same on every target measured, and it is here anyway because the interpreter
needs a width to detect an overflow at, not because it is expected to vary.
What does vary is whether `char` carries a sign, and
`aarch64-unknown-linux-gnu` is where.

**A pointer's width is not here**, and that is the rule rather than an
oversight. Nothing can observe one: the artifact prints `int *` rather than a
size, there is no `sizeof`, and the interpreter refuses pointer arithmetic.
`i686-unknown-linux-gnu` and `wasm32-unknown-unknown` have 4-byte pointers where
the rest have 8, so the fact is real and the reader is not. It arrives with the
backend, where a datalayout line needs it, and `CLAUDE.md` is why it does not
arrive before: an interface with no caller is invented rather than designed.

### Confirmation

`TranslationUnit::new` takes a target and there is no `Default`, so a unit
nobody said the machine of does not compile. `Target`'s fields are private and
`Target::new` is not public, so the only targets that exist are the rows of
`Target::ALL`, which is the guard ADR-0004 gave `Policy` and is held the same
way.

`every_target_is_what_clang_says_it_is` in `crates/safec-ir/src/target.rs`
writes the measurements out again rather than asking the table about itself,
which is RK-001, and its count fails if a row is added without one. Changing any
width or the signedness of `char` on `aarch64-unknown-linux-gnu` fails it.

The three rows `docs/frontend.md` recorded have stopped being divergences, and
each is held: `an_arithmetic_result_the_target_cannot_hold_stops_the_run` and
`a_shift_by_at_least_the_target_width_stops_the_run` for the two that are
undefined, `a_value_too_large_for_its_destination_is_converted` for the one that
is not, and `what_a_char_holds_follows_the_target` for the fact that makes a
target a target: `c = 200` answers `-56` on `x86_64-pc-windows-msvc` and `200`
on `aarch64-unknown-linux-gnu`.

The host claim is the corpus's. Nine cases name a target explicitly and their
`.stdout` begins with `Target "x86_64-pc-windows-msvc"`, so a change that
defaulted to the host rather than honouring `--target` differs on the two CI
runners whose host is not that. `the_target_defaults_to_the_host_and_a_flag_overrides_it`
holds the other half locally, and
`the_host_this_was_built_for_is_a_target_this_compiler_knows` fails the suite on
a machine missing from the table rather than leaving somebody a bare invocation
that cannot parse.

### Consequences

* Good, because the type `docs/architecture.md` says depends on the target now
  does, and the document's table stops being aspirational for the middle row.
* Good, because the LLVM backend has somewhere to read a datalayout and a
  triple from that is not the host.
* Good, because `Ty` is unchanged, so nothing that matches on it has to answer
  for this and no artifact line about a local's type moves.
* Bad, because a `TranslationUnit` built by hand now needs a target, and every
  test that builds one gains a line.
* Bad, because the interpreter gets slower and more complicated: an operation's
  width comes from its destination place's type, so it has to ask.
* Bad, because the set of targets is a table in the crate that must not depend
  on the backend, and the backend is what will really know which targets it can
  emit for. Nothing to do until there is one to disagree with the list.
* What would reverse this: a second frontend whose notion of a type is not C's,
  which would argue for the widths moving into `Ty` where a non-C adapter can
  set them directly rather than describing a C machine.

## Pros and Cons of the Options

### The unit carries the target

* Good, because it is additive: `Ty` and every match on it are untouched.
* Good, because one place answers, so the interpreter and the backend cannot
  disagree.
* Bad, because a width is one indirection away from the type it describes.

### `Ty` carries the width

* Good, because a type says what it is worth with nothing to ask.
* Bad, because `Int` and `Char` dissolve into one integer kind with a width and
  a sign, and `--emit safety-ir` stops saying which one a program wrote.
* Bad, because the lowering would have to know the target to build a type at
  all, which puts the target in the frontend rather than below it.

### A triple, and a table on each side

* Good, because the IR carries the least it can.
* Bad, because there would be two answers to what a pointer is worth and
  nothing to keep them equal. This is the shape `shown` and `echoed` were
  deliberately not given, and RK-020 is what it costs when two paired answers
  drift.

## More Information

* [`docs/architecture.md`](../architecture.md), *What the output depends on*,
  for the table this makes true and for the `sizeof(long)` example that shows
  why uniformity is the wrong fix.
* [`docs/frontend.md`](../frontend.md), *What the interpreter answers
  differently*, for the three rows this changes.
* C17 6.5 p5 for a signed overflow, 6.5.7 p4 for a shift that is not
  representable, 6.3.1.3 p3 for a conversion that does not fit, and 6.5.16.1 p2
  for the conversion at an assignment. Read in N2310, which carries the C17 text
  with change bars for C2x; none of those paragraphs is marked.
* Widths measured with `clang 20.1.6 -dM -E` per target, rather than recalled.
