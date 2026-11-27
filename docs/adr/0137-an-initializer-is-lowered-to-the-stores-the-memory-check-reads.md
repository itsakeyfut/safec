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
* The memory check does not follow what a static holds (ADR-0115), so a static's starting value is the backend's question, not the check's, with one exception: a store of a literal's or a `const` object's address into a pointer that may write it is asked where it is made (ADR-0121), and an initializer is that store made before the program begins.

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

* where the object's type holds no pointer: one `memset` of the whole object to zero, then a store per written leaf, in the list's order. Always, not only where a leaf is unwritten: padding and a union's bytes past its first member are no leaf's, and `clang` zeroes them, so a program reading them back as zero kept that value (#1241 review);
* where it holds a pointer: a store per written leaf, in the list's order, then zero or a null pointer into every leaf none wrote, after them so an item reading the object reads what C gives it. Past a bound on the leaves, the initializer is refused, visibly, and so is one holding a union whose first member is not its largest, whose other bytes no leaf reaches.
* an array of character type from a string literal holds no pointer and its leaves are the literal's bytes, so it is one copy of them out of the static the literal is, and one fill of zeros past them where the array is longer: two calls however long the literal, where a store per byte made a 2000-character literal take the memory check a minute.

Each copy and fill an initializer makes is to this compiler's own `memcpy` and `memset`, never the program's, which may be a `static` of another meaning (C17 7.1.3 p1), and is marked as initializing the object, which a `const` object may be given (6.7.3 p6), so the read-only check does not ask it of the local it initializes.

For a static, `StaticInit` gains an aggregate: an entry per member or element that does not start as zero, each by its index and itself a `StaticInit`, sparse so that a designator's large index costs one entry (#1242), with a leaf for the address of a static plus a byte offset. A string literal used as a pointer's initial value is that leaf. A `char` array keeps `StaticInit::Bytes`. An address leaf given to a pointer that does not point at `const` carries the initializer's span, and the memory check asks it of a read-only object as it asks a store (`memory::read_only_statics`).

The length an initializer gives an array of unknown size (6.7.9 p22) is counted by the type check, from the same resolution, once the items are typed: a struct item taken whole and brace elision decide it, and with designators a constant's value does.

### Confirmation

* Corpus cases of each form, built and run at `--safety off` and checked at `--safety memory`, land with #1241 to #1245. #1244's: `a_char_array_takes_its_length_and_bytes_from_a_string_literal` and `a_char_array_longer_than_its_string_literal_is_zero_filled` run; `a_const_char_array_from_a_string_literal_is_not_a_write` holds the initializing mark; `an_initializer_calls_no_memset_the_program_has` holds the compiler's own copy and fill. #1241's: `a_braced_initializer_fills_arrays_and_structs` and `a_static_holds_addresses_of_statics` run; `a_const_local_is_initialized_from_a_braced_list` holds the initializing mark for stores.
* A local with a pointer, initialized from a list that does not name every scalar, gets no `SC0413`: `a_local_holding_pointers_is_initialized_one_scalar_at_a_time`. The mutation that zeroes it with a `memset` brings `SC0413` back.
* A static table of string pointers builds, and its program reads what C17 gives: `a_static_holds_addresses_of_statics`.
* Past the bound, a local holding a pointer is refused: `a_large_local_holding_a_pointer_is_not_initialized_yet`.
* A static pointer that may write a literal or a `const` object is `SC0417`: `a_static_pointer_to_read_only_storage_is_asked_as_a_store`.
* A static of a million elements with two designated is two entries, and its program reads what C17 gives: `a_designated_initializer_names_its_subobject`.
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
