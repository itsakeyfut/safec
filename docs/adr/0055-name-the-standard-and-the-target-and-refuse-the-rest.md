---
status: "accepted"
date: 2026-10-20
decision-makers: itsakeyfut
---

# Predefine what C and the target say, and refuse every other reserved name

## Context and Problem Statement

No macro is predefined, so `#ifdef` answers every name the implementation defines as undefined. A review of #547 measured what that costs. `free(p)` under `#ifdef __STDC_VERSION__` was skipped, so a double free that every compiler builds was analysed away and the program was said to be safe. #547 stopped the silence by refusing an `#ifdef` or `#ifndef` of a name C reserves for the implementation (C17 7.1.3 p1), unless the source defines it. That refusal also stops every header written for more than one platform at its first `#ifdef _WIN32`. [#559](https://github.com/itsakeyfut/safec/issues/559) is what predefines names, and this record decides which names it predefines.

The question is about trust rather than coverage. The program safec analyses is the program it would build, but the same file is just as often built by `gcc`, `clang` or MSVC. Wherever safec's answer to an `#ifdef` differs from the compiler that builds the file, the verdict is about a program nobody runs.

## Decision Drivers

* Whatever group safec takes must be the group the building compiler takes, or safec must say that it cannot tell.
* Every value must be measured, not recalled, as `Target::ALL` is (ADR-0013).
* A name nobody listed must stay visible. Answering it as undefined is the silence #547 found.

## Considered Options

* Predefine what C17 6.10.8.1 requires and what is true of the target whichever compiler builds for it. That means the OS, the architecture, the data model, the byte order and the size of each type. Refuse every other reserved name.
* Predefine everything `clang 20.1.6 -dM -E` prints for the target, about 400 names.
* Predefine only what C17 6.10.8.1 requires.

## Decision Outcome

Chosen option: the first, the standard and the target, because these are the only names whose answer does not depend on which compiler builds the file. `_WIN32` is defined for every compiler that targets `x86_64-pc-windows-msvc`. `__GNUC__` is defined by `gcc` and by `clang`, but not by MSVC. So answering it either way would be right for one build and wrong for another, and it is refused instead.

Three rules come with it.

* **A name another target lists is a fact about this one too.** `_WIN32` is listed for the MSVC target, so for `x86_64-unknown-linux-gnu` it is answered as undefined, not refused. The set that is answered is the union of every target's list. Only a reserved name outside that union is refused.
* **Values follow `clang 20.1.6` for the triple, including where it leaves a name out.** For `x86_64-pc-windows-msvc`, `clang` does not define `__STDC__`, and neither does MSVC by default. So `__STDC__` is answered as undefined there, because another target lists it.
* **The include-guard idiom is read.** `#ifndef N` whose next directive is `#define N` answers `N` from the source alone, so a header guarded by `_FOO_H` is read. It is wrong only where the implementation predefines that exact guard name, which nothing measured does.

`__FILE__`, `__LINE__`, `__DATE__` and `__TIME__` are defined, so `#ifdef` answers them. Using one in code is refused as `SC0104`, because what it expands to is the place and time of the use, and that is not written yet. A `#define` or `#undef` of a predefined name is refused, as C17 6.10.8 p2 requires.

### Confirmation

Held by tests in `crates/safec/src/preprocessor.rs` and corpus cases in `crates/safec/tests/cases/preprocessor/`, each failing under the mutation named:

* `win32_is_defined_for_msvc` and `win32_is_not_defined_for_linux` are one program for two targets. Predefining nothing refuses `_WIN32`, and reading one target's list for both takes the wrong group in one of them.
* `an_ifdef_of_a_name_the_implementation_may_define_is_refused` asks about `__GNUC__`. Answering a reserved name no target predefines as undefined makes it exit 0, the free under it analysed away.
* `a_double_free_under_a_predefined_name_is_found` is the double free under `#ifdef __STDC_VERSION__`. Predefining nothing refuses the `#ifdef`, and the free is not analysed.
* `every_target_predefines_what_clang_says` writes two targets' measured values out rather than walking the lists. Changing a value fails it.
* `a_name_some_target_predefines_is_known` fails if only the run's own target is asked.
* `a_reserved_guard_is_read_once` and `a_reserved_name_followed_by_another_define_is_refused` hold the include-guard idiom from both sides.

### Consequences

* Good, because a header that selects on the platform is read, and it is read the way every compiler for that platform reads it.
* Good, because a header that selects on the compiler is refused where safec cannot know the answer. The refusal is visible, where the alternative is silent.
* Bad, because a header that tests `__GNUC__` or `_MSC_VER` before anything else is refused, even when it would be read correctly under any answer.
* Bad, because the values are a measurement of one `clang` release. A target added to `Target::ALL` needs its list measured too.
* What would reverse this: a way to say which compiler builds the file, such as a `--compiler` option. With one, the compiler's identity macros become facts as well.

## Pros and Cons of the Options

### The standard and the target

* Good, because every name it answers has one answer for the triple.
* Bad, because headers that test the compiler stay refused.

### Everything `clang` prints

* Good, because most headers are read, the way `clang` reads them.
* Bad, because safec then claims to be `clang`, and also `gcc` through `__GNUC__`. A file built by MSVC, or by a `gcc` whose extensions differ, is analysed through groups its compiler skips, and the verdict is believed. That is the worst failure this project has.
* Bad, because `#ifdef __GNUC__` selects the GNU extensions, which the parser refuses, so more headers are refused, not fewer.

### Only what C17 requires

* Good, because it is the smallest change.
* Bad, because `#ifdef _WIN32` is still refused, which was the cost this issue was opened to remove.

## More Information

* [#547](https://github.com/itsakeyfut/safec/issues/547) added the refusal of reserved names, and its review found the double free described above.
* `clang 20.1.6 --target=<triple> -std=c17 -dM -E` was run on an empty file for each row of `Target::ALL`, on 2026-10-20.
* [ADR-0013](./0013-the-translation-unit-carries-the-target-and-answers-what-a-type-is-worth.md) is the same stance applied to what a type is worth.
