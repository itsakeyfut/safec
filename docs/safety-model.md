# Safety Model

The core abstraction should eventually model:

- Value
- Place
- Ownership
- Lifetime
- Region
- Thread
- Capability

For example, an ownership state machine could look like:

```text
ALLOCATED
    ↓
OWNED
    ↓
MOVED
    ↓
INVALID
```

A borrowed value could be represented as:

```text
owner
  │
  └── borrow
        │
        └── lifetime tied to owner
```

The compiler should be able to detect errors such as:

```c
int *p = malloc(sizeof(int));

free(p);

*p = 42;
```

and report:

```text
error:
use of freed value 'p'

p allocated here
p freed here
dereferenced here
```

Similarly:

```c
int *foo() {
    int x;
    return &x;
}
```

should eventually produce a diagnostic such as:

```text
error:
pointer escapes lifetime of local variable 'x'
```

## Safe, Unsafe, Unknown

The analyzer should distinguish:

```text
Safe
Unsafe
Unknown
```

rather than pretending that every C program can be proven safe.

Inference is what a check concludes; enforcement is what that means for the
build. The two are separated by reporting the conclusion and deciding the
severity in one place:

| The analysis concluded | What is reported | Under `--deny-unknown` |
|---|---|---|
| Safe | nothing | nothing |
| Unsafe | an error | an error |
| Unknown | a warning | **an error** |

A check names its conclusion and never reads the policy. `DiagnosticSink` reads
it, once, so no check can forget to apply it: a missed promotion would let the
compiler exit successfully on code it never managed to check, which is the worst
thing it can do. See
[ADR-0001](adr/0001-promote-unproven-results-in-the-sink.md).

**Which column is the default is an open question.** The table above, and the
flag that resolves it in `crates/safec/src/cli.rs`, belong to a compiler that
accepts every program and comments on it. This project asks instead that a
program be rewritten until it can be proved, and under that reading a build
that succeeded while something went unproven is a build that says nothing.
[ADR-0033](adr/0033-a-conclusion-this-analysis-could-not-prove-does-not-build.md)
proposes that an unproven conclusion fail the build wherever a check runs, and
nothing has changed here yet.

`Unsafe` says that **some** execution of the function is undefined, not that
every one is. A check here reads one function at a time, so a parameter ranges
over every value a caller may pass rather than over the calls this translation
unit happens to contain: a parameter freed twice is a proved double free though
the only caller in the file passes a null pointer, which C17 7.22.3.3 p2 would
make well defined. What removes something from the question is a fact the
analysis established rather than one it could not rule out, so a pointer proved
null is exempt and a pointer nothing is known about is not. Today that
exemption reaches a free whose argument is written as a null pointer constant
and no further, which is what #188 is for. See
[ADR-0027](adr/0027-an-unsafe-conclusion-is-about-an-execution-this-function-has.md).


## Incremental Safety Levels

Avoid requiring every existing C program to become fully safe immediately.

A possible model:

```text
Level 0
  Ordinary C

Level 1
  Memory checked

Level 2
  Lifetime checked

Level 3
  Ownership checked

Level 4
  Thread checked

Level 5
  Fully safety-checked subset
```

This allows incremental adoption.

A level says **which checks run**, and nothing else. It does not decide how
loudly a check speaks: a result the analysis proved is an error at every level,
because there is no safety argument for knowing something is wrong and saying it
quietly. What varies is the result the analysis could *not* prove, and that is
governed by `--deny-unknown` rather than by the level. See
[Safe, Unsafe, Unknown](#safe-unsafe-unknown) above and
[ADR-0001](adr/0001-promote-unproven-results-in-the-sink.md).

Level 5 is defined as leaving nothing `Unknown`, so `--safety strict` implies
`--deny-unknown`.

For example:

```bash
safec main.c
```

could favor compatibility and diagnostics, while:

```bash
safec --safety=strict main.c
```

could require stronger guarantees.

The exact safety-level design is intentionally undecided and should be explored during development.

## Annotations

Pure inference cannot always recover the ownership and lifetime semantics missing from ordinary C.

For example:

```c
void foo(int *p);
```

does not necessarily tell us:

- Who owns `p`?
- How long is `p` valid?
- Can `p` be NULL?
- Can another thread access `p`?
- Is ownership transferred?

Therefore the language may eventually support explicit annotations.

Possible experimental syntax:

```c
void process(borrow int *p);

owner int *create();

void consume(owner int *p);
```

The exact syntax is not fixed.

The desired migration model is:

```text
Existing C
   ↓
Safety inference
   ↓
Warnings / uncertainty
   ↓
Add annotations where necessary
   ↓
Safety-checked C
```

This should be explored experimentally rather than decided prematurely.

What an annotation cannot reach is code this compiler never checked: another
translation unit, the C library, inline assembly, an allocator. Something has
to hold what is trusted there, or the levels above have nothing to stand on.
[ADR-0032](adr/0032-bound-what-is-unchecked-inside-a-declared-hatch.md)
proposes a declared region that claims nothing about its body and carries a
promise at its boundary, and decides the stance rather than the syntax.
