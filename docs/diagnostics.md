# Diagnostics

Diagnostics are a first-class feature because safety analysis is only useful if developers can understand why something is unsafe.

Example:

```text
error[SC0601]: use of moved value: `p`

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

`ariadne` renders diagnostics against their source. Its output differs in detail
from the sketch above, which is drawn in the shape rustc uses.

## Codes

A code is `SC` and four digits. `SC` is Safe C.

It is the handle a reader keeps: a message is reworded whenever a better wording
is found, and the code is what a suppression list, a grep or a bug report still
matches afterwards. So a code is assigned once and never changes, which
`each_lexical_diagnostic_keeps_the_code_it_was_assigned` in
`crates/safec/src/lexer.rs` states as a rule and holds for the lexical five.

The prefix names this compiler rather than a severity, because
[ADR-0001](adr/0001-promote-unproven-results-in-the-sink.md) lets the sink
promote a diagnostic from a warning to an error while its code stays put:
`warning[SC0601]` and `error[SC0601]` are the same diagnostic under two
policies. [ADR-0009](adr/0009-name-a-diagnostic-code-after-the-compiler-and-the-topic.md)
records why the prefix is this one and what was rejected.

### What each range holds

| Range | Topic | In use |
|---|---|---|
| `SC00xx` | not a topic: examples and tests, never emitted by the compiler | `SC0001` |
| `SC01xx` | lexical, what a character or a token is | `SC0101`, `SC0102`, `SC0103`, `SC0104`, `SC0105` |
| `SC02xx` | syntax, what a sequence of tokens is | `SC0201`, `SC0202` |
| `SC03xx` | names and types | `SC0301`, `SC0302`, `SC0303`, `SC0304` |
| `SC04xx` | memory | none yet |
| `SC05xx` | lifetime | none yet |
| `SC06xx` | ownership | `SC0601` |
| `SC07xx` | thread | none yet |
| `SC08xx` | code generation, what a backend can write | `SC0801` |
| `SC09xx` | free | none |

`SC04xx` through `SC07xx` are the four safeties [`concept.md`](concept.md) asks
the question about, in the order it names them, so they are reserved before
anything can emit from them.

`SC00xx` is the exception that keeps the rest honest. A renderer test needs a
code that means nothing, and a code that means nothing has to come from
somewhere that no diagnostic will ever claim.

`SC0601` is a reservation rather than a diagnostic. The use-after-move sketched
at the top of this document is not implemented, and the doc comments and tests
that carry that code are illustrations of one.

### Allocating the next one

* A code comes from the range of its **topic**, not of the stage that emits it.
  Which pass finds a problem is an internal fact that can change; what the
  problem is about does not, and a user with a code in a suppression list should
  not pay for a check moving.
* Inside a range, codes are handed out in order of first use from `01`, never
  reused and never renumbered. A retired diagnostic leaves its number retired.
* A new topic takes the lowest free hundred, and **the table above is updated in
  the change that first emits from it**, not afterwards. A topic that runs past
  `99` takes a second hundred from the free end and is recorded the same way.

Half of this is held by the compiler and half is not, which is worth knowing
before relying on either. `Code::new` asserts the spelling, and every code is a
`const`, so a code spelled any other way stops the build at its declaration.
Which range a code belongs in is a question only a person can answer, since it
turns on what the new diagnostic is about, so that half is held by the reviewer
of the change that adds it.

### What carries no code

A diagnostic about the invocation rather than about a program's text has no
code. `no input files` and `cannot read <path>` are that kind, and five of the
six diagnostics in `crates/safec/src/driver.rs` carry none. There is no class of
program for a reader to search for and nothing for an explanation to hang on, so
a number there would be a handle onto nothing.

`SC0801` is the sixth and is the other kind. The backend refusing an IR shape is
a fact about a function in a program, with a span to point at and a class of
program to search for, so it takes a code the way the lowering's `SC0304` does.
The refusal is made in `crates/safec-llvm`, which cannot see a `Diagnostic` at
all, so the code is attached where the diagnostic is built.

## Where this lives now

Implemented in `crates/safec/src/diagnostics.rs`, with the terminal renderer in
`crates/safec/src/diagnostics/render.rs`. What a check concludes, and how that
becomes a severity, is [ADR-0001](adr/0001-promote-unproven-results-in-the-sink.md).
