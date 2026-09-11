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
//! vertical tab; the source map breaks only on `\n`. And a file ending in `\n`
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

use crate::diagnostics::{Certainty, Diagnostic, DiagnosticSink, Label, Severity};
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
        write_notes(&notes, out)
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
            // The sink has already promoted it, and which flag asked for that
            // is not knowable here: `--safety strict` sets the same policy as
            // `--deny-unknown`, and `Policy` carries the decision rather than
            // its origin. Naming one of them would be a guess, and the wrong
            // guess tells the user a flag they never gave is to blame.
            "this could not be proven, and unproven results are errors in this compilation"
                .to_owned()
        } else {
            // Still a warning, so this is advice rather than an explanation,
            // and the flag that would escalate it can be named.
            "this could not be proven; `--deny-unknown` makes it an error".to_owned()
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
    write_notes(notes, out)
}

fn write_notes(notes: &[String], out: &mut impl io::Write) -> io::Result<()> {
    for note in notes {
        writeln!(out, "  = note: {}", shown(note))?;
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
mod tests {
    use safec_ir::analysis::Conclusion;

    /// What a check that could not prove anything reports, for a test that is
    /// about what happens to one rather than about how it is built.
    ///
    /// Through `concluded` rather than beside it, so that these tests exercise
    /// the path a check takes: an unproven diagnostic has one way to exist and
    /// this is it.
    fn unproven(message: &str) -> Diagnostic {
        Diagnostic::concluded(Conclusion::Unknown, message)
            .expect("an unknown conclusion is reported")
    }
    use super::*;
    use clap::Parser as _;

    use crate::diagnostics::Code;

    fn render(sources: &SourceMap, diagnostic: &Diagnostic) -> String {
        render_with(sources, diagnostic, ColorMode::Never)
    }

    fn render_with(sources: &SourceMap, diagnostic: &Diagnostic, color: ColorMode) -> String {
        let mut out = Vec::new();
        Renderer::new(color)
            .render(sources, diagnostic, &mut out)
            .expect("writing to a vector cannot fail");
        String::from_utf8(out).expect("the renderer writes text")
    }

    fn first_line(rendered: &str) -> String {
        rendered.lines().next().unwrap_or_default().to_owned()
    }

    fn moved_value() -> (SourceMap, Diagnostic) {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("main.c", "int *p = alloc();\nconsume(p);\n*p = 42;\n");
        let diagnostic = Diagnostic::error("use of moved value `p`")
            .with_code(Code::new("SC0601"))
            .with_label(Label::secondary(Span::new(file, 5, 6), "move occurs here"))
            .with_label(Label::primary(Span::new(file, 30, 32), "value used here"))
            .with_note("`p` was moved into `consume`");
        (sources, diagnostic)
    }

    #[test]
    fn a_diagnostic_without_a_label_still_reports_its_message() {
        let sources = SourceMap::new();
        let rendered = render(
            &sources,
            &Diagnostic::error("no input files")
                .with_code(Code::new("SC0001"))
                .with_note("pass at least one `.c` file"),
        );

        assert_eq!(
            rendered,
            "error[SC0001]: no input files\n  = note: pass at least one `.c` file\n"
        );
    }

    /// The common shape for a diagnostic about the invocation rather than the
    /// source.
    #[test]
    fn a_diagnostic_without_a_code_says_only_the_severity() {
        let sources = SourceMap::new();

        assert_eq!(
            render(&sources, &Diagnostic::error("no input files")),
            "error: no input files\n"
        );
    }

    /// The header is what an editor's problem matcher reads, so the two
    /// rendering paths have to spell it identically. Left to itself `ariadne`
    /// writes `Error:` and collapses a note and a help into `Advice:`.
    #[test]
    fn both_rendering_paths_spell_the_header_the_same_way() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("main.c", "int x;\n");

        for severity in [
            Severity::Error,
            Severity::Warning,
            Severity::Note,
            Severity::Help,
        ] {
            let bare = Diagnostic::new(severity, "m").with_code(Code::new("SC0001"));
            let anchored = bare
                .clone()
                .with_label(Label::primary(Span::new(file, 0, 3), "here"));

            let expected = format!("{}[SC0001]: m", severity.as_str());
            assert_eq!(first_line(&render(&sources, &bare)), expected);
            assert_eq!(first_line(&render(&sources, &anchored)), expected);
        }
    }

    /// The check that byte offsets are read as byte offsets. `ariadne` reads a
    /// span as character offsets by default, so without `IndexType::Byte` the
    /// span below lands a line or more past where it belongs.
    #[test]
    fn a_span_after_non_ascii_text_points_at_the_right_line() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("main.c", "/* 日本語日本語日本語 */\nint x;\nint y;\n");
        let start = sources.file(file).contents().find("int x").unwrap() as u32;

        let rendered = render(
            &sources,
            &Diagnostic::error("something about `x`")
                .with_label(Label::primary(Span::new(file, start, start + 5), "here")),
        );

        assert!(rendered.contains("int x"), "{rendered}");
        assert!(!rendered.contains("int y"), "{rendered}");
    }

    #[test]
    fn a_rendered_diagnostic_names_its_file_and_says_what_is_wrong() {
        let (sources, diagnostic) = moved_value();
        let rendered = render(&sources, &diagnostic);

        assert!(rendered.contains("error[SC0601]"), "{rendered}");
        assert!(rendered.contains("use of moved value `p`"), "{rendered}");
        assert!(rendered.contains("<main.c>"), "{rendered}");
        assert!(rendered.contains("move occurs here"), "{rendered}");
        assert!(rendered.contains("value used here"), "{rendered}");
        assert!(
            rendered.contains("  = note: `p` was moved into `consume`"),
            "{rendered}"
        );
    }

    /// `ariadne` opens a second source block whenever a label sits earlier than
    /// the one before it, and heads every block with the anchor's position, so
    /// handing over the attachment order produces a block whose header names a
    /// line it does not show. The order a check happens to attach its labels in
    /// must not change the picture.
    #[test]
    fn rendering_does_not_depend_on_the_order_labels_were_attached() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("main.c", "int *p = alloc();\nconsume(p);\n*p = 42;\n");
        let moved = Label::secondary(Span::new(file, 5, 6), "move occurs here");
        let used = Label::primary(Span::new(file, 30, 32), "value used here");

        let source_order = Diagnostic::error("m")
            .with_label(moved.clone())
            .with_label(used.clone());
        let primary_first = Diagnostic::error("m").with_label(used).with_label(moved);

        let rendered = render(&sources, &source_order);
        assert_eq!(rendered, render(&sources, &primary_first));
        // One source block, so exactly one header naming the file.
        assert_eq!(rendered.matches("<main.c>").count(), 1, "{rendered}");
    }

    /// `ariadne` panics on an end one past the file and drops the label without
    /// a word on an end far past it. The layer whose job is to report problems
    /// has to be total, so the span is moved and the move is disclosed.
    #[test]
    fn a_span_past_the_end_of_its_file_is_moved_and_noted() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("main.c", "int x;\n");

        for end in [8, 600] {
            let rendered = render(
                &sources,
                &Diagnostic::error("boom")
                    .with_label(Label::primary(Span::new(file, 0, end), "here")),
            );

            assert!(rendered.contains("int x"), "{rendered}");
            assert!(rendered.contains("here"), "{rendered}");
            assert!(
                rendered.contains("reaches past the end of <main.c> (7 bytes)"),
                "{rendered}"
            );
        }
    }

    #[test]
    fn a_span_inside_a_character_is_moved_and_noted() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("main.c", "/* 日本 */\n");

        let rendered = render(
            &sources,
            &Diagnostic::error("boom").with_label(Label::primary(Span::new(file, 3, 4), "here")),
        );

        assert!(rendered.contains("ends inside a character"), "{rendered}");
        assert!(rendered.contains("here"), "{rendered}");
    }

    /// The note names both offsets, so a reader can check which end it means.
    /// Blaming the end when the start was the one that split invites them to
    /// count the bytes and find the compiler wrong about its own diagnostic.
    /// In `/* 日本 */`, byte 4 splits the first character and byte 9 is the
    /// space, a real boundary.
    #[test]
    fn a_span_that_starts_inside_a_character_says_so_rather_than_blaming_the_end() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("main.c", "/* 日本 */\n");

        let rendered = render(
            &sources,
            &Diagnostic::error("boom").with_label(Label::primary(Span::new(file, 4, 9), "here")),
        );

        assert!(rendered.contains("starts inside a character"), "{rendered}");
        assert!(!rendered.contains("ends inside a character"), "{rendered}");
    }

    /// Byte 4 splits the first character and byte 7 splits the second, so both
    /// ends moved and the note has to say both.
    #[test]
    fn a_span_split_at_both_ends_says_both() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("main.c", "/* 日本 */\n");

        let rendered = render(
            &sources,
            &Diagnostic::error("boom").with_label(Label::primary(Span::new(file, 4, 7), "here")),
        );

        assert!(
            rendered.contains("starts and ends inside a character"),
            "{rendered}"
        );
    }

    /// The two reasons are independent, so a span can have both. Reporting only
    /// the one that reached past the end hides that the caret also moved.
    #[test]
    fn a_span_that_is_both_past_the_end_and_split_says_both() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("main.c", "/* 日本 */\n");

        let rendered = render(
            &sources,
            &Diagnostic::error("boom").with_label(Label::primary(Span::new(file, 4, 600), "here")),
        );

        assert!(rendered.contains("reaches past the end of"), "{rendered}");
        assert!(rendered.contains("starts inside a character"), "{rendered}");
    }

    /// A file name, and later an identifier out of the source, is content. A
    /// terminal obeys an escape sequence in it, so a name can colour the rest
    /// of the report or clear the line above it. Neither rendering path may
    /// pass one through, and the colour mode does not get a say: `--color
    /// always` means the renderer adds colour, not that content may.
    #[test]
    fn content_never_reaches_the_terminal_as_an_instruction() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("main.c", "int x;\n");

        let bare = Diagnostic::error("cannot read `evil\u{1b}[31m.c`");
        let anchored = bare
            .clone()
            .with_label(Label::primary(Span::new(file, 0, 3), "here\u{1b}[32m"))
            .with_note("note\u{1b}[33m");

        for mode in [ColorMode::Never, ColorMode::Always] {
            for diagnostic in [&bare, &anchored] {
                let rendered = render_with(&sources, diagnostic, mode);

                assert!(!rendered.contains("evil\u{1b}"), "{mode:?}: {rendered:?}");
                assert!(rendered.contains("evil\\u{1b}"), "{mode:?}: {rendered:?}");
            }
        }
    }

    /// A file's name reaches the terminal on the header line too.
    ///
    /// `content_never_reaches_the_terminal_as_an_instruction` puts the escape
    /// in a message, a label and a note, which are the three things this module
    /// writes. It never gives the source map a hostile *name*, so it never
    /// exercises the fourth: `ariadne` asks the cache what to call a file and
    /// prints the answer in `╭─[ name:line:col ]`, the first structural line of
    /// every anchored report.
    ///
    /// A name is not this compiler's text. It comes from a command line today
    /// and from a `#include` later, and RK-002 records what a `.c` file did to
    /// somebody's terminal when its bytes were echoed verbatim.
    ///
    /// Mutation: drop the `shown` from `SourceMapCache::display`. The escape
    /// reaches the header under every colour mode and this fails; before it was
    /// written, nothing in either crate did.
    #[test]
    fn a_file_name_is_escaped_on_the_line_that_locates_a_diagnostic() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("evil\u{1b}[31m.c", "int x;\n");

        let anchored =
            Diagnostic::error("boom").with_label(Label::primary(Span::new(file, 0, 3), "here"));

        for mode in [ColorMode::Never, ColorMode::Always] {
            let rendered = render_with(&sources, &anchored, mode);

            assert!(rendered.contains("evil\\u{1b}"), "{mode:?}: {rendered:?}");
            assert!(!rendered.contains("evil\u{1b}"), "{mode:?}: {rendered:?}");
        }
    }

    /// The other half of `content_never_reaches_the_terminal_as_an_instruction`,
    /// and it needs its own test because the text of a file is echoed by
    /// `ariadne` rather than written by this module. Nothing reached this path
    /// until a diagnostic had a label to point with, so a `.c` file containing
    /// `ESC [ 2 J` could clear the terminal of anyone who compiled it.
    #[test]
    fn source_text_is_echoed_without_the_control_characters_in_it() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("main.c", "int x = 1; /* \u{1b}[2J */\n");
        let diagnostic =
            Diagnostic::error("boom").with_label(Label::primary(Span::new(file, 4, 5), "here"));

        for mode in [ColorMode::Never, ColorMode::Always] {
            let rendered = render_with(&sources, &diagnostic, mode);

            assert!(!rendered.contains("\u{1b}[2J"), "{mode:?}: {rendered:?}");
        }

        // What replaced it is only contiguous without colour: `ariadne` colours
        // a source line one character at a time, so an escape of its own sits
        // between every pair of them.
        let plain = render_with(&sources, &diagnostic, ColorMode::Never);
        assert!(plain.contains("?[2J"), "{plain:?}");
    }

    /// One byte out for each byte in. A label span is a byte offset into the
    /// text `ariadne` is given, so a replacement of a different length would
    /// move every caret after it. Rendering against a file that already has the
    /// replacement at that byte is the same picture.
    #[test]
    fn replacing_a_control_character_does_not_move_the_caret() {
        let mut dirty = SourceMap::new();
        let dirty_file = dirty.add_virtual("main.c", "/* \u{1b} */ int y;\n");
        let mut clean = SourceMap::new();
        let clean_file = clean.add_virtual("main.c", "/* ? */ int y;\n");

        let at = |file| {
            Diagnostic::error("boom").with_label(Label::primary(Span::new(file, 13, 14), "here"))
        };

        assert_eq!(
            render(&dirty, &at(dirty_file)),
            render(&clean, &at(clean_file))
        );
    }

    /// `\r` is a line separator to `ariadne`, which breaks on it and never
    /// emits it, so it does not reach a terminal and must not be replaced.
    ///
    /// Two things would break if it were. A lone `\r` would stop separating,
    /// putting both statements onto one line, and every file written on Windows
    /// would carry a stray replacement at the end of every line.
    #[test]
    fn a_carriage_return_still_separates_lines() {
        let mut sources = SourceMap::new();
        let lone = sources.add_virtual("lone.c", "int a;\rint b;\n");
        let crlf = sources.add_virtual("crlf.c", "int a;\r\nint b;\r\n");

        let at = |file| {
            Diagnostic::error("boom").with_label(Label::primary(Span::new(file, 10, 11), "here"))
        };

        let rendered = render(&sources, &at(lone));
        assert!(rendered.contains("<lone.c>:2:"), "{rendered:?}");
        assert!(!rendered.contains(REPLACEMENT), "{rendered:?}");

        let rendered = render(&sources, &at(crlf));
        assert!(!rendered.contains(REPLACEMENT), "{rendered:?}");
    }

    /// A newline is laid out rather than acted on, and a message that runs to
    /// two lines is ordinary, so it is left as it was written.
    #[test]
    fn a_newline_in_a_message_is_left_alone() {
        let sources = SourceMap::new();

        let rendered = render(&sources, &Diagnostic::error("first\nsecond"));

        assert!(rendered.contains("first\nsecond"), "{rendered:?}");
    }

    /// A span built against a different source map. The renderer cannot quote
    /// what it does not have, but it must not drop the label in silence.
    #[test]
    fn a_label_naming_a_file_this_renderer_does_not_have_is_noted() {
        let mut sources = SourceMap::new();
        sources.add_virtual("main.c", "int x;\n");
        // Minted by another map, which is the hazard `FileId`'s doc describes
        // and, since ADR-0011, the only way a caller outside `safec_ir` can
        // build one. Its index is past anything this renderer holds.
        let mut elsewhere = SourceMap::new();
        elsewhere.add_virtual("a.c", "");
        let foreign = elsewhere.add_virtual("b.c", "");

        let rendered = render(
            &sources,
            &Diagnostic::error("boom").with_label(Label::primary(Span::new(foreign, 0, 3), "here")),
        );

        assert_eq!(
            rendered,
            "error: boom\n  = note: `here` points into a file this renderer does not have (index 1)\n"
        );
    }

    #[test]
    fn colour_is_emitted_only_when_it_was_asked_for() {
        let (sources, diagnostic) = moved_value();

        let always = render_with(&sources, &diagnostic, ColorMode::Always);
        let never = render_with(&sources, &diagnostic, ColorMode::Never);

        assert!(!never.contains('\u{1b}'), "{never:?}");
        assert!(always.contains('\u{1b}'), "{always:?}");
        // A primary label is coloured by severity and a secondary one is not,
        // so the two must not come out the same.
        assert!(always.contains("\u{1b}[31m"), "{always:?}");
        assert!(always.contains("\u{1b}[34m"), "{always:?}");
    }

    #[test]
    fn every_label_and_note_reaches_the_output() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("main.c", "int *p = alloc();\nconsume(p);\n*p = 42;\n");

        let rendered = render(
            &sources,
            &Diagnostic::error("m")
                .with_label(Label::secondary(Span::new(file, 5, 6), "allocated here"))
                .with_label(Label::secondary(Span::new(file, 18, 25), "moved here"))
                .with_label(Label::primary(Span::new(file, 30, 32), "used here"))
                .with_note("first note")
                .with_note("second note"),
        );

        for expected in [
            "allocated here",
            "moved here",
            "used here",
            "  = note: first note",
            "  = note: second note",
        ] {
            assert!(
                rendered.contains(expected),
                "{expected:?} missing from {rendered}"
            );
        }
        assert!(
            rendered.find("first note") < rendered.find("second note"),
            "{rendered}"
        );
    }

    /// An unprovable result has to be distinguishable from an ordinary warning,
    /// or the reader cannot tell which warnings an annotation would remove.
    #[test]
    fn an_unproven_diagnostic_says_that_it_could_not_be_proven() {
        let sources = SourceMap::new();

        let unproven = render(&sources, &unproven("`p` may escape"));
        assert!(
            unproven
                .contains("  = note: this could not be proven; `--deny-unknown` makes it an error"),
            "{unproven}"
        );

        let proven = render(&sources, &Diagnostic::warning("unused variable `x`"));
        assert!(!proven.contains("could not be proven"), "{proven}");
    }

    /// Once the sink has promoted it the note explains why it is an error, and
    /// it cannot name the flag responsible: `--safety strict` sets the same
    /// policy as `--deny-unknown`, so naming either one would tell half the
    /// users that a flag they never gave is to blame.
    ///
    /// Driven from the command line rather than from a hand-built `Policy`, so
    /// that the whole path from an argument to a rendered note is pinned.
    #[test]
    fn a_promoted_diagnostic_does_not_name_a_flag_the_user_may_not_have_given() {
        for args in [
            &["safec", "--safety", "strict", "a.c"][..],
            &["safec", "--deny-unknown", "a.c"][..],
        ] {
            let options = crate::cli::Cli::try_parse_from(args)
                .unwrap()
                .into_options();
            let mut sink = DiagnosticSink::with_policy(crate::diagnostics::Policy::from(&options));
            sink.report(unproven("`p` may escape"));

            let rendered = render(&SourceMap::new(), &sink.diagnostics()[0]);

            assert!(rendered.starts_with("error: "), "{args:?}: {rendered}");
            assert!(
                rendered.contains(
                    "  = note: this could not be proven, and unproven results are errors in this compilation"
                ),
                "{args:?}: {rendered}"
            );
            assert!(
                !rendered.contains("--deny-unknown"),
                "{args:?} blamed a flag: {rendered}"
            );
        }
    }

    #[test]
    fn a_diagnostic_says_which_safety_check_it_came_from() {
        let sources = SourceMap::new();

        let rendered = render(
            &sources,
            &unproven("`p` may escape").with_safety_level(crate::safety::SafetyLevel::Lifetime),
        );

        assert!(
            rendered.contains("  = note: this check belongs to safety level 2"),
            "{rendered}"
        );
    }

    /// The reason the renderer takes the map per call instead of holding it. A
    /// lexer that meets an `#include` has to add a file while diagnostics are
    /// already being reported, and it cannot do that while something borrows
    /// the map. This does not compile against a renderer that keeps the borrow,
    /// and a renderer that sized its cache once would report the second file as
    /// one it does not have.
    #[test]
    fn a_renderer_does_not_hold_the_source_map() {
        let renderer = Renderer::new(ColorMode::Never);
        let mut sources = SourceMap::new();
        let main = sources.add_virtual("main.c", "int x;\n");

        let mut out = Vec::new();
        renderer
            .render(
                &sources,
                &Diagnostic::error("first")
                    .with_label(Label::primary(Span::new(main, 0, 3), "here")),
                &mut out,
            )
            .unwrap();

        // The renderer is still alive, and the map still grows.
        let header = sources.add_virtual("header.h", "int y;\n");
        renderer
            .render(
                &sources,
                &Diagnostic::error("second")
                    .with_label(Label::primary(Span::new(header, 0, 3), "there")),
                &mut out,
            )
            .unwrap();

        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("<main.c>"), "{rendered}");
        assert!(rendered.contains("<header.h>"), "{rendered}");
        assert!(rendered.contains("there"), "{rendered}");
        assert!(
            !rendered.contains("does not have"),
            "the second file was not found: {rendered}"
        );
    }

    #[test]
    fn render_all_writes_every_diagnostic_in_the_order_they_were_reported() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("main.c", "int x;\n");
        let mut sink = DiagnosticSink::new();
        sink.report(
            Diagnostic::error("first").with_label(Label::primary(Span::new(file, 0, 3), "a")),
        );
        sink.report(Diagnostic::warning("second"));
        sink.report(
            Diagnostic::error("third").with_label(Label::primary(Span::new(file, 4, 5), "b")),
        );

        let mut out = Vec::new();
        Renderer::new(ColorMode::Never)
            .render_all(&sources, &sink, &mut out)
            .unwrap();
        let rendered = String::from_utf8(out).unwrap();

        let first = rendered.find("first").expect("first is missing");
        let second = rendered.find("second").expect("second is missing");
        let third = rendered.find("third").expect("third is missing");
        assert!(first < second && second < third, "{rendered}");
    }

    /// The two paths are one interface, so the header has to match in colour as
    /// well as in words. `ariadne` colours `error[SC0001]:` including the colon,
    /// and the hand-written path has to put the escapes in the same places.
    #[test]
    fn both_rendering_paths_colour_the_header_the_same_way() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("main.c", "int x;\n");

        for severity in [
            Severity::Error,
            Severity::Warning,
            Severity::Note,
            Severity::Help,
        ] {
            let bare = Diagnostic::new(severity, "m").with_code(Code::new("SC0001"));
            let anchored = bare
                .clone()
                .with_label(Label::primary(Span::new(file, 0, 3), "here"));

            let bare = render_with(&sources, &bare, ColorMode::Always);
            let anchored = render_with(&sources, &anchored, ColorMode::Always);

            assert!(first_line(&bare).contains('\u{1b}'), "{bare:?}");
            assert_eq!(first_line(&bare), first_line(&anchored), "{severity:?}");
        }
    }
}
