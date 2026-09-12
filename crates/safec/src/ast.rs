//! The tree the parser builds, and the only thing the phases after it walk.
//!
//! Nodes live in flat vectors and a child is an index rather than a pointer.
//! See ADR-0008 for why, and for what it rejected.
//!
//! What is here is part of the subset [`docs/frontend.md`] calls Stage 1: the
//! types C17 6.7.6 derives from a declarator over `int`, `char` and `void`, the
//! expressions of 6.5, and the statements of 6.8.3 through 6.8.5. What is not
//! here is `switch`, `do`, `goto`, a labelled statement, `break`, `continue`, a
//! declaration in a `for` initialiser, and structs. The siblings of the issues
//! that added the rest fill those in, and each of them adds variants here
//! rather than changing the shape.
//!
//! **A statement tree is bounded and an expression tree is not.** Every place a
//! statement nests inside another is a recursion in the parser, which counts
//! them, so a statement tree is at most `parser::MAX_NESTING` deep. An
//! expression tree has no such bound: a left-associative chain and a run of
//! postfix operators are folded by loops, so `a + a + ...` is as deep as it is
//! long. Walking an expression tree therefore takes two things, and only one
//! of them can be shared: a stack of its own, which `driver.rs`'s `dump_expr`
//! is what one looks like, and the children of a node, which
//! [`Expr::extend_children`] answers here so that the next walker does not have
//! to work them out again.
//!
//! [`docs/frontend.md`]: https://github.com/itsakeyfut/safec/blob/main/docs/frontend.md

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
    /// `crates/safec/src/source.rs` is the same thing one layer up. Not for
    /// reaching into an [`Ast`]: use [`Ast::expr`].
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Where a statement is in [`Ast`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StmtId(u32);

/// Where a top-level item is in [`Ast`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemId(u32);

/// Where a type is in [`Ast`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypeId(u32);

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
/// integer type (6.7.6.2 p1), whether a parameter declared as an array is
/// adjusted to a pointer (6.7.6.3 p7), and whether two declarations of one
/// name agree (6.7.6.3 p15) are all semantic rules, and this stage records
/// what was written.
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
    /// `int`.
    Int,
    /// `char`.
    Char,
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
    },
    /// C17 6.7.6.3 p5 derives "function returning T" from `D ( ... )`.
    Function {
        /// What it returns.
        returns: TypeId,
        /// What it was written to take.
        parameters: Parameters,
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
    /// The type the declarator derived.
    pub ty: TypeId,
    /// The specifiers through the declarator, and through its initializer
    /// where it has one.
    ///
    /// The `;` is not in it. One declaration may carry several declarators
    /// with one `;` between them all, so the `;` belongs to
    /// [`Item::Declaration`] and [`Stmt::Declaration`] rather than to any one
    /// of these. A parameter has neither and stops at its declarator.
    ///
    /// Every declarator of one declaration therefore begins at the same byte,
    /// because C17 6.7 gives them one set of specifiers between them. What
    /// tells two of them apart is [`Declaration::name`].
    pub span: Span,
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
    /// An expression the parser could not read.
    Error {
        /// What it gave up on.
        span: Span,
    },
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
        /// Never empty. C17 6.7 marks the list optional, but 6.7 p2 then
        /// requires a declaration to declare a declarator, a tag, or an
        /// enumeration's members, so `int;` is a constraint violation rather
        /// than the empty case: `clang -std=c17 -pedantic-errors` reports it
        /// and accepts it silently only as an extension. What is valid and
        /// empty here is `struct S { int x; };`, which declares a tag, and
        /// this parser reads no tags yet. Either way nothing downstream has to
        /// answer for an empty list, and #125 is the change that would alter
        /// that.
        declarators: Vec<InitDeclarator>,
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
    /// `While` and `For`.
    If {
        /// What is asked. 6.8.4.1 p1 makes it a constraint that this has scalar
        /// type, which is a constraint and so a later phase's.
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
    /// `for`, in the form whose three clauses are expressions. C17 6.8.5 p1.
    ///
    /// The other form, `for ( declaration expression_opt ; expression_opt )`,
    /// is not read. It used to be blocked on there being no initializer to
    /// read; there is one now, and what is left is this type and the scope:
    /// `initialiser` holds an `ExprId`, and C17 6.8.5 p5 gives a declaration
    /// written here a scope that is the loop rather than the block around it.
    /// These are three separate `Option`s rather than something that could
    /// also hold a declaration, because an interface with no caller is
    /// invented rather than designed and widening this one later is additive.
    For {
        /// What runs once before the first turn.
        initialiser: Option<ExprId>,
        /// What is asked before each turn. Absent means it always holds.
        condition: Option<ExprId>,
        /// What runs after each turn.
        step: Option<ExprId>,
        /// What runs each turn.
        body: StmtId,
        /// The keyword through the body.
        span: Span,
    },
    /// A statement the parser could not read.
    Error {
        /// What it gave up on.
        span: Span,
    },
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
    pub span: Span,
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
        /// Never empty, for the reason [`Stmt::Declaration`] gives.
        declarators: Vec<InitDeclarator>,
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
            Self::Identifier { .. } => "Identifier",
            Self::Unary { .. } => "Unary",
            Self::Binary { .. } => "Binary",
            Self::Assign { .. } => "Assign",
            Self::Conditional { .. } => "Conditional",
            Self::Call { .. } => "Call",
            Self::Subscript { .. } => "Subscript",
            Self::Comma { .. } => "Comma",
            Self::Error { .. } => "Error",
        }
    }

    /// Where this node is.
    pub fn span(&self) -> Span {
        match self {
            Self::Number { span, .. }
            | Self::Identifier { span, .. }
            | Self::Unary { span, .. }
            | Self::Binary { span, .. }
            | Self::Assign { span, .. }
            | Self::Conditional { span, .. }
            | Self::Call { span, .. }
            | Self::Subscript { span, .. }
            | Self::Comma { span, .. }
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
    /// was that walker, it is private to `driver.rs`, and the driver is the top
    /// of the dependency graph, so nothing below it could read the answer even
    /// within this crate. `docs/roadmap.md` then moves the Safety IR into a
    /// crate of its own in Phase 2, which is not this crate and not today.
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
    /// correctly, which is what RK-015 in the review knowledge bank records. The
    /// answer is held by the corpus instead: `--emit ast` prints what this
    /// returns, so a child dropped from an arm is lines missing from expected
    /// files that are compared byte for byte.
    pub fn extend_children(&self, out: &mut Vec<ExprId>) {
        match self {
            Self::Number { .. } | Self::Identifier { .. } | Self::Error { .. } => {}
            Self::Unary { operand, .. } => out.push(*operand),
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
}

impl Ast {
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

    /// Whether `left` and `right` are compatible types, C17 6.2.7 p1.
    ///
    /// Compatible and not identical, which is the relation C actually asks
    /// about: 6.5.16.1 p1 wants the pointed-to types of an assignment to be
    /// compatible, and 6.2.7 p1 makes identical types compatible and then adds
    /// cases that are compatible without being identical. `clang` keeps the
    /// two apart as `hasSameType` and `typesAreCompatible` for that reason.
    /// One of the extra cases is reachable in the subset this compiler reads,
    /// and it is in `compatible_parameters` below; a struct will bring the
    /// rest, and this is named for the relation so that whoever adds one is
    /// extending the right thing.
    ///
    /// The comparison [`Type`]'s own comment refuses to derive, written where
    /// the arena is, because a type reaches the rest of itself through ids and
    /// answering means walking them: `int *p;` and `int *q;` are two `Pointer`
    /// nodes holding two `Int` nodes, and they are the same type.
    ///
    /// **An array's length is not compared, and that is a known divergence.**
    /// 6.7.6.2 p6 makes two array types compatible only if both lengths are
    /// constant expressions and their values are equal, and nothing here
    /// evaluates a constant expression: `array_length_is_not_evaluated` in the
    /// corpus is a case that says so by name. So `int[2]` and `int[3]` answer
    /// "compatible", which is a thing this compiler fails to notice rather
    /// than a thing it says wrongly: `int (*p)[3]; int a[2]; p = &a;` is
    /// accepted here and rejected by `clang`. `types.rs` reaches this through
    /// a pointer's pointee, so the missed report is live rather than waiting
    /// for a later issue.
    ///
    /// Recursion, bounded the way `parser.rs::apply` bounds a declarator's
    /// derivations, so a type is at most `MAX_NESTING` deep. The nesting a
    /// parameter list adds is bounded by the parser's own recursion.
    pub fn compatible(&self, left: TypeId, right: TypeId) -> bool {
        match (self.ty(left), self.ty(right)) {
            (Type::Int, Type::Int) | (Type::Char, Type::Char) | (Type::Void, Type::Void) => true,
            (Type::Pointer(left), Type::Pointer(right)) => self.compatible(*left, *right),
            (Type::Array { element: left, .. }, Type::Array { element: right, .. }) => {
                self.compatible(*left, *right)
            }
            (
                Type::Function {
                    returns: left,
                    parameters: left_parameters,
                },
                Type::Function {
                    returns: right,
                    parameters: right_parameters,
                },
            ) => {
                self.compatible(*left, *right)
                    && self.compatible_parameters(left_parameters, right_parameters)
            }
            // Written out rather than `_ => false`, so that a variant added to
            // `Type` has to be answered for here: `error[E0004]` is what says
            // somebody looked, and a wildcard would call the new type
            // different from everything including itself.
            (Type::Int, _)
            | (Type::Char, _)
            | (Type::Void, _)
            | (Type::Pointer(_), _)
            | (Type::Array { .. }, _)
            | (Type::Function { .. }, _) => false,
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
    fn compatible_parameters(&self, left: &Parameters, right: &Parameters) -> bool {
        match (left, right) {
            (Parameters::Prototype(left), Parameters::Prototype(right)) => {
                left.len() == right.len()
                    && left
                        .iter()
                        .zip(right)
                        .all(|(left, right)| self.compatible(left.ty, right.ty))
            }
            (Parameters::Unspecified, Parameters::Unspecified) => true,
            (Parameters::Prototype(parameters), Parameters::Unspecified)
            | (Parameters::Unspecified, Parameters::Prototype(parameters)) => parameters
                .iter()
                .all(|parameter| !self.is_promoted_by_default(parameter.ty)),
        }
    }

    /// Whether the default argument promotions change this type, C17 6.5.2.2
    /// p6.
    ///
    /// They promote a `char` to an `int` and a `float` to a `double`. This
    /// compiler has no `float`, so `char` is the whole of it, and an arm for
    /// each of the others rather than a wildcard so that a type added later
    /// has to say which side it is on.
    fn is_promoted_by_default(&self, ty: TypeId) -> bool {
        match self.ty(ty) {
            Type::Char => true,
            Type::Int
            | Type::Void
            | Type::Pointer(_)
            | Type::Array { .. }
            | Type::Function { .. } => false,
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
/// where this spells it `int (int[10])`. Those adjustments are semantic rules,
/// and this stage records what was written.
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
        let base = match ast.ty(id) {
            Type::Int => "int",
            Type::Char => "char",
            Type::Void => "void",
            Type::Pointer(pointee) => {
                inner = if binds_tighter_than_a_pointer(ast.ty(*pointee)) {
                    format!("(*{inner})")
                } else {
                    format!("*{inner}")
                };
                id = *pointee;
                continue;
            }
            Type::Array { element, length } => {
                // The length is the source's own bytes and is not evaluated, so
                // `int a[1 + 2]` spells `int[1 + 2]` where `clang`, which does
                // evaluate it, spells `int[3]`. What 6.7.6.2 p1 asks of it is a
                // constraint, and constraints are checked later.
                let length = match length {
                    Some(length) => sources.snippet(ast.expr(*length).span()),
                    None => "",
                };
                inner = format!("{inner}[{length}]");
                id = *element;
                continue;
            }
            Type::Function {
                returns,
                parameters,
            } => {
                inner = format!("{inner}({})", spell_parameters(sources, ast, parameters));
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
        Type::Int | Type::Char | Type::Void | Type::Pointer(_) => false,
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
        .map(|parameter| spell_type(sources, ast, parameter.ty))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use safec_ir::source::SourceMap;

    /// What C17 calls compatible types, and what it does not.
    ///
    /// The pairs are written out rather than derived, for the reason RK-001
    /// gives: a table built the way the code builds one compares the code with
    /// itself. Each row is a declaration a reader can write down, and the two
    /// sides are separate nodes in the arena, which is the whole point: a
    /// derived `PartialEq` would call the two halves of every `true` row
    /// different.
    ///
    /// Mutation: have the `Pointer` arm answer `true` without comparing its
    /// pointee. `int *` and `char *` become compatible and this fails.
    /// Mutation: compare `Parameters` by length alone. `int (int)` and
    /// `int (char)` become compatible and this fails. Mutation: have
    /// `is_promoted_by_default` answer `false` for a `char`. `int (char)` and
    /// `int ()` become compatible and this fails.
    #[test]
    fn two_types_are_compatible_when_c_says_they_are() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", "23");
        let mut ast = Ast::new();
        let two = ast.push_expr(Expr::Number {
            span: Span::new(file, 0, 1),
        });
        let three = ast.push_expr(Expr::Number {
            span: Span::new(file, 1, 2),
        });

        let int = ast.push_type(Type::Int);
        let also_int = ast.push_type(Type::Int);
        let character = ast.push_type(Type::Char);
        let pointer_to_int = ast.push_type(Type::Pointer(int));
        let also_pointer_to_int = ast.push_type(Type::Pointer(also_int));
        let pointer_to_char = ast.push_type(Type::Pointer(character));
        let pointer_to_pointer = ast.push_type(Type::Pointer(pointer_to_int));
        let two_ints = ast.push_type(Type::Array {
            element: int,
            length: Some(two),
        });
        let three_ints = ast.push_type(Type::Array {
            element: also_int,
            length: Some(three),
        });
        let two_chars = ast.push_type(Type::Array {
            element: character,
            length: Some(two),
        });

        let unnamed = |ty| Declaration {
            name: None,
            ty,
            span: Span::new(file, 0, 1),
        };
        let of = |ast: &mut Ast, returns, parameters| {
            ast.push_type(Type::Function {
                returns,
                parameters,
            })
        };
        let int_of_int = of(&mut ast, int, Parameters::Prototype(vec![unnamed(int)]));
        let int_of_also_int = of(
            &mut ast,
            also_int,
            Parameters::Prototype(vec![unnamed(also_int)]),
        );
        let int_of_char = of(
            &mut ast,
            int,
            Parameters::Prototype(vec![unnamed(character)]),
        );
        let char_of_int = of(
            &mut ast,
            character,
            Parameters::Prototype(vec![unnamed(int)]),
        );
        let int_of_two_ints = of(
            &mut ast,
            int,
            Parameters::Prototype(vec![unnamed(int), unnamed(int)]),
        );
        let int_of_void = of(&mut ast, int, Parameters::Prototype(Vec::new()));
        let int_of_nothing_said = of(&mut ast, int, Parameters::Unspecified);
        let also_int_of_nothing_said = of(&mut ast, also_int, Parameters::Unspecified);

        for (left, right, same, what) in [
            (int, also_int, true, "int against int"),
            (int, character, false, "int against char"),
            (
                pointer_to_int,
                also_pointer_to_int,
                true,
                "int * against int *",
            ),
            (
                pointer_to_int,
                pointer_to_char,
                false,
                "int * against char *",
            ),
            (
                pointer_to_int,
                pointer_to_pointer,
                false,
                "int * against int **",
            ),
            (pointer_to_int, int, false, "int * against int"),
            // 6.7.6.2 p6 says these are different types and this cannot say so:
            // the lengths are expressions nobody evaluates.
            (two_ints, three_ints, true, "int[2] against int[3]"),
            (two_ints, two_chars, false, "int[2] against char[2]"),
            (
                int_of_int,
                int_of_also_int,
                true,
                "int (int) against int (int)",
            ),
            (
                int_of_int,
                int_of_char,
                false,
                "int (int) against int (char)",
            ),
            (
                int_of_int,
                char_of_int,
                false,
                "int (int) against char (int)",
            ),
            (
                int_of_int,
                int_of_two_ints,
                false,
                "int (int) against int (int, int)",
            ),
            // 6.7.6.3 p15's own case: a prototype whose parameters are
            // unchanged by the default argument promotions is compatible with
            // an empty identifier list, and a `char` parameter is changed.
            (
                int_of_void,
                int_of_nothing_said,
                true,
                "int (void) against int ()",
            ),
            (
                int_of_int,
                int_of_nothing_said,
                true,
                "int (int) against int ()",
            ),
            (
                int_of_char,
                int_of_nothing_said,
                false,
                "int (char) against int ()",
            ),
            (
                int_of_nothing_said,
                also_int_of_nothing_said,
                true,
                "int () against int ()",
            ),
        ] {
            assert_eq!(ast.compatible(left, right), same, "{what}");
            assert_eq!(ast.compatible(right, left), same, "{what}, the other way");
        }
    }

    /// Every shape of type, and how C declares one of it.
    ///
    /// The expected strings are written out rather than derived from the types,
    /// for the reason RK-001 gives: a test that builds its expectation the way
    /// the code does is comparing the code with itself. These were taken from
    /// `clang -Xclang -ast-dump` on the same declarations, so they are what
    /// another compiler prints and not what this one happens to.
    ///
    /// Mutation: drop the parentheses from the pointer arm of `spell_type`, or
    /// change where the space goes. This fails.
    #[test]
    fn every_type_is_spelled_the_way_c_declares_it() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", "10");
        let mut ast = Ast::new();
        let ten = ast.push_expr(Expr::Number {
            span: Span::new(file, 0, 2),
        });

        let int = ast.push_type(Type::Int);
        let void = ast.push_type(Type::Void);
        let character = ast.push_type(Type::Char);
        let pointer_to_int = ast.push_type(Type::Pointer(int));
        let pointer_to_pointer = ast.push_type(Type::Pointer(pointer_to_int));
        let pointer_to_char = ast.push_type(Type::Pointer(character));
        let pointer_to_void = ast.push_type(Type::Pointer(void));
        let array_of_int = ast.push_type(Type::Array {
            element: int,
            length: Some(ten),
        });
        let incomplete = ast.push_type(Type::Array {
            element: int,
            length: None,
        });
        let array_of_pointer = ast.push_type(Type::Array {
            element: pointer_to_int,
            length: Some(ten),
        });
        let pointer_to_array = ast.push_type(Type::Pointer(array_of_int));

        let unnamed = |ty| Declaration {
            name: None,
            ty,
            span: Span::new(file, 0, 2),
        };
        let takes_int = |returns| Type::Function {
            returns,
            parameters: Parameters::Prototype(vec![unnamed(int)]),
        };
        let returns_int = ast.push_type(takes_int(int));
        let returns_pointer = ast.push_type(takes_int(pointer_to_int));
        let pointer_to_function = ast.push_type(Type::Pointer(returns_int));
        let takes_nothing = ast.push_type(Type::Function {
            returns: int,
            parameters: Parameters::Prototype(Vec::new()),
        });
        let unspecified = ast.push_type(Type::Function {
            returns: int,
            parameters: Parameters::Unspecified,
        });

        for (ty, spelling) in [
            (int, "int"),
            (character, "char"),
            (void, "void"),
            (pointer_to_int, "int *"),
            (pointer_to_pointer, "int **"),
            (pointer_to_void, "void *"),
            (pointer_to_char, "char *"),
            (array_of_int, "int[10]"),
            (incomplete, "int[]"),
            (array_of_pointer, "int *[10]"),
            (pointer_to_array, "int (*)[10]"),
            (returns_int, "int (int)"),
            (returns_pointer, "int *(int)"),
            (pointer_to_function, "int (*)(int)"),
            (takes_nothing, "int (void)"),
            (unspecified, "int ()"),
        ] {
            assert_eq!(spell_type(&sources, &ast, ty), spelling, "{ty:?}");
        }
    }

    /// A span in a file that exists, because `Span` cannot be built without a
    /// `FileId` and a `FileId` cannot be built without a file. What these tests
    /// are about is the indices, so the file is a formality.
    fn span(start: u32) -> Span {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("test.c", "x".repeat(64));
        Span::new(file, start, start + 1)
    }

    /// A walk that hands its own worklist to `extend_children` reaches every
    /// node.
    ///
    /// This is the shortest correct walk of an expression tree, and it is
    /// correct only because `extend_children` appends. Nothing in the signature
    /// says which it does, and the corpus cannot tell: `dump_expr` clears its
    /// buffer before every call, so it reads the same either way. This is the
    /// only thing that does.
    ///
    /// Mutation: put `out.clear()` at the top of `extend_children`. The walk
    /// then loses everything it had queued each time it asks a node for its
    /// children, and this fails on the count. It fails quietly in a caller,
    /// which is the point: a safety analysis that visits a fifth of a function
    /// and reports nothing is the failure `docs/safety-model.md` exists to
    /// prevent, and it does not announce itself.
    #[test]
    fn the_simplest_walk_over_the_tree_reaches_every_node() {
        let s = span(0);
        let mut ast = Ast::default();

        // `a[b] + c`, built by hand rather than parsed: this is about the
        // arena, and `parser.rs` is not the thing under test.
        let a = ast.push_expr(Expr::Identifier { span: s });
        let b = ast.push_expr(Expr::Identifier { span: s });
        let subscript = ast.push_expr(Expr::Subscript {
            base: a,
            index: b,
            span: s,
        });
        let c = ast.push_expr(Expr::Identifier { span: s });
        let root = ast.push_expr(Expr::Binary {
            op: BinOp::Add,
            lhs: subscript,
            rhs: c,
            span: s,
        });

        let mut work = vec![root];
        let mut visited = 0;
        while let Some(id) = work.pop() {
            visited += 1;
            assert!(visited <= 5, "the walk did not terminate");
            ast.expr(id).extend_children(&mut work);
        }

        assert_eq!(visited, 5, "every node in the tree is reached exactly once");
    }

    /// Every node kind, and what it is called in the artifact.
    ///
    /// Written out here rather than derived from the enums, because a test that
    /// walks a table and checks it against itself proves nothing. RK-001 in the
    /// review knowledge bank is that lesson, learned on this crate's keyword
    /// table, where `return` spelled `retrun` passed the whole suite. Its own
    /// note says the next table is the AST node kinds, and this is it.
    ///
    /// Mutation: rename any arm in a `name` method. This fails.
    #[test]
    fn every_node_kind_is_named_the_way_the_artifact_spells_it() {
        let s = span(0);
        let e = ExprId(0);
        let t = StmtId(0);

        assert_eq!(Expr::Number { span: s }.name(), "Number");
        assert_eq!(Expr::Identifier { span: s }.name(), "Identifier");
        assert_eq!(
            Expr::Unary {
                op: UnOp::Not,
                operand: e,
                span: s
            }
            .name(),
            "Unary"
        );
        assert_eq!(
            Expr::Binary {
                op: BinOp::Add,
                lhs: e,
                rhs: e,
                span: s
            }
            .name(),
            "Binary"
        );
        assert_eq!(
            Expr::Assign {
                op: None,
                place: e,
                value: e,
                span: s
            }
            .name(),
            "Assign"
        );
        assert_eq!(
            Expr::Conditional {
                condition: e,
                then: e,
                otherwise: e,
                span: s
            }
            .name(),
            "Conditional"
        );
        assert_eq!(
            Expr::Call {
                callee: e,
                arguments: Vec::new(),
                span: s
            }
            .name(),
            "Call"
        );
        assert_eq!(
            Expr::Subscript {
                base: e,
                index: e,
                span: s
            }
            .name(),
            "Subscript"
        );
        assert_eq!(
            Expr::Comma {
                lhs: e,
                rhs: e,
                span: s
            }
            .name(),
            "Comma"
        );
        assert_eq!(Expr::Error { span: s }.name(), "Error");

        assert_eq!(
            Stmt::Compound {
                body: Vec::new(),
                span: s
            }
            .name(),
            "Compound"
        );
        assert_eq!(
            Stmt::Return {
                value: None,
                span: s
            }
            .name(),
            "Return"
        );
        assert_eq!(
            Stmt::Expression {
                value: None,
                span: s
            }
            .name(),
            "Expression"
        );
        assert_eq!(
            Stmt::If {
                condition: e,
                then: t,
                otherwise: None,
                span: s
            }
            .name(),
            "If"
        );
        assert_eq!(
            Stmt::While {
                condition: e,
                body: t,
                span: s
            }
            .name(),
            "While"
        );
        assert_eq!(
            Stmt::For {
                initialiser: None,
                condition: None,
                step: None,
                body: t,
                span: s
            }
            .name(),
            "For"
        );
        assert_eq!(Stmt::Error { span: s }.name(), "Error");

        assert_eq!(
            Item::Function(Function {
                ty: TypeId(0),
                name: s,
                body: StmtId(0),
                span: s
            })
            .name(),
            "Function"
        );
        assert_eq!(Item::Error { span: s }.name(), "Error");
    }

    /// Every operator, and how C spells it.
    ///
    /// Written out for the reason the test above is: a table checked against
    /// itself proves nothing, and this one is read straight into an artifact a
    /// corpus case compares byte for byte.
    ///
    /// Mutation: change any spelling. This fails.
    #[test]
    fn every_operator_is_spelled_the_way_c_spells_it() {
        for (op, spelling) in [
            (BinOp::Mul, "*"),
            (BinOp::Div, "/"),
            (BinOp::Rem, "%"),
            (BinOp::Add, "+"),
            (BinOp::Sub, "-"),
            (BinOp::Shl, "<<"),
            (BinOp::Shr, ">>"),
            (BinOp::Lt, "<"),
            (BinOp::Gt, ">"),
            (BinOp::Le, "<="),
            (BinOp::Ge, ">="),
            (BinOp::Eq, "=="),
            (BinOp::Ne, "!="),
            (BinOp::BitAnd, "&"),
            (BinOp::BitXor, "^"),
            (BinOp::BitOr, "|"),
            (BinOp::LogAnd, "&&"),
            (BinOp::LogOr, "||"),
        ] {
            assert_eq!(op.as_str(), spelling, "{op:?}");
        }

        for (op, spelling, fixity) in [
            (UnOp::Plus, "+", None),
            (UnOp::Minus, "-", None),
            (UnOp::Not, "!", None),
            (UnOp::BitNot, "~", None),
            (UnOp::Deref, "*", None),
            (UnOp::AddrOf, "&", None),
            (UnOp::PreInc, "++", Some("prefix")),
            (UnOp::PreDec, "--", Some("prefix")),
            (UnOp::PostInc, "++", Some("postfix")),
            (UnOp::PostDec, "--", Some("postfix")),
        ] {
            assert_eq!(op.as_str(), spelling, "{op:?}");
            assert_eq!(op.fixity(), fixity, "{op:?}");
        }
    }

    /// An id still names its node after more nodes are added.
    ///
    /// This is the property `Box` cannot give and the whole reason the tree is
    /// flat. See ADR-0008. A phase that walks the tree holds ids across the
    /// pushes a later phase makes, and every one of them has to still mean what
    /// it meant.
    ///
    /// Mutation: take the id from `len()` after the push rather than before.
    /// This fails.
    #[test]
    fn an_id_still_names_its_node_after_more_are_pushed() {
        let mut ast = Ast::new();

        let first = ast.push_expr(Expr::Number { span: span(0) });
        let second = ast.push_expr(Expr::Number { span: span(10) });
        ast.push_expr(Expr::Error { span: span(20) });

        assert_eq!(ast.expr(first), &Expr::Number { span: span(0) });
        assert_eq!(ast.expr(second), &Expr::Number { span: span(10) });
    }

    /// The three arenas are separate, so an index into one says nothing about
    /// the others and cannot be used against them.
    ///
    /// Mutation: give `push_stmt` the length of `exprs`. This fails.
    #[test]
    fn each_kind_of_node_is_numbered_on_its_own() {
        let mut ast = Ast::new();

        ast.push_expr(Expr::Number { span: span(0) });
        ast.push_expr(Expr::Number { span: span(1) });
        let stmt = ast.push_stmt(Stmt::Error { span: span(2) });

        assert_eq!(stmt, StmtId(0));
    }
}
