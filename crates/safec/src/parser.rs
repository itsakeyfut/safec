//! Tokens to a tree, by recursive descent.
//!
//! What it reads is the smallest whole translation unit: a function of no
//! parameters returning `int`, a compound statement, `return`, and a numeric
//! constant. Everything else in [`docs/frontend.md`]'s Stage 1 arrives in the
//! sibling issues of the one that added this, and each of them adds to
//! [`crate::ast`] rather than reshaping it.
//!
//! It is handed tokens and not the source. ADR-0006 reserved a trigger, that
//! the parser asking for a `&SourceMap` is the moment an interner is worth
//! having, and this does not fire it: a name is carried as a span, resolving
//! one to text is the printer's job, and comparing one name with another is the
//! semantic analysis's.
//!
//! [`docs/frontend.md`]: https://github.com/itsakeyfut/safec/blob/main/docs/frontend.md

use crate::ast::{Ast, Expr, Function, Item, Stmt, StmtId};
use crate::diagnostics::{Code, Diagnostic, DiagnosticSink, Label};
use crate::source::{FileId, Span};
use crate::token::{Keyword, Punct, Token, TokenKind};

/// Something was expected and something else was there.
///
/// `E01xx` is the lexer's range and `E03xx` is where the safety analysis's
/// example already sits, so the parser takes `E02xx`. One code covers every
/// shape of this because what differs between them is the message, and a code
/// exists so that a reader can search for a class of error rather than a
/// sentence.
const EXPECTED: Code = Code::new("E0201");

/// A program nested deeper than this parser will go.
///
/// Its own code rather than [`EXPECTED`], because nothing was expected: the
/// program is well formed and this compiler is declining to read it, which is a
/// different thing to tell a reader and a different thing to search for.
const TOO_DEEP: Code = Code::new("E0202");

/// How many compound statements may be open at once.
///
/// C17 5.2.4.1 asks an implementation to manage 127 levels of nested blocks, so
/// this is above the line a conforming one has to reach. It is a limit all the
/// same, and the reason to have one is that the alternative is not "no limit":
/// it is the native stack, which this parser recurses on and which ends a run
/// at around 1200 levels by killing the process, with no diagnostic and an exit
/// code the driver never chose. A limit that is reported is the same refusal
/// made legible. `clang` draws its own at 256 (`-fbracket-depth`).
///
/// The dump in `driver.rs` recurses over the same shape, so it is this that
/// keeps that recursion bounded too.
const MAX_NESTING: usize = 256;

/// Read a translation unit.
///
/// Always returns a tree. A file the parser could not read produces one holding
/// an error node, so that everything after this can walk a tree that is there
/// rather than answer for its absence.
pub fn parse(file: FileId, tokens: &[Token], diagnostics: &mut DiagnosticSink) -> Ast {
    // A directive never reaches the grammar. `TokenKind::Directive`'s own
    // comment says why the lexer keeps the line whole rather than splitting it:
    // its pieces make a parser produce an avalanche of errors about a line it
    // was never meant to read. Keeping the line whole only moves that avalanche
    // to one error instead of many, and one error is still one too many, since
    // the lexer has already reported the line and this parser stops at the
    // first thing it cannot read. Dropped here, in one place, rather than
    // skipped at each of the two loops, so that no position can be added later
    // where a directive is looked at.
    let tokens: Vec<Token> = tokens
        .iter()
        .copied()
        .filter(|token| token.kind != TokenKind::Directive)
        .collect();

    Parser {
        file,
        tokens: &tokens,
        at: 0,
        ast: Ast::new(),
        failed: false,
        budget: tokens.len() + 1,
        depth: 0,
    }
    .run(diagnostics)
}

struct Parser<'a> {
    file: FileId,
    tokens: &'a [Token],
    at: usize,
    ast: Ast,
    /// Whether a syntax error has been reported for this file.
    ///
    /// One is all this issue reports. Recovering and carrying on is a sibling
    /// issue, and until it lands a second error would be reported from a
    /// position the parser has no reason to believe in.
    failed: bool,
    /// How many more times any loop here may go round.
    ///
    /// Every iteration that succeeds consumes at least one token and every
    /// iteration that does not ends its loop, so a correct parser never spends
    /// more of this than the file has tokens. What it buys is the failure mode
    /// when that stops being true: a parser that cannot advance would otherwise
    /// hang, and a hanging suite reports nothing at all and takes the whole of
    /// CI with it. Running out is a panic that names itself.
    budget: usize,
    /// How many compound statements are open right now.
    ///
    /// Bounded by [`MAX_NESTING`], which is what stops the recursion between
    /// `statement` and `compound` reaching the end of the native stack.
    depth: usize,
}

impl Parser<'_> {
    /// Read items until the file ends or one of them fails.
    ///
    /// Stopping at the first is this issue's decision, and the sibling issue
    /// that adds recovery is what takes `failed` away. When it does, the budget
    /// below is what is left holding termination, and it owes a recovery that
    /// advances rather than one that merely reports.
    ///
    /// Mutation: drop `!self.failed` from the condition. The parser then meets
    /// the same token forever, the budget runs out, and
    /// `only_the_first_syntax_error_is_reported` fails by name instead of the
    /// suite hanging.
    fn run(mut self, diagnostics: &mut DiagnosticSink) -> Ast {
        while !self.at_end() && !self.failed {
            self.spend();
            let item = self.item(diagnostics);
            self.ast.push_item(item);
        }

        self.ast
    }

    /// Take one from the budget, or say that the parser stopped moving.
    ///
    /// Reached only when a loop here can go round without consuming anything,
    /// which is a defect in this file rather than in the program being read.
    /// The message says so, because the alternative is somebody spending an
    /// afternoon on a C file that is fine.
    fn spend(&mut self) {
        self.budget = self.budget.checked_sub(1).unwrap_or_else(|| {
            let at = self.peek().span.start();
            panic!("the parser stopped making progress at offset {at}, which is a defect in it")
        });
    }

    /// A function definition, which is the only item there is yet.
    fn item(&mut self, diagnostics: &mut DiagnosticSink) -> Item {
        let start = self.peek().span;

        if !self.eat(TokenKind::Keyword(Keyword::Int)) {
            return Item::Error {
                span: self.report(
                    "expected a declaration",
                    "this cannot begin one",
                    diagnostics,
                ),
            };
        }

        let Some(name) = self.expect(TokenKind::Identifier, "a name", diagnostics) else {
            return Item::Error { span: start };
        };

        for (kind, what) in [
            (TokenKind::Punct(Punct::LeftParen), "`(`"),
            (TokenKind::Keyword(Keyword::Void), "`void`"),
            (TokenKind::Punct(Punct::RightParen), "`)`"),
        ] {
            if self.expect(kind, what, diagnostics).is_none() {
                return Item::Error { span: start };
            }
        }

        // Counted here as well as in `statement`, so that `depth` is the
        // number of blocks open rather than the number of nested ones, and
        // `MAX_NESTING` means what it says.
        self.depth += 1;
        let body = self.compound(diagnostics);
        self.depth -= 1;

        let Some(body) = body else {
            return Item::Error { span: start };
        };

        let span = Span::new(self.file, start.start(), self.previous().span.end());
        Item::Function(Function { name, body, span })
    }

    /// A braced sequence of statements.
    fn compound(&mut self, diagnostics: &mut DiagnosticSink) -> Option<StmtId> {
        let start = self.peek().span;
        self.expect(TokenKind::Punct(Punct::LeftBrace), "`{`", diagnostics)?;

        let mut body = Vec::new();
        while !self.at_end() && !self.failed && !self.check(TokenKind::Punct(Punct::RightBrace)) {
            self.spend();
            body.push(self.statement(diagnostics));
        }

        self.expect(TokenKind::Punct(Punct::RightBrace), "`}`", diagnostics)?;

        let span = Span::new(self.file, start.start(), self.previous().span.end());
        Some(self.ast.push_stmt(Stmt::Compound { body, span }))
    }

    /// One statement, and where it went in the tree.
    ///
    /// The id rather than the node, because `compound` has already pushed a
    /// nested one and handing the node back would have the caller push a second
    /// copy of it. Two ids naming one pair of braces is exactly what ADR-0008
    /// exists to prevent: a side table keyed by id would have two slots for it
    /// and no way to say which the analysis meant.
    fn statement(&mut self, diagnostics: &mut DiagnosticSink) -> StmtId {
        let start = self.peek().span;

        if self.check(TokenKind::Punct(Punct::LeftBrace)) {
            // The only recursion in this file, so counting the depth here
            // bounds all of it. See `MAX_NESTING`.
            if self.depth == MAX_NESTING {
                let span = self.report_as(
                    TOO_DEEP,
                    "blocks are nested too deeply",
                    format!("{MAX_NESTING} blocks are already open here"),
                    diagnostics,
                );
                return self.ast.push_stmt(Stmt::Error { span });
            }

            self.depth += 1;
            let nested = self.compound(diagnostics);
            self.depth -= 1;

            return match nested {
                Some(id) => id,
                None => self.ast.push_stmt(Stmt::Error { span: start }),
            };
        }

        if self.eat(TokenKind::Keyword(Keyword::Return)) {
            let value = if self.check(TokenKind::Punct(Punct::Semicolon)) {
                None
            } else {
                let expr = self.expression(diagnostics);
                Some(self.ast.push_expr(expr))
            };

            if self
                .expect(TokenKind::Punct(Punct::Semicolon), "`;`", diagnostics)
                .is_none()
            {
                return self.ast.push_stmt(Stmt::Error { span: start });
            }

            let span = Span::new(self.file, start.start(), self.previous().span.end());
            return self.ast.push_stmt(Stmt::Return { value, span });
        }

        let span = self.report("expected a statement", "this cannot begin one", diagnostics);
        self.ast.push_stmt(Stmt::Error { span })
    }

    fn expression(&mut self, diagnostics: &mut DiagnosticSink) -> Expr {
        let span = self.peek().span;

        if self.eat(TokenKind::Number) {
            return Expr::Number { span };
        }

        Expr::Error {
            span: self.report(
                "expected an expression",
                "this cannot begin one",
                diagnostics,
            ),
        }
    }

    fn at_end(&self) -> bool {
        self.at >= self.tokens.len() || self.peek().is_eof()
    }

    fn peek(&self) -> Token {
        self.tokens[self.at.min(self.tokens.len() - 1)]
    }

    /// The token last consumed, for the end of a span.
    fn previous(&self) -> Token {
        self.tokens[self.at.saturating_sub(1).min(self.tokens.len() - 1)]
    }

    fn check(&self, kind: TokenKind) -> bool {
        !self.at_end() && self.peek().kind == kind
    }

    fn eat(&mut self, kind: TokenKind) -> bool {
        let matched = self.check(kind);
        if matched {
            self.at += 1;
        }
        matched
    }

    /// Consume `kind`, or report that it was expected.
    ///
    /// Returns the span of what was consumed, so that a caller can build the
    /// span of what it is reading without asking twice.
    fn expect(
        &mut self,
        kind: TokenKind,
        what: &str,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<Span> {
        if self.check(kind) {
            let span = self.peek().span;
            self.at += 1;
            return Some(span);
        }

        self.report(
            format!("expected {what}"),
            format!("{what} is missing here"),
            diagnostics,
        );
        None
    }

    /// Report at the token that is there, and give back its span.
    ///
    /// Only the first one speaks. Everything after it would be reported from a
    /// position nothing has recovered to, and a wall of errors about a file
    /// with one mistake in it is the shape `scan_directive` already refuses to
    /// produce.
    ///
    /// Mutation: report whether or not `failed` is set.
    /// `only_the_first_syntax_error_is_reported` fails.
    ///
    /// The token's text is never spelled into the message. RK-002 in the review
    /// knowledge bank records why: text out of a source file is content, and a
    /// `.c` holding an escape sequence must not be able to write it to a
    /// terminal through a diagnostic. The quoted source line above the caret is
    /// where a reader sees what was actually written, and the renderer escapes
    /// it on the way.
    fn report(
        &mut self,
        message: impl Into<String>,
        label: impl Into<String>,
        diagnostics: &mut DiagnosticSink,
    ) -> Span {
        self.report_as(EXPECTED, message, label, diagnostics)
    }

    /// The same, for the one thing that is not an `expected` at all.
    fn report_as(
        &mut self,
        code: Code,
        message: impl Into<String>,
        label: impl Into<String>,
        diagnostics: &mut DiagnosticSink,
    ) -> Span {
        let span = self.peek().span;

        if !self.failed {
            self.failed = true;
            diagnostics.report(
                Diagnostic::error(message)
                    .with_code(code)
                    .with_label(Label::primary(span, label)),
            );
        }

        span
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    use crate::source::SourceMap;

    struct Parsed {
        ast: Ast,
        diagnostics: DiagnosticSink,
    }

    /// Scan and parse, the way the driver does.
    fn parsed(text: &str) -> Parsed {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("test.c", text);
        let mut diagnostics = DiagnosticSink::new();
        let tokens = lex(file, sources.file(file), &mut diagnostics);
        let ast = parse(file, &tokens, &mut diagnostics);

        Parsed { ast, diagnostics }
    }

    /// The smallest whole translation unit, and the shape it makes.
    ///
    /// Mutation: have `item` push the body before the function, or `compound`
    /// return the wrong id. This fails, because the ids stop naming what the
    /// tree says they name.
    #[test]
    fn the_smallest_translation_unit_is_a_function_a_compound_and_a_return() {
        let parsed = parsed("int main(void) { return 0; }\n");

        assert_eq!(parsed.diagnostics.diagnostics().len(), 0);

        let [Item::Function(function)] = parsed.ast.items() else {
            panic!("{:?}", parsed.ast.items());
        };

        let Stmt::Compound { body, .. } = parsed.ast.stmt(function.body) else {
            panic!("{:?}", parsed.ast.stmt(function.body));
        };
        let [only] = body[..] else {
            panic!("{body:?}");
        };

        let Stmt::Return {
            value: Some(value), ..
        } = parsed.ast.stmt(only)
        else {
            panic!("{:?}", parsed.ast.stmt(only));
        };
        assert!(matches!(parsed.ast.expr(*value), Expr::Number { .. }));
    }

    /// A `return` with nothing to return still parses.
    ///
    /// Not valid in a function returning `int`, and saying so is the semantic
    /// analysis's job rather than this one's: the grammar C17 6.8.6.4 gives
    /// allows the expression to be absent.
    #[test]
    fn a_return_without_a_value_parses() {
        let parsed = parsed("int main(void) { return; }\n");

        assert_eq!(parsed.diagnostics.diagnostics().len(), 0);

        let [Item::Function(function)] = parsed.ast.items() else {
            panic!("{:?}", parsed.ast.items());
        };
        let Stmt::Compound { body, .. } = parsed.ast.stmt(function.body) else {
            panic!()
        };
        assert!(matches!(
            parsed.ast.stmt(body[0]),
            Stmt::Return { value: None, .. }
        ));
    }

    /// One syntax error per file, however many the file has.
    ///
    /// Reporting the second would mean believing a position the parser has no
    /// reason to believe in, because nothing has recovered yet. The sibling
    /// issue that adds recovery is what makes a second report worth reading.
    ///
    /// The input matters, and its first version did not. `0;` is not a
    /// statement, and the brace after it is then not where the compound
    /// expects one, so two places reach for the right to report. A file that
    /// fails at its very first token passes this whichever way `report`
    /// behaves, because the loop in `run` has ended before anything else can
    /// speak: it guards the loop and calls it the report.
    ///
    /// Two mutations, and each fails this on its own. Take the `failed` check
    /// out of `report` and the closing brace speaks as well as the statement.
    /// Drop `!self.failed` from the loop in `run` and the parser meets the same
    /// token until the budget runs out.
    #[test]
    fn only_the_first_syntax_error_is_reported() {
        let parsed = parsed("int main(void) { 0; }\n");

        assert_eq!(
            parsed.diagnostics.diagnostics().len(),
            1,
            "{:?}",
            parsed.diagnostics.diagnostics()
        );
    }

    /// The parser stops rather than reading past the end.
    ///
    /// `lex` always ends its stream with `Eof`, so an empty slice does not
    /// arrive through the driver. It arrives here, and the answer is an empty
    /// tree rather than a panic.
    #[test]
    fn no_tokens_at_all_parse_to_nothing() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("empty.c", "");
        let mut diagnostics = DiagnosticSink::new();

        let ast = parse(file, &[], &mut diagnostics);

        assert!(ast.items().is_empty());
        assert_eq!(diagnostics.diagnostics().len(), 0);
    }

    /// A file that is only a lexical error still leaves a tree, and the parser
    /// adds one report of its own rather than none or many.
    #[test]
    fn a_file_the_lexer_could_not_read_still_parses_to_a_tree() {
        let parsed = parsed("@\n");

        let [Item::Error { .. }] = parsed.ast.items() else {
            panic!("{:?}", parsed.ast.items());
        };
        assert_eq!(parsed.diagnostics.diagnostics().len(), 2);
    }

    /// A directive never reaches the grammar.
    ///
    /// The lexer keeps the line whole and reports it, and `TokenKind::Directive`
    /// says that is so a parser is not made to answer for a line that is not C.
    /// Keeping the line whole only turns an avalanche into one error, and one
    /// is still one too many: this parser stops at the first thing it cannot
    /// read, so a `#include` at the top of a real file left the function under
    /// it unparsed.
    ///
    /// Mutation: drop the filter in `parse`. The tree becomes a single error
    /// node and the parser adds a report of its own, and this fails on both.
    #[test]
    fn a_directive_is_not_something_the_grammar_reads() {
        let parsed = parsed(
            "#include <stdio.h>
int main(void) { return 0; }
",
        );

        // The lexer's own report stands. What must not be there is a second.
        assert_eq!(
            parsed.diagnostics.diagnostics().len(),
            1,
            "{:?}",
            parsed.diagnostics.diagnostics()
        );
        let [Item::Function(_)] = parsed.ast.items() else {
            panic!("{:?}", parsed.ast.items());
        };
    }

    /// Nesting is bounded, and the bound is reported rather than met.
    ///
    /// Mutation: remove the `MAX_NESTING` check in `statement`. This fails,
    /// because nothing is reported for the file past the limit.
    ///
    /// That is not the failure the bound exists to prevent, and no test here
    /// can be. Without a bound the recursion between `statement` and `compound`
    /// runs to the end of the native stack: 4000 blocks killed this compiler
    /// outright on this host, with no diagnostic and an exit code the driver
    /// never chose, and a test that reproduced it would take the suite with it
    /// rather than fail. So the input below stops one block past the limit, and
    /// what it holds is that the limit is enforced at all.
    #[test]
    fn nesting_is_bounded_and_the_bound_is_reported() {
        let source = |blocks: usize| {
            format!(
                "int main(void) {}{}
",
                "{".repeat(blocks),
                "}".repeat(blocks)
            )
        };

        let at_the_limit = parsed(&source(MAX_NESTING));
        assert_eq!(
            at_the_limit.diagnostics.diagnostics().len(),
            0,
            "{:?}",
            at_the_limit.diagnostics.diagnostics()
        );

        let beyond = parsed(&source(MAX_NESTING + 1));
        let reported = beyond.diagnostics.diagnostics();
        assert_eq!(reported.len(), 1, "{reported:?}");
        assert_eq!(reported[0].message(), "blocks are nested too deeply");
    }

    /// A nested block is one node in the arena, not two.
    ///
    /// `compound` pushes the node and hands back its id, and `statement` passes
    /// that id on. A `statement` that returned the node instead would have its
    /// caller push a second copy, so one pair of braces would have two ids,
    /// both naming the same children. ADR-0008 exists so that an id names one
    /// node, and two ids for one construct is that undone: a side table keyed
    /// by id would have two slots for those braces and no way to say which the
    /// analysis meant.
    ///
    /// The arena is read through `Debug` because the duplicate is unreachable
    /// from the root, so no walk of the tree can see it. That is the whole
    /// difficulty: it is invisible in the artifact and costs a later phase.
    ///
    /// Mutation: have `statement` return `self.ast.stmt(id).clone()` and its
    /// caller push it. This fails with three.
    #[test]
    fn a_nested_block_is_one_node_in_the_arena() {
        let parsed = parsed(
            "int main(void) { { return 0; } }
",
        );

        let arena = format!("{:?}", parsed.ast);
        assert_eq!(arena.matches("Compound").count(), 2, "{arena}");
    }

    /// A node's span reaches the end of what it covers.
    ///
    /// Nothing else in the repository can see this: the artifact prints a start
    /// position only, and every other test matches on shape. What the extent is
    /// for is a later diagnostic underlining a whole statement, which is the
    /// point at which being wrong would be expensive and quiet.
    ///
    /// Mutation: end each span at `start.end()` rather than at
    /// `self.previous().span.end()`. This fails.
    #[test]
    fn a_node_reaches_the_end_of_what_it_covers() {
        let text = "int main(void) { return 0; }
";
        let parsed = parsed(text);

        let [Item::Function(function)] = parsed.ast.items() else {
            panic!("{:?}", parsed.ast.items());
        };
        assert_eq!(&text[function.span.range()], "int main(void) { return 0; }");

        let Stmt::Compound { body, span } = parsed.ast.stmt(function.body) else {
            panic!("{:?}", parsed.ast.stmt(function.body));
        };
        assert_eq!(&text[span.range()], "{ return 0; }");
        assert_eq!(&text[parsed.ast.stmt(body[0]).span().range()], "return 0;");
    }

    /// Each thing the parser insists on says which one was missing.
    ///
    /// Mutation: give every `expect` the same `what`. This fails, because the
    /// messages stop telling the four apart.
    #[test]
    fn what_was_expected_is_named_in_the_message() {
        for (text, expected) in [
            ("int (void) { }\n", "expected a name"),
            ("int main void) { }\n", "expected `(`"),
            ("int main() { }\n", "expected `void`"),
            ("int main(void { }\n", "expected `)`"),
            ("int main(void) return 0; }\n", "expected `{`"),
            ("int main(void) { return 0 }\n", "expected `;`"),
            ("int main(void) { return; \n", "expected `}`"),
            ("int main(void) { return + ; }\n", "expected an expression"),
            ("int main(void) { 0; }\n", "expected a statement"),
        ] {
            let parsed = parsed(text);
            let reported = parsed.diagnostics.diagnostics();

            assert_eq!(reported.len(), 1, "{text:?} gave {reported:?}");
            assert_eq!(reported[0].message(), expected, "{text:?}");
        }
    }
}
