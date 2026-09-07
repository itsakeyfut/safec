# Concept

**Small, Hackable, Safety-Oriented Compiler**

A small experimental C compiler designed from the beginning around:

- Memory Safety
- Lifetime Safety
- Thread Safety
- Ownership
- Static Analysis

The goal is not to create a Rust clone or merely another tiny C compiler. The goal is to create a **small, understandable, hackable reference implementation for experimenting with safety mechanisms in the C language family**.

The ultimate objective is not simply to build another compiler.

It is to explore:

> **How much of Rust's memory, lifetime, ownership, and thread safety can be brought into the C ecosystem while preserving C's familiarity, interoperability, and incremental migration path?**

The compiler is the small experimental laboratory for answering that question.

## Core Philosophy

> Small enough to understand.  
> Hackable enough to experiment.  
> Safe enough to trust.

The compiler itself should remain small enough that one developer can understand the major components and their interactions.

## Why C Rather Than Rust?

Rust already provides a mature ownership and lifetime system, so implementing another Rust-like compiler would quickly become a large project involving:

- Ownership
- Borrow checking
- Lifetimes
- Traits
- Generics
- Macros
- Async
- and many other language features

C offers a different opportunity.

C is widely used, has a huge existing codebase, and already has an established ecosystem. A C-oriented safety compiler can therefore aim for **incremental migration** rather than requiring an entire application or codebase to be rewritten in a new language.

The conceptual direction is similar to how C++ historically extended C:

```text
C
│
└── C++ 
    ├── classes
    ├── virtual functions
    ├── templates
    └── RAII
```

This project explores:

```text
C
│
└── Safe C
    ├── Memory Safety
    ├── Lifetime Safety
    ├── Thread Safety
    └── Ownership
```

The important goal is not to immediately replace C, but to make it possible to **gradually introduce safety guarantees into existing C code**.

The analogy is about extending a language, not about C++ being a layer that can be added to a C compiler. C++ is not a superset of C, and more to the point the parser is not where the difficulty lies: C++ runs code nobody wrote, branches where no statement branches, and means several things per piece of source. C++ is therefore reached through the [Clang adapter](clang-integration.md) rather than by growing this project's frontend, and what that demands of the design is in [c-family.md](c-family.md).

Starting with C is not starting with the easy case. In C, ownership is written nowhere, so the analysis has to infer what the language never recorded; in C++ much of it is already spelled out by `std::unique_ptr` and friends. The safety model is being built against the harder of the two.

## Project Positioning

The project should not compete with TinyCC simply by being another small compiler.

TinyCC's "small" property is itself a major feature. This project should instead make smallness a means to another goal:

> A compiler small enough to make compiler and safety-system experimentation practical.

Therefore the positioning is:

**Small, Hackable, Safety-Oriented Compiler**

Potential characteristics:

- Small compiler core
- Simple and inspectable IR
- Fast compilation
- Built-in static analysis
- Memory safety analysis
- Lifetime analysis
- Ownership analysis
- Thread safety analysis
- LLVM backend
- Embeddability
- Potential Clang interoperability

## Development Language

### Rust

Use **Rust** as the implementation language.

Reasons:

1. The project itself deals with memory safety.
2. Rust provides strong tools for representing ASTs, IRs, ownership states, and compiler data structures.
3. Rust makes arena-based compiler data structures practical.
4. The compiler can remain memory-safe while experimenting with C safety.
5. Rust is already part of the project's technical background.

The project should avoid becoming dependent on a large framework that hides compiler fundamentals.

## Working Name and Taglines

Current working concept:

**Safe C Compiler**

Core tagline:

> **Small, Hackable, Safety-Oriented Compiler**

Potential explanatory tagline:

> A small experimental C compiler for memory, lifetime, ownership, and thread safety.

The project name is not yet finalized.
