---
status: "accepted"
date: 2026-11-25
decision-makers: itsakeyfut
---

# A function-like expansion is one instance of its substituted text

## Context and Problem Statement

ADR-0054 makes each expansion a file of its own, an instance, whose text
is its replacement list byte for byte, and refuses a node that holds some
of an instance's tokens and not others. Its wholeness test rests on one
fact: an expansion's tokens are contiguous in the token stream and lie in
one file. A function-like macro (#1215) breaks that fact if an argument's
tokens keep the place they were written at, since the expansion's tokens
then come from the body's instance and from the use's file, interleaved.
Something has to decide which file an argument's tokens are in.

## Decision Drivers

* ADR-0054's cut test is what keeps a node that cuts an expansion from
  being checked as if it were whole; that failure is a report lost, which
  is the worst thing this compiler can do
* `#` and `##` (#1216) make text no file holds, which has to be in some
  file to be lexed and pointed at
* a diagnostic has to point the reader at text that exists

## Considered Options

* The body with each parameter replaced by its argument's text, as one
  instance.
* The body's tokens in the body's instance, each argument's tokens where
  they were written, and a new wholeness test over several files.

## Decision Outcome

Chosen option: **one instance of the substituted text.** A function-like
expansion is a file whose text is the replacement list with each parameter
replaced by its argument, macro-replaced first where C17 6.10.3.1 asks for
that, and with `#` and `##` applied. Its `at` is the use, from the macro's
name to its closing `)`. Everything ADR-0054 decided about an instance then
holds unchanged: the expansion's tokens are contiguous and in one file, a
node holding the whole call is lifted to the use, and one that cuts it is
refused.

**An instance says where each run of its text was written.** ADR-0054's
`FileName::Expansion` holds `spelled`, the replacement list, which the
instance copied byte for byte. That stops being true, so `spelled` becomes
a list of pieces, each an offset in the instance and the span its text was
copied from: a run of the body, an argument where it was written (which
may be in another instance, and is followed until it reaches a file that
is not one), or, for the text `#` and `##` make, the operator in the
`#define`. `SourceMap::written` maps through the piece an offset is in. An
object-like expansion is one piece, so it reads as it did.

**A name not replaced stays not replaced.** C17 6.10.3.4 p2 says a name
that was not replaced because it named a macro being replaced is never
replaced again. An argument macro-replaced before substitution can carry
such a name, and re-lexing the substituted text would forget it: with
`#define x x+1` and `#define id(y) y`, `id(x)` is `x+1`, and forgetting
would make it `x+1+1`. So the preprocessor keeps the offsets of those names
in each instance it writes them into, and does not expand a name at one of
them.

**A call whose name came from an expansion and whose arguments did not**,
`#define G F` used as `G(1)`, has no single span for its use in ADR-0054's
chain. It is refused as `SC0104` until #1336 decides how its use is lifted.

Keeping an argument where it was written was rejected because it needs
ADR-0054's wholeness test rebuilt over several files, and a test that
answers "whole" for a cut is the silent failure that record measured.

This replaces ADR-0054's rule that an expansion's text is its replacement
list byte for byte, and nothing else of that record.

### Confirmation

Owed by #1215 and #1216, which name the cases:

* re-expanding a name not replaced: `id(x)` with `#define x x+1` runs to
  the value `x+1` gives;
* a call cutting a function-like expansion is refused, and one holding it
  whole is checked as written out;
* `SourceMap::written` mapping an argument's offset to the body's: a
  diagnostic in an argument points at the argument;
* `G(1)` above is refused.

### Consequences

* Good, because ADR-0054's cut test and every comparison it built are
  reused unchanged.
* Bad, because a diagnostic in an argument shows the substituted text,
  with the use as the note, rather than the argument where it was typed.
* What would reverse this: a diagnostic in an argument that a reader
  cannot place.

## More Information

#618, #1215, #1216, #1217, #1336; ADR-0054.
