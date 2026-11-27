---
status: "proposed"
date: 2026-11-26
decision-makers: itsakeyfut
---

# A byte `clang` reads specially is refused wherever it stands

## Context and Problem Statement

Three bytes mean something to `clang` 22.1.8 that they do not mean to this
lexer, and each has let this compiler read a different program:

- a carriage return no line feed follows ends a line to `clang` (#1372);
- a NUL is white space to `clang` in code and in a comment, and it is skipped
  between a `\` and the newline that closes a block comment, though not in a
  line comment (#1377, #1389);
- a Ctrl-Z, 0x1A, ends the file to `clang` targeting MSVC, in code only, and
  is an ordinary character in a comment, in a string, and for other targets
  (#1383).

Each place one of these stands needed its own fix, and a place nobody
listed was a silent double free (#1377 and #1389 for NUL). #1372 refused the
first, and this record says why the other two take the same answer.

## Decision Drivers

* A place this misses must land on a refusal, not on a different program:
  the failure has to be visible.
* None of the three is in a real header: 0 of 5088 files in the Windows SDK
  10.0.28000.0 and `clang`'s own `include` directory hold a NUL or a Ctrl-Z,
  and 0 of 3706 hold a lone carriage return.
* The lexer does not know the target, and Ctrl-Z's meaning depends on it.

## Considered Options

* Refuse each byte wherever it stands in a file, at the lexer.
* Read each byte as `clang` does, place by place and, for Ctrl-Z, target by
  target.
* Refuse each byte only in the places `clang` treats it specially.

## Decision Outcome

Chosen option: "Refuse each byte wherever it stands in a file, at the
lexer". This is the only option where a place nobody listed is still
refused. The lexer reports the first of each byte in a file, as `SC0103`,
each time the file is read, with a note on what `clang` reads it as. The
code reading each byte where it stands, such as the directive start
ADR-0134 proves, stays as it is: it is no longer reached by a file that
gets through.

### Confirmation

One corpus case per byte, each refused, fails when that byte's check is
skipped in the lexer:

* `a_carriage_return_alone_in_a_line_comment_is_refused` and two beside it
  (#1372);
* `a_nul_in_the_splice_that_closes_a_block_comment_is_refused`,
  `a_nul_in_a_line_comment_is_refused` and `a_nul_in_a_string_is_refused`
  (#1389);
* `a_ctrl_z_after_pragma_once_is_refused` and
  `a_ctrl_z_in_a_block_comment_is_refused` (#1383).

In each, the mutation makes the case exit 0 with nothing said. In
`tests/directives.rs`, the NUL and Ctrl-Z rows of the differential corpus
become refusals, so `KNOWN` holds none of them.

### Consequences

* Good, because each byte is one check in one function, and a new place it
  can stand needs no fix.
* Good, because the lexer needs no target.
* Bad, because a program `clang` accepts is refused when it holds one of
  these bytes anywhere, a comment or a string included. A NUL in a string,
  which `clang` keeps with a warning, is one.
* Bad, because a lone carriage return as a file's last byte, which moves
  no line, is refused too.
* What would reverse this: real code holding one of these bytes, which
  would make the refusal cost more than reading it as `clang` does. The
  reading would then go to the one place that byte stands in that code,
  with a case proving `clang` reads it that way.

## Pros and Cons of the Options

### Refuse each byte wherever it stands, at the lexer

* Good, because a missed place cannot exist: the check reads every text
  before it becomes tokens, a file and an expansion alike, so a NUL an
  escape in a `#line` name decodes is asked too.
* Bad, because it refuses programs `clang` builds, all visibly.

### Read each byte as `clang` does, place by place

* Good, because it refuses nothing `clang` builds.
* Bad, because each place is a rule of its own, NUL in a block comment's
  closing splice and not a line comment's for one, and Ctrl-Z needs the
  target in the lexer. A place missed reads a different program, and for
  Ctrl-Z, stopping early is a program `clang` builds that this never reads:
  safe said of code nobody checked.

### Refuse each byte only where `clang` treats it specially

* Good, because a byte in a comment or a string that means nothing special
  there is read.
* Bad, because it needs the same list of places as the second option, and a
  place missed lands on the second option's failure.

## More Information

`Unread` and `unread` in `crates/safec-c/src/lexer.rs` hold the bytes,
each variant's doc comment the place its byte was found. #1372 added the
carriage return, #1389 the NUL, and #1383 the Ctrl-Z, under #1375. ADR-0134 is the same choice made for the start of a directive
line.
