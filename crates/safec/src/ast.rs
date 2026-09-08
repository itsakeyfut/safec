//! The tree the parser builds, and the only thing the phases after it walk.
//!
//! Nodes live in flat vectors and a child is an index rather than a pointer.
//! See ADR-0008 for why, and for what it rejected.
//!
//! What is here is the subset [`docs/frontend.md`] calls Stage 1 minus almost
//! all of it: a function with no parameters, a compound statement, `return`,
//! and a numeric constant. The siblings of the issue that added it fill in
//! expressions, control flow, declarators and structs, and each of them adds
//! variants here rather than changing the shape.
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
    /// A statement the parser could not read.
    Error {
        /// What it gave up on.
        span: Span,
    },
}

/// A function definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function {
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
    /// A function definition.
    Function(Function),
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
            Self::Error { .. } => "Error",
        }
    }

    /// Where this node is.
    pub fn span(&self) -> Span {
        match self {
            Self::Number { span } | Self::Error { span } => *span,
        }
    }
}

impl Stmt {
    /// What this kind of node is called, for `--emit ast`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Compound { .. } => "Compound",
            Self::Return { .. } => "Return",
            Self::Error { .. } => "Error",
        }
    }

    /// Where this node is.
    pub fn span(&self) -> Span {
        match self {
            Self::Compound { span, .. } | Self::Return { span, .. } | Self::Error { span } => *span,
        }
    }
}

impl Item {
    /// What this kind of node is called, for `--emit ast`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Function(_) => "Function",
            Self::Error { .. } => "Error",
        }
    }

    /// Where this node is.
    pub fn span(&self) -> Span {
        match self {
            Self::Function(function) => function.span,
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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ast {
    exprs: Vec<Expr>,
    stmts: Vec<Stmt>,
    items: Vec<Item>,
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

        assert_eq!(Expr::Number { span: s }.name(), "Number");
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
        assert_eq!(Stmt::Error { span: s }.name(), "Error");

        assert_eq!(
            Item::Function(Function {
                name: s,
                body: StmtId(0),
                span: s
            })
            .name(),
            "Function"
        );
        assert_eq!(Item::Error { span: s }.name(), "Error");
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
