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
//! long. Anything that walks this owes itself an answer to the second, and
//! `driver.rs`'s `dump_expr` is what one looks like.
//!
//! [`docs/frontend.md`]: https://github.com/itsakeyfut/safec/blob/main/docs/frontend.md

use crate::source::Span;

/// Where an expression is in [`Ast`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExprId(u32);

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
    /// The specifiers through the declarator, and through the `;` as well for
    /// a declaration that carries one. A parameter has no `;` and stops at its
    /// declarator.
    pub span: Span,
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
    Declaration(Declaration),
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
    /// needs an initializer and so needs #43. These are three separate
    /// `Option`s rather than something that could also hold a declaration,
    /// because an interface with no caller is invented rather than designed and
    /// widening this one later is additive.
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
    Declaration(Declaration),
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
}

impl Stmt {
    /// What this kind of node is called, for `--emit ast`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Compound { .. } => "Compound",
            Self::Return { .. } => "Return",
            Self::Declaration(_) => "Declaration",
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
            | Self::Error { span } => *span,
            Self::Declaration(declaration) => declaration.span,
        }
    }
}

impl Item {
    /// What this kind of node is called, for `--emit ast`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Function(_) => "Function",
            Self::Declaration(_) => "Declaration",
            Self::Error { .. } => "Error",
        }
    }

    /// Where this node is.
    pub fn span(&self) -> Span {
        match self {
            Self::Function(function) => function.span,
            Self::Declaration(declaration) => declaration.span,
            Self::Error { span } => *span,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceMap;

    /// A span in a file that exists, because `Span` cannot be built without a
    /// `FileId` and a `FileId` cannot be built without a file. What these tests
    /// are about is the indices, so the file is a formality.
    fn span(start: u32) -> Span {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("test.c", "x".repeat(64));
        Span::new(file, start, start + 1)
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
