# Frontend Strategy

Initially use a hand-written lexer and recursive-descent parser.

Avoid depending on parser-generator libraries at the beginning.

Reasons:

- Better understanding of C parsing
- Easier experimentation
- Easier debugging
- Keeps the compiler architecture explicit
- Fits the "Hackable" philosophy

The C parser should be implemented incrementally.

## Stage 1

Support a practical subset:

- `int`
- `char`
- `void`
- variables
- functions
- `if`
- `while`
- `for`
- `return`
- pointers
- arrays
- structs

## Stage 2

Add:

- `typedef`
- `enum`
- `union`
- function pointers
- variadic functions

## Stage 3

Add preprocessor functionality:

- macros
- `#include`
- conditional compilation

These stages order the parser's growth, not the project's phases, and the
preprocessor is where the two diverge. [roadmap.md](roadmap.md) pulls a minimal
subset of this stage forward into Phase 1: `#include`, object-like `#define`,
`#ifdef` and `#ifndef`. What that buys is that every later phase can be tested
against a real header rather than a hand-written prototype. The rest of the
stage, and all of Stage 2, stay in Phase 9.

## What is refused today

The stages above say what the frontend is going to accept. This says what it
refuses now, because a reader has no other way to tell a decision from a gap.
Every line was measured against the built compiler and against
`clang 20.1.6 --target=x86_64-unknown-linux-gnu`; the target matters, because
`clang`'s default here is MSVC and accepts things C17 does not.

| Written | This compiler | `clang` | `clang -pedantic-errors` |
|---|---|---|---|
| `int \u00e9 = 1;` | `error[SC0103]` at the backslash | accepts | accepts |
| `ma\` newline `in(void)` | `error[SC0103]` at the backslash | accepts | accepts |
| `int a$b = 1;` | `error[SC0103]` at the `$` | accepts | `error: '$' in identifier` |

**The first two are C and are not implemented.** 6.4.2.1 puts
`universal-character-name` in `identifier-nondigit`, so `\u00e9` is an
identifier written in ASCII rather than an extension, and 5.1.1.2 p1 phase 2
deletes a backslash before a newline "splicing physical source lines to form
logical source lines", before anything is tokenized. A conforming compiler reads
both.

**The third is a choice, and this compiler makes the other one.** The same
production ends in `other implementation-defined characters`, so `$` in an
identifier is a blank C leaves for an implementation to fill. `clang` fills it
and says so only when asked; this compiler leaves it empty and reports. Neither
is more conforming than the other, and a program that relies on it is relying on
an implementation.

None of the three is a decision anybody has taken to refuse forever. They are
where the lexer stopped, and this section exists so that stopping there is
visible rather than inferred from a diagnostic.

Two more are where the *parser* stopped, and belong here for the same reason.

| Written | This compiler | `clang` | `clang -pedantic-errors` |
|---|---|---|---|
| `int x = {1};` | `error[SC0203]` at the `{` | accepts | accepts |
| `for (int i = 0; ...)` | `error[SC0201]` at the `int` | accepts | accepts |

**Both are C and are not implemented.** 6.7.9 p11 lets a scalar's initializer
be a single expression "optionally enclosed in braces", so the first is valid
with no designator and no nested list in it; unwrapping the braces is only
correct where the declared type is scalar, and the parser does not know the
type. 6.8.5 p1 gives `for` a form whose first clause is a declaration, and
6.8.5 p5 scopes that declaration to the loop rather than to the block around
it; the tree holds an expression there.

Neither is a gap the lexer left, which is why they are a table of their own:
the first three rows are about what a token is, and these two are about what a
sequence of them is allowed to be.

`lexer.rs` keeps its own list, under *Not here yet*, and it is a different list
on purpose rather than a copy of this one: it is what the *scan* does not do, so
it also holds literal prefixes and non-ASCII identifiers, which are a token's
spelling rather than a program this compiler refuses. Two of the three above are
on it. `$` is not, and that is the distinction the paragraph before this one
draws: declining to fill a blank C offers is not something the scan fails to
do.

Two more are where the type checker declines an extension.

| Written | This compiler | `clang` | `clang -pedantic-errors` |
|---|---|---|---|
| `v + 1` with `void *v` | `error[SC0306]` | accepts | `error: arithmetic on a pointer to void is a GNU extension` |
| `fp + 1` with `int (*fp)(void)` | `error[SC0306]` | accepts | `error: arithmetic on a pointer to the function type 'int (void)' is a GNU extension` |

**Both are constraint violations, and this compiler takes neither extension.** C17
6.5.6 p2 lets `+` step only "a pointer to a complete object type", p3 says the
same of `-`, and 6.5.16.2 p1 says it of `+=` and `-=`; `void` is incomplete
and a function is not an object. `clang` gives each a size of one unless asked
to be pedantic. `-=` and `v - v` answer the same, because
`types.rs::unsteppable` is the one place the additive operators and their
compound assignments ask. `++`, `--` and a subscript are the same rule by C17
6.5.2.4 p2, 6.5.3.1 p2 and 6.5.2.1 p1, and do not ask yet: `v++` and `v[1]`
pass the type checker, and what refuses them later is about something else,
`SC0801` about the backend and, for `v[1]`, `SC0403` about a null pointer
([#242](https://github.com/itsakeyfut/safec/issues/242)). A pointer to an array
of unknown length is refused by the same rule and is not a row, because `clang`
refuses it too, with or without `-pedantic-errors`.

### What an integer constant is worth, and what type it is not

The scan settles where a constant ends and stops there. `crates/safec/src/types.rs`
is what reads it, because deciding the type needs the range of `int` and the
scan is below the target. Three bases, C17 6.4.4.1 p1: a leading `0x` or `0X`
is hexadecimal, a leading `0` is octal, and anything else is decimal, so `010`
is eight. The suffixes of p1 are read far enough to reject a spelling C does not
have, and then discarded.

**Rejected, because there is no type to give them to.** 6.4.4.1 p5 asks for the
first type in a list that holds the value, and the list is `int`,
`unsigned int`, `long`, and so on. `crates/safec/src/ast.rs`'s `Type` has `Int`,
`Char`, `Void` and the types derived from them, and
`crates/safec-ir/src/target.rs`'s `Target` answers `int()` and `char()`. There
is no `unsigned int` and no `long` for p5's table to select, so `int` is the
only type a constant can get here, and a constant p5 would give any other is
reported rather than read as an `int`.

**A suffix is not decoration, which is why it is refused rather than dropped.**
It decides what arithmetic on the constant means: 6.4.4.1 p5 makes `3u` an
`unsigned int`, 6.3.1.8's conversions then make `-6 / 3u` an unsigned division,
and its value is 1431655763 and not -2. An earlier version of this section said
the suffix was read and then discarded, and priced the divergence entirely in
refused programs; that was false, and what it left out was the expensive half.
Discarding it compiled `-6 / 3u` to a signed division and `2147483647l + 1l` to
a 32-bit `add nsw`, both silently. A refusal is visible and a wrongly-signed
division is not.

What the missing types cost is the rows below. Measured against
`clang 20.1.6 --target=x86_64-unknown-linux-gnu`, as the tables above are.

| Written | This compiler | `clang` | `clang -pedantic-errors` |
|---|---|---|---|
| `int x = 2147483648;` | `error[SC0305]` | accepts | accepts |
| `int x = 1u;` | `error[SC0305]` | accepts | accepts |
| `int x = 0L;` | `error[SC0305]` | accepts | accepts |
| `int x = 1.5;` | `error[SC0305]` | accepts | accepts |
| `int x = 09;` | `error[SC0106]` | error | error |
| `int x = 1lL;` | `error[SC0106]` | error | error |
| `int x = 1e;` | `error[SC0106]` | error | error |
| `int x = 0x1.8;` | `error[SC0106]` | error | error |
| `int x = 0b101;` | `error[SC0106]` | accepts | error |

**The first four are gaps and the rest are not.** `SC0305` says this compiler
has no type for a constant C gives one to, which is what the paragraphs above
are about; a floating constant is the same sentence with no floating type in
it. `SC0106` says the spelling is not a constant at all, and `clang` refuses
each of those too.

The last four are where that line is easiest to draw wrongly, so each is
measured rather than reasoned about. `1e` has an exponent with no digits and
`0x1.8` is a hexadecimal floating constant with no binary exponent, both of
which 6.4.4.2 p1 declines to spell, so they are the program's mistake and not a
type this compiler is short of. `0b101` shows why the fourth column is there: a
binary constant is C23, `clang` takes it as an extension, and only
`-pedantic-errors` answers for C17. RK-032 in the review knowledge bank is what
that column exists for.

A value beyond `INT_MAX` is reported as this compiler's gap rather than as a
violation of 6.4.4 p2, although the two are the same sentence read against this
implementation's type list. Blaming the program would be telling someone whose
program `clang` compiles that they wrote it wrong, when what is missing is
`long`. `int x = -2147483648;` is inside that: the constant is 2147483648 and
the minus is an operator, so the smallest `int` cannot be written yet.

**Whether `SC0305` fires depends on the target**, because it is decided against
the range of `int`. [`architecture.md`](architecture.md)'s output table says a
diagnostic depends on neither the host nor the target, and this is a divergence
from it, recorded there as well. Nothing observes it today: `int` is 32 bits on
every row of `Target::ALL`. It becomes observable the day `long` is in that
table, because `long` is 32 bits on `x86_64-pc-windows-msvc` and 64 on
`x86_64-unknown-linux-gnu`.

Character constants (6.4.4.4) are not here, because the parser builds no
expression for one: `crates/safec/src/parser.rs`'s `primary` reads a number and
an identifier, and a character constant is refused as `expected an expression`.

### Which zero is a null pointer constant

| Written | This compiler | `clang` | `clang -pedantic-errors` |
|---|---|---|---|
| `int *p = 0;` | accepts | accepts | accepts |
| `int *p = 1 - 1;` | `error[SC0302]` | accepts | accepts |
| `int *p = -0;` | `error[SC0302]` | accepts | accepts |
| `p = 1 - 1;` with `int *p` | `error[SC0302]` | accepts | accepts |
| `p == 1 - 1` with `int *p` | `error[SC0306]` | accepts | accepts |
| `p == -0` with `int *p` | `error[SC0306]` | accepts | accepts |

**These are refused on purpose.** C17 6.3.2.3 p3 makes any integer
constant expression with the value 0 a null pointer constant, and nothing here
evaluates a constant expression, so only a literal zero is recognised as one.
The alternative was not reporting an integer given to a pointer at all, which
is the mistake the check exists for; `types.rs::is_null_pointer_constant` says
why the false report costs less. C17 6.7.9 p11 gives an initializer the
constraints of simple assignment, so the two spellings answer alike, and C17
6.5.9 p2 lets a pointer be compared with a null pointer constant and no other
integer, so a comparison answers alike too. The rows stop being refused the day
a constant expression can be evaluated.

### What the lowering refuses

The frontend accepts these and `crates/safec/src/lowering.rs` cannot build IR
for them, so they are reported as `SC0304` and the function they are in is left
a declaration. They are gaps rather than decisions, and each one is a program
`clang 20.1.6` compiles.

| Written | Why it stops here |
|---|---|
| `int a[3];`, or any array or function type | the IR holds `int`, `char`, `void` and pointers to them |
| `int g;` at file scope, used inside a function | every place the IR can name starts at a local |
| `1 = 2` | C17 6.5.16 p2 wants a modifiable lvalue and nothing checks that yet, so the first thing to notice is a stage that needs somewhere to write |
| a second definition of one name | C17 6.9 p5 allows one, and nothing before this stage counts them |

The reason a refusal is reported rather than lowered around is that the IR has
no way to say a function has a hole in it: a body with a piece missing and a
body that is complete are the same shape, and an analysis reading the first
would answer about code that was never there. A declaration says the one true
thing, which is that the body is not here.

### What the interpreter answers differently

`crates/safec-ir/src/interp.rs` runs the IR so that a program can be tested
without a backend, and it computes at the target's widths. The unit says what
those are; ADR-0013 is where that decision lives, and `--target` is how a run
names one. Each row below was measured against the interpreter and against
`clang 20.1.6 --target=x86_64-pc-windows-msvc`, where `int` is 32 bits and
`char` is signed.

| Written | This interpreter | `clang` |
|---|---|---|
| `int x = 2147483647; return x + 1;` | stops | `-2147483648` |
| `int x = 1; return x << 31;` | stops | `-2147483648` |
| `char c = 300; return c;` | `44` | `44` |
| `char c = 200; return c;` | `-56`, and `200` for an unsigned `char` | the same |

**The first two are undefined and the last two are not**, and that is the
difference these four rows are about. It is not a claim that the table is
exhaustive: what a program means here is measured case by case against `clang`,
and a row appears when a measurement disagrees. C17 6.5 p5 leaves an operation undefined whose result "is not
in the range of representable values for its type", and 6.5.7 p4 says the same
of a left shift whose value is not representable, which `1 << 31` is not at 32
bits. This stops on both rather than answering, which is what it does for a
division by zero and is the opposite of what a real machine does. A compiler
that answered `-2147483648` would be deciding what C declined to.

`x << 32` is the third undefined one and is not in the table, because there is
no number to put in the last column: it is a shift count at least the width,
which C17 6.5.7 p3 leaves undefined, and what `clang` answers is whatever the
hardware did. Measured once here it was `1324023808`, and it is not a fact worth
writing down. `clang` warns about it, and this stops on it.

The last two are conversions rather than operations. C17 6.5.16.1 p2 converts
the value of an assignment's right operand to the type of the assignment, and
6.3.1.3 says what that gives, so nothing is undefined and nothing stops. 300
truncates to `0x2C` and 44 is positive, so the sign does not show; 200 is where
it does, and whether `char` carries one is the target's to say under C17 6.2.5
p15. Of the targets this compiler knows, `aarch64-unknown-linux-gnu` is the one
where it does not.

A run is bounded twice: by how deep the calls go and by how many blocks it
enters. A program that would answer after more steps than the second bound
allows is stopped instead, which is the trade a test instrument makes so that a
suite fails rather than hangs.

A read through a pointer whose object's scope has ended stops the run and says
which, rather than answering whatever is still in the slot, and so does a write
through the same pointer. The write matters as much: one that went through would
put a value into a slot the next scope at that depth is about to use, so the
program that paid for it would not be the one that did it. The IR says where a
block-scoped local's storage began and ended, which is what makes the two
tellable apart; ADR-0012 has the shape and #74 built it. A local the function
itself declares has no such marker, because its storage is the frame's and a
pointer into a frame that has returned is already caught.

One decision rather than a gap. A pointer is a place, so there is no null: a
comparison of a pointer with a zero constant is answered as unequal rather than
by a value that could be either.

## One problem, one diagnostic

**An input the scan reported anything about is not parsed**, and one the parser
gave up on has its names and types left alone. So one problem produces one
diagnostic here where `clang` produces several:

| Written | This compiler | `clang` |
|---|---|---|
| `int main(void) { int x = @; return x }` | `error[SC0103]` at the `@` | two syntax errors, and no lexical one |
| `#define N 4` and a use of `N` | `error[SC0104]` at the directive | compiles it |

Both were measured against the built compiler and against
`clang 20.1.6 -std=c17 --target=x86_64-unknown-linux-gnu`. The first row is a
whole program on purpose: the same statements at file scope are three errors
rather than two, because `return` outside a function is a different mistake.

`clang` has no lexical diagnostic for `@` at all. `-Xclang -dump-tokens` shows
it handing the parser `unknown '@'` and letting the grammar refuse it, which is
why both of its errors are syntax errors. This compiler reports the character
where the scan met it, and stops.

**The rule is "reported anything", not "could not read it whole",** and the
difference is worth stating because the second is what a reader would guess.
`char c = '';` is a complete token: nothing is lost, `SC0105` is reported, and
the missing `;` after it is not. All five lexical codes are under the rule,
`SC0101` to `SC0105`. The lexer knows which of its diagnostics dropped input
and the driver does not, and nothing carries that distinction today; until
something does, the wider rule is the one that cannot be wrong in the direction
that matters.

**What it costs is the rest of the file.** A stray character on line 500 means
the syntax errors on lines 1 to 499 are not reported either, and a user fixes
them one run at a time. `clang` recovers instead and says everything it can in
one pass, which is the better answer for somebody working through a large file.

**What it buys is that no stage speaks about a program an earlier one did not
finish reading.** The parser's report on a token stream with a hole in it is a
second diagnostic about the first one's problem, which is the first row above.
Names and types were already held back for the same reason and by the same
count, one stage later; what the parser's gate adds is that second diagnostic
and the tree, which is why the second row prints nothing at all.

The gate is per input, so `a.c` failing does not stop `b.c` being read.
`driver.rs::compile` is the one place that decision lives.

## Stage 4

Work toward broader C compatibility, potentially C11/C17.

Do not target complete C compatibility from day one.
