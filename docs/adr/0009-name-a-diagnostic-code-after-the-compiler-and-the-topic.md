---
status: "accepted"
date: 2026-09-09
decision-makers: itsakeyfut
---

# Name a diagnostic code after this compiler and the topic, not after the severity

## Context and Problem Statement

A diagnostic code is a user-facing interface. Once it is in somebody's
suppression list or their grep history it cannot be renumbered, so what it looks
like has to be decided before there is anybody to break. Today it is `E` and
four digits, taken from `rustc` by a survey recorded in a doc comment on
`parser.rs`'s `EXPECTED`, and nothing says what the `E` stands for or which
range the next stage may take.

That has to be answered now because the next stage is being designed against it.
#56 needs one code for an undeclared name and cannot pick one without repeating
the survey, which is the failure #38 was filed to prevent, one phase later.

This record and the change that carries it out are one change. The allocation
itself lives in `docs/diagnostics.md`, where somebody adding a diagnostic will
look for it; what is below is the half of it that a table cannot hold.

## Decision Drivers

* **A code outlives every wording it labels.** `lexer.rs`'s
  `each_lexical_diagnostic_keeps_the_code_it_was_assigned` already states the
  rule: a message can be reworded whenever a better wording is found, and the
  code is the handle that does not move. So the cost of getting the shape wrong
  is paid once and never refunded.
* **A code cannot carry a severity, because a severity moves.** ADR-0001 has the
  sink promote an unproven result, so one diagnostic is a warning under one
  policy and an error under another while its code is the same. `render.rs`
  prints the severity outside the brackets and the code inside, and its own test
  loops over all four severities with a code attached, so `warning[E0001]` was
  a header this compiler rendered. An `E` that means "error" contradicts the
  word beside it.
* **The letters already mean two things inside this repository.** `ast.rs` cites
  `E0004`, `E0369` and `E0502`, and `docs/c-family.md` cites `E0502` and
  `E0451`; all five are `rustc`'s. `lexer.rs` and `parser.rs` define `E0101` to
  `E0202`, which were this compiler's. Nothing distinguished them, and a grep
  for `E05` returned both meanings. `E0201` is live in `rustc` as well, verified
  with `rustc --explain E0201`.
* **Grouping is what creates an allocation problem.** `rustc` groups nothing:
  its index is 517 pages from `E0001` to `E0805` in numeric order, verified on
  disk in this toolchain's `share/doc/rust/html/error_codes`. It never has to
  decide which range the borrow checker gets. Any grouping buys meaning and owes
  a record of who holds what.

## Considered Options

* **Keep `E` and four digits**, recording the collision.
* **`C` and four digits**, for the language or for "compiler".
* **`SC` and four digits**, for Safe C, with ranges by topic.
* **Names rather than numbers**, the way `clang` does it.
* **A flat sequence with no grouping**, the way `rustc` does it.

## Decision Outcome

Chosen option: **`SC` and four digits, in ranges by topic**, spelled
`error[SC0201]`. `SC` is Safe C.

The prefix is two letters so that a header stays short, carries no severity so
that promotion cannot contradict it, and is not `rustc`'s so that the two
meanings inside this repository stop sharing a spelling.

Ranges are by topic rather than by the pass that emits, because a user does not
know which pass found their problem and because a diagnostic that moves between
passes should not move its number. `docs/diagnostics.md` holds the table of what
is allocated; this record holds why there is a table at all.

The numbers of the codes already in use do not change. `E0101` becomes `SC0101`
and `E0201` becomes `SC0201`, so only the letter moves and #38's "changing any
code already in use" is honoured in the part that matters, which is the number.

### Confirmation

`each_lexical_diagnostic_keeps_the_code_it_was_assigned` in
`crates/safec/src/lexer.rs` pins all five lexical codes by string, and thirteen
expected files under `crates/safec/tests/cases/` hold a rendered code byte for
byte. Changing any code, in either half, fails them by name.

What that does **not** confirm is the shape: nothing fails if a later stage
picks a code outside its topic's range, or spells one `E`. The rule lives in
`docs/diagnostics.md` and in this record, and it is enforced by whoever reviews
the change that adds the next code. Saying so is the point, because a rule that
looks enforced and is not is worse than one that is honestly unenforced.

### Consequences

* Good, because a code no longer says something the severity beside it can
  contradict. `warning[SC0301]` and `error[SC0301]` are both honest.
* Good, because the two namespaces in this repository become distinguishable.
  `E0004` is `rustc`'s and `SC0201` is this compiler's, in a comment as much as
  in output.
* Good, because the next stage reads a table instead of surveying. That is the
  whole of what #38 asked for and it only works because the ranges mean
  something.
* Bad, because `SC` and four digits is also ShellCheck's shape, `SC2086` being
  its best known code. That is a collision of the kind this record does not
  treat as serious: ShellCheck's codes never appear in this repository and never
  appear in a C compiler's output, so nothing here has to tell the two apart.
  The `rustc` collision was different because both spellings already sit in
  these files.
* Bad, because a topic range is a guess about how many codes a topic needs, and
  a topic that overflows a hundred has nowhere obvious to go. Nothing in
  `docs/roadmap.md` suggests one will.
* What would reverse this: a decision to follow `clang` and make the stable
  handle a name rather than a number. That is a better fit for
  `docs/concept.md`'s claim that a diagnostic exists so a developer can
  understand why something is unsafe, since `use-after-move` explains itself and
  `SC0601` does not. It was not taken because this compiler already has a `Code`
  type, thirteen expected files and a rule that codes never change, and because
  a number is an index into an explanation the way `rustc --explain` is, which
  `clang` has no equivalent of. Reversing means giving that up, and it gets more
  expensive with every code allocated.

## Pros and Cons of the Options

### Keep `E` and four digits

* Good, because nothing changes and no expected file moves.
* Bad, because `E` means error and the renderer prints it beside a severity that
  may not be one.
* Bad, because the collision is the in-repository kind: two meanings, one
  spelling, in files that already contain both.

### `C` and four digits

* Good, because it names the language the compiler reads.
* Bad, because `C` and four digits is MSVC's compiler diagnostic, `C4996` being
  the one everybody has seen. MSVC is a C compiler, so this collides inside the
  world of the user rather than outside it, which is worse than `rustc`.

### `SC` and four digits, ranges by topic

* Good, because it is this compiler's own name, carries no severity, and is
  short in a header.
* Bad, because it overlaps ShellCheck's shape, weighed above.

### Names rather than numbers, as `clang` does

* Good, because the handle explains itself. `clang` has no numeric codes at all:
  a warning's handle is its flag, `-Wdollar-in-identifier-extension`, verified
  on this host.
* Bad, because `clang`'s errors that sit behind no flag have no handle at all,
  which is most of them, and because there is then nowhere for a long
  explanation to hang.

### A flat sequence, as `rustc` does

* Good, because there is no allocation to record and no range to overflow.
  `rustc` has 517 codes with 288 numbers retired or unused between `E0001` and
  `E0805`, and never had to decide who owns a range.
* Bad, because a number then carries no information at all, and this project has
  already spent two ranges saying something with them.

## More Information

* `crates/safec/src/lexer.rs` and `crates/safec/src/parser.rs`: the codes in
  use, and the doc comment on `EXPECTED` that did the survey this replaces.
* `crates/safec/src/diagnostics/render.rs`: the header that prints the severity
  outside the brackets, and the test that renders one at all four severities.
* [ADR-0001](0001-promote-unproven-results-in-the-sink.md): why a severity moves
  while a code does not.
* `docs/diagnostics.md`: the table this record explains.
