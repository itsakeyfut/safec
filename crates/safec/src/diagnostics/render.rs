//! Rendering diagnostics for a terminal.
//!
//! The only module that knows a rendering library exists. Everything upstream
//! builds a [`Diagnostic`] and hands it over, which is what keeps a
//! `--error-format=json` mode, or a different rendering library, a change
//! confined to this file.
//!
//! Two divergences from [`SourceFile`] are worth
//! knowing about, because both make a rendered gutter disagree with a
//! [`LineCol`](safec_ir::source::LineCol).
//!
//! `ariadne` breaks lines on seven separators, among them a lone `\r` and a
//! vertical tab; the source map breaks only on `\n`. `echoed` replaces every
//! one of them but `\r` before `ariadne` sees it, the vertical tab as a
//! control character and U+2028 and U+2029 as characters that break a line
//! (`is_obeyed`), so a lone `\r` is the separator still counted differently. And a file ending in `\n`
//! has a final empty line for the source map, deliberately, so that it is
//! numbered the way an editor numbers it. `ariadne` has no such line, so an
//! offset at end of file is numbered differently by the two. That last one is
//! the ordinary case for `error: unexpected end of file`, not an exotic one.
//!
//! Reconciling them would mean either teaching the source map about separators
//! it does not otherwise care about or giving up `ariadne`'s renderer.

use std::borrow::Cow;
use std::fmt;
use std::io;
use std::ops::Range;

use ariadne::{Color, Config, Fmt, IndexType, Label as AriadneLabel, Report, ReportKind, Source};

use crate::diagnostics::{Certainty, Diagnostic, DiagnosticSink, Label, Remedy, Severity};
use crate::options::ColorMode;
use safec_ir::print::{is_obeyed, shown};
use safec_ir::source::{FileId, SourceFile, SourceMap, Span};

/// How `ariadne` names a region: a file, and a byte range within it.
///
/// Not an id. In `ariadne`'s vocabulary this whole pair is the span, and the
/// [`FileId`] alone is its `SourceId`.
type AriadneSpan = (FileId, Range<usize>);

/// The word a diagnostic is spelled with, code included.
///
/// One function owns this so that both rendering paths agree: `ariadne` writes
/// its own header, and the header-only path writes one by hand.
fn header(diagnostic: &Diagnostic) -> String {
    match diagnostic.code() {
        Some(code) => format!("{}[{code}]", diagnostic.severity()),
        None => diagnostic.severity().to_string(),
    }
}

fn severity_color(severity: Severity) -> Color {
    match severity {
        Severity::Error => Color::Red,
        Severity::Warning => Color::Yellow,
        Severity::Note | Severity::Help => Color::Cyan,
    }
}

/// A label span, made safe to hand to `ariadne`.
///
/// `ariadne` slices the file text with these offsets directly. An end one past
/// the file, or either end inside a character, panics inside it; an end far
/// past the file makes it drop the label silently. Neither is acceptable in the
/// one layer whose job is to report problems, so a span is clamped to the file
/// and snapped to character boundaries first, and the caller is told whether
/// that had to happen.
fn clamp(file: &SourceFile, span: Span) -> (Range<usize>, Adjusted) {
    let text = file.contents();
    let mut start = (span.start() as usize).min(text.len());
    let mut end = (span.end() as usize).min(text.len());
    let mut adjusted = Adjusted {
        past_the_end: start != span.start() as usize || end != span.end() as usize,
        start_split: false,
        end_split: false,
    };

    // `text.len()` is always a boundary, so neither loop can run off the end.
    while !text.is_char_boundary(start) {
        start -= 1;
        adjusted.start_split = true;
    }
    while !text.is_char_boundary(end) {
        end += 1;
        adjusted.end_split = true;
    }

    (start..end.max(start), adjusted)
}

/// What had to be done to a label's span before it could be rendered.
///
/// Three flags rather than one reason, because the reasons combine: a span can
/// reach past the end of the file and split a character, and either endpoint
/// can be the one that split. A single value has to pick one of them to report,
/// and the note it produces names both offsets, so a reader who counts them
/// finds the claim is about the other end.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Adjusted {
    /// An endpoint reached past the end of the file and was pulled back.
    past_the_end: bool,
    /// The start fell inside a character and was moved down to its boundary.
    start_split: bool,
    /// The end fell inside a character and was moved up to its boundary.
    end_split: bool,
}

impl Adjusted {
    /// Whether anything had to move at all.
    fn any(self) -> bool {
        self.past_the_end || self.start_split || self.end_split
    }
}

/// Renders diagnostics against the source they point into.
///
/// Holds no borrow of a [`SourceMap`]. The map is passed to each call instead,
/// so a driver can keep loading files while a renderer is alive, which is what
/// `#include` will need: a lexer that hits one has to add a file to the map,
/// and it cannot do that while something holds the map immutably.
///
/// The line index `ariadne` insists on building for each file lives for one
/// call. [`Renderer::render_all`] builds it once for a whole batch, which is
/// the path a driver should take; [`Renderer::render`] builds it for the single
/// diagnostic it writes.
#[derive(Debug)]
pub struct Renderer {
    config: Config,
    color: bool,
}

impl Renderer {
    /// A renderer that colours its output as `color` asks.
    ///
    /// `color` is resolved here rather than carried further: `Auto` asks
    /// whether the stream is a terminal, and nothing downstream should have to
    /// ask again.
    pub fn new(color: ColorMode) -> Self {
        let color = match color {
            ColorMode::Always => true,
            ColorMode::Never => false,
            ColorMode::Auto => io::IsTerminal::is_terminal(&io::stderr()),
        };

        Self {
            config: Config::default()
                .with_color(color)
                // Spans are byte offsets. `ariadne` reads them as character
                // offsets unless told otherwise. That would misplace every
                // caret on a line holding non-ASCII text, exactly the case the
                // source map takes care to get right.
                .with_index_type(IndexType::Byte),
            color,
        }
    }

    /// Write one diagnostic to `out`.
    ///
    /// A span that lies outside its file, or that splits a character, is
    /// clamped rather than dropped or panicked on, and the diagnostic gains a
    /// note saying so. A span naming a file this renderer does not have is
    /// reported the same way. Nothing a caller can build makes a label vanish
    /// without a trace in `out`.
    ///
    /// Rendering a batch by calling this in a loop rebuilds `ariadne`'s line
    /// index for every diagnostic. [`Renderer::render_all`] builds it once.
    pub fn render(
        &self,
        sources: &SourceMap,
        diagnostic: &Diagnostic,
        out: &mut impl io::Write,
    ) -> io::Result<()> {
        self.write_one(&mut SourceMapCache::new(sources), diagnostic, out)
    }

    /// Write every diagnostic in a sink, in the order they were reported.
    ///
    /// One line index per file for the whole batch.
    pub fn render_all(
        &self,
        sources: &SourceMap,
        sink: &DiagnosticSink,
        out: &mut impl io::Write,
    ) -> io::Result<()> {
        let mut cache = SourceMapCache::new(sources);
        for diagnostic in sink.diagnostics() {
            self.write_one(&mut cache, diagnostic, out)?;
        }
        Ok(())
    }

    fn write_one(
        &self,
        cache: &mut SourceMapCache<'_>,
        diagnostic: &Diagnostic,
        out: &mut impl io::Write,
    ) -> io::Result<()> {
        let mut notes: Vec<String> = diagnostic.notes().to_vec();
        notes.extend(safety_level_note(diagnostic));
        notes.extend(certainty_note(diagnostic));
        let mut labels: Vec<(&Label, AriadneSpan)> = Vec::new();

        for label in diagnostic.labels() {
            match cache.file(label.span().file()) {
                Some(file) => {
                    let (range, adjusted) = clamp(file, label.span());
                    if let Some(note) = adjustment_note(label, file, adjusted) {
                        notes.push(note);
                    }
                    labels.push((label, (label.span().file(), range)));
                }
                None => notes.push(no_such_file(label)),
            }
        }

        // `ariadne` starts a new source block whenever a label sits earlier
        // than the one before it, and headers every block with the *anchor's*
        // position. Handing over the attachment order therefore produces a
        // second block whose header names a line it does not show, purely
        // because of the order a check happened to call `with_label`. Sorting
        // by position first makes the rendering independent of that order; the
        // narrative lives in the label messages, not in their sequence.
        labels.sort_by(|(_, left), (_, right)| {
            left.0.cmp(&right.0).then(left.1.start.cmp(&right.1.start))
        });

        // The anchor decides which position the header names. Prefer the
        // primary label, and fall back to the earliest usable one.
        let anchor = diagnostic
            .primary_label()
            .and_then(|primary| {
                labels
                    .iter()
                    .find(|(label, _)| std::ptr::eq(*label, primary))
            })
            .or_else(|| labels.first())
            .map(|(_, span)| span.clone());

        let Some(anchor) = anchor else {
            return render_header_only(diagnostic, &notes, self.color, out);
        };

        // `ariadne`'s built-in kinds spell the header `Error:` and collapse a
        // note and a help into `Advice:`, and it puts the code in front of the
        // word. A custom kind puts the spelling back under `Severity`, so both
        // rendering paths agree and a help stays distinguishable from a note.
        let header = header(diagnostic);
        let mut report = Report::build(
            ReportKind::Custom(&header, severity_color(diagnostic.severity())),
            anchor,
        )
        .with_config(self.config)
        .with_message(shown(diagnostic.message()));

        for (label, span) in &labels {
            let color = if label.is_primary() {
                severity_color(diagnostic.severity())
            } else {
                Color::Blue
            };
            report = report.with_label(
                AriadneLabel::new(span.clone())
                    .with_message(shown(label.message()))
                    .with_color(color),
            );
        }

        // Notes are written after the block rather than handed to `ariadne`,
        // which renders them inside it as `Note:`. This keeps one spelling for
        // both paths.
        let mut rendered = Vec::new();
        report.finish().write(&mut *cache, &mut rendered)?;
        if !self.color {
            strip_header_escapes(&mut rendered);
        }
        out.write_all(&rendered)?;
        write_notes(&notes, out)?;
        write_remedies(diagnostic.remedies(), out)
    }
}

/// Which check spoke, for a diagnostic that came from a safety analysis.
fn safety_level_note(diagnostic: &Diagnostic) -> Option<String> {
    diagnostic
        .safety_level()
        .map(|level| format!("this check belongs to safety level {}", level.number()))
}

/// Says that a diagnostic is one an annotation could remove.
///
/// Without it an unprovable result is indistinguishable from an ordinary
/// warning, and the reader has no way to tell which warnings are the ones the
/// migration in `docs/safety-model.md` is about.
fn certainty_note(diagnostic: &Diagnostic) -> Option<String> {
    (diagnostic.certainty() == Certainty::Unproven).then(|| {
        if diagnostic.severity().is_error() {
            // The sink has already promoted it, and what asked for that is not
            // knowable here: it is the default wherever a check runs, and
            // `Policy` carries the decision rather than its origin. Naming a
            // flag would be a guess, and the wrong guess tells the user a flag
            // they never gave is to blame.
            "this could not be proven, and unproven results are errors in this compilation"
                .to_owned()
        } else {
            // Still a warning, so this is advice rather than an explanation,
            // and here the origin *is* knowable: every producer of an unproven
            // conclusion is reached only by a run that asked to be checked, so
            // naming the flag is not a guess. Two of the three sit inside the
            // safety gate in `driver::lowered`, which a run below it never
            // reaches. The third is `driver::undelivered`, which answers nothing
            // unless a level was asked for that the run cannot deliver, and an
            // unasked level resolves to one it can. See ADR-0035.
            "this could not be proven; it is an error without `--allow-unknown`".to_owned()
        }
    })
}

/// Every reason the span moved, not the first one that applied.
///
/// The note names the offsets it is talking about, so a reader can check it.
/// That makes an incomplete reason worse than none: it invites the reader to
/// count bytes and conclude the compiler is confused about its own diagnostic.
fn adjustment_note(label: &Label, file: &SourceFile, adjusted: Adjusted) -> Option<String> {
    if !adjusted.any() {
        return None;
    }

    let mut reasons = Vec::new();
    if adjusted.past_the_end {
        reasons.push(format!(
            "reaches past the end of {} ({} bytes)",
            file.name(),
            file.len()
        ));
    }
    match (adjusted.start_split, adjusted.end_split) {
        (true, true) => reasons.push("starts and ends inside a character".to_owned()),
        (true, false) => reasons.push("starts inside a character".to_owned()),
        (false, true) => reasons.push("ends inside a character".to_owned()),
        (false, false) => {}
    }

    Some(format!(
        "the span {}..{} for `{}` {}, and was moved to fit",
        label.span().start(),
        label.span().end(),
        label.message(),
        reasons.join(" and ")
    ))
}

fn no_such_file(label: &Label) -> String {
    format!(
        "`{}` points into a file this renderer does not have (index {})",
        label.message(),
        label.span().file().index()
    )
}

/// The header and the notes, for a diagnostic with nothing to point at.
///
/// `ariadne` colours the whole of `error[SC0601]:`, colon included, and this
/// matches it byte for byte rather than merely word for word. The two paths are
/// one interface: a reader who asked for colour and got it on the diagnostics
/// that point at source, but not on the ones that do not, would reasonably read
/// the difference as meaning something.
fn render_header_only(
    diagnostic: &Diagnostic,
    notes: &[String],
    color: bool,
    out: &mut impl io::Write,
) -> io::Result<()> {
    let head = format!("{}:", header(diagnostic));
    let message = shown(diagnostic.message());
    if color {
        let head = head.fg(severity_color(diagnostic.severity()));
        writeln!(out, "{head} {message}")?;
    } else {
        writeln!(out, "{head} {message}")?;
    }
    write_notes(notes, out)?;
    write_remedies(diagnostic.remedies(), out)
}

fn write_notes(notes: &[String], out: &mut impl io::Write) -> io::Result<()> {
    for note in notes {
        writeln!(out, "  = note: {}", shown(note))?;
    }
    Ok(())
}

/// What to change, after what happened.
///
/// Written here rather than handed to `ariadne` for [`write_notes`]'s reason:
/// one spelling for both paths, and a remedy that only appeared on diagnostics
/// with a caret would be a different interface depending on whether the check
/// had a span. Both paths call this, which is two calls and two mutations
/// rather than one.
///
/// After the notes, because a note says what happened and a remedy says what to
/// do about it, and the second reads as the conclusion of the first.
///
/// Written through `shown`, though these are strings this compiler wrote
/// rather than the text of a source file, because every place that echoes
/// text has to answer for what it may one day hold.
fn write_remedies(remedies: &[Remedy], out: &mut impl io::Write) -> io::Result<()> {
    for remedy in remedies {
        writeln!(out, "  = help: {}", shown(remedy.message()))?;
    }
    Ok(())
}

/// A file's text, made safe to echo, with every byte offset preserved.
///
/// [`shown`] is the wrong tool here. It turns one character into several, and a
/// label span is a byte offset into this very text, which `ariadne` slices with
/// to find the line and place the caret. A substitution that changed a length
/// would move every caret after it. So each byte of a control character becomes
/// one byte, and the text stays the size the spans were measured against.
///
/// `\r` is left alone, unlike in a message. `ariadne` breaks lines on it and
/// never emits it, so it does not reach a terminal on this path, and replacing
/// it would stop it separating lines: a file with CRLF endings would render as
/// one long line.
///
/// Borrowed unless there is something to replace, so an ordinary file is still
/// not copied and ADR-0003's reason for storing borrowed text holds for every
/// input that is not trying something.
fn echoed(text: &str) -> Cow<'_, str> {
    let replaced = |ch: char| is_obeyed(ch) && ch != '\r';

    if !text.chars().any(replaced) {
        return Cow::Borrowed(text);
    }

    let mut safe = String::with_capacity(text.len());
    for ch in text.chars() {
        if replaced(ch) {
            // One byte out for each byte in, so every offset after this one is
            // still the offset the span was built from.
            for _ in 0..ch.len_utf8() {
                safe.push(REPLACEMENT);
            }
        } else {
            safe.push(ch);
        }
    }
    Cow::Owned(safe)
}

/// What a control character in a source file is shown as.
///
/// One byte, because [`echoed`] has to hand back text of the same size. That
/// rules out the characters that would say it better, U+FFFD and the Control
/// Pictures block among them.
const REPLACEMENT: char = '?';

/// Remove the colour `ariadne` insists on putting in a custom header.
///
/// `Config::with_color(false)` silences its built-in kinds but not a custom
/// one, which always emits its colour. Only the header line is touched, which
/// is enough because the other two sources of an escape are already dealt with:
/// [`shown`] handles the message on this line, and [`echoed`] handles the
/// source lines below it. Every escape left here is this renderer's own, which
/// is what lets this take the whole line rather than tell the two apart.
fn strip_header_escapes(rendered: &mut Vec<u8>) {
    let end = rendered
        .iter()
        .position(|&b| b == b'\n')
        .unwrap_or(rendered.len());
    let mut header = Vec::with_capacity(end);
    let mut rest = &rendered[..end];
    while let Some(start) = rest.iter().position(|&b| b == 0x1b) {
        header.extend_from_slice(&rest[..start]);
        match rest[start..].iter().position(|&b| b == b'm') {
            Some(finish) => rest = &rest[start + finish + 1..],
            None => {
                rest = &rest[start..];
                break;
            }
        }
    }
    header.extend_from_slice(rest);
    rendered.splice(..end, header);
}

/// A span naming a file this renderer has never heard of.
///
/// [`Renderer::render`] checks for this before handing anything to `ariadne`,
/// so this is a backstop rather than the reporting path.
struct UnknownFile(FileId);

impl fmt::Debug for UnknownFile {
    /// Written out rather than derived. A derived `Debug` does not count as a
    /// read of the field, so `dead_code` fires on it, and this says more than
    /// `UnknownFile(FileId(3))` would.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "no file with index {} in this source map",
            self.0.index()
        )
    }
}

/// Lets `ariadne` read a [`SourceMap`] by [`FileId`].
///
/// The trait hands back a borrowed `Source`, so the cache has to own one per
/// file. Storing `&str` rather than `String` keeps the file contents from being
/// copied; what is duplicated is the line index, which
/// [`SourceFile`] already has and `ariadne` insists
/// on building for itself. A compilation reports a handful of diagnostics, so
/// that is not worth designing around.
#[derive(Debug)]
struct SourceMapCache<'a> {
    sources: &'a SourceMap,
    /// Indexed by [`FileId::index`], filled on first use of each file.
    cached: Vec<Option<Source<Cow<'a, str>>>>,
}

impl<'a> SourceMapCache<'a> {
    /// A cache for one rendering call.
    ///
    /// Sizing `cached` once is sound because this borrows the map for as long
    /// as it lives, and it lives for a single call, so no file can be added
    /// underneath it. A cache that outlived the borrow would need to grow in
    /// [`ariadne::Cache::fetch`] instead, which is the one accessor that takes
    /// `&mut self`, and forgetting that would report every file added
    /// afterwards as one this renderer does not have.
    fn new(sources: &'a SourceMap) -> Self {
        Self {
            sources,
            cached: (0..sources.len()).map(|_| None).collect(),
        }
    }

    /// The file a span names, or `None` if this map does not have it.
    ///
    /// The result borrows the map rather than the cache, so a caller can hold
    /// it while the cache is borrowed mutably to render.
    fn file(&self, id: FileId) -> Option<&'a SourceFile> {
        if id.index() < self.cached.len() {
            Some(self.sources.file(id))
        } else {
            None
        }
    }
}

impl<'a> ariadne::Cache<FileId> for SourceMapCache<'a> {
    type Storage = Cow<'a, str>;

    fn fetch(&mut self, id: &FileId) -> Result<&Source<Self::Storage>, impl fmt::Debug> {
        // Copied out before `cached` is borrowed, so that the closure below
        // does not hold a second borrow of `self`.
        let sources = self.sources;
        let id = *id;

        match self.cached.get_mut(id.index()) {
            Some(slot) => {
                Ok(slot.get_or_insert_with(|| Source::from(echoed(sources.file(id).contents()))))
            }
            None => Err(UnknownFile(id)),
        }
    }

    fn display<'b>(&self, id: &'b FileId) -> Option<impl fmt::Display + 'b> {
        // Mirrors `fetch`: a handle from another source map is reported as a
        // file this renderer does not have, rather than panicking inside it.
        (id.index() < self.cached.len())
            .then(|| shown(&self.sources.file(*id).name().to_string()).into_owned())
    }
}

#[cfg(test)]
mod tests;
