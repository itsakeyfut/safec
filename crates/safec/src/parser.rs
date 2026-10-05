//! Tokens to a tree, by recursive descent.
//!
//! What it reads is C17 6.7.6's declarators over `int`, `char` and `void`, the
//! operators of 6.5 from an integer constant or an identifier up to the comma
//! operator, and the statements of 6.8.3 through 6.8.5: a compound statement,
//! `return`, an expression statement, `if`, `while` and the `for` whose clauses
//! are expressions. What it does not read yet is `switch`, `do`, `goto`, a
//! labelled statement, `break`, `continue`, a declaration in a `for`
//! initialiser, structs, member access, casts, `sizeof`, the type qualifiers,
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
    Ast, Attribute, BinOp, Declaration, Expr, ExprId, Function, InitDeclarator, Item, Nullability,
    Parameters, Specifier, Stmt, StmtId, Type, TypeId, UnOp,
};
use crate::diagnostics::{Code, Diagnostic, DiagnosticSink, Label};
use crate::token::{Annotation, Keyword, Punct, Token, TokenKind};
use safec_ir::source::{FileId, Span};

/// Something was expected and something else was there.
///
/// `SC02xx` is syntax's range, which `docs/diagnostics.md` allocates rather
/// than this comment. One code covers every shape of this because what differs
/// between them is the message, and a code exists so that a reader can search
/// for a class of error rather than a sentence.
const EXPECTED: Code = Code::new("SC0201");

/// A program nested deeper than this parser will go.
///
/// Its own code rather than [`EXPECTED`], because nothing was expected: the
/// program is well formed and this compiler is declining to read it, which is a
/// different thing to tell a reader and a different thing to search for.
const TOO_DEEP: Code = Code::new("SC0202");

/// An initializer written in braces, which C17 6.7.9 allows and this stage
/// does not read.
///
/// Its own code for [`TOO_DEEP`]'s reason and one more: nothing is wrong with
/// the program, so a reader has to be able to tell an unimplemented feature
/// from a defect, and a number they can search for is how they do it.
///
/// `int x = {1};` is the case that makes this worth saying. 6.7.9 p11 lets a
/// scalar's initializer be a single expression "optionally enclosed in
/// braces", so that program is valid C with no designator and no nested list
/// in it, and `clang -std=c17 -pedantic-errors` accepts it in silence. This
/// refuses it all the same, because unwrapping the braces is only correct
/// where the declared type is scalar and this stage does not know the type.
/// `docs/frontend.md` carries the row, which is where a reader looks to tell a
/// gap from a decision.
const BRACED_INITIALIZER: Code = Code::new("SC0203");

/// A nullability specifier or `__attribute__` written where it cannot apply.
///
/// `__attribute__` applies before a function definition at file scope, and
/// nowhere else, because the one form of it this compiler reads makes that
/// definition a hatch and a hatch is about a body. See ADR-0038.
///
/// `_Nonnull` and `_Nullable` apply to two things, the pointer a parameter of a
/// declared function holds and the pointer such a function returns, because
/// those are the places ADR-0037 and ADR-0050 give them a meaning: the body or
/// the caller believes `_Nonnull` and the other side is checked against it, and
/// `_Nullable` says what level 5 would otherwise not assume. Anywhere else one
/// would be read and mean nothing, which is a promise written down and silently
/// dropped, so it is refused instead. `clang` accepts them on a local and on a
/// pointer inside another; `docs/frontend.md` carries those rows.
///
/// The parser's rather than a later stage's, because it is the stage that
/// knows what a declarator declares: whether it is a parameter, and whether the
/// function type a parameter list belongs to is the declared function's own.
const MISPLACED_ANNOTATION: Code = Code::new("SC0204");

/// An annotation this compiler does not read: an attribute, or a nullability
/// specifier other than the two it reads.
///
/// `__attribute__` is read in one form, the hatch ADR-0038 decides, and
/// anything else written with it is refused rather than skipped. A skipped
/// attribute is one whose meaning this compiler changed without saying so, and
/// ADR-0037 measured `clang` deleting a test the author wrote on the strength of
/// `nonnull`. `_Null_unspecified` and `_Nullable_result` are refused for the
/// same reason, wherever a declarator can hold them (ADR-0050).
///
/// Its own code rather than [`MISPLACED_ANNOTATION`], because it is a different
/// program to fix: that one is a known word in the wrong place, and this is a
/// word this compiler does not know. Reported from two stages. This one refuses
/// what the shape alone shows; `sema::resolve` refuses a name that is not
/// `annotate` and a string that is not `"safec_unchecked"`, because comparing
/// text is not the parser's. See [`Attribute`].
pub(crate) const UNREAD_ANNOTATION: Code = Code::new("SC0205");

/// What [`UNREAD_ANNOTATION`] says under its caret about an attribute, from
/// either stage.
pub(crate) const UNREAD_ATTRIBUTE_LABEL: &str =
    "the one attribute it reads is `annotate(\"safec_unchecked\")`";

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
/// `Parser::deeper`, which is the one place this is counted, and the limit is
/// tighter than `clang`'s for shapes `clang` does not count at all.
///
/// C17 5.2.4.1 asks an implementation to manage "127 nesting levels of blocks",
/// "63 nesting levels of parenthesized expressions within a full expression"
/// and "63 nesting levels of parenthesized declarators within a full
/// declarator" and "12 pointer, array, and function declarators (in any
/// combinations) modifying an arithmetic, structure, union, or void type in a
/// declaration". One program holding all four at once is accepted: 127 nested
/// blocks with 63 nested parentheses inside the innermost, which is what p1
/// asks for when it says an implementation must translate "at least one
/// program" containing an instance of every limit.
///
/// A selection or iteration statement takes a level too, and by C's own
/// reckoning that is what a block count is. 6.8.4 p3: "A selection statement is
/// a block whose scope is a strict subset of the scope of its enclosing block.
/// Each associated substatement is also a block." 6.8.5 p5 says the same of an
/// iteration statement and its body. So `for (;;) if (x) { }` is three blocks
/// per level and this refuses it at 86, which is 258 blocks rather than 86 of
/// anything. The counter and C agree; the shapes just do not look alike.
///
/// What 5.2.4.1 sets no minimum for is a chain of unary or assignment
/// operators, which is the only room this takes beyond them.
///
/// **This bounds the parser, not the tree.** A left-associative chain and a run
/// of postfix operators are folded by a loop, so `a + a + ...` and `a++++` are
/// as deep in the tree as they are long while `depth` never rises. Anything
/// that walks the tree owes itself an answer to that; `dump_expr` in
/// `driver/dumps.rs` uses an explicit stack, and says so.
/// A test outside this crate reads it, because a process-level test of the
/// deepest tree the printer can be handed is only honest if the depth it builds
/// follows this number rather than restating it.
pub const MAX_NESTING: usize = 256;

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

/// How a declaration and a function definition both begin: the specifiers and
/// the first declarator.
///
/// The base is carried alongside the derived type because the two are needed
/// for different things and neither can be recovered from the other. `ty` is
/// what this declarator declared; `base` is what the specifiers said, and it
/// is what every later declarator in the same declaration folds its own
/// derivations onto.
struct Declared {
    /// Where the specifiers began, which is where the declaration's span
    /// starts and where a first declarator that nests too deep is reported.
    start: Span,
    /// What the specifiers named, before any declarator derived from it.
    base: TypeId,
    /// The span of the name this declarator wrote.
    name: Span,
    /// The type this declarator derived from `base`.
    ty: TypeId,
    /// The nullability specifier on the pointer the declared function
    /// returns, which [`Parser::placed`] allows only at file scope.
    return_nullability: Option<Nullability>,
    /// What the declaration declares, which every later declarator in it
    /// shares.
    declares: Declares,
}

/// What a declarator declares, which is what decides whether a nullability
/// specifier may be written in it.
///
/// See [`MISPLACED_ANNOTATION`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Declares {
    /// A parameter, whose own pointer is what a nullability specifier qualifies.
    Parameter,
    /// A declaration or definition at file scope, whose function type's
    /// parameters are the declared function's own.
    FileScope,
    /// A declaration inside a block. `lowering.rs::declare` reads only
    /// file-scope items, so a function declared here is not one a call is
    /// checked against, and nothing in it may carry a nullability specifier.
    BlockScope,
}

/// One step a declarator derives, in the order it wraps the base type.
///
/// Collected rather than applied while reading. In `T (D)(params)` the inner
/// `D`'s base is `T` modified by the suffixes that follow the closing
/// parenthesis, and those have not been read when `D` is. Collecting lets the
/// fold happen once, at the end, in the order C17 6.7.6 p4 to p6 derive.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Derivation {
    /// C17 6.7.6.1, and the nullability specifier written after its `*`, if
    /// one was.
    Pointer {
        /// Which one, and where.
        nullability: Option<Nullability>,
    },
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
/// them stop agreeing, and two spellings of one decision have already left a
/// case here answered by neither.
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

/// The nullability specifier this token is, if it is one this compiler reads.
///
/// Every annotation written out, so that one added later is answered for here
/// by `error[E0004]` rather than read as no specifier at all. The two refused
/// ones are [`Parser::unread_specifier`]'s.
fn read_specifier(kind: TokenKind) -> Option<Specifier> {
    let TokenKind::Annotation(annotation) = kind else {
        return None;
    };
    match annotation {
        Annotation::Nonnull => Some(Specifier::Nonnull),
        Annotation::Nullable => Some(Specifier::Nullable),
        Annotation::NullUnspecified | Annotation::NullableResult | Annotation::Attribute => None,
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
/// `BinOp::Add` with the right precedence. A second copy of one decision has
/// already left a case here answered by neither.
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
        let first = self.peek().span;

        // Before the specifiers, and outside the span of what follows it, which
        // `Function::span` says begins at the specifiers.
        let attribute = if self.check(TokenKind::Annotation(Annotation::Attribute)) {
            let Some(attribute) = self.attribute(diagnostics) else {
                return Item::Error { span: first };
            };
            Some(attribute)
        } else {
            None
        };
        let start = self.peek().span;

        let Some(declared) = self.declared(Declares::FileScope, diagnostics) else {
            return Item::Error { span: first };
        };

        if self.check(TokenKind::Punct(Punct::LeftBrace)) {
            let Some(body) = self.compound(diagnostics) else {
                return Item::Error { span: first };
            };

            let span = Span::new(self.file, start.start(), self.previous().span.end());
            return Item::Function(Function {
                ty: declared.ty,
                name: declared.name,
                body,
                span,
                attribute,
                return_nullability: declared.return_nullability,
            });
        }

        // Refused here, where the missing body is first known, rather than
        // read and dropped: a declaration marked as a hatch says nothing this
        // compiler can act on, and a promise written down and silently ignored
        // is what `MISPLACED_ANNOTATION` exists to refuse.
        if let Some(attribute) = attribute {
            self.report_at(
                attribute.span,
                MISPLACED_ANNOTATION,
                "`__attribute__` cannot apply here",
                "a hatch is a function definition, and this declaration has no body",
                diagnostics,
            );
            return Item::Error { span: first };
        }

        let Some(declarators) = self.init_declarator_list(declared, diagnostics) else {
            return Item::Error { span: start };
        };

        if self
            .expect(TokenKind::Punct(Punct::Semicolon), "`;`", diagnostics)
            .is_none()
        {
            return Item::Error { span: start };
        }

        let span = Span::new(self.file, start.start(), self.previous().span.end());
        Item::Declaration { declarators, span }
    }

    /// The specifiers and one declarator, which is how a declaration and a
    /// definition both begin.
    fn declared(
        &mut self,
        declares: Declares,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<Declared> {
        let start = self.peek().span;
        let base = self.specifiers(diagnostics)?;
        let (name, ty, returns) = self.named_declarator(start, base, declares, diagnostics)?;

        Some(Declared {
            start,
            base,
            name,
            ty,
            return_nullability: returns,
            declares,
        })
    }

    /// One declarator that has to have a name, with its derivations folded onto
    /// the base.
    ///
    /// `start` is where a declarator that nests too deep is reported, so it is
    /// that declarator's own first token rather than the declaration's: the
    /// fourth declarator of a list is not a reason to put a caret on the
    /// specifiers, which are shared and are not what is wrong.
    fn named_declarator(
        &mut self,
        start: Span,
        base: TypeId,
        declares: Declares,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<(Span, TypeId, Option<Nullability>)> {
        let (name, derivations) = self.declarator(true, diagnostics)?;
        let (ty, returns) = self.apply(start, base, derivations, declares, diagnostics)?;

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

        Some((name, ty, returns))
    }

    /// The rest of C17 6.7's `init-declarator-list`, after [`Parser::declared`]
    /// has read the first declarator.
    ///
    /// The specifiers are read once and every declarator folds its own
    /// derivations onto them, which is what makes `int *p, a[10];` a pointer
    /// and an array rather than two of either.
    ///
    /// The `;` is the caller's to expect, because the caller is the one that
    /// has already decided this is not a function definition.
    fn init_declarator_list(
        &mut self,
        declared: Declared,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<Vec<InitDeclarator>> {
        let Declared {
            start,
            base,
            mut name,
            mut ty,
            return_nullability: mut returns,
            declares,
        } = declared;
        let mut declarators = Vec::new();

        loop {
            let init = self.initializer(diagnostics)?;
            declarators.push(InitDeclarator {
                declaration: Declaration {
                    name: Some(name),
                    ty,
                    // From the specifiers, which all of these share, through
                    // this one's initializer. `Declaration::span` says why they
                    // all begin at the same byte and what tells them apart.
                    span: Span::new(self.file, start.start(), self.previous().span.end()),
                    // The pointer a function declared at file scope returns,
                    // the one place besides a parameter `apply` allows one.
                    nullability: returns,
                },
                init,
            });

            if !self.eat(TokenKind::Punct(Punct::Comma)) {
                return Some(declarators);
            }

            // The `,` is eaten before anything else is read, so a round of this
            // loop always consumes a token and `spend`'s budget is not what
            // stops it going round forever.
            let at = self.peek().span;
            (name, ty, returns) = self.named_declarator(at, base, declares, diagnostics)?;
        }
    }

    /// C17 6.7.9's `initializer`, in its `assignment-expression` form only.
    ///
    /// [`Parser::assignment`] rather than [`Parser::expression`], which is the
    /// difference between the two and the whole of why both exist. An
    /// initializer is an assignment-expression, so the `,` in
    /// `int a = 1, b = 2;` separates declarators. Read with `expression` the
    /// comma operator takes it instead: `b = 2` is swallowed into `a`'s
    /// initializer, one name is declared where the source wrote two, and
    /// nothing is reported here at all.
    ///
    /// `Some(None)` is a declarator with no initializer and is the ordinary
    /// case. `None` is a refusal that has already been reported.
    fn initializer(&mut self, diagnostics: &mut DiagnosticSink) -> Option<Option<ExprId>> {
        if !self.eat(TokenKind::Punct(Punct::Equal)) {
            return Some(None);
        }

        if self.check(TokenKind::Punct(Punct::LeftBrace)) {
            self.report_noted(
                BRACED_INITIALIZER,
                "a braced initializer is not read yet",
                "this stage reads an expression here, and not a brace",
                "C17 6.7.9 p11 makes `= { e }` and `= e` the same thing for a \
                 scalar, so dropping the braces is exact wherever the type is \
                 one. The form that needs designators and nested lists is for \
                 an aggregate, and arrives with structs and arrays",
                diagnostics,
            );
            return None;
        }

        Some(Some(self.assignment(diagnostics)))
    }

    /// `__attribute__((name("argument")))`, the one shape of it this parser
    /// reads. See [`Attribute`] for what is left to `sema::resolve`.
    ///
    /// Everything else inside the inner parentheses is refused with
    /// [`UNREAD_ANNOTATION`]: an attribute with no argument, one whose argument
    /// is not a string, one with more than one argument or string, and a list
    /// of more than one attribute. Each is an attribute that is not the
    /// hatch's, and telling which from the shape alone needs no text.
    fn attribute(&mut self, diagnostics: &mut DiagnosticSink) -> Option<Attribute> {
        let start = self.advance().span;
        self.expect(TokenKind::Punct(Punct::LeftParen), "`(`", diagnostics)?;
        self.expect(TokenKind::Punct(Punct::LeftParen), "`(`", diagnostics)?;

        // Whatever is here is taken as the name, identifier or not. What is not
        // a name is refused all the same: `__attribute__(())` by the `(` it
        // lacks after it, at the same caret, and `__attribute__((1("x")))` by
        // `sema::resolve`, which compares it with `annotate`. A test here that
        // it is an identifier was measured to be one no mutation could break.
        let name = self.advance().span;

        if !self.eat(TokenKind::Punct(Punct::LeftParen)) {
            return self.unread(name, diagnostics);
        }
        if !self.check(TokenKind::String) {
            return self.unread(self.peek().span, diagnostics);
        }
        let argument = self.advance().span;
        // A second argument, or a second string that C would join to the
        // first, is not the hatch's one string: `clang` reads both, and a
        // `SC0201` there would call valid GNU C malformed.
        if !self.eat(TokenKind::Punct(Punct::RightParen)) {
            return self.unread(self.peek().span, diagnostics);
        }
        // A second attribute in the list, for the same reason.
        if !self.eat(TokenKind::Punct(Punct::RightParen)) {
            return self.unread(self.peek().span, diagnostics);
        }
        self.expect(TokenKind::Punct(Punct::RightParen), "`)`", diagnostics)?;

        Some(Attribute {
            span: Span::new(self.file, start.start(), self.previous().span.end()),
            name,
            argument,
        })
    }

    /// Refuse an attribute this compiler does not read, at `at`.
    fn unread(&mut self, at: Span, diagnostics: &mut DiagnosticSink) -> Option<Attribute> {
        self.report_at(
            at,
            UNREAD_ANNOTATION,
            "safec does not read this attribute",
            UNREAD_ATTRIBUTE_LABEL,
            diagnostics,
        );
        None
    }

    /// The declaration specifiers, of which this stage reads one.
    ///
    /// `__attribute__` arrives here wherever a declaration begins with it
    /// other than at file scope, and a second one after the first arrives here
    /// too: [`Parser::item`] reads one and hands the rest to this.
    fn specifiers(&mut self, diagnostics: &mut DiagnosticSink) -> Option<TypeId> {
        if self.check(TokenKind::Annotation(Annotation::Attribute)) {
            self.misplaced_attribute(diagnostics);
            return None;
        }

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
            // One, and only directly after the `*`, which is where `clang`
            // reads it. A second is left for `core`, which refuses it.
            let nullability = self.nullability(diagnostics)?;
            derivations.push(Derivation::Pointer { nullability });
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

        // `int _Nonnull x;` is the one place a reader who knows the word but
        // not its position will write it, and "expected a name" would say
        // nothing about why. The other way to arrive here is a second one
        // after the first, which is after a `*` and needs its own words:
        // telling that reader to write it after the `*` is telling them what
        // they did. `clang` warns about a duplicate and reads one, and refuses
        // `_Nonnull _Nullable` as two that conflict.
        if let Some(specifier) = read_specifier(self.peek().kind) {
            let label = if read_specifier(self.previous().kind).is_some() {
                "a pointer takes one nullability specifier, and this is the second"
            } else {
                "a nullability specifier is written after the `*` of the pointer it qualifies"
            };
            self.report_at(
                self.peek().span,
                MISPLACED_ANNOTATION,
                format!("`{}` cannot apply here", specifier.spelling()),
                label,
                diagnostics,
            );
            return None;
        }
        if self.unread_specifier(diagnostics) {
            return None;
        }

        // After a specifier or a `*`, which GNU C allows and this compiler
        // does not read.
        if self.check(TokenKind::Annotation(Annotation::Attribute)) {
            self.misplaced_attribute(diagnostics);
            return None;
        }

        if named {
            self.report("expected a name", "a name is missing here", diagnostics);
            return None;
        }

        Some((None, Vec::new()))
    }

    /// The nullability specifier after a `*`, if one is there.
    ///
    /// `Some(None)` is a pointer with none, which is the ordinary case, and
    /// `None` is a specifier this compiler does not read, already reported.
    fn nullability(&mut self, diagnostics: &mut DiagnosticSink) -> Option<Option<Nullability>> {
        if let Some(specifier) = read_specifier(self.peek().kind) {
            let at = self.advance().span;
            return Some(Some(Nullability { specifier, at }));
        }
        if self.unread_specifier(diagnostics) {
            return None;
        }
        Some(None)
    }

    /// Refuse `_Null_unspecified` or `_Nullable_result` if one is there, and
    /// say whether one was.
    ///
    /// Read as a word and refused, rather than read as a name and refused for
    /// something else, so that the reader is told the compiler does not read
    /// it. `_Null_unspecified` would leave the pointer as unannotated, which
    /// is a promise written down and dropped, and `_Nullable_result` is read by
    /// `clang` as `_Nullable` is, so reading it would be a second spelling for
    /// one answer (ADR-0050).
    fn unread_specifier(&mut self, diagnostics: &mut DiagnosticSink) -> bool {
        let TokenKind::Annotation(
            annotation @ (Annotation::NullUnspecified | Annotation::NullableResult),
        ) = self.peek().kind
        else {
            return false;
        };
        self.report_as(
            UNREAD_ANNOTATION,
            format!("safec does not read `{}`", annotation.as_str()),
            "the nullability specifiers it reads are `_Nonnull` and `_Nullable`",
            diagnostics,
        );
        true
    }

    /// Refuse the `__attribute__` that is there, which is not before a
    /// function definition at file scope.
    fn misplaced_attribute(&mut self, diagnostics: &mut DiagnosticSink) {
        self.report_as(
            MISPLACED_ANNOTATION,
            "`__attribute__` cannot apply here",
            "it is read once, before a function definition at file scope",
            diagnostics,
        );
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
            // The parameter's own pointer is the last derivation, the one that
            // makes the parameter's type. `apply` refuses one written anywhere
            // else, so this is the only one there is to keep.
            let nullability = match derivations.last() {
                Some(Derivation::Pointer { nullability }) => *nullability,
                _ => None,
            };
            let (ty, _) = self.apply(start, base, derivations, Declares::Parameter, diagnostics)?;

            let span = Span::new(self.file, start.start(), self.previous().span.end());
            parameters.push(Declaration {
                name,
                ty,
                span,
                nullability,
            });

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
    ///
    /// It is also where a nullability specifier is placed or refused, because
    /// this is the first point at which the whole declarator is known: which
    /// derivation is the declared entity's own type is only settled once the
    /// last one is read. See [`MISPLACED_ANNOTATION`]. The specifier on the
    /// pointer a function declared at file scope returns comes back beside the
    /// type, because that pointer is not the declared entity's own and no
    /// derivation of the type keeps it.
    fn apply(
        &mut self,
        start: Span,
        base: TypeId,
        derivations: Vec<Derivation>,
        declares: Declares,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<(TypeId, Option<Nullability>)> {
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

        let returns = self.placed(&derivations, declares, diagnostics)?;

        let mut ty = base;
        for derivation in derivations {
            ty = self.ast.push_type(match derivation {
                Derivation::Pointer { nullability: _ } => Type::Pointer(ty),
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

        Some((ty, returns))
    }

    /// Refuse every nullability specifier in one declarator that is not on a
    /// parameter's own pointer or on the pointer a declared function returns,
    /// and hand back the second.
    ///
    /// Two places can hold one. A pointer derivation holds the one written
    /// after its `*`, which is allowed where it is the last derivation of a
    /// parameter, or the one just before the last of a file-scope declarator
    /// whose last is a function, which is that function's return. A function
    /// derivation holds its parameters, whose own specifiers were allowed when
    /// each was read, and is kept only where the function is the declared one:
    /// the last derivation of a file-scope declarator. Everywhere else,
    /// `void (*fp)(int * _Nonnull)` and `int * _Nonnull (*fp)(void)` among
    /// them, there is no declared function whose calls could be checked
    /// against it.
    ///
    /// `Some` is placed, carrying the return's specifier if one was written,
    /// and `None` has been reported.
    fn placed(
        &mut self,
        derivations: &[Derivation],
        declares: Declares,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<Option<Nullability>> {
        let last = derivations.len().saturating_sub(1);
        let declares_a_function = declares == Declares::FileScope
            && matches!(derivations.last(), Some(Derivation::Function(_)));
        let mut returns = None;

        for (index, derivation) in derivations.iter().enumerate() {
            let own = index == last;
            let refused = match derivation {
                Derivation::Pointer {
                    nullability: Some(written),
                } => match declares {
                    Declares::Parameter if own => None,
                    Declares::Parameter => Some((
                        *written,
                        "this qualifies a pointer inside the parameter's type",
                    )),
                    Declares::FileScope if declares_a_function && index + 1 == last => {
                        returns = Some(*written);
                        None
                    }
                    Declares::FileScope | Declares::BlockScope => Some((
                        *written,
                        "this is not the pointer a parameter holds or a function returns",
                    )),
                },
                Derivation::Function(Parameters::Prototype(parameters))
                    if !(own && declares == Declares::FileScope) =>
                {
                    // A parameter whose own type is a function, `int g(int *
                    // _Nonnull p)` in a parameter list, is the second line's:
                    // C17 6.7.6.3 p8 adjusts it to a pointer, so no function is
                    // declared by it.
                    let label = if own && declares == Declares::BlockScope {
                        "this is a parameter of a function declared inside a block"
                    } else {
                        "this is a parameter of a function type, not of a declared function"
                    };
                    parameters
                        .iter()
                        .find_map(|parameter| parameter.nullability)
                        .map(|written| (written, label))
                }
                Derivation::Pointer { nullability: None }
                | Derivation::Array(_)
                | Derivation::Function(_) => None,
            };

            if let Some((written, label)) = refused {
                self.report_at(
                    written.at,
                    MISPLACED_ANNOTATION,
                    format!("`{}` cannot apply here", written.specifier.spelling()),
                    label,
                    diagnostics,
                );
                return None;
            }
        }

        Some(returns)
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
                    // `__attribute__` goes with the declarations, where
                    // `specifiers` refuses it by name, rather than to the
                    // expressions, which would call it a missing one.
                    let item = if specifier(parser.peek().kind).is_some()
                        || parser.check(TokenKind::Annotation(Annotation::Attribute))
                    {
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

        let Some(declared) = self.declared(Declares::BlockScope, diagnostics) else {
            return self.ast.push_stmt(Stmt::Error { span: start });
        };

        let Some(declarators) = self.init_declarator_list(declared, diagnostics) else {
            return self.ast.push_stmt(Stmt::Error { span: start });
        };

        if self
            .expect(TokenKind::Punct(Punct::Semicolon), "`;`", diagnostics)
            .is_none()
        {
            return self.ast.push_stmt(Stmt::Error { span: start });
        }

        let span = Span::new(self.file, start.start(), self.previous().span.end());
        self.ast.push_stmt(Stmt::Declaration { declarators, span })
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
    ///
    /// `expression` and not `assignment`, because 6.8.3 p1 spells it
    /// `expression`, so `i = 0, j = 0;` is one statement. The same decision is
    /// made in `controlling` and in `clause`, and all three are pinned: the
    /// comma in `expression_statement.c` is what fails when this one is
    /// narrowed, and `a_comma_in_a_controlling_expression.c` is what fails when
    /// either of the others is.
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

        let span = start.to(self.previous().span);
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

                let span = start.to(parser.previous().span);
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
                let span = start.to(parser.previous().span);
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
    /// expression_opt ; expression_opt )`, is still refused, at the `int`. It
    /// used to be blocked on there being no initializer to read, which is no
    /// longer true: what is left is that `Stmt::For`'s first clause holds an
    /// expression, and that 6.8.5 p5 gives a declaration written here a scope
    /// of its own. `docs/frontend.md` carries the row, so the refusal is
    /// visible to a reader who is not in this file.
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
                let span = start.to(parser.previous().span);
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
    /// The token's text is never spelled into the message, because text out of
    /// a source file is content, and a
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
        let built = Diagnostic::error(message)
            .with_code(code)
            .with_label(Label::primary(span, label));

        self.report_built(span, built, diagnostics)
    }

    /// The same, at the token that is there, with a note under the label.
    ///
    /// For a refusal that is about this compiler rather than about the program:
    /// the label says what cannot be read and the note says what it is waiting
    /// on, so a reader can tell an unimplemented feature from a defect without
    /// going to look.
    fn report_noted(
        &mut self,
        code: Code,
        message: impl Into<String>,
        label: impl Into<String>,
        note: impl Into<String>,
        diagnostics: &mut DiagnosticSink,
    ) -> Span {
        let span = self.peek().span;
        let built = Diagnostic::error(message)
            .with_code(code)
            .with_label(Label::primary(span, label))
            .with_note(note);

        self.report_built(span, built, diagnostics)
    }

    /// Report one, unless something already has.
    ///
    /// The one place `failed` is set, so that "only the first one speaks" is a
    /// property of this function rather than a rule every reporter remembers.
    fn report_built(
        &mut self,
        span: Span,
        diagnostic: Diagnostic,
        diagnostics: &mut DiagnosticSink,
    ) -> Span {
        if !self.failed {
            self.failed = true;
            diagnostics.report(diagnostic);
        }

        span
    }
}

#[cfg(test)]
mod tests;
