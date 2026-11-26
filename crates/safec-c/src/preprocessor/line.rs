//! A directive's line as every reader of one takes it.
//!
//! C makes each comment one space before a directive is read (C17 5.1.1.2
//! p1, phase 3). Readers took the line as text, and each had to remember
//! that: `_Pragma`'s text was classified with its comments in it until the
//! hunt before #618 closed found a heap overflow `clang` compiled and safec
//! passed. So no reader takes text. Each takes a [`Line`], whose field is
//! private to this module and which only [`Line::read`] and
//! [`Line::operand`] make, both with the comments made spaces: handing a
//! reader the raw text instead fails `E0308`.
//!
//! And no reader decides where a name ends. Each asks [`Line::name_at`],
//! which reads the tokens the lexer made of the line, as `#if`'s are read.
//! Five readers wrote the rule out by hand before, and two of them were
//! holes `clang` read otherwise: `#ifndef A\u00e9` asked about `A`, and
//! `#pragma once$` was read as `once`. Found by review and by architecture
//! review 12.

use super::spliced;
use crate::diagnostics::DiagnosticSink;
use crate::lexer::lex_replacement;
use crate::token::{Punct, TokenKind};
use safec_ir::source::SourceMap;

/// A directive's line, or `_Pragma`'s text, with each comment in it made
/// spaces of the same length, so every offset into it is where it was.
#[derive(Debug)]
pub(super) struct Line {
    text: String,
    /// What the lexer makes of the text, as a replacement list's, where no
    /// `#` begins a directive, each with its offsets in the text.
    tokens: Vec<Lexed>,
}

/// A token of a [`Line`], by its offsets in the line's text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Lexed {
    pub(super) kind: TokenKind,
    pub(super) start: usize,
    pub(super) end: usize,
}

/// What stands where a name is read in a [`Line`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Name<'a> {
    /// A name, and the offset just past it.
    Read(&'a str, usize),
    /// Nothing, or a token that is no name: a number, a literal, a
    /// punctuator, or characters no token can begin, `@` among them, which
    /// `clang` reads as no name either.
    Not,
    /// A name the lexer could not read whole: `$`, a universal character
    /// name or a letter outside ASCII touches its end. Each goes on with the
    /// name in `clang` and this reads none of them, so what is there is not
    /// read as the shorter name before it. Any other character touching it
    /// ends it, as `clang` ends it: `#else@` is an `#else`, and read as no
    /// directive, a group `clang` compiles was skipped with nothing said.
    /// Found by review.
    Glued,
    /// A named character `\N{...}` touches its end. `clang` ends the name,
    /// reads it on, or refuses it, by the character the name names, and
    /// this has no table of the names, so the line is refused wherever it
    /// is, a skipped group's included: read as going on, `\N{NO-BREAK SPACE}`
    /// after `else` in a skipped group passed over a group `clang` takes.
    /// Found by the hunt before #1359 closed.
    Undecided,
}

/// Whether `c` is white space outside ASCII to `clang` 22.1.8: Unicode's
/// White_Space, as [`char::is_whitespace`] reads it, and U+180E, which
/// `clang` reads as white space (`-Wunicode-whitespace`) and Unicode no
/// longer does. Read as no white space, U+180E after `else` in a skipped
/// group passed over a group `clang` takes. Found by the hunt before #1359
/// closed.
pub(super) fn spacing(c: char) -> bool {
    !c.is_ascii() && (c.is_whitespace() || c == '\u{180e}')
}

/// The character a universal character name at the start of `text` names:
/// `\u` and four hex digits, or `\U` and eight. `None` where `text` does
/// not begin with one whole, which `clang` reads as a `\` and what follows.
fn universal(text: &str) -> Option<char> {
    let digits = if text.starts_with("\\u") {
        4
    } else if text.starts_with("\\U") {
        8
    } else {
        return None;
    };
    let hex = text.get(2..2 + digits)?;
    if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    char::from_u32(u32::from_str_radix(hex, 16).ok()?)
}

impl Line {
    /// The line `raw`, a directive's from its `#` to its end; or, where a
    /// comment in it goes on past it, `Err` with that comment made spaces to
    /// the end, which may be read for what it is but not acted on.
    ///
    /// A block comment that does not end on the line is not read: the lexer
    /// ends a directive at its newline, so the lines inside the comment
    /// would be compiled as code, and `#define X /*` above a null test left
    /// the test to be believed where C has none. Found by review. Nor is a
    /// splice in a comment, which carries the comment onto the line below,
    /// since C splices lines before it reads comments (C17 5.1.1.2 p1,
    /// phases 2 and 3): asked of the line after its comments were made
    /// spaces, which takes the splice with them, `#include "a.h" // \` was
    /// read as an include. Found by review, after it had been blessed.
    pub(super) fn read(raw: &str) -> Result<Line, Line> {
        let (text, ended) = blanked(raw);
        let line = Line::lexed(text);
        if ended && (spliced(&line.text) || !spliced(raw)) {
            Ok(line)
        } else {
            Err(line)
        }
    }

    /// `_Pragma`'s text, read as a `#pragma` line's is (C17 6.10.9 p1); or
    /// `None` where a comment in it does not end in it.
    pub(super) fn operand(text: &str) -> Option<Line> {
        let (text, ended) = blanked(text);
        ended.then(|| Line::lexed(text))
    }

    fn lexed(text: String) -> Line {
        let mut scratch = SourceMap::new();
        let file = scratch.add_virtual("directive", text.as_str());
        let tokens = lex_replacement(file, scratch.file(file), &mut DiagnosticSink::new())
            .into_iter()
            .filter(|token| !token.is_eof())
            // White space outside ASCII is white space to `clang`, which
            // warns of it (`-Wunicode-whitespace`), and the lexer leaves it
            // as characters no token can begin. Kept, `#pragma pack` and a
            // no-break space before its `(` was read as no `pack`.
            .filter(|token| {
                token.kind != TokenKind::Unknown || !text[token.span.range()].chars().all(spacing)
            })
            .map(|token| Lexed {
                kind: token.kind,
                start: token.span.start() as usize,
                end: token.span.end() as usize,
            })
            .collect();
        Line { text, tokens }
    }

    /// The text, its comments made spaces.
    pub(super) fn text(&self) -> &str {
        &self.text
    }

    /// The tokens that start at `offset` or after it.
    pub(super) fn tokens_from(&self, offset: usize) -> &[Lexed] {
        let first = self.tokens.partition_point(|token| token.start < offset);
        &self.tokens[first..]
    }

    /// The text of `token`.
    pub(super) fn spelled(&self, token: Lexed) -> &str {
        &self.text[token.start..token.end]
    }

    /// Where the tokens from `offset` on start and end, if any do.
    pub(super) fn rest_at(&self, offset: usize) -> Option<(u32, u32)> {
        let rest = self.tokens_from(offset);
        Some((rest.first()?.start as u32, rest.last()?.end as u32))
    }

    /// The name that is the first token at `offset` or after it. A keyword
    /// or an annotation is a name here, since preprocessing knows only
    /// identifiers (C17 6.4 p1).
    pub(super) fn name_at(&self, offset: usize) -> Name<'_> {
        let rest = self.tokens_from(offset);
        let Some(&token) = rest.first() else {
            return Name::Not;
        };
        match token.kind {
            TokenKind::Identifier | TokenKind::Keyword(_) | TokenKind::Annotation(_) => {
                // What touches the name's end, as `clang` reads it: a
                // universal character name goes on with it unless it names
                // white space, and one not whole is a `\` that ends it.
                let after = &self.text[token.end..];
                let goes_on = after.starts_with('$')
                    || universal(after).is_some_and(|c| !spacing(c))
                    || after.starts_with(|c: char| !c.is_ascii() && !spacing(c));
                if after.starts_with("\\N{") {
                    Name::Undecided
                } else if goes_on {
                    Name::Glued
                } else {
                    Name::Read(self.spelled(token), token.end)
                }
            }
            _ => Name::Not,
        }
    }

    /// The directive's name and the offset just past it: `""` where none
    /// follows the `#`, and a line marker's number as written; `None` where
    /// the name is [`Name::Glued`], or where white space outside ASCII
    /// stands between the `#` and what follows it, as [`Line::spaced_oddly`]
    /// says.
    pub(super) fn directive(&self) -> Option<(&str, usize)> {
        let hash = match self.tokens.first() {
            Some(&token) if token.kind == TokenKind::Punct(Punct::Hash) => token,
            _ => return Some(("", 0)),
        };
        if self.spaced_oddly() {
            return None;
        }
        match self.tokens.get(1) {
            Some(&number) if number.kind == TokenKind::Number => {
                Some((self.spelled(number), number.end))
            }
            _ => match self.name_at(hash.end) {
                Name::Read(name, end) => Some((name, end)),
                Name::Not => Some(("", hash.end)),
                Name::Glued | Name::Undecided => None,
            },
        }
    }

    /// Whether the directive's own name is [`Name::Undecided`], which is
    /// refused in every group, a skipped one included.
    pub(super) fn undecided(&self) -> bool {
        matches!(
            (self.tokens.first(), self.tokens.get(1)),
            (Some(hash), Some(_)) if hash.kind == TokenKind::Punct(Punct::Hash)
                && !self.spaced_oddly()
                && self.name_at(hash.end) == Name::Undecided
        )
    }

    /// Whether white space outside ASCII stands between the `#` and the
    /// token after it, which after a name is white space. In a group `clang`
    /// skips, the line is no directive and is passed over, not counted. In a
    /// group it compiles, `clang` reads a directive with a warning and
    /// `clang -E` refuses the line as `invalid preprocessing directive`, so
    /// it is refused here rather than read either way. Read as a space, as
    /// it is after a name, `#` and a
    /// no-break space and `else` in a skipped group chose a group `clang`
    /// never chooses, and a double free in it exited 0. Found by the hunt
    /// before #1359 closed.
    pub(super) fn spaced_oddly(&self) -> bool {
        let (Some(hash), Some(next)) = (self.tokens.first(), self.tokens.get(1)) else {
            return false;
        };
        hash.kind == TokenKind::Punct(Punct::Hash)
            && self.text[hash.end..next.start].chars().any(spacing)
    }
}

/// `text` with each comment in it made spaces of the same length, and
/// whether every block comment in it ends in it; one that does not is made
/// spaces to the end. A string or character literal is passed over, since
/// `/*` in one opens nothing.
fn blanked(text: &str) -> (String, bool) {
    let mut bytes = text.as_bytes().to_vec();
    let mut at = 0;
    let mut quote = None;
    let mut ended = true;
    while at < bytes.len() {
        match (quote, bytes[at]) {
            // An escape in a literal, whose next byte closes nothing.
            (Some(_), b'\\') if at + 1 < bytes.len() => {
                at += 2;
                continue;
            }
            (Some(open), byte) if byte == open => quote = None,
            (Some(_), _) => {}
            (None, b'"' | b'\'') => quote = Some(bytes[at]),
            (None, b'/') if bytes.get(at + 1) == Some(&b'/') => {
                bytes[at..].fill(b' ');
                break;
            }
            (None, b'/') if bytes.get(at + 1) == Some(&b'*') => {
                let Some(close) = text[at + 2..].find("*/") else {
                    bytes[at..].fill(b' ');
                    ended = false;
                    break;
                };
                let end = close + at + 4;
                bytes[at..end].fill(b' ');
                at = end;
                continue;
            }
            (None, _) => {}
        }
        at += 1;
    }
    let text = String::from_utf8(bytes).expect("only whole comments are made spaces");
    (text, ended)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> Line {
        Line::read(text).expect("a line that reads")
    }

    /// A directive's name begins after any white space, and is what the
    /// lexer reads: one a `$`, a universal character name or a letter
    /// outside ASCII touches is no name at all, and any other character
    /// touching it ends it, as `clang` ends it.
    ///
    /// White space outside ASCII between the `#` and what follows it makes
    /// no directive, as `clang` reads it, though after the name it is white
    /// space.
    ///
    /// Mutations: read a name a `$` touches as the name before it, refuse
    /// one that `@` or a lone `\` touches, read the token after `#` as a
    /// name whatever it is, or read white space outside ASCII after `#` as
    /// a space; an assertion fails.
    #[test]
    fn a_directive_s_name_is_what_the_lexer_reads_whole() {
        assert_eq!(line("#  pragma once").directive(), Some(("pragma", 9)));
        assert_eq!(line("#\u{b}\u{c}line 4").directive(), Some(("line", 7)));
        assert_eq!(line("%:line 4").directive(), Some(("line", 6)));
        assert_eq!(line("#pragmas").directive(), Some(("pragmas", 8)));
        assert_eq!(line("#if(1)").directive(), Some(("if", 3)));
        assert_eq!(line("# 40 \"a.c\"").directive(), Some(("40", 4)));
        assert_eq!(line("#").directive(), Some(("", 1)));
        assert_eq!(line("# \"a\"").directive(), Some(("", 1)));
        assert_eq!(line("#line$ 4").directive(), None);
        assert_eq!(line("#line\\u00e9 4").directive(), None);
        assert_eq!(line("#line\u{e9} 4").directive(), None);
        assert_eq!(line("#$line").directive(), Some(("", 1)));
        assert_eq!(line("#else@").directive(), Some(("else", 5)));
        assert_eq!(line("#endif\\").directive(), Some(("endif", 6)));
        assert_eq!(line("#else\u{3000}").directive(), Some(("else", 5)));
        assert_eq!(line("#\u{a0}define X").directive(), None);
        assert_eq!(line("# \u{3000}else").directive(), None);
        assert_eq!(line("#define\u{a0}X").directive(), Some(("define", 7)));
        assert_eq!(line("#\u{a0}").directive(), Some(("", 1)));
    }

    /// A comment is made spaces outside a literal only: `/*` and `//` in a
    /// string or a character constant, or after an escaped quote in one,
    /// open nothing.
    ///
    /// Mutations: take only `"` as opening a literal, or read no escape in
    /// one; an assertion fails.
    #[test]
    fn a_comment_is_made_spaces_only_outside_a_literal() {
        assert_eq!(
            line("#define C '\"' /* x */").text(),
            "#define C '\"'        "
        );
        assert_eq!(line("#define C '/*' 1").text(), "#define C '/*' 1");
        assert_eq!(
            line("#define S \"a\\\"/*\" 1 // x").text(),
            "#define S \"a\\\"/*\" 1     "
        );
    }

    /// A name is read where one stands, after white space, white space
    /// outside ASCII or a comment; a literal, a number or a character no
    /// token can begin there is none, and a `$` touching a name's end makes
    /// it one not read.
    ///
    /// Mutations: answer `Read` for a keyword-free identifier only, skip the
    /// check of what touches its end, or keep white space outside ASCII as
    /// a token; an assertion fails.
    #[test]
    fn a_name_is_read_only_where_the_lexer_reads_one_whole() {
        let read = line("#define A /* x */ int(b) \"c\" 1 d$ \u{a0}e");
        assert_eq!(read.name_at(7), Name::Read("A", 9));
        assert_eq!(read.name_at(9), Name::Read("int", 21));
        assert_eq!(read.name_at(22), Name::Read("b", 23));
        assert_eq!(read.name_at(24), Name::Not);
        assert_eq!(read.name_at(29), Name::Not);
        assert_eq!(read.name_at(31), Name::Glued);
        assert_eq!(read.name_at(34), Name::Read("e", 37));
        assert_eq!(read.name_at(read.text().len()), Name::Not);
    }

    /// What touches a name's end is read as `clang` 22.1.8 reads it, each
    /// row measured with `clang -std=c17 -fsyntax-only` after `#else` in a
    /// skipped group: a named character is undecided, a universal character
    /// name goes on unless it names white space or is not whole, and a
    /// character outside ASCII goes on unless it is white space to `clang`,
    /// U+180E among it.
    ///
    /// Mutations: read `\N{` as going on, a universal character name for
    /// white space as going on, one not whole as going on, or U+180E as no
    /// white space; that row fails.
    #[test]
    fn what_touches_a_name_is_read_as_clang_reads_it() {
        for (after, expected) in [
            ("\\N{NO-BREAK SPACE}", Name::Undecided),
            ("\\N{LATIN SMALL LETTER E WITH ACUTE}", Name::Undecided),
            ("\\u00e", Name::Read("else", 5)),
            ("\\u00a0", Name::Read("else", 5)),
            ("\\U000000a0", Name::Read("else", 5)),
            ("\\u00e9", Name::Glued),
            ("\\U000000e9", Name::Glued),
            ("\\U000000", Name::Read("else", 5)),
            ("\\q", Name::Read("else", 5)),
            ("$", Name::Glued),
            ("@", Name::Read("else", 5)),
            ("\u{e9}", Name::Glued),
            ("\u{180e}", Name::Read("else", 5)),
            ("\u{a0}", Name::Read("else", 5)),
        ] {
            let text = format!("#else{after}");
            assert_eq!(line(&text).name_at(1), expected, "{text:?}");
        }
        assert!(line("#else\\N{NO-BREAK SPACE}").undecided());
        assert!(!line("#else \\N{NO-BREAK SPACE}").undecided());
        assert!(!line("#else\\u00e9").undecided());
    }
}
