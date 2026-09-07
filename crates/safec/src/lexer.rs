//! Turning source text into tokens.
//!
//! One rule shapes this module: **the scan does not stop, and every byte gets a
//! place.** Three decisions follow from it rather than standing on their own.
//!
//! [`TokenKind::Eof`] is the rule at the end of the file. A parser asking for
//! the next token always gets one, so running out of input is an unexpected
//! token like any other instead of a second error path every caller has to
//! remember.
//!
//! [`TokenKind::Unknown`] is the rule at a character that can begin no token.
//! It is reported and kept. Stopping at the first one is the same mistake the
//! driver already refuses to make with the first unreadable path: the user pays
//! for it by fixing one thing per run.
//!
//! A preprocessing directive is the rule at syntax that is not implemented yet.
//! `#` is an ordinary punctuator, and a directive is a matter of what a later
//! stage does with the tokens on that line, so the scan says once that it
//! cannot handle the line and then scans it like any other.
//!
//! What the scan does *not* do is decide meaning. A [`TokenKind::Number`] names
//! a span and stops; its value and its type belong to a stage that knows the
//! target's type sizes. See ADR-0006.
//!
//! # Not here yet
//!
//! Line splicing (a `\` before a newline), literal prefixes (`L"..."`, `u8'x'`)
//! and non-ASCII identifiers. Each currently scans as something else and is
//! reported or mis-grouped rather than silently accepted.

use crate::diagnostics::{Code, Diagnostic, DiagnosticSink, Label};
use crate::source::{FileId, SourceFile, Span};
use crate::token::{Keyword, Punct, Token, TokenKind};

// Lexical diagnostics take `E01xx`. A code is assigned once and never reused,
// so the ranges are set aside now rather than renumbered when the next stage
// starts reporting.
const UNTERMINATED_COMMENT: Code = Code::new("E0101");
const UNTERMINATED_LITERAL: Code = Code::new("E0102");
const UNEXPECTED_CHARACTERS: Code = Code::new("E0103");
const UNSUPPORTED_DIRECTIVE: Code = Code::new("E0104");

/// Scan `source` into tokens, reporting what it could not make sense of.
///
/// Always returns a stream, and the stream always ends with [`TokenKind::Eof`].
/// A caller checks [`DiagnosticSink::has_errors`] to learn whether the scan
/// found anything wrong; it does not learn that from the return value, because
/// the tokens are still worth having when it did.
pub fn lex(file: FileId, source: &SourceFile, diagnostics: &mut DiagnosticSink) -> Vec<Token> {
    Lexer {
        file,
        text: source.contents(),
        offset: 0,
        at_line_start: true,
    }
    .run(diagnostics)
}

struct Lexer<'a> {
    file: FileId,
    text: &'a str,
    /// A byte offset into `text`, always on a character boundary.
    offset: usize,
    /// Whether a newline has been passed since the last token.
    ///
    /// Kept here rather than on [`Token`]. The preprocessor will want it per
    /// token, which is why `clang` records it there, but nothing outside this
    /// scan wants it today, and tokens are built in one place.
    at_line_start: bool,
}

impl<'a> Lexer<'a> {
    fn run(mut self, diagnostics: &mut DiagnosticSink) -> Vec<Token> {
        let mut tokens = Vec::new();

        loop {
            self.skip_trivia(diagnostics);

            let start = self.offset;
            let Some(c) = self.peek() else { break };

            if self.at_line_start && c == '#' {
                self.report_directive(diagnostics);
            }

            let kind = self.scan(c, diagnostics);
            debug_assert!(self.offset > start, "the scan made no progress");
            tokens.push(Token::new(kind, self.span(start)));
            self.at_line_start = false;
        }

        tokens.push(Token::new(
            TokenKind::Eof,
            Span::at(self.file, self.offset as u32),
        ));
        tokens
    }

    fn scan(&mut self, c: char, diagnostics: &mut DiagnosticSink) -> TokenKind {
        if is_identifier_start(c) {
            return self.scan_word();
        }
        if c.is_ascii_digit()
            || (c == '.' && self.rest()[1..].starts_with(|c: char| c.is_ascii_digit()))
        {
            return self.scan_number();
        }
        if c == '"' {
            return self.scan_quoted('"', TokenKind::String, "string literal", diagnostics);
        }
        if c == '\'' {
            return self.scan_quoted(
                '\'',
                TokenKind::Character,
                "character constant",
                diagnostics,
            );
        }
        // Checked after the cases above so that `.5` is a number and `...` is a
        // punctuator, and within itself longest-first, which is C's rule: the
        // scan takes the longest run of characters that could be a token even
        // when a shorter one would let the rest parse.
        if let Some(punct) = Punct::starting(self.rest()) {
            self.offset += punct.as_str().len();
            return TokenKind::Punct(punct);
        }
        self.scan_unknown(diagnostics)
    }

    /// An identifier, or the keyword it is spelled the same as.
    fn scan_word(&mut self) -> TokenKind {
        let start = self.offset;
        while self.peek().is_some_and(is_identifier_continue) {
            self.bump();
        }
        match Keyword::from_spelling(&self.text[start..self.offset]) {
            Some(keyword) => TokenKind::Keyword(keyword),
            None => TokenKind::Identifier,
        }
    }

    /// A preprocessing number, which is wider than the set of valid constants.
    ///
    /// `123abc` is one token rather than a number and an identifier, so
    /// whatever rejects it later has the whole thing to point at.
    fn scan_number(&mut self) -> TokenKind {
        self.bump();
        while let Some(c) = self.peek() {
            match c {
                // A sign is part of the number only after an exponent marker,
                // which is why `1e+5` is one token and `x+5` is three.
                'e' | 'E' | 'p' | 'P' => {
                    self.bump();
                    if matches!(self.peek(), Some('+' | '-')) {
                        self.bump();
                    }
                }
                '.' => {
                    self.bump();
                }
                c if is_identifier_continue(c) => {
                    self.bump();
                }
                _ => break,
            }
        }
        TokenKind::Number
    }

    fn scan_quoted(
        &mut self,
        quote: char,
        kind: TokenKind,
        what: &str,
        diagnostics: &mut DiagnosticSink,
    ) -> TokenKind {
        let opening = self.offset;
        self.bump();

        loop {
            match self.peek() {
                // A literal closes on the line it opens on. Reporting at the
                // newline rather than running to the end of the file keeps one
                // missing quote from swallowing the rest of the program.
                None | Some('\n') => {
                    diagnostics.report(
                        Diagnostic::error(format!("unterminated {what}"))
                            .with_code(UNTERMINATED_LITERAL)
                            .with_label(Label::primary(
                                Span::new(self.file, opening as u32, opening as u32 + 1),
                                "unclosed from here",
                            ))
                            .with_note(format!(
                                "a {what} has to be closed on the line it opens on"
                            )),
                    );
                    return kind;
                }
                Some('\\') => {
                    self.bump();
                    // Not past a newline: that is a line splice, which is not
                    // implemented, and consuming it here would hide the
                    // unterminated literal instead of reporting it.
                    if self.peek().is_some_and(|c| c != '\n') {
                        self.bump();
                    }
                }
                Some(c) if c == quote => {
                    self.bump();
                    return kind;
                }
                Some(_) => {
                    self.bump();
                }
            }
        }
    }

    /// A run of characters that can begin no token.
    ///
    /// One token and one diagnostic for the whole run, not one per character.
    /// A file that is not C at all would otherwise bury its own first line
    /// under thousands of identical reports.
    fn scan_unknown(&mut self, diagnostics: &mut DiagnosticSink) -> TokenKind {
        let start = self.offset;
        while self.peek().is_some_and(begins_no_token) {
            self.bump();
        }

        let span = self.span(start);
        let count = self.text[start..self.offset].chars().count();
        // The characters themselves are shown by the quoted source line, not
        // spelled into the message: a message is not the place to put file
        // content.
        let message = if count == 1 {
            "unexpected character".to_owned()
        } else {
            format!("{count} unexpected characters")
        };

        diagnostics.report(
            Diagnostic::error(message)
                .with_code(UNEXPECTED_CHARACTERS)
                .with_label(Label::primary(span, "not part of any token")),
        );
        TokenKind::Unknown
    }

    fn report_directive(&self, diagnostics: &mut DiagnosticSink) {
        diagnostics.report(
            Diagnostic::error("preprocessor directives are not supported yet")
                .with_code(UNSUPPORTED_DIRECTIVE)
                .with_label(Label::primary(
                    Span::new(self.file, self.offset as u32, self.offset as u32 + 1),
                    "directive begins here",
                ))
                .with_note(
                    "the preprocessor arrives in stage 3; the line is scanned as ordinary tokens",
                ),
        )
    }

    /// The one place whitespace and comments are passed over.
    ///
    /// Kept to one function so that recording them later is a push rather than
    /// a search. They are not put in the stream: a side table keyed by token
    /// index is the shape that works, and mixing them in breaks every parser
    /// written against `tokens[i]` and `tokens[i + 1]` being adjacent.
    fn skip_trivia(&mut self, diagnostics: &mut DiagnosticSink) {
        loop {
            match self.peek() {
                // Only '\n' opens a line, which is what `SourceFile` counts by.
                // A vertical tab is whitespace and does not.
                Some('\n') => {
                    self.bump();
                    self.at_line_start = true;
                }
                Some(c) if c.is_whitespace() => {
                    self.bump();
                }
                Some('/') if self.rest().starts_with("//") => {
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.bump();
                    }
                }
                Some('/') if self.rest().starts_with("/*") => {
                    self.skip_block_comment(diagnostics);
                }
                _ => return,
            }
        }
    }

    fn skip_block_comment(&mut self, diagnostics: &mut DiagnosticSink) {
        let opening = self.offset;
        self.offset += "/*".len();

        loop {
            if self.rest().starts_with("*/") {
                self.offset += "*/".len();
                return;
            }
            match self.bump() {
                Some('\n') => self.at_line_start = true,
                Some(_) => {}
                None => {
                    diagnostics.report(
                        Diagnostic::error("unterminated block comment")
                            .with_code(UNTERMINATED_COMMENT)
                            .with_label(Label::primary(
                                Span::new(self.file, opening as u32, opening as u32 + 2),
                                "unclosed from here",
                            ))
                            .with_note("the comment runs to the end of the file"),
                    );
                    return;
                }
            }
        }
    }

    fn rest(&self) -> &'a str {
        &self.text[self.offset..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.offset += c.len_utf8();
        Some(c)
    }

    fn span(&self, start: usize) -> Span {
        Span::new(self.file, start as u32, self.offset as u32)
    }
}

/// ASCII only. C11 allows more through universal character names and C23
/// through `XID_Start`, and neither is implemented: a non-ASCII identifier is
/// reported rather than accepted.
fn is_identifier_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_identifier_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Whether no token can begin with `c`.
fn begins_no_token(c: char) -> bool {
    !c.is_whitespace()
        && !is_identifier_start(c)
        && !c.is_ascii_digit()
        && c != '"'
        && c != '\''
        && !Punct::can_start_with(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceMap;

    struct Scan {
        sources: SourceMap,
        file: FileId,
        tokens: Vec<Token>,
        diagnostics: DiagnosticSink,
    }

    impl Scan {
        fn kinds(&self) -> Vec<TokenKind> {
            self.tokens.iter().map(|token| token.kind).collect()
        }

        /// The text each token covers, `Eof` excluded.
        fn texts(&self) -> Vec<&str> {
            self.tokens
                .iter()
                .filter(|token| !token.is_eof())
                .map(|token| self.sources.snippet(token.span))
                .collect()
        }

        fn messages(&self) -> Vec<&str> {
            self.diagnostics
                .diagnostics()
                .iter()
                .map(Diagnostic::message)
                .collect()
        }
    }

    fn scan(text: &str) -> Scan {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("test.c", text);
        let mut diagnostics = DiagnosticSink::new();
        let tokens = lex(file, sources.file(file), &mut diagnostics);

        Scan {
            sources,
            file,
            tokens,
            diagnostics,
        }
    }

    #[test]
    fn an_empty_file_scans_to_nothing_but_the_end_of_it() {
        let scan = scan("");

        assert_eq!(scan.kinds(), [TokenKind::Eof]);
        assert_eq!(scan.tokens[0].span, Span::at(scan.file, 0));
        assert!(scan.diagnostics.is_empty());
    }

    /// Every stream ends with it, which is what lets a parser ask for the next
    /// token without a second path for having none.
    #[test]
    fn the_end_of_the_file_is_a_token_at_the_end_of_the_text() {
        for text in ["", "int", "int x;\n", "/* unterminated"] {
            let scan = scan(text);
            let last = *scan.tokens.last().expect("a stream is never empty");

            assert!(last.is_eof(), "{text:?}");
            assert_eq!(
                last.span,
                Span::at(scan.file, text.len() as u32),
                "{text:?}"
            );
        }
    }

    #[test]
    fn a_word_is_a_keyword_or_an_identifier() {
        let scan = scan("int main void_ x1 _y borrow");

        assert_eq!(
            scan.kinds(),
            [
                TokenKind::Keyword(Keyword::Int),
                TokenKind::Identifier,
                TokenKind::Identifier,
                TokenKind::Identifier,
                TokenKind::Identifier,
                TokenKind::Identifier,
                TokenKind::Eof,
            ]
        );
        assert_eq!(scan.texts(), ["int", "main", "void_", "x1", "_y", "borrow"]);
    }

    /// A number is delimited the way C delimits a preprocessing number, which
    /// is wider than the constants that are valid. `123abc` is one token so
    /// that whatever rejects it has all of it to point at.
    #[test]
    fn a_number_runs_as_far_as_a_preprocessing_number_does() {
        for (text, expected) in [
            ("123", "123"),
            ("123abc", "123abc"),
            ("1.5e+10", "1.5e+10"),
            ("0x1p-3", "0x1p-3"),
            (".5f", ".5f"),
            ("0", "0"),
        ] {
            let scan = scan(text);
            assert_eq!(scan.kinds()[0], TokenKind::Number, "{text}");
            assert_eq!(scan.texts(), [expected], "{text}");
        }
    }

    /// A sign continues a number only after an exponent marker.
    #[test]
    fn a_sign_is_part_of_a_number_only_after_an_exponent() {
        assert_eq!(scan("1e+5").texts(), ["1e+5"]);
        assert_eq!(scan("x+5").texts(), ["x", "+", "5"]);
    }

    #[test]
    fn a_dot_is_a_number_before_a_digit_and_a_punctuator_otherwise() {
        assert_eq!(scan(".5").kinds()[0], TokenKind::Number);
        assert_eq!(scan("...").kinds()[0], TokenKind::Punct(Punct::Ellipsis));
        assert_eq!(scan("p.x").kinds()[1], TokenKind::Punct(Punct::Dot));
    }

    #[test]
    fn the_scan_takes_the_longest_punctuator_that_fits() {
        assert_eq!(scan("a >>= b").texts(), ["a", ">>=", "b"]);
        assert_eq!(scan("a >> b").texts(), ["a", ">>", "b"]);
        assert_eq!(scan("a+++b").texts(), ["a", "++", "+", "b"]);
    }

    #[test]
    fn whitespace_and_comments_do_not_reach_the_stream() {
        let scan = scan("int /* here */ x; // and here\ny\n");

        assert_eq!(scan.texts(), ["int", "x", ";", "y"]);
        assert!(scan.diagnostics.is_empty());
    }

    #[test]
    fn a_string_and_a_character_constant_keep_their_quotes() {
        let scan = scan(r#"c = 'a'; s = "hi\"there";"#);

        assert!(scan.diagnostics.is_empty(), "{:?}", scan.messages());
        assert_eq!(
            scan.texts(),
            ["c", "=", "'a'", ";", "s", "=", r#""hi\"there""#, ";"]
        );
    }

    /// One missing quote must not swallow the rest of the program, so the
    /// literal ends at the newline and the next line scans normally.
    #[test]
    fn an_unterminated_literal_ends_at_its_line() {
        let scan = scan("s = \"oops;\nint x;\n");

        assert_eq!(scan.messages(), ["unterminated string literal"]);
        assert_eq!(
            scan.texts(),
            ["s", "=", "\"oops;", "int", "x", ";"],
            "the next line was not scanned"
        );
    }

    #[test]
    fn an_unterminated_block_comment_is_reported_and_ends_the_scan() {
        let scan = scan("int x;\n/* on and on");

        assert_eq!(scan.messages(), ["unterminated block comment"]);
        assert_eq!(scan.texts(), ["int", "x", ";"]);
        assert!(scan.tokens.last().unwrap().is_eof());
    }

    /// A run of them, not one each. A file that is not C at all would otherwise
    /// bury its own first line under thousands of identical reports.
    #[test]
    fn a_run_of_unexpected_characters_is_one_token_and_one_diagnostic() {
        let scan = scan("int x = @@@@;");

        assert_eq!(scan.messages(), ["4 unexpected characters"]);
        assert_eq!(scan.texts(), ["int", "x", "=", "@@@@", ";"]);
        assert_eq!(scan.kinds()[3], TokenKind::Unknown);
    }

    #[test]
    fn a_single_unexpected_character_is_said_in_the_singular() {
        assert_eq!(scan("@").messages(), ["unexpected character"]);
    }

    /// The rule this module is built on. Stopping at the first one is the same
    /// mistake as stopping at the first unreadable input.
    #[test]
    fn the_scan_does_not_stop_at_the_first_thing_it_cannot_read() {
        let scan = scan("@ int x; ` y \"unclosed\n");

        assert_eq!(scan.diagnostics.error_count(), 3);
        assert!(scan.texts().contains(&"int"));
        assert!(scan.texts().contains(&"y"));
    }

    #[test]
    fn a_directive_is_reported_once_and_its_line_is_still_scanned() {
        let scan = scan("#include <stdio.h>\nint x;\n");

        assert_eq!(
            scan.messages(),
            ["preprocessor directives are not supported yet"]
        );
        assert_eq!(
            scan.texts(),
            ["#", "include", "<", "stdio", ".", "h", ">", "int", "x", ";"],
            "the directive line is ordinary tokens"
        );
    }

    /// `#` is a punctuator wherever it appears. Only the first token on a line
    /// could begin a directive.
    #[test]
    fn a_hash_that_does_not_open_a_line_is_only_a_punctuator() {
        let scan = scan("a # b\n");

        assert!(scan.diagnostics.is_empty(), "{:?}", scan.messages());
        assert_eq!(scan.kinds()[1], TokenKind::Punct(Punct::Hash));
    }

    #[test]
    fn a_directive_after_a_comment_still_opens_its_line() {
        let scan = scan("/* note */\n#define X 1\n");

        assert_eq!(scan.diagnostics.error_count(), 1);
    }

    /// The rule stated as an invariant: the spans are ordered, do not overlap,
    /// and stay inside the file.
    #[test]
    fn token_spans_run_forward_and_stay_inside_the_file() {
        let scan = scan("int f(void) { return 'a' + 1.5e-3; } @@ /* x */ #x\n");
        let length = scan.sources.file(scan.file).len();

        let mut previous_end = 0;
        for token in &scan.tokens {
            assert!(token.span.start() >= previous_end, "{token:?}");
            assert!(token.span.end() <= length, "{token:?}");
            previous_end = token.span.end();
        }
    }

    #[test]
    fn a_scan_of_the_phase_one_target_produces_what_a_parser_needs() {
        let scan = scan("int add(int a, int b) { return a + b; }");

        assert!(scan.diagnostics.is_empty(), "{:?}", scan.messages());
        assert_eq!(
            scan.texts(),
            [
                "int", "add", "(", "int", "a", ",", "int", "b", ")", "{", "return", "a", "+", "b",
                ";", "}"
            ]
        );
    }
}
