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
