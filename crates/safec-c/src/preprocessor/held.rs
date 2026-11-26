//! What one input's preprocessing holds, charged at the one door it comes in
//! by: a file added to the source map, and the tokens it is lexed into.
//!
//! The preprocessor holds [`Sources`] instead of the [`SourceMap`] it wraps,
//! which reads through [`Deref`] and has no `DerefMut`, so a way in that
//! skips the charge does not compile: `sources.expand(..)` on it fails
//! `E0596`. The bound was charged by hand at four call sites before, review
//! of #1337 found each could pass no length with the suite green, and an
//! inclusion was charged nowhere: 4,000 inclusions of a guarded 64 KB header
//! held 257 MB, and of a 512 KB one 1.8 GB past three minutes, with nothing
//! printed.
//!
//! Tokens are charged where a file added is lexed, not where they are handed
//! on, because every token the preprocessor holds was lexed from one, and
//! not every one is handed on: a `#if`'s expression, an argument expanded
//! before it is substituted, and a skipped group are read and dropped.
//! Charged only as they were handed on, forty uses of a mebibyte-long macro
//! in one `#if` held 2.2 GB with nothing printed. Found by review. A token
//! dropped is charged all the same, so an input is charged more than its
//! peak, never less.
//!
//! What is not charged: the input's own text and tokens, which the driver
//! reads before preprocessing and which grow with what was given, and the
//! scratch maps that lex a few bytes and are dropped at once.

use std::io;
use std::ops::Deref;
use std::path::Path;

use super::TOO_MANY_EXPANSIONS;
use crate::diagnostics::{Diagnostic, DiagnosticSink, Label};
use crate::lexer::{lex, lex_replacement};
use crate::token::Token;
use safec_ir::source::{FileId, Piece, SourceMap, Span};

/// How many bytes one input's preprocessing may hold in all, by the
/// estimate the costs below make of what it takes, from what was measured.
///
/// What a file of many uses costs is memory, so memory is what is bounded:
/// with a count of expansions starting again at each use, forty uses of a
/// tree just under [`super::MAX_EXPANSIONS`] held 818 MB and printed nothing
/// until it ended, and a count of expansions for the input, which this
/// replaced, refused a file of 66,000 lines of `s = F(s);` with `#define
/// F(x) (G + x)`, two expansions a line, at line 65,540. Found by review of
/// #1215. Past it the next addition is refused, once.
pub(super) const MAX_HELD_IN_ALL: usize = 256 << 20;

/// What one file added to the map costs beside its text and its tokens, in
/// bytes.
///
/// Measured, not reckoned, with the bound lifted: a release build read
/// `--emit tokens` of forty uses of `M15`, `#define M{n} (M{n-1} + M{n-1})`
/// down to `#define M0 1`, 2,621,400 expansions, at a peak working set of
/// 817 MB, and ten such uses at 209 MB: 311 bytes an expansion either way.
/// Less its text at [`TEXT_COST`] and its tokens at [`TOKEN_COST`], about
/// five bytes and four tokens an expansion, the end of each among them,
/// that is about 200.
pub(super) const INSTANCE_COST: usize = 200;

/// What one byte of a file's text costs in the map, at most, in bytes: the
/// byte, and the start of a line it can be.
///
/// A file's text is kept until the input ends, and so is where each of its
/// lines starts, four bytes a line, in a list that grows by doubling. A
/// header of comments or one under an include guard costs about the byte
/// alone: 400 inclusions of a 64 KB one, 26 MB of text, peaked at 32 MB.
/// One of nothing but line breaks costs the most: 400 inclusions of 65,536
/// of them peaked at 161 MB, 5.9 bytes a byte. Charged as a dense list's
/// tokens are, the 10.5 MB of headers `windows.h`, `stdio.h`, `stdlib.h`
/// and `string.h` are read from would take most of the bound.
pub(super) const TEXT_COST: usize = 6;

/// What one token lexed from a file added costs, at most, in bytes.
///
/// A token handed on is held until the input ends, in a list that grows by
/// doubling, beside the list it was lexed into. Measured with the bound
/// lifted: 400 inclusions of a header of 65,536 commas, 26 million tokens,
/// peaked at 535 MB, 19.2 bytes a token beside the text, where the same
/// length of comments peaked at 34 MB.
pub(super) const TOKEN_COST: usize = 20;

/// The source map, through which alone the preprocessor adds a file to it.
pub(super) struct Sources<'map> {
    map: &'map mut SourceMap,
    /// What has been charged, kept past [`MAX_HELD_IN_ALL`] once it was
    /// refused, which no charge that is admitted can leave it.
    held: usize,
}

/// A file to add to the map and lex, and what it is.
pub(super) enum Addition<'a> {
    /// The file at the path, `u64` bytes long by its metadata, charged
    /// before it is read, so a header too large is never read at all, and
    /// lexed as a file, its directives read.
    Include(&'a Path, u64),
    /// An expansion of the replacement list at the span, as written.
    Expand(Span),
    /// An expansion of the text, whose runs were copied from the pieces.
    Substitute(String, Vec<Piece>),
}

/// Why a file was not added.
#[derive(Debug)]
pub(super) enum Unadded {
    /// It would take what is held past [`MAX_HELD_IN_ALL`], reported, once.
    OverBound,
    /// It could not be read, not yet reported.
    Io(io::Error),
}

impl Deref for Sources<'_> {
    type Target = SourceMap;

    fn deref(&self) -> &SourceMap {
        self.map
    }
}

impl<'map> Sources<'map> {
    /// `map`, with `held` already charged.
    pub(super) fn new(map: &'map mut SourceMap, held: usize) -> Self {
        Self { map, held }
    }

    /// What has been charged.
    pub(super) fn held(&self) -> usize {
        self.held
    }

    /// Charge `addition`, add it, used at `at`, and lex it, charging its
    /// tokens; or refuse it where either would take what is held past the
    /// bound, reported once at `at`. An expansion is lexed where it is used,
    /// so a character no token can begin is reported at each use, which is
    /// where C makes it an error.
    pub(super) fn add(
        &mut self,
        addition: Addition<'_>,
        at: Span,
        diagnostics: &mut DiagnosticSink,
    ) -> Result<(FileId, Vec<Token>), Unadded> {
        let (length, label) = match &addition {
            Addition::Include(_, length) => (
                usize::try_from(*length).unwrap_or(usize::MAX),
                "included here",
            ),
            Addition::Expand(spelled) => (spelled.len() as usize, "expanded here"),
            Addition::Substitute(text, _) => (text.len(), "expanded here"),
        };
        let cost = length
            .saturating_mul(TEXT_COST)
            .saturating_add(INSTANCE_COST);
        if !self.charge(cost, Some((at, label)), diagnostics) {
            return Err(Unadded::OverBound);
        }
        let (file, tokens) = match addition {
            Addition::Include(path, _) => {
                let file = self.map.include(path, at).map_err(Unadded::Io)?;
                (file, lex(file, &self.map.file_owned(file), diagnostics))
            }
            Addition::Expand(spelled) => {
                let file = self.map.expand(spelled, at);
                let tokens = lex_replacement(file, &self.map.file_owned(file), diagnostics);
                (file, tokens)
            }
            Addition::Substitute(text, pieces) => {
                let file = self.map.substitute(text, pieces, at);
                let tokens = lex_replacement(file, &self.map.file_owned(file), diagnostics);
                (file, tokens)
            }
        };
        let cost = tokens.len().saturating_mul(TOKEN_COST);
        if !self.charge(cost, Some((at, label)), diagnostics) {
            return Err(Unadded::OverBound);
        }
        Ok((file, tokens))
    }

    /// Charge and add a file that is not on disk, named `name`, as
    /// [`SourceMap::add_virtual`] does, which its caller reads itself; a
    /// refusal is said at `used`, or of no place where it is used nowhere,
    /// as what a target predefines is not.
    pub(super) fn add_virtual(
        &mut self,
        name: &str,
        contents: &str,
        used: Option<(Span, &str)>,
        diagnostics: &mut DiagnosticSink,
    ) -> Result<FileId, Unadded> {
        let cost = contents
            .len()
            .saturating_mul(TEXT_COST)
            .saturating_add(INSTANCE_COST);
        if !self.charge(cost, used, diagnostics) {
            return Err(Unadded::OverBound);
        }
        Ok(self.map.add_virtual(name, contents))
    }

    /// Record the first and last thing of `file` the parser is handed, as
    /// [`SourceMap::set_extent`] does; it adds nothing to charge.
    pub(super) fn set_extent(&mut self, file: FileId, first: Span, last: Span) {
        self.map.set_extent(file, first, last);
    }

    /// Charge `cost` for what is at `at`, under its label, or refuse it,
    /// said once, where it would take what is held past the bound.
    fn charge(
        &mut self,
        cost: usize,
        at: Option<(Span, &str)>,
        diagnostics: &mut DiagnosticSink,
    ) -> bool {
        if self.held.saturating_add(cost) > MAX_HELD_IN_ALL {
            if self.held <= MAX_HELD_IN_ALL {
                self.held = MAX_HELD_IN_ALL + 1;
                // What this one would take it to, not what is held: one
                // large expansion can pass the bound from well under it.
                let mut report = Diagnostic::error("this input would take too much memory to read")
                    .with_code(TOO_MANY_EXPANSIONS)
                    .with_note(format!(
                        "what it holds would pass {} MiB, the most one input's may",
                        MAX_HELD_IN_ALL >> 20
                    ));
                if let Some((at, label)) = at {
                    report = report.with_label(Label::primary(at, label));
                }
                diagnostics.report(report);
            }
            return false;
        }
        self.held += cost;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each kind of file added is charged its text, at six bytes a byte,
    /// its tokens, at twenty each, and a fixed 200, up to the bound itself:
    /// one that takes what is held to the bound exactly is added, and the
    /// next is refused, said once. Asked of the door rather than of an
    /// input, since the length and the tokens are the door's to take: no
    /// caller can pass none. The costs are written as numbers, not as the
    /// constants, so a constant changed from what was measured fails here.
    ///
    /// Mutations: charge any kind's text or tokens as nothing, change a
    /// cost, or refuse at the bound rather than past it; an assertion
    /// fails.
    #[test]
    fn each_file_added_is_charged_its_text_and_tokens_up_to_the_bound() {
        let directory = std::env::temp_dir().join(format!("safec_held_{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let header = directory.join("h.h");
        std::fs::write(&header, "a b").unwrap();

        // Three bytes and three tokens, the end of the file among them.
        let cost = 3 * 6 + 3 * 20 + 200;
        for kind in ["include", "expand", "substitute"] {
            let mut map = SourceMap::new();
            let file = map.add_virtual("t.c", "a b");
            let at = Span::new(file, 0, 3);
            let mut diagnostics = DiagnosticSink::new();
            let mut sources = Sources::new(&mut map, MAX_HELD_IN_ALL - cost);
            let add = |sources: &mut Sources<'_>, diagnostics: &mut DiagnosticSink| {
                let addition = match kind {
                    "include" => Addition::Include(&header, 3),
                    "expand" => Addition::Expand(at),
                    _ => Addition::Substitute(
                        "a b".to_owned(),
                        vec![Piece {
                            offset: 0,
                            from: at,
                        }],
                    ),
                };
                sources.add(addition, at, diagnostics).is_ok()
            };

            assert!(add(&mut sources, &mut diagnostics), "{kind} at the bound");
            assert_eq!(sources.held(), MAX_HELD_IN_ALL, "{kind}");
            assert!(
                !add(&mut sources, &mut diagnostics),
                "{kind} past the bound"
            );
            assert!(
                !add(&mut sources, &mut diagnostics),
                "{kind} past the bound again"
            );
            assert_eq!(diagnostics.diagnostics().len(), 1, "{kind} said once");
        }
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// A file not on disk is charged its text and the fixed cost, and its
    /// refusal is said where it is used, or of no place.
    ///
    /// Mutations: charge its text as nothing, or drop the label; an
    /// assertion fails.
    #[test]
    fn a_file_not_on_disk_is_charged_its_text_and_refused_where_it_is_used() {
        let mut map = SourceMap::new();
        let file = map.add_virtual("t.c", "#if defined X");
        let at = Span::new(file, 0, 3);
        let mut diagnostics = DiagnosticSink::new();
        let mut sources = Sources::new(&mut map, MAX_HELD_IN_ALL - (2 * 6 + 200));

        assert!(
            sources
                .add_virtual("v", "ab", Some((at, "evaluated here")), &mut diagnostics)
                .is_ok()
        );
        assert_eq!(sources.held(), MAX_HELD_IN_ALL);
        assert!(
            sources
                .add_virtual("v", "", Some((at, "evaluated here")), &mut diagnostics)
                .is_err()
        );
        let reported = &diagnostics.diagnostics()[0];
        assert_eq!(reported.labels()[0].span(), at);
    }
}
