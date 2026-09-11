---
status: "accepted"
date: 2026-09-11
decision-makers: itsakeyfut
---

# Write LLVM IR as text, from a crate of its own

## Context and Problem Statement

`docs/architecture.md` draws one arrow from the Safety IR to a backend and says
what the backend must not do: "the project should avoid making LLVM APIs leak
throughout the frontend and safety-analysis code". Under *Potential initial
library* it names `inkwell`.

Phase 3's first piece is `--emit llvm-ir`, so both halves of that are now
decisions about code rather than intentions: what produces the IR, and where it
lives. The second is the same question ADR-0011 answered one crate earlier, and
the first is one that document answered before there was anything to measure.

## Decision Drivers

* **What a byte-exact artifact costs.** `--emit safety-ir` is held by a corpus
  that compares byte for byte, because a user redirects, greps and diffs an
  artifact. `--emit llvm-ir` is the same kind of thing, and its text has to be
  this repository's to decide.
* **What is installed.** `inkwell` links against LLVM through `llvm-config`.
  Measured on the development host: `clang 20.1.6` is present and `llc`, `opt`,
  `llvm-as` and `llvm-config` are not.
* `docs/repository.md`: **the boundary that must not be crossed is the boundary
  worth making a crate**, because cargo enforces the arrow at build time.
* A guard is worth taking only if a reversal fails to build.

## Considered Options

* Write textual LLVM IR from a crate, `safec-llvm`, that depends only on
  `safec-ir`.
* Use `inkwell` now, from a crate of its own.
* Use `llvm-sys` or the LLVM C API directly.
* A module inside `safec`, with either of the above.
* A trait with one implementation, so that a library can be swapped in later.

## Decision Outcome

Chosen option: **textual LLVM IR, from `crates/safec-llvm`**. The workspace
graph gains one arrow and no others:

```text
safec  ->  safec-llvm  ->  safec-ir
safec  ->  safec-ir
```

`safec-llvm` depends on `safec-ir` and on nothing else, in the workspace or out
of it. It cannot see the lexer, the parser, the AST or `Diagnostic`, and a line
in a manifest is what says so rather than a reviewer noticing. That is the arrow
`docs/architecture.md` states in a sentence, held the way ADR-0011 holds its
own.

Text rather than a library, for three reasons that are about this repository
rather than about LLVM:

**A byte-exact corpus is a property this project can keep.** `inkwell` prints
what its LLVM prints, so an expectation blessed against one version of LLVM is
re-blessed by the next, and the corpus stops being the compiler's own claim.
Six `--emit llvm-ir` cases compare byte for byte today.

**Nothing has to be installed.** No `llvm-config`, no system package on three CI
runners, no version to pin, and the MSRV job keeps checking a workspace that
builds on the floor. `clang` is still wanted, to answer whether what was written
is IR at all, and `tests/llvm.rs` is explicit about being unable to ask.

**There is nothing to abstract over yet.** A trait with one implementation is
invented rather than designed, and `CLAUDE.md` says so. The shape a second
backend needs is a question a second backend gets to answer.

The cost is real and is named rather than argued away: everything LLVM's
builders check, this has to check itself or leave to `clang`'s parser. A
`store i1` into an `i32` is a compile error with `inkwell` and a test failure
here, which is later and further from the mistake.

**The trigger for reversing this is `--emit object`**, which is #92. An object
cannot go to a stream and cannot be compared byte for byte against an
expectation this repository wrote, so the reason above stops applying on exactly
that day. The question then is whether to spawn `clang -x ir` or to link a
library, and this record is what that decision reverses.

### What the backend refuses, and how

`safec-llvm` answers a `Vec<Refusal>` rather than reporting. It cannot see
`Diagnostic`, which ADR-0011 left in `safec` and gave a different trigger for
moving. That is the shape `interp::Trap` already has for the interpreter, and
the two now match on purpose: **what the backend refuses is what the interpreter
refuses**, because two consumers of one IR that disagreed about which programs it
can express would make the IR mean two things. Arithmetic on a pointer counts
elements and `ir::Ty` holds no width to count them with, which ADR-0013 decided,
and both say that in the same words. Where the reason differs the words do too:
an index is refused here because scaling needs a width and there because nothing
builds one yet, and both are true.

A function this cannot write becomes a `declare` and the rest of the module is
written, which is what the frontend's lowering already does with a function it
cannot build. The module stays something LLVM parses, the exit code still says
the run failed, and a link is where the missing symbol surfaces. The exception is
a signature this cannot spell at all: `void` is a result type and nothing else,
so a parameter of it leaves no declaration either, and a call to such a function
is refused in turn.

### Confirmation

Adding `safec = { path = "../safec" }` to `crates/safec-llvm/Cargo.toml` is the
mutation, and cargo refuses the workspace before compiling anything with
`error: cyclic package dependency`. A `use safec::` with no such line does not
resolve either, as `E0432` or `E0433` depending on how far the path gets. The
manifest is the guard because the manifest is where the reversal would be
written, which is ADR-0011's argument and the same one line of evidence.

The text is held by six corpus cases, all six of which `llvm.rs` also hands to
`clang`. That the text is *LLVM* is a different claim: spelling `Add` as `Sub`
changes every corpus expectation and leaves `the_emitted_ir_is_what_llvm_accepts`
passing, which is what says they are two tests rather than one written twice.
The other direction was measured too: storing a comparison's `i1` without
widening it and then re-blessing the corpus leaves all 92 cases green and that
test still failing.

That the backend can be reached with no frontend in the graph is held by the
fifteen hand-built tests in `crates/safec-llvm/src/emit.rs`, of which
`a_unit_built_by_hand_becomes_a_module` is the whole module written out byte for
byte. `cargo tree -p safec-llvm` prints two lines, which is the same claim from
the other side.

### Consequences

* Good, because the arrow `docs/architecture.md` asks for is now held by cargo,
  and the artifact is this repository's to pin.
* Good, because nothing about building or testing this workspace needs an LLVM
  installation, and the one thing that wants a `clang` says so when it has none.
* Bad, because LLVM's own verifier is not in the loop while the text is being
  built: a malformed module is found by a test rather than by a type error.
* Bad, because everything a later backend needs from LLVM, from attributes to
  debug information, is text this has to learn to write. One of those is already
  owed rather than merely future: a `char` parameter or result carries
  `signext` on some ABIs and nothing on others, `clang` writes it, and this does
  not, so a `clang`-compiled caller of a function this wrote passes an
  unextended byte. It is invisible inside a module this wrote whole and matters
  the moment two toolchains meet, which is `--emit object`.
* Neutral, because `#92` reopens it with the evidence that only an object file
  can provide.

## More Information

`docs/architecture.md`'s *LLVM Integration* section is updated in the same
change, per the rule in `docs/adr/README.md` that a design knowingly diverging
from a document in `docs/` records why and updates that document.

ADR-0011 is the record this one follows in shape, and ADR-0013 is why a pointer
has no width to do arithmetic with.
