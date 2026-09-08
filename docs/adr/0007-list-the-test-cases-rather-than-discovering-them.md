---
status: "accepted"
date: 2026-09-08
decision-makers: itsakeyfut
---

# List the test cases in a literal table rather than discovering them on disk

## Context and Problem Statement

`--emit` output is an interface: a caller redirects it, greps it and diffs it.
Pinning it needs a corpus of C programs paired with the output the compiler is
expected to produce for them, and adding to that corpus has to be cheap or it
will not grow. The obvious harness walks a directory and runs whatever it finds.

The choice has to be made before the corpus exists, because every later phase
adds cases to it and changing the format afterwards means touching every case.

## Decision Drivers

* `CLAUDE.md` defines a guard as a change to the code that makes a **named**
  test fail. A harness that loops inside one test function gives every case one
  name.
* This repository has already shipped a test that compared a table with itself
  and proved nothing: `Keyword::from_spelling(kw.as_str()) == kw` held for any
  spelling whatsoever, so `return` spelled `retrun` passed the whole suite.
* `docs/repository.md` asks for a deliberately small dependency list. The crate
  depends on `clap` and `ariadne`.

## Considered Options

* A literal table of cases, expanded by a macro into one test each
* Walking `tests/cases/` and running whatever `.c` files are there
* A snapshot library, such as `insta`

## Decision Outcome

Chosen option: **a literal table**, because it is the only one of the three that
makes an empty corpus a failure rather than a pass, and because it is the only
one that gives each case a test name of its own.

A directory walk is the same shape as the keyword-table test that was removed
from this repository: it restates the directory instead of defining what should
be there. It passes when `tests/cases/` is empty, and it passes when somebody
deletes half the corpus. The table is the definition, and a second test walks
the directory in the other direction so that a file nobody listed is reported
rather than silently unused.

A case's arguments live on its line in the same table, so that what a case is
and how it is run are read in one place.

### Confirmation

`every_file_in_the_corpus_belongs_to_a_case_in_the_table` in
`crates/safec/tests/cases.rs`.

The mutation: delete the `add:` line from the `cases!` table while leaving
`crates/safec/tests/cases/add.c` in place. That test fails, naming `add`, and no
other test does. It is the reversal this record exists to make expensive:
whatever replaces the table has to keep answering the question of which files
are covered.

The other half is confirmed by the shape of the expansion rather than by an
assertion. Changing the separator in `driver.rs::dump_tokens` fails a test
called `add`, rather than one test standing for the whole corpus, and that is
only true while the macro generates one function per entry.

### Consequences

* Good, because an empty or halved corpus is a failure, and a broken case names
  itself.
* Good, because adding a case is two files and one line, with no Rust written.
* Bad, because the line is a second thing to remember when adding a case. The
  directory guard is what makes forgetting it loud rather than silent.
* Bad, because a case's name is also its source file name, so one program
  cannot be pinned under two artifacts without a second copy of it. The roadmap
  points Phase 1, Phase 2 and Phase 3 at the same MVP program, so this will be
  met. It fails as `error[E0428]: the name is defined multiple times` rather
  than quietly, which is why it is recorded here and left rather than fixed
  ahead of a caller.
* What would reverse this: anything that makes the set of cases implicit. If the
  corpus grows past the point where a person maintains the table by hand, the
  answer is a generated table checked into the repository, not a walk at run
  time, because the property being kept is that the set of cases is written down
  somewhere a diff can show.

## Pros and Cons of the Options

### A literal table

* Good, because one test per case, named after the case.
* Good, because it cannot pass on an empty directory.
* Good, because no dependency.
* Bad, because it is one more line to write per case.

### Walking the directory

* Good, because a case is only its files.
* Bad, because it passes with no cases at all, which is the failure this
  repository has already had once.
* Bad, because one test name covers every case, so a failure says the corpus
  broke and not which part of it.

### A snapshot library

* Good, because regeneration and diff rendering come for free.
* Bad, because it is a dependency taken for test ergonomics, against
  `docs/repository.md`.
* Bad, because it still leaves the discovery question open, and the interesting
  half of this decision is discovery rather than comparison.

## More Information

* [`crates/safec/tests/cases.rs`](../../crates/safec/tests/cases.rs), where the
  table and both guards live.
* [`docs/repository.md`](../repository.md) for the dependency principle.
* The keyword-table test that motivated the drivers is recorded in the review
  knowledge bank as RK-001.
