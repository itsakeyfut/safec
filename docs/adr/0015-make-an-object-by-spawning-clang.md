---
status: "accepted"
date: 2026-09-11
decision-makers: itsakeyfut
---

# Make an object by spawning `clang`, not by linking LLVM

## Context and Problem Statement

[ADR-0014](./0014-write-llvm-ir-as-text-from-a-crate-of-its-own.md) decided that
the backend writes textual LLVM IR, and named its own reversal trigger:

> **The trigger for reversing this is `--emit object`**, which is #92. An object
> cannot go to a stream and cannot be compared byte for byte against an
> expectation this repository wrote, so the reason above stops applying on
> exactly that day. The question then is whether to spawn `clang -x ir` or to
> link a library, and this record is what that decision reverses.

That day is this one. Text can be printed and an object cannot, so whatever
makes one is a dependency: on every machine that builds this compiler, or on
every machine that runs it, and the choice is which.

## Decision Drivers

* What it costs a contributor to build this workspace, and a CI runner to test
  it. Today that is a Rust toolchain and nothing else.
* What it costs a user to run `safec --emit object`.
* `docs/architecture.md`: LLVM is a backend rather than the foundation, and the
  frontend and the analyses must not see it.
* An artifact this compiler cannot check itself needs something outside the
  repository to check it, which `tests/llvm.rs` already established for the IR.

## Considered Options

* Spawn `clang -c -x ir` and keep the bytes it answers.
* Link `inkwell`, which wraps the LLVM C API.
* Link `llvm-sys`, or the C API directly.
* Write ELF, Mach-O, COFF and wasm objects here.

## Decision Outcome

Chosen option: **spawn `clang -c -x ir`**, with the IR on one pipe and the
object on the other.

Measured rather than argued. Every one of the eight targets in `Target::ALL`
assembles this way with no sysroot, from exactly the text `--emit llvm-ir`
already writes. The host's object links and the program answers 3, which is the
first clause of Phase 3's *Done when*. `llvm-config`, which `inkwell` needs, is
absent on the machine this was written on. CI has run a `clang --version` step
on `ubuntu-latest`, `windows-latest` and `macos-latest` since #91, and three
merges to `main` have been green with it.

Writing the formats here was never a real option and is listed because leaving
it out would suggest it had not been thought about: four formats, four
architectures each, and a relocation model per target, to produce what a tool
already on the machine produces.

**The cost is a runtime dependency, and it is the first one this compiler has.**
`safec --emit object` on a machine with no `clang` does not work, and says so in
a sentence that names the tool rather than failing later with something opaque.
That is a different claim from ADR-0014's, which only ever wanted a `clang` for
a *test*, and it is the reason this is a record rather than a line in a doc
comment. Nothing about *building* the workspace changes: no `llvm-config`, no
system package, no pinned version, and the MSRV job keeps checking a workspace
that builds on the floor.

Nothing is pinned, because nothing is linked. The floor is LLVM 15, which is
where opaque pointers became the default and which the IR this writes uses.
**That floor is taken from LLVM's release history rather than measured**:
nothing older than `clang 20.1.6` is on the machine this was written on. A
`clang` below it answers in its own words and the diagnostic passes those
through rather than interpreting them.

This does **not** supersede ADR-0014, which is a disagreement with that record
rather than an oversight: it expected "this record is what that decision
reverses", and nothing about it is reversed. ADR-0014 decided what the backend
writes, and it still writes text; this decides what a second tool makes of the
text. The two are separable, which is the point: a WASM or a C backend writes
its own text and this record says nothing about what turns that into anything.

### The pipe

The IR goes in on standard input and the object comes back on standard output,
so nothing needs a temporary file. That is not only tidiness: `clang` records
the name it was given, so a temporary path would reach the artifact, and the
same module assembled twice would not be the same bytes. Through the pipe it is,
which was measured.

Standard input is written on a thread, because both pipes are open at once and a
module larger than the pipe buffer would deadlock against a `clang` that has
begun answering before it has finished reading. About 64 KiB on this host,
measured during #91's review, which an ordinary `.c` file reaches.

### Confirmation

`an_object_is_for_the_machine_the_run_named` in `crates/safec/tests/object.rs`
makes an object for all eight targets and reads what each one is out of its own
header, against eight measurements written out rather than walked. It needs no
`llvm-objdump`: every format puts the machine at a fixed offset. Passing the
host's triple to `clang` rather than the one the run named fails seven of the
eight rows here and a different seven on each CI runner.

`an_object_the_linker_accepts` links the host's object and runs the program,
which answers 3.

`a_clang_that_answers_nothing_is_not_a_success` and
`a_clang_that_cannot_be_run_does_not_blame_the_module` hold the two ways a tool
on the path is not the tool this asked for. Spawning one is trusting a name, and
a name on `PATH` is not a promise: a cache or a distributing wrapper is
routinely installed as `clang`, so an exit status is not evidence that an object
exists, and a spawn that never started is not `clang` refusing a module. Both
arrange it with a stub the real `clang` builds, so neither needs anything new on
the machine.

`a_missing_clang_says_what_to_install` runs the compiler with an emptied `PATH`
and holds that the diagnostic names `clang` and the version it needs. It is the
only test here that needs `clang` to be *absent*, and the only thing this
compiler reads `PATH` for.

Every other test of `--emit object` says what it did not check where there is no
`clang`, and `SAFEC_REQUIRE_LLVM` makes that a failure, which CI sets. A feature
that needs a tool cannot be tested without it, so the gate is the same one
`tests/llvm.rs` already uses.

### Consequences

* Good, because building and testing this workspace still needs nothing but a
  Rust toolchain, on three runners and on a contributor's machine.
* Good, because eight targets work today rather than whichever ones a linked
  LLVM happened to be built with.
* Bad, because `--emit object` is the first thing here that does not work on a
  machine that has the compiler. A user who can run `safec` cannot necessarily
  make an object with it.
* Bad, because a process boundary is slower than a function call and hides
  whatever `clang` decides to do, including any warning it emits while
  succeeding, which this keeps rather than reports. What it no longer hides is
  a `clang` that is not one: four answers are told apart rather than two, and
  only the one where `clang` ran and spoke passes its words through.
* Neutral, because `--emit executable` in #93 meets the same question and will
  answer it the same way or say why not.

## More Information

ADR-0014 is the record this one answers. ADR-0013 is why a target is one of a
measured few, which is what makes "an object for the machine the run named" a
thing eight rows can be checked against.
