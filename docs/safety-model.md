# Safety Model

The core abstraction should eventually model:

- Value
- Place
- Ownership
- Lifetime
- Region
- Thread
- Capability

For example, an ownership state machine could look like:

```text
ALLOCATED
    ↓
OWNED
    ↓
MOVED
    ↓
INVALID
```

A borrowed value could be represented as:

```text
owner
  │
  └── borrow
        │
        └── lifetime tied to owner
```

The compiler should be able to detect errors such as:

```c
int *p = malloc(sizeof(int));

free(p);

*p = 42;
```

and report:

```text
error:
use of freed value 'p'

p allocated here
p freed here
dereferenced here
```

Similarly:

```c
int *foo() {
    int x;
    return &x;
}
```

should eventually produce a diagnostic such as:

```text
error:
pointer escapes lifetime of local variable 'x'
```

## Safe, Unsafe, Unknown

The analyzer should distinguish:

```text
Safe
Unsafe
Unknown
```

rather than pretending that every C program can be proven safe.

Inference is what a check concludes; enforcement is what that means for the
build. The two are separated by reporting the conclusion and deciding the
severity in one place:

| The analysis concluded | What is reported | Under `--allow-unknown` |
|---|---|---|
| Safe | nothing | nothing |
| Unsafe | an error | an error |
| Unknown | **an error** | a warning |

The first column of answers is the default, wherever a check runs at all. At
`--safety off` nothing runs and there is no conclusion to report, so the table
is about level 1 and upward.

A check names its conclusion and never reads the policy. `DiagnosticSink` reads
it, once, so no check can forget to apply it: a missed promotion would let the
compiler exit successfully on code it never managed to check, which is the worst
thing it can do. See
[ADR-0001](adr/0001-promote-unproven-results-in-the-sink.md).

**A build that succeeded is a claim.** Reporting `Unknown` as a warning by
default would belong to a compiler that accepts every program and comments on
it: a successful build would mean either that the program was proved or that
the analysis gave up, with one exit code for both, and the distinction this
section is built on would survive only in text somebody may not have read. This
project asks instead that a program be rewritten until it can be proved, and
`safec` says what to change. See
[ADR-0033](adr/0033-a-conclusion-this-analysis-could-not-prove-does-not-build.md),
which also records what that costs: the false positive rate becomes something a
user feels rather than something they can ignore, and the answer to it is the
hatch and the annotation below.

`Unsafe` says that **some** execution of the function is undefined, not that
every one is. A check here reads one function at a time, so a parameter ranges
over every value a caller may pass rather than over the calls this translation
unit happens to contain: a parameter freed twice is a proved double free though
the only caller in the file passes a null pointer, which C17 7.22.3.3 p2 would
make well defined. What removes something from the question is a fact the
analysis established rather than one it could not rule out, so a pointer proved
null is exempt and a pointer nothing is known about is not. That exemption
reaches a free whose argument is written as a null pointer constant and a free
of a local the nullability check established is null, which are the two
spellings of one program. It is wider than the clause in one place and narrower
in another, both on purpose: the constant arm skips every constant rather than
only zero, so `free(17)` is exempt from this check and is a constraint
violation for the frontend to refuse, which is #154; a local that does not hold
a pointer is never established null, so `int x = 0; free(x);` is reported rather
than exempted; and neither is a free that is not at the root of its full
expression, so `int x = (p = 0, 1) + (free(p), 0);` is reported, and so is
`int x = 1 + (free(p), 0);` after an earlier `p = 0;` although C defines it.
The first of those two is undefined by C17 6.5 p2, not merely undefined on one
of the orders, and the second is the false positive the rule costs. See
[ADR-0027](adr/0027-an-unsafe-conclusion-is-about-an-execution-this-function-has.md).

**The memory level asks whether an access fits the object it reaches**, as well
as whether the object is live. An allocation asked for with a constant size
takes exactly that, a local takes its type's size, and a pointer that arrives
from outside the function, a parameter, a call's result or a pointer read out
of memory, is believed to point at one whole object of the type it arrives as.
An access is proved to fit only from those; it is `Unsafe` where every object it
may reach is known exactly and is narrower than the access, and `Unknown`
everywhere else, a pointer moved off its start among them. What is believed
where a pointer arrives is asked where it leaves, passed to a call, returned
or stored, with the pointers stored in what it points at, each as wide as the
receiver reads it; a `void *` is asked only whether it starts an object, since
it carries no width. Memory a pointer arrives pointing at is believed to hold
what the type it arrived as says, at every level, through any conversion,
copy, or store and reload, so a `void **` read through an `int **` believes
nothing of what it reads. What is
asked there is
never a proof, since C leaves the departure itself defined. See
[ADR-0060](adr/0060-an-access-is-proved-inside-its-object-only-where-its-size-was-read.md).


## Incremental Safety Levels

Avoid requiring every existing C program to become fully safe immediately.

A possible model:

```text
Level 0
  Ordinary C

Level 1
  Memory checked

Level 2
  Lifetime checked

Level 3
  Ownership checked

Level 4
  Thread checked

Level 5
  Fully safety-checked subset
```

This allows incremental adoption.

**An indeterminate pointer is level 1's.** A write, a read or a free through a
pointer that points at no object is the same class of memory error as one
through a pointer to freed memory, so the memory check reports it, as
`SC0411`: a pointer local read before anything was assigned to it, and a
pointer read out of an allocation nothing wrote. The second is reported
where it is read, whatever it is read for, since Annex J.2 lists any use of
it.

At levels 1 to 4, a level says **which checks run**, and nothing else. It does
not decide how
loudly a check speaks: a result the analysis proved is an error at every level,
because there is no safety argument for knowing something is wrong and saying it
quietly. What varies is the result the analysis could *not* prove, and that is
governed by the policy rather than by which checks a level selects. The level
decides one thing about it: whether any check runs, and so whether there is a
conclusion to promote. See [Safe, Unsafe, Unknown](#safe-unsafe-unknown) above,
[ADR-0001](adr/0001-promote-unproven-results-in-the-sink.md) and
[ADR-0033](adr/0033-a-conclusion-this-analysis-could-not-prove-does-not-build.md).

Level 5 is defined as leaving nothing `Unknown`, so `--allow-unknown` is refused
beside `--safety strict` rather than ignored.

**Level 5 is the other kind of safety: it restricts what may be written.**
Levels 1 to 4 accept any C the frontend accepts, so an existing program adopts
the checks without being rewritten first. Some programs cannot be proved as they
are written, so leaving nothing `Unknown` takes a subset: level 5 does not accept
some of what C allows. A construct outside it is reported by the check that
already settles it where one does, as null is, and otherwise as a diagnostic
about the program's text, with a code of its own and a remedy saying how to write
it inside, rather than as a conclusion it could not prove. The level is chosen
per translation unit, and what a declaration from code the translation unit
cannot see does not say is answered conservatively. Levels are not mixed inside
one: a function to be held to level 5 before the rest of its file is moved to a
file of its own, compiled by its own run, and meets the rest through
declarations, so that boundary is the one between them. A hatch is not a lower
level, only a place where what was not proved is listed.
[ADR-0053](adr/0053-levels-are-not-mixed-inside-a-translation-unit.md) has why. What level 5 restricts is a list, and an entry is
added by the phase whose check needs it, with a record of its own; the first is
null, under [Annotations](#annotations) below.
[ADR-0049](adr/0049-level-5-restricts-what-may-be-written-and-levels-1-to-4-do-not.md)
has the reasoning and the options it rejected. None of it is implemented yet;
`--safety strict` is wired in Phase 8.

**A level can be asked for and not delivered**, and two independent things cause
it: the checks a level selects may not exist yet, which is true of every level
above 1 today, and the artifact may stop before the Safety IR the checks read,
which is true of `--emit tokens` and `--emit ast`. A run therefore delivers the
lowest of three things, the level it asked for, the highest level with checks
behind it, and what its artifact can carry. Where that is below what was asked
for, the run reports what it did not establish, as a conclusion it could not
prove rather than as a refusal, so `--allow-unknown` is what a program still
being migrated reaches for here as well. It follows that the level a run
defaults to is what its artifact can carry rather than one level for all of
them: `--emit ast` claims nothing about safety and says nothing.
[ADR-0035](adr/0035-a-level-a-run-cannot-deliver-is-a-conclusion-it-could-not-prove.md)
has the reasoning and the four options it rejected.

For example:

```bash
safec --safety=off main.c
```

is the way in, and compiles ordinary C with nothing said about it, while:

```bash
safec main.c
```

is level 1 and rejects what it cannot prove, because `--safety` defaults to
`memory`. A program that is still being migrated asks for the middle with
`--allow-unknown`, which reports the same conclusions and lets the build
through.

What is decided is the two kinds, the first restriction, and that levels are
not mixed inside a translation unit. Which restrictions follow is decided as the
phases that need them are built.

## Annotations

Pure inference cannot always recover the ownership and lifetime semantics missing from ordinary C.

For example:

```c
void foo(int *p);
```

does not necessarily tell us:

- Who owns `p`?
- How long is `p` valid?
- Can `p` be NULL?
- Can another thread access `p`?
- Is ownership transferred?

Therefore the language may eventually support explicit annotations.

Possible experimental syntax:

```c
void process(borrow int *p);

owner int *create();

void consume(owner int *p);
```

The exact syntax is not fixed.

**Two annotations answer the third question.** `_Nonnull`, written after the
`*` of a pointer parameter, says the parameter is not null:

```c
void process(int * _Nonnull p) { *p = 1; }
```

The body believes it, so the read above is not reported, and every call in the
same translation unit is checked against it, so `process(0)` is an error and so
is passing a pointer that nothing established is not null. What is believed
without a check is only what no such call reaches: a caller in another
translation unit, another file of the same `safec` run among them, or one
compiled by something else. That is the boundary promise
[ADR-0032](adr/0032-bound-what-is-unchecked-inside-a-declared-hatch.md) asks
of a hatch, at the scale of one declaration.

Written after the `*` of the pointer a function returns, it says the same of the
result: every `return` in the body is checked against it, so `return 0;` is an
error and so is reaching the end of the body without a `return`, and every call
believes it of what it returns. `_Nullable`, the other specifier, says a
pointer may be null, which below level 5 is what an unannotated one already
is.

It is `clang`'s spelling rather than one in the style sketched above, because it
is reserved to the implementation, `clang` compiles it unchanged, and it changes
no code. [ADR-0037](adr/0037-a-nonnull-parameter-is-believed-by-its-body-and-checked-at-every-call-in-its-translation-unit.md)
has the reasoning and the options it rejected, and
[`frontend.md`](frontend.md#where-a-nullability-specifier-is-read) says where it is read and
where it is refused. It says nothing about the other four questions, which wait
for the phases that ask them.

**At level 5 the default turns over.** A pointer is not null unless it is written
`_Nullable`, the other nullability specifier `clang` accepts, and the
nullability check reports giving a pointer that is not `_Nullable` a value that
may be null, as it does for a `_Nonnull` parameter today. Across a translation
unit's boundary nothing unannotated is believed: what a function declared and
not defined here takes and returns, and what a function with external linkage
defined here takes, is `_Nullable` where the declaration says neither, since
believing what the other side never said is how a null would be dereferenced in
silence. The default covers a declaration's own pointers, a parameter's and
the one a function returns, and not a local, whose every write the check
already sees, or a pointer inside another, which is believed of nothing until a
specifier can be written on it. Every function here has external linkage
today, because the parser reads no storage class, so what the default changes
is the pointer a function defined here returns, and a `return` that may hand
back null is `SC0408`. Below level 5, `_Nullable` restricts nothing.
[ADR-0050](adr/0050-at-level-5-a-pointer-is-non-null-unless-it-is-written-nullable.md)
has the reasoning and the measurement. Until the checks below level 5 are
implemented, a run asked for it is refused as asking for more than it
delivers, and what the default finds is reported beside that.

**A `#pragma clang assume_nonnull` region will be read as a written
`_Nonnull`**, once the preprocessor reads `#pragma` at all, which it does not
yet. Inside one, a declaration's own single-level pointer that writes neither
specifier is a promise, believed across the boundary as a written `_Nonnull`
is, where the paragraph above believes nothing, so a header written for level
5 need not annotate each pointer to build clean under `clang`. A pointer to a
pointer is not covered, a written `_Nullable` wins, a hatch promises only what
it writes, and a region not closed in the file it opened in is refused and
promises nothing.
[ADR-0056](adr/0056-a-pragma-clang-assume-nonnull-region-is-read-as-a-written-nonnull.md)
has the reasoning and what `clang` was measured to do.

The desired migration model is:

```text
Existing C
   ↓
Safety inference
   ↓
Refusals / uncertainty
   ↓
Add annotations where necessary
   ↓
Safety-checked C
```

This should be explored experimentally rather than decided prematurely.

### Hatches

What an annotation cannot reach is code whose safety no check can establish:
an allocator, a call into a library nobody annotated, a stretch of pointer work
the analysis loses track of. Something has to hold what is trusted there, or
the levels above have nothing to stand on.
[ADR-0032](adr/0032-bound-what-is-unchecked-inside-a-declared-hatch.md) decides
the stance: a declared hatch claims nothing about its body, carries a promise at
its boundary, and the checked side reads the promise.

**A hatch is a function definition.** Written before one,

```c
__attribute__((annotate("safec_unchecked")))
int raw(int *p) { return *p; }
```

it changes what a conclusion inside the body is about, and not what the check
concluded. The checks run over the body as over any other. What they could not
prove there is not reported, because it is a statement about the hatch rather
than about the program, and `--emit hatches` lists it under the hatch instead.
What they proved is reported as it would be anywhere, because a hatch is for
what cannot be proved and a program proved undefined is not that. The same
command lists every hatch, so the set of them is something an audit can count.
[ADR-0038](adr/0038-a-hatch-is-a-function-definition-and-what-it-could-not-prove-is-listed-rather-than-reported.md)
has the reasoning and the forms it rejected, and
[`frontend.md`](frontend.md#where-the-hatch-is-read) says where it is read.

**Its boundary is its prototype.** A `_Nonnull` parameter of a hatch is
checked at every call in the translation unit, as any other is, and a
`_Nonnull` written on the pointer a hatch returns is believed at every call,
while a `return` that does not keep it is listed under the hatch. Level 5's
default is not made for a hatch, so it promises only what it writes. Otherwise a
caller assumes the worst of what the hatch may have done: after a call to one, every
allocation the caller still holds live is unproven, whether or not the hatch
was handed it, because what a hatch can reach through memory is not something
this check follows and its body is the one whose unproven conclusions are not
reported. An allocation already proved freed stays proved. A hatch cannot
yet say otherwise; declaring what it writes and frees, and a hatch that is a
region inside a function rather than a whole one, are
[#249](https://github.com/itsakeyfut/safec/issues/249).

**So the guarantee is conditional, and says on what.** A program that builds
is proved outside its hatches, given the promises at their boundaries. A
promise that is wrong is not detected: nothing checks a hatch's body against
its prototype, and a caller in another translation unit is not checked against
a `_Nonnull` either. That is the cost of having somewhere to put what cannot be
proved, and what bounds it is that each hatch is written at a named place, in a
list a command can print.

**A function this check cannot read is the other boundary.** A call to one
whose body is not in the translation unit is believed to return a fresh
allocation, or one code this check cannot read could already reach, and never
one that the call itself, or another call this check cannot read, freed
([ADR-0039](adr/0039-an-allocation-code-this-check-cannot-read-may-reach-stays-exposed-while-it-lives.md)).
So a result that is no allocation at all, a string literal, an object with
static storage duration or a local of another frame the callee was handed, is
believed to be one, and freeing it is not detected either
([#375](https://github.com/itsakeyfut/safec/issues/375)).
Where the callee's body is compiled here, what it returns is asked at its
`return`
([ADR-0041](adr/0041-a-pointer-a-function-returns-is-asked-at-its-return-as-a-dereference-of-it-would-be.md)),
so a callee that hands back what it freed itself is reported there. Where it
is not, or where what it hands back was freed by another function, a wrong
belief is not detected, as a wrong hatch promise is not: `release(); r =
acquire(); free(r);` builds where `release` freed what `acquire` hands out
([#379](https://github.com/itsakeyfut/safec/issues/379)). No reading of the
caller's own code closes that, since nothing in it tells the defined executions
from the undefined ones
([#393](https://github.com/itsakeyfut/safec/issues/393)); what would is a
declaration that says whether a result hands ownership over, the `owner` sketch
under *Annotations* above.
