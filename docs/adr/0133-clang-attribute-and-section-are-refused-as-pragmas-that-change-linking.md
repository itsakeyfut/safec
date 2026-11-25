---
status: "accepted"
date: 2026-11-26
decision-makers: itsakeyfut
---

# `clang attribute` and `clang section` are refused as pragmas that change linking

## Context and Problem Statement

ADR-0129 warns about and ignores every pragma it does not name, and
ADR-0132 moved `GCC visibility` to the refused, because ignoring it
exported symbols `clang` hides. Review of #1345 found the same with a
pragma ADR-0129 does not name: `clang 22.1.8` writes a function between
`#pragma clang attribute push(__attribute__((visibility("hidden"))),
apply_to = function)` and `pop` as `hidden` for
`x86_64-unknown-linux-gnu`, and safec, warning and ignoring it, wrote it
with default visibility. `clang section` places what it covers in a named
section, which a linker script or another unit can depend on.

## Decision Drivers

* ADR-0129's driver, that a pragma ignored which changes layout, code or
  linking is a program compiled as something other than what was written
* a warning is passed over in a build that prints many, and the program
  it warns about is still built

## Considered Options

* Refuse `clang attribute` and `clang section`, as `GCC visibility` is.
* Leave them unknown, warned about and ignored.

## Decision Outcome

Chosen option: **refuse both**, as `SC0104`, "`#pragma clang attribute`
is not read" and "`#pragma clang section` is not read", until this
compiler writes attributes and sections.

Leaving them warned about was rejected because what each changes is what
a library exports or where its code lives, which nothing in the build
then reports, as ADR-0132 found for `GCC visibility`.

This adds two names to ADR-0129's refused class, and changes nothing else
of that record. The floating-point pragmas, `clang fp` and
`float_control`, stay unknown until this compiler reads a floating type.

### Confirmation

Reading either as unknown again:
`a_pragma_is_classified_as_adr_0129_lists_it` in
`crates/safec-c/src/preprocessor/directive.rs` fails, and
`a_pragma_that_changes_the_program_is_refused` loses its reports.

### Consequences

* Good, because a header that hides or places what it declares stops the
  build rather than being compiled to do otherwise.
* Bad, because such a header is refused where it built before with a
  warning.
* What would reverse this: attributes and sections written by this
  compiler's code generation.

## More Information

#1345; ADR-0129; ADR-0132.
