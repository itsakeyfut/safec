//! The tree the parser builds, and the only thing the phases after it walk.
//!
//! Nodes live in flat vectors and a child is an index rather than a pointer.
//! See ADR-0008 for why, and for what it rejected.
//!
//! What is here is part of the subset [`docs/frontend.md`] calls Stage 1: the
//! types C17 6.7.6 derives from a declarator over `int`, `char` and `void`, the
//! expressions of 6.5, and the statements of 6.8.3 through 6.8.5. What is not
//! here is structs. The siblings of the issues that added the rest fill those in,
//! and each of them adds variants here rather than changing the shape.
//!
//! **A statement tree is bounded and an expression tree is not.** Every place a
//! statement nests inside another is a recursion in the parser, which counts
//! them, so a statement tree is at most `parser::MAX_NESTING` deep. An
//! expression tree has no such bound: a left-associative chain and a run of
//! postfix operators are folded by loops, so `a + a + ...` is as deep as it is
//! long. Walking an expression tree therefore takes two things, and only one of
//! them can be shared: a stack of its own, which `driver/dumps.rs`'s
//! `dump_expr` is what one looks like, and the children of a node, which
//! [`Expr::extend_children`] answers here so that the next walker does not have
//! to work them out again.
//!
//! [`docs/frontend.md`]: https://github.com/itsakeyfut/safec/blob/main/docs/frontend.md

use safec_ir::ir::{Rank, Sign};
use safec_ir::source::{SourceMap, Span};

/// Where an expression is in [`Ast`].
///
/// `Hash` because a side table keyed by id is what ADR-0008 exists to make
/// possible, and `sema.rs` is the first caller: it records which declaration
/// each identifier expression refers to. The other three ids have no such
/// caller yet and do not derive it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExprId(u32);

impl ExprId {
    /// The index this handle refers to.
    ///
    /// For a side table with a slot per expression, which is the dense half of
    /// what ADR-0008 says an id is for. `FileId::index` in
    /// `crates/safec-ir/src/source.rs` is the same thing one layer up. Not for
    /// reaching into an [`Ast`]: use [`Ast::expr`].
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Where a statement is in [`Ast`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StmtId(u32);

/// Where a top-level item is in [`Ast`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemId(u32);

/// Where a type is in [`Ast`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TypeId(u32);

/// Where an enumeration constant is in [`Ast`], in the order the source
/// declares them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EnumeratorId(u32);

impl EnumeratorId {
    /// The index this handle refers to, for a side table with a slot per
    /// enumerator, as [`ExprId::index`] is.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Where an enumeration is in [`Ast`]: one `enum` specifier as written,
/// with its list or without.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EnumerationId(u32);

/// One `enum` specifier, C17 6.7.2.2: `enum E { A, B }` with its constants,
/// or `enum E` naming one declared before.
///
/// **Beside the type, as `const` is** (see [`Ast::push_const`]). The type an
/// enumeration is stays a [`Type::Integer`], so every question about an
/// integer answers for it, and [`Ast::enumeration_of`] says which
/// enumeration it is for the few that ask: whether two types are compatible
/// (6.2.7 p1), and which tag a specifier names, which the name resolver
/// answers as it does for a struct. Its sign is the type checker's, which
/// knows the constants' values: see [`Ast::set_enumeration_sign`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Enumeration {
    /// The tag written, if one was.
    pub tag: Option<Span>,
    /// The constants of a list, or `None` for `enum E` without one.
    pub constants: Option<Vec<EnumeratorId>>,
}

/// One enumeration constant, C17 6.7.2.2: its name, and how its value is
/// given.
///
/// **Its value is the type checker's**, since a written value is an integer
/// constant expression only the type checker evaluates. The parser records
/// how to reach it: from the value written, or from the one before plus one
/// (6.7.2.2 p3), with `0` for the first of a list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Enumerator {
    /// Where the name is written.
    pub name: Span,
    /// The value written after `=`, if one was.
    pub written: Option<ExprId>,
    /// The enumerator written before this one in the same list, if any.
    pub previous: Option<EnumeratorId>,
}

impl Type {
    /// `int`.
    pub const INT: Type = Type::Integer {
        rank: Rank::Int,
        sign: Sign::Signed,
    };
    /// Plain `char`.
    pub const CHAR: Type = Type::Integer {
        rank: Rank::Char,
        sign: Sign::Plain,
    };
}

/// A type, as C17 6.7.6 derives one from a declarator.
///
/// The standard describes the derivation inductively, and this is the result
/// of it rather than the declarator that produced it. 6.7.6 p6:
///
/// > If, in the declaration `"T D1"`, `D1` has the form `(D)` then `ident` has
/// > the type specified by the declaration `"T D"`. Thus, a declarator in
/// > parentheses is identical to the unparenthesized declarator, but the
/// > binding of complicated declarators may be altered by parentheses.
///
/// That sentence is the whole of what tells `int *f(int)` from
/// `int (*f)(int)`, and it is why the tree holds the type and not the
/// declarator: the two declarators differ only in where the parentheses are,
/// while the types they derive are different in shape.
///
/// What a type *means* is not here. Whether the length of an array has an
/// integer type (6.7.6.2 p1) and whether two declarations of one name agree
/// (6.7.6.3 p15) are semantic rules, and a `Type` is what was written. The
/// one adjustment this stage makes, a parameter's by 6.7.6.3 p7 and p8, is
/// made beside the written type rather than in place of it: see
/// [`Declaration::ty`].
///
/// **Not comparable, on purpose.** Whether two types are the same is 6.7.6.3
/// p15's question and it is not this one: a `Type` holds where its parameters
/// were written and reaches the rest of itself through arena indices, so
/// `int *p;` and `int *q;` would compare unequal while being the same type.
/// Deriving `PartialEq` would make the natural way to ask that question
/// compile and answer wrongly. Without it, asking is `error[E0369]`, and
/// whoever writes the comparison writes it knowing what it has to ignore.
/// [`Ast::compatible`] is that comparison, written.
#[derive(Clone, Debug)]
pub enum Type {
    /// An integer type, a rank and a sign, as the IR's is: `int` is
    /// `Integer { Int, Signed }` and `char` `Integer { Char, Plain }`. See
    /// ADR-0116.
    Integer {
        /// Which of C's standard integer types, by rank.
        rank: Rank,
        /// How it is signed: see [`Sign`].
        sign: Sign,
    },
    /// `void`.
    Void,
    /// C17 6.7.6.1 p1 derives "pointer to T" from `* D`.
    Pointer(TypeId),
    /// C17 6.7.6.2 p3 derives "array of T" from `D [ ... ]`.
    Array {
        /// What the array is of.
        element: TypeId,
        /// How many, or `None` for `[]`, which 6.7.6.2 p4 makes an incomplete
        /// type. Not evaluated. 6.7.6.2 p1 asks that it have an integer type,
        /// and that its value be greater than zero if it is a constant
        /// expression at all; both are constraints and are checked later.
        length: Option<ExprId>,
        /// The source between the brackets, `None` for `[]`, kept so that the
        /// type can be spelled the way it was written. The length's own span
        /// does not do: parentheses make no node (`Parser::primary` says why),
        /// so `[(1 + 2) * 3]` has a length whose span starts after the `(`.
        written: Option<Span>,
    },
    /// C17 6.7.6.3 p5 derives "function returning T" from `D ( ... )`.
    Function {
        /// What it returns.
        returns: TypeId,
        /// What it was written to take.
        parameters: Parameters,
        /// Whether a prototype ends in `, ...`, which C17 6.7.6.3 p9 makes
        /// a function taking further arguments of no declared type. Only a
        /// prototype can (`Parameters::Unspecified` never is), and one with
        /// at least one parameter (6.7.6.3's `parameter-type-list`).
        variadic: bool,
    },
    /// `__builtin_va_list`, the type a variadic function's arguments are read
    /// through (C17 7.16 p3). A type of its own rather than the array or
    /// pointer one target lays it out as, so that only `__builtin_va_start`
    /// and the builtins beside it take one, and nothing reads it as the
    /// pointer it is on some targets. Laid out by the target in the IR
    /// (#1214). See ADR-0126.
    VaList,
    /// A structure, C17 6.7.2.1: `struct S`, `struct S { ... }` or
    /// `struct { ... }`.
    ///
    /// **Typed, and not yet lowered.** Which tag one names, and so whether
    /// two occurrences are one type, is the resolver's (`Resolution::tag`);
    /// the type checker types an object of one and the member an access
    /// names (`Types::member`). The lowering refuses a struct wherever
    /// it is asked to lower one, under `SC0304`, since the IR holds none, so
    /// no stage after it has to answer for one: a file-scope object nothing
    /// uses is dropped as every one is. Each occurrence is its own `TypeId`,
    /// as every type is here.
    Struct {
        /// Whether it was written `union`, C17 6.7.2.1 p6's other structure:
        /// every member at the start, overlapping. Everything else about one
        /// is a struct's, its tag included, which shares one name space with
        /// a struct's (6.2.3 p1).
        union: bool,
        /// The tag, if one was written.
        tag: Option<Span>,
        /// The members, if a `{ ... }` was written: never empty, because C17
        /// 6.7.2.1 p1's grammar asks for at least one member declaration.
        members: Option<Vec<Declaration>>,
    },
}

/// What a function declarator wrote between its parentheses.
///
/// Two spellings that C17 6.7.6.3 gives two different meanings, in two
/// different paragraphs, so they are two values here rather than one.
///
/// p10, on `(void)`:
///
/// > The special case of an unnamed parameter of type void as the only item in
/// > the list specifies that the function has no parameters.
///
/// p14, on `()`:
///
/// > An empty list in a function declarator that is part of a definition of
/// > that function specifies that the function has no parameters. The empty
/// > list in a function declarator that is not part of a definition of that
/// > function specifies that no information about the number or types of the
/// > parameters is supplied.
///
/// So `()` means one thing in a definition and another in a declaration, and
/// which of the two applies is not something this stage knows. It records the
/// spelling and leaves the reading to the phase that can tell a definition
/// from a declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Parameters {
    /// A parameter-type-list. Empty is `(void)`, per p10.
    Prototype(Vec<Declaration>),
    /// An empty identifier list, `()`, per p14.
    Unspecified,
}

/// One declared name and the type C derives for it.
///
/// The same shape at file scope, inside a block, and as a parameter, because
/// C17 6.7.6's `parameter-declaration` is a declaration. The name is optional
/// for the same reason: 6.7.7's abstract-declarator has no identifier, and a
/// parameter may be written with one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declaration {
    /// The span of the name, or `None` for an abstract declarator.
    pub name: Option<Span>,
    /// The type C17 gives the name, which every reader of the declaration
    /// asks about.
    ///
    /// The same as [`Declaration::written`] except for a parameter, which
    /// 6.7.6.3 p7 adjusts from an array of `T` to a pointer to `T` and p8
    /// from a function to a pointer to it. Adjusting here rather than where
    /// each reader asks is what makes a reader that forgets impossible: the
    /// name's type in the body, a call's arguments, whether two declarations
    /// agree and the lowering's local all read this field.
    pub ty: TypeId,
    /// The type the declarator derived, as written: what `--emit ast` and a
    /// function type's spelling show. Anything that needs the declarator's own
    /// expressions reads it too, because an array's length is in this type and
    /// not in the adjusted pointer: `sema.rs` resolves a parameter's array
    /// length through it, and the lowering evaluates the outermost length on
    /// entry to a definition from it and refuses what the adjusted pointer
    /// cannot hold, an inner length that is not a constant among them.
    pub written: TypeId,
    /// The specifiers through the declarator, and through its initializer
    /// where it has one.
    ///
    /// The `;` is not in it. One declaration may carry several declarators
    /// with one `;` between them all, so the `;` belongs to
    /// [`Item::Declaration`] and [`Stmt::Declaration`] rather than to any one
    /// of these. A parameter has neither and stops at its declarator.
    ///
    /// Every declarator of one declaration therefore begins at the same byte,
    /// because C17 6.7 gives them one set of specifiers between them, where
    /// that span can be joined. What tells two of them apart is
    /// [`Declaration::name`].
    ///
    /// **Where it would cut an included file or an expansion**, a declarator
    /// from a header written after specifiers outside it, it is
    /// [`InitDeclarator::span`] instead, which holds the file whole wherever
    /// the declarator does. Either way it holds every token of this
    /// declarator, and it is [`InitDeclarator::span`] that holds no other's.
    /// The fallback rather than the declarator's own span everywhere, because
    /// every position printed from this span would otherwise move (#555).
    pub span: Span,
    /// The nullability specifier written on this declaration's own pointer, if
    /// one was: a parameter's, or, for a function declared at file scope, the
    /// pointer it returns.
    ///
    /// Only those two carry one. The parser refuses it everywhere else, with
    /// `SC0204`, before a declaration is built. It is not part of
    /// [`Declaration::ty`]: it selects no type and changes no code, and what it
    /// does mean is ADR-0037 for `_Nonnull` and ADR-0050 for `_Nullable`.
    pub nullability: Option<Nullability>,
    /// The width a bit-field is declared with, its constant expression
    /// after the `:` (C17 6.7.2.1 p4), and `None` for any other declaration.
    /// Only a member has one, and a member that has one may have no name.
    pub width: Option<ExprId>,
}

/// A nullability specifier written after a `*`, and where.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Nullability {
    /// Which of the two this compiler reads.
    pub specifier: Specifier,
    /// The word itself.
    pub at: Span,
}

/// Which nullability specifier was written.
///
/// Two, and not the four `clang` accepts in C: the parser refuses
/// `_Null_unspecified` and `_Nullable_result` as annotations it does not read
/// (ADR-0050).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Specifier {
    /// `_Nonnull`: the pointer is not null, which the body believes and every
    /// call is checked against (ADR-0037).
    Nonnull,
    /// `_Nullable`: the pointer may be null, which is what an unannotated one
    /// already is below level 5 (ADR-0050).
    Nullable,
}

impl Specifier {
    /// How it is written.
    pub fn spelling(self) -> &'static str {
        match self {
            Self::Nonnull => "_Nonnull",
            Self::Nullable => "_Nullable",
        }
    }
}

/// One declarator of a declaration, with the initializer that followed it.
///
/// C17 6.7's `init-declarator`. A declaration shares its specifiers across
/// every declarator and shares nothing else, which is why each one folds its
/// own derivations onto the base type rather than the declaration deriving
/// once: `int *p, a[10];` makes a pointer and an array, not two of either.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InitDeclarator {
    /// The name and the type this declarator derived.
    pub declaration: Declaration,
    /// C17 6.7's `init-declarator` itself: from the declarator's first token
    /// through its initializer, without the specifiers it shares.
    ///
    /// What a span over this declarator falls back to where the span it is
    /// built from first would cut an included file or an expansion: a
    /// declarator from a header, `a` of `int` then `#include` of `a, b`, cuts
    /// the header from its specifiers and holds it whole from its own first
    /// token. `b`, the header's last token, holds it whole from them. See [`Declaration::span`] and ADR-0054.
    ///
    /// `None` where this span itself would cut one, `int` then `#include` of
    /// `x, *a` then `= 0;`, whose declarator starts partway through the
    /// header and ends outside it. Nothing is made of it then, rather than a
    /// wider span that could hold another declarator's tokens.
    pub span: Option<Span>,
    /// C17 6.7.9's `initializer`, in its `assignment-expression` form only.
    ///
    /// A braced initializer is refused by the parser and never reaches here.
    pub init: Option<ExprId>,
}

/// An operator with an operand on each side.
///
/// Its own type rather than the lexer's [`Punct`], which holds `;` and `{` as
/// well: reusing that would make `Binary { op: Punct::Semicolon }` a value this
/// type permits, and every phase after the parser would owe it an unreachable
/// arm.
///
/// [`Punct`]: crate::token::Punct
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `%`
    Rem,
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `<<`
    Shl,
    /// `>>`
    Shr,
    /// `<`
    Lt,
    /// `>`
    Gt,
    /// `<=`
    Le,
    /// `>=`
    Ge,
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `&`
    BitAnd,
    /// `^`
    BitXor,
    /// `|`
    BitOr,
    /// `&&`
    LogAnd,
    /// `||`
    LogOr,
}

/// An operator with one operand, on one side or the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    /// `+x`
    Plus,
    /// `-x`
    Minus,
    /// `!x`
    Not,
    /// `~x`
    BitNot,
    /// `*x`
    Deref,
    /// `&x`
    AddrOf,
    /// `++x`
    PreInc,
    /// `--x`
    PreDec,
    /// `x++`
    PostInc,
    /// `x--`
    PostDec,
}

impl BinOp {
    /// How C spells this operator.
    ///
    /// The same shape as `TokenKind::name` and [`Expr::name`], and for the same
    /// reason: the artifact is an interface, so a spelling is decided in one
    /// place rather than at each site that prints one.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mul => "*",
            Self::Div => "/",
            Self::Rem => "%",
            Self::Add => "+",
            Self::Sub => "-",
            Self::Shl => "<<",
            Self::Shr => ">>",
            Self::Lt => "<",
            Self::Gt => ">",
            Self::Le => "<=",
            Self::Ge => ">=",
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::BitAnd => "&",
            Self::BitXor => "^",
            Self::BitOr => "|",
            Self::LogAnd => "&&",
            Self::LogOr => "||",
        }
    }
}

impl UnOp {
    /// How C spells this operator.
    ///
    /// `++` and `--` each answer for two variants, because C spells the prefix
    /// and the postfix form the same way. [`UnOp::fixity`] is what tells them
    /// apart.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plus => "+",
            Self::Minus => "-",
            Self::Not => "!",
            Self::BitNot => "~",
            Self::Deref => "*",
            Self::AddrOf => "&",
            Self::PreInc | Self::PostInc => "++",
            Self::PreDec | Self::PostDec => "--",
        }
    }

    /// The word that tells `++x` from `x++`, where there is one.
    ///
    /// `None` for every operator C writes on one side only, because a word that
    /// never varies distinguishes nothing and would sit on every unary line.
    /// `clang` writes the same two words for the same two operators.
    pub fn fixity(self) -> Option<&'static str> {
        match self {
            Self::PreInc | Self::PreDec => Some("prefix"),
            Self::PostInc | Self::PostDec => Some("postfix"),
            Self::Plus | Self::Minus | Self::Not | Self::BitNot | Self::Deref | Self::AddrOf => {
                None
            }
        }
    }
}

/// An expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    /// A numeric constant, still spelled the way the source spells it.
    ///
    /// The value is not worked out here. `Token`'s own comment says a number's
    /// value and type belong to a stage that knows the target's sizes, and this
    /// is not that stage.
    Number {
        /// Where the constant is written.
        span: Span,
    },
    /// A character constant, C17 6.4.4.4: one character between `'`s, its
    /// escapes read by `crate::literal`. Its value is the type checker's, as
    /// a number's is, since it turns on whether the target's `char` is signed.
    Character {
        /// The constant, quotes included.
        span: Span,
    },
    /// A string literal, C17 6.4.5: an array of `char` with static storage
    /// duration holding the characters and a terminating zero. Adjacent
    /// literals are one (5.1.1.2 phase 6), so this is a run of tokens, each
    /// read by `crate::literal`. Its type and bytes are the type checker's
    /// and the lowering's, as a character constant's value is.
    String {
        /// Each literal of the run, quotes included, in the order written.
        parts: Vec<Span>,
        /// The whole run.
        span: Span,
        /// How many `char`s the array holds, terminating zero included, as
        /// an [`Expr::Length`] the type's length can name.
        length: ExprId,
    },
    /// A length this compiler counted rather than one the source wrote: a
    /// string literal's, which C17 6.4.5 p6 makes the bytes of the run and a
    /// zero. A node of its own because an array type holds its length as an
    /// expression, so that a literal's type is `char[N]` to every rule that
    /// reads one. No walk reaches it: it is no operand of anything.
    Length {
        /// The count.
        value: u64,
        /// The literal it counts.
        span: Span,
    },
    /// An identifier standing for something. C17 6.5.1.
    ///
    /// `Identifier` and not `Name`, because `--emit tokens` already calls this
    /// span an identifier and the two artifacts are read side by side. It is
    /// also the word C17 6.5.1 and ADR-0006 both use for it.
    ///
    /// A span and not text, like [`Function::name`]: resolving one is the
    /// printer's job and comparing two is the semantic analysis's.
    Identifier {
        /// Where the name is written.
        span: Span,
    },
    /// An identifier the parser found to name an enumeration constant, C17
    /// 6.4.4.3: a constant of type `int` (6.7.2.2 p3), not an object.
    ///
    /// **A variant of its own and not an [`Expr::Identifier`]**, because
    /// every rule that reads an identifier takes it to designate an object or
    /// a function, and an enumeration constant is neither: it is no lvalue,
    /// has no address and is assigned nothing. As its own variant, every
    /// match that has to answer for it fails to compile until it does.
    Enumerator {
        /// Which constant.
        enumerator: EnumeratorId,
        /// Where its name is written.
        span: Span,
    },
    /// One operand with an operator on one side of it. C17 6.5.3 and 6.5.2.
    Unary {
        /// Which operator, and which side it was written on.
        op: UnOp,
        /// What it applies to.
        operand: ExprId,
        /// The operator and the operand together.
        span: Span,
    },
    /// Two operands with an operator between them. C17 6.5.5 to 6.5.14.
    Binary {
        /// Which operator.
        op: BinOp,
        /// The operand on the left.
        lhs: ExprId,
        /// The operand on the right.
        rhs: ExprId,
        /// Both operands and the operator.
        span: Span,
    },
    /// An assignment. C17 6.5.16.
    ///
    /// Not a [`BinOp`]. C gives assignment a production of its own, and an
    /// assignment writes to a place rather than making a value out of two.
    /// `docs/roadmap.md` gives the Safety IR "values, places, operations" to
    /// hold, so that is a distinction the lowering will have to make; how it
    /// makes one is Phase 2's to decide and is not decided here. Folding this
    /// into [`Expr::Binary`] is the reversal, and what it costs is the semantic
    /// analysis and the lowering each taking it apart again.
    Assign {
        /// The operation folded in, or `None` for a plain `=`.
        op: Option<BinOp>,
        /// What is written to.
        place: ExprId,
        /// What is written.
        value: ExprId,
        /// The whole assignment.
        span: Span,
    },
    /// `condition ? then : otherwise`. C17 6.5.15.
    Conditional {
        /// What is asked.
        condition: ExprId,
        /// What it is when that holds.
        then: ExprId,
        /// What it is when it does not.
        otherwise: ExprId,
        /// The whole conditional.
        span: Span,
    },
    /// A call. C17 6.5.2.
    Call {
        /// What is called.
        callee: ExprId,
        /// What it is called with, in order.
        arguments: Vec<ExprId>,
        /// The callee through the closing parenthesis.
        span: Span,
    },
    /// `base[index]`. C17 6.5.2.
    Subscript {
        /// What is indexed.
        base: ExprId,
        /// What indexes it.
        index: ExprId,
        /// The base through the closing bracket.
        span: Span,
    },
    /// `s.m` or `p->m`. C17 6.5.2.3.
    ///
    /// The member is a span, which the type checker resolves to a member of
    /// the struct the base's tag names (`Types::member`), since only it knows
    /// the base's type.
    Member {
        /// The struct, or for `->` the pointer to it.
        base: ExprId,
        /// The member's name.
        member: Span,
        /// Whether it was written `->` rather than `.`.
        arrow: bool,
        /// The base through the member's name.
        span: Span,
    },
    /// `sizeof ( type-name )`. C17 6.5.3.4.
    SizeofType {
        /// The type whose size it is.
        ty: TypeId,
        /// The `sizeof` through the closing parenthesis.
        span: Span,
    },
    /// `sizeof unary-expression`. C17 6.5.3.4.
    ///
    /// The operand is typed and never evaluated (p2): nothing lowers it.
    SizeofExpr {
        /// The expression whose type's size it is.
        operand: ExprId,
        /// The `sizeof` and its operand.
        span: Span,
    },
    /// `( type-name ) cast-expression`. C17 6.5.4.
    Cast {
        /// The type converted to.
        ty: TypeId,
        /// What is converted.
        operand: ExprId,
        /// The opening parenthesis through the operand.
        span: Span,
    },
    /// The comma operator. C17 6.5.17.
    ///
    /// Not what separates the arguments of a call: an argument is an
    /// assignment-expression, so C's own grammar keeps the two apart and the
    /// tree does not have to.
    Comma {
        /// What is evaluated and discarded.
        lhs: ExprId,
        /// What the whole expression is.
        rhs: ExprId,
        /// Both sides and the comma.
        span: Span,
    },
    /// `__builtin_va_start(list, last)`, `__builtin_va_end(list)` or
    /// `__builtin_va_copy(dest, src)`: what `<stdarg.h>`'s `va_start`,
    /// `va_end` and `va_copy` are (C17 7.16.1; #1214). See ADR-0126.
    VaOp {
        /// Which of the three.
        op: VaOp,
        /// Its operands, as written: the list first.
        operands: Vec<ExprId>,
        /// The builtin's name through the closing parenthesis.
        span: Span,
    },
    /// `__builtin_va_arg(list, type)`, the next argument past a variadic
    /// function's parameters, read as `type`: what `<stdarg.h>`'s `va_arg`
    /// is (C17 7.16.1.1; #1214). See ADR-0126.
    VaArg {
        /// The list it is read out of.
        list: ExprId,
        /// The type it is read as.
        ty: TypeId,
        /// The builtin's name through the closing parenthesis.
        span: Span,
    },
    /// An expression the parser could not read.
    Error {
        /// What it gave up on.
        span: Span,
    },
}

/// Which of the builtins that start, end or copy a list of variadic
/// arguments an [`Expr::VaOp`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VaOp {
    /// `__builtin_va_start(list, last)`: `list` reads from the first argument
    /// past `last`, the function's last parameter (C17 7.16.1.4).
    Start,
    /// `__builtin_va_end(list)`: `list` is read no more (C17 7.16.1.3).
    End,
    /// `__builtin_va_copy(dest, src)`: `dest` reads on from where `src` has
    /// got to (C17 7.16.1.2).
    Copy,
}

impl VaOp {
    /// The builtin's name, as written.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Start => "__builtin_va_start",
            Self::End => "__builtin_va_end",
            Self::Copy => "__builtin_va_copy",
        }
    }

    /// How many operands it takes.
    pub fn arity(self) -> usize {
        match self {
            Self::End => 1,
            Self::Start | Self::Copy => 2,
        }
    }
}

/// The first clause of a `for`. C17 6.8.5 p1.
///
/// **A declaration is a statement of its own, which this points at**, always
/// a [`Stmt::Declaration`], rather than declarators held here. Every
/// constraint on a declaration is checked by a walk over
/// [`Ast::stmt_ids`], and a declaration that is a statement reaches all of
/// them; declarators held inline would need an arm in each, and a missing
/// one would accept a constraint violation only inside a `for`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForStart {
    /// `for (i = 0; ...)`, evaluated as a void expression.
    Expression(ExprId),
    /// `for (int i = 0; ...)`, whose names C17 6.8.5 p5 scopes to the loop.
    Declaration(StmtId),
}

/// A statement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stmt {
    /// A braced sequence of statements.
    Compound {
        /// The statements between the braces, in order.
        body: Vec<StmtId>,
        /// The braces and everything in them.
        span: Span,
    },
    /// `return`, with or without a value.
    Return {
        /// What is returned, if anything.
        value: Option<ExprId>,
        /// The keyword through the semicolon.
        span: Span,
    },
    /// A declaration where a statement could have been.
    ///
    /// C17 6.8.2 makes a block-item either a declaration or a statement, and
    /// the two share a list. `clang` does the same, wrapping one in a
    /// `DeclStmt`.
    Declaration {
        /// C17 6.7's `init-declarator-list`, in the order it was written.
        ///
        /// C17 6.7 marks the list optional, but 6.7 p2 then requires a
        /// declaration to declare a declarator, a tag, or an enumeration's
        /// members, so `int;` is a constraint violation rather than the empty
        /// case: `clang -std=c17 -pedantic-errors` reports it, and without
        /// `-pedantic-errors` it warns and accepts it as an extension. The
        /// parser refuses `int;` and `struct { int x; };` under `SC0206`.
        ///
        /// So the list is empty exactly where the declaration declares a tag,
        /// `struct S;` and `struct S { int x; };`, or an enumeration's
        /// members, `enum { A };`, or is a typedef, which declares no object
        /// and whose names the parser has already resolved; what a typedef
        /// declared is in [`Self::Declaration::typedefs`].
        declarators: Vec<InitDeclarator>,
        /// The type each typedef declarator declared, in the order written,
        /// where the specifiers wrote `typedef`, and empty otherwise.
        ///
        /// **Apart from [`Self::Declaration::declarators`]**, which a typedef
        /// leaves empty, so that no reader of objects can take a typedef for
        /// one: a reader that does not know this field refuses what it would
        /// need, rather than lowering a type as an object. What it is for is
        /// a variable length array typedef, whose length C17 6.7.8 p3 works
        /// out where the typedef is reached, and a file-scope one, which
        /// 6.7.6.2 p2 forbids (#1294).
        typedefs: Vec<TypeId>,
        /// What the specifiers named, before any declarator derived from it.
        /// The only type a declaration with no declarator has, and what the
        /// type checker asks of one.
        specified: TypeId,
        /// The enumeration constants this declaration declares, wherever in it
        /// they are written: in its specifiers, a member's, or a type name in
        /// an initializer. Here so that a pass that walks in scope order meets
        /// each written value where it is in scope.
        enumerators: Vec<EnumeratorId>,
        /// The storage-class specifier written, if one was.
        storage: Option<Storage>,
        /// The specifiers through the `;`.
        span: Span,
    },
    /// An expression evaluated for its effect. C17 6.8.3 p1: `expression_opt ;`.
    ///
    /// A null statement is this with nothing in it. C gives it no production of
    /// its own: 6.8.3 p3 describes it as "consisting of just a semicolon", and
    /// the `_opt` in the one production above is where it comes from. `clang`
    /// splits the two into a `NullStmt` and an expression; the tree here
    /// follows the grammar instead.
    Expression {
        /// What is evaluated, or `None` for a null statement.
        value: Option<ExprId>,
        /// The expression through the semicolon, or just the semicolon.
        span: Span,
    },
    /// `if`, with or without an `else`. C17 6.8.4 p1.
    ///
    /// A substatement that failed without consuming anything can carry a span
    /// outside this one: `if (1) }` ends here at the `)`, while the `Error` in
    /// `then` points at the brace that broke it. So a walker may not assume a
    /// child sits inside its parent, and a diagnostic underlining a whole
    /// statement has to say which of the two it means. The same holds of
    /// `While`, `Do` and `For`.
    If {
        /// What is asked. 6.8.4.1 p1 makes it a constraint that this has scalar
        /// type, which `types.rs` checks.
        condition: ExprId,
        /// What runs when it holds.
        then: StmtId,
        /// What runs when it does not, if anything was written.
        otherwise: Option<StmtId>,
        /// The keyword through the last substatement.
        span: Span,
    },
    /// `while`. C17 6.8.5 p1.
    While {
        /// What is asked before each turn.
        condition: ExprId,
        /// What runs while it holds.
        body: StmtId,
        /// The keyword through the body.
        span: Span,
    },
    /// `do`, whose body runs before its condition is first asked. C17 6.8.5.2.
    Do {
        /// What runs on every turn, the first included.
        body: StmtId,
        /// What is asked after each turn.
        condition: ExprId,
        /// The keyword through the `;` after the condition.
        span: Span,
    },
    /// `for`, in either of the forms C17 6.8.5 p1 gives: a first clause that
    /// is an expression, or one that is a declaration.
    For {
        /// What runs once before the first turn, if anything was written.
        start: Option<ForStart>,
        /// What is asked before each turn. Absent means it always holds.
        condition: Option<ExprId>,
        /// What runs after each turn.
        step: Option<ExprId>,
        /// What runs each turn.
        body: StmtId,
        /// The keyword through the body.
        span: Span,
    },
    /// `break`, which leaves the innermost loop. C17 6.8.6.3.
    Break {
        /// The keyword through the `;`.
        span: Span,
    },
    /// `continue`, which ends this turn of the innermost loop. C17 6.8.6.2.
    Continue {
        /// The keyword through the `;`.
        span: Span,
    },
    /// `goto`, which goes to a label in the same function. C17 6.8.6.1.
    Goto {
        /// The label's name, which name resolution matches with a
        /// [`Stmt::Labeled`] of the same function.
        label: Span,
        /// The keyword through the `;`.
        span: Span,
    },
    /// A statement with a label in front of it. C17 6.8.1.
    Labeled {
        /// The label's name.
        label: Span,
        /// The statement it labels.
        body: StmtId,
        /// The name through the end of the statement.
        span: Span,
    },
    /// `switch`, which goes to the label in its body whose value its
    /// controlling expression has. C17 6.8.4.2.
    Switch {
        /// What decides, an integer (6.8.4.2 p1).
        condition: ExprId,
        /// What holds the labels, usually a compound statement.
        body: StmtId,
        /// The keyword through the body.
        span: Span,
    },
    /// `case`, a label of the innermost `switch`. C17 6.8.1.
    Case {
        /// The value, an integer constant expression (6.8.4.2 p3).
        value: ExprId,
        /// The statement it labels.
        body: StmtId,
        /// The keyword through the end of the statement.
        span: Span,
    },
    /// `default`, the label of the innermost `switch` that no `case` matched.
    Default {
        /// The statement it labels.
        body: StmtId,
        /// The keyword through the end of the statement.
        span: Span,
    },
    /// A statement the parser could not read.
    Error {
        /// What it gave up on.
        span: Span,
    },
}

/// A storage-class specifier this compiler reads, C17 6.7.1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Storage {
    /// `static`: at file scope, internal linkage; in a block, static storage
    /// duration (6.2.2 p3, 6.2.4 p3).
    Static,
    /// `extern`: the object or function another declaration, perhaps in
    /// another unit, defines (6.2.2 p4).
    Extern,
}

/// A function definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function {
    /// The type the declarator derived, which C17 6.9.1 requires to be a
    /// function type. Requiring it is a constraint and so a later phase's; what
    /// is here is what was written.
    pub ty: TypeId,
    /// The span of the name, not the name.
    ///
    /// Resolving it to text is the printer's job, and comparing one name with
    /// another is the semantic analysis's. ADR-0006 names the second of those
    /// as the moment an interner is worth having, and it has not arrived.
    pub name: Span,
    /// The body, which is always a compound statement.
    pub body: StmtId,
    /// The whole definition, from the type to the closing brace.
    ///
    /// It begins at the specifiers, not at a hatch's attribute before them.
    pub span: Span,
    /// The `__attribute__` written before the definition, which is what makes
    /// it a hatch, or `None` if none was.
    ///
    /// Only a definition carries one: the parser refuses it on a declaration,
    /// because what a hatch says is about a body. See ADR-0038.
    pub attribute: Option<Attribute>,
    /// The nullability specifier on the pointer it returns, if one was
    /// written, which [`Declaration::nullability`] carries for a declaration.
    ///
    /// Not on [`Type::Function`]: types are compared for compatibility, and a
    /// specifier is not part of the type (ADR-0037).
    pub return_nullability: Option<Nullability>,
    /// The storage-class specifier written, if one was: `static` gives the
    /// function internal linkage (C17 6.2.2 p3).
    pub storage: Option<Storage>,
}

/// `__attribute__((name("argument")))`, written before a function definition.
///
/// The one shape of GNU C's attribute syntax the parser reads, and it reads the
/// shape only. Whether the name is `annotate` and the argument is
/// `"safec_unchecked"`, the one spelling this compiler gives a meaning, is
/// asked by `sema::resolve`, which refuses anything else: comparing the text
/// here would be the parser reaching for the source, which ADR-0006 keeps as
/// the signal for something else. `resolve` runs for every artifact that reads
/// this, so nothing downstream meets an attribute nobody compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attribute {
    /// From `__attribute__` through the last `)`.
    pub span: Span,
    /// The attribute's name.
    pub name: Span,
    /// The string literal it was given, quotes included.
    pub argument: Span,
}

/// Something at the top level of a translation unit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    /// A function definition: a declarator with a body after it.
    Function(Function),
    /// A declaration with no body, of an object or of a function.
    ///
    /// Which of the two it is follows from the type, and reading that is the
    /// semantic analysis's. What this says is only that no body was written.
    Declaration {
        /// C17 6.7's `init-declarator-list`, in the order it was written.
        ///
        /// Empty exactly where [`Stmt::Declaration::declarators`] says.
        declarators: Vec<InitDeclarator>,
        /// As [`Stmt::Declaration::typedefs`] says.
        typedefs: Vec<TypeId>,
        /// What the specifiers named, as [`Stmt::Declaration::specified`].
        specified: TypeId,
        /// As [`Stmt::Declaration::enumerators`].
        enumerators: Vec<EnumeratorId>,
        /// The storage-class specifier written, if one was.
        storage: Option<Storage>,
        /// The specifiers through the `;`.
        span: Span,
    },
    /// An item the parser could not read.
    Error {
        /// What it gave up on.
        span: Span,
    },
}

impl Expr {
    /// What this kind of node is called, for `--emit ast`.
    ///
    /// The same shape as `TokenKind::name`, and for the same reason: the
    /// artifact is an interface, so the spelling is decided in one place rather
    /// than written out wherever a node is printed.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Number { .. } => "Number",
            Self::Character { .. } => "Character",
            Self::String { .. } => "String",
            Self::Length { .. } => "Length",
            Self::Identifier { .. } => "Identifier",
            Self::Enumerator { .. } => "Enumerator",
            Self::Unary { .. } => "Unary",
            Self::Binary { .. } => "Binary",
            Self::Assign { .. } => "Assign",
            Self::Conditional { .. } => "Conditional",
            Self::Call { .. } => "Call",
            Self::Subscript { .. } => "Subscript",
            Self::Member { .. } => "Member",
            Self::SizeofType { .. } => "SizeofType",
            Self::SizeofExpr { .. } => "SizeofExpr",
            Self::Cast { .. } => "Cast",
            Self::Comma { .. } => "Comma",
            Self::VaOp { .. } => "VaOp",
            Self::VaArg { .. } => "VaArg",
            Self::Error { .. } => "Error",
        }
    }

    /// Where this node is.
    pub fn span(&self) -> Span {
        match self {
            Self::Number { span, .. }
            | Self::Character { span }
            | Self::String { span, .. }
            | Self::Length { span, .. }
            | Self::Identifier { span, .. }
            | Self::Enumerator { span, .. }
            | Self::Unary { span, .. }
            | Self::Binary { span, .. }
            | Self::Assign { span, .. }
            | Self::Conditional { span, .. }
            | Self::Call { span, .. }
            | Self::Subscript { span, .. }
            | Self::Member { span, .. }
            | Self::SizeofType { span, .. }
            | Self::SizeofExpr { span, .. }
            | Self::Cast { span, .. }
            | Self::Comma { span, .. }
            | Self::VaOp { span, .. }
            | Self::VaArg { span, .. }
            | Self::Error { span, .. } => *span,
        }
    }

    /// Append every expression this one is built from, in the order they were
    /// written.
    ///
    /// **The shape of the tree, in one place.** An expression tree has no bound
    /// on its depth, for the reason the module comment above gives, so anything
    /// that walks it needs its own stack and needs to know what the children
    /// are. The first is each walker's own problem. The second is this, on the
    /// tree rather than inside whichever walker was written first: `dump_expr`
    /// was that walker, it is private to `driver/dumps.rs`, and the driver is
    /// the top of the dependency graph, so nothing below it could read the
    /// answer even within this crate. `docs/roadmap.md` then moves the Safety
    /// IR into a crate of its own in Phase 2, which is not this crate and not
    /// today.
    ///
    /// **Appends rather than replaces, and the caller owns the buffer.** That
    /// is the whole of the contract and it is not visible in the signature,
    /// which is why the name says it. A walk can hand its own worklist straight
    /// in and needs nothing else; a walk that wants the children of one node
    /// alone clears first, as `dump_expr` does. Clearing here instead would
    /// silently cost the first of those every node but its last child, and
    /// `the_simplest_walk_over_the_tree_reaches_every_node` is what stops that
    /// being a quiet change.
    ///
    /// Not an iterator: `Call`'s children are one field and then a `Vec`, so
    /// the eight arms below have no one shape to return. Appending also lets a
    /// whole walk allocate once rather than once per node.
    ///
    /// The order is the order they were written, which is what lets a pre-order
    /// walk push them reversed and get them back in it.
    ///
    /// Forgetting one is not a compile error, and `error[E0004]` here says only
    /// that a new variant has to be looked at rather than that it was answered
    /// correctly. The
    /// answer is held by the corpus instead: `--emit ast` prints what this
    /// returns, so a child dropped from an arm is lines missing from expected
    /// files that are compared byte for byte.
    pub fn extend_children(&self, out: &mut Vec<ExprId>) {
        match self {
            Self::Number { .. }
            | Self::Character { .. }
            | Self::String { .. }
            | Self::Length { .. }
            | Self::Identifier { .. }
            | Self::Enumerator { .. }
            | Self::SizeofType { .. }
            | Self::Error { .. } => {}
            // `sizeof`'s operand is a child for the walks that resolve and
            // type it, and is never evaluated (C17 6.5.3.4 p2): what lowers
            // an expression does not descend into it.
            Self::Unary { operand, .. }
            | Self::Member { base: operand, .. }
            | Self::SizeofExpr { operand, .. }
            | Self::Cast { operand, .. }
            | Self::VaArg { list: operand, .. } => out.push(*operand),
            Self::VaOp { operands, .. } => out.extend(operands.iter().copied()),
            Self::Binary { lhs, rhs, .. } => out.extend([*lhs, *rhs]),
            Self::Assign { place, value, .. } => out.extend([*place, *value]),
            Self::Comma { lhs, rhs, .. } => out.extend([*lhs, *rhs]),
            Self::Subscript { base, index, .. } => out.extend([*base, *index]),
            Self::Conditional {
                condition,
                then,
                otherwise,
                ..
            } => out.extend([*condition, *then, *otherwise]),
            Self::Call {
                callee, arguments, ..
            } => {
                out.push(*callee);
                out.extend(arguments.iter().copied());
            }
        }
    }
}

impl Stmt {
    /// What this kind of node is called, for `--emit ast`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Compound { .. } => "Compound",
            Self::Return { .. } => "Return",
            Self::Declaration { .. } => "Declaration",
            Self::Expression { .. } => "Expression",
            Self::If { .. } => "If",
            Self::While { .. } => "While",
            Self::Do { .. } => "Do",
            Self::Break { .. } => "Break",
            Self::Continue { .. } => "Continue",
            Self::Goto { .. } => "Goto",
            Self::Labeled { .. } => "Labeled",
            Self::Switch { .. } => "Switch",
            Self::Case { .. } => "Case",
            Self::Default { .. } => "Default",
            Self::For { .. } => "For",
            Self::Error { .. } => "Error",
        }
    }

    /// Where this node is.
    pub fn span(&self) -> Span {
        match self {
            Self::Compound { span, .. }
            | Self::Return { span, .. }
            | Self::Expression { span, .. }
            | Self::If { span, .. }
            | Self::While { span, .. }
            | Self::Do { span, .. }
            | Self::Break { span }
            | Self::Continue { span }
            | Self::Goto { span, .. }
            | Self::Labeled { span, .. }
            | Self::Switch { span, .. }
            | Self::Case { span, .. }
            | Self::Default { span, .. }
            | Self::For { span, .. }
            | Self::Declaration { span, .. }
            | Self::Error { span } => *span,
        }
    }
}

impl Item {
    /// What this kind of node is called, for `--emit ast`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Function(_) => "Function",
            Self::Declaration { .. } => "Declaration",
            Self::Error { .. } => "Error",
        }
    }

    /// Where this node is.
    pub fn span(&self) -> Span {
        match self {
            Self::Function(function) => function.span,
            Self::Declaration { span, .. } | Self::Error { span } => *span,
        }
    }
}

/// One translation unit.
///
/// The vectors are private and every accessor takes `&self`, which is what
/// makes an id outlive a push: a caller cannot hold a `&Expr` across one,
/// because pushing needs `&mut self` and the borrow checker says so. That is
/// `error[E0502]` rather than a rule somebody has to remember, and it is the
/// half of ADR-0008 the compiler holds.
#[derive(Clone, Debug, Default)]
pub struct Ast {
    exprs: Vec<Expr>,
    stmts: Vec<Stmt>,
    items: Vec<Item>,
    types: Vec<Type>,
    /// Which types are `const`-qualified, by id. See [`Ast::push_const`].
    consts: std::collections::HashSet<TypeId>,
    /// The type each `const` copy was made from. See [`Ast::unqualified`].
    copied_from: std::collections::HashMap<TypeId, TypeId>,
    /// Every enumeration constant, in the order declared.
    enumerators: Vec<Enumerator>,
    /// Every `enum` specifier, as written.
    enumerations: Vec<Enumeration>,
    /// Which enumeration each enumerated type is, by the type's id, `const`
    /// copies included.
    enumeration_of: std::collections::HashMap<TypeId, EnumerationId>,
    /// Whether every enumerated type's sign is the one the type checker
    /// chose, rather than the `int` the parser gave it. See
    /// [`Ast::compatible`].
    enumerations_settled: bool,
}

impl Ast {
    /// `ty`, `const`-qualified (C17 6.7.3): a new id whose [`Type`] is a copy
    /// of `ty`'s, which [`Ast::is_const`] says is qualified. `ty` itself when
    /// it already is, since a qualifier written twice is written once (p5).
    ///
    /// **Beside the type and not a variant of it**, by the owner's decision on
    /// #1108. A `Type::Const` wrapper is a compile error where a type is
    /// matched whole, and an answer nobody gave wherever one is asked with
    /// `matches!` or `if let`: `int *const p` would be no pointer to each of
    /// them, in silence. As a copy, every question about a type answers as it
    /// did, and only the few that are about `const` ask.
    pub fn push_const(&mut self, ty: TypeId) -> TypeId {
        if self.is_const(ty) {
            return ty;
        }
        let copy = self.push_type(self.ty(ty).clone());
        self.consts.insert(copy);
        self.copied_from.insert(copy, ty);
        if let Some(&enumeration) = self.enumeration_of.get(&ty) {
            self.enumeration_of.insert(copy, enumeration);
        }
        copy
    }

    /// An enumerated type for `enumeration`: an `int` until the type checker
    /// gives it its sign.
    pub fn push_enumeration(&mut self, enumeration: Enumeration) -> TypeId {
        let id = EnumerationId(self.enumerations.len() as u32);
        self.enumerations.push(enumeration);
        let ty = self.push_type(Type::INT);
        self.enumeration_of.insert(ty, id);
        ty
    }

    /// Which enumeration `ty` is, where it is an enumerated type.
    pub fn enumeration_of(&self, ty: TypeId) -> Option<EnumerationId> {
        self.enumeration_of.get(&ty).copied()
    }

    /// The enumeration an id names.
    pub fn enumeration(&self, id: EnumerationId) -> &Enumeration {
        &self.enumerations[id.0 as usize]
    }

    /// Every enumerated type, by id, `const` copies included.
    pub fn enumerated_types(&self) -> Vec<TypeId> {
        let mut types: Vec<TypeId> = self.enumeration_of.keys().copied().collect();
        types.sort_by_key(|ty| ty.0);
        types
    }

    /// Give the enumerated type `ty` its sign, C17 6.7.2.2 p4: the integer
    /// type the implementation chose for it, which the type checker knows
    /// once the constants have values. The one place a type changes after it
    /// was pushed, and only from the `int` [`Ast::push_enumeration`] wrote.
    ///
    /// # Panics
    ///
    /// If `ty` is no enumerated type.
    pub fn set_enumeration_sign(&mut self, ty: TypeId, sign: Sign) {
        assert!(
            self.enumeration_of.contains_key(&ty),
            "only an enumerated type's sign is set late"
        );
        self.types[ty.0 as usize] = Type::Integer {
            rank: Rank::Int,
            sign,
        };
    }

    /// Say that every enumerated type has its sign. Before this,
    /// [`Ast::compatible`] compares an enumerated type with an integer type
    /// by rank alone, since the sign is not known yet.
    pub fn settle_enumerations(&mut self) {
        self.enumerations_settled = true;
    }

    /// The type `ty` was copied from to qualify it, followed to the first
    /// that is no copy, and `ty` itself where it is none. **What the name
    /// resolver bound**, where the copy was made after it ran: the type
    /// check qualifies a member of a `const` struct, and a struct copied so
    /// was bound to no tag, so asking its tag panicked (#1200 review).
    pub fn unqualified(&self, ty: TypeId) -> TypeId {
        let mut ty = ty;
        while let Some(&from) = self.copied_from.get(&ty) {
            ty = from;
        }
        ty
    }

    /// Whether `ty` is `const`-qualified. See [`Ast::push_const`].
    pub fn is_const(&self, ty: TypeId) -> bool {
        self.consts.contains(&ty)
    }

    /// An empty translation unit.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an expression and return where it went.
    pub fn push_expr(&mut self, expr: Expr) -> ExprId {
        let id = ExprId(self.exprs.len() as u32);
        self.exprs.push(expr);
        id
    }

    /// Add a statement and return where it went.
    pub fn push_stmt(&mut self, stmt: Stmt) -> StmtId {
        let id = StmtId(self.stmts.len() as u32);
        self.stmts.push(stmt);
        id
    }

    /// Add a top-level item and return where it went.
    pub fn push_item(&mut self, item: Item) -> ItemId {
        let id = ItemId(self.items.len() as u32);
        self.items.push(item);
        id
    }

    /// Add a type and return where it went.
    pub fn push_type(&mut self, ty: Type) -> TypeId {
        let id = TypeId(self.types.len() as u32);
        self.types.push(ty);
        id
    }

    /// Add an enumeration constant and return where it went.
    pub fn push_enumerator(&mut self, enumerator: Enumerator) -> EnumeratorId {
        let id = EnumeratorId(self.enumerators.len() as u32);
        self.enumerators.push(enumerator);
        id
    }

    /// The enumeration constant an id names.
    pub fn enumerator(&self, id: EnumeratorId) -> &Enumerator {
        &self.enumerators[id.0 as usize]
    }

    /// How many enumeration constants have been declared.
    pub fn enumerator_count(&self) -> usize {
        self.enumerators.len()
    }

    /// The ids of the enumeration constants declared after the first `count`.
    pub fn enumerators_since(&self, count: usize) -> Vec<EnumeratorId> {
        (count as u32..self.enumerators.len() as u32)
            .map(EnumeratorId)
            .collect()
    }

    /// Every enumeration constant's id, in the order declared.
    pub fn enumerator_ids(&self) -> impl Iterator<Item = EnumeratorId> {
        (0..self.enumerators.len() as u32).map(EnumeratorId)
    }

    /// The expression an id names.
    pub fn expr(&self, id: ExprId) -> &Expr {
        &self.exprs[id.0 as usize]
    }

    /// The statement an id names.
    pub fn stmt(&self, id: StmtId) -> &Stmt {
        &self.stmts[id.0 as usize]
    }

    /// The item an id names.
    pub fn item(&self, id: ItemId) -> &Item {
        &self.items[id.0 as usize]
    }

    /// The type an id names.
    pub fn ty(&self, id: TypeId) -> &Type {
        &self.types[id.0 as usize]
    }

    /// Every top-level item, in the order they were written.
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// Every expression id, in the order the nodes were pushed.
    ///
    /// The half of ADR-0008 that was left for its first caller: a pass that
    /// wants to say something about every expression needs to be able to name
    /// them all, and until now nothing did. Reaching a node from the roots
    /// finds the same ones, since nothing pushes a node it does not attach,
    /// but only if the walker knows every place a child can hang.
    pub fn expr_ids(&self) -> impl Iterator<Item = ExprId> + use<> {
        (0..self.exprs.len() as u32).map(ExprId)
    }

    /// Every type id, in the order the types were pushed, for the reason
    /// [`Ast::expr_ids`] gives: a test asks what a type written at a place
    /// was bound to, and the table is keyed by the id.
    pub fn type_ids(&self) -> impl Iterator<Item = TypeId> + use<> {
        (0..self.types.len() as u32).map(TypeId)
    }

    /// Every statement id, in the order the nodes were pushed, for the same
    /// reason as [`Ast::expr_ids`]: a pass about every declaration in a body
    /// names them through their statements.
    pub fn stmt_ids(&self) -> impl Iterator<Item = StmtId> + use<> {
        (0..self.stmts.len() as u32).map(StmtId)
    }

    /// Whether `left` and `right` are compatible types, C17 6.2.7 p1.
    ///
    /// Compatible and not identical, which is the relation C actually asks
    /// about: 6.5.16.1 p1 wants the pointed-to types of an assignment to be
    /// compatible, and 6.2.7 p1 makes identical types compatible and then adds
    /// cases that are compatible without being identical. `clang` keeps the
    /// two apart as `hasSameType` and `typesAreCompatible` for that reason.
    /// One of the extra cases is reachable in the subset this compiler reads,
    /// and it is in `compatible_parameters` below; another is a struct, which
    /// is compatible with a struct of the same tag, answered by
    /// `same_tag`: `Resolution::same_tag` is what a caller passes. It is
    /// named for the relation so that whoever adds a case is extending the
    /// right thing.
    ///
    /// The comparison [`Type`]'s own comment refuses to derive, written where
    /// the arena is, because a type reaches the rest of itself through ids and
    /// answering means walking them: `int *p;` and `int *q;` are two `Pointer`
    /// nodes holding two `Int` nodes, and they are the same type.
    ///
    /// **An array's length is not compared here.** 6.7.6.2 p6 makes two
    /// array types compatible only if both lengths are constant expressions
    /// and their values are equal, and this compares types alone, not the
    /// values `types.rs` gives a constant expression:
    /// `array_length_is_not_evaluated` in the corpus is a case that says so
    /// by name. So `int[2]` and `int[3]` answer "compatible" here, and
    /// `types.rs`'s `Checker::compatible_unqualified` is what compares the lengths
    /// wherever a program is type-checked, so `int (*p)[3]; int a[2]; p =
    /// &a;` is refused as `clang` refuses it (#621). What still answers
    /// without the lengths is `sema.rs`'s comparison of two declarations of
    /// one name, which runs before any value exists.
    ///
    /// Recursion, bounded the way `parser.rs::apply` bounds a declarator's
    /// derivations, so a type is at most `MAX_NESTING` deep. The nesting a
    /// parameter list adds is bounded by the parser's own recursion.
    pub fn compatible(
        &self,
        left: TypeId,
        right: TypeId,
        same_tag: &impl Fn(TypeId, TypeId) -> bool,
    ) -> bool {
        // C17 6.7.3 p11: "both shall have the identically qualified version
        // of a compatible type", at every level the comparison descends to.
        if self.is_const(left) != self.is_const(right) {
            return false;
        }
        // C17 6.7.2.2 p4 and 6.2.7 p1: two enumerated types are compatible
        // only when they are one enumeration, which is one tag as two
        // structs are, and one is compatible with its integer type. Before
        // that type's sign is chosen, the name resolver compares the two by
        // rank alone, and the type checker compares them again once it is
        // (`Checker::check_redeclared_lengths`).
        match (self.enumeration_of(left), self.enumeration_of(right)) {
            (Some(_), Some(_)) => return same_tag(left, right),
            (Some(_), None) | (None, Some(_)) if !self.enumerations_settled => {
                return matches!(
                    (self.ty(left), self.ty(right)),
                    (
                        Type::Integer {
                            rank: Rank::Int,
                            ..
                        },
                        Type::Integer {
                            rank: Rank::Int,
                            ..
                        }
                    )
                );
            }
            _ => {}
        }
        match (self.ty(left), self.ty(right)) {
            (
                Type::Integer {
                    rank: left_rank,
                    sign: left_sign,
                },
                Type::Integer {
                    rank: right_rank,
                    sign: right_sign,
                },
            ) => left_rank == right_rank && left_sign == right_sign,
            (Type::Void, Type::Void) | (Type::VaList, Type::VaList) => true,
            (Type::Pointer(left), Type::Pointer(right)) => self.compatible(*left, *right, same_tag),
            (Type::Array { element: left, .. }, Type::Array { element: right, .. }) => {
                self.compatible(*left, *right, same_tag)
            }
            (
                Type::Function {
                    returns: left,
                    parameters: left_parameters,
                    variadic: left_variadic,
                },
                Type::Function {
                    returns: right,
                    parameters: right_parameters,
                    variadic: right_variadic,
                },
            ) => {
                self.compatible(*left, *right, same_tag)
                    && self.compatible_parameters(
                        (left_parameters, *left_variadic),
                        (right_parameters, *right_variadic),
                        same_tag,
                    )
            }
            // Written out rather than `_ => false`, so that a variant added to
            // `Type` has to be answered for here: `error[E0004]` is what says
            // somebody looked, and a wildcard would call the new type
            // different from everything including itself.
            (
                Type::Integer {
                    rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                    sign: _,
                },
                _,
            )
            | (Type::Void, _)
            | (Type::VaList, _)
            | (Type::Pointer(_), _)
            | (Type::Array { .. }, _)
            | (Type::Function { .. }, _) => false,
            // Two struct types are one type exactly when they are one tag (C17
            // 6.2.7 p1, with 6.7.2.3 p4 and p5 saying when they are), which is
            // the resolver's to answer: `same_tag` asks it, as it does for two
            // enumerations, so that this module depends on nothing that knows
            // what a tag is.
            (Type::Struct { .. }, Type::Struct { .. }) => same_tag(left, right),
            (Type::Struct { .. }, _) => false,
        }
    }

    /// [`Ast::compatible`], with the qualifiers of the two types themselves
    /// left out: C17 6.7.6.3 p15 compares each parameter "taken as having the
    /// unqualified version of its declared type", so `void f(const int);`
    /// and `void f(int);` declare one function.
    pub fn compatible_unqualified(
        &self,
        left: TypeId,
        right: TypeId,
        same_tag: &impl Fn(TypeId, TypeId) -> bool,
    ) -> bool {
        if self.is_const(left) == self.is_const(right) {
            return self.compatible(left, right, same_tag);
        }
        // Each id is a copy of the other's type, so compare the copies'
        // children with the qualifier set aside: the one that is `const`
        // stands in for its unqualified twin.
        let (qualified, plain) = if self.is_const(left) {
            (left, right)
        } else {
            (right, left)
        };
        self.compatible_ignoring(qualified, plain, same_tag)
    }

    /// Whether `qualified`, a `const` type, is compatible with `plain` once
    /// its own qualifier is set aside.
    fn compatible_ignoring(
        &self,
        qualified: TypeId,
        plain: TypeId,
        same_tag: &impl Fn(TypeId, TypeId) -> bool,
    ) -> bool {
        match (self.ty(qualified), self.ty(plain)) {
            (Type::Pointer(left), Type::Pointer(right)) => self.compatible(*left, *right, same_tag),
            (Type::Array { element: left, .. }, Type::Array { element: right, .. }) => {
                self.compatible(*left, *right, same_tag)
            }
            (Type::Struct { .. }, Type::Struct { .. }) => same_tag(qualified, plain),
            // Two integer types are compatible only where they are one type
            // (C17 6.2.7 p1): `const long long *` takes no `int *`. Every
            // kind is named on the left, so one added later has to answer
            // here. Mutation: compare kinds alone; `a_const_pointer_takes_no_other_integer_type`
            // fails.
            (
                Type::Integer {
                    rank: left_rank,
                    sign: left_sign,
                },
                Type::Integer {
                    rank: right_rank,
                    sign: right_sign,
                },
            ) => left_rank == right_rank && left_sign == right_sign,
            (Type::Void, Type::Void) | (Type::VaList, Type::VaList) => true,
            // 6.7.3 p9 leaves a qualified function type undefined, so no
            // valid program asks this, and the answer is the one this gave
            // before its arms were written out.
            (Type::Function { .. }, Type::Function { .. }) => true,
            (
                Type::Pointer(_)
                | Type::Array { .. }
                | Type::Struct { .. }
                | Type::Integer { .. }
                | Type::Void
                | Type::VaList
                | Type::Function { .. },
                _,
            ) => false,
        }
    }

    /// Whether two parameter lists are compatible, C17 6.7.6.3 p15.
    ///
    /// Two prototypes agree parameter by parameter. A prototype and an empty
    /// identifier list are the case p15 spells out:
    ///
    /// > If one type has a parameter type list and the other type is specified
    /// > by a function declarator that is not part of a function definition
    /// > and that contains an empty identifier list, the parameter list shall
    /// > not have an ellipsis terminator and the type of each parameter shall
    /// > be compatible with the type that results from the application of the
    /// > default argument promotions.
    ///
    /// So `int (void)` and `int ()` are compatible, and so are `int (int)` and
    /// `int ()`, while `int (char)` and `int ()` are not. `clang` agrees on
    /// all three, measured. Answering `false` for every one of them, which is
    /// what reading p10 and p14 alone gives, rejected `int (*p)(void) = q`
    /// where `q` is `int (*)()`: a program `clang` compiles.
    ///
    /// **And on `...`**: two prototypes agree on it, and one against `()`
    /// has none, since p15 asks that "the parameter type list shall not
    /// have an ellipsis terminator" (#1213). Mutation: ignore it; a
    /// variadic declaration and one without the `...` are one function,
    /// and `two_declarations_disagree_on_the_ellipsis` in
    /// `a_variadic_prototype_is_held_to_its_parameters_and_its_declarations`
    /// builds.
    fn compatible_parameters(
        &self,
        (left, left_variadic): (&Parameters, bool),
        (right, right_variadic): (&Parameters, bool),
        same_tag: &impl Fn(TypeId, TypeId) -> bool,
    ) -> bool {
        match (left, right) {
            (Parameters::Prototype(left), Parameters::Prototype(right)) => {
                left_variadic == right_variadic
                    && left.len() == right.len()
                    && left.iter().zip(right).all(|(left, right)| {
                        self.compatible_unqualified(left.ty, right.ty, same_tag)
                    })
            }
            (Parameters::Unspecified, Parameters::Unspecified) => true,
            (Parameters::Prototype(parameters), Parameters::Unspecified) => {
                !left_variadic
                    && parameters
                        .iter()
                        .all(|parameter| !self.is_promoted_by_default(parameter.ty))
            }
            (Parameters::Unspecified, Parameters::Prototype(parameters)) => {
                !right_variadic
                    && parameters
                        .iter()
                        .all(|parameter| !self.is_promoted_by_default(parameter.ty))
            }
        }
    }

    /// Whether the default argument promotions change this type, C17 6.5.2.2
    /// p6.
    ///
    /// They promote a `char` or a `short` to an `int` and a `float` to a
    /// `double`. This compiler has no `float`, so the integers are the whole
    /// of it, and an arm for
    /// each of the others rather than a wildcard so that a type added later
    /// has to say which side it is on.
    pub(crate) fn is_promoted_by_default(&self, ty: TypeId) -> bool {
        match self.ty(ty) {
            // C17 6.3.1.1 p2: a rank below `int`'s is promoted.
            Type::Integer {
                rank: Rank::Char | Rank::Short,
                sign: _,
            } => true,
            Type::Integer {
                rank: Rank::Int | Rank::Long | Rank::LongLong,
                sign: _,
            } => false,
            Type::Void
            | Type::VaList
            | Type::Pointer(_)
            | Type::Array { .. }
            | Type::Function { .. }
            | Type::Struct { .. } => false,
        }
    }
}

/// A type, in the declarator notation C itself writes.
///
/// Here rather than in `driver.rs`, which is where it was written for
/// `--emit ast`, because a diagnostic has to spell a type too and two spellers
/// are two things to drift. The corpus pins what this prints, byte for byte,
/// so moving it is guarded by tests that already exist.
///
/// `int *`, `int[10]`, `int (*)(int)`: the same spelling `clang` prints, so the
/// two dumps can be diffed rather than read against one another. It diverges in
/// one place, and deliberately: `clang` shows a parameter after the adjustments
/// C17 6.7.6.3 p7 and p8 make, so it spells `int f(int [10])` as `int (int *)`,
/// where this spells it `int (int[10])`, because it is handed
/// [`Declaration::written`]: what the source says, which is what an artifact
/// of the source shows. A diagnostic about one argument spells the parameter's
/// [`Declaration::ty`] instead, so the two meet as `int *` beside
/// `int (int[10])`.
///
/// The walk down the spine is a loop and not a recursion, so a type of ten
/// thousand pointers costs no stack. Only a parameter list recurses, and how
/// deeply one can nest is bounded by the parser. `Parser::apply` bounds the
/// spine too, which is what every other walk of a type will rely on; the loop
/// here means this one does not have to.
pub fn spell_type(sources: &SourceMap, ast: &Ast, id: TypeId) -> String {
    let mut id = id;
    let mut inner = String::new();

    loop {
        // `const` before a base and after the `*` of a pointer, as clang
        // spells `const int *` and `int *const`.
        let constant = if ast.is_const(id) { "const " } else { "" };
        let base = match ast.ty(id) {
            Type::Integer { rank, sign } => {
                format!("{constant}{}", safec_ir::ir::integer_name(*rank, *sign))
            }
            Type::Void => format!("{constant}void"),
            Type::VaList => format!("{constant}__builtin_va_list"),
            // The tag as written, and `(anonymous)` for none. A base of more
            // than one word, so it is spelled here and returned with the same
            // spacing as the others below.
            Type::Struct { tag, union, .. } => {
                let tag = tag.map_or("(anonymous)", |tag| sources.snippet(tag));
                let keyword = if *union { "union" } else { "struct" };
                let spelled = format!("{constant}{keyword} {tag}");
                return if inner.is_empty() || inner.starts_with('[') {
                    format!("{spelled}{inner}")
                } else {
                    format!("{spelled} {inner}")
                };
            }
            Type::Pointer(pointee) => {
                let star = match (constant.is_empty(), inner.is_empty()) {
                    (true, _) => "*".to_owned(),
                    (false, true) => "*const".to_owned(),
                    (false, false) => "*const ".to_owned(),
                };
                inner = if binds_tighter_than_a_pointer(ast.ty(*pointee)) {
                    format!("({star}{inner})")
                } else {
                    format!("{star}{inner}")
                };
                id = *pointee;
                continue;
            }
            Type::Array {
                element,
                length,
                written,
            } => {
                // The length is the source's own bytes and is not evaluated, so
                // `int a[1 + 2]` spells `int[1 + 2]` where `clang`, which does
                // evaluate it, spells `int[3]`. What 6.7.6.2 p1 asks of it is a
                // constraint, and constraints are checked later. The bytes are
                // what was written between the brackets, not the length's span,
                // which would drop a leading `(` and keep its `)`.
                // A length nobody wrote, a string literal's, is spelled as
                // the count it is.
                let length = match (written, length.map(|length| ast.expr(length))) {
                    (Some(written), _) => sources.snippet(*written).to_owned(),
                    (None, Some(Expr::Length { value, .. })) => value.to_string(),
                    (None, _) => String::new(),
                };
                inner = format!("{inner}[{length}]");
                id = *element;
                continue;
            }
            Type::Function {
                returns,
                parameters,
                variadic,
            } => {
                let ellipsis = if *variadic { ", ..." } else { "" };
                inner = format!(
                    "{inner}({}{ellipsis})",
                    spell_parameters(sources, ast, parameters)
                );
                id = *returns;
                continue;
            }
        };

        // A space between the base and what follows, unless there is nothing
        // to follow or it begins with `[`. That is the spacing `clang` prints:
        // `int *`, `int (*)(int)`, and `int[10]` with no space at all.
        return if inner.is_empty() || inner.starts_with('[') {
            format!("{base}{inner}")
        } else {
            format!("{base} {inner}")
        };
    }
}

/// Whether a derivation binds its operand more tightly than a pointer does.
///
/// An array and a function do, so a pointer to either is written with the `*`
/// in parentheses to say that the pointer is the outer one. That rule alone is
/// what makes `int (*)(int)` and `int *(int)` two different strings, and C17
/// 6.7.6 p6 is where the binding it reflects is stated.
///
/// An exhaustive `match` and not a `matches!`, for the reason the emit gate
/// above gives: a `matches!` answers `false` for a variant nobody has thought
/// about, and the answer here is the one thing that tells two types apart in
/// the artifact. `[*]`, which `parser.rs` already lists as a shape it does not
/// read yet, is a variant this will have to answer for.
fn binds_tighter_than_a_pointer(ty: &Type) -> bool {
    match ty {
        Type::Array { .. } | Type::Function { .. } => true,
        Type::Integer {
            rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
            sign: _,
        }
        | Type::Void
        | Type::VaList
        | Type::Pointer(_)
        | Type::Struct { .. } => false,
    }
}

/// What goes between a function type's parentheses.
///
/// `(void)` for a prototype that declared no parameters and `()` for an empty
/// identifier list, because C17 6.7.6.3 gives the two spellings two meanings.
fn spell_parameters(sources: &SourceMap, ast: &Ast, parameters: &Parameters) -> String {
    let Parameters::Prototype(parameters) = parameters else {
        return String::new();
    };

    if parameters.is_empty() {
        return "void".to_owned();
    }

    parameters
        .iter()
        .map(|parameter| spell_type(sources, ast, parameter.written))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests;
