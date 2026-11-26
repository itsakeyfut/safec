//! What one input's preprocessing holds, charged at the only two doors it
//! comes in by: a file added to the source map, and a token handed on.
//!
//! Each door is a type whose field is private to this module, so a way in
//! that skips the charge does not compile. The preprocessor holds
//! [`Sources`] instead of the [`SourceMap`] it wraps, which reads through
//! [`Deref`] and has no `DerefMut`: `sources.expand(..)` on it fails `E0596`.
//! And the tokens it hands on are a [`Handed`], whose `Vec` cannot be pushed
//! to from outside: `handed.0.push(..)` fails `E0616`. The bound was charged
//! by hand at four call sites before, review of #1337 found each could pass
//! no length with the suite green, and an inclusion was charged nowhere:
//! 4,000 inclusions of a guarded 64 KB header held 257 MB, and of a 512 KB
//! one 1.8 GB past three minutes, with nothing printed.

use std::io;
use std::ops::Deref;
use std::path::Path;

use super::TOO_MANY_EXPANSIONS;
use crate::diagnostics::{Diagnostic, DiagnosticSink, Label};
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
/// #1215. Past it the next addition or token is refused, once.
pub(super) const MAX_HELD_IN_ALL: usize = 256 << 20;

/// What one file added to the map costs beside its text, in bytes.
///
/// Measured, not reckoned, with the bound lifted: a release build read
/// `--emit tokens` of forty uses of `M15`, `#define M{n} (M{n-1} + M{n-1})`
/// down to `#define M0 1`, 2,621,400 expansions of a few bytes each, at a
/// peak working set of 817 MB, and ten such uses at 209 MB. Less the
/// tokens they hand on, charged at [`TOKEN_COST`], that is about 233 bytes
/// an expansion either way, its text among it.
pub(super) const INSTANCE_COST: usize = 240;

/// What one byte of a file's text costs in the map, at most, in bytes: the
/// byte, and the start of a line it can be.
///
/// A file's text is kept until the input ends, and so is where each of its
/// lines starts, four bytes a line, in a list that grows by doubling. A
/// header of comments or one under an include guard costs about the byte
/// alone: 400 inclusions of a 64 KB one, 26 MB of text, peaked at 32 MB.
/// One of nothing but line breaks costs the most: 400 inclusions of 65,536
/// of them peaked at 161 MB, 5.9 bytes a byte. Charged as a dense list's
/// tokens are, at 17 a byte, the 10.5 MB of headers `windows.h`, `stdio.h`,
/// `stdlib.h` and `string.h` are read from would take 70% of the bound.
pub(super) const TEXT_COST: usize = 6;

/// What one token handed on costs, at most, in bytes.
///
/// A token handed on is held until the input ends, and the list it is in
/// grows by doubling. Measured with the bound lifted: 400 inclusions of a
/// header of 65,536 commas, 26 million tokens, peaked at 535 MB, 19.2 bytes
/// a token beside the text, where the same length of comments peaked at
/// 34 MB.
pub(super) const TOKEN_COST: usize = 20;

/// The source map, through which alone the preprocessor adds a file to it.
pub(super) struct Sources<'map> {
    map: &'map mut SourceMap,
    /// What has been charged, kept past [`MAX_HELD_IN_ALL`] once it was
    /// refused, which no charge that is admitted can leave it.
    held: usize,
}

/// A file to add to the map, and what it is.
pub(super) enum Addition<'a> {
    /// The file at the path, `u64` bytes long by its metadata, charged
    /// before it is read, so a header too large is never read at all.
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

    /// Charge `addition` and add it, used at `at`; or refuse it where it
    /// would take what is held past the bound, reported once at `at`.
    pub(super) fn add(
        &mut self,
        addition: Addition<'_>,
        at: Span,
        diagnostics: &mut DiagnosticSink,
    ) -> Result<FileId, Unadded> {
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
        Ok(match addition {
            Addition::Include(path, _) => self.map.include(path, at).map_err(Unadded::Io)?,
            Addition::Expand(spelled) => self.map.expand(spelled, at),
            Addition::Substitute(text, pieces) => self.map.substitute(text, pieces, at),
        })
    }

    /// Charge and add a file that is not on disk, named `name`, as
    /// [`SourceMap::add_virtual`] does; it is used nowhere, so a refusal is
    /// said of no place.
    pub(super) fn add_virtual(
        &mut self,
        name: &str,
        contents: &str,
        diagnostics: &mut DiagnosticSink,
    ) -> Result<FileId, Unadded> {
        let cost = contents
            .len()
            .saturating_mul(TEXT_COST)
            .saturating_add(INSTANCE_COST);
        if !self.charge(cost, None, diagnostics) {
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

/// The tokens handed on, each charged as it is.
#[derive(Debug, Default)]
pub(super) struct Handed(Vec<Token>);

impl Handed {
    /// Hand `token` on; or, where it would take what is held past the bound,
    /// refuse it, said once, and every token after it.
    pub(super) fn push(
        &mut self,
        token: Token,
        sources: &mut Sources<'_>,
        diagnostics: &mut DiagnosticSink,
    ) -> bool {
        if !sources.charge(TOKEN_COST, Some((token.span, "read here")), diagnostics) {
            return false;
        }
        self.0.push(token);
        true
    }

    /// Whether nothing has been handed on.
    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The tokens handed on.
    pub(super) fn into_tokens(self) -> Vec<Token> {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::TokenKind;

    /// Each kind of file added is charged its own text, at [`TEXT_COST`] a
    /// byte, and [`INSTANCE_COST`], up to the bound itself: one that takes
    /// what is held to the bound exactly is added, and the next is refused,
    /// said once. Asked of the door rather than of an input, since the
    /// length is the door's to take: no caller can pass none.
    ///
    /// Mutations: charge any kind's text as nothing, charge the fixed cost
    /// alone, or refuse at the bound rather than past it; an assertion
    /// fails.
    #[test]
    fn each_file_added_is_charged_its_text_up_to_the_bound() {
        let directory = std::env::temp_dir().join(format!("safec_held_{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let header = directory.join("h.h");
        std::fs::write(&header, "ab").unwrap();

        for kind in ["include", "expand", "substitute", "virtual"] {
            let mut map = SourceMap::new();
            let file = map.add_virtual("t.c", "ab");
            let at = Span::new(file, 0, 2);
            let mut diagnostics = DiagnosticSink::new();
            let mut sources =
                Sources::new(&mut map, MAX_HELD_IN_ALL - INSTANCE_COST - 2 * TEXT_COST);
            let add = |sources: &mut Sources<'_>, diagnostics: &mut DiagnosticSink| match kind {
                "include" => sources
                    .add(Addition::Include(&header, 2), at, diagnostics)
                    .is_ok(),
                "expand" => sources.add(Addition::Expand(at), at, diagnostics).is_ok(),
                "substitute" => sources
                    .add(
                        Addition::Substitute(
                            "ab".to_owned(),
                            vec![Piece {
                                offset: 0,
                                from: at,
                            }],
                        ),
                        at,
                        diagnostics,
                    )
                    .is_ok(),
                _ => sources.add_virtual("v", "ab", diagnostics).is_ok(),
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

    /// Each token handed on is charged [`TOKEN_COST`], up to the bound
    /// itself, and past it refused, said once.
    ///
    /// Mutations: charge a token nothing, or refuse at the bound rather than
    /// past it; an assertion fails.
    #[test]
    fn each_token_handed_on_is_charged_up_to_the_bound() {
        let mut map = SourceMap::new();
        let file = map.add_virtual("t.c", "a");
        let token = Token::new(TokenKind::Identifier, Span::new(file, 0, 1));
        let mut diagnostics = DiagnosticSink::new();
        let mut sources = Sources::new(&mut map, MAX_HELD_IN_ALL - TOKEN_COST);
        let mut handed = Handed::default();

        assert!(handed.push(token, &mut sources, &mut diagnostics));
        assert_eq!(sources.held(), MAX_HELD_IN_ALL);
        assert!(!handed.push(token, &mut sources, &mut diagnostics));
        assert!(!handed.push(token, &mut sources, &mut diagnostics));
        assert_eq!(handed.into_tokens(), [token]);
        assert_eq!(diagnostics.diagnostics().len(), 1);
    }
}
