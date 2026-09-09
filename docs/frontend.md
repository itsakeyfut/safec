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

`lexer.rs` keeps its own list, under *Not here yet*, and it is a different list
on purpose rather than a copy of this one: it is what the *scan* does not do, so
it also holds literal prefixes and non-ASCII identifiers, which are a token's
spelling rather than a program this compiler refuses. Two of the three above are
on it. `$` is not, and that is the distinction the paragraph before this one
draws: declining to fill a blank C offers is not something the scan fails to
do.

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
