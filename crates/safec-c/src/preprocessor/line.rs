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

use super::spliced;

/// A directive's line, or `_Pragma`'s text, with each comment in it made
/// spaces of the same length, so every offset into it is where it was.
#[derive(Debug)]
pub(super) struct Line {
    text: String,
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
        let line = Line { text };
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
        ended.then_some(Line { text })
    }

    /// The text, its comments made spaces.
    pub(super) fn text(&self) -> &str {
        &self.text
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
