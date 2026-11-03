---
status: "accepted"
date: 2026-11-03
decision-makers: itsakeyfut
---

# Each belief's routes are generated as a matrix and asked against a control

## Context and Problem Statement

The memory check keeps beliefs about what memory holds: how wide a pointer
stored there points (ADR-0060, ADR-0061), whether it may be indeterminate
(ADR-0057), and whether a pointer there was lost or is stale (ADR-0045). Each
belief has to survive every way a pointer reaches memory: a store, a library
copy, a `realloc` of what holds it, a call this check cannot read, a return; at
a local or one or two levels below; on every path or one only.

No issue in the repository was found by a failing corpus case. The corpus
records fixes, and review finds the defects, and the longest chains of fixes
were the same program one route over: #719, #721 and #724 were three issues
for one such matrix, and #724, #735 and #767 found routes main was already
silent on. A route nobody wrote a case for is believed by default, and the
first reader to try it is a reviewer.

## Decision Drivers

* **A route that disagrees is a named test failing**, not a review finding.
* **The expected answer does not come from the checker.** A matrix blessed from
  the checker's own output records whatever it does, defects included.
* **ADR-0007 holds**: the set of tests is a literal table, each has a name, and
  an empty table fails.

## Considered Options

* **A literal table of rows, each a belief, a route, a depth and a path shape,
  each row a named test that generates a hazard program and a control program
  and asks the belief's rule of both; and the whole matrix's conclusions
  blessed as one summary.**
* The same table asked only against the plain store's answer for the same row.
* A blessed summary alone.
* Generated programs written into the corpus as ordinary cases.

## Decision Outcome

Chosen option: **a table of rows, each asked a rule and a control, and a
blessed summary beside it**, in `tests/routes.rs`.

**The rule comes from the belief, not from the checker.** Each belief names one
hazard and one control: a pointer to a `char` read back as an `int *` and
written through, against one to an `int`; memory nothing wrote read back,
against memory a store wrote first; a lost or stale pointer freed and used,
against the same program with nothing freed. The hazard has to carry the
belief's code and the control must not, and the two differ in one thing, so
that the control asks what the belief says is safe rather than a program
further from the hazard.
**A control is not optional**: a rule asked only of the hazard passes a checker
that reports every program.

**A row that disagrees is listed, and the list is exact.** A row whose answer
is not the rule's is either a defect or a belief another record took on
purpose, a `void *` a call returns believing no width (ADR-0060) among them.
Each such row is named in one table with the issue or the record that says
why, and a listed row that now agrees fails too, so no entry outlives what it
excused. **What a listed row agreeing means is said in its entry**: a cost, a
false report on a control that is defined C or a silence an open issue answers
for, where agreeing is a fix that removes the entry; or a belief, a report a
record takes because the control is not provable from the unit, a pointer
handed to a call this check cannot read or read where the check stopped
following it, where agreeing is the check saying safe about what it did not
prove, and fails as a regression.

**The summary is what main answered.** One line per row, the hazard's and the
control's exit and codes, is blessed as the corpus is. A change that silences a
route and its plain store together keeps every row agreeing with itself and
moves the summary, which is the diff a reviewer reads: the rule finds a defect
in one build, and the summary finds a regression between two.

**The table is checked against its axes**, so that a row left out is a failure
rather than a gap: the rows are the product of the beliefs, the routes, the
depths and the shapes, less a list of combinations that cannot be written, each
with its reason. **And every generated program is valid C to this frontend**:
each builds at `--safety off` with nothing said, so a template that does not
type-check fails as itself rather than as a disagreement.

### Confirmation

Each mutation applied on its own to `tests/routes.rs`, the checker or the
summary, the matrix run, and the file restored.

* The checker's `memcpy` pouring no width: `width_memcpy_d1_both` fails, among
  others. Its copy carrying no unwritten mark into a block:
  `fresh_memcpy_d1_both` fails.
* A row deleted from the table: `the_matrix_is_every_combination_of_its_axes`.
* `ACCEPTED` listing a row that agrees: that row's test.
* The control asked the hazard's program: most rows fail.
* A summary line changed: `the_matrix_answers_what_was_blessed`.
* A template assigning a `char *` to an `int *`:
  `every_generated_program_is_c_this_frontend_reads`.
* Run against `0586f663`, before #735: `width_call_d1_both` and the three
  rows beside it fail on the hazard, the silences #735 closed.

### Consequences

* Good, because the matrix is the regression oracle a larger change to the
  memory check is measured against.
* Bad, because a row's programs are written by templates, and a template can be
  wrong about C; the `--safety off` build catches only what does not type-check.
  Two were wrong about their belief while the matrix was written, and the
  control is what showed it.
* Bad, because **a row listed on its control asks nothing of its hazard**: the
  control already carries the belief's code, so the hazard's cannot be told
  from it, and such a row holds only that the route is not silent altogether.
  The summary notices more, and only where the codes move.
* Bad, because the matrix's 79 rows run the compiler some 470 times, a process
  each, about four seconds here.

## Pros and Cons of the Options

### Asked only against the plain store's answer

* Good, because it needs no rule per belief.
* Bad, because a change that silences the plain store and the route together
  passes.

### A blessed summary alone

* Good, because it is the least code.
* Bad, because it records today's defects as expected, and finds a disagreement
  only where someone reads the diff.

### Generated programs written into the corpus

* Good, because each program can be read as a file.
* Bad, because some 158 programs, up to four files each, and their table lines
  have to be kept in step with the generator, by a check of their own.

## More Information

* ADR-0007 for the literal table, which this follows rather than reverses.
* Issue #736 and its design comment; #699, the spike that asked whether a
  verdict can be derived apart from the checker.
