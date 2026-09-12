# Key Design Principles

## 1. Safety IR is the central abstraction

Do not make LLVM IR the safety model.

LLVM IR is a code-generation representation. The project needs a higher-level semantic representation capable of expressing:

- ownership
- lifetime
- borrowing
- regions
- thread relationships

It is also defined by the analyses rather than by the frontend that feeds it. A
second frontend is on the roadmap and it carries C++, so an IR shaped around the
C AST is one that has to be rebuilt to reach it. The testable form of this: an
analysis should run over IR built by hand in a test, with no frontend present.
See [c-family.md](c-family.md).

## 2. Separate inference from enforcement

The analyzer distinguishes `Safe`, `Unsafe` and `Unknown` rather than pretending
that every C program can be proven safe. A check says what it concluded and
never reads the policy; what each conclusion means for the build is decided in
one place.

The mapping, and where it is applied, are in
[the safety model](safety-model.md#safe-unsafe-unknown).

## 3. Gradual adoption

Existing C code should be able to enter the system without requiring an immediate rewrite.

## 4. Small core

Avoid unnecessary language features and framework complexity.

## 5. Hackability over abstraction purity

The architecture should make experiments easy.

## 6. Real C compatibility is a long-term goal

Do not sacrifice the initial learning and experimentation loop by attempting full C compatibility immediately.

## 7. The compiler and analyzer should reinforce each other

New safety semantics should be testable in the experimental compiler and eventually transferable to Clang-based analysis.

## 8. A failure has to be readable

What the compiler does when it is wrong is part of the design rather than an
accident of it. Where two designs are both correct, prefer the one whose failure
a reader can see: a build that stops with a name over one that hangs, and either
over a program that passes unchecked.

What is printed for a program the analysis rejects is
[diagnostics](diagnostics.md); this is about everything else.
