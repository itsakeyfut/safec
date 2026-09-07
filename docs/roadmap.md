# Roadmap

## Initial MVP

The first milestone should be intentionally small:

```text
C subset
    ↓
Lexer
    ↓
Parser
    ↓
AST
    ↓
CFG
    ↓
Lifetime / use-after-free analysis
    ↓
LLVM IR
    ↓
Native executable
```

A minimal first working program:

```c
int add(int a, int b) {
    return a + b;
}

int main() {
    return add(1, 2);
}
```

Then introduce memory safety:

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
owner → move → invalid
```

Finally begin thread-safety analysis.

## Phases

### Phase 0: Project Skeleton

- Cargo project
- CLI
- diagnostics
- minimal documentation
- architecture notes

### Phase 1: Mini C

- lexer
- parser
- AST
- basic types
- functions
- control flow
- basic LLVM backend
- per-input diagnostic gating: the first phase that runs per input has to be
  gated on that input, so that a broken `a.c` does not stop `b.c` from being
  looked at

### Phase 2: CFG and Dataflow

- CFG construction
- basic blocks
- dataflow framework
- variable state tracking
- room in the edge representation for an edge no statement produced. C does not
  force the issue, which is the danger: an exception is that shape, `longjmp` is
  that shape, and every analysis written after this phase walks these edges. The
  cheap version now is to leave room rather than to build unwinding, and to say
  so where the edge type is defined. See [c-family.md](c-family.md)

### Phase 3: Memory Safety

- allocation tracking
- free tracking
- use-after-free detection
- double-free detection
- nullability experiments

### Phase 4: Lifetime Safety

- stack regions
- heap regions
- parameter lifetimes
- return-value lifetime
- escape analysis
- dangling-reference detection

### Phase 5: Ownership

- owned values
- borrowed values
- move semantics
- ownership transfer
- borrowing rules
- experimental annotations

### Phase 6: Thread Safety

- thread creation/join
- shared state
- synchronization primitives
- data-race detection
- thread ownership/capability experiments

### Phase 7: Clang Integration

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
