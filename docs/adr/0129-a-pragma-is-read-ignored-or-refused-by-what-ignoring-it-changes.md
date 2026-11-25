---
status: "accepted (its listing of `GCC visibility` among the pragmas ignored superseded by ADR-0132; `clang attribute` and `clang section` refused by ADR-0133)"
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

Each class is a corpus case in `tests/cases/preprocessor`, and
`a_pragma_is_classified_as_adr_0129_lists_it` in
`crates/safec-c/src/preprocessor/directive.rs` holds every name above in
its class. Leaving `#pragma once` unread defines the header's function
twice in `a_header_marked_once_is_read_once`; reading `pack` as ignored
drops its report from `a_pragma_that_changes_the_program_is_refused`;
warning about an ignored pragma gives
`a_pragma_ignoring_which_changes_nothing_is_ignored_silently` a `.stderr`;
dropping the warning empties `an_unknown_pragma_is_warned_about_and_ignored`'s.

### Consequences

* Good, because a header's pragmas no longer stop the program that
  includes it, and those that would change it still do.
* Bad, because a pragma that changes meaning and is on neither list is
  compiled past with a warning, not refused.
* What would reverse this: a pragma met in a real header that changes
  meaning and was warned about, which would argue for refusing the unknown.

## More Information

#618, #1220; ADR-0056.
