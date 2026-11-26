---
status: "proposed"
date: 2026-11-26
decision-makers: itsakeyfut
---

# A line this cannot prove it reads as `clang` does is refused, in every group

## Context and Problem Statement

A preprocessor that reads a line differently from `clang` compiles a
different program. In a skipped group it does so silently: a line taken
for no directive leaves the group skipped where `clang` takes it, and a
double free in that group exits 0.

Under #1359, eight such holes were opened and then closed one spelling at
a time, all found by a review or a hunt and none by the corpus:

- `#else@`;
- a no-break space after `#`;
- `\N{...}`, `\u{...}` and `\U00110000` after a name;
- U+180E.

Architecture review 13 found the next one, a NUL after `#`. Reading each
spelling as `clang` does needs `clang`'s whole lexer, including its table
of character names. Some spellings `clang` reads one way in a skipped
group and another in a compiled one.

## Decision Drivers

* row 6 is the one failure this compiler must not have, and a skipped
  group is where it hides, since nothing in it is compiled to object to
* a list of spellings that are refused grows by one hole at a time, and
  the next spelling is found by a person
* real headers write text in skipped groups that C passes over (FreeBSD
  `sys/queue.h`'s `#warn`), so refusing every line there is too much

## Considered Options

* Refuse a line unless everything from its start to the end of its name
  is a spelling proved to read as `clang` reads it, in every group.
* Read each spelling as `clang` does, adding spellings as they are found.
* Refuse only the spellings found so far.

## Decision Outcome

Chosen option: the first. A directive line is passed over in a skipped
group, or acted on in a compiled one, only where everything from its
start to the end of its name is one of these:

- ASCII space, tab, vertical tab or form feed;
- `#` or `%:`;
- a name `Line::name_at` reads as `Read`.

Any other line is refused with an error, in a skipped group as well. The
same holds below the line, in the lexer:

- a lone carriage return, which `clang` reads as a line break, is refused
  wherever it is (#1372);
- a backslash with a vertical tab or form feed before its newline is
  spliced, as a space or tab before it already is, since that is `clang`'s
  reading and it is proved (#1371).

Each of these lands on row 4 when it is wrong; the options rejected land
on row 6.

### Confirmation

Nothing holds this yet, because the code that carries it lands with
#1377, #1371 and #1372 under #1375. Each of those adds a corpus case that
fails when its refusal is dropped. #1379 adds a differential corpus that
compares every directive spelling with `clang -E` in both kinds of group.
This record becomes `accepted` when those cases exist, and its
Confirmation then names them.

### Consequences

* Good, because a spelling nobody listed is refused rather than misread,
  so the next hole is row 4 rather than row 6.
* Bad, because a header with odd bytes in a skipped group, or a file with
  old Mac line endings, is refused where `clang` reads it.
* What would reverse this: a lexer that reads identifiers, white space
  and line ends exactly as `clang` does, held to it by the differential
  corpus, would let each refusal become a reading.

## Pros and Cons of the Options

### Refuse what is not proved

* Good, because it fails visibly for every spelling at once.
* Bad, because it refuses some programs `clang` builds.

### Read as `clang` reads, spelling by spelling

* Good, because no program `clang` builds is refused.
* Bad, because each spelling missed is a silent row 6, and `\N{...}` needs
  a table of names this does not carry, while an out-of-range universal
  character name is read by `clang` differently by group.

### Refuse only the spellings found

* Good, because it is what exists today.
* Bad, because it is the list that grew by one hole at a time under #1359.

## More Information

- The parent issue is #1375. Architecture review 13 is at
  https://claude.ai/artifact/DzFScQuSmfggX6PKkMEekj.
- `Line` lives in `crates/safec-c/src/preprocessor/line.rs`.
- `docs/frontend.md` promises the spaced splice that #1371 extends.
