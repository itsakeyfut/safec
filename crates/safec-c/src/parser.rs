//! Tokens to a tree, by recursive descent.
//!
//! What it reads is C17 6.7.6's declarators over `int`, `char` and `void`, the
//! operators of 6.5 from an integer constant or an identifier up to the comma
//! operator, and the statements of 6.8.3 through 6.8.5: a compound statement,
//! `return`, an expression statement, `if`, `while`, `do` and both forms of `for`,
//! with `switch`, `break`, `continue`, `goto` and labels from 6.8.1, 6.8.4.2 and 6.8.6, and a struct or union with its members, anonymous ones and bit-fields too, and member access (6.7.2.1, 6.5.2.3). What
//! it does not read yet is casts,
//! `sizeof`, the type qualifiers but `const`, the storage classes but `static`, `extern` and `typedef`, a variadic function's ellipsis,
//! `volatile` and `_Atomic` in a parameter's brackets, `[*]`, and a prefixed string literal. Each arrives in a
//! sibling of the issues that built this, and each adds to [`crate::ast`]
//! rather than reshaping it. Until then each is refused as `SC0207`, valid C
//! not read yet, rather than as `SC0201`, which says the program is not C.
//! `[*]` is the exception: it is refused as `SC0207` before the parser knows
//! whether it is at function prototype scope, the only place C allows it, so
//! it may be met in a program that is not C (`docs/diagnostics.md`).
//! This list is a summary: what is refused as `SC0207`, and where, is
//! decided by `unread_keyword` and the checks that call
//! `Parser::report_unread`.
//!
//! It is handed tokens and not the source. ADR-0006 reserved a trigger, that
//! the parser asking for a `&SourceMap` is the moment an interner is worth
//! having, and this does not fire it: a name is carried as a span, resolving
//! one to text is the printer's job, and comparing one name with another is the
//! semantic analysis's.
//!
//! [`docs/frontend.md`]: https://github.com/itsakeyfut/safec/blob/main/docs/frontend.md

use crate::ast::{
    Ast, Attribute, BinOp, Declaration, Enumeration, Enumerator, EnumeratorId, Expr, ExprId,
    ForStart, Function, InitDeclarator, Item, Nullability, Parameters, Specifier, Stmt, StmtId,
    Storage, Type, TypeId, UnOp, spell_type,
};
use crate::diagnostics::{Code, Diagnostic, DiagnosticSink, Label};
use crate::preprocessor::Preprocessed;
use crate::preprocessor::cut_words;
use crate::token::{Annotation, Keyword, Punct, Token, TokenKind};
use safec_ir::ir::{Rank, Sign};
use safec_ir::source::{SourceMap, Span};

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

/// A declaration that declares nothing: specifiers and then its `;`.
///
/// C17 6.7 p2, a constraint, requires a declaration to declare a declarator, a
/// tag, or an enumeration's members, and `int;` declares none of them. Not
/// [`EXPECTED`], because no missing token would make it right: the fix is to
/// name something or delete the line. The parser's, for the reason
/// [`MISPLACED_ANNOTATION`] is, since it is the stage that knows there was no
/// declarator. A tag is the one thing that would make this valid, and
/// [`Parser::declared`] lets a struct with one through. In a member list it is
/// C17 6.7.2.1 p2's constraint instead, and the same code.
const NOTHING_DECLARED: Code = Code::new("SC0206");

/// Valid C this parser does not read yet.
///
/// Not [`EXPECTED`], because nothing is wrong with the program: a reader has
/// to be able to tell a construct this compiler has not been taught from a
/// defect in their file, and a code they can search for is how they do it.
/// The parser's counterpart of the lexer's `SC0107`, and one code for every
/// construct of this kind for the same reason [`EXPECTED`] is one code. Two
/// other refusals of valid C keep codes of their own: a braced initializer
/// the one it was given first, [`BRACED_INITIALIZER`], and an annotation
/// this compiler does not read [`UNREAD_ANNOTATION`], which is about a word
/// rather than a construct.
///
/// Which construct it is is decided where the parse failed, by
/// [`unread_keyword`] and by the few token pairs [`Parser::refuse_unread`]
/// asks about. What is not C at that position keeps [`EXPECTED`].
const UNREAD: Code = Code::new("SC0207");

/// A typedef written with an initializer, `typedef int T = 1;`. A typedef
/// declaration declares no object, only a name for a type (C17 6.7.8 p3), so
/// there is nothing to initialize. The parser's, since it is the stage that reads `typedef` and
/// resolves typedef names, which no later stage sees.
const TYPEDEF_INITIALIZED: Code = Code::new("SC0208");

/// A name declared as a typedef name and as an ordinary identifier in one
/// scope, `typedef int T; T T;` at file scope (C17 6.7 p3, 6.2.1 p4). The
/// type check's code for a name declared twice incompatibly, since that is
/// what it is; the parser reports it because only the parser knows typedef
/// names.
const REDECLARED_KIND: Code = Code::new("SC0313");

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

/// One step tighter again: the conditional operator, C17 6.5.15, which is
/// where a `case` value is read from, since 6.6's `constant-expression` is a
/// `conditional-expression`.
const CONDITIONAL: u8 = 3;

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
    /// Where this declarator began, its first token after the specifiers,
    /// which is where [`InitDeclarator::span`] starts.
    declarator: Span,
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
    /// The storage-class specifier written, if one was.
    storage: Option<Storage>,
    /// Whether the specifiers wrote `typedef`, which makes each declarator a
    /// typedef name rather than an object or a function (C17 6.7.8).
    typedef: bool,
}

/// What a declaration's specifiers were followed by.
enum Specified {
    /// A declarator with a name, which every declaration but one has.
    Named(Declared),
    /// The `;`, after specifiers that declared a tag and nothing else, which
    /// C17 6.7 p2 allows: `struct S;` and `struct S { int x; };`. The type
    /// is what they named.
    Tag(TypeId),
    /// A `:` where a member's declarator would be, after specifiers of this
    /// type: an unnamed bit-field, `int : 3;` (C17 6.7.2.1 p1), whose width
    /// is next.
    Unnamed(TypeId),
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
    /// A member of a struct, which may carry none either: what a member's
    /// pointer promises is a question about the struct, and #27's.
    Member,
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
        /// Whether `const` was written after the `*`, qualifying the pointer
        /// itself rather than what it points to.
        constant: bool,
    },
    /// C17 6.7.6.2.
    Array {
        /// How many, or `None` for `[]`.
        length: Option<ExprId>,
        /// The tokens between the brackets, for `Type::Array::written`.
        written: Option<Span>,
        /// Whether `const` was written in a parameter's brackets, which
        /// qualifies the pointer the parameter is adjusted to (C17 6.7.6.3
        /// p7): `int a[const]` is an `int *const a`.
        constant: bool,
    },
    /// C17 6.7.6.3.
    Function(Parameters),
}

/// What one identifier names in one scope, as far as the parser has to know
/// to read what follows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Named {
    /// An object or a function, which hides a typedef name of an outer scope.
    Ordinary,
    /// A typedef name, for this type.
    Typedef(TypeId),
    /// An enumeration constant (C17 6.4.4.3), which is an ordinary identifier
    /// too, and which the parser reads as [`Expr::Enumerator`].
    Enumerator(EnumeratorId),
}

/// The ordinary identifiers one scope declares. Tags are the name
/// resolver's, which reads a typedef name and an enumeration constant as
/// nothing but the type and the constant the parser resolved them to.
#[derive(Debug, Default)]
struct Scope {
    names: std::collections::HashMap<String, Named>,
}

/// One of C17 6.7.2's type specifiers that is a single keyword and names a
/// basic type, alone or with the others: `void`, `char`, `short`, `int`,
/// `long`, `signed` and `unsigned`. `struct` is more than one token and is
/// [`Parser::struct_specifier`]'s; `float`, `double`, `_Bool` and `_Complex`
/// are not read yet ([`unread_keyword`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Word {
    Void,
    Char,
    Short,
    Int,
    Long,
    Signed,
    Unsigned,
}

impl Word {
    /// The word this token is, if it is one.
    fn of(kind: TokenKind) -> Option<Self> {
        Some(match kind {
            TokenKind::Keyword(Keyword::Void) => Self::Void,
            TokenKind::Keyword(Keyword::Char) => Self::Char,
            TokenKind::Keyword(Keyword::Short) => Self::Short,
            TokenKind::Keyword(Keyword::Int) => Self::Int,
            TokenKind::Keyword(Keyword::Long) => Self::Long,
            TokenKind::Keyword(Keyword::Signed) => Self::Signed,
            TokenKind::Keyword(Keyword::Unsigned) => Self::Unsigned,
            _ => return None,
        })
    }

    /// How C writes it.
    fn spelled(self) -> &'static str {
        match self {
            Self::Void => "void",
            Self::Char => "char",
            Self::Short => "short",
            Self::Int => "int",
            Self::Long => "long",
            Self::Signed => "signed",
            Self::Unsigned => "unsigned",
        }
    }
}

/// Why [`Parser::refuse_specifier`] refuses a type specifier.
#[derive(Clone, Copy, Debug)]
enum Refused {
    /// A word no list holds beside this one written before it.
    Clash(&'static str),
    /// Any type specifier after a `struct`.
    AfterTag(&'static str),
    /// A `struct` or an `enum` after other type specifiers.
    BeforeTag(&'static str),
    /// Any type specifier after a typedef name.
    AfterTypedefName,
}

/// The words a declaration's specifiers have written so far, as the multiset
/// C17 6.7.2 p2 compares against its lists: each of them at most once but
/// `long`, which may be written twice.
#[derive(Clone, Copy, Debug, Default)]
struct Words {
    void: bool,
    char: bool,
    short: bool,
    int: bool,
    longs: u8,
    sign: Option<Word>,
}

impl Words {
    fn is_empty(self) -> bool {
        !self.void
            && !self.char
            && !self.short
            && !self.int
            && self.longs == 0
            && self.sign.is_none()
    }

    /// These words and `word`, or the word already written that 6.7.2 p2 has
    /// no list for beside it. Asked a word at a time, so that what is
    /// reported is the word that made the set one no list holds, where it is
    /// written, as `clang` reports it.
    #[inline(never)]
    fn with(self, word: Word) -> Result<Self, &'static str> {
        let mut next = self;
        let clash = |taken: bool, with: &'static str| if taken { Err(with) } else { Ok(()) };
        match word {
            Word::Void => {
                clash(self.char, "char")?;
                clash(self.short, "short")?;
                clash(self.int, "int")?;
                clash(self.longs > 0, "long")?;
                if let Some(sign) = self.sign {
                    return Err(sign.spelled());
                }
                clash(self.void, "void")?;
                next.void = true;
            }
            Word::Char => {
                clash(self.void, "void")?;
                clash(self.short, "short")?;
                clash(self.int, "int")?;
                clash(self.longs > 0, "long")?;
                clash(self.char, "char")?;
                next.char = true;
            }
            Word::Short => {
                clash(self.void, "void")?;
                clash(self.char, "char")?;
                clash(self.longs > 0, "long")?;
                clash(self.short, "short")?;
                next.short = true;
            }
            Word::Int => {
                clash(self.void, "void")?;
                clash(self.char, "char")?;
                clash(self.int, "int")?;
                next.int = true;
            }
            Word::Long => {
                clash(self.void, "void")?;
                clash(self.char, "char")?;
                clash(self.short, "short")?;
                clash(self.longs == 2, "long long")?;
                next.longs += 1;
            }
            Word::Signed | Word::Unsigned => {
                clash(self.void, "void")?;
                if let Some(sign) = self.sign {
                    return Err(sign.spelled());
                }
                next.sign = Some(word);
            }
        }
        Ok(next)
    }

    /// The type a set 6.7.2 p2 lists names, which every set [`Words::with`]
    /// built is: `void`, or an integer of the rank its words give and the
    /// sign it wrote, `signed` where it wrote none but `char`'s, which is
    /// plain (6.2.5 p15). `None` for a set with no word at all.
    #[inline(never)]
    fn ty(self) -> Option<Type> {
        if self.is_empty() {
            return None;
        }
        if self.void {
            return Some(Type::Void);
        }
        let rank = if self.char {
            Rank::Char
        } else if self.short {
            Rank::Short
        } else {
            match self.longs {
                0 => Rank::Int,
                1 => Rank::Long,
                _ => Rank::LongLong,
            }
        };
        let sign = match (self.sign, rank) {
            (Some(Word::Unsigned), _) => Sign::Unsigned,
            (Some(_), _) => Sign::Signed,
            (None, Rank::Char) => Sign::Plain,
            (None, Rank::Short | Rank::Int | Rank::Long | Rank::LongLong) => Sign::Signed,
        };
        Some(Type::Integer { rank, sign })
    }
}

/// Whether declaration specifiers begin at this token: one
/// [`Word`] names, or `struct`, which is more than one token
/// and is read by [`Parser::struct_specifier`]. The one test for where a
/// declaration, a parameter list or a `for`'s declaration begins.
///
/// The keyword half of the answer: every place that asks goes through
/// [`Parser::begins_specifiers_at`], which adds typedef names, since a name
/// alone does not say. Asked, none of them consuming, by `compound` and
/// `for_statement`, to tell a declaration from a statement (C17 6.8.2, 6.8.5);
/// `parenthesised_declarator`, to tell a parenthesised declarator from a
/// parameter list; `type_name_follows`, to tell a type name in parentheses from
/// an expression; `ends_a_label_c17_forbids`; and the recovery that finds where
/// an item ends. All of them have to agree about
/// what begins a declaration, and a second copy of the answer is what would let
/// them stop agreeing, and two spellings of one decision have already left a
/// case here answered by neither.
fn begins_specifiers(kind: TokenKind) -> bool {
    Word::of(kind).is_some()
        || matches!(
            kind,
            TokenKind::Keyword(
                Keyword::Struct
                    | Keyword::Union
                    | Keyword::Enum
                    | Keyword::Const
                    | Keyword::Static
                    | Keyword::Extern
                    | Keyword::Typedef
            )
        )
}

/// Where a parse failed, which decides what C would have allowed there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Position {
    /// Where a declaration begins at file scope.
    FileScope,
    /// Where a block-item begins and is not a statement: a declaration
    /// whose specifiers this parser does not read. C17 6.8.2 allows one in a
    /// block and 6.8.4 and 6.8.5 do not allow one as a substatement, which is
    /// why this is not [`Position::Statement`].
    BlockScope,
    /// Where a statement begins, in a block or as a substatement.
    Statement,
    /// Where a member declaration begins: C17 6.7.2.1's
    /// `specifier-qualifier-list`, or a static assertion.
    Member,
    /// Where a parameter declaration begins.
    Parameter,
    /// Where a `for`'s first clause begins, which may be a declaration, but
    /// only of an object with storage class `auto` or `register` (C17 6.8.5
    /// p3).
    ForInit,
    /// Where a primary expression begins.
    Expression,
}

/// A construct C allows where the parse failed and this parser does not read,
/// and the C17 clause that allows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Unread {
    what: &'static str,
    clause: &'static str,
}

/// What this keyword would have begun at this position, if it is valid C there
/// that this parser does not read.
///
/// One table, so every position asks the same question. Each row says which
/// positions C allows the keyword in, because one that is C in one place is a
/// constraint violation in another: `register` outside a block or a
/// parameter breaks C17 6.7.1 p6. Those keep [`EXPECTED`]. So does `_Imaginary`, which only Annex G gives a meaning;
/// `case` and `default` outside a `switch` are [`Parser::case_statement`]'s. `restrict` has no row: 6.7.3 p2 allows it only on a
/// pointer to an object, so leading a declaration it is C only before a
/// typedef name for one, and after a `*` it is [`Parser::declarator`]'s to
/// refuse.
fn unread_keyword(keyword: Keyword, at: Position) -> Option<Unread> {
    use Position::{BlockScope, Expression, FileScope, ForInit, Member, Parameter};

    // Which positions each kind of keyword belongs in. A type specifier or a
    // qualifier begins a declaration anywhere one may begin.
    const SPECIFIER: &[Position] = &[FileScope, BlockScope, Member, Parameter, ForInit];
    const STORAGE: &[Position] = &[FileScope, BlockScope];
    const AUTOMATIC: &[Position] = &[BlockScope, ForInit];

    let (what, clause, allowed): (&str, &str, &[Position]) = match keyword {
        // At file scope only: C17 6.7.1 p3 wants `static` or `extern` beside
        // it in a block, which the one token asked about here cannot see.
        Keyword::ThreadLocal => ("`_Thread_local`", "6.7.1", &[FileScope]),
        Keyword::Auto => ("`auto`", "6.7.1", AUTOMATIC),
        Keyword::Register => ("`register`", "6.7.1", &[BlockScope, Parameter, ForInit]),
        Keyword::Volatile => ("`volatile`", "6.7.3", SPECIFIER),
        // The qualifier's clause. `Parser::refuse_unread` cites 6.7.2.4 instead
        // where a `(` follows, which C17 6.7.2.4 p4 makes the type specifier.
        Keyword::Atomic => ("`_Atomic`", "6.7.3", SPECIFIER),
        Keyword::Float => ("`float`", "6.7.2", SPECIFIER),
        Keyword::Double => ("`double`", "6.7.2", SPECIFIER),
        Keyword::Bool => ("`_Bool`", "6.7.2", SPECIFIER),
        Keyword::Complex => ("`_Complex`", "6.7.2", SPECIFIER),
        Keyword::Inline => ("`inline`", "6.7.4", STORAGE),
        Keyword::Noreturn => ("`_Noreturn`", "6.7.4", STORAGE),
        Keyword::Alignas => (
            "`_Alignas`",
            "6.7.5",
            &[FileScope, BlockScope, Member, ForInit],
        ),
        Keyword::StaticAssert => (
            "`_Static_assert`",
            "6.7.10",
            &[FileScope, BlockScope, Member],
        ),
        Keyword::Alignof => ("`_Alignof`", "6.5.3.4", &[Expression]),
        Keyword::Generic => ("`_Generic`", "6.5.1.1", &[Expression]),
        _ => return None,
    };
    allowed.contains(&at).then_some(Unread { what, clause })
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
        Punct::Question => (CONDITIONAL, Right, Conditional),

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
pub fn parse(tokens: &Preprocessed, sources: &SourceMap, diagnostics: &mut DiagnosticSink) -> Ast {
    let tokens = tokens.tokens();
    // A directive never reaches the grammar, and `Preprocessed` says why none
    // can: the preprocessor read each or reported it. Asserted rather than
    // filtered, because one here is that stage's defect, and dropping it would
    // compile a line nobody read.
    debug_assert!(
        tokens
            .iter()
            .all(|token| token.kind != TokenKind::Directive),
        "a directive reached the parser, which the preprocessor should have read or reported"
    );

    // For each token, where the next brace or the end of the file is, found
    // once here rather than by a scan from every `;` recovery meets.
    let mut next_brace = vec![tokens.len(); tokens.len()];
    for at in (0..tokens.len()).rev() {
        next_brace[at] = match tokens[at].kind {
            TokenKind::Punct(Punct::LeftBrace | Punct::RightBrace) | TokenKind::Eof => at,
            _ => next_brace.get(at + 1).copied().unwrap_or(tokens.len()),
        };
    }

    Parser {
        tokens,
        sources,
        next_brace,
        at: 0,
        ast: Ast::new(),
        failed: false,
        reported: 0,
        budget: tokens.len() + 1,
        depth: 0,
        loops: 0,
        switches: 0,
        storage: None,
        typedef: false,
        names: vec![Scope::default()],
        declaring: false,
        enumerated: false,
        tag_written: false,
    }
    .run(diagnostics)
}

struct Parser<'a> {
    tokens: &'a [Token],
    /// For joining across an instance, which needs its chain of uses. See
    /// [`Parser::join`].
    sources: &'a SourceMap,
    /// For each token, the index of the first `{`, `}` or end of file at or
    /// after it, which [`Parser::body_after_declarations`] reads instead of
    /// scanning. A scan from every `;` recovery met was quadratic in a file of
    /// broken declarations with no brace in it. Found by review.
    next_brace: Vec<usize>,
    at: usize,
    ast: Ast,
    /// Whether a syntax error has been reported for the item being read.
    ///
    /// One per item, cleared by [`Parser::run`] before each. A second error
    /// inside the item that failed would be reported from a position the
    /// parser has no reason to believe in; the next item begins where
    /// [`Parser::resync`] put the cursor, which is a position it does (#35).
    failed: bool,
    /// The token the item's error was reported at, which recovery does not
    /// read again. Set by [`Parser::report_built`] with `failed`, and moved
    /// back to the last token read by [`Parser::join`], whose cut is among
    /// the tokens already consumed.
    reported: usize,
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
    /// How many `while`, `do` and `for` bodies are being read right now.
    ///
    /// C17 6.8.6.2 p1 allows `continue` only in a loop body, and 6.8.6.3 p1
    /// `break` only in a loop or `switch` body. See [`Parser::jump_statement`].
    loops: usize,
    /// How many `switch` bodies are being read right now, which is where a
    /// `case`, a `default` and a `break` that is in no loop may stand.
    switches: usize,
    /// The storage-class specifier the last [`Parser::specifiers`] read, which
    /// [`Parser::declared`] takes before anything nested in the declarator
    /// can read specifiers of its own.
    storage: Option<Storage>,
    /// Whether the last [`Parser::specifiers`] read `typedef`, taken with
    /// [`Self::storage`] for the same reason.
    typedef: bool,
    /// What each identifier names in each open scope, innermost last: the
    /// type a typedef name stands for, or `None` for an ordinary identifier
    /// declared there, which hides a typedef name of an outer scope (C17
    /// 6.2.1 p4). The parser has to know, since `T * x;` is a declaration
    /// where `T` names a type and an expression where it does not (6.7.8).
    /// File scope is the first entry and is never popped.
    names: Vec<Scope>,
    /// Whether what is being read belongs to a declaration, whose
    /// [`Stmt::Declaration::enumerators`] carries any enumeration constant
    /// declared in it. An enumeration defined anywhere else, in a cast in an
    /// expression statement or in a function definition's specifiers, has
    /// no declaration to carry it and is refused as not read yet.
    declaring: bool,
    /// Whether the last [`Parser::specifiers`] read an enumeration, so that
    /// `enum E { A };` declares something with no declarator (C17 6.7 p2).
    enumerated: bool,
    /// Whether the last [`Parser::specifiers`] wrote a struct or union
    /// specifier, rather than naming one through a typedef: what tells an
    /// anonymous member from `T;` with `T` a typedef of an untagged struct,
    /// which C17 6.7.2.1 p13 does not make one (#1207 review).
    tag_written: bool,
}

impl Parser<'_> {
    /// Read items until the file ends, recovering from one that fails at the
    /// end of that item and reading on (#35).
    ///
    /// An item that fails is kept as `Item::Error` and has reported one
    /// error, and [`Parser::resync`] moves past it. The budget is what holds
    /// termination: a recovery that did not advance would meet the same item
    /// until it ran out, and that is a panic naming the defect rather than a
    /// hang.
    ///
    /// Mutation: do not clear `failed` before each item.
    /// `three_items_with_a_syntax_error_each_report_three` reports one.
    fn run(mut self, diagnostics: &mut DiagnosticSink) -> Ast {
        while !self.at_end() {
            self.spend();
            self.failed = false;
            let start = self.at;
            let item = self.item(diagnostics);
            self.ast.push_item(item);
            if self.failed {
                self.resync(start);
            }
        }

        self.ast
    }

    /// Move the cursor to the end of the item that began at token `start`
    /// and failed, found from the tokens rather than from wherever the failed
    /// read stopped, whose state nothing vouches for.
    ///
    /// The item ends after a `;` at brace depth 0; after a `}` that brings
    /// the depth back to 0, where what follows begins an item or ends the
    /// file, which is how a function definition ends; after a `}` with no
    /// `{` open; or at the end of the file. A struct's closing brace is
    /// followed by `;` or a declarator, so `struct S { ... };` scans on to
    /// its `;`. An end is taken only past the token the item's error was
    /// reported at, [`Parser::reported`]: `struct S { int a; } const s;` is
    /// reported at `const`, and ending at the `}` before it would read
    /// `const` again and report it twice. Found by review. And never before
    /// what the failed read consumed, so nothing it took is read twice.
    ///
    /// Mutation: take an end at or before the reported token.
    /// `valid_c_this_parser_does_not_read_is_told_so_and_malformed_c_is_not`
    /// reports `const` twice.
    ///
    /// Mutation: leave the cursor at `start`. The budget runs out in
    /// `three_items_with_a_syntax_error_each_report_three`. Mutation: end at
    /// any `}` that brings the depth to 0.
    /// `recovery_ends_a_struct_at_its_semicolon_not_its_brace` reports three.
    fn resync(&mut self, start: usize) {
        let reported = self.reported;
        let mut depth = 0usize;
        let mut end = start;
        while end < self.tokens.len() && !self.tokens[end].is_eof() {
            let kind = self.tokens[end].kind;
            end += 1;
            let past = end > reported;
            match kind {
                // Not the end where a body follows a declaration list: an
                // old-style definition, C17 6.9.1 p6, puts `int a; int b;`
                // between its declarator and its `{`, and ending the item at
                // the first `;` reported that body as an item of its own.
                // Found by review.
                TokenKind::Punct(Punct::Semicolon) if depth == 0 && past => {
                    match self.body_after_declarations(end) {
                        Some(body) => end = body,
                        None => break,
                    }
                }
                TokenKind::Punct(Punct::LeftBrace) => depth += 1,
                TokenKind::Punct(Punct::RightBrace) if depth == 0 && past => break,
                TokenKind::Punct(Punct::RightBrace) if depth == 0 => {}
                TokenKind::Punct(Punct::RightBrace) => {
                    depth -= 1;
                    let next = self.tokens.get(end).copied();
                    // What begins an item includes a typedef name and what the
                    // parser refuses at the start of one, `inline` among them,
                    // or a `}` before one swallowed it. Found by review. Not
                    // a name, which is a struct's declarator here.
                    let ends_here = next.is_none_or(|next| {
                        next.is_eof()
                            || self.begins_specifiers_at(next)
                            || next.kind == TokenKind::Annotation(Annotation::Attribute)
                            || matches!(next.kind, TokenKind::Keyword(keyword)
                                if unread_keyword(keyword, Position::FileScope).is_some())
                    });
                    if depth == 0 && ends_here && past {
                        break;
                    }
                }
                _ => {}
            }
        }
        self.at = self.at.max(end);
    }

    /// Where the `{` of a body is, if the tokens from `from` are declarations
    /// ending in `;` and then that `{`, which is how an old-style definition's
    /// declaration list leads to its body.
    ///
    /// A `{` that does not follow a `;` is somebody else's: `int g(void) {`
    /// begins the next definition. So is anything after a `}`, a `{` nested
    /// in a declaration, or the end of the file. Nothing at file scope begins
    /// with `{`, so taking one that does follow a `;` hides no valid item.
    fn body_after_declarations(&self, from: usize) -> Option<usize> {
        let at = *self.next_brace.get(from)?;
        let brace = self.tokens.get(at)?;
        let after_semicolon =
            at > 0 && self.tokens[at - 1].kind == TokenKind::Punct(Punct::Semicolon);
        (brace.kind == TokenKind::Punct(Punct::LeftBrace) && after_semicolon).then_some(at)
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
        let enumerators = self.ast.enumerator_count();
        let declaring = std::mem::replace(&mut self.declaring, true);
        let item = self.item_read(enumerators, diagnostics);
        self.declaring = declaring;
        item
    }

    /// [`Parser::item`], with `enumerators` the count of enumeration
    /// constants declared before it, so that the ones it declares are the
    /// rest.
    fn item_read(&mut self, enumerators: usize, diagnostics: &mut DiagnosticSink) -> Item {
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

        let declared = match self.declared(Declares::FileScope, diagnostics) {
            Some(Specified::Named(declared)) => declared,
            // Asked for only in a member list, which this is not.
            Some(Specified::Unnamed(_)) => {
                unreachable!("an unnamed bit-field outside a member list")
            }
            Some(Specified::Tag(specified)) => {
                // A hatch is a function definition, and this is not one.
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
                self.advance();
                let span = self.join(start, self.previous().span, diagnostics);
                return Item::Declaration {
                    declarators: Vec::new(),
                    typedefs: Vec::new(),
                    specified,
                    enumerators: self.ast.enumerators_since(enumerators),
                    storage: None,
                    span,
                };
            }
            None => return Item::Error { span: first },
        };
        let specified = declared.base;
        let storage = declared.storage;

        if self.check(TokenKind::Punct(Punct::LeftBrace)) {
            if declared.typedef {
                self.report(
                    "a typedef has no body",
                    "a function definition cannot be a typedef",
                    diagnostics,
                );
                return Item::Error { span: first };
            }
            // **A function definition's declarator writes its function type**
            // (C17 6.9.1 p2, footnote 162): `typedef int F(void); F g { ... }`
            // takes it from a typedef and is not C, and was accepted (#1205
            // review).
            if declared.ty == declared.base
                && matches!(self.ast.ty(declared.ty), Type::Function { .. })
            {
                self.report(
                    "a function definition writes its parameter list",
                    "C17 6.9.1 p2: the type may not come from a typedef",
                    diagnostics,
                );
                return Item::Error { span: first };
            }
            // **An enumeration defined in a definition's specifiers or
            // parameters has no declaration to carry it**, so the name
            // resolver would meet its values nowhere. C allows it, and the
            // constants of one in the parameters are in scope in the body
            // (6.2.1 p4), so it is refused as not read yet.
            if self.ast.enumerator_count() > enumerators {
                self.report_unread(
                    Unread {
                        what: "an enumeration defined in a function definition's header",
                        clause: "6.7.2.2",
                    },
                    diagnostics,
                );
                return Item::Error { span: first };
            }
            if self
                .declare_name(declared.name, Named::Ordinary, diagnostics)
                .is_none()
            {
                return Item::Error { span: first };
            }
            // **The body's outermost block is the parameters' scope** (C17
            // 6.2.1 p4), so a typedef there that redeclares a parameter is
            // refused as one in the same scope. Read in a scope of its own,
            // it was not (#1205 review).
            self.names.push(Scope::default());
            self.declare_parameters(declared.ty, diagnostics);
            // The body's own declarations are each one, and what is between
            // them is not.
            self.declaring = false;
            let body = self.function_body(diagnostics);
            self.declaring = true;
            self.names.pop();
            let Some(body) = body else {
                return Item::Error { span: first };
            };

            let span = self.join(start, self.previous().span, diagnostics);
            return Item::Function(Function {
                ty: declared.ty,
                name: declared.name,
                body,
                span,
                attribute,
                return_nullability: declared.return_nullability,
                storage,
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

        let typedef = declared.typedef;
        let Some(declarators) = self.init_declarator_list(declared, diagnostics) else {
            return Item::Error { span: start };
        };

        if self
            .expect(TokenKind::Punct(Punct::Semicolon), "`;`", diagnostics)
            .is_none()
        {
            return Item::Error { span: start };
        }

        let span = self.join(start, self.previous().span, diagnostics);
        Item::Declaration {
            // **A typedef declares no object**, so it leaves nothing to type
            // or lower: what its names stand for is resolved here, and the
            // tag a `typedef struct S { ... } T;` declares is kept as any tag
            // declaration's is.
            typedefs: if typedef {
                declarators.iter().map(|d| d.declaration.ty).collect()
            } else {
                Vec::new()
            },
            declarators: if typedef { Vec::new() } else { declarators },
            specified,
            enumerators: self.ast.enumerators_since(enumerators),
            storage,
            span,
        }
    }

    /// The specifiers and one declarator, which is how a declaration and a
    /// definition both begin.
    fn declared(
        &mut self,
        declares: Declares,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<Specified> {
        let start = self.peek().span;
        let at = match declares {
            Declares::FileScope => Position::FileScope,
            Declares::BlockScope => Position::BlockScope,
            Declares::Member => Position::Member,
            Declares::Parameter => Position::Parameter,
        };
        let base = self.specifiers(at, diagnostics)?;
        let storage = self.storage.take();
        let typedef = std::mem::take(&mut self.typedef);
        let enumerated = std::mem::take(&mut self.enumerated);
        // C17 6.7 p2: a declaration may declare a tag and nothing else, and a
        // struct with a tag is the one this compiler reads. In a member list
        // any struct goes back to `members`, which tells an anonymous member
        // from one that declares nothing.
        if self.check(TokenKind::Punct(Punct::Semicolon)) {
            if let Type::Struct { tag, .. } = self.ast.ty(base) {
                if tag.is_some() || declares == Declares::Member {
                    return Some(Specified::Tag(base));
                }
            }
            // An enumeration with a list declares its constants. In a member
            // list it declares no member, which 6.7.2.1 p2 asks of a member
            // declaration.
            if enumerated && declares != Declares::Member {
                return Some(Specified::Tag(base));
            }
        }
        // A bit-field with no name, `int : 3;`, before a name is asked for.
        if declares == Declares::Member && self.check(TokenKind::Punct(Punct::Colon)) {
            return Some(Specified::Unnamed(base));
        }
        // Before asking for a name, so that `int;` is told what it is rather
        // than that a name is missing. `int *;` has a declarator and goes on,
        // to be refused as one without a name, which is a syntax error.
        if self.check(TokenKind::Punct(Punct::Semicolon)) {
            // Through `report_built`, so that it is the only report: this
            // returns before the `;` is read, and whatever is asked of it next
            // would otherwise be refused as well.
            let specifiers = self.join(start, self.previous().span, diagnostics);
            let built = Diagnostic::error("this declaration declares nothing")
                .with_code(NOTHING_DECLARED)
                .with_label(Label::primary(specifiers, "no name and no tag"))
                .with_note("C17 6.7 p2");
            self.report_built(specifiers, built, diagnostics);
            return None;
        }
        let declarator = self.peek().span;
        let (name, ty, returns) = self.named_declarator(start, base, declares, diagnostics)?;

        Some(Specified::Named(Declared {
            start,
            base,
            declarator,
            name,
            ty,
            return_nullability: returns,
            declares,
            storage,
            typedef,
        }))
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
            mut declarator,
            mut name,
            mut ty,
            return_nullability: mut returns,
            declares,
            storage: _,
            typedef,
        } = declared;
        let mut declarators = Vec::new();

        loop {
            // **In scope from the end of its declarator** (C17 6.2.1 p7), so
            // `char T = 1, n = sizeof(T);` measures the `char` and `typedef
            // int T, U[sizeof(T)];` the `int`. Declared after the whole list,
            // the first read `sizeof(T)` as the outer typedef's and built
            // wrong code (#1205 review).
            let named = if typedef {
                Named::Typedef(ty)
            } else {
                Named::Ordinary
            };
            self.declare_name(name, named, diagnostics)?;
            let init = self.initializer(diagnostics)?;
            if typedef && init.is_some() {
                self.report_at(
                    declarator,
                    TYPEDEF_INITIALIZED,
                    "a typedef cannot be initialized",
                    "a typedef name declares no object",
                    diagnostics,
                );
                return None;
            }
            let end = self.previous().span;
            let own = self.try_join(declarator, end);
            declarators.push(InitDeclarator {
                declaration: Declaration {
                    name: Some(name),
                    ty,
                    // Not a parameter, so nothing is adjusted.
                    written: ty,
                    span: self.declaration_span(start, declarator, end, diagnostics),
                    // The pointer a function declared at file scope returns,
                    // the one place besides a parameter `apply` allows one.
                    nullability: returns,
                    width: None,
                },
                init,
                span: own,
            });

            if !self.eat(TokenKind::Punct(Punct::Comma)) {
                return Some(declarators);
            }

            // The `,` is eaten before anything else is read, so a round of this
            // loop always consumes a token and `spend`'s budget is not what
            // stops it going round forever.
            declarator = self.peek().span;
            (name, ty, returns) = self.named_declarator(declarator, base, declares, diagnostics)?;
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
            span: self.join(start, self.previous().span, diagnostics),
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
    ///
    /// `at` is where this declaration began, which decides what else C would
    /// have allowed in place of a specifier this parser reads.
    fn specifiers(&mut self, at: Position, diagnostics: &mut DiagnosticSink) -> Option<TypeId> {
        if self.check(TokenKind::Annotation(Annotation::Attribute)) {
            self.misplaced_attribute(diagnostics);
            return None;
        }

        // C17 6.7 p2 takes the specifiers in any order, so `const` may come
        // before the type specifier or after it, and a qualifier written twice
        // is one (6.7.3 p5).
        // `static` and `extern` likewise, at file scope and in a block, which
        // is where C17 6.7.1 lets a declaration name a storage class; one per
        // declaration (p2).
        //
        // **Held here and handed over at the end**, because a struct's body
        // is read between the two halves and each member's specifiers are
        // read by this function too: kept in the parser throughout, a member
        // cleared the outer `static` of `static struct S { int a; } *p;`.
        // Mutation: write each half straight into `Parser::storage`;
        // `a_storage_class_before_a_struct_definition_is_kept` fails.
        //
        // **The type is a multiset of words** (6.7.2 p2), in any order and
        // among the qualifiers and the storage class: `long unsigned const
        // int` is an `unsigned long`. Each word is checked as it is read, see
        // [`Words::with`], and a `struct` stands alone.
        let scoped = matches!(at, Position::FileScope | Position::BlockScope);
        let mut storage = None;
        let mut typedef = false;
        let mut constant = false;
        let mut words = Words::default();
        constant |=
            self.eat_qualifiers_and_storage(scoped, (&mut storage, &mut typedef), diagnostics)?;
        // A `struct` stands alone, so it is read before the loop rather than
        // in it: called from inside the loop, the refusal of a struct nested
        // past `MAX_NESTING` cost some 65 KB more stack, and one nested to the
        // limit overflowed the stack rather than being refused, measured.
        self.enumerated = false;
        let tagged = if self.check(TokenKind::Keyword(Keyword::Struct)) {
            Some((self.struct_specifier(false, diagnostics)?, "struct"))
        } else if self.check(TokenKind::Keyword(Keyword::Union)) {
            Some((self.struct_specifier(true, diagnostics)?, "union"))
        } else if self.check(TokenKind::Keyword(Keyword::Enum)) {
            Some((self.enum_specifier(diagnostics)?, "enum"))
        } else {
            None
        };
        let enumerated = std::mem::take(&mut self.enumerated);
        let tagged_keyword = |next: TokenKind| {
            matches!(
                next,
                TokenKind::Keyword(Keyword::Struct | Keyword::Union | Keyword::Enum)
            )
        };
        // **A typedef name is a type specifier only where no other is
        // written** (C17 6.7.2 p2): in `T x;` the `T` is the type, and in
        // `int T;` it is the declarator, which hides the typedef name.
        let mut named = None;
        loop {
            constant |=
                self.eat_qualifiers_and_storage(scoped, (&mut storage, &mut typedef), diagnostics)?;
            let next = self.peek().kind;
            if tagged.is_none() && named.is_none() && words.is_empty() {
                if let Some(ty) = self.typedef_name(self.peek()) {
                    self.advance();
                    named = Some(ty);
                    continue;
                }
            }
            if named.is_some() && (Word::of(next).is_some() || tagged_keyword(next)) {
                return self.refuse_specifier(Refused::AfterTypedefName, diagnostics);
            }
            if let Some(word) = Word::of(next) {
                if let Some((_, keyword)) = tagged {
                    return self.refuse_specifier(Refused::AfterTag(keyword), diagnostics);
                }
                match words.with(word) {
                    Ok(next) => words = next,
                    Err(previous) => {
                        return self.refuse_specifier(Refused::Clash(previous), diagnostics);
                    }
                }
                self.advance();
                continue;
            }
            if tagged_keyword(next) {
                let keyword = match next {
                    TokenKind::Keyword(Keyword::Struct) => "struct",
                    TokenKind::Keyword(Keyword::Union) => "union",
                    _ => "enum",
                };
                return self.refuse_specifier(
                    match tagged {
                        Some((_, before)) => Refused::AfterTag(before),
                        None => Refused::BeforeTag(keyword),
                    },
                    diagnostics,
                );
            }
            break;
        }
        self.storage = storage;
        self.typedef = typedef;
        // Set after every member's specifiers were read, so this is the
        // outer declaration's answer: an `enum` with a list here, which is
        // what set it last.
        self.enumerated = enumerated;
        self.tag_written = tagged.is_some();
        let ty = match (tagged, named, words.ty()) {
            (Some((tagged, _)), _, _) => tagged,
            (None, Some(named), _) => named,
            (None, None, Some(ty)) => self.ast.push_type(ty),
            (None, None, None) => return self.no_type_specifier(at, diagnostics),
        };
        let ty = if constant {
            self.ast.push_const(ty)
        } else {
            ty
        };
        self.no_unread_specifier_after(ty, at, diagnostics)
    }

    /// Report where declaration specifiers named no type: valid C this
    /// compiler does not read yet, or no declaration at all. Out of
    /// [`Parser::specifiers`]' frame for the reason
    /// [`Parser::refuse_specifier`] gives.
    #[inline(never)]
    #[cold]
    fn no_type_specifier(
        &mut self,
        at: Position,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<TypeId> {
        if self.refuse_unread(at, diagnostics) {
            return None;
        }
        self.report(
            "expected a declaration",
            "this cannot begin one",
            diagnostics,
        );
        None
    }

    /// Report a type specifier C17 6.7.2 p2 has no list for beside the ones
    /// before it, where it is written, and answer nothing.
    ///
    /// **Its own function, never inlined**, because [`Parser::specifiers`]
    /// is on the path a struct nested in a struct recurses through, once per
    /// level up to [`MAX_NESTING`], and a diagnostic built in its body made
    /// its frame large enough that a struct nested to the limit overflowed
    /// the stack, where it is refused as nested too deep.
    /// `a_struct_nested_past_the_limit_is_refused_rather_than_read` is what
    /// found it.
    #[inline(never)]
    #[cold]
    fn refuse_specifier(
        &mut self,
        refused: Refused,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<TypeId> {
        let (message, label) = match refused {
            Refused::Clash("long long") => (
                "`long long long` is too long".to_owned(),
                "C17 6.7.2 p2 lists no type with these specifiers",
            ),
            Refused::Clash(previous) => (
                format!("cannot combine with previous `{previous}` declaration specifier"),
                "C17 6.7.2 p2 lists no type with these specifiers",
            ),
            Refused::AfterTag(keyword) => (
                format!("cannot combine with previous `{keyword}` declaration specifier"),
                "a second type",
            ),
            Refused::BeforeTag(keyword) => (
                format!(
                    "cannot combine {} `{keyword}` with the type specifiers before it",
                    if keyword == "enum" { "an" } else { "a" }
                ),
                "a second type",
            ),
            Refused::AfterTypedefName => (
                "cannot combine a typedef name with another type specifier".to_owned(),
                "a second type",
            ),
        };
        self.report(message, label, diagnostics);
        None
    }

    /// Every `const` here, and every storage class where `scoped` says one
    /// may stand, recorded in `storage`; whether there was a `const`. A
    /// second storage class is a constraint violation (C17 6.7.1 p2),
    /// reported where it is written.
    #[inline(never)]
    fn eat_qualifiers_and_storage(
        &mut self,
        scoped: bool,
        (storage, typedef): (&mut Option<Storage>, &mut bool),
        diagnostics: &mut DiagnosticSink,
    ) -> Option<bool> {
        let mut constant = false;
        loop {
            if self.eat(TokenKind::Keyword(Keyword::Const)) {
                constant = true;
                continue;
            }
            // `typedef` is a storage class in the grammar only (C17 6.7.1 p5),
            // and one per declaration like the rest (p2).
            let written = match self.peek().kind {
                TokenKind::Keyword(Keyword::Static) if scoped => Some(Storage::Static),
                TokenKind::Keyword(Keyword::Extern) if scoped => Some(Storage::Extern),
                TokenKind::Keyword(Keyword::Typedef) if scoped => None,
                _ => return Some(constant),
            };
            if storage.is_some() || *typedef {
                self.report(
                    "a declaration names at most one storage class",
                    "a second storage class",
                    diagnostics,
                );
                return None;
            }
            self.advance();
            match written {
                Some(written) => *storage = Some(written),
                None => *typedef = true,
            }
        }
    }

    /// The type `token` names, where it is an identifier the innermost
    /// scope that declares it declares as a typedef name.
    fn typedef_name(&self, token: Token) -> Option<TypeId> {
        match self.named(token)? {
            Named::Typedef(ty) => Some(ty),
            Named::Ordinary | Named::Enumerator(_) => None,
        }
    }

    /// What `token` names, where it is an identifier some open scope declares:
    /// the innermost declaration of it.
    fn named(&self, token: Token) -> Option<Named> {
        if token.kind != TokenKind::Identifier {
            return None;
        }
        let text = self.sources.snippet(token.span);
        self.names
            .iter()
            .rev()
            .find_map(|scope| scope.names.get(text))
            .copied()
    }

    /// Whether specifiers begin at `token`: [`begins_specifiers`], or a
    /// typedef name. The one test every place that tells a declaration from
    /// an expression asks.
    fn begins_specifiers_at(&self, token: Token) -> bool {
        begins_specifiers(token.kind) || self.typedef_name(token).is_some()
    }

    /// Declare `name` in the innermost scope as `named`, which hides whatever
    /// an outer scope declares it as. **Not as another kind in the same
    /// scope**: `typedef int T; T T;` at file scope declares `T` twice in one
    /// name space, which C17 6.7 p3 forbids, and was accepted (#1205 review).
    /// Nor an enumeration constant again, as anything: 6.7 p3 allows only a
    /// typedef and a tag a second declaration. Two ordinary declarations of
    /// one name are sema's to judge. `None` once reported.
    fn declare_name(
        &mut self,
        name: Span,
        named: Named,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<()> {
        let text = self.sources.snippet(name).to_owned();
        let already = self
            .names
            .last()
            .and_then(|scope| scope.names.get(&text))
            .copied();
        // **A typedef name may be declared again in its scope only as the
        // same type** (C17 6.7 p3, as C11 relaxed it): `typedef int T;
        // typedef long T;` was accepted, the second silently winning (#1205
        // review). Compared by spelling, since the parser has no type
        // compatibility of its own and two spellings of one type are spelled
        // alike once resolved.
        if let (Some(Named::Typedef(before)), Named::Typedef(now)) = (already, named) {
            if spell_type(self.sources, &self.ast, before)
                != spell_type(self.sources, &self.ast, now)
            {
                self.report_at(
                    name,
                    REDECLARED_KIND,
                    format!("typedef redefinition of `{text}` with a different type"),
                    "declared in this scope already as another type",
                    diagnostics,
                );
                return None;
            }
        }
        let clash = match (already, named) {
            (None, _)
            | (Some(Named::Ordinary), Named::Ordinary)
            | (Some(Named::Typedef(_)), Named::Typedef(_)) => false,
            (Some(Named::Enumerator(_)), _)
            | (Some(_), Named::Enumerator(_))
            | (Some(Named::Ordinary), Named::Typedef(_))
            | (Some(Named::Typedef(_)), Named::Ordinary) => true,
        };
        if clash {
            self.report_at(
                name,
                REDECLARED_KIND,
                format!("redefinition of `{text}` as a different kind of name"),
                "declared in this scope already",
                diagnostics,
            );
            return None;
        }
        if let Some(scope) = self.names.last_mut() {
            scope.names.insert(text, named);
        }
        Some(())
    }

    /// The names a function definition's parameters declare, in the scope
    /// its body is read in (C17 6.2.1 p4), so a parameter hides a typedef
    /// name of the same spelling there.
    fn declare_parameters(&mut self, function: TypeId, diagnostics: &mut DiagnosticSink) {
        if let Type::Function {
            parameters: Parameters::Prototype(parameters),
            ..
        } = self.ast.ty(function)
        {
            let names: Vec<Span> = parameters.iter().filter_map(|p| p.name).collect();
            for name in names {
                // Each parameter is in a scope of its own here, the body's, so
                // two of one name are sema's to report, not this.
                let _ = self.declare_name(name, Named::Ordinary, diagnostics);
            }
        }
    }

    /// Every `const` here, and whether there was one.
    fn eat_consts(&mut self) -> bool {
        let mut any = false;
        while self.eat(TokenKind::Keyword(Keyword::Const)) {
            any = true;
        }
        any
    }

    /// `ty`, unless what follows it is a second specifier C allows here and this
    /// parser does not read, which is refused as one.
    ///
    /// C17 6.7 p2 lets declaration specifiers come in any order, so `int const`
    /// and `int unsigned` are as valid as `const int` and `unsigned int`, and
    /// the second word is what this parser does not read. Without this it is
    /// read as the declarator, which then says a name is missing.
    #[inline(never)]
    fn no_unread_specifier_after(
        &mut self,
        ty: TypeId,
        at: Position,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<TypeId> {
        if self.refuse_unread(at, diagnostics) {
            return None;
        }
        Some(ty)
    }

    /// C17 6.7.2.2's `enum-specifier`: `enum Tag { A, B = 5, C }`, or
    /// `enum Tag` for one declared before, as an enumerated type: see
    /// [`crate::ast::Enumeration`] for what it is and who decides its sign.
    /// Which enumeration a tag names is the name resolver's, as a struct's is,
    /// since the two share one name space (6.7.2.3 p2).
    ///
    /// Each constant is declared once its definition has been read, value and
    /// all (C17 6.2.1 p7), so `enum { A = A }` reads an outer `A`.
    fn enum_specifier(&mut self, diagnostics: &mut DiagnosticSink) -> Option<TypeId> {
        self.advance();

        let tag = if self.check(TokenKind::Identifier) {
            Some(self.advance().span)
        } else {
            None
        };

        if !self.check(TokenKind::Punct(Punct::LeftBrace)) {
            if tag.is_none() {
                self.report(
                    "expected a tag or `{`",
                    "a tag or `{` is missing here",
                    diagnostics,
                );
                return None;
            }
            return Some(self.ast.push_enumeration(Enumeration {
                tag,
                constants: None,
            }));
        }

        if !self.declaring {
            self.report_unread(
                Unread {
                    what: "an enumeration defined outside a declaration",
                    clause: "6.7.2.2",
                },
                diagnostics,
            );
            return None;
        }
        self.advance();

        let mut constants = Vec::new();
        let mut previous = None;
        loop {
            if previous.is_some() && self.check(TokenKind::Punct(Punct::RightBrace)) {
                break;
            }
            let name = self.expect(
                TokenKind::Identifier,
                "an enumeration constant",
                diagnostics,
            )?;
            let written = if self.eat(TokenKind::Punct(Punct::Equal)) {
                Some(self.infix_from(CONDITIONAL, diagnostics))
            } else {
                None
            };
            let enumerator = self.ast.push_enumerator(Enumerator {
                name,
                written,
                previous,
            });
            self.declare_name(name, Named::Enumerator(enumerator), diagnostics)?;
            constants.push(enumerator);
            previous = Some(enumerator);
            if !self.eat(TokenKind::Punct(Punct::Comma)) {
                break;
            }
        }
        self.expect(TokenKind::Punct(Punct::RightBrace), "`}`", diagnostics)?;
        // A list declares its constants, which is what lets `enum E { A };`
        // declare nothing else (6.7 p2). `enum E;` alone declares nothing:
        // footnote 131 says the form a struct has for it does not exist here.
        self.enumerated = true;
        Some(self.ast.push_enumeration(Enumeration {
            tag,
            constants: Some(constants),
        }))
    }

    /// `struct`, an optional tag, and an optional member list. C17 6.7.2.1.
    ///
    /// One of the two has to be there: `struct` alone names nothing. The
    /// member list goes through [`Parser::deeper`], because a member can be a
    /// struct and so can its, and the tree would otherwise be as deep as the
    /// source cared to make it.
    ///
    /// **Never inlined**, for the reason [`Parser::refuse_specifier`] gives:
    /// this is on the path a nested struct recurses through, and inlined
    /// into [`Parser::specifiers`] its frame was paid at every level, which
    /// overflowed the stack before the nesting limit was reached.
    #[inline(never)]
    fn struct_specifier(
        &mut self,
        union: bool,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<TypeId> {
        self.advance();

        let tag = if self.check(TokenKind::Identifier) {
            Some(self.advance().span)
        } else {
            None
        };

        let members = if self.check(TokenKind::Punct(Punct::LeftBrace)) {
            self.advance();
            Some(self.deeper(
                diagnostics,
                |parser, diagnostics| parser.members(diagnostics),
                |_, _| None,
            )?)
        } else {
            None
        };

        if tag.is_none() && members.is_none() {
            self.report(
                "expected a tag or `{`",
                "a tag or `{` is missing here",
                diagnostics,
            );
            return None;
        }

        Some(self.ast.push_type(Type::Struct {
            union,
            tag,
            members,
        }))
    }

    /// The member declarations of a struct through its `}`.
    ///
    /// Each is C17 6.7.2.1 p1's `struct-declaration`: specifiers and at least
    /// one named declarator, with no initializer, or an untagged struct or
    /// union specifier with none at all, p13's anonymous member (#1207). A bit-field's `:` and an
    /// `=` are refused as the `;` they are not, and so is a `}` before any
    /// member, since the grammar asks for one and `clang -pedantic-errors`
    /// calls `struct S {};` a GNU extension.
    fn members(&mut self, diagnostics: &mut DiagnosticSink) -> Option<Vec<Declaration>> {
        let mut members = Vec::new();
        loop {
            if self.check(TokenKind::Punct(Punct::RightBrace)) {
                if members.is_empty() {
                    self.report(
                        "expected a member",
                        "a struct has at least one",
                        diagnostics,
                    );
                    return None;
                }
                self.advance();
                return Some(members);
            }

            let at = self.peek().span;
            let declared = match self.declared(Declares::Member, diagnostics)? {
                Specified::Named(declared) => declared,
                // **An anonymous member** (#1207): a struct or union with
                // neither a tag nor a declarator, C17 6.7.2.1 p13, whose
                // members are the containing one's. A member with no name,
                // whose type is the anonymous one. **Written here**: `T;`
                // with `T` a typedef of an untagged struct is not one, and
                // declares nothing, which `clang` lays out so off MSVC.
                // Mutation: drop `tag_written`;
                // `a_typedef_of_an_untagged_struct_is_no_anonymous_member` is
                // accepted.
                Specified::Tag(base)
                    if self.tag_written
                        && matches!(
                            self.ast.ty(base),
                            Type::Struct {
                                tag: None,
                                members: Some(_),
                                ..
                            }
                        ) =>
                {
                    let end = self.previous().span;
                    members.push(Declaration {
                        name: None,
                        ty: base,
                        written: base,
                        span: self.declaration_span(at, at, end, diagnostics),
                        nullability: None,
                        width: None,
                    });
                    self.expect(TokenKind::Punct(Punct::Semicolon), "`;`", diagnostics)?;
                    continue;
                }
                Specified::Tag(_) => {
                    self.declares_no_member(at, diagnostics);
                    return None;
                }
                Specified::Unnamed(base) => {
                    let width = self.bit_width(diagnostics)?;
                    let end = self.previous().span;
                    members.push(Declaration {
                        name: None,
                        ty: base,
                        written: base,
                        span: self.declaration_span(at, at, end, diagnostics),
                        nullability: None,
                        width,
                    });
                    self.expect(TokenKind::Punct(Punct::Semicolon), "`;`", diagnostics)?;
                    continue;
                }
            };
            let Declared {
                start,
                base,
                declarator,
                name,
                ty,
                return_nullability,
                declares,
                storage: _,
                // A member has no storage class, so `specifiers` never reads
                // `typedef` here.
                typedef: _,
            } = declared;
            let width = self.bit_width(diagnostics)?;
            let end = self.previous().span;
            members.push(Declaration {
                name: Some(name),
                ty,
                written: ty,
                span: self.declaration_span(start, declarator, end, diagnostics),
                nullability: return_nullability,
                width,
            });
            while self.eat(TokenKind::Punct(Punct::Comma)) {
                let at = self.peek().span;
                let (name, ty, returns) = self.named_declarator(at, base, declares, diagnostics)?;
                let width = self.bit_width(diagnostics)?;
                let end = self.previous().span;
                members.push(Declaration {
                    name: Some(name),
                    ty,
                    written: ty,
                    span: self.declaration_span(start, at, end, diagnostics),
                    nullability: returns,
                    width,
                });
            }
            self.expect(TokenKind::Punct(Punct::Semicolon), "`;`", diagnostics)?;
        }
    }

    /// A member declaration of a struct type with no declarator that is not
    /// an anonymous member: one with a tag, which declares nothing, and
    /// 6.7.2.1 p2 forbids that in a member list, since `struct T;` names a
    /// tag a member list has nowhere to put.
    ///
    /// `at` is where the member declaration began, which is where the report
    /// points: the parser is standing on its `;` by now.
    fn declares_no_member(&mut self, at: Span, diagnostics: &mut DiagnosticSink) {
        let built = Diagnostic::error("this member declares nothing")
            .with_code(NOTHING_DECLARED)
            .with_label(Label::primary(at, "a tag, and no member's name"))
            .with_note("C17 6.7.2.1 p2");
        self.report_built(at, built, diagnostics);
    }

    /// A bit-field's width, `: constant-expression` (C17 6.7.2.1 p1), where
    /// a `:` follows a member's declarator, and `None` where none does
    /// (#1206). Read as a conditional expression, as a `case` label's is;
    /// that it is a constant, and fits, is the type check's to say.
    fn bit_width(&mut self, diagnostics: &mut DiagnosticSink) -> Option<Option<ExprId>> {
        if !self.eat(TokenKind::Punct(Punct::Colon)) {
            return Some(None);
        }
        Some(Some(self.infix_from(CONDITIONAL, diagnostics)))
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
            let mut nullability = self.nullability(diagnostics)?;
            // C17 6.7.6.1 lets a pointer carry qualifiers after its `*`, and
            // this parser reads `const` of them. The rest are asked once,
            // after the nullability and any `const`, which is enough: a
            // qualifier before the nullability leaves it unread and is the
            // token here, and one after it is the token here too.
            let constant = self.eat_consts();
            // clang reads a nullability after the qualifiers as well as
            // before them, `*const _Nonnull`, and headers write both. Found
            // by review.
            if constant && nullability.is_none() {
                nullability = self.nullability(diagnostics)?;
            }
            if matches!(
                self.peek().kind,
                TokenKind::Keyword(Keyword::Volatile | Keyword::Restrict | Keyword::Atomic)
            ) {
                self.report_unread(
                    Unread {
                        what: "a qualifier after `*`",
                        clause: "6.7.6.1",
                    },
                    diagnostics,
                );
                return None;
            }
            derivations.push(Derivation::Pointer {
                nullability,
                constant,
            });
        }

        let (name, inner) = self.core(named, diagnostics)?;

        // The suffix written last is the first to wrap the base, because
        // 6.7.6.2 p3 reads `D [ ... ]` by handing "array of T" to `D`. That is
        // what makes `int a[3][5]` three arrays of five and not five of three.
        // An abstract declarator is allowed only for a parameter, so `named`
        // being false is what says this declarator declares one. C17 6.7.6.2 p1
        // lets `static` and qualifiers into the brackets of its outermost array
        // derivation only, and a parenthesised declarator with derivations of
        // its own puts the array inside them.
        let mut suffixes = self.suffixes(!named, !named && inner.is_empty(), diagnostics)?;
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
    #[inline(never)]
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
    /// declarator begins with `*`, `(` or a name. A typedef name is what makes
    /// this genuinely ambiguous (C17 6.7.6.3 p11): `int (T)` with `T` a typedef
    /// name is a parameter list, so [`Parser::begins_specifiers_at`] answers.
    fn parenthesised_declarator(&self) -> bool {
        let after = self.peek_at(1);
        !self.begins_specifiers_at(after) && after.kind != TokenKind::Punct(Punct::RightParen)
    }

    /// The `[...]` and `(...)` that follow a direct-declarator, in source order.
    ///
    /// `parameter` says the declarator declares a parameter, whose prototype
    /// scope is where C17 6.7.6.2 p4 allows `[*]`. `outermost` says the first
    /// of these is its outermost array derivation, the one place 6.7.6.2 p1
    /// and 6.7.6.3 p7 let `static` and qualifiers into an array's brackets.
    fn suffixes(
        &mut self,
        parameter: bool,
        outermost: bool,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<Vec<Derivation>> {
        let mut suffixes = Vec::new();

        loop {
            if self.check(TokenKind::Punct(Punct::LeftBracket)) {
                self.spend();
                self.advance();

                if parameter {
                    let qualified = matches!(
                        self.peek().kind,
                        TokenKind::Keyword(
                            Keyword::Static
                                | Keyword::Const
                                | Keyword::Volatile
                                | Keyword::Restrict
                                | Keyword::Atomic
                        )
                    );
                    if qualified && outermost && suffixes.is_empty() {
                        let (constant, length, written) = self.bracket_qualifiers(diagnostics)?;
                        suffixes.push(Derivation::Array {
                            length,
                            written,
                            constant,
                        });
                        continue;
                    }
                    if self.star_closes() {
                        self.unread_star(diagnostics);
                        return None;
                    }
                }

                // 6.7.6 spells what goes here `assignment-expression` and not
                // `expression`, so a comma ends the subscript rather than being
                // an operator inside it.
                let (length, written) = if self.check(TokenKind::Punct(Punct::RightBracket)) {
                    (None, None)
                } else {
                    let start = self.peek().span;
                    let length = self.assignment(diagnostics);
                    (
                        Some(length),
                        Some(self.join(start, self.previous().span, diagnostics)),
                    )
                };

                self.expect(TokenKind::Punct(Punct::RightBracket), "`]`", diagnostics)?;
                suffixes.push(Derivation::Array {
                    length,
                    written,
                    constant: false,
                });
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

    /// Whether the brackets hold `*` alone from here, C17 6.7.6.2's `[*]`.
    fn star_closes(&self) -> bool {
        self.check(TokenKind::Punct(Punct::Star))
            && self.peek_at(1).kind == TokenKind::Punct(Punct::RightBracket)
    }

    /// Refuse `[*]`, with or without qualifiers before the `*`.
    ///
    /// **Not `report_unread`, whose words say this is valid here.** C17
    /// 6.7.6.2 p4 allows `[*]` only at function prototype scope, and a
    /// definition's own parameters are not there; this is met before the
    /// parser has seen whether a body follows. So the words say where C
    /// allows it, which holds at every position, and `docs/diagnostics.md`
    /// records the exception to `SC0207` (#532).
    fn unread_star(&mut self, diagnostics: &mut DiagnosticSink) {
        self.report_noted(
            UNREAD,
            "`[*]` is not read yet",
            "allowed only at function prototype scope, and not read yet",
            "C17 6.7.6.2 p4 allows `[*]` only at function prototype scope, which a \
             definition's own parameters are not at; this compiler does not read \
             it anywhere yet",
            diagnostics,
        );
    }

    /// The brackets of a parameter's outermost array derivation, from just
    /// after its `[` through its `]`, where they begin with `static` or a
    /// qualifier: C17 6.7.6.2 p1's `[ type-qualifier-list_opt
    /// assignment-expression_opt ]`, `[ static type-qualifier-list_opt
    /// assignment-expression ]` and `[ type-qualifier-list static
    /// assignment-expression ]`. Whether `const` was among them, the length,
    /// and the tokens of the length.
    ///
    /// **`static` says the argument has at least that many elements** (6.7.6.3
    /// p7), and nothing here keeps it: the memory check believes a parameter
    /// as the pointer it is adjusted to, as it does without one, which is the
    /// conclusion that does not depend on a caller keeping the promise.
    /// `restrict` permits an optimization this compiler does not make, so
    /// reading it and keeping nothing is what it means here. `volatile` and
    /// `_Atomic` are qualifiers this compiler does not read anywhere, and are
    /// refused as not read yet, as they are elsewhere.
    fn bracket_qualifiers(
        &mut self,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<(bool, Option<ExprId>, Option<Span>)> {
        let mut constant = false;
        let mut is_static = false;
        // 6.7.6.2 p1 writes `static` once, before the qualifiers or after
        // them, and not between two lists of them: `[const static const 2]`
        // is not C. Mutation: drop this; `[const static const 2]` in
        // `valid_c_this_parser_does_not_read_is_told_so_and_malformed_c_is_not`
        // is read.
        let mut qualified_before_static = false;
        loop {
            let qualifier = matches!(
                self.peek().kind,
                TokenKind::Keyword(Keyword::Const | Keyword::Restrict)
            );
            if qualifier && is_static && qualified_before_static {
                break;
            }
            qualified_before_static |= qualifier && !is_static;
            match self.peek().kind {
                TokenKind::Keyword(Keyword::Const) => constant = true,
                TokenKind::Keyword(Keyword::Restrict) => {}
                TokenKind::Keyword(Keyword::Static) if !is_static => is_static = true,
                TokenKind::Keyword(Keyword::Static) => {
                    self.report(
                        "`static` written twice in an array parameter's brackets",
                        "C17 6.7.6.2 p1 writes it once",
                        diagnostics,
                    );
                    return None;
                }
                TokenKind::Keyword(Keyword::Volatile | Keyword::Atomic) => {
                    self.report_unread(
                        Unread {
                            what: "`volatile` or `_Atomic` in an array parameter's brackets",
                            clause: "6.7.6.3 p7",
                        },
                        diagnostics,
                    );
                    return None;
                }
                _ => break,
            }
            self.advance();
        }
        // `[const *]` is `[*]` with a qualifier, which is valid where `[*]`
        // is, and not read where `[*]` is not. Mutation: drop this;
        // `a_star_length_says_where_c_allows_it_rather_than_that_it_is_allowed_here`
        // gets `SC0201` for `[const *]`.
        if !is_static && self.star_closes() {
            self.unread_star(diagnostics);
            return None;
        }
        if self.check(TokenKind::Punct(Punct::RightBracket)) {
            if is_static {
                self.report(
                    "`static` may not be used without an array size",
                    "C17 6.7.6.2 p1 writes a length after `static`",
                    diagnostics,
                );
                return None;
            }
            self.advance();
            return Some((constant, None, None));
        }
        let start = self.peek().span;
        let before = self.previous().span;
        let length = self.assignment(diagnostics);
        // A length that read no token has been reported, and has no span to
        // join: `[static ;` ends here rather than at a span that ends before
        // it starts.
        if self.previous().span == before {
            return None;
        }
        let written = self.join(start, self.previous().span, diagnostics);
        self.expect(TokenKind::Punct(Punct::RightBracket), "`]`", diagnostics)?;
        Some((constant, Some(length), Some(written)))
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
        // **A prototype is a scope of its own** (C17 6.2.1 p4), in which each
        // parameter's name hides a typedef name from the end of its
        // declarator: `int g(int T, T y)` is not C where `T` is a typedef
        // name, and was read as one (#1205 review).
        self.names.push(Scope::default());
        let read = self.parameter_items(diagnostics);
        self.names.pop();
        read
    }

    /// The parameters of a list, in the scope [`Parser::parameter_list`]
    /// opened.
    fn parameter_items(&mut self, diagnostics: &mut DiagnosticSink) -> Option<Parameters> {
        let mut parameters: Vec<Declaration> = Vec::new();

        loop {
            // C17 6.7.6.3's `parameter-type-list` puts `, ...` after at least
            // one parameter, so a `...` first is not C and is left to
            // `specifiers` to call a missing declaration.
            // It has to close the list, and 6.7.6.3 p10 makes a lone `void`
            // the whole of one, so `(void, ...)` is not C either.
            let only_void = parameters.len() == 1
                && parameters[0].name.is_none()
                && matches!(self.ast.ty(parameters[0].ty), Type::Void);
            if !parameters.is_empty()
                && !only_void
                && self.check(TokenKind::Punct(Punct::Ellipsis))
                && self.peek_at(1).kind == TokenKind::Punct(Punct::RightParen)
            {
                self.report_unread(
                    Unread {
                        what: "a variadic function's `...`",
                        clause: "6.7.6.3",
                    },
                    diagnostics,
                );
                return None;
            }
            let start = self.peek().span;
            let base = self.specifiers(Position::Parameter, diagnostics)?;
            let (name, derivations) = self.declarator(false, diagnostics)?;
            if let Some(name) = name {
                self.declare_name(name, Named::Ordinary, diagnostics)?;
            }
            // The parameter's own pointer is the last derivation, the one that
            // makes the parameter's type. `apply` refuses one written anywhere
            // else, so this is the only one there is to keep.
            let nullability = match derivations.last() {
                Some(Derivation::Pointer {
                    nullability,
                    constant: _,
                }) => *nullability,
                _ => None,
            };
            let bracketed_const = matches!(
                derivations.last(),
                Some(Derivation::Array { constant: true, .. })
            );
            let (written, _) =
                self.apply(start, base, derivations, Declares::Parameter, diagnostics)?;
            // C17 6.7.6.3 p7 and p8: an array of `T` is a pointer to `T`, and
            // a function a pointer to it. `Declaration::ty` says why here.
            let ty = match self.ast.ty(written) {
                Type::Array { element, .. } => {
                    let element = *element;
                    let pointer = self.ast.push_type(Type::Pointer(element));
                    // 6.7.6.3 p7: the qualifiers in the brackets are the
                    // pointer's. Mutation: drop them; `a = 0` in
                    // `a_qualifier_in_an_array_parameter_s_brackets_is_the_pointer_s`
                    // is accepted.
                    if bracketed_const {
                        self.ast.push_const(pointer)
                    } else {
                        pointer
                    }
                }
                Type::Function { .. } => self.ast.push_type(Type::Pointer(written)),
                Type::Integer {
                    rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                    sign: _,
                }
                | Type::Void
                | Type::Pointer(_)
                | Type::Struct { .. } => written,
            };

            let span = self.join(start, self.previous().span, diagnostics);
            parameters.push(Declaration {
                name,
                ty,
                written,
                span,
                nullability,
                width: None,
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
            // C17 J.2 leaves a qualified `void` there undefined, and clang
            // refuses it, so it is refused rather than read as `(void)`.
            // Found by review.
            if self.ast.is_const(parameters[0].ty) {
                self.report_at(
                    parameters[0].span,
                    EXPECTED,
                    "the `void` of an empty parameter list cannot be `const`",
                    "write `void` alone here",
                    diagnostics,
                );
                return None;
            }
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
            if let Derivation::Pointer { constant: true, .. } = derivation {
                let pointer = self.ast.push_type(Type::Pointer(ty));
                ty = self.ast.push_const(pointer);
                continue;
            }
            ty = self.ast.push_type(match derivation {
                Derivation::Pointer {
                    nullability: _,
                    constant: _,
                } => Type::Pointer(ty),
                Derivation::Array {
                    length,
                    written,
                    constant: _,
                } => Type::Array {
                    element: ty,
                    length,
                    written,
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
                    constant: _,
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
                    Declares::FileScope | Declares::BlockScope | Declares::Member => Some((
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
                Derivation::Pointer {
                    nullability: None,
                    constant: _,
                }
                | Derivation::Array { .. }
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
        self.names.push(Scope::default());
        let read = self.block_items(start, diagnostics);
        self.names.pop();
        read
    }

    /// A function definition's body: a compound statement in the scope its
    /// caller opened for the parameters, rather than one of its own.
    fn function_body(&mut self, diagnostics: &mut DiagnosticSink) -> Option<StmtId> {
        let start = self.peek().span;
        self.expect(TokenKind::Punct(Punct::LeftBrace), "`{`", diagnostics)?;
        self.block_items(start, diagnostics)
    }

    /// The items of a block whose `{` [`Parser::compound`] has read, in the
    /// scope it opened.
    fn block_items(&mut self, start: Span, diagnostics: &mut DiagnosticSink) -> Option<StmtId> {
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
                    // Which of the two is here is decided by
                    // `begins_specifiers`, the test the declaration's own
                    // specifiers begin with.
                    // `__attribute__` goes with the declarations, where
                    // `specifiers` refuses it by name, rather than to the
                    // expressions, which would call it a missing one.
                    // **A typedef name followed by `:` is a label**, since
                    // labels have a name space of their own (C17 6.2.3 p1).
                    let labels = parser.peek().kind == TokenKind::Identifier
                        && parser.peek_at(1).kind == TokenKind::Punct(Punct::Colon);
                    let item = if !labels && parser.begins_specifiers_at(parser.peek())
                        || parser.check(TokenKind::Annotation(Annotation::Attribute))
                    {
                        parser.declaration_statement(diagnostics)
                    } else if parser.refuse_unread(Position::BlockScope, diagnostics) {
                        // A declaration this parser does not read, asked here
                        // rather than in `statement`, which also reads
                        // substatements: C17 6.8.4 and 6.8.5 allow no
                        // declaration as one, so there it is not C.
                        let span = parser.peek().span;
                        parser.ast.push_stmt(Stmt::Error { span })
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

        let span = self.join(start, self.previous().span, diagnostics);
        Some(self.ast.push_stmt(Stmt::Compound { body, span }))
    }

    /// A declaration where C17 6.8.2 allows one instead of a statement.
    fn declaration_statement(&mut self, diagnostics: &mut DiagnosticSink) -> StmtId {
        let enumerators = self.ast.enumerator_count();
        let declaring = std::mem::replace(&mut self.declaring, true);
        let statement = self.declaration_read(enumerators, diagnostics);
        self.declaring = declaring;
        statement
    }

    /// [`Parser::declaration_statement`], as [`Parser::item_read`] is
    /// [`Parser::item`].
    fn declaration_read(&mut self, enumerators: usize, diagnostics: &mut DiagnosticSink) -> StmtId {
        let start = self.peek().span;

        let declared = match self.declared(Declares::BlockScope, diagnostics) {
            Some(Specified::Named(declared)) => declared,
            // Asked for only in a member list, which this is not.
            Some(Specified::Unnamed(_)) => {
                unreachable!("an unnamed bit-field outside a member list")
            }
            Some(Specified::Tag(specified)) => {
                self.advance();
                let span = self.join(start, self.previous().span, diagnostics);
                let enumerators = self.ast.enumerators_since(enumerators);
                return self.ast.push_stmt(Stmt::Declaration {
                    declarators: Vec::new(),
                    typedefs: Vec::new(),
                    specified,
                    enumerators,
                    storage: None,
                    span,
                });
            }
            None => return self.ast.push_stmt(Stmt::Error { span: start }),
        };
        let specified = declared.base;
        let storage = declared.storage;

        let typedef = declared.typedef;
        let Some(declarators) = self.init_declarator_list(declared, diagnostics) else {
            return self.ast.push_stmt(Stmt::Error { span: start });
        };

        if self
            .expect(TokenKind::Punct(Punct::Semicolon), "`;`", diagnostics)
            .is_none()
        {
            return self.ast.push_stmt(Stmt::Error { span: start });
        }

        let span = self.join(start, self.previous().span, diagnostics);
        let enumerators = self.ast.enumerators_since(enumerators);
        self.ast.push_stmt(Stmt::Declaration {
            // As at file scope: a typedef declares no object.
            typedefs: if typedef {
                declarators.iter().map(|d| d.declaration.ty).collect()
            } else {
                Vec::new()
            },
            declarators: if typedef { Vec::new() } else { declarators },
            specified,
            enumerators,
            storage,
            span,
        })
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

            let span = self.join(start, self.previous().span, diagnostics);
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

        if self.check(TokenKind::Keyword(Keyword::Do)) {
            return self.do_statement(start, diagnostics);
        }

        if self.check(TokenKind::Keyword(Keyword::Switch)) {
            return self.switch_statement(start, diagnostics);
        }

        if self.check(TokenKind::Keyword(Keyword::Case))
            || self.check(TokenKind::Keyword(Keyword::Default))
        {
            return self.case_statement(start, diagnostics);
        }

        if self.check(TokenKind::Keyword(Keyword::Break))
            || self.check(TokenKind::Keyword(Keyword::Continue))
        {
            return self.jump_statement(start, diagnostics);
        }

        // C17 6.8.6.1's `goto identifier ;`. A `goto` followed by anything
        // else is not C, and is left to read as the expression it cannot be.
        if self.check(TokenKind::Keyword(Keyword::Goto))
            && self.peek_at(1).kind == TokenKind::Identifier
        {
            return self.goto_statement(start, diagnostics);
        }

        // C17 6.8.1's `identifier : statement`. A label with nothing after it
        // in its block, or with a declaration after it, is C23 and not C17,
        // and is left to read as an expression, which reports it.
        if self.check(TokenKind::Identifier)
            && self.peek_at(1).kind == TokenKind::Punct(Punct::Colon)
            && !self.ends_a_label_c17_forbids()
        {
            return self.labeled_statement(start, diagnostics);
        }

        // A statement C allows here that this parser does not read: a keyword
        // that begins one, or a label.
        if self.refuse_unread(Position::Statement, diagnostics) {
            return self.ast.push_stmt(Stmt::Error { span: start });
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

        let span = self.join(start, self.previous().span, diagnostics);
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

                let span = parser.join(start, parser.previous().span, diagnostics);
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
                parser.loops += 1;
                let body = parser.statement(diagnostics);
                parser.loops -= 1;
                let span = parser.join(start, parser.previous().span, diagnostics);
                parser.ast.push_stmt(Stmt::While {
                    condition,
                    body,
                    span,
                })
            },
            |parser, span| parser.ast.push_stmt(Stmt::Error { span }),
        )
    }

    /// `break ;` or `continue ;`, C17 6.8.6 p1, where 6.8.6.2 p1 and 6.8.6.3
    /// p1 allow it: `continue` in a loop body, `break` in a loop or `switch`
    /// body, however deeply nested.
    ///
    /// Anywhere else it is a constraint violation, reported at the keyword
    /// under [`EXPECTED`], which is the code `break` outside a loop had while
    /// this parser read neither, naming what C allows it in.
    fn jump_statement(&mut self, start: Span, diagnostics: &mut DiagnosticSink) -> StmtId {
        let breaks = self.check(TokenKind::Keyword(Keyword::Break));
        let misplaced = if breaks {
            (self.loops + self.switches == 0).then_some((
                "`break` is not in a loop or a `switch`",
                "C17 6.8.6.3 p1 allows `break` only in a loop or a `switch`",
            ))
        } else {
            (self.loops == 0).then_some((
                "`continue` is not in a loop",
                "C17 6.8.6.2 p1 allows `continue` only in a loop",
            ))
        };
        if let Some((message, label)) = misplaced {
            self.report(message, label, diagnostics);
            return self.ast.push_stmt(Stmt::Error { span: start });
        }
        self.advance();

        if self
            .expect(TokenKind::Punct(Punct::Semicolon), "`;`", diagnostics)
            .is_none()
        {
            return self.ast.push_stmt(Stmt::Error { span: start });
        }

        let span = self.join(start, self.previous().span, diagnostics);
        self.ast.push_stmt(if breaks {
            Stmt::Break { span }
        } else {
            Stmt::Continue { span }
        })
    }

    /// `switch ( expression ) statement`. C17 6.8.4 p1.
    ///
    /// The body counts in `switches`, which is what lets a `case` or a
    /// `break` stand in it, however deep.
    fn switch_statement(&mut self, start: Span, diagnostics: &mut DiagnosticSink) -> StmtId {
        self.advance();

        let Some(condition) = self.controlling(diagnostics) else {
            return self.ast.push_stmt(Stmt::Error { span: start });
        };

        self.deeper(
            diagnostics,
            |parser, diagnostics| {
                parser.switches += 1;
                let body = parser.statement(diagnostics);
                parser.switches -= 1;
                let span = parser.join(start, parser.previous().span, diagnostics);
                parser.ast.push_stmt(Stmt::Switch {
                    condition,
                    body,
                    span,
                })
            },
            |parser, span| parser.ast.push_stmt(Stmt::Error { span }),
        )
    }

    /// `case constant-expression : statement` or `default : statement`,
    /// C17 6.8.1 p1, which p2 allows only in a `switch` body. Outside one it
    /// is reported at the keyword under [`EXPECTED`], naming where C allows
    /// it.
    fn case_statement(&mut self, start: Span, diagnostics: &mut DiagnosticSink) -> StmtId {
        let case = self.check(TokenKind::Keyword(Keyword::Case));
        if self.switches == 0 {
            let (message, label) = if case {
                (
                    "`case` is not in a `switch`",
                    "C17 6.8.1 p2 allows `case` only in a `switch`",
                )
            } else {
                (
                    "`default` is not in a `switch`",
                    "C17 6.8.1 p2 allows `default` only in a `switch`",
                )
            };
            self.report(message, label, diagnostics);
            return self.ast.push_stmt(Stmt::Error { span: start });
        }
        self.advance();

        let value = case.then(|| self.infix_from(CONDITIONAL, diagnostics));
        if self
            .expect(TokenKind::Punct(Punct::Colon), "`:`", diagnostics)
            .is_none()
        {
            return self.ast.push_stmt(Stmt::Error { span: start });
        }

        self.deeper(
            diagnostics,
            |parser, diagnostics| {
                let body = parser.statement(diagnostics);
                let span = parser.join(start, parser.previous().span, diagnostics);
                parser.ast.push_stmt(match value {
                    Some(value) => Stmt::Case { value, body, span },
                    None => Stmt::Default { body, span },
                })
            },
            |parser, span| parser.ast.push_stmt(Stmt::Error { span }),
        )
    }

    /// `goto identifier ;`, its keyword and name already known to be here.
    /// Which label it names is name resolution's (C17 6.2.1 p3).
    fn goto_statement(&mut self, start: Span, diagnostics: &mut DiagnosticSink) -> StmtId {
        self.advance();
        let label = self.advance().span;
        if self
            .expect(TokenKind::Punct(Punct::Semicolon), "`;`", diagnostics)
            .is_none()
        {
            return self.ast.push_stmt(Stmt::Error { span: start });
        }
        let span = self.join(start, self.previous().span, diagnostics);
        self.ast.push_stmt(Stmt::Goto { label, span })
    }

    /// `identifier : statement`, its name and `:` already known to be here.
    ///
    /// The statement nests inside the label, so it takes a level.
    fn labeled_statement(&mut self, start: Span, diagnostics: &mut DiagnosticSink) -> StmtId {
        let label = self.advance().span;
        self.advance();
        self.deeper(
            diagnostics,
            |parser, diagnostics| {
                let body = parser.statement(diagnostics);
                let span = parser.join(start, parser.previous().span, diagnostics);
                parser.ast.push_stmt(Stmt::Labeled { label, body, span })
            },
            |parser, span| parser.ast.push_stmt(Stmt::Error { span }),
        )
    }

    /// `do statement while ( expression ) ;`. C17 6.8.5 p1.
    ///
    /// The body is read inside `deeper` and counts in `loops`, as a `while`'s
    /// does; the condition after it is not a nested statement and takes no
    /// level.
    fn do_statement(&mut self, start: Span, diagnostics: &mut DiagnosticSink) -> StmtId {
        self.advance();

        let body = self.deeper(
            diagnostics,
            |parser, diagnostics| {
                parser.loops += 1;
                let body = parser.statement(diagnostics);
                parser.loops -= 1;
                Some(body)
            },
            |_, _| None,
        );
        let Some(body) = body else {
            return self.ast.push_stmt(Stmt::Error { span: start });
        };

        if self
            .expect(TokenKind::Keyword(Keyword::While), "`while`", diagnostics)
            .is_none()
        {
            return self.ast.push_stmt(Stmt::Error { span: start });
        }
        let Some(condition) = self.controlling(diagnostics) else {
            return self.ast.push_stmt(Stmt::Error { span: start });
        };
        if self
            .expect(TokenKind::Punct(Punct::Semicolon), "`;`", diagnostics)
            .is_none()
        {
            return self.ast.push_stmt(Stmt::Error { span: start });
        }

        let span = self.join(start, self.previous().span, diagnostics);
        self.ast.push_stmt(Stmt::Do {
            body,
            condition,
            span,
        })
    }

    /// `for ( expression_opt ; expression_opt ; expression_opt ) statement` and
    /// `for ( declaration expression_opt ; expression_opt ) statement`, C17
    /// 6.8.5 p1.
    ///
    /// The second form's declaration ends at its own `;`, so it is read as the
    /// declaration a block would hold, by [`Parser::declaration_statement`],
    /// and the first clause is that statement. See [`ForStart`] for why it is
    /// a statement.
    fn for_statement(&mut self, start: Span, diagnostics: &mut DiagnosticSink) -> StmtId {
        // A `for` is a block of its own (C17 6.8.5 p5), so a name its clause
        // declares ends with it.
        self.names.push(Scope::default());
        let read = self.for_parts(start, diagnostics);
        self.names.pop();
        read
    }

    /// A `for` statement, in the scope [`Parser::for_statement`] opened.
    fn for_parts(&mut self, start: Span, diagnostics: &mut DiagnosticSink) -> StmtId {
        self.advance();

        let failed = |parser: &mut Self| parser.ast.push_stmt(Stmt::Error { span: start });

        if self
            .expect(TokenKind::Punct(Punct::LeftParen), "`(`", diagnostics)
            .is_none()
        {
            return failed(self);
        }

        // The same test a block makes, so a `for` reads a declaration where a
        // block would and nowhere else.
        // A `for` clause declares objects with automatic storage only (C17
        // 6.8.5 p3), so a typedef there is not C.
        if self.check(TokenKind::Keyword(Keyword::Typedef)) {
            self.report(
                "a `for` clause cannot declare a typedef",
                "C17 6.8.5 p3 allows only objects here",
                diagnostics,
            );
            return failed(self);
        }
        let first = if self.begins_specifiers_at(self.peek())
            || self.check(TokenKind::Annotation(Annotation::Attribute))
        {
            let declaration = self.declaration_statement(diagnostics);
            // Not observable today, since a tree with a syntax error goes no
            // further, but it keeps `ForStart::Declaration` pointing only at a
            // declaration, which `types.rs` asserts.
            if matches!(self.ast.stmt(declaration), Stmt::Error { .. }) {
                return failed(self);
            }
            Some(ForStart::Declaration(declaration))
        } else if self.refuse_unread(Position::ForInit, diagnostics) {
            // Not `BlockScope`'s question: 6.8.5 p3 allows a `for` only a
            // declaration of an object with storage class `auto` or `register`.
            return failed(self);
        } else {
            let Some(initialiser) = self.clause(TokenKind::Punct(Punct::Semicolon), diagnostics)
            else {
                return failed(self);
            };
            initialiser.map(ForStart::Expression)
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
                parser.loops += 1;
                let body = parser.statement(diagnostics);
                parser.loops -= 1;
                let span = parser.join(start, parser.previous().span, diagnostics);
                parser.ast.push_stmt(Stmt::For {
                    start: first,
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
        // The token the left operand starts at, which every node built here
        // starts at too. From the tokens rather than the operand's span,
        // because a parenthesis makes no node: `(x + 1)` as an included
        // operand ends at its `)`, and a join from the node's `1` would cut
        // the inclusion. See ADR-0054.
        let first = self.peek().span;
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
                    let span = self.join(first, self.previous().span, diagnostics);
                    self.ast.push_expr(Expr::Binary { op, lhs, rhs, span })
                }
                InfixKind::Assign(op) => {
                    let value = self.infix_from(next, diagnostics);
                    let span = self.join(first, self.previous().span, diagnostics);
                    self.ast.push_expr(Expr::Assign {
                        op,
                        place: lhs,
                        value,
                        span,
                    })
                }
                InfixKind::Comma => {
                    let rhs = self.infix_from(next, diagnostics);
                    let span = self.join(first, self.previous().span, diagnostics);
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
                        let span = self.join(first, self.previous().span, diagnostics);
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

    /// A prefix operator and its operand, `sizeof`, a cast, or a postfix
    /// expression. C17 6.5.3 and 6.5.4.
    ///
    /// **A cast is read here, as a cast-expression**, so that it binds as
    /// tightly as a prefix operator: `(char)x + 1` adds to the cast and
    /// `-(int)x` negates it. 6.5.3 makes the operand of `++`, `--` and
    /// `sizeof` a unary-expression rather than a cast-expression, which this
    /// reads more widely: `++(int)x` parses and the type check refuses it,
    /// since a cast is no lvalue (6.5.4 footnote 104).
    fn unary(&mut self, diagnostics: &mut DiagnosticSink) -> ExprId {
        if self.check(TokenKind::Keyword(Keyword::Sizeof)) {
            return self.sizeof(diagnostics);
        }
        if self.check(TokenKind::Punct(Punct::LeftParen)) && self.type_name_follows() {
            return self.cast(diagnostics);
        }
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
                let span = parser.join(start, parser.previous().span, diagnostics);
                parser.ast.push_expr(Expr::Unary { op, operand, span })
            },
            |parser, span| parser.ast.push_expr(Expr::Error { span }),
        )
    }

    /// Whether the token after the one here begins a type name: what may
    /// begin a parameter's specifiers, less `register`, which is a storage
    /// class and no part of a type name (C17 6.7.7). Asked with a `(` here,
    /// which this tells apart as a cast's or `sizeof`'s from a parenthesised
    /// expression's.
    fn type_name_follows(&self) -> bool {
        let kind = self.peek_at(1).kind;
        self.begins_specifiers_at(self.peek_at(1))
            || matches!(kind, TokenKind::Keyword(keyword)
                if keyword != Keyword::Register
                    && unread_keyword(keyword, Position::Parameter).is_some())
    }

    /// A type name, C17 6.7.7: specifiers and qualifiers, then a declarator
    /// that names nothing. `None` once reported. A storage class in it is
    /// refused, as a member's is.
    fn type_name(&mut self, diagnostics: &mut DiagnosticSink) -> Option<TypeId> {
        let start = self.peek().span;
        let base = self.specifiers(Position::Member, diagnostics)?;
        let (name, derivations) = self.declarator(false, diagnostics)?;
        if let Some(name) = name {
            self.report_at(
                name,
                EXPECTED,
                "a type name declares nothing",
                "a name here",
                diagnostics,
            );
            return None;
        }
        let (ty, _) = self.apply(start, base, derivations, Declares::BlockScope, diagnostics)?;
        Some(ty)
    }

    /// The `)` closing a type name, and a refusal of the compound literal a
    /// `{` after it would begin (C17 6.5.2.5), which is not read yet.
    fn close_type_name(&mut self, diagnostics: &mut DiagnosticSink) -> Option<()> {
        self.expect(TokenKind::Punct(Punct::RightParen), "`)`", diagnostics)?;
        if self.check(TokenKind::Punct(Punct::LeftBrace)) {
            self.report_unread(
                Unread {
                    what: "a compound literal",
                    clause: "6.5.2.5",
                },
                diagnostics,
            );
            return None;
        }
        Some(())
    }

    /// `( type-name ) cast-expression`, with the `(` here. C17 6.5.4.
    fn cast(&mut self, diagnostics: &mut DiagnosticSink) -> ExprId {
        let start = self.advance().span;
        let Some(ty) = self.type_name(diagnostics) else {
            return self.ast.push_expr(Expr::Error { span: start });
        };
        if self.close_type_name(diagnostics).is_none() {
            return self.ast.push_expr(Expr::Error { span: start });
        }
        self.deeper(
            diagnostics,
            |parser, diagnostics| {
                let operand = parser.unary(diagnostics);
                let span = parser.join(start, parser.previous().span, diagnostics);
                parser.ast.push_expr(Expr::Cast { ty, operand, span })
            },
            |parser, span| parser.ast.push_expr(Expr::Error { span }),
        )
    }

    /// `sizeof ( type-name )` or `sizeof unary-expression`, with the
    /// `sizeof` here. C17 6.5.3.4. A `(` that begins no type name begins the
    /// operand, so `sizeof (x)` is the size of `x`.
    fn sizeof(&mut self, diagnostics: &mut DiagnosticSink) -> ExprId {
        let start = self.advance().span;
        if self.check(TokenKind::Punct(Punct::LeftParen)) && self.type_name_follows() {
            self.advance();
            let Some(ty) = self.type_name(diagnostics) else {
                return self.ast.push_expr(Expr::Error { span: start });
            };
            if self.close_type_name(diagnostics).is_none() {
                return self.ast.push_expr(Expr::Error { span: start });
            }
            let span = self.join(start, self.previous().span, diagnostics);
            return self.ast.push_expr(Expr::SizeofType { ty, span });
        }
        self.deeper(
            diagnostics,
            |parser, diagnostics| {
                let operand = parser.unary(diagnostics);
                let span = parser.join(start, parser.previous().span, diagnostics);
                parser.ast.push_expr(Expr::SizeofExpr { operand, span })
            },
            |parser, span| parser.ast.push_expr(Expr::Error { span }),
        )
    }

    /// A primary expression with whatever follows it. C17 6.5.2.
    fn postfix(&mut self, diagnostics: &mut DiagnosticSink) -> ExprId {
        // The token the primary starts at, for the reason `climb` gives.
        let first = self.peek().span;
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
                    self.call(base, first, diagnostics)
                }
                Punct::LeftBracket => {
                    self.spend();
                    self.subscript(base, first, diagnostics)
                }
                // C17 6.5.2.3 p1: `.` and `->` are followed by an identifier,
                // which names a member and is looked up by nothing yet.
                Punct::Dot | Punct::Arrow => {
                    self.spend();
                    self.advance();
                    let Some(member) =
                        self.expect(TokenKind::Identifier, "a member's name", diagnostics)
                    else {
                        let span = self.ast.expr(base).span();
                        return self.ast.push_expr(Expr::Error { span });
                    };
                    let span = self.join(first, member, diagnostics);
                    self.ast.push_expr(Expr::Member {
                        base,
                        member,
                        arrow: punct == Punct::Arrow,
                        span,
                    })
                }
                Punct::PlusPlus | Punct::MinusMinus => {
                    self.spend();
                    let op = if punct == Punct::PlusPlus {
                        UnOp::PostInc
                    } else {
                        UnOp::PostDec
                    };
                    let end = self.advance().span;
                    let span = self.join(first, end, diagnostics);
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
    fn call(&mut self, callee: ExprId, start: Span, diagnostics: &mut DiagnosticSink) -> ExprId {
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

        let span = self.join(start, self.previous().span, diagnostics);
        self.ast.push_expr(Expr::Call {
            callee,
            arguments,
            span,
        })
    }

    /// `base[index]`, from the `[` that is there to the `]`.
    fn subscript(&mut self, base: ExprId, start: Span, diagnostics: &mut DiagnosticSink) -> ExprId {
        self.advance();

        let index = self.expression(diagnostics);

        if self
            .expect(TokenKind::Punct(Punct::RightBracket), "`]`", diagnostics)
            .is_none()
        {
            return self.ast.push_expr(Expr::Error { span: start });
        }

        let span = self.join(start, self.previous().span, diagnostics);
        self.ast.push_expr(Expr::Subscript { base, index, span })
    }

    /// A character constant, C17 6.4.4.4, if what is between its quotes is one
    /// character this compiler reads.
    ///
    /// More than one is valid C with a value the implementation chooses (p10),
    /// and refused as not read yet, as is anything [`unescape`] says is.
    /// What it says is not C is a syntax error. The value is the type
    /// checker's.
    ///
    /// [`unescape`]: crate::literal::unescape
    fn character(&mut self, span: Span, diagnostics: &mut DiagnosticSink) -> ExprId {
        let text = self.sources.snippet(span);
        // The lexer scans an encoding prefix into the token (C17 6.4.4.4 p1).
        if !text.starts_with('\'') {
            self.report_unread(
                Unread {
                    what: "a prefixed character constant",
                    clause: "6.4.4.4",
                },
                diagnostics,
            );
            self.advance();
            return self.ast.push_expr(Expr::Error { span });
        }
        let body = text
            .strip_prefix('\'')
            .and_then(|text| text.strip_suffix('\''))
            .unwrap_or("");
        let refused = match crate::literal::unescape(body) {
            Ok(values) if values.len() == 1 => None,
            // The lexer has reported an empty one already.
            Ok(values) if values.is_empty() => Some(None),
            Ok(_) => Some(Some(Unread {
                what: "a character constant of more than one character",
                clause: "6.4.4.4",
            })),
            Err(crate::literal::Unreadable::Unread(what)) => Some(Some(Unread {
                what,
                clause: "6.4.4.4",
            })),
            Err(crate::literal::Unreadable::Malformed(what)) => {
                self.report(
                    what,
                    "this is not a character constant C allows",
                    diagnostics,
                );
                Some(None)
            }
        };
        match refused {
            None => {
                self.advance();
                self.ast.push_expr(Expr::Character { span })
            }
            Some(unread) => {
                if let Some(unread) = unread {
                    self.report_unread(unread, diagnostics);
                }
                self.advance();
                self.ast.push_expr(Expr::Error { span })
            }
        }
    }

    /// A string literal, C17 6.4.5, and every one written straight after it,
    /// which translation phase 6 joins into one (5.1.1.2 p1). Each is read
    /// as a character constant's characters are, by
    /// [`unescape`](crate::literal::unescape): what it says this compiler
    /// does not read is refused as not read yet, and what it says is not C
    /// is a syntax error. An encoding prefix, which the lexer scans into the
    /// token, is not read yet (6.4.5 p3).
    ///
    /// Every literal of the run is read, refused or not, so that the next
    /// token is what follows the run; the first refusal is the one reported.
    fn string(&mut self, first: Span, diagnostics: &mut DiagnosticSink) -> ExprId {
        let mut parts = Vec::new();
        let mut refused = false;
        // The terminating zero (C17 6.4.5 p6).
        let mut count: u64 = 1;
        while self.check(TokenKind::String) {
            let span = self.peek().span;
            if !refused {
                let text = self.sources.snippet(span);
                let body = text
                    .strip_prefix('"')
                    .and_then(|text| text.strip_suffix('"'));
                match body.map(crate::literal::unescape) {
                    Some(Ok(values)) => count += values.len() as u64,
                    None => {
                        refused = true;
                        self.report_unread(
                            Unread {
                                what: "a prefixed string literal",
                                clause: "6.4.5",
                            },
                            diagnostics,
                        );
                    }
                    Some(Err(crate::literal::Unreadable::Unread(what))) => {
                        refused = true;
                        self.report_unread(
                            Unread {
                                what,
                                clause: "6.4.5",
                            },
                            diagnostics,
                        );
                    }
                    Some(Err(crate::literal::Unreadable::Malformed(what))) => {
                        refused = true;
                        self.report(what, "this is not a string literal C allows", diagnostics);
                    }
                }
            }
            self.advance();
            parts.push(span);
        }
        if refused {
            return self.ast.push_expr(Expr::Error { span: first });
        }
        let span = self.join(first, self.previous().span, diagnostics);
        let length = self.ast.push_expr(Expr::Length { value: count, span });
        self.ast.push_expr(Expr::String {
            parts,
            span,
            length,
        })
    }

    /// Part of C17 6.5.1: an integer constant, a character constant, a
    /// string literal, an identifier, or a parenthesised expression.
    fn primary(&mut self, diagnostics: &mut DiagnosticSink) -> ExprId {
        let span = self.peek().span;

        if self.eat(TokenKind::Number) {
            return self.ast.push_expr(Expr::Number { span });
        }

        if self.check(TokenKind::Character) {
            return self.character(span, diagnostics);
        }

        if self.check(TokenKind::String) {
            return self.string(span, diagnostics);
        }

        if self.check(TokenKind::Identifier) {
            let named = self.named(self.peek());
            self.advance();
            if let Some(Named::Enumerator(enumerator)) = named {
                return self.ast.push_expr(Expr::Enumerator { enumerator, span });
            }
            return self.ast.push_expr(Expr::Identifier { span });
        }

        // A `(` that begins a type name is a cast's, read by `unary` before
        // this is reached.
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

        if self.refuse_unread(Position::Expression, diagnostics) {
            return self.ast.push_expr(Expr::Error { span });
        }

        let span = self.report(
            "expected an expression",
            "this cannot begin one",
            diagnostics,
        );
        self.ast.push_expr(Expr::Error { span })
    }

    /// A declarator's [`Declaration::span`]: from the specifiers at `start`
    /// through `end` where that joins, and from the declarator's own first
    /// token, `declarator`, where it would cut an included file or an
    /// expansion that the declarator holds whole. Where both cut, refused as
    /// the first, as before there was a fallback. `Declaration::span` says why
    /// the fallback and not the declarator's span everywhere.
    ///
    /// Mutation: drop the fallback. A declarator list from a header is
    /// refused as `SC0207`, and
    /// `a_declarator_list_from_a_header_is_read_as_written_out` fails.
    fn declaration_span(
        &mut self,
        start: Span,
        declarator: Span,
        end: Span,
        diagnostics: &mut DiagnosticSink,
    ) -> Span {
        match (self.try_join(start, end), self.try_join(declarator, end)) {
            (Some(span), _) | (None, Some(span)) => span,
            (None, None) => self.join(start, end, diagnostics),
        }
    }

    /// [`Parser::join`] where a cut is not an error, because the caller has a
    /// span to fall back to: `None` where it would cut, reporting nothing.
    fn try_join(&self, from: Span, to: Span) -> Option<Span> {
        self.sources.join(from, to).ok()
    }

    /// The span from `from` to `to`, or, where it would cut an instance,
    /// `from`, with the cut reported.
    ///
    /// Every span this parser builds is made here, from the first and last
    /// token of what it read, and through [`SourceMap::join`], which lifts
    /// both ends into the nearest file they share and answers the cut where
    /// one does not reach its instance's edge. So a node holding a whole
    /// inclusion is joined where its `#include` is, and one holding part of
    /// one is refused rather than joined. See ADR-0054. Answering `from`
    /// keeps the item reading to its end, failed, and recovery takes over.
    ///
    /// Mutation: lift with `lift_start` and `lift_end`, which panic on a cut.
    /// `a_construct_that_cuts_an_include_is_refused` panics.
    fn join(&mut self, from: Span, to: Span, diagnostics: &mut DiagnosticSink) -> Span {
        let cut = match self.sources.join(from, to) {
            Ok(span) => return span,
            Err(cut) => cut,
        };
        // Not "this construct cuts the file": the span here may be one this
        // compiler builds for itself, and a declarator's falls back to its own
        // tokens only through `declaration_span`.
        let (what, label, included) = cut_words(self.sources, cut);
        let mut diagnostic = Diagnostic::error(format!("{what} is not read where it is yet"))
            .with_code(UNREAD)
            .with_label(Label::primary(cut.span, label))
            .with_note("safec reads an included file or a macro's expansion only where everything it builds over it holds all of it or none of it");
        if let Some(at) = included {
            diagnostic = diagnostic.with_label(Label::secondary(at, "included here"));
        }
        let first = !self.failed;
        self.report_built(cut.span, diagnostic, diagnostics);
        // At the last token read, not the next: a cut is among the tokens
        // this node already consumed, so recovery may end the item at its
        // own `;`. Reported at the next token, a declaration whose cut is
        // seen only once its `;` is read lost the item after it to recovery,
        // which looked for a `;` past that one (#555).
        if first {
            self.reported = self.at.saturating_sub(1);
        }
        from
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

        // Where what is missing would have been, which is the token that is
        // there, unless that token is in another file: `int x` at the end of a
        // header put the caret on the `int main` after the `#include`, in a
        // file where nothing is missing. Found by review. The end of the token
        // before is in the file the item was being read in.
        let next = self.peek().span;
        let at = match self.at.checked_sub(1).map(|at| self.tokens[at].span) {
            Some(before) if before.file() != next.file() => Span::at(before.file(), before.end()),
            _ => next,
        };
        self.report_at(
            at,
            EXPECTED,
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

    /// Report valid C this parser does not read yet, if the token here begins
    /// some at `at`, and say whether it did.
    ///
    /// A keyword asks [`unread_keyword`].
    #[inline(never)]
    fn refuse_unread(&mut self, at: Position, diagnostics: &mut DiagnosticSink) -> bool {
        let next = self.peek_at(1).kind;
        let unread = match self.peek().kind {
            TokenKind::Keyword(Keyword::Atomic) if next == TokenKind::Punct(Punct::LeftParen) => {
                unread_keyword(Keyword::Atomic, at).map(|unread| Unread {
                    clause: "6.7.2.4",
                    ..unread
                })
            }
            TokenKind::Keyword(keyword) => unread_keyword(keyword, at),
            _ => None,
        };
        let Some(unread) = unread else {
            return false;
        };
        self.report_unread(unread, diagnostics);
        true
    }

    /// Whether the token after a label's `:` is one C17 does not allow there:
    /// the `}` that closes its block, or the start of a declaration.
    fn ends_a_label_c17_forbids(&self) -> bool {
        let after = self.peek_at(2);
        after.kind == TokenKind::Punct(Punct::RightBrace)
            || self.begins_specifiers_at(after)
            || matches!(after.kind, TokenKind::Keyword(keyword)
                if unread_keyword(keyword, Position::BlockScope).is_some())
    }

    /// The report [`UNREAD`] makes, at the token here, in one wording for
    /// every construct.
    fn report_unread(&mut self, unread: Unread, diagnostics: &mut DiagnosticSink) -> Span {
        self.report_noted(
            UNREAD,
            format!("{} is not read yet", unread.what),
            "valid C, which this compiler does not read yet",
            format!(
                "C17 {} makes it valid; this compiler does not read it yet",
                unread.clause
            ),
            diagnostics,
        )
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
            self.reported = self.at;
            diagnostics.report(diagnostic);
        }

        span
    }
}

#[cfg(test)]
mod tests;
