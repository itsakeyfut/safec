---
status: "accepted"
date: 2026-11-21
decision-makers: itsakeyfut
---

# A cast between pointers is followed, and one between a pointer and an integer is unproven

## Context and Problem Statement

#615 reads casts. The lowering already passes a `void *` converted to another
pointer type on as the value it is, `Rvalue::Use`, because C converts the two
implicitly (C17 6.3.2.3 p1) and `int *p = malloc(4);` is that conversion.

The design first planned to stop following every other cast that gives a
pointer, on the ground that a pointer whose element type changes defeats the
memory check's width reasoning (ADR-0094, ADR-0112). That premise did not
hold when #1113 was implemented: C already changes a pointer's element type
without a cast, through `void *`, and the check reads that in bytes. Probed
with `void *` in the middle, `c[7]` of an `int *` of 8 bytes read as `char *`
is believed and `c[8]` is `SC0412`, and a free through one type followed by
a use through the other is `SC0402`. So `(char *)p` is the same program as
`(char *)(void *)p`.

A cast between a pointer and an integer is different. The integer carries no
allocation, so followed as a copy, `g((long)p)` with a `g` that frees leaves
`p` believed live after it, and made a pointer the check does not follow,
`free((int *)(long)p); *p = 1;` is believed too. Answering either needs rules
about what escapes through an integer, which this check does not have.

## Decision Drivers

* `(int *)malloc(n)` and `(char *)p` are written everywhere.
* Phase 11's rule: a construct the check cannot read is `Unknown`, never safe.

## Considered Options

* Follow every cast between pointers as `Rvalue::Use`, and report every
  conversion between a pointer and an integer as unproven where it is
  written.
* Follow only a cast to or from `void *`, and make every other pointer cast a
  value the check does not follow.
* Follow casts between a pointer and an integer too, treating the pointer as
  escaping into the integer.

## Decision Outcome

Chosen option: the first, by the owner's decisions while #1113 was
implemented, because it reads every pointer cast through a path the check
already reads, and leaves the one conversion it cannot answer as a doubt at
the place it is written rather than as a silence after it.

* A cast between two pointer types lowers as the implicit conversion does,
  `Rvalue::Use`, and a cast between two integer types as an assignment
  converts one, `Rvalue::Use` into a temporary of the type cast to.
* A null pointer constant cast to a pointer type is the null it is.
* A cast between a pointer and an integer otherwise lowers to
  `Rvalue::Reinterpret`, and the memory check reports each one `Unknown`,
  `SC0416`, at the cast. What it gives names nothing the check follows and
  may be null.

The rejected options fail in the wrong place. Making `(char *)p` a value
the check does not follow doubts every program that reads memory in bytes,
which the check already proves. Following an integer back into a pointer
without an escape rule is row 6: the free above is believed.

### Confirmation

`a_cast_of_malloc_is_followed_as_its_implicit_conversion`,
`a_use_after_free_through_a_char_cast_is_reported` and
`a_char_cast_reads_the_allocation_in_bytes` change when a pointer cast lowers
as `Rvalue::Reinterpret`; `a_null_pointer_cast_is_null` does when `(int *)0`
does; and `a_cast_between_a_pointer_and_an_integer_is_unproven` goes silent
when `memory.rs` pushes no finding for a `Reinterpret`. A new reader of an
`Rvalue` is `E0004`.

### Consequences

* Good, because the common casts keep their allocation and their widths.
* Bad, because a program that keeps a pointer in an integer is doubted at
  every such cast, and nothing after it is proved.
* What would reverse the second half: an escape rule for a pointer converted
  to an integer, after which a round trip could be followed.

## More Information

ADR-0094, ADR-0112, #615, #1113.
