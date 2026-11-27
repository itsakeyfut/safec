---
status: "proposed"
date: 2026-11-27
decision-makers: itsakeyfut
---

# An initializer is lowered to the stores the memory check reads, and a static's to a constant shaped like its type

## Context and Problem Statement

C17 6.7.9 lets an object be initialized from a braced list, a designated list, a compound literal or a string literal, with every element the list does not name set to zero (p21). Before #1196 an initializer could only be one expression. A static's starting value could only be zero, one integer, or a string literal's bytes (`StaticInit`, ADR-0115). So a local array or struct had no initializer at all, and a static table of string pointers, the shape real code uses for names and messages, could not be written.

Two shapes have to be chosen: what a local's initializer becomes in the IR, which the memory check reads; and how a static's starting value is held, which the backend writes.

## Decision Drivers

* The memory check keeps what a local holds per slot where the local holds pointers (ADR-0092 rule 3), and reads every store it is given.
* Measured on `main` at `12fcf658`: a `memset` that zeroes a local the check keeps per slot, an array of pointers, is reported as writing something that is not a pointer where pointers are read back (`SC0413`), and a later read is a null doubt (`SC0403`). A `memset` of a local with no pointer in it is read as a write.
* An initializer can be far larger than what it names: `char buf[4096] = {0}` writes 4096 bytes from one written zero.
* The memory check does not follow what a static holds (ADR-0115), so a static's starting value is the backend's question, not the check's.

## Considered Options

* For a local:
  * **Leaf stores where the type holds a pointer, `memset` and leaf stores where it does not.**
  * A `memset` of the whole object, then a store per written leaf.
  * A copy from an unnamed constant static, then a store per leaf that is not constant.
  * A store per leaf, always.
* For a static:
  * **A constant shaped like the type, whose leaves are an integer, zero, or the address of a static plus an offset.**
  * A byte image, with any address refused.

## Decision Outcome

Chosen options: "leaf stores where the type holds a pointer, `memset` and leaf stores where it does not", and "a constant shaped like the type". The first is the only option the memory check reads without a false report and without growing with the object's size where it holds no pointer. The second is the only one that writes a table of string pointers.

For a local, the initializer is first resolved, as 6.7.9 p17 to p21 say, into the leaves it writes: each a path of members and elements to a scalar, and a value. Then:

* where the object's type holds no pointer: when some leaf is not written, one `memset` of the whole object to zero, then a store per written leaf, in the list's order;
* where it holds a pointer: a store per leaf of the whole object, zero or a null pointer for the ones not written. Past a bound on the leaves, the initializer is refused, visibly.

For a static, `StaticInit` gains an aggregate: one entry per member or element, each itself a `StaticInit`, with a leaf for the address of a static plus a byte offset. A string literal used as a pointer's initial value is that leaf. A `char` array keeps `StaticInit::Bytes`.

### Confirmation

* Corpus cases of each form, built and run at `--safety off` and checked at `--safety memory`, land with #1241 to #1245.
* A local with a pointer, initialized from `{0}`, gets no report. The mutation that zeroes it with a `memset` brings `SC0413` back.
* A static table of string pointers builds, and its program prints what C17 gives.
* `E0004` at every reader of `StaticInit` holds the new variant: the printer, the interpreter and the backend.

### Consequences

* Good, because the memory check reads an initializer as the stores it is, so what it proves of a written local still holds.
* Good, because a static can hold what real code puts in one, its strings' addresses included.
* Bad, because a local holding pointers is written leaf by leaf, so a large one makes a large function, and past the bound it is refused.
* Bad, because the memory check still doubts what is read out of a static, a table of pointers included, as ADR-0115 does today.
* What would reverse this: the memory check reading a `memset` to zero as a write of null pointers, after which the `memset` route could serve every local.

## Pros and Cons of the Options

### A `memset` of the whole object, then a store per written leaf

* Good, because the IR grows with what the source writes.
* Bad, because a local holding pointers gets a false `SC0413` and `SC0403`, measured.

### A copy from an unnamed constant static

* Good, because it is what `clang` emits for a large constant initializer.
* Bad, because the memory check does not follow a static's contents, so every constant pointer copied in is doubted when it is read.

### A store per leaf, always

* Good, because it is the most precise.
* Bad, because `char buf[4096] = {0}` is 4096 stores.

### A byte image for a static

* Good, because it is one shape for every type.
* Bad, because an address has no bytes until link time, so a table of string pointers is refused.

## More Information

#1196 is the parent. ADR-0115 is the static's table this extends, ADR-0092 the per-slot record the leaf stores feed, and ADR-0124 the layout that a leaf's path is an offset into.
