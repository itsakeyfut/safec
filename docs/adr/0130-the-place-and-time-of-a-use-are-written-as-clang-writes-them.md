---
status: "accepted"
date: 2026-11-25
decision-makers: itsakeyfut
---

# The place and time of a use are written as `clang` writes them

## Context and Problem Statement

ADR-0055 defines `__FILE__`, `__LINE__`, `__DATE__` and `__TIME__`, so
`#ifdef` answers them, and refuses using one in code as `SC0104`, because
what each expands to was not written yet. #1219 writes it. C17 6.10.8.1
fixes the form of each but leaves the date and time of a build, and which
line a use spread over several lines is on, to the implementation.

## Decision Drivers

* a program built here should print what the same program built by the
  compiler it replaces prints
* the corpus has to bless the same output on every run

## Considered Options

* Write each as `clang` does, `SOURCE_DATE_EPOCH` included.
* Keep `__DATE__` and `__TIME__` refused, and write only the place.

## Decision Outcome

Chosen option: **as `clang` writes them**, measured with `clang 22.1.8
-E`:

* `__DATE__` is `"Mmm dd yyyy"`, the day padded with a space, and
  `__TIME__` is `"hh:mm:ss"`. Both are the time the translation started,
  or, where `SOURCE_DATE_EPOCH` is set to a number of seconds, that
  instant; either in UTC (`SOURCE_DATE_EPOCH=0` gives `"Jan  1 1970"` and
  `"00:00:00"`). The corpus sets it.
* **UTC where `clang` writes local time.** `std` cannot say what the local
  offset is, and the workspace forbids the `unsafe` a call to
  `localtime` needs, so local time would be a dependency carrying a time
  zone database. The two differ by the offset, at most a day in the date;
  the strings keep their length, so nothing a check concludes changes.
* `__FILE__` is the presumed name: the input's or the included file's
  path as it was named, on the command line or in the `#include`, or the
  name the last `#line` gave.
* `__LINE__` is the presumed line of the use: inside an expansion, the
  line of the outermost use's last token, so in a call spread over lines
  it is the line of the `)`.

Keeping the date and time refused was rejected because a program that
prints its build date is ordinary, and what either value is cannot change
what a check concludes: each is a string literal of a fixed length.

This replaces ADR-0055's sentence that using one in code is refused, and
nothing else of that record.

### Confirmation

* Taking the clock where `SOURCE_DATE_EPOCH` is set, or the line of a
  call's name rather than of its `)`, or of an argument's use rather than
  where it was written:
  `the_place_and_time_of_a_use_are_what_clang_writes` in `tests/run`,
  built with `SOURCE_DATE_EPOCH=0`, runs to another exit code.
* Counting the date from 1 January, or padding the day with `0`:
  `the_date_and_time_of_a_number_of_seconds_are_what_utc_says` in
  `crates/safec-c/src/preprocessor.rs`.
* `#line` changing the name and the line is owed by #1220.

### Consequences

* Good, because a program's output matches the one it is migrated from.
* Bad, because two builds an hour apart are not byte-identical unless
  `SOURCE_DATE_EPOCH` is set, as with `clang`; and without it, a build
  away from UTC prints a time `clang` would not.
* What would reverse this: a program whose behaviour depends on
  `__TIME__` being local, which would justify the dependency.

## More Information

#618, #1219; ADR-0055.
