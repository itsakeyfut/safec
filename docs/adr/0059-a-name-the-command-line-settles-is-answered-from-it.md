---
status: "accepted"
date: 2026-10-24
decision-makers: itsakeyfut
---

# A name the command line settles is answered from it

## Context and Problem Statement

ADR-0055 refuses an `#ifdef` or `#ifndef` of a name whose answer depends on the compiler that builds the file. That covers a name one compiler for the target defines and another does not, and a name C reserves for the implementation that the target's compilers are not known to define. Refusing is right while safec cannot know which compiler builds the file, and it has a cost: a header guarded by a reserved name, such as `_FOO_H`, is refused at every inclusion. #561 tried three times to read such a guard without knowing the compiler. Review found a way through each design that let a double free be analysed away.

`-D` and `-U` are how a build tells `cc` what is defined. #607 adds them, and with them comes a decision about what `-U` of such a name claims.

## Decision Drivers

* A refusal is visible and is never believed. An answer taken from a claim nobody checked is believed.
* `--target` is already a claim about the build that safec does not check, and output depends on it (ADR-0013).
* A name the command line names is a fact about the build the user described, whoever's compiler that is.
* Whatever safec analyses has to be one build that can exist.

## Considered Options

* A name the command line names, by `-D` or `-U`, is settled: `#ifdef` and `#ifndef` answer it from the command line, reserved or not.
* `-U` removes a definition, as `cc` does, and settles nothing: a reserved or unsettled name stays refused.

## Decision Outcome

Chosen option: **the command line settles the names it names.** `-U __GNUC__` is read as "the build being described does not define `__GNUC__`", and `#ifdef __GNUC__` skips its group instead of being refused. `-U _FOO_H` lets a header guarded by `_FOO_H` be read, once.

The build safec analyses is then the one the command line describes, as it already is for `--target`. If the user's compiler in fact predefines a name the user passed to `-U`, safec analyses a build that is not the user's, and its verdict is about that other build. The claim that makes this possible is on the command line, where the user wrote it, and nowhere else.

Two limits keep it to one build that can exist:

* **A name the target predefines is not overridden.** `-U _WIN32` for `x86_64-pc-windows-msvc`, or `-D _WIN32=2`, is refused. A `-D` with the same value is accepted, as a second identical `#define` is. The names C17 6.10.8 lists are refused under `-D` and `-U` alike.
* **A name given to both `-D` and `-U` is refused.** `cc` applies them left to right, and the argument parser collects each flag on its own, so applying them in any fixed order would describe a build `cc` would not make.

`-D` itself settles nothing new: a name it defines is a macro, and a macro is answered.

### Confirmation

Corpus cases in `tests/cases/preprocessor/`, each failing under the mutation named in `tests/cases.rs`:

* `a_reserved_name_undefined_on_the_command_line_is_answered`: answer a reserved name as refused whatever the command line says, and its `#ifdef` is `SC0104` again.
* `an_include_guard_named_with_a_reserved_name_is_read_when_the_command_line_undefines_it`: the same mutation, and the header is refused at its first inclusion.
* `a_name_this_target_predefines_cannot_be_undefined_on_the_command_line`: let `-U` remove a predefined name, and the case builds.

`a_name_both_defined_and_undefined_is_refused` in `crates/safec-c/src/cli.rs` holds the refusal of a name given to both flags, in either order: drop the check, and the run is accepted.

### Consequences

* Good, because a header guarded by a reserved name can be read, by a user who says what their build defines. This is the input #561 was held on.
* Good, because `CC=safec` in an existing build passes the same `-D` flags the real compiler gets.
* Bad, because a false `-U` produces a verdict about a build that is not the user's. Nothing in safec can catch that. The command line is the record of it.
* What would reverse this: a way to name the compiler, such as a `--compiler` option, that makes the answer a fact rather than a claim. `-U` of a name that compiler is known to define would then be refused.

## Pros and Cons of the Options

### The command line settles the names it names

* Good, because one build is analysed, and it is the one described.
* Bad, because the description is trusted.

### `-U` removes a definition and settles nothing

* Good, because nothing is answered that safec did not derive.
* Bad, because `-U` then does nothing for a reserved or unsettled name, and a header guarded by `_FOO_H` stays refused with no way to read it.

## More Information

[ADR-0055](./0055-name-the-standard-and-the-target-and-refuse-the-rest.md) is the rule this narrows. Its refusal still holds for every name the command line does not name. `gcc` documents that `-D` and `-U` "are processed in the order they are given on the command line" (<https://gcc.gnu.org/onlinedocs/gcc/Preprocessor-Options.html>, read 2026-10-24). [#607](https://github.com/itsakeyfut/safec/issues/607) is where the questions were settled.
