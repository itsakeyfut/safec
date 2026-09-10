//! From the typed AST to the Safety IR.
//!
//! The first reader of what [`crate::sema`] and [`crate::types`] work out, and
//! the last place a program is still shaped the way it was written. Everything
//! after this reads blocks and places.
//!
//! **What cannot be lowered is reported, and its function is left a
//! declaration.** The alternative is a definition with a hole in it, and
//! nothing in the IR says a hole is there: an analysis walking such a function
//! would conclude about code it never saw. A declaration says exactly what is
//! true, that the body is not here, which is a thing [`Function::declaration`]
//! already exists to say.
//!
//! Three shapes are refused today, all of them `SC0304`: an expression the
//! frontend could not type, a type the IR cannot hold, and a name the IR
//! cannot reach. They are one code rather than three because the code says
//! which stage could not go on and the message says what it met, which is how
//! `parser.rs` spells every syntax error.

use std::collections::HashMap;

use crate::ast::{Ast, BinOp as AstBinOp, Expr, ExprId, Item, Parameters, Stmt, StmtId, Type};
use crate::ast::{TypeId, UnOp as AstUnOp, spell_type};
use crate::diagnostics::{Code, Diagnostic, DiagnosticSink, Label};
use crate::ir::{
    BinOp, Block, BlockId, FuncId, Function, LocalId, Operand, Operation, Origin, Place,
    Projection, Rvalue, Terminator, TranslationUnit, Ty, TyId, UnOp,
};
use crate::sema::Resolution;
use crate::source::{SourceMap, Span};
use crate::types::Types;

/// Something the frontend accepted and this stage cannot express.
///
/// One code for every shape of it, for the reason `types.rs` gives for
/// `MISMATCH`: what differs between them is the message, and a reader filtering
/// on the code wants "the IR could not be built" rather than a list of ways
/// that can happen.
const LOWERING: Code = Code::new("SC0304");

/// Build the IR of one translation unit.
///
/// Every function that can be lowered is, whatever the ones beside it did: a
/// program is not one thing that fails, and a caller of a function this stage
/// refused still resolves, because the refusal leaves a declaration behind.
pub fn lower(
    sources: &SourceMap,
    ast: &Ast,
    resolution: &Resolution,
    types: &Types,
    diagnostics: &mut DiagnosticSink,
) -> TranslationUnit {
    let mut lowering = Lowering {
        sources,
        ast,
        resolution,
        types,
        unit: TranslationUnit::new(),
        locals: HashMap::new(),
        functions: HashMap::new(),
        pending: HashMap::new(),
    };

    lowering.declare(diagnostics);
    lowering.define(diagnostics);
    lowering.unit
}

/// One translation unit, being lowered.
struct Lowering<'a> {
    sources: &'a SourceMap,
    ast: &'a Ast,
    resolution: &'a Resolution,
    types: &'a Types,
    unit: TranslationUnit,
    /// The local a declared name means, keyed by the span of that name.
    ///
    /// A [`crate::sema::Binding`] says a name and a type and not which function
    /// it belongs to, and a resolution is keyed on the use site, so a
    /// declaration cannot be asked which binding it made. A name's span can:
    /// it is where the name was declared, so it is one per declaration, and a
    /// use reaches it through the binding it resolved to.
    locals: HashMap<Span, LocalId>,
    /// The same, one layer up: which function a name at this span became.
    functions: HashMap<Span, FuncId>,
    /// Where a `&&`, `||` or `?:` puts its answer, and where control rejoins.
    ///
    /// Filled when the first operand has been evaluated and read when the last
    /// one has. Keyed by the expression, because the two moments are two turns
    /// of the loop in [`Lowering::value`] rather than two lines of one
    /// function.
    pending: HashMap<ExprId, Pending>,
}

/// A branch a value is waiting on.
struct Pending {
    /// Where every arm writes its answer.
    answer: LocalId,
    /// Where the arms come back together.
    join: BlockId,
    /// Which block the `else` of a `?:` starts at.
    otherwise: Option<BlockId>,
}

/// One function, being built.
struct Builder {
    function: Function,
    /// The block being written into, or none where control cannot arrive.
    ///
    /// `None` after a terminator, which is what makes a `return` in the middle
    /// of a body need no special case: the statements after it open a block of
    /// their own that nothing jumps to. Leaving a block reserved and unfilled
    /// is what [`Function::blocks`] panics about, so the block is opened when
    /// something is written into it rather than when the last one ended.
    block: Option<BlockId>,
    /// What has been written into the block since it was opened.
    operations: Vec<Operation>,
}

impl Builder {
    /// The block being written into, opening one if there is none.
    fn open(&mut self) -> BlockId {
        match self.block {
            Some(block) => block,
            None => {
                let block = self.function.reserve_block();
                self.block = Some(block);
                block
            }
        }
    }

    /// Write an operation into the current block.
    fn push(&mut self, operation: Operation) {
        self.open();
        self.operations.push(operation);
    }

    /// End the current block, and leave none open.
    fn end(&mut self, terminator: Terminator) {
        let block = self.open();
        let operations = std::mem::take(&mut self.operations);
        self.function.fill_block(
            block,
            Block {
                operations,
                terminator,
            },
        );
        self.block = None;
    }

    /// Begin writing into a block whose id was reserved earlier.
    ///
    /// # Panics
    ///
    /// If a block is still open. Two open blocks would mean operations written
    /// into whichever was current, which is the bug this shape exists to make
    /// impossible.
    fn switch(&mut self, block: BlockId) {
        assert!(self.block.is_none(), "a block was left open");
        self.block = Some(block);
    }

    /// Whether control can still arrive at what comes next.
    fn reachable(&self) -> bool {
        self.block.is_some()
    }
}

/// What is still to be done to one expression.
///
/// The walk is a stack rather than a recursion because the tree is not bounded
/// by the parser's own nesting limit: a chain folded by a loop, which is how
/// every left-associative operator is read, adds a level per operator. RK-001's
/// neighbour in the review knowledge bank is the entry, and `driver.rs`'s
/// `dump_expr` is the walker that paid for it first.
enum Task {
    /// Push the value of this expression.
    Value(ExprId),
    /// Push the place this expression names.
    Place(ExprId),
    /// The operands this node reads are on the stacks; produce its value.
    Finish(ExprId),
    /// The same, for a node being read as a place.
    FinishPlace(ExprId),
    /// The first operand of a `&&`, `||` or `?:` is done; branch on it.
    Split(ExprId),
    /// The `then` arm of a `?:` is done; start the `else`.
    Second(ExprId),
    /// The last arm is done; come back together.
    Merge(ExprId),
}

impl Lowering<'_> {
    /// Give every function a [`FuncId`] before any body is built.
    ///
    /// A call names its callee by id, and a callee can be defined after its
    /// caller or be the caller itself, so the ids come first and the bodies
    /// second. Until a body arrives the function is a declaration, which is
    /// what it truthfully is.
    fn declare(&mut self, diagnostics: &mut DiagnosticSink) {
        for item in self.ast.items() {
            let (name, ty) = match item {
                Item::Function(function) => (function.name, function.ty),
                Item::Declaration(declaration) => match declaration.name {
                    // A declaration of an object rather than a function is not
                    // in the IR at all: every place is rooted at a local, so
                    // there is nothing for a global to be. A use of one is
                    // reported where it is used, which is where a reader can
                    // see what it cost.
                    Some(name) => (name, declaration.ty),
                    None => continue,
                },
                Item::Error { .. } => continue,
            };

            let Type::Function {
                returns,
                parameters,
            } = self.ast.ty(ty)
            else {
                continue;
            };
            let (returns, parameters) = (*returns, parameters.clone());
            let Some((returns, lowered)) = self.signature(name, returns, &parameters, diagnostics)
            else {
                continue;
            };

            let id = self
                .unit
                .push_function(Function::declaration(name, returns, lowered));
            self.functions.insert(name, id);
        }
    }

    /// The declaration a call is checked against, and the locals a body starts
    /// with.
    ///
    /// Local 0 is the return place and the parameters follow it, which is what
    /// [`Function::new`] lays out. An empty parameter list is lowered as no
    /// parameters: C17 6.7.6.3 p14 makes `()` say nothing about the count
    /// rather than say there are none, and the IR has no way to spell "not
    /// said". Nothing here reads it, because a call carries the arguments it
    /// passes, and `types.rs` is where the count is checked.
    fn signature(
        &mut self,
        name: Span,
        returns: TypeId,
        parameters: &Parameters,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<(TyId, Vec<TyId>)> {
        let returns = self.ty(name, returns, diagnostics)?;
        let mut lowered = Vec::new();
        if let Parameters::Prototype(parameters) = parameters {
            for parameter in parameters {
                let at = parameter.name.unwrap_or(parameter.span);
                lowered.push(self.ty(at, parameter.ty, diagnostics)?);
            }
        }

        Some((returns, lowered))
    }

    /// Lower every function that has a body.
    fn define(&mut self, diagnostics: &mut DiagnosticSink) {
        for index in 0..self.ast.items().len() {
            let Item::Function(function) = &self.ast.items()[index] else {
                continue;
            };
            let (name, ty, body) = (function.name, function.ty, function.body);

            let Some(id) = self.functions.get(&name).copied() else {
                continue;
            };
            let Some(built) = self.body(name, ty, body, diagnostics) else {
                continue;
            };

            self.unit.fill_function(id, built);
        }
    }

    /// One function's blocks, or nothing where something could not be lowered.
    fn body(
        &mut self,
        name: Span,
        ty: TypeId,
        body: StmtId,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<Function> {
        let Type::Function {
            returns,
            parameters,
        } = self.ast.ty(ty)
        else {
            return None;
        };
        let (returns, parameters) = (*returns, parameters.clone());
        let (returns, lowered) = self.signature(name, returns, &parameters, diagnostics)?;
        let function = Function::new(name, returns, lowered);

        // A parameter is a local before the body's first statement, and its
        // name is how the body reaches it.
        if let Parameters::Prototype(parameters) = &parameters {
            for (parameter, local) in parameters.iter().zip(function.parameters()) {
                if let Some(at) = parameter.name {
                    self.locals.insert(at, local);
                }
            }
        }

        let mut builder = Builder {
            function,
            block: None,
            operations: Vec::new(),
        };
        builder.open();
        self.stmt(&mut builder, body, diagnostics)?;

        // Falling off the end leaves whatever the return place holds, which is
        // what C says of `main` and undefined behaviour to read anywhere else.
        // Saying that is the safety analyses' job and not this stage's.
        if builder.reachable() {
            builder.end(Terminator::Return);
        }

        Some(builder.function)
    }

    /// The IR type of a type the frontend wrote.
    ///
    /// A loop rather than a recursion, following `ast::spell_type`: the chain
    /// is bounded by the parser, and a walk over a type is the shape that stops
    /// being bounded first when something else builds one.
    fn ty(&mut self, at: Span, id: TypeId, diagnostics: &mut DiagnosticSink) -> Option<TyId> {
        let mut pointers = 0usize;
        let mut current = id;

        let base = loop {
            match self.ast.ty(current) {
                Type::Int => break Ty::Int,
                Type::Char => break Ty::Char,
                Type::Void => break Ty::Void,
                Type::Pointer(pointee) => {
                    pointers += 1;
                    current = *pointee;
                }
                // An array is not a pointer and saying it is would tell the
                // memory analysis that one object is another. #74 is where the
                // IR learns to hold the rest.
                Type::Array { .. } | Type::Function { .. } => {
                    diagnostics.report(
                        Diagnostic::error(format!(
                            "`{}` cannot be lowered to the Safety IR yet",
                            spell_type(self.sources, self.ast, id)
                        ))
                        .with_code(LOWERING)
                        .with_label(Label::primary(at, "this is the declaration"))
                        .with_note("the IR holds `int`, `char`, `void` and pointers to them"),
                    );
                    return None;
                }
            }
        };

        let mut ty = self.unit.push_type(base);
        for _ in 0..pointers {
            ty = self.unit.push_type(Ty::Pointer(ty));
        }

        Some(ty)
    }

    /// What the frontend said this expression's type is, or a report that it
    /// said nothing.
    ///
    /// Asked of every expression as it is reached, and not only of the ones
    /// that need a temporary to hold them. `x[0]` where `x` is an `int` needs
    /// none: it is a place, and lowering it without asking would have built a
    /// projection into an object with no elements while the type checker had
    /// already answered that it does not know what this is.
    fn typed(&mut self, id: ExprId, diagnostics: &mut DiagnosticSink) -> Option<TypeId> {
        let span = self.ast.expr(id).span();
        let Some(ty) = self.types.of(id) else {
            diagnostics.report(
                Diagnostic::error("this expression has no type, so it cannot be lowered")
                    .with_code(LOWERING)
                    .with_label(Label::primary(span, "the type of this is not known"))
                    .with_note(
                        "the safety analyses read the IR, and an operation whose type nothing \
                         worked out is one they cannot answer about",
                    ),
            );
            return None;
        };

        Some(ty)
    }

    /// The IR type of an expression, or a report that it has none.
    fn ty_of(&mut self, id: ExprId, diagnostics: &mut DiagnosticSink) -> Option<TyId> {
        let ty = self.typed(id, diagnostics)?;
        self.ty(self.ast.expr(id).span(), ty, diagnostics)
    }

    /// A statement, and everything under it.
    ///
    /// Recursion is what the statements a walker meets are bounded by:
    /// `parser::MAX_NESTING` counts a nested statement and not a folded
    /// operator, which is why `driver.rs`'s `dump_stmt` recurses where its
    /// `dump_expr` does not.
    fn stmt(
        &mut self,
        builder: &mut Builder,
        id: StmtId,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<()> {
        match self.ast.stmt(id) {
            Stmt::Compound { body, .. } => {
                for &statement in body {
                    self.stmt(builder, statement, diagnostics)?;
                }
            }
            Stmt::Return { value, span } => {
                let (value, span) = (*value, *span);
                if let Some(value) = value {
                    let operand = self.value(builder, value, diagnostics)?;
                    let place = Place::local(builder.function.return_place());
                    builder.push(Operation {
                        place,
                        value: Rvalue::Use(operand),
                        origin: Origin::Written(span),
                    });
                }
                builder.end(Terminator::Return);
            }
            Stmt::Declaration(declaration) => {
                let (name, ty, span) = (declaration.name, declaration.ty, declaration.span);
                // A declaration with no name declares nothing to write to, and
                // one with no initializer writes nothing: `Declaration` carries
                // no initializer, so a local is made and left alone until
                // something assigns to it.
                if let Some(name) = name {
                    let ty = self.ty(span, ty, diagnostics)?;
                    let local = builder.function.push_local(ty);
                    self.locals.insert(name, local);
                }
            }
            Stmt::Expression { value, .. } => {
                if let Some(value) = *value {
                    self.value(builder, value, diagnostics)?;
                }
            }
            Stmt::If {
                condition,
                then,
                otherwise,
                ..
            } => {
                let (condition, then, otherwise) = (*condition, *then, *otherwise);
                let condition = self.value(builder, condition, diagnostics)?;
                let taken = builder.function.reserve_block();
                let skipped = builder.function.reserve_block();
                let join = builder.function.reserve_block();
                builder.end(Terminator::Branch {
                    condition,
                    then: taken,
                    otherwise: skipped,
                });

                builder.switch(taken);
                self.stmt(builder, then, diagnostics)?;
                if builder.reachable() {
                    builder.end(Terminator::Goto(join));
                }

                // An `if` with no `else` still has an edge that skips the body,
                // and it is the same edge as an empty `else`.
                builder.switch(skipped);
                if let Some(otherwise) = otherwise {
                    self.stmt(builder, otherwise, diagnostics)?;
                }
                if builder.reachable() {
                    builder.end(Terminator::Goto(join));
                }

                builder.switch(join);
            }
            Stmt::While {
                condition, body, ..
            } => {
                let (condition, body) = (*condition, *body);
                let header = builder.function.reserve_block();
                builder.end(Terminator::Goto(header));

                // The condition is asked again on every turn, so it is written
                // into the header rather than before it: the back edge at the
                // end of the body arrives here, above the branch.
                builder.switch(header);
                let condition = self.value(builder, condition, diagnostics)?;
                let inside = builder.function.reserve_block();
                let after = builder.function.reserve_block();
                builder.end(Terminator::Branch {
                    condition,
                    then: inside,
                    otherwise: after,
                });

                builder.switch(inside);
                self.stmt(builder, body, diagnostics)?;
                if builder.reachable() {
                    builder.end(Terminator::Goto(header));
                }

                builder.switch(after);
            }
            Stmt::For {
                initialiser,
                condition,
                step,
                body,
                ..
            } => {
                let (initialiser, condition, step, body) = (*initialiser, *condition, *step, *body);
                if let Some(initialiser) = initialiser {
                    self.value(builder, initialiser, diagnostics)?;
                }

                let header = builder.function.reserve_block();
                builder.end(Terminator::Goto(header));
                builder.switch(header);

                let inside = builder.function.reserve_block();
                let after = builder.function.reserve_block();
                match condition {
                    Some(condition) => {
                        let condition = self.value(builder, condition, diagnostics)?;
                        builder.end(Terminator::Branch {
                            condition,
                            then: inside,
                            otherwise: after,
                        });
                    }
                    // 6.8.5.3 p2: an absent condition is replaced by a non-zero
                    // constant, so the loop has no exit edge of its own.
                    None => builder.end(Terminator::Goto(inside)),
                }

                builder.switch(inside);
                self.stmt(builder, body, diagnostics)?;
                if builder.reachable() {
                    if let Some(step) = step {
                        self.value(builder, step, diagnostics)?;
                    }
                }
                if builder.reachable() {
                    builder.end(Terminator::Goto(header));
                }

                builder.switch(after);
            }
            // The driver hands this stage a tree nothing reported about, so a
            // node the parser gave up on cannot be here. Reporting it would be
            // a second diagnostic about the first one's problem, which is what
            // the per-input gate exists to stop.
            Stmt::Error { .. } => {}
        }

        Some(())
    }

    /// The value of an expression, with an explicit stack rather than a
    /// recursion.
    ///
    /// See [`Task`] for why. Values and places come back on two stacks, and a
    /// node pops exactly what it reads, so the stacks are empty of its operands
    /// by the time it pushes its own.
    fn value(
        &mut self,
        builder: &mut Builder,
        root: ExprId,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<Operand> {
        let mut tasks = vec![Task::Value(root)];
        let mut values: Vec<Operand> = Vec::new();
        let mut places: Vec<Place> = Vec::new();

        while let Some(task) = tasks.pop() {
            match task {
                Task::Value(id) => {
                    self.begin_value(id, &mut tasks, &mut values, diagnostics)?;
                }
                Task::Place(id) => {
                    self.begin_place(id, &mut tasks, &mut places, diagnostics)?;
                }
                Task::Finish(id) => {
                    self.finish_value(builder, id, &mut values, &mut places, diagnostics)?;
                }
                Task::FinishPlace(id) => self.finish_place(id, &mut values, &mut places),
                Task::Split(id) => {
                    self.split(builder, id, &mut tasks, &mut values, diagnostics)?;
                }
                Task::Second(id) => self.second(builder, id, &mut tasks, &mut values),
                Task::Merge(id) => self.merge(builder, id, &mut values),
            }
        }

        Some(values.pop().expect("a value for the root"))
    }

    /// Push what an expression needs before its value can be built.
    fn begin_value(
        &mut self,
        id: ExprId,
        tasks: &mut Vec<Task>,
        values: &mut Vec<Operand>,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<()> {
        self.typed(id, diagnostics)?;

        match self.ast.expr(id) {
            Expr::Number { span } => values.push(Operand::Constant(self.constant(*span))),
            Expr::Identifier { .. } | Expr::Subscript { .. } => {
                tasks.push(Task::Finish(id));
                tasks.push(Task::Place(id));
            }
            Expr::Unary { op, operand, .. } => {
                tasks.push(Task::Finish(id));
                match op {
                    // These read a place rather than a value, and `&x` never
                    // reads `x` at all.
                    AstUnOp::Deref
                    | AstUnOp::AddrOf
                    | AstUnOp::PreInc
                    | AstUnOp::PreDec
                    | AstUnOp::PostInc
                    | AstUnOp::PostDec => tasks.push(Task::Place(*operand)),
                    AstUnOp::Plus | AstUnOp::Minus | AstUnOp::Not | AstUnOp::BitNot => {
                        tasks.push(Task::Value(*operand));
                    }
                }
            }
            Expr::Binary { op, lhs, rhs, .. } => {
                // `&&` and `||` are control flow: C17 6.5.13 p4 and 6.5.14 p4
                // say the right operand is not evaluated unless the left says
                // to, and `ir::BinOp` has no variant to lower them to.
                if matches!(op, AstBinOp::LogAnd | AstBinOp::LogOr) {
                    tasks.push(Task::Split(id));
                    tasks.push(Task::Value(*lhs));
                } else {
                    tasks.push(Task::Finish(id));
                    tasks.push(Task::Value(*rhs));
                    tasks.push(Task::Value(*lhs));
                }
            }
            Expr::Assign { place, value, .. } => {
                tasks.push(Task::Finish(id));
                tasks.push(Task::Value(*value));
                tasks.push(Task::Place(*place));
            }
            Expr::Conditional { condition, .. } => {
                tasks.push(Task::Split(id));
                tasks.push(Task::Value(*condition));
            }
            Expr::Call { arguments, .. } => {
                tasks.push(Task::Finish(id));
                for &argument in arguments.iter().rev() {
                    tasks.push(Task::Value(argument));
                }
            }
            Expr::Comma { lhs, rhs, .. } => {
                // 6.5.17 p2: the left is evaluated as a void expression, so its
                // value is built and dropped rather than not built.
                tasks.push(Task::Finish(id));
                tasks.push(Task::Value(*rhs));
                tasks.push(Task::Value(*lhs));
            }
            // The parser reported whatever made this, and the driver's gate
            // means it never reaches here. Reporting it again would be a second
            // diagnostic about the first one's problem, so the function is
            // abandoned without one.
            Expr::Error { .. } => return None,
        }

        Some(())
    }

    /// Push what an expression needs before the place it names can be built.
    fn begin_place(
        &mut self,
        id: ExprId,
        tasks: &mut Vec<Task>,
        places: &mut Vec<Place>,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<()> {
        self.typed(id, diagnostics)?;

        match self.ast.expr(id) {
            Expr::Identifier { .. } => {
                places.push(Place::local(self.local(id, diagnostics)?));
            }
            Expr::Unary {
                op: AstUnOp::Deref,
                operand,
                ..
            } => {
                tasks.push(Task::FinishPlace(id));
                tasks.push(Task::Value(*operand));
            }
            Expr::Subscript { base, index, .. } => {
                tasks.push(Task::FinishPlace(id));
                tasks.push(Task::Value(*index));
                tasks.push(Task::Value(*base));
            }
            // Everything else has a value and no place. C17 6.5.16 p2 wants a
            // modifiable lvalue on the left of an assignment, and whether this
            // program breaks that constraint is the type checker's to say; what
            // is reported here is only that there is nothing to write into.
            _ => {
                diagnostics.report(
                    Diagnostic::error("this cannot be assigned to")
                        .with_code(LOWERING)
                        .with_label(Label::primary(
                            self.ast.expr(id).span(),
                            "this names no place",
                        ))
                        .with_note("the IR writes to places, and this expression is a value"),
                );
                return None;
            }
        }

        Some(())
    }

    /// Build a node's value from the operands its children left.
    fn finish_value(
        &mut self,
        builder: &mut Builder,
        id: ExprId,
        values: &mut Vec<Operand>,
        places: &mut Vec<Place>,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<()> {
        match self.ast.expr(id) {
            // A constant is its own value, so nothing asks it to finish. The
            // arm is here because the match is written out: an expression kind
            // added later has to say what it does rather than fall through.
            Expr::Number { .. } => {}
            Expr::Identifier { .. } | Expr::Subscript { .. } => {
                let place = places.pop().expect("a place");
                values.push(Operand::Copy(place));
            }
            Expr::Unary { op, span, .. } => {
                let span = *span;
                match op {
                    AstUnOp::Plus => {}
                    AstUnOp::Minus | AstUnOp::Not | AstUnOp::BitNot => {
                        let operand = values.pop().expect("an operand");
                        let op = match op {
                            AstUnOp::Minus => UnOp::Neg,
                            AstUnOp::Not => UnOp::Not,
                            _ => UnOp::BitNot,
                        };
                        let into = self.temporary(builder, id, diagnostics)?;
                        builder.push(Operation {
                            place: Place::local(into),
                            value: Rvalue::Unary { op, operand },
                            origin: Origin::Written(span),
                        });
                        values.push(Operand::Copy(Place::local(into)));
                    }
                    AstUnOp::Deref => {
                        let mut place = places.pop().expect("a place");
                        place.projection.push(Projection::Deref);
                        values.push(Operand::Copy(place));
                    }
                    AstUnOp::AddrOf => {
                        let place = places.pop().expect("a place");
                        let into = self.temporary(builder, id, diagnostics)?;
                        builder.push(Operation {
                            place: Place::local(into),
                            value: Rvalue::Address(place),
                            origin: Origin::Written(span),
                        });
                        values.push(Operand::Copy(Place::local(into)));
                    }
                    AstUnOp::PreInc | AstUnOp::PreDec | AstUnOp::PostInc | AstUnOp::PostDec => {
                        let place = places.pop().expect("a place");
                        let before = matches!(op, AstUnOp::PostInc | AstUnOp::PostDec);
                        let step = match op {
                            AstUnOp::PreInc | AstUnOp::PostInc => BinOp::Add,
                            _ => BinOp::Sub,
                        };

                        // A postfix operator is the same write and a different
                        // answer: the old value has to be kept before the place
                        // is written, because the place is where it was.
                        let kept = if before {
                            let kept = self.temporary(builder, id, diagnostics)?;
                            builder.push(Operation {
                                place: Place::local(kept),
                                value: Rvalue::Use(Operand::Copy(place.clone())),
                                origin: Origin::Written(span),
                            });
                            Some(kept)
                        } else {
                            None
                        };

                        builder.push(Operation {
                            place: place.clone(),
                            value: Rvalue::Binary {
                                op: step,
                                lhs: Operand::Copy(place.clone()),
                                rhs: Operand::Constant(1),
                            },
                            origin: Origin::Written(span),
                        });
                        values.push(match kept {
                            Some(kept) => Operand::Copy(Place::local(kept)),
                            None => Operand::Copy(place),
                        });
                    }
                }
            }
            Expr::Binary { op, span, .. } => {
                let (op, span) = (*op, *span);
                let rhs = values.pop().expect("a right operand");
                let lhs = values.pop().expect("a left operand");
                let op = binary(op).expect("`&&` and `||` are branches, not operators");
                let into = self.temporary(builder, id, diagnostics)?;
                builder.push(Operation {
                    place: Place::local(into),
                    value: Rvalue::Binary { op, lhs, rhs },
                    origin: Origin::Written(span),
                });
                values.push(Operand::Copy(Place::local(into)));
            }
            Expr::Assign { op, span, .. } => {
                let (op, span) = (*op, *span);
                let value = values.pop().expect("a value");
                let place = places.pop().expect("a place");
                let value = match op {
                    // `a += b` reads `a`, so the place is both what is read and
                    // what is written, and one operation says both.
                    Some(op) => Rvalue::Binary {
                        op: binary(op).expect("a compound assignment is never `&&` or `||`"),
                        lhs: Operand::Copy(place.clone()),
                        rhs: value,
                    },
                    None => Rvalue::Use(value),
                };
                builder.push(Operation {
                    place: place.clone(),
                    value,
                    origin: Origin::Written(span),
                });
                // 6.5.16 p3: the value of an assignment is what the left
                // operand holds afterwards.
                values.push(Operand::Copy(place));
            }
            Expr::Call {
                callee,
                arguments,
                span,
            } => {
                let (callee, count, span) = (*callee, arguments.len(), *span);
                let at = values.len() - count;
                let arguments: Vec<Operand> = values.split_off(at);
                let called = self.callee(callee, diagnostics)?;
                let into = self.temporary(builder, id, diagnostics)?;
                let then = builder.function.reserve_block();

                // A call ends a block: control leaves the function here, and
                // ADR-0010 is where that is argued.
                builder.end(Terminator::Call {
                    callee: called,
                    arguments,
                    destination: Some(Place::local(into)),
                    then,
                    origin: Origin::Written(span),
                });
                builder.switch(then);
                values.push(Operand::Copy(Place::local(into)));
            }
            Expr::Comma { .. } => {
                let rhs = values.pop().expect("a right operand");
                values.pop().expect("a left operand");
                values.push(rhs);
            }
            Expr::Conditional { .. } | Expr::Error { .. } => {}
        }

        Some(())
    }

    /// Build a node's place from what its children left.
    fn finish_place(&mut self, id: ExprId, values: &mut Vec<Operand>, places: &mut Vec<Place>) {
        match self.ast.expr(id) {
            Expr::Unary { .. } => {
                let operand = values.pop().expect("a pointer");
                let Operand::Copy(mut place) = operand else {
                    // A constant is not a place, so `*0` has nowhere to point.
                    // Nothing the frontend accepts reaches this today, and
                    // saying so is cheaper than a diagnostic nobody can produce.
                    return;
                };
                place.projection.push(Projection::Deref);
                places.push(place);
            }
            Expr::Subscript { .. } => {
                let index = values.pop().expect("an index");
                let base = values.pop().expect("a base");
                let Operand::Copy(mut place) = base else {
                    return;
                };
                place.projection.push(Projection::Index(index));
                places.push(place);
            }
            _ => {}
        }
    }

    /// Branch on the first operand of a `&&`, `||` or `?:`.
    fn split(
        &mut self,
        builder: &mut Builder,
        id: ExprId,
        tasks: &mut Vec<Task>,
        values: &mut Vec<Operand>,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<()> {
        let condition = values.pop().expect("a condition");
        let answer = self.temporary(builder, id, diagnostics)?;
        let join = builder.function.reserve_block();

        match self.ast.expr(id) {
            Expr::Binary { op, rhs, span, .. } => {
                let (op, rhs, span) = (*op, *rhs, *span);
                // The left operand is the answer unless the right is reached,
                // so it is written before the branch and overwritten after.
                builder.push(Operation {
                    place: Place::local(answer),
                    value: Rvalue::Use(condition),
                    origin: Origin::Written(span),
                });

                let second = builder.function.reserve_block();
                let (then, otherwise) = match op {
                    AstBinOp::LogAnd => (second, join),
                    _ => (join, second),
                };
                builder.end(Terminator::Branch {
                    condition: Operand::Copy(Place::local(answer)),
                    then,
                    otherwise,
                });
                builder.switch(second);

                self.pending.insert(
                    id,
                    Pending {
                        answer,
                        join,
                        otherwise: None,
                    },
                );
                tasks.push(Task::Merge(id));
                tasks.push(Task::Value(rhs));
            }
            Expr::Conditional { then, .. } => {
                let then = *then;
                let taken = builder.function.reserve_block();
                let otherwise = builder.function.reserve_block();
                builder.end(Terminator::Branch {
                    condition,
                    then: taken,
                    otherwise,
                });
                builder.switch(taken);

                self.pending.insert(
                    id,
                    Pending {
                        answer,
                        join,
                        otherwise: Some(otherwise),
                    },
                );
                tasks.push(Task::Second(id));
                tasks.push(Task::Value(then));
            }
            _ => {}
        }

        Some(())
    }

    /// Write the `then` arm of a `?:` and start its `else`.
    fn second(
        &mut self,
        builder: &mut Builder,
        id: ExprId,
        tasks: &mut Vec<Task>,
        values: &mut Vec<Operand>,
    ) {
        let Expr::Conditional {
            otherwise, span, ..
        } = self.ast.expr(id)
        else {
            return;
        };
        let (otherwise, span) = (*otherwise, *span);
        let pending = &self.pending[&id];
        let (answer, join, start) = (pending.answer, pending.join, pending.otherwise);

        let value = values.pop().expect("the first arm");
        builder.push(Operation {
            place: Place::local(answer),
            value: Rvalue::Use(value),
            origin: Origin::Written(span),
        });
        builder.end(Terminator::Goto(join));
        builder.switch(start.expect("a `?:` reserves its second arm"));

        tasks.push(Task::Merge(id));
        tasks.push(Task::Value(otherwise));
    }

    /// Write the last arm and come back together.
    fn merge(&mut self, builder: &mut Builder, id: ExprId, values: &mut Vec<Operand>) {
        let span = self.ast.expr(id).span();
        let Pending { answer, join, .. } = self.pending.remove(&id).expect("a branch to merge");

        let value = values.pop().expect("the last arm");
        builder.push(Operation {
            place: Place::local(answer),
            value: Rvalue::Use(value),
            origin: Origin::Written(span),
        });
        builder.end(Terminator::Goto(join));
        builder.switch(join);

        values.push(Operand::Copy(Place::local(answer)));
    }

    /// A local to hold what an expression works out.
    ///
    /// Every value that is not already in a place gets one, because a place is
    /// what the analyses ask about: `docs/safety-model.md`'s memory axis asks
    /// whether a place is still allocated, and a value with nowhere to live is
    /// a question it cannot be asked.
    fn temporary(
        &mut self,
        builder: &mut Builder,
        id: ExprId,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<LocalId> {
        let ty = self.ty_of(id, diagnostics)?;
        Some(builder.function.push_local(ty))
    }

    /// The local an identifier means.
    ///
    /// A name with no local is one the IR cannot reach: an object at file
    /// scope is the case that arrives first, because every place is rooted at a
    /// local and #74 is where that changes. A function used as a value is the
    /// other.
    fn local(&mut self, id: ExprId, diagnostics: &mut DiagnosticSink) -> Option<LocalId> {
        let span = self.ast.expr(id).span();
        let local = self
            .resolution
            .resolved(id)
            .map(|binding| self.resolution.binding(binding).name)
            .and_then(|name| self.locals.get(&name).copied());

        if local.is_none() {
            diagnostics.report(
                Diagnostic::error("this name cannot be lowered to the Safety IR")
                    .with_code(LOWERING)
                    .with_label(Label::primary(span, "this is not a local or a parameter"))
                    .with_note(
                        "every place the IR can name starts at a local, so an object declared                          outside a function has nothing to be",
                    ),
            );
        }

        local
    }

    /// The function a call names.
    fn callee(&mut self, callee: ExprId, diagnostics: &mut DiagnosticSink) -> Option<FuncId> {
        let span = self.ast.expr(callee).span();
        let named = self
            .resolution
            .resolved(callee)
            .map(|binding| self.resolution.binding(binding).name)
            .and_then(|name| self.functions.get(&name).copied());

        if named.is_none() {
            diagnostics.report(
                Diagnostic::error("this call cannot be lowered to the Safety IR")
                    .with_code(LOWERING)
                    .with_label(Label::primary(span, "this is not a function this stage found"))
                    .with_note("a call names a function by an id, and a call through a pointer has no id to name"),
            );
        }

        named
    }

    /// What an integer constant is worth.
    ///
    /// The text as written, read as a decimal. Suffixes, other bases and the
    /// rules of 6.4.4.1 for which type a constant has are the frontend's and
    /// have not arrived; what is here reads what the lexer accepted.
    fn constant(&self, span: Span) -> i128 {
        self.sources.snippet(span).trim().parse().unwrap_or(0)
    }
}

/// The IR operator an AST operator means, where one exists.
///
/// `LogAnd` and `LogOr` have none, and that is deliberate: they are branches,
/// which is what the doc comment on `ir::BinOp` says and what `split` builds.
fn binary(op: AstBinOp) -> Option<BinOp> {
    Some(match op {
        AstBinOp::Mul => BinOp::Mul,
        AstBinOp::Div => BinOp::Div,
        AstBinOp::Rem => BinOp::Rem,
        AstBinOp::Add => BinOp::Add,
        AstBinOp::Sub => BinOp::Sub,
        AstBinOp::Shl => BinOp::Shl,
        AstBinOp::Shr => BinOp::Shr,
        AstBinOp::Lt => BinOp::Lt,
        AstBinOp::Gt => BinOp::Gt,
        AstBinOp::Le => BinOp::Le,
        AstBinOp::Ge => BinOp::Ge,
        AstBinOp::Eq => BinOp::Eq,
        AstBinOp::Ne => BinOp::Ne,
        AstBinOp::BitAnd => BinOp::BitAnd,
        AstBinOp::BitXor => BinOp::BitXor,
        AstBinOp::BitOr => BinOp::BitOr,
        AstBinOp::LogAnd | AstBinOp::LogOr => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    use crate::parser::parse;
    use crate::sema::resolve;
    use crate::types::check;

    /// One program, lowered, and whatever was said about it on the way.
    struct Lowered {
        unit: TranslationUnit,
        diagnostics: DiagnosticSink,
        sources: SourceMap,
    }

    /// Lower a program the way the driver would, gate and all.
    ///
    /// The gate matters: `lower` is written for a tree nothing reported about,
    /// so a test that fed it a broken one would be testing a case the compiler
    /// does not produce. What is asserted below about a refusal is therefore
    /// always about a program the frontend accepted in silence.
    fn lowered(text: &str) -> Lowered {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", text);
        let mut diagnostics = DiagnosticSink::new();

        let tokens = lex(file, sources.file(file), &mut diagnostics);
        let mut ast = parse(file, &tokens, &mut diagnostics);
        assert!(!diagnostics.has_errors(), "the input did not parse");

        let resolution = resolve(&sources, &ast, &mut diagnostics);
        let types = check(&sources, &mut ast, &resolution, &mut diagnostics);
        assert!(!diagnostics.has_errors(), "the input did not check");

        let unit = lower(&sources, &ast, &resolution, &types, &mut diagnostics);
        Lowered {
            unit,
            diagnostics,
            sources,
        }
    }

    /// The codes of everything reported, in order.
    fn codes(lowered: &Lowered) -> Vec<String> {
        lowered
            .diagnostics
            .diagnostics()
            .iter()
            .filter_map(|diagnostic| diagnostic.code().map(|code| code.as_str().to_owned()))
            .collect()
    }

    /// The function that name belongs to.
    fn function<'a>(lowered: &'a Lowered, name: &str) -> &'a Function {
        lowered
            .unit
            .functions()
            .iter()
            .find(|function| lowered.sources.snippet(function.name) == name)
            .expect("a function of that name")
    }

    /// Which blocks each block can reach, in order.
    fn edges(function: &Function) -> Vec<Vec<usize>> {
        function
            .blocks()
            .map(|block| {
                let mut successors = Vec::new();
                block.terminator.successors(&mut successors);
                successors.iter().map(|block| block.index()).collect()
            })
            .collect()
    }

    /// The MVP program of `docs/roadmap.md` lowers, which is the first clause
    /// of Phase 2's Done-when.
    ///
    /// Mutation: have the `Call` arm write its result into the return place
    /// rather than into a temporary. `main` stops reading the call's answer
    /// back and this fails.
    ///
    /// Mutation: lower `a + b` as `BinOp::Sub`. The assertion on `add`'s
    /// operation fails.
    #[test]
    fn the_mvp_lowers() {
        let lowered = lowered(
            "int add(int a, int b) {\n    return a + b;\n}\n\nint main() {\n    return add(1, 2);\n}\n",
        );
        assert_eq!(codes(&lowered), Vec::<String>::new());

        let add = function(&lowered, "add");
        assert!(add.is_defined());
        let [entry] = add.blocks().collect::<Vec<_>>()[..] else {
            panic!("one block");
        };
        let [sum, returned] = &entry.operations[..] else {
            panic!("{:?}", entry.operations);
        };
        let [a, b] = add.parameters().collect::<Vec<_>>()[..] else {
            panic!("two parameters");
        };
        assert_eq!(
            sum.value,
            Rvalue::Binary {
                op: BinOp::Add,
                lhs: Operand::Copy(Place::local(a)),
                rhs: Operand::Copy(Place::local(b)),
            }
        );
        assert_eq!(returned.place, Place::local(add.return_place()));
        assert_eq!(
            returned.value,
            Rvalue::Use(Operand::Copy(sum.place.clone()))
        );
        assert_eq!(entry.terminator, Terminator::Return);

        // A call ends a block, so `return add(1, 2);` is two of them.
        let main = function(&lowered, "main");
        let [entry, after] = main.blocks().collect::<Vec<_>>()[..] else {
            panic!("two blocks");
        };
        assert!(entry.operations.is_empty());
        let Terminator::Call {
            callee,
            arguments,
            destination,
            then,
            origin,
        } = &entry.terminator
        else {
            panic!("{:?}", entry.terminator);
        };
        assert_eq!(
            lowered.sources.snippet(lowered.unit.function(*callee).name),
            "add"
        );
        assert_eq!(arguments, &[Operand::Constant(1), Operand::Constant(2)]);
        assert_eq!(then.index(), 1);
        assert_eq!(lowered.sources.snippet(origin.span()), "add(1, 2)");

        let destination = destination.clone().expect("somewhere to put the answer");
        let [returned] = &after.operations[..] else {
            panic!("{:?}", after.operations);
        };
        assert_eq!(returned.place, Place::local(main.return_place()));
        assert_eq!(returned.value, Rvalue::Use(Operand::Copy(destination)));
        assert_eq!(after.terminator, Terminator::Return);
    }

    /// A `while` is blocks and edges, and the body comes back to the condition.
    ///
    /// Mutation: end the body with `Goto` the block after the loop rather than
    /// the header. The back edge goes and this fails.
    ///
    /// Mutation: evaluate the condition before the header rather than in it.
    /// The header's operations are empty, the condition is asked once, and the
    /// assertion on where the comparison sits fails.
    #[test]
    fn a_while_loop_comes_back_to_its_condition() {
        let lowered = lowered(
            "int f(int n) {\n    while (n) {\n        n = n - 1;\n    }\n    return n;\n}\n",
        );
        assert_eq!(codes(&lowered), Vec::<String>::new());

        let f = function(&lowered, "f");
        let edges = edges(f);
        // The entry goes to the header, the header branches, the body comes
        // back, and the exit returns.
        assert_eq!(edges, vec![vec![1], vec![2, 3], vec![1], vec![]]);

        let blocks: Vec<_> = f.blocks().collect();
        assert!(matches!(blocks[1].terminator, Terminator::Branch { .. }));

        // The same edge again, said as the body's terminator rather than as an
        // index, so that the two ways of asking agree.
        let mut header = Vec::new();
        blocks[0].terminator.successors(&mut header);
        assert_eq!(blocks[2].terminator, Terminator::Goto(header[0]));

        // The condition is asked in the header, which is what the back edge
        // arrives above.
        assert_eq!(blocks[1].operations.len(), 0);
        assert!(matches!(
            blocks[1].terminator,
            Terminator::Branch {
                condition: Operand::Copy(_),
                ..
            }
        ));
    }

    /// An `if` with no `else` still has an edge that skips the body.
    ///
    /// Mutation: give the arm with no `else` no block, branching straight to
    /// the join. The skipping edge stops being a block of its own and the
    /// shape asserted here fails.
    #[test]
    fn an_if_with_no_else_still_joins() {
        let lowered =
            lowered("int f(int n) {\n    if (n) {\n        n = 0;\n    }\n    return n;\n}\n");
        assert_eq!(codes(&lowered), Vec::<String>::new());

        let f = function(&lowered, "f");
        // The entry branches to the body and to the empty arm, and both reach
        // the join, which returns.
        assert_eq!(edges(f), vec![vec![1, 2], vec![3], vec![3], vec![]]);
    }

    /// `&&` is a branch, because C says the right operand may not be evaluated.
    ///
    /// C17 6.5.13 p4. Mutation: lower it as an `ir::BinOp`; there is no variant
    /// to lower it to, so that mutation does not compile, which is the guard
    /// `ir::BinOp`'s doc comment claims.
    ///
    /// Mutation: write the left operand into the answer after the branch rather
    /// than before it. The answer of `0 && x` stops being written at all and
    /// the assertion on the first block's operations fails.
    #[test]
    fn a_short_circuit_is_a_branch_and_not_an_operator() {
        let lowered = lowered("int f(int a, int b) {\n    return a && b;\n}\n");
        assert_eq!(codes(&lowered), Vec::<String>::new());

        let f = function(&lowered, "f");
        // The entry branches on the left operand; one arm evaluates the right
        // and both arrive at the join.
        assert_eq!(edges(f), vec![vec![2, 1], vec![], vec![1]]);

        let blocks: Vec<_> = f.blocks().collect();
        let [held] = &blocks[0].operations[..] else {
            panic!("{:?}", blocks[0].operations);
        };
        let [a, _] = f.parameters().collect::<Vec<_>>()[..] else {
            panic!("two parameters");
        };
        assert_eq!(held.value, Rvalue::Use(Operand::Copy(Place::local(a))));
    }

    /// A function can call itself, which needs its id before its body.
    ///
    /// Mutation: give a function its id only once its body is built. `f` has
    /// no id to call and this fails.
    #[test]
    fn a_recursive_call_names_the_function_it_is_in() {
        let lowered = lowered("int f(int n) {\n    return f(n);\n}\n");
        assert_eq!(codes(&lowered), Vec::<String>::new());

        let f = function(&lowered, "f");
        let [entry, _] = f.blocks().collect::<Vec<_>>()[..] else {
            panic!("two blocks");
        };
        let Terminator::Call { callee, .. } = &entry.terminator else {
            panic!("{:?}", entry.terminator);
        };
        assert_eq!(
            lowered.sources.snippet(lowered.unit.function(*callee).name),
            "f"
        );
    }

    /// A function whose body is elsewhere can still be called.
    ///
    /// Mutation: skip `Item::Declaration` when handing out ids. The call has no
    /// callee, `SC0304` is reported, and this fails on both counts.
    #[test]
    fn a_declared_function_can_be_called() {
        let lowered = lowered("int g(int n);\n\nint f(int n) {\n    return g(n);\n}\n");
        assert_eq!(codes(&lowered), Vec::<String>::new());

        let g = function(&lowered, "g");
        assert!(!g.is_defined());
        assert!(function(&lowered, "f").is_defined());
    }

    /// An expression the frontend could not type is reported, and its function
    /// keeps no body.
    ///
    /// `x[0]` where `x` is an `int` is the case that reaches here: `types.rs`
    /// answers `None` for it and reports nothing, so nothing before this stage
    /// says the program has a hole in it.
    ///
    /// Mutation: lower an untyped expression as `Ty::Int`. No code is reported
    /// and this fails. Mutation: report and lower the rest of the function
    /// anyway. `is_defined` starts answering true and this fails.
    #[test]
    fn an_expression_with_no_type_is_reported_and_lowers_nothing() {
        let lowered = lowered("int f(void) {\n    int x;\n    return x[0];\n}\n");
        assert_eq!(codes(&lowered), ["SC0304"]);
        assert!(!function(&lowered, "f").is_defined());
    }

    /// A type the IR cannot hold is reported at the declaration that wrote it.
    ///
    /// The IR has `int`, `char`, `void` and pointers, and #74 is where the rest
    /// arrives. Lowering an array as a pointer would tell the memory analysis
    /// that one object is another.
    ///
    /// Mutation: lower `Type::Array` as a pointer to its element. Nothing is
    /// reported and this fails.
    #[test]
    fn a_type_the_ir_cannot_hold_is_reported() {
        let lowered = lowered("int f(void) {\n    int a[3];\n    return 0;\n}\n");
        assert_eq!(codes(&lowered), ["SC0304"]);
        assert!(!function(&lowered, "f").is_defined());
    }

    /// A name that is not a local is reported where it is used.
    ///
    /// Every place the IR can name starts at a local, so an object at file
    /// scope has nothing to be until #74.
    ///
    /// Mutation: give a use with no local the return place instead. Nothing is
    /// reported, `f` gains a body that writes to the wrong place, and this
    /// fails.
    #[test]
    fn a_name_the_ir_cannot_reach_is_reported() {
        let lowered = lowered("int g;\n\nint f(void) {\n    return g;\n}\n");
        assert_eq!(codes(&lowered), ["SC0304"]);
        assert!(!function(&lowered, "f").is_defined());
    }

    /// One function's refusal is not another's.
    ///
    /// Mutation: stop lowering at the first function that fails. `good` loses
    /// its body and this fails.
    #[test]
    fn a_function_that_cannot_be_lowered_does_not_take_the_others_with_it() {
        let lowered = lowered(
            "int bad(void) {\n    int a[3];\n    return 0;\n}\n\nint good(void) {\n    return 1;\n}\n",
        );
        assert_eq!(codes(&lowered), ["SC0304"]);
        assert!(!function(&lowered, "bad").is_defined());
        assert!(function(&lowered, "good").is_defined());
    }

    /// What a statement after a `return` becomes.
    ///
    /// C allows it and the parser reads it, so the lowering has to put it
    /// somewhere: it opens a block nothing jumps to, which is what unreachable
    /// code is.
    ///
    /// Mutation: keep the block open after a terminator. The statement after
    /// the `return` is written into a block that was already filled, which is
    /// `fill_block`'s panic.
    #[test]
    fn a_statement_after_a_return_is_a_block_nothing_reaches() {
        let lowered = lowered("int f(int n) {\n    return n;\n    n = 1;\n}\n");
        assert_eq!(codes(&lowered), Vec::<String>::new());

        let f = function(&lowered, "f");
        let edges = edges(f);
        assert_eq!(edges.len(), 2);
        assert!(edges.iter().all(|successors| successors.is_empty()));
        assert!(
            !edges
                .iter()
                .skip(1)
                .any(|successors| successors.contains(&1))
        );
    }

    /// Every operation says where in the source it came from.
    ///
    /// Mutation: give an operation `Origin::Generated` instead. The assertion
    /// that the source wrote it fails, and with it the promise that a
    /// diagnostic about this operation can quote what the user typed.
    #[test]
    fn every_operation_says_the_source_wrote_it() {
        let lowered = lowered("int f(int n) {\n    n = n + 1;\n    return n;\n}\n");
        let f = function(&lowered, "f");

        for block in f.blocks() {
            for operation in &block.operations {
                assert!(
                    matches!(operation.origin, Origin::Written(_)),
                    "{:?}",
                    operation.origin
                );
                assert!(!lowered.sources.snippet(operation.origin.span()).is_empty());
            }
        }
    }
}
