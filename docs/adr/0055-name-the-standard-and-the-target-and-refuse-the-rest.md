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

The compilers taken to build for a triple are `gcc` and `clang` for Linux and Darwin, `cl.exe` and `clang` for MSVC, and `clang` alone for `wasm32-unknown-unknown`. Each name in the union of what `clang` predefines for some target falls into one of three classes for a given triple:

* **Predefined:** `clang` defines it, and so do the other compilers. For `gcc` this means its documentation says so. For `cl.exe` it means Microsoft's documentation says so with no option. `_WIN32`, `_WIN64`, `_M_X64`, `_M_AMD64` and `__STDC_HOSTED__` are the whole MSVC list.
* **Unsettled, and refused:** one compiler defines it and another does not.
  * For MSVC, `clang` defines the GNU names, `__x86_64__` and `__SIZEOF_LONG__` among them, which `cl.exe` does not. `cl.exe` defines `__STDC__` and `__STDC_VERSION__` only under an option.
  * Elsewhere, `clang` defines `__LITTLE_ENDIAN__`, and on a 32-bit target `__ILP32__`. Neither is named in `gcc`'s documentation.
  * On Darwin, `__arm64__` is unsettled, because no `gcc` for Darwin was measured.
  * On Linux, `linux` and `unix`, and `i386` on `i686`, are unsettled though C does not reserve them, because `gcc` and `clang` define them in their default GNU modes and not under `-std=c17`. A mode is as much a part of the build as a compiler is.
* **Undefined:** the name is in the union, but no compiler for this triple defines it. `__linux__` for MSVC is one, and `_WIN32` for Linux is another.

A reserved name outside the union is refused, as #547 refused it, unless the source defines it.

A first version took every value from `clang`'s list for the triple. Review found two problems with that, and the classes above replace it:

* `cl.exe` does not define `__x86_64__`, so `#ifdef __x86_64__` took a group `cl.exe` skips, and a double free in that group was analysed away.
* Answering a name in the union as undefined, wherever this target's list leaves it out, answers wrongly for any compiler that does define it.

**An include guard named with a reserved name is refused**, as any other `#ifndef` of one is. A guard such as `_FOO_H` therefore stops its header at every inclusion. Three versions that read the guard idiom were each found by review to let a verdict through:

* The pair taken anywhere passed `#ifndef _MT` inside a function. `cl.exe` predefines `_MT`, so the double free it compiles in the `#else` was analysed away.
* The pair taken as a fallback passed `#define __SIZE_TYPE__ char`.
* The pair taken only around a whole file passed a header that defines a macro another file's `#ifdef` asks about. A compiler that predefines the guard's name skips the whole header, so that macro is undefined in its build, and the double free under that `#ifdef` was analysed away.

Whether a header is read decides more than its own declarations. Reading the idiom safely means treating every macro a guarded header defines or undefines as unknown to later conditionals. That is left to [#561](https://github.com/itsakeyfut/safec/issues/561).

`__FILE__`, `__LINE__`, `__DATE__` and `__TIME__` are defined, so `#ifdef` answers them. Using one in code is refused as `SC0104`, because what it expands to is the place and time of the use, and that is not written yet.

C17 6.10.8 p2 forbids a `#define` or an `#undef` of the names that subclause lists, so one is refused as `SC0117`. A target's own names are not 6.10.8's. `__linux__` may be defined again the same way, or undefined, as `clang` accepts.

### Confirmation

Held by tests in `crates/safec-c/src/preprocessor.rs` and corpus cases in `crates/safec-c/tests/cases/preprocessor/`, each failing under the mutation named:

* `win32_is_defined_for_msvc` and `win32_is_not_defined_for_linux` are one program for two targets. Predefining nothing refuses `_WIN32`, and reading one target's list for both takes the wrong group in one of them.
* `an_ifdef_of_a_name_the_implementation_may_define_is_refused` asks about `__GNUC__`. Answering a reserved name no target predefines as undefined makes it exit 0, the free under it analysed away.
* `a_double_free_under_a_predefined_name_is_found` is the double free under `#ifdef __STDC_VERSION__`. Predefining nothing refuses the `#ifdef`, and the free is not analysed.
* `a_name_its_compilers_disagree_on_is_refused_for_msvc` puts the double free `cl.exe` builds under `#else` of `#ifdef __x86_64__`. If unsettled names are answered from the lists, it is reported as a double free rather than refused.
* `every_target_predefines_what_its_compilers_agree_on` writes two targets' values, and their unsettled names, out rather than walking the lists. Changing a value, or moving a name between the classes, fails it.
* `a_name_some_target_predefines_is_known` fails if only the run's own target is asked.
* `a_reserved_guard_is_refused` is the third of those programs. Reading `#ifndef N` followed by `#define N` as a guard makes it exit 0.
* `a_gnu_mode_s_name_is_refused_for_linux` fails if `unix` is answered as undefined on Linux.
* `a_predefined_macro_defined_again_is_refused` and `a_target_s_name_may_be_defined_again` hold `SC0117` to the names 6.10.8 lists.

### Consequences

* Good, because a header that selects on the platform is read, and it is read the way every compiler for that platform reads it.
* Good, because a header that selects on the compiler is refused where safec cannot know the answer. The refusal is visible, where the alternative is silent.
* Bad, because a header that tests `__GNUC__` or `_MSC_VER` before anything else is refused, even when it would be read correctly under any answer, and so is a header guarded by a reserved name such as `_FOO_H`.
* Bad, because the values are a measurement of one `clang` release, and the other compilers' side is their documentation, not a run. `gcc`'s operating system and architecture names, `__linux__` and `__x86_64__` among them, are taken as shared with `clang` without being measured here. A target added to `Target::ALL` needs its lists measured too.
* Bad, because a header that tests `__x86_64__` to choose code for MSVC is refused, even though `_M_X64` would have answered.
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
* `cl.exe`'s side is <https://learn.microsoft.com/en-us/cpp/preprocessor/predefined-macros>. `gcc`'s side is <https://gcc.gnu.org/onlinedocs/cpp/Common-Predefined-Macros.html> and <https://gcc.gnu.org/onlinedocs/cpp/Standard-Predefined-Macros.html>. All three were read on 2026-10-20.
* [ADR-0013](./0013-the-translation-unit-carries-the-target-and-answers-what-a-type-is-worth.md) is the same stance applied to what a type is worth.
