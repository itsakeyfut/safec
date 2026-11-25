---
status: "accepted"
date: 2026-11-25
decision-makers: itsakeyfut
---

# A pragma is read, ignored, or refused by what ignoring it would change

## Context and Problem Statement

Every `#pragma` is refused as `SC0104` today, and ADR-0056 says any other
than `clang assume_nonnull` stays refused until something reads it. System
headers and ordinary code are full of pragmas, so that refusal stops a
program at its first `#include`. C17 6.10.6 p1 lets an implementation
ignore a pragma it does not recognise, but some pragmas change what a
program is: `pack` changes a struct's layout, and a layout that disagrees
with the other translation units' is a miscompile nothing reports.

## Decision Drivers

* a pragma ignored that changes layout, code, linking or what a macro
  means is a program compiled as something other than what was written
* a pragma refused that changes nothing stops a build for no reason
* a pragma this has never heard of is either, and the reader should see
  that it was passed over

## Considered Options

* Refuse the pragmas known to change meaning, ignore the ones known not
  to, and warn about and ignore every other.
* Refuse every pragma not known to be harmless.
* Ignore every pragma not read, as C allows, refusing only `pack`.

## Decision Outcome

Chosen option: **three classes, and a warning for the unknown.**

* **Read**: `#pragma once` (a file already read once, by its canonical
  path, is not read again), and `_Pragma("...")`, which is the `#pragma`
  its string spells (C17 6.10.9).
* **Ignored, silently**, because ignoring them changes no program's
  meaning here: `STDC FP_CONTRACT`, `STDC FENV_ACCESS` and
  `STDC CX_LIMITED_RANGE` (a translation that does not contract, and does
  not reorder floating-point operations, is one each allows);
  `GCC diagnostic`, `clang diagnostic`, `warning` and `message`, which
  speak to another compiler's diagnostics; `GCC system_header`; `region`
  and `endregion`; and `GCC visibility`.
* **Refused as `SC0104`**, because ignoring them changes layout, code,
  linking or what a later line means: `pack`, `align` and `options align`,
  `ms_struct`; `push_macro` and `pop_macro`; `redefine_extname` and
  `weak`; `comment`, which names a library to link; `section`,
  `code_seg`, `data_seg`, `bss_seg`, `const_seg` and `init_seg`;
  `GCC poison`; and `clang assume_nonnull`, until ADR-0056 is built.
* **Any other is warned about** as `SC0118`, "this pragma is not read,
  and is ignored", and ignored. A pragma in a group that is skipped is not
  read at all, as any directive there is not.

A pragma is classified by its first one or two tokens after `pragma`, as
written; `STDC` and `GCC` and `clang` are namespaces and are read with the
name after them.

Refusing every unknown pragma was rejected because a system header that
uses one this list has not met yet would stop every program that includes
it, which is a false refusal at the most common line there is. Ignoring
every one silently was rejected because a pragma that does change meaning
and is not on the refused list would then be passed over with nothing on
the screen; the warning is what moves that failure from silent to seen.

This replaces ADR-0056's sentence that any other `#pragma` stays refused,
and nothing else of that record.

### Confirmation

Owed by #1220, which names one case per class: `#pragma once` read twice
compiles once; a `STDC` pragma compiles with no warning; `#pragma pack(4)`
is `SC0104`; an unknown pragma is `SC0118` and compiles. Mutations: read
`pack` as ignored, and its case builds; drop the warning, and the unknown
case's `.stderr` loses it.

### Consequences

* Good, because a header's pragmas no longer stop the program that
  includes it, and those that would change it still do.
* Bad, because a pragma that changes meaning and is on neither list is
  compiled past with a warning, not refused.
* What would reverse this: a pragma met in a real header that changes
  meaning and was warned about, which would argue for refusing the unknown.

## More Information

#618, #1220; ADR-0056.
