---
status: "accepted"
date: 2026-09-06
decision-makers: itsakeyfut
---

# Pass the source map to each render call rather than holding it

## Context and Problem Statement

`Renderer<'a>` held `&'a SourceMap` for its whole life, and sized the cache of
`ariadne` line indexes once, at construction, from `sources.len()`.

Two defects hid behind each other. Holding the map immutably means nothing can
add a file while a renderer is alive, which is a compile error rather than a
subtle one: a lexer that meets an `#include` has to call `SourceMap::load`, and
it cannot while the renderer borrows the map. And because no file could be
added, the cache being sized once was never wrong, so relaxing the borrow on its
own would have reported every file added afterwards as one the renderer does not
have.

Nothing outside the tests calls the renderer yet, so this is the cheapest moment
to settle how it relates to the map. Leaving it until a driver exists means the
driver gets written around the phase order the borrow imposes, and unwinding
that later touches the preprocessor, the driver and the renderer together.

## Decision Drivers

* The renderer must not constrain when files are added. `#include` is not an
  exotic case; it is how C works, and diagnostics are reported while parsing
  rather than only after it.
* A latent defect that is currently unreachable is worse than a visible one,
  because it becomes reachable the moment someone fixes the thing that was
  hiding it, and nothing fails until then.
* Copying every file's text into the cache would escape the borrow, but a
  compiler should not copy the source it is compiling to report on it.
* The cache exists to avoid rebuilding a line index for each diagnostic in a
  batch. It does not need to outlive the batch.

## Considered Options

* **Take the map per call, and build the cache per call.**
* **Keep a cache in the renderer that owns its text** (`Source<String>`), so the
  renderer holds no borrow but copies each file it renders against.
* **Keep the borrow and only fix the cache**, growing it lazily in `fetch`.

## Decision Outcome

Chosen option: **take the map per call, and build the cache per call**.

```rust
pub struct Renderer { config: Config, color: bool }

impl Renderer {
    pub fn new(color: ColorMode) -> Self;
    pub fn render(&self, sources: &SourceMap, d: &Diagnostic, out: &mut impl Write) -> io::Result<()>;
    pub fn render_all(&self, sources: &SourceMap, sink: &DiagnosticSink, out: &mut impl Write) -> io::Result<()>;
}
```

`render_all` builds one cache for the whole batch, which is the path a driver
should take. `render` builds one for the single diagnostic it writes.

The lifetime parameter is gone, `&mut self` becomes `&self`, and no file text is
copied: the cache still stores `Source<&str>` borrowed from the map, because it
now lives no longer than the borrow does.

Sizing the cache at construction is sound again, and for a reason rather than by
accident: the cache borrows the map for as long as it exists, and it exists for
one call, so no file can be added underneath it. That reason is written where
the sizing happens, along with what a cache that outlived the borrow would have
to do instead.

### Confirmation

`a_renderer_does_not_hold_the_source_map` in
`crates/safec/src/diagnostics/render.rs` renders a diagnostic, adds a file to
the map while the renderer is still alive, and renders a diagnostic against the
new file.

It guards both halves at once, in two different ways:

* The borrow: the test does not compile against a renderer that holds the map,
  because `add_virtual` needs `&mut SourceMap`. A reversal is caught by the
  compiler rather than by an assertion.
* The cache: sizing it once, as it was sized before, makes the assertion fail.
  Verified by replacing the sizing with a fixed length of one, which fails this
  test and nothing else.

### Consequences

* Good, because a driver can report diagnostics while it is still loading
  files, so the phase order is the driver's choice rather than something the
  renderer imposed on it.
* Good, because the latent cache defect is not merely fixed but unreachable: the
  cache cannot outlive the borrow it was sized against.
* Good, because `Renderer` has no lifetime parameter, so it can be held in a
  struct or passed around without infecting everything with `'a`.
* Bad, because rendering a batch by calling `render` in a loop rebuilds a file's
  line index for every diagnostic. `render_all` exists for that and its doc
  comment says so, but nothing enforces the choice.
* What would reverse this: a renderer that has to keep state across calls which
  is expensive to rebuild and cannot borrow the map. Nothing in the roadmap
  needs that; the line index is the only such state and it is cheap.

## Pros and Cons of the Options

### Take the map per call, and build the cache per call

* Good, because it removes the borrow and the latent cache defect together,
  rather than fixing one and leaving the other armed.
* Good, because it copies nothing.
* Good, because the cache's lifetime is exactly the batch it serves, which is
  also the only span over which reusing it is worth anything.
* Bad, because the map appears in every call signature, which is noise at the
  call site compared to a renderer that already knows it.

### A renderer that owns its cached text

* Good, because it also removes the borrow, and the cache survives across calls.
* Bad, because it copies the text of every file it renders against, to buy
  reuse across batches that a compiler does not do: diagnostics are rendered
  once, at the end, or streamed as they are found.
* Bad, because the cache then has to be keyed and grown by hand, which is the
  defect this record is partly about.

### Keep the borrow and only grow the cache lazily

* Good, because it is two lines.
* Bad, because it fixes the defect that cannot currently be hit and leaves the
  one that will stop the preprocessor.
* Bad, because it would leave the code looking correct while the actual
  constraint, the borrow, is undocumented and only discovered when a lexer
  cannot compile.

## More Information

* `crates/safec/src/diagnostics/render.rs`: `Renderer`, `SourceMapCache`.
* [`docs/frontend.md`](../frontend.md): stage 3 adds the preprocessor, which is
  where `#include` arrives.
* [ADR-0001](0001-promote-unproven-results-in-the-sink.md): the review that
  produced this record also produced that one.
