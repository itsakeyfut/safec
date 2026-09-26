# Diagnostics

Diagnostics are a first-class feature because safety analysis is only useful if developers can understand why something is unsafe.

Example:

```text
error[SC0601]: use of moved value: `p`

  --> main.c:12:5
   |
 8 |     owner int* p = alloc();
   |                 - move occurs here
 9 |
10 |     consume(p);
   |             - value moved here
11 |
12 |     *p = 42;
   |     ^^ value used here after move
   |
   = note: `p` was moved into `consume`
```

`ariadne` renders diagnostics against their source. Its output differs in detail
from the sketch above, which is drawn in the shape rustc uses.

## Codes

A code is `SC` and four digits. `SC` is Safe C.

It is the handle a reader keeps: a message is reworded whenever a better wording
is found, and the code is what a suppression list, a grep or a bug report still
matches afterwards. So a code is assigned once and never changes, which
`each_lexical_diagnostic_keeps_the_code_it_was_assigned` in
`crates/safec/src/lexer.rs` states as a rule and holds for the lexical five.

The prefix names this compiler rather than a severity, because
[ADR-0001](adr/0001-promote-unproven-results-in-the-sink.md) lets the sink
promote a diagnostic from a warning to an error while its code stays put:
`warning[SC0601]` and `error[SC0601]` are the same diagnostic under two
policies. [ADR-0009](adr/0009-name-a-diagnostic-code-after-the-compiler-and-the-topic.md)
records why the prefix is this one and what was rejected.

### What each range holds

| Range | Topic | In use |
|---|---|---|
| `SC00xx` | not a topic: examples and tests, never emitted by the compiler | `SC0001` |
| `SC01xx` | lexical, what a character or a token is | `SC0101`, `SC0102`, `SC0103`, `SC0104`, `SC0105`, `SC0106` |
| `SC02xx` | syntax, what a sequence of tokens is | `SC0201`, `SC0202`, `SC0203`, `SC0204`, `SC0205` |
| `SC03xx` | names and types | `SC0301`, `SC0302`, `SC0303`, `SC0304`, `SC0305`, `SC0306`, `SC0307` |
| `SC04xx` | memory | `SC0401`, `SC0402`, `SC0403`, `SC0404`, `SC0405` |
| `SC05xx` | lifetime | none yet |
| `SC06xx` | ownership | `SC0601` |
| `SC07xx` | thread | none yet |
| `SC08xx` | code generation, what a backend can write | `SC0801` |
| `SC09xx` | free | none |

`SC04xx` through `SC07xx` are the four safeties [`concept.md`](concept.md) asks
the question about, in the order it names them, so they were reserved before
anything could emit from them. The first of the four now does, five times:
`SC0401` is a value freed where it may already have been freed, `SC0402` is
a value used where it may already have been freed, `SC0403` is a value read
or written through a pointer that may be null, `SC0404` is a value freed
through a pointer that may not be the start of its allocation, and `SC0405` is
a pointer that may be null passed to a parameter declared `_Nonnull`. They are
the first codes in this compiler that say something about what a program does
rather than about how it is written.

**`SC0403` is a third class of program and not a third severity.** The other
two are about a pointer that pointed somewhere once and no longer does; this is
about one that may never have pointed anywhere. A reader filtering on a code is
looking for programs to fix, and the fix for a use after free is about where the
`free` went while the fix for this is a test the source does not have.

**`SC0405` asks `SC0403`'s question somewhere else, and is a class of its own
for the reason the two above are.** A parameter declared `_Nonnull` is believed
by the function's body, so the read that would have been `SC0403` inside it is
not reported there; what it rests on is that every call in the translation
unit passes a pointer it established is not null, and `SC0405` is that call
when it does not. The fix is at the call, or at the promise if the promise was wrong,
and never inside the body. It carries a second label at the `_Nonnull` it
broke. [ADR-0037](adr/0037-a-nonnull-parameter-is-believed-by-its-body-and-checked-at-every-call-in-its-translation-unit.md)
is the decision. The two frontend refusals that come with it, `SC0204` for a
`_Nonnull` where it cannot apply and `SC0307` for two declarations that
disagree about one, are in [`frontend.md`](frontend.md#where-the-nonnull-annotation-is-read).

**A hatch moves an unproven conclusion rather than removing it.** Inside a
function definition declared a hatch, an `SC04xx` that could not be proved is
not reported: it is written by `--emit hatches` under that hatch instead,
because it is a statement about the hatch and not about the program. One that
was proved is reported as it would be anywhere. The code does not change in
either place, so a reader searching for `SC0403` finds it in the listing as
well. [ADR-0038](adr/0038-a-hatch-is-a-function-definition-and-what-it-could-not-prove-is-listed-rather-than-reported.md)
is the decision. `SC0204` also refuses the hatch's attribute where it cannot
apply, and `SC0205` is an attribute this compiler does not read, which is every
one but the hatch's; both are in [`frontend.md`](frontend.md#where-the-hatch-is-read).

**`SC0404` is a fourth class, for the same reason.** It is about a pointer that
points somewhere real and is not the thing `free` takes: C17 7.22.3.3 p2 makes
freeing anything but what an allocation function returned undefined, and
`free(p + 1)` reaches the allocation `p` does. The fix is different again: a
double free is a mistake about ownership, a use after free one about lifetime,
and this is a mistake about which value was handed over. One call can carry
both it and `SC0401` at one caret, the second in `free(p); free(p + 1);`,
because the two are different questions about the same call.
[ADR-0036](adr/0036-a-pointer-carries-where-in-its-allocation-it-points.md)
records what is proved and what is left unproven.

**Either can be a proof or a suspicion, and freeing more than one allocation
at once is where the difference is easiest to misread.** A local that may hold
either of two allocations frees exactly one of them, so freeing one of the two
afterwards by name is a warning: this check cannot say which member went.
Freeing the *same* local twice stays an error, because whichever member it
held, it held the same one both times.
[ADR-0020](adr/0020-a-free-of-a-may-set-is-a-fact-about-the-set.md) records
why those are different answers and what it cost to give them the same one.

The two are one check answering about two things it found, rather than two
checks, which is why they arrived one after the other and share every secondary
label they carry. They take separate codes because a code is the handle a reader
searches with and a suppression list keys on, and freeing twice and reading
through a dangling pointer are two classes of program to look for.

**A suspicion says what was established, and where nothing was it says so.**
There are three reasons either code can go out unproven, and two of them have a
free behind them: a site the paths or the sites reaching a caret disagree
about was really freed on one of them, and a site handed to a call this check
cannot read may have been freed by that callee. Those read `this may free a
value that was freed already` and `this may use a value after it was freed`,
and a reader is being told there is something to suspect.

The third is this check having stopped following the pointer, and there the
same words would assert a first free that nothing worked out. A program with
one `free` in it, and a program with none, both used to get them. So those read
`this frees a pointer this check stopped following` and `this uses a pointer
this check stopped following`, which are the only diagnostics here that name
the analysis rather than the program, because the analysis is the whole of what
happened. The code does not change: the reader who greps `SC0401` is looking
for everything this check said about freeing, and where it gave up is part of
that.

**What that third one no longer swallows is a null this check could have
proved.** C17 7.22.3.3 p2 defines `free` of a null pointer as doing nothing, so
these two programs are the same program:

```c
void free(void *p);

int f(void)  { free(0); return 0; }
int g(void)  { int *p = 0; free(p); return 0; }
```

`clang` accepts both, and so does this compiler. The declaration is part of the
program rather than decoration: without it C17 6.5.2.2 has no prototype to
check the call against, and both compilers refuse the file before either of
them has an opinion about the free. It used to be silent about the
first and to say `this frees a pointer this check stopped following` about the
second, an error wherever a check runs, because a constant argument is
recognised where it is written while a local that was given one reaches no
site, and reaching no site is how this check spells having lost a pointer. A
pointer proved null and a pointer never followed arrived at the same place with
nothing to tell them apart.

What tells them apart is the nullability check: a free whose argument that
check established is null is left out of the report, and nothing weaker exempts
one. Both halves of that are
[ADR-0027](adr/0027-an-unsafe-conclusion-is-about-an-execution-this-function-has.md),
which is also where the conditions the exemption rests on are written down,
because an exemption is the half of a rule that can go quiet.

**What still gets those words is a pointer that really was lost**, and that is
what the third reason is for: `free(*pp)` names a place this check follows
locals rather than the targets of, and a local whose address escaped is
answered the same way however plainly the program reads. Neither is a null this
compiler established, and neither is exempt.

**What `SC0402` not being emitted does not mean.** The check follows a pointer
through a copy, through pointer arithmetic and into a controlling expression,
so `q = p; *q`, `p[i]`, `if (*p)` and `*p;` are all read: the last of those
throws its value away, and C17 6.5.3.2 p4 makes the dereference undefined all
the same. What it does not read is a place it follows no allocation for, and
that is the rule rather than a list. Two ways to reach one are known: a
pointer read out of another pointer, `int *p = *pp;`, which never had a site;
and a name with nothing dereferenced, so `free(p); p;` is quiet about reading
an indeterminate pointer, which is undefined by 6.2.4 p2 and belongs to an axis
this compiler has no check for yet. `int *r = &*p;` used to be a third and is
not: the lowering applies C17 6.5.3.2 p3, which makes that pointer the same
pointer, so `r` carries what `p` carried.

Three things that used to make it quiet no longer do, and all three are worth
knowing because the shapes look like they would still be silent. A local whose
address has been taken stays unproven for the rest of the function, whatever is
assigned to it afterwards, so `int **pp = &p; p = malloc(8); *pp = q; *p = 1;`
reports rather than saying nothing. Where two dereferences of one place share a
caret, which both operands of a `||` do, the two are one report and the
**stronger** of them is what it says, so a proof is never spent by a suspicion
beside it. And a local that saved a pointer across a turn of a loop is read
against the allocation it holds rather than the one the loop made next.

That last one is worth a sentence more, because what it used to do was not only
be quiet. A site is named by a local, so a loop that allocates every turn files
each allocation under one name, and whoever still holds the previous one is
holding something this check can no longer name. Before
[ADR-0018](adr/0018-a-site-names-one-allocation-at-a-time.md) a `free` of the
saved pointer produced nothing while the `free` beside it with nothing wrong
with it carried the warning.

**What the first of those costs is the proof.** A read of a local whose address
has been taken is reported as a warning however clear the free beside it looks,
because something holds that address and this check does not follow what every
holder of it does. `int **pp = &p; free(p); *pp = q; *p = 1;` is the case, and
it used to be an `error`: a proof this check did not have, about a program with
no defect in it. The warning it carries now says this check stopped following
the pointer rather than that the read may be after the free, because the write
through `pp` is one this check *did* follow and it can see that `p` holds `q`.
What is missing is a way to say so, which is
[#196](https://github.com/itsakeyfut/safec/issues/196)'s work rather than this
rule's. [ADR-0017](adr/0017-record-each-half-of-an-escape-where-its-subject-lives.md)
records where each half of what an escape means is kept, and why the half about
the allocations those locals hold is kept separately from it.

The first and the third report `SC0402` as a warning rather than an error,
because neither is something this check proved: what it knows is that it
stopped being able to follow the pointer. The second is the one rule of the
three that can leave an `error` standing, because what it decides is which of
two reports at one caret survives rather than what either of them concluded.
An unproven result fails a build wherever a check runs, and
[ADR-0001](adr/0001-promote-unproven-results-in-the-sink.md) is why that is
the policy's decision rather than the check's.

**One shape is still quiet, and it is not the boundary above.** A write through
a pointer is followed where this check knows where it lands, so
`int **pp = &p; *pp = q; free(p); *q = 1;` now reports the last line: the write
gives `p` what `q` holds, the `free` reaches it, and the read is a proved use
after free. What it does not know is where a write lands when it never saw the
address taken, and `*pp = q;` for such a `pp` changes nothing here, so the
allocation that was really freed keeps whatever was known about it and a later
use of it is read against a site this check believes is live.

**And one where it is followed and still not proved.** A write through a
pointer replaces what the target held where the pointer names one local and
this check has seen every address taken into it, which is what makes the
program above an `error` rather than a suspicion. Take the pointer's own
address and that stops being true: `int **pp = &p; opaque(&pp); *pp = q;`
leaves `pp` holding something `opaque` may have replaced, so the write is
followed as a *may* write and the proof is not available.
[ADR-0028](adr/0028-replace-what-a-target-held-where-a-write-must-land-in-it.md)
records what separates the two.

That remainder is a place with an allocation followed, concluded live and
wrong, rather than a place with no allocation. It is deliberate:
[ADR-0019](adr/0019-follow-a-write-through-a-pointer-only-where-it-lands.md)
records what was measured when the rule was widened to cover it, which was a
second silence somewhere else.

This is written here rather than only beside the code because a boundary a user
cannot find is one they will discover by being wrong about it, and
[`safety-model.md`](safety-model.md#safe-unsafe-unknown) is clear that silence
is the most expensive answer this compiler gives.

`SC00xx` is the exception that keeps the rest honest. A renderer test needs a
code that means nothing, and a code that means nothing has to come from
somewhere that no diagnostic will ever claim.

`SC0601` is a reservation rather than a diagnostic. The use-after-move sketched
at the top of this document is not implemented, and the doc comments and tests
that carry that code are illustrations of one.

### Allocating the next one

* A code comes from the range of its **topic**, not of the stage that emits it.
  Which pass finds a problem is an internal fact that can change; what the
  problem is about does not, and a user with a code in a suppression list should
  not pay for a check moving.
* Inside a range, codes are handed out in order of first use from `01`, never
  reused and never renumbered. A retired diagnostic leaves its number retired.
* A new topic takes the lowest free hundred, and **the table above is updated in
  the change that first emits from it**, not afterwards. A topic that runs past
  `99` takes a second hundred from the free end and is recorded the same way.

Half of this is held by the compiler and half is not, which is worth knowing
before relying on either. `Code::new` asserts the spelling, and every code is a
`const`, so a code spelled any other way stops the build at its declaration.
Which range a code belongs in is a question only a person can answer, since it
turns on what the new diagnostic is about, so that half is held by the reviewer
of the change that adds it.

### What carries no code

A diagnostic about the invocation, or about the machine a run is on, rather than
about a program's text has no code. `no input files`, `cannot read <path>` and
`--emit object needs clang` are that kind, and eleven of the twelve diagnostics
`crates/safec/src/driver.rs` builds with `Diagnostic::error` carry none. There
is no class of program for a reader to search for and nothing for an
explanation to hang on, so a number there would be a handle onto nothing.

`SC0801` is the one of those twelve that does not, and is the other kind. The
backend refusing an IR shape is a fact about a function in a program, with a
span to point at and a class of program to search for, so it takes a code the
way the lowering's `SC0304` does.
The refusal is made in `crates/safec-llvm`, which cannot see a `Diagnostic` at
all, so the code is attached where the diagnostic is built.

**The safety checks' codes are a third kind and are why the count needs a
second command.** `SC0401`, `SC0402` and `SC0404` are built by `memory_finding` and
`SC0403` and `SC0405` by `nullability_finding`, each with
`Diagnostic::concluded` rather than `Diagnostic::error`, because what
a safety check answers is a [conclusion](safety-model.md#safe-unsafe-unknown)
and the severity follows from it: the same finding is an error or a warning
depending on what could be proved, so the constructor that decides has to be
the one that reads the conclusion. The finding is made in `crates/safec-ir`,
which cannot see a `Diagnostic` either, and for the same reason as the
backend's.

**There is a fourth kind, and it crosses the two above.** `driver.rs`'s
`undelivered` says that a run asked to be held to a level it cannot deliver. That
is a fact about the invocation, so by the rule above it carries no code and has no
span, and it is built with `Diagnostic::concluded` all the same, because a check
that never ran established nothing and being unable to establish something is a
[conclusion](safety-model.md#safe-unsafe-unknown). So `concluded` is no longer the
same question as "what a safety check answers": two of its three call sites are
checks and the third is the invocation. ADR-0035 is the decision and says what it
costs.

**The count is checked by nobody and has been wrong twice.** It said six while
`driver.rs` built seven, from the change that added a refusal without coming
back here; and a comment beside the code in that file counted the uncoded ones
as five while there were six, from the change that added `--emit object`. That
comment no longer counts anything, because this is the one place that does. The
count goes down as well as up: `--emit executable` removed `the compilation
pipeline is not implemented yet`, which nothing could reach once every kind
produced something. **It has since been wrong a third time**, found by a review
of the change that added `SC0401`: it said ten of eleven while the file built
twelve. **And a fourth time**, found by two review lenses independently on the
change that added `undelivered`: the second command below answered three while
this document said two and called them what a check answers.

A number in prose about code in another file is exactly RK-017's shape, and two
commands settle this one, which is one more than it used to take:

```sh
grep -c "Diagnostic::error("     crates/safec/src/driver.rs   # the twelve
grep -c "Diagnostic::concluded(" crates/safec/src/driver.rs   # the three: two checks, one invocation
```

The second exists because a safety check does not build its diagnostic the same
way, and a count that runs only the first will be wrong again the moment the
next check lands. Its comment says how the three divide rather than only how many
there are, because the number alone was true while the sentence beside it was
not.

The open parenthesis is not decoration. Without it both commands count the
prose beside the code as well as the code, and the second went from answering
one to answering three the moment a comment named the constructor it was
counting. A command that a comment can move is not settling anything.

## Remedies

**A note says what happened. A remedy says what to change.** The two render
differently, `= note:` and `= help:`, and they are different fields on a
`Diagnostic` rather than one field with a convention about which sentences go
in it.

A safety check cannot report without one. `Diagnostic::concluded` takes a
remedy as an argument rather than offering a `with_` method for it, so a
rejection built without one does not compile. That is the whole mechanism, and
[ADR-0034](adr/0034-a-diagnostic-that-rejects-a-program-carries-the-change-that-would-make-it-compile.md)
records why it is a field rather than a diagnostic reported alongside.

**The hole is that `Diagnostic::error` is public.** A check that builds a
rejection through it instead carries no remedy and nothing says so. What that
costs is a rejection reaching a reader without its remedy, which is visible in
the output and in a corpus expectation rather than silent.

**A remedy is a claim, like a label is.** So one is written against what its
row established rather than against what its words suggest, and two rows here
are worth reading for what they do not say.

`Unproven::Lost` names no cause. That variant has five producers and a
`Finding` does not say which answered, so `nothing here says the program is
wrong: this check could no longer say which allocation this pointer holds` is
what is true of all five. It tells a reader the one thing that matters there,
which is not to go hunting for a defect.
[#213](https://github.com/itsakeyfut/safec/issues/213) carries the reason and
replaces it with five.

`Unproven::Disagreement` has two causes of its own, paths that disagree about a
free and a call this check cannot read, and a `Finding` does not separate those
either. Its remedy names what would let the check conclude rather than which of
the two happened, which is why it has two halves.

`Unproven::Unsequenced` is the one remedy about something C decided rather than
about what the program meant, so it asks for a statement boundary rather than
for the free to move. See [ADR-0022](adr/0022-say-where-c-sequences-one-evaluation-before-another.md).

## Where this lives now

Implemented in `crates/safec/src/diagnostics.rs`, with the terminal renderer in
`crates/safec/src/diagnostics/render.rs`. What a check concludes, and how that
becomes a severity, is [ADR-0001](adr/0001-promote-unproven-results-in-the-sink.md).
