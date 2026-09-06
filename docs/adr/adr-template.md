---
status: "proposed | accepted | rejected | deprecated | superseded by ADR-NNNN"
date: YYYY-MM-DD
decision-makers: who decided
---

# Short title, in the form "do X because Y" or "X is Z"

## Context and Problem Statement

What is the problem, and why does it need deciding now? Two or three sentences.
If something already depends on the answer while this record is still `proposed`,
say so here.

## Decision Drivers

* the constraints that actually narrow the choice

## Considered Options

* option 1
* option 2

## Decision Outcome

Chosen option: "option 1", because ...

### Confirmation

Which test or guard fails if this decision is violated, and **what change to the
code makes it fail**? Name the mutation, not just the test: a test that passes
whatever the code does confirms nothing. If nothing would fail, say so plainly.
A decision that looks enforced and is not is worse than one that is honestly
unenforced.

### Consequences

* Good, because ...
* Bad, because ...
* What would reverse this: ...

## Pros and Cons of the Options

### option 1

* Good, because ...
* Bad, because ...

### option 2

* Good, because ...
* Bad, because ...

## More Information

Links to the code, the document in `docs/` this serves, and the review or
measurement it rests on.
