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

Potential initial library:

- `inkwell`

Possible lower-level alternatives later:

- `llvm-sys`
- LLVM C API

The project should avoid making LLVM APIs leak throughout the frontend and safety-analysis code.
