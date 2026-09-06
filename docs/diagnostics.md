# Diagnostics

Diagnostics are a first-class feature because safety analysis is only useful if developers can understand why something is unsafe.

Example:

```text
error[E0301]: use of moved value: `p`

  --> main.c:12:5
   |
 8 |     owner int* p = alloc();
   |                 - move occurs here
 9 |
10 |     consume(p);
   |             - value moved here
11 |
12 |     *p = 42;
   |     ^^ value used here after move
   |
   = note: `p` was moved into `consume`
```

`ariadne` is a possible implementation library for source-oriented diagnostics.

## Where this lives now

Implemented in `crates/safec/src/diagnostics.rs`, with the terminal renderer in
`crates/safec/src/diagnostics/render.rs`. What a check concludes, and how that
becomes a severity, is [ADR-0001](adr/0001-promote-unproven-results-in-the-sink.md).
