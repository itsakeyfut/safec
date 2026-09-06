# Clang Integration

The compiler should be designed to support the broader C/C++ safety-analysis project.

There should eventually be two frontends feeding the same safety-analysis infrastructure:

```text
                 ┌──────────────────┐
                 │   Safety Model   │
                 │                  │
                 │ Memory           │
                 │ Lifetime         │
                 │ Thread           │
                 │ Ownership        │
                 └────────┬─────────┘
                          │
             ┌────────────┴────────────┐
             │                         │
             ▼                         ▼
      ┌──────────────┐          ┌──────────────┐
      │   Safe C     │          │    Clang     │
      │   Compiler   │          │   Adapter    │
      └──────┬───────┘          └──────┬───────┘
             │                         │
             └──────────┬──────────────┘
                        ▼
                   Safety IR
                        │
                        ▼
                Safety Analysis
```

## Safe C Compiler

Used as an experimental environment.

It allows safety rules to be designed as part of the language/compiler itself.

## Clang Adapter

Used to apply the same safety model to real-world C/C++ code.

This separation is important:

- Safe C explores the ideal safety model.
- Clang integration tests the model against real C/C++.
- The Safety IR and analysis engine become the shared foundation.

## The Whole Picture

The long-term architecture is:

```text
                         C / C++
                            │
              ┌─────────────┴─────────────┐
              │                           │
           Clang                       Safe C
              │                           │
              ▼                           ▼
       Clang Adapter                  Frontend
              │                           │
              └─────────────┬─────────────┘
                            ▼
                       Safety IR
                            │
             ┌──────────────┼──────────────┐
             ▼              ▼              ▼
          Memory         Lifetime        Thread
          Analysis       Analysis        Analysis
             │              │              │
             └──────────────┼──────────────┘
                            ▼
                     Backend / Runtime
                       │           │
                       ▼           ▼
                     LLVM      Interpreter
```
