---
status: "accepted"
date: 2026-09-06
decision-makers: itsakeyfut
---

# Hold each source file behind an `Arc` so one can be read while another is added

## Context and Problem Statement

`SourceMap` stored `Vec<SourceFile>` and handed out `&SourceFile` from
`SourceMap::file`. `SourceMap::load` takes `&mut self`. A caller therefore
cannot read one file and add another at the same time.

That is exactly what `#include` is. A lexer scanning `a.c` holds its text,
meets a directive, and has to put `a.h` into the map before it can carry on:

```rust
let text = sources.file(id).contents();
while offset < text.len() {
    if text[offset..].starts_with("#include \"") {
        let _header = sources.load("a.h");
    }
    // ...
}
```

```
error[E0502]: cannot borrow `*sources` as mutable because it is also borrowed as immutable
```

[ADR-0003](0003-pass-the-source-map-to-each-render-call.md) removed the
renderer's borrow of the map, and gave this as the reason: "a lexer that meets
an `#include` has to call `SourceMap::load`, and it cannot while the renderer
borrows the map." It fixed the renderer. The map itself was left as it was, so
the conflict is still there for the caller the record was written about, one
layer further in.

Nothing has hit it yet because nothing reads a file for longer than a statement.
That ends with the first lexer, which is Phase 1.

## Decision Drivers

* `#include` is not an exotic case. It is how C is written, and a compiler that
  cannot report a diagnostic inside a header is not usable.
* A borrow that outlives a statement dictates the order phases may run in. That
  is the same defect ADR-0003 was about, and the reason to settle it before
  there are phases written around it.
* The map's contents are the source being compiled. Copying them to work around
  the borrow is the one thing a compiler should not do to its own input.
* Phase 8 is thread safety analysis, and the roadmap mentions no parallel
  front end but does not rule one out. Whatever is chosen should not be the
  thing that makes sharing impossible later.
* Whatever is done must not disturb the renderer's cache, which borrows file
  text for the length of one call and is why ADR-0003 reads as it does.

## Considered Options

* **Put each file behind an `Arc`** and add an accessor that returns a handle.
* **Have the lexer copy the text** it is scanning out of the map.
* **Move the files into an arena or interner** that hands out indices and
  borrows nothing.
* **Run the preprocessor as a separate pass** that flattens a translation unit
  into one buffer before anything else reads it.

## Decision Outcome

Chosen option: **put each file behind an `Arc`**.

```rust
pub struct SourceMap {
    files: Vec<Arc<SourceFile>>,
}

impl SourceMap {
    pub fn file(&self, id: FileId) -> &SourceFile { &self.files[id.index()] }

    pub fn file_owned(&self, id: FileId) -> Arc<SourceFile> {
        Arc::clone(&self.files[id.index()])
    }
}
```

`file` keeps returning a borrow and is still the accessor to reach for. It has
to: the renderer's cache stores `Source<&'a str>` borrowed from the map for the
length of one render call, which is the arrangement ADR-0003 settled, and it
cannot be built from a handle that lives only as long as the expression. The two
accessors say different things at the call site, which is the point. `file` is
for a read that finishes; `file_owned` says the handle is meant to outlive the
borrow, and costs an atomic increment to say it.

`Arc` rather than `Rc`. Nothing is shared across threads today and the
difference is one atomic operation on a clone that happens once per file, not
once per token. Phase 8 is thread safety analysis, and an `Rc` in the substrate
every later phase is built on is the kind of thing that is found late and
changed everywhere at once.

The change is six lines in `source.rs` and nothing anywhere else. `file`,
`files`, `snippet`, the renderer and `SourceMapCache` all compile unchanged,
because `&self.files[i]` still derefs to a `&SourceFile` living in the map.

### Confirmation

`a_file_can_be_read_while_another_is_added` in `crates/safec/src/source.rs`
holds a file's text, adds a second file to the map, and reads both.

The guard is the compiler rather than the assertions, in the same way ADR-0003's
is. The reversal is to write `file` where the test writes `file_owned`, and it
does not build:

```
error[E0502]: cannot borrow `sources` as mutable because it is also borrowed as immutable
```

So a change that puts the borrow back cannot pass, whether or not anyone
remembers this record.

### Consequences

* Good, because the phase order is the lexer's choice rather than something the
  source map imposes on it, and `#include` has somewhere to go.
* Good, because it copies nothing. The text of a header is stored once and read
  through a handle.
* Good, because it cost six lines and no call site changed, which is what makes
  it worth doing before rather than after the lexer.
* Good, because `Arc` leaves the door open for a parallel front end without
  committing to one.
* Bad, because there are now two accessors for one thing, and a reader has to
  learn which to reach for. The doc comments say, and `file` is the default.
* Bad, because `file_owned` has no caller outside its test until the lexer
  lands. It is an interface built for a use that is one phase away, which is a
  real cost even when the shape is not in doubt.
* Bad, because an `Arc` per file is a pointer chase that `Vec<SourceFile>` did
  not have, on a structure the whole compiler reads. Files are read through
  `contents()` and scanned as one `&str`, so the indirection is paid once per
  file rather than per byte.
* What would reverse this: an arena, if the map ever needs to hand out borrows
  that outlive `&self` for reasons an `Arc` cannot serve. Nothing in the roadmap
  needs that, and it would be a larger change to `FileId` than to `SourceMap`.

## Pros and Cons of the Options

### An `Arc` per file

* Good, because the handle is independent of the map, which is the property the
  lexer actually needs.
* Good, because it changes no existing call site and no existing test.
* Bad, because it adds an accessor whose first real caller does not exist yet.

### The lexer copies the text it is scanning

* Good, because `SourceMap` does not change at all.
* Bad, because it copies the source being compiled, once per translation unit
  and once per header, to work around a borrow rather than for any reason of its
  own.
* Bad, because the copy and the map then disagree about nothing in particular
  until someone edits one, and spans are offsets into whichever the reader
  assumed.

### An arena or interner handing out indices

* Good, because nothing borrows anything, which settles the question for every
  caller at once.
* Bad, because reading a file then goes through the arena on every access, so
  the lexer's inner loop pays for a lookup that a `&str` gives it for free.
* Bad, because it is a much larger change to the substrate the rest of the
  compiler is built on, made before there is a caller to shape it.

### A preprocessor pass that flattens each unit first

* Good, because the lexer would then see one buffer and never load anything.
* Bad, because it moves the conflict rather than removing it: the preprocessor
  reads `a.c` and loads `a.h`, which is the same borrow against the same map.
* Bad, because dodging it by reading headers with `fs::read_to_string` and not
  adding them to the map means a header has no `FileId`, so no span can point
  inside one and no diagnostic can quote a header line. For this project that is
  not a trade, it is a missing feature.

## More Information

* `crates/safec/src/source.rs`: `SourceMap`, `SourceMap::file`,
  `SourceMap::file_owned`.
* [ADR-0003](0003-pass-the-source-map-to-each-render-call.md): the same conflict
  one layer out, and the record that named this one without fixing it.
* [`docs/frontend.md`](../frontend.md): stage 3 adds the preprocessor, which is
  where `#include` arrives.
* [`docs/roadmap.md`](../roadmap.md): Phase 1 adds the lexer, Phase 8 the thread
  safety analysis.
