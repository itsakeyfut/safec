//! Tokens to a tree, by recursive descent.
//!
//! What it reads is C17 6.7.6's declarators over `int`, `char` and `void`, the
//! operators of 6.5 from an integer constant or an identifier up to the comma
//! operator, a compound statement and `return`. What it does not read yet is
//! control flow, structs, member access, casts, `sizeof`, the type qualifiers,
//! the storage classes, `typedef`, a variadic function's ellipsis, `[static N]`
//! and `[*]`, and the two primary expressions the lexer already hands it: a
//! character constant and a string literal. Each arrives in a sibling of the
//! issues that built this, and each adds to [`crate::ast`] rather than
//! reshaping it.
//!
//! It is handed tokens and not the source. ADR-0006 reserved a trigger, that
//! the parser asking for a `&SourceMap` is the moment an interner is worth
//! having, and this does not fire it: a name is carried as a span, resolving
//! one to text is the printer's job, and comparing one name with another is the
//! semantic analysis's.
//!
//! [`docs/frontend.md`]: https://github.com/itsakeyfut/safec/blob/main/docs/frontend.md

use crate::ast::{
    Ast, BinOp, Declaration, Expr, ExprId, Function, Item, Parameters, Stmt, StmtId, Type, TypeId,
    UnOp,
};
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

/// How deep this parser will go before it declines.
///
/// One number for every kind of nesting, because there is one thing being
/// protected. The alternative to a limit is not "no limit": it is the native
/// stack, which this parser recurses on and which ends a run by killing the
/// process, with no diagnostic and an exit code the driver never chose. A
/// stack overflow is not a panic anything can catch, so the limit has to be
/// here rather than in a test. A limit that is reported is the same refusal
/// made legible.
///
/// What counts is a recursion and not a bracket. Blocks and brackets are the
/// obvious sources, and `clang` bounds those with `-fbracket-depth`, whose
/// default is the same 256. It counts each kind separately, not together: 200
/// nested `[` and 200 nested `(` in one expression is 400 open at once and
/// `clang 20.1.6` accepts it, while 300 of either alone is `fatal error:
/// bracket nesting level exceeded maximum of 256`. One counter here rather
/// than three, because what is being protected is one stack.
///
/// Brackets are also not the only source. `!!!!x` recurses once per operator,
/// and so does `a = b = c`, because assignment is right-associative and its
/// right side is read by re-entering the climb. Neither writes a bracket, and
/// `clang` bounds neither. So every recursion in this file goes through
/// [`Parser::deeper`], which is the one place this is counted, and the limit is
/// tighter than `clang`'s for shapes `clang` does not count at all.
///
/// C17 5.2.4.1 asks an implementation to manage "127 nesting levels of blocks",
/// "63 nesting levels of parenthesized expressions within a full expression"
/// and "63 nesting levels of parenthesized declarators within a full
/// declarator" and "12 pointer, array, and function declarators (in any
/// combinations) modifying an arithmetic, structure, union, or void type in a
/// declaration". This clears all four. What it sets no minimum for is a chain
/// of unary or assignment operators, which is the only room this takes beyond
/// them.
///
/// **This bounds the parser, not the tree.** A left-associative chain and a run
/// of postfix operators are folded by a loop, so `a + a + ...` and `a++++` are
/// as deep in the tree as they are long while `depth` never rises. Anything
/// that walks the tree owes itself an answer to that; `dump_expr` in
/// `driver.rs` uses an explicit stack, and says so.
const MAX_NESTING: usize = 256;

/// The loosest binding there is: the comma operator, C17 6.5.17.
///
/// Named because the climb also starts from it, and the same number written in
/// two places is a second thing to keep in agreement.
const COMMA: u8 = 1;

/// One step tighter: assignment, C17 6.5.16.
///
/// Named because an argument in a call is read at this power and so stops at a
/// comma. That is the whole of what tells `f(a, b)` from `f((a, b))`.
const ASSIGNMENT: u8 = 2;

/// One step a declarator derives, in the order it wraps the base type.
///
/// Collected rather than applied while reading. In `T (D)(params)` the inner
/// `D`'s base is `T` modified by the suffixes that follow the closing
/// parenthesis, and those have not been read when `D` is. Collecting lets the
/// fold happen once, at the end, in the order C17 6.7.6 p4 to p6 derive.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Derivation {
    /// C17 6.7.6.1.
    Pointer,
    /// C17 6.7.6.2.
    Array(Option<ExprId>),
    /// C17 6.7.6.3.
    Function(Parameters),
}

/// The type a declaration specifier names, if this token is one.
///
/// One place, asked from three. `specifiers` asks and consumes;
/// `parenthesised_declarator` asks without consuming, to tell a parenthesised
/// declarator from a parameter list; `compound` asks without consuming, to tell
/// C17 6.8.2's two kinds of block-item apart. All three have to agree about
/// what begins a declaration, and a second copy of the answer is what would let
/// them stop agreeing, which is the shape RK-003 records the cost of.
///
/// `docs/frontend.md` gives Stage 1 `int`, `char` and `void`. C's other
/// specifiers, its qualifiers and its storage classes are later stages, and
/// the module comment lists them among what is not read yet.
fn specifier(kind: TokenKind) -> Option<Type> {
    match kind {
        TokenKind::Keyword(Keyword::Int) => Some(Type::Int),
        TokenKind::Keyword(Keyword::Char) => Some(Type::Char),
        TokenKind::Keyword(Keyword::Void) => Some(Type::Void),
        _ => None,
    }
}

/// Which of two operators of equal power takes its operand first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Assoc {
    /// `a - b - c` is `(a - b) - c`.
    Left,
    /// `a = b = c` is `a = (b = c)`.
    Right,
}

/// What reading an infix operator makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InfixKind {
    /// Two operands and the operator between them.
    Binary(BinOp),
    /// An assignment, with the operation folded in or `None` for a plain `=`.
    Assign(Option<BinOp>),
    /// `?`, which reads a whole expression and then insists on `:`.
    Conditional,
    /// The comma operator.
    Comma,
}

/// How tightly an infix operator binds, which way it groups, and what it makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Infix {
    /// Higher binds tighter. [`COMMA`] is the loosest.
    power: u8,
    /// What to do about two of the same power in a row.
    assoc: Assoc,
    /// Which node comes out of it.
    kind: InfixKind,
}

/// The table C17 6.5 gives, from the loosest binding to the tightest.
///
/// One table and not two. A binding power in one place and a node kind in
/// another have to agree, and nothing would notice `a - b` arriving at
/// `BinOp::Add` with the right precedence. RK-003 in the review knowledge bank
/// is what a second copy of one decision has already cost here.
///
/// `?` is in it, because the conditional binds like an infix operator even
/// though it reads a whole expression before its `:`. `:` is not, and neither
/// are `++` and `--`: those are read where the operand they attach to is.
fn infix(punct: Punct) -> Option<Infix> {
    use Assoc::{Left, Right};
    use InfixKind::{Assign, Binary, Comma, Conditional};

    let (power, assoc, kind) = match punct {
        // 6.5.17
        Punct::Comma => (COMMA, Left, Comma),

        // 6.5.16
        Punct::Equal => (ASSIGNMENT, Right, Assign(None)),
        Punct::StarEqual => (ASSIGNMENT, Right, Assign(Some(BinOp::Mul))),
        Punct::SlashEqual => (ASSIGNMENT, Right, Assign(Some(BinOp::Div))),
        Punct::PercentEqual => (ASSIGNMENT, Right, Assign(Some(BinOp::Rem))),
        Punct::PlusEqual => (ASSIGNMENT, Right, Assign(Some(BinOp::Add))),
        Punct::MinusEqual => (ASSIGNMENT, Right, Assign(Some(BinOp::Sub))),
        Punct::LessLessEqual => (ASSIGNMENT, Right, Assign(Some(BinOp::Shl))),
        Punct::GreaterGreaterEqual => (ASSIGNMENT, Right, Assign(Some(BinOp::Shr))),
        Punct::AmpersandEqual => (ASSIGNMENT, Right, Assign(Some(BinOp::BitAnd))),
        Punct::CaretEqual => (ASSIGNMENT, Right, Assign(Some(BinOp::BitXor))),
        Punct::PipeEqual => (ASSIGNMENT, Right, Assign(Some(BinOp::BitOr))),

        // 6.5.15
        Punct::Question => (3, Right, Conditional),

        // 6.5.14 down to 6.5.5
        Punct::PipePipe => (4, Left, Binary(BinOp::LogOr)),
        Punct::AmpersandAmpersand => (5, Left, Binary(BinOp::LogAnd)),
        Punct::Pipe => (6, Left, Binary(BinOp::BitOr)),
        Punct::Caret => (7, Left, Binary(BinOp::BitXor)),
        Punct::Ampersand => (8, Left, Binary(BinOp::BitAnd)),
        Punct::EqualEqual => (9, Left, Binary(BinOp::Eq)),
        Punct::BangEqual => (9, Left, Binary(BinOp::Ne)),
        Punct::Less => (10, Left, Binary(BinOp::Lt)),
        Punct::Greater => (10, Left, Binary(BinOp::Gt)),
        Punct::LessEqual => (10, Left, Binary(BinOp::Le)),
        Punct::GreaterEqual => (10, Left, Binary(BinOp::Ge)),
        Punct::LessLess => (11, Left, Binary(BinOp::Shl)),
        Punct::GreaterGreater => (11, Left, Binary(BinOp::Shr)),
        Punct::Plus => (12, Left, Binary(BinOp::Add)),
        Punct::Minus => (12, Left, Binary(BinOp::Sub)),
        Punct::Star => (13, Left, Binary(BinOp::Mul)),
        Punct::Slash => (13, Left, Binary(BinOp::Div)),
        Punct::Percent => (13, Left, Binary(BinOp::Rem)),

        _ => return None,
    };

    Some(Infix { power, assoc, kind })
}

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
    /// How many recursive reads are in progress right now.
    ///
    /// Bounded by [`MAX_NESTING`] and changed only by [`Parser::deeper`], which
    /// is what stops any of this file's recursions reaching the end of the
    /// native stack.
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

    /// Read something one level deeper, or say there is no room for it.
    ///
    /// Every recursion in this file goes through here. [`MAX_NESTING`] says why
    /// the count is one number rather than one per kind of nesting, and why a
    /// bracket is not what it counts.
    ///
    /// A pair of closures rather than an `enter` and a `leave`, so that a level
    /// cannot be taken and not given back. A rule that has to be remembered at
    /// each of several sites, some of which return early, is a rule with an
    /// exception waiting in it.
    fn deeper<T>(
        &mut self,
        diagnostics: &mut DiagnosticSink,
        read: impl FnOnce(&mut Self, &mut DiagnosticSink) -> T,
        too_deep: impl FnOnce(&mut Self, Span) -> T,
    ) -> T {
        if self.depth == MAX_NESTING {
            let span = self.report_as(
                TOO_DEEP,
                "nesting is too deep",
                format!("{MAX_NESTING} levels are already open here"),
                diagnostics,
            );
            return too_deep(self, span);
        }

        self.depth += 1;
        let read = read(self, diagnostics);
        self.depth -= 1;
        read
    }

    /// A declaration or a function definition, which C17 6.9 makes the two
    /// things a translation unit is a sequence of.
    ///
    /// The two are read the same way until the declarator ends. A `{` after it
    /// is a definition and a `;` is a declaration, which is the only thing that
    /// tells them apart and the only thing this decides. Whether the declarator
    /// of a definition derived a function type is 6.9.1 p2's constraint, and a
    /// constraint is checked later.
    fn item(&mut self, diagnostics: &mut DiagnosticSink) -> Item {
        let start = self.peek().span;

        let Some((name, ty)) = self.declared(diagnostics) else {
            return Item::Error { span: start };
        };

        if self.check(TokenKind::Punct(Punct::LeftBrace)) {
            let Some(body) = self.compound(diagnostics) else {
                return Item::Error { span: start };
            };

            let span = Span::new(self.file, start.start(), self.previous().span.end());
            return Item::Function(Function {
                ty,
                name,
                body,
                span,
            });
        }

        if self
            .expect(TokenKind::Punct(Punct::Semicolon), "`;`", diagnostics)
            .is_none()
        {
            return Item::Error { span: start };
        }

        let span = Span::new(self.file, start.start(), self.previous().span.end());
        Item::Declaration(Declaration {
            name: Some(name),
            ty,
            span,
        })
    }

    /// The specifiers and one declarator, which is how a declaration and a
    /// definition both begin.
    fn declared(&mut self, diagnostics: &mut DiagnosticSink) -> Option<(Span, TypeId)> {
        let start = self.peek().span;
        let base = self.specifiers(diagnostics)?;
        let (name, derivations) = self.declarator(true, diagnostics)?;
        let ty = self.apply(start, base, derivations, diagnostics)?;

        let Some(name) = name else {
            // `declarator` was asked for a name and reports before it returns
            // without one, so nothing reaches here today. It reports rather
            // than falling silent because of what falling silent would cost: a
            // `None` nobody spoke for leaves `failed` clear, so the run carries
            // on and can reach the end of the file having put an error node in
            // the artifact and exited zero.
            self.report("expected a name", "a name is missing here", diagnostics);
            return None;
        };

        Some((name, ty))
    }

    /// The declaration specifiers, of which this stage reads one.
    fn specifiers(&mut self, diagnostics: &mut DiagnosticSink) -> Option<TypeId> {
        let Some(ty) = specifier(self.peek().kind) else {
            self.report(
                "expected a declaration",
                "this cannot begin one",
                diagnostics,
            );
            return None;
        };

        self.advance();
        Some(self.ast.push_type(ty))
    }

    /// A declarator, and the derivations it applies to the base type.
    ///
    /// C17 6.7.6 p4 sets up the notation this follows: in a declaration `T D1`,
    /// `T` is the base and `D1` derives the identifier's type from it. p5 makes
    /// a bare identifier the base itself; 6.7.6.1 p1, 6.7.6.2 p3 and 6.7.6.3 p5
    /// each say that a derivation wraps the base and hands the result to the
    /// declarator *inside* it. So the derivations are collected outermost-first
    /// and folded onto the base in that order.
    ///
    /// `named` is the difference between 6.7.6's `declarator`, which has an
    /// identifier, and 6.7.7's `abstract-declarator`, which has none. A
    /// parameter may write either.
    fn declarator(
        &mut self,
        named: bool,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<(Option<Span>, Vec<Derivation>)> {
        let mut derivations = Vec::new();

        while self.check(TokenKind::Punct(Punct::Star)) {
            self.spend();
            self.advance();
            derivations.push(Derivation::Pointer);
        }

        let (name, inner) = self.core(named, diagnostics)?;

        // The suffix written last is the first to wrap the base, because
        // 6.7.6.2 p3 reads `D [ ... ]` by handing "array of T" to `D`. That is
        // what makes `int a[3][5]` three arrays of five and not five of three.
        let mut suffixes = self.suffixes(diagnostics)?;
        suffixes.reverse();
        derivations.append(&mut suffixes);

        derivations.extend(inner);
        Some((name, derivations))
    }

    /// A direct-declarator's core: an identifier, a parenthesised declarator,
    /// or nothing where 6.7.7 allows nothing.
    fn core(
        &mut self,
        named: bool,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<(Option<Span>, Vec<Derivation>)> {
        if self.check(TokenKind::Identifier) {
            return Some((Some(self.advance().span), Vec::new()));
        }

        if self.check(TokenKind::Punct(Punct::LeftParen)) && self.parenthesised_declarator() {
            self.advance();

            let inner = self.deeper(
                diagnostics,
                |parser, diagnostics| parser.declarator(named, diagnostics),
                |_, _| None,
            )?;

            self.expect(TokenKind::Punct(Punct::RightParen), "`)`", diagnostics)?;
            return Some(inner);
        }

        if named {
            self.report("expected a name", "a name is missing here", diagnostics);
            return None;
        }

        Some((None, Vec::new()))
    }

    /// Whether the `(` that is there opens a parenthesised declarator rather
    /// than a parameter list.
    ///
    /// Both are written `(` in the same position: C17 6.7.6's
    /// `direct-declarator` has `( declarator )`, and 6.7.7's
    /// `direct-abstract-declarator` has `( parameter-type-list_opt )` with
    /// nothing before it. What follows tells them apart, because a parameter
    /// list either ends at once or begins with a declaration specifier, and a
    /// declarator begins with `*`, `(` or a name. `typedef` is what makes this
    /// genuinely ambiguous in full C, and `typedef` is a later stage.
    fn parenthesised_declarator(&self) -> bool {
        let after = self.peek_at(1).kind;
        specifier(after).is_none() && after != TokenKind::Punct(Punct::RightParen)
    }

    /// The `[...]` and `(...)` that follow a direct-declarator, in source order.
    fn suffixes(&mut self, diagnostics: &mut DiagnosticSink) -> Option<Vec<Derivation>> {
        let mut suffixes = Vec::new();

        loop {
            if self.check(TokenKind::Punct(Punct::LeftBracket)) {
                self.spend();
                self.advance();

                // 6.7.6 spells what goes here `assignment-expression` and not
                // `expression`, so a comma ends the subscript rather than being
                // an operator inside it.
                let length = if self.check(TokenKind::Punct(Punct::RightBracket)) {
                    None
                } else {
                    Some(self.assignment(diagnostics))
                };

                self.expect(TokenKind::Punct(Punct::RightBracket), "`]`", diagnostics)?;
                suffixes.push(Derivation::Array(length));
            } else if self.check(TokenKind::Punct(Punct::LeftParen)) {
                self.spend();
                self.advance();

                let parameters = self.parameters(diagnostics)?;

                self.expect(TokenKind::Punct(Punct::RightParen), "`)`", diagnostics)?;
                suffixes.push(Derivation::Function(parameters));
            } else {
                break;
            }
        }

        Some(suffixes)
    }

    /// What a function declarator wrote between its parentheses.
    fn parameters(&mut self, diagnostics: &mut DiagnosticSink) -> Option<Parameters> {
        if self.check(TokenKind::Punct(Punct::RightParen)) {
            return Some(Parameters::Unspecified);
        }

        // A parameter's own type nests inside this one's, so the list takes a
        // level. See `MAX_NESTING`.
        self.deeper(
            diagnostics,
            |parser, diagnostics| parser.parameter_list(diagnostics),
            |_, _| None,
        )
    }

    fn parameter_list(&mut self, diagnostics: &mut DiagnosticSink) -> Option<Parameters> {
        let mut parameters = Vec::new();

        loop {
            let start = self.peek().span;
            let base = self.specifiers(diagnostics)?;
            let (name, derivations) = self.declarator(false, diagnostics)?;
            let ty = self.apply(start, base, derivations, diagnostics)?;

            let span = Span::new(self.file, start.start(), self.previous().span.end());
            parameters.push(Declaration { name, ty, span });

            if !self.eat(TokenKind::Punct(Punct::Comma)) {
                break;
            }
            self.spend();
        }

        // 6.7.6.3 p10: "The special case of an unnamed parameter of type void
        // as the only item in the list specifies that the function has no
        // parameters." So `(void)` is the empty prototype, and `()`, which p14
        // gives a different meaning, is answered above.
        if parameters.len() == 1
            && parameters[0].name.is_none()
            && matches!(self.ast.ty(parameters[0].ty), Type::Void)
        {
            return Some(Parameters::Prototype(Vec::new()));
        }

        Some(Parameters::Prototype(parameters))
    }

    /// Fold the derivations a declarator collected onto the base type.
    ///
    /// The one place a type's depth is decided, so the one place it is bounded.
    /// C17 6.7.6 p7 permits that:
    ///
    /// > As discussed in 5.2.4.1, an implementation may limit the number of
    /// > pointer, array, and function declarators that modify an arithmetic,
    /// > structure, union, or void type, either directly or via one or more
    /// > `typedef`s.
    ///
    /// It is load-bearing for every phase that walks a type, not for the
    /// printer alone. The pointer loop and the suffix loop fold without
    /// recursing, so a declarator can build a type far deeper than the parser
    /// ever went, which is exactly the shape that killed the expression printer
    /// once already. Bounding it here means no walker has to ask.
    ///
    /// 5.2.4.1's own limits sit far below [`MAX_NESTING`]: it asks an
    /// implementation to manage "63 nesting levels of parenthesized declarators
    /// within a full declarator".
    fn apply(
        &mut self,
        start: Span,
        base: TypeId,
        derivations: Vec<Derivation>,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<TypeId> {
        if derivations.len() > MAX_NESTING {
            // At the declarator, not at whatever follows it. The count is only
            // known once the whole declarator has been read, so reporting where
            // the parser is standing would put the caret on the `;` after it,
            // which is a byte with nothing wrong with it.
            self.report_at(
                start,
                TOO_DEEP,
                "nesting is too deep",
                format!("this declarator derives more than {MAX_NESTING} types"),
                diagnostics,
            );
            return None;
        }

        let mut ty = base;
        for derivation in derivations {
            ty = self.ast.push_type(match derivation {
                Derivation::Pointer => Type::Pointer(ty),
                Derivation::Array(length) => Type::Array {
                    element: ty,
                    length,
                },
                Derivation::Function(parameters) => Type::Function {
                    returns: ty,
                    parameters,
                },
            });
        }

        Some(ty)
    }

    /// A braced sequence of statements.
    fn compound(&mut self, diagnostics: &mut DiagnosticSink) -> Option<StmtId> {
        let start = self.peek().span;
        self.expect(TokenKind::Punct(Punct::LeftBrace), "`{`", diagnostics)?;

        // The one place a block is counted, so the body of a function and a
        // block inside it are the same thing to `depth`.
        let body = self.deeper(
            diagnostics,
            |parser, diagnostics| {
                let mut body = Vec::new();
                while !parser.at_end()
                    && !parser.failed
                    && !parser.check(TokenKind::Punct(Punct::RightBrace))
                {
                    parser.spend();

                    // C17 6.8.2 makes a block-item a declaration or a
                    // statement, and a declaration is not a statement in C.
                    // Which of the two is here is decided by the same
                    // `specifier` the declaration reads with.
                    let item = if specifier(parser.peek().kind).is_some() {
                        parser.declaration_statement(diagnostics)
                    } else {
                        parser.statement(diagnostics)
                    };
                    body.push(item);
                }
                Some(body)
            },
            |_, _| None,
        )?;

        self.expect(TokenKind::Punct(Punct::RightBrace), "`}`", diagnostics)?;

        let span = Span::new(self.file, start.start(), self.previous().span.end());
        Some(self.ast.push_stmt(Stmt::Compound { body, span }))
    }

    /// A declaration where C17 6.8.2 allows one instead of a statement.
    fn declaration_statement(&mut self, diagnostics: &mut DiagnosticSink) -> StmtId {
        let start = self.peek().span;

        let Some((name, ty)) = self.declared(diagnostics) else {
            return self.ast.push_stmt(Stmt::Error { span: start });
        };

        if self
            .expect(TokenKind::Punct(Punct::Semicolon), "`;`", diagnostics)
            .is_none()
        {
            return self.ast.push_stmt(Stmt::Error { span: start });
        }

        let span = Span::new(self.file, start.start(), self.previous().span.end());
        self.ast.push_stmt(Stmt::Declaration(Declaration {
            name: Some(name),
            ty,
            span,
        }))
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
            return match self.compound(diagnostics) {
                Some(id) => id,
                None => self.ast.push_stmt(Stmt::Error { span: start }),
            };
        }

        if self.eat(TokenKind::Keyword(Keyword::Return)) {
            let value = if self.check(TokenKind::Punct(Punct::Semicolon)) {
                None
            } else {
                Some(self.expression(diagnostics))
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

        if self.check(TokenKind::Keyword(Keyword::If)) {
            return self.if_statement(start, diagnostics);
        }

        if self.check(TokenKind::Keyword(Keyword::While)) {
            return self.while_statement(start, diagnostics);
        }

        if self.check(TokenKind::Keyword(Keyword::For)) {
            return self.for_statement(start, diagnostics);
        }

        // What is left is C17 6.8.3's `expression_opt ;`, which is also where a
        // token that begins nothing at all ends up. There is no test here for
        // whether an expression can begin: asking would be a second copy of
        // what `primary` already knows, and `primary` reports for itself.
        // `clang` reaches the same place and says the same thing.
        self.expression_statement(start, diagnostics)
    }

    /// `expression_opt ;`, C17 6.8.3 p1.
    ///
    /// The null statement is this with nothing in it. p3 calls it "a null
    /// statement (consisting of just a semicolon)" and gives it no production,
    /// so it gets no node of its own either.
    fn expression_statement(&mut self, start: Span, diagnostics: &mut DiagnosticSink) -> StmtId {
        let value = if self.check(TokenKind::Punct(Punct::Semicolon)) {
            None
        } else {
            Some(self.expression(diagnostics))
        };

        if self
            .expect(TokenKind::Punct(Punct::Semicolon), "`;`", diagnostics)
            .is_none()
        {
            return self.ast.push_stmt(Stmt::Error { span: start });
        }

        let span = Span::new(self.file, start.start(), self.previous().span.end());
        self.ast.push_stmt(Stmt::Expression { value, span })
    }

    /// `if ( expression ) statement`, with or without `else`. C17 6.8.4 p1.
    ///
    /// **The `else` binds inward, and that is not an accident.** 6.8.4.1 p3
    /// says "an else is associated with the lexically nearest preceding if that
    /// is allowed by the syntax", and taking the `else` here, in the innermost
    /// `if` still being read, is what makes it so. A recursive descent gets the
    /// rule for free; breaking it would take a flag telling the inner `if` to
    /// leave the `else` alone.
    fn if_statement(&mut self, start: Span, diagnostics: &mut DiagnosticSink) -> StmtId {
        self.advance();

        let Some(condition) = self.controlling(diagnostics) else {
            return self.ast.push_stmt(Stmt::Error { span: start });
        };

        // Both substatements nest inside this one, so this takes a level. A
        // chain of `else if` is a chain of these, because `else if` is not a
        // construct in C: it is `else` followed by an `if` statement, so it
        // takes one level per link and writes no bracket at all. See
        // `MAX_NESTING`.
        self.deeper(
            diagnostics,
            |parser, diagnostics| {
                let then = parser.statement(diagnostics);

                let otherwise = if parser.eat(TokenKind::Keyword(Keyword::Else)) {
                    Some(parser.statement(diagnostics))
                } else {
                    None
                };

                let span = Span::new(parser.file, start.start(), parser.previous().span.end());
                parser.ast.push_stmt(Stmt::If {
                    condition,
                    then,
                    otherwise,
                    span,
                })
            },
            |parser, span| parser.ast.push_stmt(Stmt::Error { span }),
        )
    }

    /// `while ( expression ) statement`. C17 6.8.5 p1.
    fn while_statement(&mut self, start: Span, diagnostics: &mut DiagnosticSink) -> StmtId {
        self.advance();

        let Some(condition) = self.controlling(diagnostics) else {
            return self.ast.push_stmt(Stmt::Error { span: start });
        };

        self.deeper(
            diagnostics,
            |parser, diagnostics| {
                let body = parser.statement(diagnostics);
                let span = Span::new(parser.file, start.start(), parser.previous().span.end());
                parser.ast.push_stmt(Stmt::While {
                    condition,
                    body,
                    span,
                })
            },
            |parser, span| parser.ast.push_stmt(Stmt::Error { span }),
        )
    }

    /// `for ( expression_opt ; expression_opt ; expression_opt ) statement`.
    /// C17 6.8.5 p1.
    ///
    /// The other form the same paragraph gives, `for ( declaration
    /// expression_opt ; expression_opt )`, needs an initializer and so needs
    /// the issue that reads one. Until then `for (int i = 0; ...)` is refused.
    fn for_statement(&mut self, start: Span, diagnostics: &mut DiagnosticSink) -> StmtId {
        self.advance();

        let failed = |parser: &mut Self| parser.ast.push_stmt(Stmt::Error { span: start });

        if self
            .expect(TokenKind::Punct(Punct::LeftParen), "`(`", diagnostics)
            .is_none()
        {
            return failed(self);
        }

        let Some(initialiser) = self.clause(TokenKind::Punct(Punct::Semicolon), diagnostics) else {
            return failed(self);
        };
        let Some(condition) = self.clause(TokenKind::Punct(Punct::Semicolon), diagnostics) else {
            return failed(self);
        };
        let Some(step) = self.clause(TokenKind::Punct(Punct::RightParen), diagnostics) else {
            return failed(self);
        };

        self.deeper(
            diagnostics,
            |parser, diagnostics| {
                let body = parser.statement(diagnostics);
                let span = Span::new(parser.file, start.start(), parser.previous().span.end());
                parser.ast.push_stmt(Stmt::For {
                    initialiser,
                    condition,
                    step,
                    body,
                    span,
                })
            },
            |parser, span| parser.ast.push_stmt(Stmt::Error { span }),
        )
    }

    /// One of `for`'s three clauses, and the token that ends it.
    ///
    /// `Some(None)` is a clause that was left out, which 6.8.5 p1 allows for
    /// all three. `None` is the closing token missing.
    fn clause(
        &mut self,
        end: TokenKind,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<Option<ExprId>> {
        let clause = if self.check(end) {
            None
        } else {
            Some(self.expression(diagnostics))
        };

        let what = if end == TokenKind::Punct(Punct::Semicolon) {
            "`;`"
        } else {
            "`)`"
        };
        self.expect(end, what, diagnostics)?;

        Some(clause)
    }

    /// The parenthesised expression `if` and `while` are controlled by.
    ///
    /// 6.8.4 p1 and 6.8.5 p1 both spell it `expression` and not
    /// `assignment-expression`, so the comma operator is in it: `while (a, b)`
    /// reads both and is controlled by the second.
    fn controlling(&mut self, diagnostics: &mut DiagnosticSink) -> Option<ExprId> {
        self.expect(TokenKind::Punct(Punct::LeftParen), "`(`", diagnostics)?;
        let condition = self.expression(diagnostics);
        self.expect(TokenKind::Punct(Punct::RightParen), "`)`", diagnostics)?;

        Some(condition)
    }

    /// The whole of C17 6.5, comma operator included.
    fn expression(&mut self, diagnostics: &mut DiagnosticSink) -> ExprId {
        self.infix_from(COMMA, diagnostics)
    }

    /// An expression up to but not including the comma operator.
    ///
    /// What an argument in a call is, per C17 6.5.2's `argument-expression-list`.
    fn assignment(&mut self, diagnostics: &mut DiagnosticSink) -> ExprId {
        self.infix_from(ASSIGNMENT, diagnostics)
    }

    /// Read an expression, taking only operators that bind at least as tightly
    /// as `min`.
    ///
    /// Precedence climbing over [`infix`], which is the shape `clang` uses
    /// (`ParseRHSOfBinaryExpression` over `getBinOpPrec`). A function per level
    /// would read closer to C's own grammar and would spend a stack frame per
    /// level on every expression, however shallow; this spends one per operator
    /// that actually nests.
    ///
    /// This is a recursion, so it takes a level. What nests here is not only
    /// `( )`: a right-associative operator reads its right side by re-entering,
    /// so `a = b = c` goes one deeper per `=` with no bracket written.
    fn infix_from(&mut self, min: u8, diagnostics: &mut DiagnosticSink) -> ExprId {
        self.deeper(
            diagnostics,
            |parser, diagnostics| parser.climb(min, diagnostics),
            |parser, span| parser.ast.push_expr(Expr::Error { span }),
        )
    }

    fn climb(&mut self, min: u8, diagnostics: &mut DiagnosticSink) -> ExprId {
        let mut lhs = self.unary(diagnostics);

        while !self.failed {
            let TokenKind::Punct(punct) = self.peek().kind else {
                break;
            };
            let Some(operator) = infix(punct) else { break };
            if operator.power < min {
                break;
            }

            // Spent here rather than at the top of the loop, because this is
            // the point past which the operator is certainly consumed. A loop
            // that spends on the round it breaks out of would run the budget
            // down without the parser having moved.
            self.spend();
            self.advance();

            // Left-associative means the right side stops at the next operator
            // of the same power, so that it joins the left side instead.
            let next = match operator.assoc {
                Assoc::Left => operator.power + 1,
                Assoc::Right => operator.power,
            };

            lhs = match operator.kind {
                InfixKind::Binary(op) => {
                    let rhs = self.infix_from(next, diagnostics);
                    let span = self.joined(lhs, rhs);
                    self.ast.push_expr(Expr::Binary { op, lhs, rhs, span })
                }
                InfixKind::Assign(op) => {
                    let value = self.infix_from(next, diagnostics);
                    let span = self.joined(lhs, value);
                    self.ast.push_expr(Expr::Assign {
                        op,
                        place: lhs,
                        value,
                        span,
                    })
                }
                InfixKind::Comma => {
                    let rhs = self.infix_from(next, diagnostics);
                    let span = self.joined(lhs, rhs);
                    self.ast.push_expr(Expr::Comma { lhs, rhs, span })
                }
                InfixKind::Conditional => {
                    // C17 6.5.15 puts a whole expression between `?` and `:`,
                    // the comma operator included, because the `:` is what ends
                    // it. Only the third operand is read at this power.
                    let then = self.expression(diagnostics);

                    if self
                        .expect(TokenKind::Punct(Punct::Colon), "`:`", diagnostics)
                        .is_none()
                    {
                        let span = self.ast.expr(then).span();
                        self.ast.push_expr(Expr::Error { span })
                    } else {
                        let otherwise = self.infix_from(next, diagnostics);
                        let span = self.joined(lhs, otherwise);
                        self.ast.push_expr(Expr::Conditional {
                            condition: lhs,
                            then,
                            otherwise,
                            span,
                        })
                    }
                }
            };
        }

        lhs
    }

    /// A prefix operator and its operand, or a postfix expression. C17 6.5.3.
    fn unary(&mut self, diagnostics: &mut DiagnosticSink) -> ExprId {
        let op = match self.peek().kind {
            TokenKind::Punct(Punct::Plus) => UnOp::Plus,
            TokenKind::Punct(Punct::Minus) => UnOp::Minus,
            TokenKind::Punct(Punct::Bang) => UnOp::Not,
            TokenKind::Punct(Punct::Tilde) => UnOp::BitNot,
            TokenKind::Punct(Punct::Star) => UnOp::Deref,
            TokenKind::Punct(Punct::Ampersand) => UnOp::AddrOf,
            TokenKind::Punct(Punct::PlusPlus) => UnOp::PreInc,
            TokenKind::Punct(Punct::MinusMinus) => UnOp::PreDec,
            _ => return self.postfix(diagnostics),
        };

        let start = self.advance().span;

        // 6.5.3 makes the operand a unary-expression, so `- -x` is read here
        // and not by the climb. Nothing brackets it, which is why it takes a
        // level of its own.
        self.deeper(
            diagnostics,
            |parser, diagnostics| {
                let operand = parser.unary(diagnostics);
                let span = Span::new(parser.file, start.start(), parser.previous().span.end());
                parser.ast.push_expr(Expr::Unary { op, operand, span })
            },
            |parser, span| parser.ast.push_expr(Expr::Error { span }),
        )
    }

    /// A primary expression with whatever follows it. C17 6.5.2.
    ///
    /// Member access is not here yet, and neither are casts or `sizeof`.
    fn postfix(&mut self, diagnostics: &mut DiagnosticSink) -> ExprId {
        let mut base = self.primary(diagnostics);

        while !self.failed {
            let TokenKind::Punct(punct) = self.peek().kind else {
                break;
            };

            // Every arm consumes the punctuator it matched, so the budget is
            // spent only where the loop moves.
            base = match punct {
                Punct::LeftParen => {
                    self.spend();
                    self.call(base, diagnostics)
                }
                Punct::LeftBracket => {
                    self.spend();
                    self.subscript(base, diagnostics)
                }
                Punct::PlusPlus | Punct::MinusMinus => {
                    self.spend();
                    let op = if punct == Punct::PlusPlus {
                        UnOp::PostInc
                    } else {
                        UnOp::PostDec
                    };
                    let end = self.advance().span;
                    let span = Span::new(self.file, self.ast.expr(base).span().start(), end.end());
                    self.ast.push_expr(Expr::Unary {
                        op,
                        operand: base,
                        span,
                    })
                }
                _ => break,
            };
        }

        base
    }

    /// The arguments of a call, from the `(` that is there to the `)`.
    ///
    /// Each argument is an assignment-expression, which is the whole of what
    /// keeps a separator from being the comma operator: `f(a, b)` is two
    /// arguments and `f((a, b))` is one.
    fn call(&mut self, callee: ExprId, diagnostics: &mut DiagnosticSink) -> ExprId {
        let start = self.ast.expr(callee).span();
        self.advance();

        let mut arguments = Vec::new();
        if !self.check(TokenKind::Punct(Punct::RightParen)) {
            arguments.push(self.assignment(diagnostics));
            while !self.failed && self.eat(TokenKind::Punct(Punct::Comma)) {
                self.spend();
                arguments.push(self.assignment(diagnostics));
            }
        }

        if self
            .expect(TokenKind::Punct(Punct::RightParen), "`)`", diagnostics)
            .is_none()
        {
            return self.ast.push_expr(Expr::Error { span: start });
        }

        let span = Span::new(self.file, start.start(), self.previous().span.end());
        self.ast.push_expr(Expr::Call {
            callee,
            arguments,
            span,
        })
    }

    /// `base[index]`, from the `[` that is there to the `]`.
    fn subscript(&mut self, base: ExprId, diagnostics: &mut DiagnosticSink) -> ExprId {
        let start = self.ast.expr(base).span();
        self.advance();

        let index = self.expression(diagnostics);

        if self
            .expect(TokenKind::Punct(Punct::RightBracket), "`]`", diagnostics)
            .is_none()
        {
            return self.ast.push_expr(Expr::Error { span: start });
        }

        let span = Span::new(self.file, start.start(), self.previous().span.end());
        self.ast.push_expr(Expr::Subscript { base, index, span })
    }

    /// Part of C17 6.5.1: an integer constant, an identifier, or a
    /// parenthesised expression.
    ///
    /// Not all of it. A character constant is a constant by 6.4.4 and a string
    /// literal is a primary expression by 6.5.1, the lexer already gives both
    /// their own token, and neither is read here. Both are refused with
    /// `expected an expression`, which is the wording #36 exists to fix.
    fn primary(&mut self, diagnostics: &mut DiagnosticSink) -> ExprId {
        let span = self.peek().span;

        if self.eat(TokenKind::Number) {
            return self.ast.push_expr(Expr::Number { span });
        }

        if self.eat(TokenKind::Identifier) {
            return self.ast.push_expr(Expr::Identifier { span });
        }

        if self.eat(TokenKind::Punct(Punct::LeftParen)) {
            // Parentheses make no node. The grouping is already the shape of
            // the tree, and a node for them is one every later phase would have
            // to see through.
            //
            // What that gives up is visible in the artifact: the expression
            // that comes back keeps its own span, so `(a + b) * c` has its
            // multiplication starting at the `a` and not at the `(`. Widening
            // the inner node's span instead would make a node's span stop
            // meaning "this operator and its operands", which is the one thing
            // every span here does mean. Writing the source back out and
            // saying that a pair of parentheses is redundant are the other two
            // things given up, and none of the three is asked for.
            let inner = self.expression(diagnostics);

            if self
                .expect(TokenKind::Punct(Punct::RightParen), "`)`", diagnostics)
                .is_none()
            {
                return self.ast.push_expr(Expr::Error { span });
            }

            return inner;
        }

        let span = self.report(
            "expected an expression",
            "this cannot begin one",
            diagnostics,
        );
        self.ast.push_expr(Expr::Error { span })
    }

    /// The smallest span covering two nodes.
    ///
    /// Through [`Span::to`] rather than by building one from the two offsets,
    /// because that is where the assertion lives that the two spans are in the
    /// same file. Reaching for `Span::new` with this parser's own `FileId`
    /// resolves a cross-file join to one side instead of refusing it, and
    /// `Span::to`'s own comment calls that a bug. Nothing can produce one until
    /// `#include` lands, which is this same phase.
    fn joined(&self, from: ExprId, to: ExprId) -> Span {
        self.ast.expr(from).span().to(self.ast.expr(to).span())
    }

    /// Consume whatever token is there, and give it back.
    ///
    /// For a caller that has already decided from `peek` what it is holding.
    fn advance(&mut self) -> Token {
        let token = self.peek();
        self.at += 1;
        token
    }

    fn at_end(&self) -> bool {
        self.at >= self.tokens.len() || self.peek().is_eof()
    }

    fn peek(&self) -> Token {
        self.tokens[self.at.min(self.tokens.len() - 1)]
    }

    /// The token `ahead` places past the one that is there.
    fn peek_at(&self, ahead: usize) -> Token {
        self.tokens[(self.at + ahead).min(self.tokens.len() - 1)]
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
        self.report_at(self.peek().span, code, message, label, diagnostics)
    }

    /// The same, at a span the caller names rather than at the token that is
    /// there.
    ///
    /// For a report whose reason is only known once the thing it is about has
    /// been read past. The parser is standing somewhere else by then, and a
    /// caret on the token it happens to be standing on is a caret on innocent
    /// code.
    fn report_at(
        &mut self,
        span: Span,
        code: Code,
        message: impl Into<String>,
        label: impl Into<String>,
        diagnostics: &mut DiagnosticSink,
    ) -> Span {
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
        sources: SourceMap,
    }

    /// Scan and parse, the way the driver does.
    fn parsed(text: &str) -> Parsed {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("test.c", text);
        let mut diagnostics = DiagnosticSink::new();
        let tokens = lex(file, sources.file(file), &mut diagnostics);
        let ast = parse(file, &tokens, &mut diagnostics);

        Parsed {
            ast,
            diagnostics,
            sources,
        }
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

    /// An `else` belongs to the nearest `if` it is allowed to belong to.
    ///
    /// C17 6.8.4.1 p3: "An else is associated with the lexically nearest
    /// preceding if that is allowed by the syntax." So `if (a) if (b) x; else
    /// y;` runs `y;` when `a` holds and `b` does not, and runs nothing at all
    /// when `a` does not.
    ///
    /// Mutation, and it is not a typo: a recursive descent already binds
    /// inward, so breaking this takes adding something. Give `if_statement` a
    /// flag saying an enclosing `if` wants the `else`, pass it to the inner
    /// `statement`, and have the inner `if` leave the `else` alone. This test
    /// then finds the `else` on the outer one and fails.
    #[test]
    fn a_dangling_else_binds_to_the_nearest_if() {
        let parsed = parsed("int main(void) { if (a) if (b) x; else y; }\n");

        assert_eq!(parsed.diagnostics.diagnostics().len(), 0);

        let [Item::Function(function)] = parsed.ast.items() else {
            panic!("{:?}", parsed.ast.items());
        };
        let Stmt::Compound { body, .. } = parsed.ast.stmt(function.body) else {
            panic!()
        };

        // The outer `if` has no `else`, and the inner one has it.
        let Stmt::If {
            then,
            otherwise: None,
            ..
        } = parsed.ast.stmt(body[0])
        else {
            panic!("{:?}", parsed.ast.stmt(body[0]));
        };
        assert!(matches!(
            parsed.ast.stmt(*then),
            Stmt::If {
                otherwise: Some(_),
                ..
            }
        ));
    }

    /// A null statement is an expression statement with no expression.
    ///
    /// C17 6.8.3 p1 gives one production, `expression_opt ;`, and p3 describes
    /// the null statement as "consisting of just a semicolon" without giving it
    /// one of its own. `clang` splits the two into `NullStmt` and `Stmt`; this
    /// follows the grammar instead.
    ///
    /// Mutation: give the null statement a variant of its own, or read `;` as
    /// an expression statement whose value is an `Expr::Error`. Either way the
    /// first of these two stops matching `value: None`.
    #[test]
    fn a_null_statement_is_an_expression_statement_with_nothing_in_it() {
        let parsed = parsed("int main(void) { ; i; }\n");

        assert_eq!(parsed.diagnostics.diagnostics().len(), 0);

        let [Item::Function(function)] = parsed.ast.items() else {
            panic!("{:?}", parsed.ast.items());
        };
        let Stmt::Compound { body, .. } = parsed.ast.stmt(function.body) else {
            panic!()
        };

        assert!(matches!(
            parsed.ast.stmt(body[0]),
            Stmt::Expression { value: None, .. }
        ));
        assert!(matches!(
            parsed.ast.stmt(body[1]),
            Stmt::Expression { value: Some(_), .. }
        ));
    }

    /// One syntax error per file, however many the file has.
    ///
    /// Reporting the second would mean believing a position the parser has no
    /// reason to believe in, because nothing has recovered yet. The sibling
    /// issue that adds recovery is what makes a second report worth reading.
    ///
    /// The input matters, and its first version did not. `)` begins no
    /// expression, and the brace after it is then not where the compound
    /// expects one, so two places reach for the right to report. A file that
    /// fails at its very first token passes this whichever way `report`
    /// behaves, because the loop in `run` has ended before anything else can
    /// speak: it guards the loop and calls it the report.
    ///
    /// It was `{ 0; }` until statements arrived, which is the shape to keep in
    /// mind when picking a replacement: an input chosen because the parser
    /// rejects it stops testing anything the day the parser accepts it.
    ///
    /// Two mutations, and each fails this on its own. Take the `failed` check
    /// out of `report` and the closing brace speaks as well as the statement.
    /// Drop `!self.failed` from the loop in `run` and the parser meets the same
    /// token until the budget runs out.
    #[test]
    fn only_the_first_syntax_error_is_reported() {
        let parsed = parsed("int main(void) { ) }\n");

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
    /// Four shapes, because four recursions can reach it and a bracket is only
    /// one of them: nested blocks, nested parentheses, a chain of a
    /// right-associative operator, which re-enters the climb once per operator,
    /// and a chain of a prefix operator, which re-enters `unary`. The last two
    /// write no bracket at all.
    ///
    /// Mutation: remove the `MAX_NESTING` check from `deeper`. This fails,
    /// because nothing is reported for the input past the limit. Taking the
    /// `deeper` call out of `compound`, `infix_from` or `unary` fails it for
    /// one shape each.
    ///
    /// That is not the failure the bound exists to prevent, and no test here
    /// can be. Without a bound the recursion runs to the end of the native
    /// stack: 4000 blocks killed this compiler outright on this host, with no
    /// diagnostic and an exit code the driver never chose, and a test that
    /// reproduced it would take the suite down rather than fail. So each input
    /// below stops one level past the limit, and what they hold is that the
    /// limit is enforced at all.
    #[test]
    fn nesting_is_bounded_and_the_bound_is_reported() {
        // A function's body is a level like any other, so `n` braces is `n`
        // deep. The two expression shapes are two levels deeper for the same
        // count: the body is one, and reading the statement's expression at all
        // is the second, before any of the nesting being counted here.
        let blocks = |levels: usize| {
            format!(
                "int main(void) {}{}\n",
                "{".repeat(levels),
                "}".repeat(levels)
            )
        };
        let parens = |levels: usize| {
            format!(
                "int main(void) {{ return {}0{}; }}\n",
                "(".repeat(levels),
                ")".repeat(levels)
            )
        };
        let assignments =
            |levels: usize| format!("int main(void) {{ return {}0; }}\n", "a = ".repeat(levels));

        let prefixes =
            |levels: usize| format!("int main(void) {{ return {}0; }}\n", "!".repeat(levels));

        // The three statements that hold a statement, nested through their
        // bodies. One level each, on top of the function body's, and none of
        // them writes a bracket per level: `else if` in particular is `else`
        // followed by an `if` statement rather than a construct of its own, so
        // a chain of them is a chain of nested `if`s.
        let else_ifs = |levels: usize| {
            format!(
                "int main(void) {{ {}; }}\n",
                "if (a) ; else ".repeat(levels)
            )
        };
        let whiles =
            |levels: usize| format!("int main(void) {{ {}; }}\n", "while (a) ".repeat(levels));
        let fors =
            |levels: usize| format!("int main(void) {{ {}; }}\n", "for (;;) ".repeat(levels));

        // A declarator at file scope, so nothing above it has taken a level.
        // The first of these two is bounded by `apply`, which counts the
        // derivations a declarator folds; the second by `deeper`, which counts
        // the parenthesised declarators it recurses through.
        let derivations = |levels: usize| format!("int {}p;\n", "*".repeat(levels));
        let parenthesised =
            |levels: usize| format!("int {}*p{};\n", "(".repeat(levels), ")".repeat(levels));

        for (shape, at_the_limit) in [
            (&blocks as &dyn Fn(usize) -> String, MAX_NESTING),
            (&parens, MAX_NESTING - 2),
            (&assignments, MAX_NESTING - 2),
            (&prefixes, MAX_NESTING - 2),
            (&else_ifs, MAX_NESTING - 1),
            (&whiles, MAX_NESTING - 1),
            (&fors, MAX_NESTING - 1),
            (&derivations, MAX_NESTING),
            (&parenthesised, MAX_NESTING),
        ] {
            let inside = parsed(&shape(at_the_limit));
            assert_eq!(
                inside.diagnostics.diagnostics().len(),
                0,
                "{:?}",
                inside.diagnostics.diagnostics()
            );

            let beyond = parsed(&shape(at_the_limit + 1));
            let reported = beyond.diagnostics.diagnostics();
            assert_eq!(reported.len(), 1, "{reported:?}");
            assert_eq!(reported[0].message(), "nesting is too deep");

            // Inside the thing that is too deep, never after it. `deeper`
            // reports where it stands, which is the token that opened the
            // level; `apply` only knows the count once the whole declarator has
            // been read, so it has to be handed where that declarator began.
            // Reporting where the parser is standing by then puts the caret on
            // the `;`, which is a byte with nothing wrong with it.
            let [label] = reported[0].labels() else {
                panic!("{:?}", reported[0].labels());
            };
            let source = beyond.sources.file(label.span().file());
            assert!(
                (label.span().end() as usize) < source.contents().trim_end().len(),
                "the caret is on the last token of {:?}",
                source.contents()
            );
        }
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

    /// So does an expression built out of other expressions.
    ///
    /// Held separately from the statement above because nothing else can see
    /// it: the artifact prints a start position only, and every other test
    /// matches on shape. What the extent is for is a later diagnostic
    /// underlining a whole subexpression, which is the point at which being
    /// wrong is expensive and quiet.
    ///
    /// Mutation: have `joined` end at `from` rather than at `to`, or drop the
    /// closing token from any of the three spans below. This fails.
    #[test]
    fn a_composite_expression_reaches_the_end_of_its_last_operand() {
        let text = "int main(void) { return f(a, b) + c[d]; }\n";
        let parsed = parsed(text);

        let sum = parsed.ast.expr(returned(&parsed));
        assert_eq!(&text[sum.span().range()], "f(a, b) + c[d]");

        let Expr::Binary { lhs, rhs, .. } = sum else {
            panic!("{sum:?}");
        };
        assert_eq!(&text[parsed.ast.expr(*lhs).span().range()], "f(a, b)");
        assert_eq!(&text[parsed.ast.expr(*rhs).span().range()], "c[d]");
    }

    /// Every prefix operator reaches the variant that spells it.
    ///
    /// The spellings themselves are checked in `ast.rs`; what is checked here
    /// is the other half, which punctuator the parser routes to which variant.
    /// Without it `!x` could be read as `~x`, or `*p` as `&p`, and both dumps
    /// and both suites would agree with themselves.
    ///
    /// Mutation: swap any two arms of the match in `unary`. This fails.
    #[test]
    fn every_prefix_operator_reaches_the_variant_that_spells_it() {
        for (text, expected) in [
            ("+a", UnOp::Plus),
            ("-a", UnOp::Minus),
            ("!a", UnOp::Not),
            ("~a", UnOp::BitNot),
            ("*a", UnOp::Deref),
            ("&a", UnOp::AddrOf),
            ("++a", UnOp::PreInc),
            ("--a", UnOp::PreDec),
        ] {
            let parsed = parsed(&format!("int main(void) {{ return {text}; }}\n"));
            let Expr::Unary { op, .. } = parsed.ast.expr(returned(&parsed)) else {
                panic!("{text:?} gave {:?}", parsed.ast.expr(returned(&parsed)));
            };
            assert_eq!(*op, expected, "{text:?}");
        }

        for (text, expected) in [("a++", UnOp::PostInc), ("a--", UnOp::PostDec)] {
            let parsed = parsed(&format!("int main(void) {{ return {text}; }}\n"));
            let Expr::Unary { op, .. } = parsed.ast.expr(returned(&parsed)) else {
                panic!("{text:?} gave {:?}", parsed.ast.expr(returned(&parsed)));
            };
            assert_eq!(*op, expected, "{text:?}");
        }
    }

    /// Parentheses change what a declarator derives, which is the whole reason
    /// the tree holds a type and not a declarator.
    ///
    /// C17 6.7.6 p6: "a declarator in parentheses is identical to the
    /// unparenthesized declarator, but the binding of complicated declarators
    /// may be altered by parentheses." `int *f(int)` is a function returning a
    /// pointer; `int (*f)(int)` is a pointer to a function. The two differ only
    /// in where the parentheses are.
    ///
    /// Mutation: stop treating `(` as opening a parenthesised declarator in
    /// `core`, or apply the suffixes before the pointers in `declarator`. Both
    /// make the two derive the same thing, and this fails.
    #[test]
    fn parentheses_change_what_a_declarator_derives() {
        let function_returning_pointer = parsed("int *f(int);\n");
        let [Item::Declaration(declaration)] = function_returning_pointer.ast.items() else {
            panic!("{:?}", function_returning_pointer.ast.items());
        };
        let Type::Function { returns, .. } = function_returning_pointer.ast.ty(declaration.ty)
        else {
            panic!("{:?}", function_returning_pointer.ast.ty(declaration.ty));
        };
        assert!(matches!(
            function_returning_pointer.ast.ty(*returns),
            Type::Pointer(_)
        ));

        let pointer_to_function = parsed("int (*f)(int);\n");
        let [Item::Declaration(declaration)] = pointer_to_function.ast.items() else {
            panic!("{:?}", pointer_to_function.ast.items());
        };
        let Type::Pointer(pointee) = pointer_to_function.ast.ty(declaration.ty) else {
            panic!("{:?}", pointer_to_function.ast.ty(declaration.ty));
        };
        assert!(matches!(
            pointer_to_function.ast.ty(*pointee),
            Type::Function { .. }
        ));
    }

    /// The suffix written last is the first to wrap the base.
    ///
    /// C17 6.7.6.2 p3 reads `D [ ... ]` by handing "array of T" to `D`, so
    /// `int a[3][5]` is three arrays of five and not five of three. Nothing
    /// else in the artifact can tell the two apart, because both spell
    /// `int[3][5]`.
    ///
    /// Mutation: drop the `reverse` in `declarator`. This fails on the length.
    #[test]
    fn the_last_suffix_written_wraps_the_base_first() {
        let parsed = parsed("int a[3][5];\n");

        let [Item::Declaration(declaration)] = parsed.ast.items() else {
            panic!("{:?}", parsed.ast.items());
        };
        let Type::Array {
            element,
            length: Some(outer),
        } = parsed.ast.ty(declaration.ty)
        else {
            panic!("{:?}", parsed.ast.ty(declaration.ty));
        };
        let Type::Array {
            length: Some(inner),
            ..
        } = parsed.ast.ty(*element)
        else {
            panic!("{:?}", parsed.ast.ty(*element));
        };

        let text = |id| {
            let span = parsed.ast.expr(id).span();
            parsed.sources.file(span.file()).contents()[span.range()].to_owned()
        };
        assert_eq!(text(*outer), "3");
        assert_eq!(text(*inner), "5");
    }

    /// A call with no arguments is a call, not a parse error.
    ///
    /// C17 6.5.2's `argument-expression-list` is optional, and `f()` is the
    /// ordinary way to write a call to a function of no parameters.
    ///
    /// Mutation: read an argument unconditionally in `call` rather than only
    /// when the next token is not `)`. This fails.
    #[test]
    fn a_call_can_have_no_arguments() {
        let parsed = parsed("int main(void) { return f(); }\n");

        assert_eq!(
            parsed.diagnostics.diagnostics().len(),
            0,
            "{:?}",
            parsed.diagnostics.diagnostics()
        );

        let Expr::Call { arguments, .. } = parsed.ast.expr(returned(&parsed)) else {
            panic!("{:?}", parsed.ast.expr(returned(&parsed)));
        };
        assert!(arguments.is_empty(), "{arguments:?}");
    }

    /// The expression a lone `return` in a lone function reads.
    fn returned(parsed: &Parsed) -> ExprId {
        let [Item::Function(function)] = parsed.ast.items() else {
            panic!("{:?}", parsed.ast.items());
        };
        let Stmt::Compound { body, .. } = parsed.ast.stmt(function.body) else {
            panic!("{:?}", parsed.ast.stmt(function.body));
        };
        let Stmt::Return {
            value: Some(value), ..
        } = parsed.ast.stmt(body[0])
        else {
            panic!("{:?}", parsed.ast.stmt(body[0]));
        };

        *value
    }

    /// The table C17 6.5 gives, written out rather than walked.
    ///
    /// RK-001 in the review knowledge bank is why it is written out: a test
    /// that walks a table is comparing the table with itself, and this
    /// repository has already shipped one, on the C keyword list, where
    /// `return` spelled `retrun` passed the entire suite.
    ///
    /// Mutation: swap two rows, or change one operator's power, associativity
    /// or node kind. This fails.
    #[test]
    fn the_precedence_table_is_the_one_c17_gives() {
        use Assoc::{Left, Right};
        use InfixKind::{Assign, Binary, Comma, Conditional};

        for (punct, power, assoc, kind) in [
            (Punct::Comma, 1, Left, Comma),
            (Punct::Equal, 2, Right, Assign(None)),
            (Punct::StarEqual, 2, Right, Assign(Some(BinOp::Mul))),
            (Punct::SlashEqual, 2, Right, Assign(Some(BinOp::Div))),
            (Punct::PercentEqual, 2, Right, Assign(Some(BinOp::Rem))),
            (Punct::PlusEqual, 2, Right, Assign(Some(BinOp::Add))),
            (Punct::MinusEqual, 2, Right, Assign(Some(BinOp::Sub))),
            (Punct::LessLessEqual, 2, Right, Assign(Some(BinOp::Shl))),
            (
                Punct::GreaterGreaterEqual,
                2,
                Right,
                Assign(Some(BinOp::Shr)),
            ),
            (Punct::AmpersandEqual, 2, Right, Assign(Some(BinOp::BitAnd))),
            (Punct::CaretEqual, 2, Right, Assign(Some(BinOp::BitXor))),
            (Punct::PipeEqual, 2, Right, Assign(Some(BinOp::BitOr))),
            (Punct::Question, 3, Right, Conditional),
            (Punct::PipePipe, 4, Left, Binary(BinOp::LogOr)),
            (Punct::AmpersandAmpersand, 5, Left, Binary(BinOp::LogAnd)),
            (Punct::Pipe, 6, Left, Binary(BinOp::BitOr)),
            (Punct::Caret, 7, Left, Binary(BinOp::BitXor)),
            (Punct::Ampersand, 8, Left, Binary(BinOp::BitAnd)),
            (Punct::EqualEqual, 9, Left, Binary(BinOp::Eq)),
            (Punct::BangEqual, 9, Left, Binary(BinOp::Ne)),
            (Punct::Less, 10, Left, Binary(BinOp::Lt)),
            (Punct::Greater, 10, Left, Binary(BinOp::Gt)),
            (Punct::LessEqual, 10, Left, Binary(BinOp::Le)),
            (Punct::GreaterEqual, 10, Left, Binary(BinOp::Ge)),
            (Punct::LessLess, 11, Left, Binary(BinOp::Shl)),
            (Punct::GreaterGreater, 11, Left, Binary(BinOp::Shr)),
            (Punct::Plus, 12, Left, Binary(BinOp::Add)),
            (Punct::Minus, 12, Left, Binary(BinOp::Sub)),
            (Punct::Star, 13, Left, Binary(BinOp::Mul)),
            (Punct::Slash, 13, Left, Binary(BinOp::Div)),
            (Punct::Percent, 13, Left, Binary(BinOp::Rem)),
        ] {
            assert_eq!(
                infix(punct),
                Some(Infix { power, assoc, kind }),
                "{punct:?}"
            );
        }

        // Punctuators that look like they belong and do not. `:` and the
        // increments are read where the expression they belong to is read, and
        // answering here would take them out from under it.
        for punct in [
            Punct::Colon,
            Punct::PlusPlus,
            Punct::MinusMinus,
            Punct::Dot,
            Punct::Arrow,
            Punct::Bang,
            Punct::Tilde,
            Punct::LeftParen,
            Punct::LeftBracket,
            Punct::Semicolon,
            Punct::RightBrace,
        ] {
            assert_eq!(infix(punct), None, "{punct:?}");
        }
    }

    /// Two operators of the same power group to the left.
    ///
    /// `a - b - c` is `(a - b) - c` and not `a - (b - c)`, which is a different
    /// number.
    ///
    /// Mutation: in `climb`, give `Assoc::Left` the same next power as
    /// `Assoc::Right`. This fails.
    #[test]
    fn left_associative_operators_group_to_the_left() {
        let parsed = parsed("int main(void) { return a - b - c; }\n");

        let Expr::Binary { lhs, rhs, .. } = parsed.ast.expr(returned(&parsed)) else {
            panic!("{:?}", parsed.ast.expr(returned(&parsed)));
        };

        assert!(matches!(parsed.ast.expr(*lhs), Expr::Binary { .. }));
        assert!(matches!(parsed.ast.expr(*rhs), Expr::Identifier { .. }));
    }

    /// Assignment groups to the right. C17 6.5.16.
    ///
    /// `a = b = c` is `a = (b = c)`. The other grouping assigns to the result
    /// of an assignment, which is not even a place.
    ///
    /// Mutation: give assignment `Assoc::Left` in the table. This fails.
    #[test]
    fn assignment_groups_to_the_right() {
        let parsed = parsed("int main(void) { return a = b = c; }\n");

        let Expr::Assign { place, value, .. } = parsed.ast.expr(returned(&parsed)) else {
            panic!("{:?}", parsed.ast.expr(returned(&parsed)));
        };

        assert!(matches!(parsed.ast.expr(*place), Expr::Identifier { .. }));
        assert!(matches!(parsed.ast.expr(*value), Expr::Assign { .. }));
    }

    /// A comma between arguments separates them; one inside parentheses is the
    /// operator.
    ///
    /// C17 6.5.2 makes an argument an assignment-expression, which is the whole
    /// of what tells the two apart. Nothing in the tree has to.
    ///
    /// Mutation: read an argument with `expression` rather than `assignment`.
    /// `f(a, b)` becomes one argument, and this fails.
    #[test]
    fn a_comma_in_a_call_separates_arguments() {
        let separated = parsed("int main(void) { return f(a, b); }\n");
        let Expr::Call { arguments, .. } = separated.ast.expr(returned(&separated)) else {
            panic!("{:?}", separated.ast.expr(returned(&separated)));
        };
        assert_eq!(arguments.len(), 2, "{arguments:?}");

        let grouped = parsed("int main(void) { return f((a, b)); }\n");
        let Expr::Call { arguments, .. } = grouped.ast.expr(returned(&grouped)) else {
            panic!("{:?}", grouped.ast.expr(returned(&grouped)));
        };
        assert_eq!(arguments.len(), 1, "{arguments:?}");
        assert!(matches!(grouped.ast.expr(arguments[0]), Expr::Comma { .. }));
    }

    /// C17 6.5.15 puts a whole expression between `?` and `:`, the comma
    /// operator included, because the `:` is what ends it.
    ///
    /// Only the third operand is read at the conditional's own power.
    ///
    /// Mutation: read the middle with `assignment` rather than `expression`.
    /// The comma then ends it, ``expected `:``` is reported, and this fails.
    #[test]
    fn the_middle_of_a_conditional_is_a_full_expression() {
        let parsed = parsed("int main(void) { return a ? b, c : d; }\n");

        assert_eq!(
            parsed.diagnostics.diagnostics().len(),
            0,
            "{:?}",
            parsed.diagnostics.diagnostics()
        );

        let Expr::Conditional { then, .. } = parsed.ast.expr(returned(&parsed)) else {
            panic!("{:?}", parsed.ast.expr(returned(&parsed)));
        };
        assert!(matches!(parsed.ast.expr(*then), Expr::Comma { .. }));
    }

    /// Each thing the parser insists on says which one was missing.
    ///
    /// Mutation: give every `expect` the same `what`. This fails, because the
    /// messages stop telling the four apart.
    #[test]
    fn what_was_expected_is_named_in_the_message() {
        for (text, expected) in [
            ("int (void) { }\n", "expected a name"),
            ("int (*)(void) { }\n", "expected a name"),
            ("int main void) { }\n", "expected `;`"),
            ("int main(void { }\n", "expected `)`"),
            ("int main(void) return 0; }\n", "expected `;`"),
            ("int (*p;\n", "expected `)`"),
            ("int a[10;\n", "expected `]`"),
            ("int f(int;\n", "expected `)`"),
            ("int f(int a, );\n", "expected a declaration"),
            ("int main(void) { return 0 }\n", "expected `;`"),
            ("int main(void) { return; \n", "expected `}`"),
            ("int main(void) { return + ; }\n", "expected an expression"),
            ("int main(void) { 0 }\n", "expected `;`"),
            ("int main(void) { if x) ; }\n", "expected `(`"),
            ("int main(void) { if (x ; }\n", "expected `)`"),
            ("int main(void) { while x) ; }\n", "expected `(`"),
            ("int main(void) { while (x ; }\n", "expected `)`"),
            ("int main(void) { for ;;) ; }\n", "expected `(`"),
            ("int main(void) { for (x x; ) ; }\n", "expected `;`"),
            ("int main(void) { for (;; x ; }\n", "expected `)`"),
            ("int main(void) { return (a; }\n", "expected `)`"),
            ("int main(void) { return f(a; }\n", "expected `)`"),
            ("int main(void) { return a[i; }\n", "expected `]`"),
            ("int main(void) { return a ? b c; }\n", "expected `:`"),
        ] {
            let parsed = parsed(text);
            let reported = parsed.diagnostics.diagnostics();

            assert_eq!(reported.len(), 1, "{text:?} gave {reported:?}");
            assert_eq!(reported[0].message(), expected, "{text:?}");
        }
    }
}
