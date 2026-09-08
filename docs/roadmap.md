# Roadmap

## Initial MVP

The first working program should be intentionally small:

```c
int add(int a, int b) {
    return a + b;
}

int main() {
    return add(1, 2);
}
```

Getting that far means a frontend, a Safety IR, and something that runs the IR.
Native code is not required for it: the interpreter in
[architecture.md](architecture.md) exists so that the compiler can be tested
without LLVM, and it arrives first.

Then memory safety:

```c
int *p = malloc(sizeof(int));
*p = 42;
free(p);
*p = 10;   // detect
```

Then lifetime safety:

```c
int *foo() {
    int x = 42;
    return &x;  // detect
}
```

Then ownership:

```text
owner -> move -> invalid
```

Finally thread safety.

## Phases

The pipeline is in [architecture.md](architecture.md) and is not repeated here.
**The order of the phases is not the order of the pipeline.** Code generation is
Phase 3 and the dataflow framework is Phase 4, because a backend needs only the
Safety IR while the analyses need the framework as well. Data still flows the
way `architecture.md` draws it.

Each phase closes when its **Done when** line is true. That line exists so that
"is the phase finished" is a question with an answer rather than a feeling, and
so that the milestone can be closed.

### Phase 0: Project Skeleton

Complete.

- Cargo project
- CLI
- diagnostics
- minimal documentation
- architecture notes

### Phase 1: Mini C

The hand-written frontend, from source text to a typed AST. See
[frontend.md](frontend.md), whose Stage 1 is the subset this phase accepts.

- lexer
- a minimal preprocessor: `#include`, object-like `#define`, `#ifdef` and
  `#ifndef`. Function-like macros, `#` and `##`, and evaluating a full `#if`
  expression are Phase 9. What `#include` buys early is that every later phase
  can be tested against a real header instead of a hand-written prototype
- recursive-descent parser
- AST
- basic types, functions, control flow
- name resolution and scopes, including C's separate namespace for tags
- declarations against definitions, and forward declarations
- type checking, and the typed AST the Safety IR is lowered from
- per-input diagnostic gating: the first phase that runs per input has to be
  gated on that input, so that a broken `a.c` does not stop `b.c` from being
  looked at
- a corpus of C programs with the diagnostics they are expected to produce.
  [repository.md](repository.md) draws `tests/` and `examples/`; this is the
  asset every later phase is tested against, so it is cheapest to start it here

A macro expansion is the first thing that needs `Span`'s third coordinate. That
coordinate has to serve three consumers and not one: a macro expansion, a
template instantiation, and an operation no source text produced. Deciding its
shape against macros alone means redoing it in Phase 2. See
[c-family.md](c-family.md).

**Done when:** the MVP program above parses and type-checks, `--emit ast` prints
it, and a file that fails to parse does not stop the next file from being read.

### Phase 2: Safety IR

[architecture.md](architecture.md) calls this the central architectural
boundary. It is where [safety-model.md](safety-model.md)'s Value and Place live,
and it is the layer the Clang adapter has to be able to reach without the C
frontend existing.

- the IR: values, places, operations
- basic blocks and control-flow edges. The IR is block-structured from the
  start, so that the backend and the analyses read one control-flow structure
  rather than each building its own
- room in the edge representation for an edge no statement produced. C does not
  force the issue, which is the danger: an exception is that shape, `longjmp` is
  that shape, and every analysis written after this phase walks these edges. The
  cheap version now is to leave room rather than to build unwinding, and to say
  so where the edge type is defined. See [c-family.md](c-family.md)
- an operation that can say it was not written. A destructor at the end of a
  scope has a location to blame and no source text, so attribution has to
  distinguish "written here" from "generated, caused by this"
- function identity as an opaque id rather than a name. Two `static` functions
  in different translation units already share a name in C
- room to hold more than one analysis of a single source range, so that a
  template instantiation can be the unit of analysis later
- lowering from the typed AST
- `--emit safety-ir`, in a textual form. A user redirects, greps and diffs it,
  which makes it an interface rather than debug output
- an interpreter for the IR
- extracting the IR into its own crate, so that cargo rather than a review
  comment enforces that an analysis cannot depend on the frontend. See
  [repository.md](repository.md)

**Done when:** the MVP program lowers to Safety IR, the interpreter runs it and
produces 3, and a test can build IR by hand and run something over it with no
frontend present.

### Phase 3: Code Generation

The backend, reading the Safety IR. [architecture.md](architecture.md) asks that
LLVM stay a backend rather than leak into the frontend and the analyses, which
is a claim worth a test rather than a paragraph.

- the LLVM backend, through `inkwell`, and the LLVM version that pins
- Safety IR to LLVM IR: `--emit llvm-ir`
- object files: `--emit object`
- linking, and a native executable: `--emit executable`
- targets and triples
- `-o`. The driver parses it today and reports it as unsupported
  (`driver.rs:169`), because writing the artifact somewhere the user did not ask
  for is worse than saying no

**Done when:** the MVP program compiles to a native executable that exits with
3, and `-o` puts it where it was asked to.

### Phase 4: CFG and Dataflow

Phase 2 built the graph. This phase builds what walks it.

- the dataflow framework: lattices, transfer functions, fixpoint iteration
- variable state tracking
- whatever the analyses need from the graph itself: reachability, dominance,
  loop structure
- the shape a check reports through, so that a check names its conclusion and
  never reads the policy. See [safety-model.md](safety-model.md)

**Done when:** a trivial analysis reaches a fixpoint over a program with a loop,
and its result is testable against IR built by hand.

### Phase 5: Memory Safety

Safety level 1. The first phase that reports something about a C program.

- allocation tracking
- free tracking
- use-after-free detection
- double-free detection
- nullability
- the first annotation, and only as much annotation machinery as nullability
  needs. The rest waits for the phase that needs it
- whether an annotation is trusted or checked. A trusted annotation that is
  wrong turns `Unknown` into `Safe` silently, which is the failure
  [safety-model.md](safety-model.md) exists to prevent
- wiring `--safety memory` so that the level selects these checks

**Done when:** the use-after-free example above is an error, level 0 reports
nothing, and a case the analysis cannot prove is a warning that `--deny-unknown`
turns into an error.

### Phase 6: Lifetime Safety

Safety level 2.

- stack regions
- heap regions
- parameter lifetimes
- return-value lifetime
- escape analysis
- dangling-reference detection
- lifetime annotations, as far as this phase needs them
- wiring `--safety lifetime`

**Done when:** the `return &x` example above is an error, and a pointer that
escapes through a parameter is one too.

### Phase 7: Ownership

Safety level 3.

- owned values
- borrowed values
- move semantics
- ownership transfer
- borrowing rules
- the `owner` and `borrow` annotations, and what happens when one is wrong
- wiring `--safety ownership`

Use-after-move and double-free are the same defect seen from two ends. A check
that finds one and not the other has not modelled the transfer, which makes this
phase the one that says whether Phase 5's free tracking was really an ownership
analysis wearing another name.

**Done when:** a value used after being moved is an error, and the same analysis
is what reports a double free.

### Phase 8: Thread Safety

Safety levels 4 and 5.

- thread creation and join
- shared state
- synchronization primitives
- data-race detection
- thread ownership and capability experiments
- wiring `--safety thread`, and `--safety strict`, which is defined as leaving
  nothing `Unknown` and therefore implies `--deny-unknown`

**Done when:** a value shared between threads without synchronization is
reported, and `--safety strict` on a program the analysis cannot fully prove
fails rather than passing quietly.

### Phase 9: Broader C

Everything the subset left out. See [frontend.md](frontend.md), Stages 2 and 4.

- the rest of the preprocessor: function-like macros, `#` and `##`, `#if` with a
  full constant expression, conditional compilation
- `typedef`, `enum`, `union`
- function pointers
- variadic functions
- broader compatibility, potentially C11 or C17

**Done when:** a real single-file C program from outside this repository
compiles and runs.

### Phase 10: Clang Integration

```text
Clang AST / CFG
      ↓
Clang Adapter
      ↓
Safety IR
      ↓
Shared Safety Analysis
```

Use real-world C/C++ projects as validation targets.

This is where C++ arrives, and it is the permanent answer for C++ rather than a
step toward a C++ frontend of this project's own. What that costs the design,
and which parts of it are due before this phase, are in
[c-family.md](c-family.md).

**Done when:** an analysis written for C reports a real defect in a C++ project
nobody wrote for this compiler, through IR the C frontend never touched.
