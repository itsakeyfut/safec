# Frontend Strategy

Initially use a hand-written lexer and recursive-descent parser.

Avoid depending on parser-generator libraries at the beginning.

Reasons:

- Better understanding of C parsing
- Easier experimentation
- Easier debugging
- Keeps the compiler architecture explicit
- Fits the "Hackable" philosophy

The C parser should be implemented incrementally.

## Stage 1

Support a practical subset:

- `int`
- `char`
- `void`
- variables
- functions
- `if`
- `while`
- `return`
- pointers
- arrays
- structs

## Stage 2

Add:

- `typedef`
- `enum`
- `union`
- function pointers
- variadic functions

## Stage 3

Add preprocessor functionality:

- macros
- `#include`
- conditional compilation

These stages order the parser's growth, not the project's phases, and the
preprocessor is where the two diverge. [roadmap.md](roadmap.md) pulls a minimal
subset of this stage forward into Phase 1: `#include`, object-like `#define`,
`#ifdef` and `#ifndef`. What that buys is that every later phase can be tested
against a real header rather than a hand-written prototype. The rest of the
stage, and all of Stage 2, stay in Phase 9.

## Stage 4

Work toward broader C compatibility, potentially C11/C17.

Do not target complete C compatibility from day one.
