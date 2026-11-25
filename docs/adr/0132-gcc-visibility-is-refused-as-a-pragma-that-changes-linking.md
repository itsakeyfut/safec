---
status: "accepted"
date: 2026-11-25
decision-makers: itsakeyfut
---

# `GCC visibility` is refused as a pragma that changes linking

## Context and Problem Statement

ADR-0129 sorts pragmas by what ignoring them would change, refusing those
that change layout, code, linking or what a later line means, and lists
`GCC visibility` among those ignored. Review of #1220 measured it: for
`x86_64-unknown-linux-gnu`, `clang 22.1.8` writes a function and an object
between `#pragma GCC visibility push(hidden)` and `pop` as `hidden`, and
safec, ignoring the pragma, wrote them with default visibility and said
nothing. An object built so and linked into a shared library exports
symbols `clang` would hide, which another module can then interpose.

## Decision Drivers

* ADR-0129's own driver, that a pragma ignored which changes linking is a
  program compiled as something other than what was written
* `__attribute__((visibility(...)))`, the other spelling of the same
  setting, is refused today, so ignoring the pragma left one spelling
  passed over in silence

## Considered Options

* Refuse `GCC visibility`, as `pack` is refused.
* Keep it ignored, since an executable that is not a shared library sees
  no difference.

## Decision Outcome

Chosen option: **refuse it**, as `SC0104`, "`#pragma GCC visibility` is
not read", until visibility is something this compiler writes.

Keeping it ignored was rejected because the difference it makes is in
what a shared library exports, which nothing in the build reports and the
program's own tests do not exercise.

This replaces ADR-0129's listing of `GCC visibility` among the pragmas
ignored, and nothing else of that record.

### Confirmation

Reading `GCC visibility` as ignored again:
`a_pragma_is_classified_as_adr_0129_lists_it` in
`crates/safec-c/src/preprocessor/directive.rs` fails, and
`a_pragma_that_changes_the_program_is_refused` loses its report.

### Consequences

* Good, because a header that hides its symbols stops the build rather
  than being compiled to export them.
* Bad, because such a header is refused where it built before as an
  executable that would have run the same.
* What would reverse this: visibility written by this compiler's code
  generation, at which point the pragma is read rather than refused.

## More Information

#1220; ADR-0129.
