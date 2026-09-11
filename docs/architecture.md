# Architecture

The conceptual pipeline is:

```text
C Source
   ↓
Lexer
   ↓
Parser
   ↓
AST
   ↓
Semantic Analysis
   ↓
Typed AST
   ↓
Safety IR
   ↓
Safety Analysis
   ├── Memory
   ├── Lifetime
   ├── Ownership
   └── Thread
   ↓
Code Generation
   ↓
LLVM IR
   ↓
Native Executable
```

The Safety IR should be a central architectural boundary.

Do not directly couple:

```text
AST → LLVM
```

Instead:

```text
AST
 ↓
Safety IR
 ↓
Backend
```

This makes it possible to add:

- LLVM backend
- Interpreter
- WASM backend
- C backend
- other future backends

without coupling safety analysis to a particular code generator.

## What the output depends on

> **Output depends on the target, never on the host.**

The host is the machine `safec` runs on. The target is the machine it compiles
for. Every stage in the pipeline above answers to the second and to nothing
else, so that a run on Windows compiling for Linux produces what a run on Linux
compiling for Linux produces, byte for byte.

| Output | Depends on the host | Depends on the target |
|---|---|---|
| diagnostics | no | no |
| tokens, AST, Safety IR | no | yes, once type widths reach them |
| LLVM IR, an object, an executable | no | yes, by definition |
| what a safety analysis concludes | no | yes, where it turns on a width |

Two mistakes hide behind the word "multi-platform", and they point opposite
ways.

**Do not make the artifact uniform.** `sizeof(long)` is eight bytes on
`x86_64-unknown-linux-gnu` and four on `x86_64-pc-windows-msvc`. A compiler that
reports the same number for both is lying about the target, and the fact that
one clang binary gives both answers from one host is the whole point.

**Do not let the host into a diagnostic.** A diagnostic is this compiler's own
speech and is a search key: a user who hits one looks for its words, and two
users on different machines have to be able to find the same page. Text that
varies by host turns one problem into three.

### Where this cannot be reached

Three, and saying so is part of the intent rather than a caveat on it.

- **A path the user typed.** `safec` echoes the path it was given, so a Windows
  user sees `\`. Normalising it would print something they cannot paste back
  into their shell. What is unified is this compiler's words, not the user's.
- **Filesystem case.** `#include "Foo.h"` finds `foo.h` on Windows and macOS and
  not on Linux. Deciding it uniformly means rejecting code that really does
  compile where the user is. This arrives with the preprocessor.
- **An error kind with no name of ours.** `std::io::ErrorKind` is
  `#[non_exhaustive]`, so however many kinds are given words of their own, some
  failure will always fall through to what the operating system said.

### A known divergence

`cannot read` puts the operating system's message in a note, so it reads
`The system cannot find the file specified. (os error 2)` on Windows and
`No such file or directory (os error 2)` on Linux. `clang` does not do this: it
says `no such file or directory` in its own words on every platform.

This is a divergence rather than a defect. Quoting the underlying error is what
most tools do and it carries real information. It is recorded here because the
rule above is what this project wants, and something that contradicts it should
be visible rather than discovered twice.

The shape of the fix, when it is taken, is to name the common kinds in this
compiler's own words and keep the operating system's string for the rest, rather
than to drop the note: a diagnostic made testable by saying less is a worse
diagnostic. It is tracked as issue 17, and this section goes when that lands.

## What Defines the Safety IR

The IR is defined by what the analyses need, not by what the C frontend
produces. An IR shaped around the C AST is an IR that a second frontend cannot
reach, and there is already a second frontend on the roadmap: the
[Clang adapter](clang-integration.md), which carries C++.

Two consequences are worth stating early, because both are cheap while the IR
does not exist:

- An IR operation must be able to say it was not written. C++ destroys objects,
  copies them and destroys temporaries without a statement saying so, and the
  attribution has to distinguish "written here" from "caused by this, and
  generated". `Span` already anticipates the coordinate this needs.
- The control-flow graph must be able to carry an edge no statement produced.
  This is Phase 2 work rather than Clang-adapter work, because every analysis
  after it walks that graph.

[c-family.md](c-family.md) has the rest, and the trigger for each.

## Interpreter

A small interpreter for the Safety IR is strongly recommended.

```text
Safe C
  ↓
Safety IR
  ↓
Interpreter
```

Advantages:

- Fast compiler tests
- Easier debugging
- Deterministic execution
- Easier experimentation with language semantics
- Ability to test the compiler without invoking LLVM

The interpreter does not need to be production-grade initially.

## LLVM Integration

LLVM should be treated as a backend rather than the foundation of the compiler architecture.

Initial direction:

```text
Safety IR
   ↓
LLVM Backend
   ↓
LLVM IR
```

That arrow is now `crates/safec-llvm`, which writes textual LLVM IR and links
against no LLVM at all. This document named `inkwell` as the potential initial
library and the first backend deliberately went the other way:
[ADR-0014](adr/0014-write-llvm-ir-as-text-from-a-crate-of-its-own.md) carries
the reasoning, what it rejected, and the trigger for reversing it, which is
`--emit object`.

That trigger has fired and the answer was none of the three this document once
listed. `--emit object` asks `clang` to make an object of the text, as a child
process, so nothing here links against LLVM and nothing has to be installed to
*build* this compiler.
[ADR-0015](adr/0015-make-an-object-by-spawning-clang.md) carries that decision
and what it costs, which is that `--emit object` needs a `clang` to *run*.

Still the libraries to reach for on the day that changes:

- `inkwell`
- `llvm-sys`
- LLVM C API

The project should avoid making LLVM APIs leak throughout the frontend and safety-analysis code. That is a crate boundary rather than a convention: `safec-llvm` depends on `safec-ir` and on nothing else, so it cannot see a token, a tree or a diagnostic, and cargo is what says so.
