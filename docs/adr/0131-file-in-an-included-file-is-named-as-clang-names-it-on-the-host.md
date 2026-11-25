---
status: "accepted"
date: 2026-11-25
decision-makers: itsakeyfut
---

# `__FILE__` in an included file is named as `clang` names it on the host

## Context and Problem Statement

ADR-0130 writes the place and time of a use as `clang` writes them, and
said `__FILE__` in an included file is the path as the `#include` named it.
Review of #1219 measured `clang 22.1.8 -E` on Windows and found the two
claims disagree: for `#include "sub/h.h"` in `inc.c`, `clang` writes
`.\sub/h.h`, and for `#include "g.h"` in that header, `.\sub\g.h`. A program
that prints `__FILE__`, or compares it, is a different program here if the
spellings differ.

## Decision Drivers

* ADR-0130's driver, that a program built here prints what the same
  program built by the compiler it replaces prints
* a diagnostic names a file the same way on every host, which
  `resolved`'s `/` was chosen for

## Considered Options

* Write `__FILE__` in an included file as `clang` does: the directory of
  the includer's name, `.` where it has none, joined to the name the
  `#include` wrote by the host's separator.
* Keep the name as the `#include` wrote it, the same on every host, and
  record that it is not `clang`'s.

## Decision Outcome

Chosen option: **as `clang` does.** `__FILE__` of an input is its path as
it was named, and of an included file its includer's `__FILE__` up to the
last separator, `.` where there is none, then the host's separator, then
the name the `#include` wrote. A diagnostic still names the file as
`resolved` does, with `/`, so only the value a program sees follows the
host, as it follows the host under `clang`.

Keeping the `#include`'s spelling was rejected because it holds the value
the same across hosts that `clang` itself does not, and so differs from
`clang` on every one of them.

This replaces ADR-0130's sentence that an included file's `__FILE__` is
its path as the `#include` named it, and nothing else of that record.

### Confirmation

Writing the name as the `#include` wrote it, or joining with `/` on
Windows: `a_file_macro_in_a_header_names_it_as_clang_does` in
`crates/safec-c/src/preprocessor.rs` fails on that host, its expected
value built from the host's separator.

### Consequences

* Good, because `__FILE__` in a header matches the compiler being
  replaced on the host it runs on.
* Bad, because a test of the value has to say which host it expects, and
  `clang` on Linux, `./sub/h.h`, was not measured here.
* What would reverse this: `clang` on another host measured writing
  something else.

## More Information

#1219; ADR-0130.
