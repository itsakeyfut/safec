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
//! Everything it refuses is `SC0304`: an expression the frontend could not
//! type, a type the IR cannot hold, a name it cannot reach, a constant it
//! cannot read, a call with no function to name, and a second definition of
//! one name. One code rather than six, for the reason `types.rs` gives for
//! `MISMATCH`: what differs between them is the message, and a reader
//! filtering on the code wants to know the IR could not be built rather than a
//! list of ways that can happen.
//!
//! `parser.rs` splits its two codes along a different line, and the difference
//! is worth naming: `TOO_DEEP` is separate from `EXPECTED` because a program
//! nested too deeply is well formed and refused, while an unexpected token is
//! a program nobody wrote correctly. Every case here is the first kind, so the
//! split has nothing to divide.

use std::collections::{HashMap, HashSet};

use crate::ast::{Ast, BinOp as AstBinOp, Expr, ExprId, Item, Parameters, Stmt, StmtId, Type};
use crate::ast::{TypeId, UnOp as AstUnOp, spell_type};
use crate::diagnostics::{Code, Diagnostic, DiagnosticSink, Label};
use crate::sema::Resolution;
use crate::types::Types;
use safec_ir::ir::{
    BinOp, Block, BlockId, Element, FuncId, Function, LocalId, Operand, Operation, Origin, Place,
    Projection, Rvalue, Terminator, TranslationUnit, Ty, TyId, UnOp,
};
use safec_ir::source::{SourceMap, Span};
use safec_ir::target::Target;

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
    target: Target,
    diagnostics: &mut DiagnosticSink,
) -> TranslationUnit {
    let mut lowering = Lowering {
        sources,
        ast,
        resolution,
        types,
        // The one thing this stage learns about the machine, and it only
        // passes it on: what a type is worth is asked of the unit, below this.
        unit: TranslationUnit::new(target),
        locals: HashMap::new(),
        scopes: Vec::new(),
        functions: HashMap::new(),
        refused: HashSet::new(),
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
    /// The compound statements that are open, innermost last, each holding the
    /// locals it declared directly.
    ///
    /// The function's body is the first, so a scope narrower than the function
    /// is one at index 1 or beyond. Only those get storage markers: a local
    /// declared in the body itself lives exactly as long as the frame, which
    /// every consumer already knows, and ADR-0012 argues why that is enough.
    scopes: Vec<Vec<LocalId>>,
    /// Which function a name at file scope became, keyed by the name itself.
    ///
    /// Not by span, the way a local is: a prototype and the definition that
    /// follows it are two declarations of one function, at two spans, and a
    /// call resolves to whichever the resolver had in scope. Keyed by span,
    /// `int add(int, int);` and the `add` below it become two functions, and a
    /// call written between them reaches the one with no body. Keyed by the
    /// text, they are one, which is what C means by them and what
    /// `sema.rs::lookup` already compares.
    functions: HashMap<String, FuncId>,
    /// The names whose signature this stage could not read.
    ///
    /// Reported once, where the declaration is. A call to one of them is not
    /// reported again: the caller wrote an ordinary call and the fault is in a
    /// declaration somewhere else.
    refused: HashSet<String>,
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
    elements: Vec<Element>,
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
        self.element(Element::Assign(operation));
    }

    /// Write any element into the current block.
    fn element(&mut self, element: Element) {
        self.open();
        self.elements.push(element);
    }

    /// End the current block, and leave none open.
    fn end(&mut self, terminator: Terminator) {
        let block = self.open();
        let elements = std::mem::take(&mut self.elements);
        self.function.fill_block(
            block,
            Block {
                elements,
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
/// every left-associative operator is read, adds a level per operator. RK-008
/// in the review knowledge bank is the entry, and `driver.rs`'s `dump_expr` is
/// the walker that paid for it first.
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
                // A declaration of an object is the ordinary case here and is
                // answered where it is used. A *definition* of one is not: C17
                // 6.9.1 p2 requires the identifier in a function definition to
                // have a function type, `int (*f)(int) { ... }` does not, and
                // nothing before this stage checks it. Dropping it in silence
                // would leave a translation unit missing a function that the
                // file plainly contains, and the artifact saying `declared`
                // about a body it can see.
                if matches!(item, Item::Function(_)) {
                    diagnostics.report(
                        Diagnostic::error("this defines something that is not a function")
                            .with_code(LOWERING)
                            .with_label(Label::primary(
                                name,
                                format!("this declares `{}`", spell_type(self.sources, self.ast, ty)),
                            ))
                            .with_note(
                                "C17 6.9.1 p2 requires the identifier in a function definition to have a function type",
                            ),
                    );
                    self.refused.insert(self.sources.snippet(name).to_owned());
                }
                continue;
            };
            let (returns, parameters) = (*returns, parameters.clone());
            let Some((returns, lowered)) = self.signature(name, returns, &parameters, diagnostics)
            else {
                // The signature was reported and there is no honest function to
                // put here: inventing one would tell a caller a return type
                // this compiler could not read. What is remembered instead is
                // the name, so that a call to it says nothing more. The user
                // has been told once, about the declaration, and a second
                // diagnostic pointing at an ordinary call would be blaming code
                // that is fine.
                self.refused.insert(self.sources.snippet(name).to_owned());
                continue;
            };

            // A name declared twice is one function. The first declaration is
            // the one whose span the IR carries, which is where a reader of a
            // diagnostic about the callee is pointed.
            if !self.functions.contains_key(self.sources.snippet(name)) {
                let id = self
                    .unit
                    .push_function(Function::declaration(name, returns, lowered));
                self.functions
                    .insert(self.sources.snippet(name).to_owned(), id);
            }
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

            let Some(id) = self.functions.get(self.sources.snippet(name)).copied() else {
                continue;
            };
            // C17 6.9 p5 allows one external definition of a name and this
            // compiler does not check it yet, so a second one arrives here
            // rather than being reported before it. The first body is kept,
            // because replacing it would leave every call that was lowered
            // against it pointing at another function's blocks.
            if self.unit.function(id).is_defined() {
                diagnostics.report(
                    Diagnostic::error(format!(
                        "`{}` is defined more than once",
                        self.sources.snippet(name)
                    ))
                    .with_code(LOWERING)
                    .with_label(Label::primary(name, "this definition is not used"))
                    .with_label(Label::secondary(
                        self.unit.function(id).name,
                        "the first one is here",
                    )),
                );
                continue;
            }
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
            elements: Vec::new(),
        };
        builder.open();
        self.stmt(&mut builder, body, diagnostics)?;

        // Falling off the end returns whatever the return place holds. C17
        // 6.9.1 p12 makes reading that undefined, and 5.1.2.2.3 p1 makes
        // `main` the exception by returning zero, which is not written here
        // because nothing in the IR says which function is the entry point.
        // Whoever gives it one writes that zero.
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
                // memory analysis that one object is another. Nothing has
                // asked the IR to hold either an array or a function yet, and
                // #74, which is the issue for what it cannot say, is about
                // storage duration rather than about these.
                Type::Array { .. } | Type::Function { .. } => {
                    diagnostics.report(
                        Diagnostic::error(format!(
                            "cannot compile something of type `{}` yet",
                            spell_type(self.sources, self.ast, id)
                        ))
                        .with_code(LOWERING)
                        .with_label(Label::primary(at, "declared here"))
                        .with_note("`int`, `char`, `void` and pointers to them are all this compiler holds so far"),
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
                Diagnostic::error("cannot compile an expression whose type is not known")
                    .with_code(LOWERING)
                    .with_label(Label::primary(
                        span,
                        "nothing worked out what type this has",
                    ))
                    .with_note("this is a gap in this compiler rather than a fault in the program"),
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
            Stmt::Compound { body, span } => {
                let (body, span) = (body.clone(), *span);
                self.scopes.push(Vec::new());

                // Not `?`: the scope has to be closed whether or not a
                // statement inside it could be lowered, or the next compound
                // at this depth would inherit an entry that is still open.
                let mut lowered = Some(());
                for statement in body {
                    if self.stmt(builder, statement, diagnostics).is_none() {
                        lowered = None;
                        break;
                    }
                }

                // The function's own body collects nothing, because the
                // `Declaration` arm only records a local when a scope narrower
                // than the body is open, so there is nothing to close for it
                // and no need to ask whether this is that one.
                let declared = self.scopes.pop().expect("the scope this arm pushed");
                if builder.reachable() {
                    // Reverse order of declaration, which is the order a C++
                    // destructor would run in and costs nothing to get right
                    // while the list is being written.
                    for &local in declared.iter().rev() {
                        builder.element(Element::StorageDead {
                            // Nobody wrote "end this storage": the `}` is what
                            // it exists because of, which is what `Generated`
                            // means and what stops a diagnostic quoting a
                            // block of source back as if a user had asked for
                            // this. The last byte of a compound statement is
                            // its `}`, and the lowering only ever sees a tree
                            // that parsed without a word said about it.
                            origin: Origin::Generated(Span::new(
                                span.file(),
                                span.end() - 1,
                                span.end(),
                            )),
                            local,
                        });
                    }
                }
                lowered?;
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

                    // Only a scope narrower than the function's own body. The
                    // first entry is the body, and ADR-0012 says why a local
                    // that lives as long as the frame needs no marker.
                    if self.scopes.len() > 1 {
                        let scope = self.scopes.last_mut().expect("a scope is open");
                        scope.push(local);
                        // Generated for the same reason as the closing half:
                        // the declaration is what this exists because of, and
                        // is not itself an instruction to begin storage that
                        // somebody wrote.
                        builder.element(Element::StorageLive {
                            local,
                            origin: Origin::Generated(span),
                        });
                    }
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
                Task::FinishPlace(id) => {
                    self.finish_place(builder, id, &mut values, &mut places, diagnostics)?;
                }
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
            Expr::Number { span } => {
                let span = *span;
                values.push(Operand::Constant(self.constant(span, diagnostics)?));
            }
            Expr::Identifier { .. } | Expr::Subscript { .. } => {
                tasks.push(Task::Finish(id));
                tasks.push(Task::Place(id));
            }
            Expr::Unary { op, operand, .. } => {
                tasks.push(Task::Finish(id));
                match op {
                    // C17 6.5.3.2 p4 makes the operand of `*` a value and the
                    // result an lvalue, so `*(p + i)` and `*p++` are ordinary
                    // C. Asking for the operand's place instead would refuse
                    // both: what has a place here is `*p`, not `p + i`.
                    AstUnOp::Deref => tasks.push(Task::Place(id)),
                    // These read a place, and `&x` never reads `x` at all.
                    AstUnOp::AddrOf
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
            // Everything else has a value and no place. An assignment, `&`,
            // `++` and `--` each ask for one, so the message says that rather
            // than naming assignment: `----n` asks through the innermost `--`
            // and nothing in it is being assigned to. C17 6.5.16 p2 wants a
            // modifiable lvalue on the left of an assignment, and whether a
            // program breaks that constraint is the type checker's to say; what
            // is said here is only that there is nothing to read or write.
            //
            // The kinds are written out rather than wildcarded, so an
            // expression kind added later is `error[E0004]` here and has to say
            // whether it names a place.
            Expr::Number { .. }
            | Expr::Unary { .. }
            | Expr::Binary { .. }
            | Expr::Assign { .. }
            | Expr::Conditional { .. }
            | Expr::Call { .. }
            | Expr::Comma { .. }
            | Expr::Error { .. } => {
                diagnostics.report(
                    Diagnostic::error("this expression names no place")
                        .with_code(LOWERING)
                        .with_label(Label::primary(
                            self.ast.expr(id).span(),
                            "this is a value, not somewhere a value can live",
                        ))
                        .with_note(
                            "an assignment, `&`, `++` and `--` each need a place to work on",
                        ),
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
                    // `begin_place` put the `Deref` on, because `*p` is the
                    // place rather than `p` being one.
                    AstUnOp::Deref => {
                        let place = places.pop().expect("a place");
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

                        // 6.5.3.1 p2 and 6.5.2.4 p2 make `++E` and `E++` mean
                        // `E += 1`, which 6.5.16.2 p3 makes `E = E + 1`. So the
                        // step happens at the promoted type, the same as a
                        // compound assignment above, and for the same reason.
                        let stepped = self.promoted(builder);
                        builder.push(Operation {
                            place: Place::local(stepped),
                            value: Rvalue::Binary {
                                op: step,
                                lhs: Operand::Copy(place.clone()),
                                rhs: Operand::Constant(1),
                            },
                            origin: Origin::Written(span),
                        });
                        builder.push(Operation {
                            place: place.clone(),
                            value: Rvalue::Use(Operand::Copy(Place::local(stepped))),
                            origin: Origin::Written(span),
                        });
                        // A prefix operator answers what the place holds after
                        // the write, and that is taken here for the reason the
                        // assignment above gives: what a call does to the same
                        // place afterwards must not change this answer. 6.5.3.1
                        // p2 is the clause.
                        let answer = match kept {
                            Some(kept) => kept,
                            None => {
                                let held = self.temporary(builder, id, diagnostics)?;
                                builder.push(Operation {
                                    place: Place::local(held),
                                    value: Rvalue::Use(Operand::Copy(place)),
                                    origin: Origin::Written(span),
                                });
                                held
                            }
                        };
                        values.push(Operand::Copy(Place::local(answer)));
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
                    // 6.5.16.2 p3: `a += b` is `a = a + b` but for evaluating
                    // `a` once. Spelled that way here too, through a temporary
                    // of the promoted type, so that the two spellings are one
                    // IR and the addition is checked at the width C performs it
                    // at rather than at the width it is stored into.
                    Some(op) => {
                        let computed = self.promoted(builder);
                        builder.push(Operation {
                            place: Place::local(computed),
                            value: Rvalue::Binary {
                                op: binary(op)
                                    .expect("a compound assignment is never `&&` or `||`"),
                                lhs: Operand::Copy(place.clone()),
                                rhs: value,
                            },
                            origin: Origin::Written(span),
                        });
                        Rvalue::Use(Operand::Copy(Place::local(computed)))
                    }
                    None => Rvalue::Use(value),
                };
                builder.push(Operation {
                    place: place.clone(),
                    value,
                    origin: Origin::Written(span),
                });

                // 6.5.16 p3: the value is what the left operand holds after
                // the assignment, and that is fixed here rather than read back
                // later. `(b = 1) + g(&b)` is the case: 6.5.2.2 p10 makes the
                // callee's execution indeterminately sequenced with the rest
                // of the expression, so `g` may write `b` before the addition
                // happens, and a copy taken now is one and not seven.
                let held = self.temporary(builder, id, diagnostics)?;
                builder.push(Operation {
                    place: Place::local(held),
                    value: Rvalue::Use(Operand::Copy(place)),
                    origin: Origin::Written(span),
                });
                values.push(Operand::Copy(Place::local(held)));
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
            // A conditional is answered by `merge` and never asks to finish,
            // and an `Error` is refused before it can. Both arms are here
            // because the match is written out rather than wildcarded.
            Expr::Conditional { .. } | Expr::Error { .. } => {}
        }

        Some(())
    }

    /// Build a node's place from what its children left.
    fn finish_place(
        &mut self,
        builder: &mut Builder,
        id: ExprId,
        values: &mut Vec<Operand>,
        places: &mut Vec<Place>,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<()> {
        match self.ast.expr(id) {
            Expr::Unary { .. } => {
                let operand = values.pop().expect("a pointer");
                let mut place = self.pointed_at(id, operand, diagnostics)?;
                place.projection.push(Projection::Deref);
                places.push(place);
            }
            // C17 6.5.2.1 p2 defines `E1[E2]` as `(*((E1)+(E2)))`, and this
            // builds exactly that: one C expression is one shape in the IR, so
            // an analysis asking what an access reaches has one thing to read
            // rather than two spellings of it.
            //
            // `Projection::Index` is what an array wants and nothing builds one
            // yet, because an array is a type this stage refuses. Whoever gives
            // the IR arrays decides whether a subscript on one is an `Index`.
            Expr::Subscript { base, span, .. } => {
                let (base, span) = (*base, *span);
                let offset = values.pop().expect("an index");
                let pointer = values.pop().expect("a base");
                // Of the base's type rather than the subscript's: what is
                // worked out here is the address, and the subscript is what
                // that address reaches.
                let addressed = self.temporary(builder, base, diagnostics)?;

                builder.push(Operation {
                    place: Place::local(addressed),
                    value: Rvalue::Binary {
                        op: BinOp::Add,
                        lhs: pointer,
                        rhs: offset,
                    },
                    origin: Origin::Written(span),
                });
                places.push(Place {
                    local: addressed,
                    projection: vec![Projection::Deref],
                });
            }
            // Nothing else schedules a `FinishPlace`, and the kinds are written
            // out rather than wildcarded because the cost of being wrong is
            // paid elsewhere: an arm that pushed no place would leave the next
            // `places.pop()` taking an outer expression's place instead.
            Expr::Number { .. }
            | Expr::Identifier { .. }
            | Expr::Binary { .. }
            | Expr::Assign { .. }
            | Expr::Conditional { .. }
            | Expr::Call { .. }
            | Expr::Comma { .. }
            | Expr::Error { .. } => {}
        }

        Some(())
    }

    /// The place an operand names, or a report that it names none.
    ///
    /// Everything this stage builds a value from is either a constant or a
    /// copy of a place, so a projection onto a constant is the only way to get
    /// here: `*0` is that shape. The frontend refuses it today, because
    /// `types.rs` gives an indirection through a non-pointer no type at all,
    /// and reporting rather than returning is what keeps that from being an
    /// invariant somebody has to remember: the two stacks stay in step, and a
    /// change upstream cannot turn this into a panic.
    fn pointed_at(
        &mut self,
        id: ExprId,
        operand: Operand,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<Place> {
        match operand {
            Operand::Copy(place) => Some(place),
            Operand::Constant(_) => {
                diagnostics.report(
                    Diagnostic::error("a constant does not point at anything")
                        .with_code(LOWERING)
                        .with_label(Label::primary(
                            self.ast.expr(id).span(),
                            "this has nothing to reach",
                        )),
                );
                None
            }
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
                // The left operand decides the answer unless the right is
                // reached, so it is written before the branch and overwritten
                // after. What is written is whether it is non-zero and not
                // what it is: C17 6.5.13 p3 and 6.5.14 p3 say `&&` and `||`
                // yield 1 or 0, so `3 && 5` is 1 rather than 5.
                builder.push(Operation {
                    place: Place::local(answer),
                    value: truth(condition),
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
            // Only a `&&`, a `||` and a `?:` split, and the kinds are written
            // out because an arm that fell through here would leave `join`
            // reserved and never filled, which is a panic at whatever later
            // moment somebody walks the graph.
            Expr::Number { .. }
            | Expr::Identifier { .. }
            | Expr::Unary { .. }
            | Expr::Assign { .. }
            | Expr::Call { .. }
            | Expr::Subscript { .. }
            | Expr::Comma { .. }
            | Expr::Error { .. } => {}
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
        // A `&&` or `||` answers 1 or 0 and a `?:` answers what its arm is
        // worth. C17 6.5.13 p3 and 6.5.14 p3 say the first, and 6.5.15 p4 the
        // second: the conditional operator's value is the operand's, converted,
        // rather than a truth value.
        let value = match self.ast.expr(id) {
            Expr::Binary { .. } => truth(value),
            _ => Rvalue::Use(value),
        };
        builder.push(Operation {
            place: Place::local(answer),
            value,
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

    /// A temporary of the type an arithmetic operation is performed at.
    ///
    /// `int`, always, because C17 6.3.1.1 p2 promotes every integer type this
    /// frontend has to it and 6.5.16.2 p3 makes `E1 op= E2` mean `E1 = E1 op
    /// E2`, which puts the operation at the promoted type and the narrowing in
    /// the assignment. `types.rs` says the same about `a + b` and is where this
    /// stops being a constant: the day `long` parses, the promoted type of a
    /// pair is a question again.
    ///
    /// Without this, `c += 100` on a `char` writes its addition straight into
    /// an 8-bit place, and the interpreter reads that place's type as the width
    /// the operation happened at. C says 200 is an ordinary `int` there and the
    /// truncation to `char` is a conversion, so a run that stopped would be
    /// reporting a defined program as undefined. ADR-0013 rests on an
    /// operation's destination carrying the promoted type; this is what makes
    /// that true where no expression node does.
    fn promoted(&mut self, builder: &mut Builder) -> LocalId {
        let int = self.unit.push_type(Ty::Int);
        builder.function.push_local(int)
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
                Diagnostic::error("cannot compile a use of this name yet")
                    .with_code(LOWERING)
                    .with_label(Label::primary(span, "this is not a local or a parameter"))
                    .with_note("an object declared outside a function is not supported so far"),
            );
        }

        local
    }

    /// The function a call names.
    ///
    /// Nothing reaches the report below today, and the path that would is worth
    /// naming: a callee this stage cannot resolve is a pointer to a function,
    /// and a pointer to a function is a type [`Lowering::ty`] refuses where it
    /// is declared, so the function holding the call is already refused by
    /// then. The report is here because the day `Ty` grows a function type is
    /// the day this becomes reachable, and a `None` returned in silence would
    /// be a function dropped with nothing said.
    fn callee(&mut self, callee: ExprId, diagnostics: &mut DiagnosticSink) -> Option<FuncId> {
        let span = self.ast.expr(callee).span();
        let named = self
            .resolution
            .resolved(callee)
            .map(|binding| self.resolution.binding(binding).name)
            .and_then(|name| self.functions.get(self.sources.snippet(name)).copied());

        if named.is_none() && !self.refused.contains(self.sources.snippet(span)) {
            diagnostics.report(
                Diagnostic::error("cannot compile this call yet")
                    .with_code(LOWERING)
                    .with_label(Label::primary(
                        span,
                        "this is not a function this compiler found",
                    ))
                    .with_note("a call through a function pointer is not supported so far"),
            );
        }

        named
    }

    /// What an integer constant is worth.
    ///
    /// A decimal constant with no suffix, and a report for everything else.
    /// The lexer takes a number to be a digit followed by whatever looks like
    /// it belongs to one, so `0x10`, `1u`, `1.5` and a value too large for an
    /// `i128` all arrive here as text, and reading them as decimals gives four
    /// wrong answers with nothing said. `010` is the one that hides: C17
    /// 6.4.4.1 p2 makes a leading `0` an octal constant, so it is eight, and a
    /// decimal reading makes it ten. A plausible wrong number is worse than a
    /// refusal, and worse again than a number nobody can produce.
    ///
    /// Working the value and the type out properly belongs to the frontend,
    /// where 6.4.4.1's table decides which type a constant has. #76 is that
    /// work; until it lands this stage reads what it can read and says so about
    /// the rest.
    fn constant(&mut self, span: Span, diagnostics: &mut DiagnosticSink) -> Option<i128> {
        let text = self.sources.snippet(span);
        // 6.4.4.1 p1: a decimal constant is a nonzero digit and more digits.
        // `0` on its own is an octal constant and is zero read either way.
        let decimal = text == "0"
            || (text.starts_with(|c: char| c.is_ascii_digit() && c != '0')
                && text.bytes().all(|byte| byte.is_ascii_digit()));

        if let Some(value) = text.parse::<i128>().ok().filter(|_| decimal) {
            return Some(value);
        }

        diagnostics.report(
            Diagnostic::error("cannot compile this constant yet")
                .with_code(LOWERING)
                .with_label(Label::primary(span, "this is not a plain decimal constant"))
                .with_note(
                    "a leading zero makes a constant octal, so `010` read as a decimal would be \
                     ten where C says eight, and a hexadecimal spelling, a suffix, a floating \
                     constant or a value too large to hold would not be read as a number at all",
                ),
        );
        None
    }
}

/// Whether an operand is non-zero, as a value.
///
/// C's truth values are 1 and 0 rather than whatever decided them: 6.5.13 p3
/// and 6.5.14 p3 say `&&` and `||` yield one or the other, so an answer copied
/// from the operand that decided it would make `3 && 5` five. The IR has no
/// truth of its own, so the comparison is the operation that says it.
fn truth(operand: Operand) -> Rvalue {
    Rvalue::Binary {
        op: BinOp::Ne,
        lhs: operand,
        rhs: Operand::Constant(0),
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

    /// The assignments in a block, in order, with the storage markers dropped.
    ///
    /// Most of these tests are about what a program computes, and a marker is
    /// not that. The ones that are about markers read `block.elements`
    /// directly, which is the only way to see both.
    fn assigns(block: &Block) -> Vec<&Operation> {
        block
            .elements
            .iter()
            .filter_map(|element| match element {
                Element::Assign(operation) => Some(operation),
                Element::StorageLive { .. } | Element::StorageDead { .. } => None,
            })
            .collect()
    }

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

        // A target this test suite does not otherwise care about: what a
        // program lowers to does not turn on the machine, and the one test
        // that is about the machine names its own.
        let target = Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple");
        let unit = lower(
            &sources,
            &ast,
            &resolution,
            &types,
            target,
            &mut diagnostics,
        );
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
        let id = lowered
            .unit
            .functions()
            .find(|id| lowered.sources.snippet(lowered.unit.function(*id).name) == name)
            .expect("a function of that name");
        lowered.unit.function(id)
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

    /// Every storage marker in a function, as `(kind, local)` in order.
    fn markers(function: &Function) -> Vec<(&'static str, usize)> {
        function
            .blocks()
            .flat_map(|block| block.elements.iter())
            .filter_map(|element| match element {
                Element::Assign(_) => None,
                Element::StorageLive { local, origin: _ }
                | Element::StorageDead { local, origin: _ } => {
                    Some((element.name(), local.index()))
                }
            })
            .collect()
    }

    /// Two programs that differ in one pair of braces are two IRs.
    ///
    /// This is the whole point of the markers. Before them the two below were
    /// byte for byte the same artifact, so an analysis handed either had the
    /// same material and had to answer both the same way: silent about the
    /// dangling one, or wrong about the other. ADR-0012 has the argument.
    ///
    /// Mutation: delete the `StorageDead` loop from the `Compound` arm. The two
    /// differ only in `StorageLive` then, and the assertion that the flat
    /// program has no markers at all still holds, so this fails on the first
    /// comparison. Mutation: delete both loops, and the two are equal again.
    #[test]
    fn the_same_program_in_a_nested_scope_is_not_the_same_ir() {
        let nested = lowered("int g(void) { int *p; { int x; x = 42; p = &x; } return *p; }\n");
        let flat = lowered("int g(void) { int *p; int x; x = 42; p = &x; return *p; }\n");

        let (nested, flat) = (function(&nested, "g"), function(&flat, "g"));

        // The same locals in the same order, so the difference is not that one
        // program declares something the other does not.
        assert_eq!(nested.locals().len(), flat.locals().len());

        let marked = markers(nested);
        let [(opens, live), (closes, dead)] = marked[..] else {
            panic!("one pair, and nothing else: {marked:?}");
        };
        assert_eq!((opens, closes), ("StorageLive", "StorageDead"));
        assert_eq!(live, dead, "one local, opened and closed");

        assert_eq!(markers(flat), [], "the flat program has nothing to say");
    }

    /// A local the function itself declares gets no marker.
    ///
    /// Its storage is the frame's, which every consumer already models: the
    /// interpreter pops the frame and a pointer into a returned one is caught
    /// by its generation. ADR-0012 argues why that is enough, and the cost of
    /// marking them anyway would be a pair per local in every artifact.
    ///
    /// Mutation: drop the `self.scopes.len() > 1` test in the `Declaration`
    /// arm. Every local gets a pair and this fails.
    #[test]
    fn a_local_the_function_declares_has_no_marker() {
        let lowered = lowered("int f(int n) { int a; int b; a = n; b = a; return b; }\n");

        assert_eq!(markers(function(&lowered, "f")), []);
    }

    /// A scope that a `return` leaves ends no storage.
    ///
    /// The frame is going, so there is nothing for a marker to say, and an
    /// element written after a block has ended would be written into the next
    /// block instead.
    ///
    /// Mutation: emit the `StorageDead` loop whether or not `builder.reachable`
    /// says the end is reachable. A `StorageDead` appears and this fails.
    #[test]
    fn a_scope_a_return_leaves_ends_no_storage() {
        let lowered = lowered("int f(int n) { if (n) { int x; x = 1; return x; } return 0; }\n");

        let marked = markers(function(&lowered, "f"));
        let [(opens, _)] = marked[..] else {
            panic!("the scope opens and nothing closes it: {marked:?}");
        };
        assert_eq!(opens, "StorageLive");
    }

    /// A local in a loop body gets its storage back on each iteration.
    ///
    /// C17 6.2.4 p6: an automatic object's lifetime "extends from entry into
    /// the block with which it is associated until execution of that block ends
    /// in any way". Each iteration enters and ends the body, so each iteration
    /// is a fresh lifetime. Without the `StorageLive` the second iteration
    /// would be writing into storage the IR says is gone.
    ///
    /// Mutation: emit `StorageDead` and no `StorageLive`. This fails, and the
    /// interpreter stops on the second iteration of any loop that declares
    /// anything.
    #[test]
    fn a_local_in_a_loop_body_gets_its_storage_back() {
        let lowered = lowered(
            "int f(void) {\n    int n; int s;\n    n = 0; s = 0;\n    while (n < 3) { int x; x = n; s = s + x; n = n + 1; }\n    return s;\n}\n",
        );
        let f = function(&lowered, "f");

        let marked = markers(f);
        let [(opens, live), (closes, dead)] = marked[..] else {
            panic!("one pair, and nothing else: {marked:?}");
        };
        assert_eq!((opens, closes), ("StorageLive", "StorageDead"));
        assert_eq!(live, dead, "one local, opened and closed");

        // Both are in the body rather than around the loop, which is what makes
        // the second iteration a fresh lifetime rather than a read of storage
        // the first one ended.
        let body: Vec<usize> = f
            .blocks()
            .enumerate()
            .filter(|(_, block)| {
                block
                    .elements
                    .iter()
                    .any(|element| !matches!(element, Element::Assign(_)))
            })
            .map(|(index, _)| index)
            .collect();
        assert_eq!(body.len(), 1, "both markers are in one block");
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
        let [sum, returned] = assigns(entry)[..] else {
            panic!("{:?}", entry.elements);
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
        assert!(assigns(entry).is_empty());
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
        let [returned] = assigns(after)[..] else {
            panic!("{:?}", after.elements);
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
    /// The comparison moves into the block before the loop, the condition is
    /// asked once however many turns the loop takes, and this fails.
    #[test]
    fn a_while_loop_comes_back_to_its_condition() {
        // `n > 0` rather than `n`, because the comparison is an operation and
        // which block holds it is the whole point: a condition evaluated once,
        // before the loop, is a different program that the shape of the edges
        // alone cannot tell apart.
        let lowered = lowered(
            "int f(int n) {\n    while (n > 0) {\n        n = n - 1;\n    }\n    return n;\n}\n",
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

        // The condition is asked in the header, which is where the back edge
        // arrives, so the comparison is written there and not before the loop.
        assert!(assigns(blocks[0]).is_empty());
        let [compared] = assigns(blocks[1])[..] else {
            panic!("{:?}", blocks[1].elements);
        };
        assert!(matches!(
            compared.value,
            Rvalue::Binary { op: BinOp::Gt, .. }
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
    ///
    /// Mutation: write the operand itself into the answer rather than whether
    /// it is non-zero. `3 && 5` becomes five where C17 6.5.13 p3 says one, and
    /// both assertions on the comparison fail.
    #[test]
    fn a_short_circuit_is_a_branch_and_answers_one_or_zero() {
        let lowered = lowered("int f(int a, int b) {\n    return a && b;\n}\n");
        assert_eq!(codes(&lowered), Vec::<String>::new());

        let f = function(&lowered, "f");
        // The entry branches on the left operand; one arm evaluates the right
        // and both arrive at the join.
        assert_eq!(edges(f), vec![vec![2, 1], vec![], vec![1]]);

        let blocks: Vec<_> = f.blocks().collect();
        let [a, b] = f.parameters().collect::<Vec<_>>()[..] else {
            panic!("two parameters");
        };

        // Each arm writes whether its operand is non-zero, because 6.5.13 p3
        // makes the answer 1 or 0 rather than whatever decided it.
        let [decided] = assigns(blocks[0])[..] else {
            panic!("{:?}", blocks[0].elements);
        };
        assert_eq!(
            decided.value,
            Rvalue::Binary {
                op: BinOp::Ne,
                lhs: Operand::Copy(Place::local(a)),
                rhs: Operand::Constant(0),
            }
        );

        let [answered] = assigns(blocks[2])[..] else {
            panic!("{:?}", blocks[2].elements);
        };
        assert_eq!(answered.place, decided.place);
        assert_eq!(
            answered.value,
            Rvalue::Binary {
                op: BinOp::Ne,
                lhs: Operand::Copy(Place::local(b)),
                rhs: Operand::Constant(0),
            }
        );
    }

    /// A conditional operator answers what its arm is worth, and not a truth
    /// value.
    ///
    /// C17 6.5.15 p4: the value is the operand's, which is what separates it
    /// from `&&` and `||` a few lines above in this module.
    ///
    /// Mutation: normalise a `?:` arm the way a short circuit is normalised.
    /// `a ? b : c` starts answering 1 or 0 and this fails.
    #[test]
    fn a_conditional_answers_the_arm_it_took() {
        let lowered = lowered("int f(int a, int b, int c) {\n    return a ? b : c;\n}\n");
        assert_eq!(codes(&lowered), Vec::<String>::new());

        let f = function(&lowered, "f");
        let [_, b, c] = f.parameters().collect::<Vec<_>>()[..] else {
            panic!("three parameters");
        };
        let blocks: Vec<_> = f.blocks().collect();

        // Block 1 is the join, and the arms are the two blocks the branch
        // names, in the order they were reserved.
        assert_eq!(edges(f), vec![vec![2, 3], vec![], vec![1], vec![1]]);
        let [taken] = assigns(blocks[2])[..] else {
            panic!("{:?}", blocks[2].elements);
        };
        let [skipped] = assigns(blocks[3])[..] else {
            panic!("{:?}", blocks[3].elements);
        };
        assert_eq!(taken.value, Rvalue::Use(Operand::Copy(Place::local(b))));
        assert_eq!(skipped.value, Rvalue::Use(Operand::Copy(Place::local(c))));
        assert_eq!(taken.place, skipped.place);
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

    /// A constant is worth what C says it is worth, or it is not lowered.
    ///
    /// The lexer takes a number to be a digit and whatever follows that looks
    /// like part of one, so every spelling below reaches this stage as text.
    /// Read as decimals they give: sixteen as zero, one as zero, eight as ten,
    /// and a constant too large as zero. The third is the dangerous one,
    /// because ten is a number the program could have meant.
    ///
    /// Mutation: read every constant with `parse().unwrap_or(0)`. `010` lowers
    /// to ten with nothing reported and this fails.
    #[test]
    fn a_constant_this_stage_cannot_read_is_not_guessed_at() {
        for spelling in [
            "0x10",
            "1u",
            "010",
            "1.5",
            "9999999999999999999999999999999999999999",
        ] {
            let lowered = lowered(&format!("int f(void) {{\n    return {spelling};\n}}\n"));
            assert_eq!(codes(&lowered), ["SC0304"], "{spelling}");
            assert!(!function(&lowered, "f").is_defined(), "{spelling}");
        }

        // What it can read, it reads: a plain decimal, and the zero that is an
        // octal constant with the same value either way.
        for (spelling, value) in [("42", 42), ("0", 0)] {
            let lowered = lowered(&format!("int f(void) {{\n    return {spelling};\n}}\n"));
            assert_eq!(codes(&lowered), Vec::<String>::new(), "{spelling}");

            let f = function(&lowered, "f");
            let [block] = f.blocks().collect::<Vec<_>>()[..] else {
                panic!("one block");
            };
            let [returned] = assigns(block)[..] else {
                panic!("{:?}", block.elements);
            };
            assert_eq!(returned.value, Rvalue::Use(Operand::Constant(value)));
        }
    }

    /// A type the IR cannot hold is reported at the declaration that wrote it.
    ///
    /// The IR has `int`, `char`, `void` and pointers, and nothing has asked it
    /// for more. Lowering an array as a pointer would tell the memory analysis
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
    /// The program touches every place an operation is made: an assignment, a
    /// binary operator, a unary one, an address, an increment, a short circuit,
    /// a conditional, a call and a return. A shorter one would leave most of
    /// those unwatched, which is what this test did before somebody mutated the
    /// arm it was not watching and nothing failed.
    ///
    /// Mutation: give any one of them `Origin::Generated` instead. The
    /// assertion that the source wrote it fails, and with it the promise that a
    /// diagnostic about this operation can quote what the user typed.
    #[test]
    fn every_operation_says_the_source_wrote_it() {
        let lowered = lowered(
            "int g(int *q);\n\nint f(int n) {\n    int *p;\n    p = &n;\n    n = -n;\n    n++;\n    n = n && 1;\n    n = n ? 2 : 3;\n    n = n + g(p);\n    return n;\n}\n",
        );
        assert_eq!(codes(&lowered), Vec::<String>::new());

        let f = function(&lowered, "f");
        let mut seen = 0;
        for block in f.blocks() {
            for operation in assigns(block) {
                assert!(
                    matches!(operation.origin, Origin::Written(_)),
                    "{:?}",
                    operation.origin
                );
                assert!(!lowered.sources.snippet(operation.origin.span()).is_empty());
                seen += 1;
            }

            // A call is a terminator and carries the same field, so it answers
            // the same question the operations do.
            if let Terminator::Call { origin, .. } = &block.terminator {
                assert!(matches!(origin, Origin::Written(_)), "{origin:?}");
                seen += 1;
            }
        }

        assert!(seen > 15, "the program makes more than fifteen of them");
    }

    /// The one operation a function's only block ends up holding before it
    /// returns, for a program of the shape the operator tests below use.
    fn only_operation(lowered: &Lowered, name: &str) -> Operation {
        let function = function(lowered, name);
        let blocks: Vec<_> = function.blocks().collect();
        let [computed, _returned] = assigns(blocks[0])[..] else {
            panic!("{:?}", blocks[0].elements);
        };
        computed.clone()
    }

    /// Every binary operator becomes the one it means.
    ///
    /// The expected side is written out rather than taken from `binary`, for
    /// the reason RK-001 gives: a table built the way the code builds one
    /// compares the code with itself. `&&` and `||` are absent because they are
    /// branches, which the test above holds.
    ///
    /// Mutation: map any one of these to another variant, `Mul` to `Div` say.
    /// This fails, naming the operator that moved.
    #[test]
    fn every_binary_operator_lowers_to_the_one_it_means() {
        for (spelling, expected) in [
            ("*", BinOp::Mul),
            ("/", BinOp::Div),
            ("%", BinOp::Rem),
            ("+", BinOp::Add),
            ("-", BinOp::Sub),
            ("<<", BinOp::Shl),
            (">>", BinOp::Shr),
            ("<", BinOp::Lt),
            (">", BinOp::Gt),
            ("<=", BinOp::Le),
            (">=", BinOp::Ge),
            ("==", BinOp::Eq),
            ("!=", BinOp::Ne),
            ("&", BinOp::BitAnd),
            ("^", BinOp::BitXor),
            ("|", BinOp::BitOr),
        ] {
            let lowered = lowered(&format!(
                "int f(int a, int b) {{\n    return a {spelling} b;\n}}\n"
            ));
            assert_eq!(codes(&lowered), Vec::<String>::new(), "{spelling}");

            let f = function(&lowered, "f");
            let [a, b] = f.parameters().collect::<Vec<_>>()[..] else {
                panic!("two parameters");
            };
            assert_eq!(
                only_operation(&lowered, "f").value,
                Rvalue::Binary {
                    op: expected,
                    lhs: Operand::Copy(Place::local(a)),
                    rhs: Operand::Copy(Place::local(b)),
                },
                "{spelling}"
            );
        }
    }

    /// Every unary operator becomes the one it means, and `+` becomes nothing.
    ///
    /// C17 6.5.3.3 p2 makes unary `+` the value of its operand, so there is no
    /// operation for it to become.
    ///
    /// Mutation: map `-` to `UnOp::Not`, or give `+` an operation of its own.
    /// This fails on the operator that moved.
    #[test]
    fn every_unary_operator_lowers_to_the_one_it_means() {
        for (spelling, expected) in [
            ("-", Some(UnOp::Neg)),
            ("!", Some(UnOp::Not)),
            ("~", Some(UnOp::BitNot)),
            ("+", None),
        ] {
            let lowered = lowered(&format!("int f(int a) {{\n    return {spelling}a;\n}}\n"));
            assert_eq!(codes(&lowered), Vec::<String>::new(), "{spelling}");

            let f = function(&lowered, "f");
            let [a] = f.parameters().collect::<Vec<_>>()[..] else {
                panic!("one parameter");
            };
            let blocks: Vec<_> = f.blocks().collect();

            match expected {
                Some(op) => {
                    let [applied, _] = assigns(blocks[0])[..] else {
                        panic!("{:?}", blocks[0].elements);
                    };
                    assert_eq!(
                        applied.value,
                        Rvalue::Unary {
                            op,
                            operand: Operand::Copy(Place::local(a)),
                        },
                        "{spelling}"
                    );
                }
                None => {
                    let [returned] = assigns(blocks[0])[..] else {
                        panic!("{:?}", blocks[0].elements);
                    };
                    assert_eq!(
                        returned.value,
                        Rvalue::Use(Operand::Copy(Place::local(a))),
                        "{spelling}"
                    );
                }
            }
        }
    }

    /// What a pointer reaches is a place, and it is not the pointer's place.
    ///
    /// The memory axis of `docs/safety-model.md` turns on this: `p` and `*p`
    /// are two places, and an analysis told they are one would say a pointer is
    /// live when what it points at is not.
    ///
    /// Mutation: drop the `Deref` that `finish_place` pushes. Reading and
    /// writing both land on the pointer itself and this fails.
    #[test]
    fn a_pointer_is_read_and_written_through_a_deref() {
        let lowered = lowered("int f(int *p) {\n    *p = 1;\n    return *p;\n}\n");
        assert_eq!(codes(&lowered), Vec::<String>::new());

        let f = function(&lowered, "f");
        let [p] = f.parameters().collect::<Vec<_>>()[..] else {
            panic!("one parameter");
        };
        let pointee = Place {
            local: p,
            projection: vec![Projection::Deref],
        };
        let blocks: Vec<_> = f.blocks().collect();
        let [written, held, returned] = assigns(blocks[0])[..] else {
            panic!("{:?}", blocks[0].elements);
        };

        assert_eq!(written.place, pointee);
        assert_eq!(written.value, Rvalue::Use(Operand::Constant(1)));
        assert_eq!(held.value, Rvalue::Use(Operand::Copy(pointee.clone())));
        assert_eq!(returned.place, Place::local(f.return_place()));
        // Reading through the pointer reads what it points at, which is the
        // assertion a lowering that dropped the projection would fail.
        assert_eq!(returned.value, Rvalue::Use(Operand::Copy(pointee)));
        assert_ne!(written.place, Place::local(p));
    }

    /// A pointer's type is a pointer, however many times over.
    ///
    /// Mutation: have `ty` stop wrapping, so every pointer becomes what it
    /// points at. The three parameters below become one type and this fails.
    #[test]
    fn a_pointer_type_is_not_the_type_it_points_at() {
        let lowered = lowered("int f(int **pp, int *p, char c) {\n    return 0;\n}\n");
        assert_eq!(codes(&lowered), Vec::<String>::new());

        let f = function(&lowered, "f");
        let [pp, p, c] = f.parameters().collect::<Vec<_>>()[..] else {
            panic!("three parameters");
        };
        let int = f.local(f.return_place());

        assert_eq!(lowered.unit.ty(f.local(p)), Ty::Pointer(int));
        assert_eq!(lowered.unit.ty(f.local(pp)), Ty::Pointer(f.local(p)));
        assert_eq!(lowered.unit.ty(f.local(c)), Ty::Char);
        assert_ne!(f.local(c), int);
    }

    /// Taking an address is the one operation that turns a place into a value.
    ///
    /// Mutation: lower `&x` as a copy of `x`. The lifetime analysis loses the
    /// only shape it starts from, and this fails.
    #[test]
    fn an_address_is_taken_of_the_place_it_names() {
        let lowered = lowered("int f(int n) {\n    int *p;\n    p = &n;\n    return 0;\n}\n");
        assert_eq!(codes(&lowered), Vec::<String>::new());

        let f = function(&lowered, "f");
        let [n] = f.parameters().collect::<Vec<_>>()[..] else {
            panic!("one parameter");
        };
        let taken = f
            .blocks()
            .flat_map(|block| assigns(block).into_iter().cloned().collect::<Vec<_>>())
            .find(|operation| matches!(operation.value, Rvalue::Address(_)))
            .expect("an address is taken");

        assert_eq!(taken.value, Rvalue::Address(Place::local(n)));
    }

    /// A subscript and the arithmetic it is defined as are one shape.
    ///
    /// C17 6.5.2.1 p2 makes `E1[E2]` mean `(*((E1)+(E2)))`, so both spellings
    /// work the address out into a local and reach through it. An analysis
    /// asking what an access touches then has one thing to read rather than
    /// two spellings of it.
    ///
    /// Mutation: have `begin_value` ask for the operand's place under a `*`.
    /// `*(p + i)` stops lowering, `SC0304` is reported, and this fails.
    ///
    /// Mutation: swap the base and the index in `finish_place`. The address is
    /// worked out from the wrong operand and this fails.
    ///
    /// Mutation: give the subscript a `Projection::Index` on the base's place
    /// instead. The two spellings stop agreeing and this fails.
    #[test]
    fn an_element_is_reached_by_a_subscript_and_by_arithmetic() {
        let lowered = lowered(
            "int f(int *p, int i) {\n    p[i] = 1;\n    *(p + i) = 2;\n    return p[i];\n}\n",
        );
        assert_eq!(codes(&lowered), Vec::<String>::new());

        let f = function(&lowered, "f");
        let [p, i] = f.parameters().collect::<Vec<_>>()[..] else {
            panic!("two parameters");
        };
        let address = Rvalue::Binary {
            op: BinOp::Add,
            lhs: Operand::Copy(Place::local(p)),
            rhs: Operand::Copy(Place::local(i)),
        };
        let operations: Vec<_> = f
            .blocks()
            .flat_map(|block| assigns(block).into_iter().cloned().collect::<Vec<_>>())
            .collect();

        // Both spellings work the address out first and reach through it, and
        // neither leaves a projection rooted at the pointer itself.
        let addressed: Vec<_> = operations
            .iter()
            .filter(|operation| operation.value == address)
            .map(|operation| operation.place.local)
            .collect();
        let reached: Vec<_> = operations
            .iter()
            .filter(|operation| operation.place.projection == vec![Projection::Deref])
            .map(|operation| operation.place.local)
            .collect();

        assert_eq!(addressed.len(), 3, "one per subscript and one per `p + i`");
        assert_eq!(reached, addressed[..2], "the two writes reach through it");
        assert!(
            operations.iter().all(
                |operation| operation.place.local != p || operation.place.projection.is_empty()
            ),
            "nothing is a projection of the pointer itself"
        );

        // The last one is the read, and it reaches through an address of its
        // own rather than through either write's.
        let [returned] = &operations[operations.len() - 1..] else {
            panic!("a return");
        };
        assert_eq!(returned.place, Place::local(f.return_place()));
        assert_eq!(
            returned.value,
            Rvalue::Use(Operand::Copy(Place {
                local: addressed[2],
                projection: vec![Projection::Deref],
            }))
        );
    }

    /// A comma evaluates its left operand and answers its right.
    ///
    /// C17 6.5.17 p2. Mutation: answer the left operand. This fails.
    #[test]
    fn a_comma_answers_its_right_operand() {
        let lowered = lowered("int f(int a, int b) {\n    return (a, b);\n}\n");
        assert_eq!(codes(&lowered), Vec::<String>::new());

        let f = function(&lowered, "f");
        let [_, b] = f.parameters().collect::<Vec<_>>()[..] else {
            panic!("two parameters");
        };
        // The left operand is evaluated and dropped, and a name needs no
        // operation to be evaluated, so the only operation is the return.
        let [returned] = assigns(f.blocks().next().expect("a block"))[..] else {
            panic!("one operation");
        };
        assert_eq!(returned.place, Place::local(f.return_place()));
        assert_eq!(returned.value, Rvalue::Use(Operand::Copy(Place::local(b))));
    }

    /// A compound assignment is the assignment C says it stands for.
    ///
    /// C17 6.5.16.2 p3 makes `a -= b` mean `a = a - b` except that `a` is
    /// evaluated once, so it lowers to two operations: the subtraction into a
    /// temporary of the promoted type, and the place written from that. Writing
    /// the subtraction straight into `a` is shorter and says the arithmetic
    /// happened at `a`'s width, which is false whenever `a` is narrower than
    /// `int`, and is what made `char c; c += 100;` a program this compiler
    /// stopped on.
    ///
    /// Mutation: swap the operands, so `a -= b` means `b - a`. This fails.
    /// Mutation: write the `Binary` into `a` and drop the temporary. The
    /// assertions below fail, and so does
    /// `a_compound_assignment_on_a_char_is_not_undefined` in
    /// `crates/safec/tests/interp.rs`.
    #[test]
    fn a_compound_assignment_is_the_assignment_it_stands_for() {
        let lowered = lowered("int f(int a, int b) {\n    a -= b;\n    return a;\n}\n");
        assert_eq!(codes(&lowered), Vec::<String>::new());

        let f = function(&lowered, "f");
        let [a, b] = f.parameters().collect::<Vec<_>>()[..] else {
            panic!("two parameters");
        };
        let operations: Vec<_> = f
            .blocks()
            .flat_map(|block| assigns(block).into_iter().cloned().collect::<Vec<_>>())
            .collect();

        let computed = operations[0].place.local;
        assert!(operations[0].place.projection.is_empty());
        assert_ne!(
            computed, a,
            "the subtraction does not happen at `a`'s width"
        );
        assert_eq!(
            f.local(computed),
            f.local(a),
            "`int`, being the promoted type"
        );
        assert_eq!(
            operations[0].value,
            Rvalue::Binary {
                op: BinOp::Sub,
                lhs: Operand::Copy(Place::local(a)),
                rhs: Operand::Copy(Place::local(b)),
            }
        );

        assert_eq!(operations[1].place, Place::local(a));
        assert_eq!(
            operations[1].value,
            Rvalue::Use(Operand::Copy(Place::local(computed)))
        );
    }

    /// An increment answers differently before and after.
    ///
    /// C17 6.5.2.4 p2 makes the postfix form answer the value before the
    /// increment, and 6.5.3.1 p2 makes the prefix form answer the one after.
    /// Both write the same thing to the same place.
    ///
    /// Mutation: keep the old value for the prefix form rather than the
    /// postfix one. The two functions below lower alike and this fails.
    #[test]
    fn an_increment_answers_before_or_after_the_write() {
        let after = lowered("int f(int n) {\n    return n++;\n}\n");
        let before = lowered("int f(int n) {\n    return ++n;\n}\n");
        assert_eq!(codes(&after), Vec::<String>::new());
        assert_eq!(codes(&before), Vec::<String>::new());

        let taken = |lowered: &Lowered| {
            let f = function(lowered, "f");
            let operations: Vec<_> = f
                .blocks()
                .flat_map(|block| assigns(block).into_iter().cloned().collect::<Vec<_>>())
                .collect();
            // Whether the copy of the old value is taken before the write or
            // the copy of the new one after it is the whole difference.
            let stepped = operations
                .iter()
                .position(|operation| matches!(operation.value, Rvalue::Binary { .. }))
                .expect("an increment");
            let held = operations
                .iter()
                .position(|operation| {
                    matches!(&operation.value, Rvalue::Use(Operand::Copy(place))
                        if place.local == f.parameters().next().expect("a parameter"))
                })
                .expect("a copy of the place");
            held < stepped
        };

        assert!(taken(&after), "the postfix form keeps the value it had");
        assert!(
            !taken(&before),
            "the prefix form answers the value it wrote"
        );
    }

    /// A `for` becomes a header, a body, a step and an exit.
    ///
    /// C17 6.8.5.3 p1 puts the step at the end of the body and the condition
    /// before each turn, and p2 makes an absent condition a non-zero constant,
    /// so a `for (;;)` has no exit edge of its own.
    ///
    /// Mutation: send a `for` with no condition to the block after the loop.
    /// The loop stops being one and this fails.
    ///
    /// Mutation: put the step before the body rather than after it. The
    /// assertion on which block holds it fails.
    #[test]
    fn a_for_loop_asks_before_each_turn_and_steps_after_each_body() {
        let counted = lowered(
            "int f(int n) {\n    int i;\n    for (i = 0; i < n; i = i + 1) {\n        n = n - 1;\n    }\n    return n;\n}\n",
        );
        assert_eq!(codes(&counted), Vec::<String>::new());

        let f = function(&counted, "f");
        // The entry runs the initialiser and goes to the header; the header
        // asks and branches; the body runs, steps, and comes back.
        assert_eq!(edges(f), vec![vec![1], vec![2, 3], vec![1], vec![]]);

        // The body holds its own statement and the step after it, and each
        // assignment carries the copy that 6.5.16 p3 fixes its value with.
        let blocks: Vec<_> = f.blocks().collect();
        let stepped = assigns(blocks[2])
            .last()
            .copied()
            .expect("the step is written after the body");
        assert!(matches!(stepped.value, Rvalue::Use(Operand::Copy(_))));
        assert_eq!(
            assigns(blocks[2])[3].value,
            Rvalue::Binary {
                op: BinOp::Add,
                lhs: Operand::Copy(assigns(blocks[2])[4].place.clone()),
                rhs: Operand::Constant(1),
            }
        );

        let forever = lowered("int f(int n) {\n    for (;;) {\n        n = n - 1;\n    }\n}\n");
        assert_eq!(codes(&forever), Vec::<String>::new());

        let f = function(&forever, "f");
        let edges = edges(f);
        let [entry, header, body, after] = &edges[..] else {
            panic!("{edges:?}");
        };
        assert_eq!(entry, &vec![1]);
        assert_eq!(header, &vec![2]);
        assert_eq!(body, &vec![1]);
        assert_eq!(after, &Vec::<usize>::new());
    }

    /// A prototype and the definition under it are one function.
    ///
    /// Keyed by the span of the name they would be two, because the resolver
    /// answers with whichever declaration was in scope, so a call written
    /// between them would reach a function with no body.
    ///
    /// Mutation: key the function map on the name's span. Two `add`s appear,
    /// the call reaches the one with no body, and this fails.
    #[test]
    fn a_prototype_and_its_definition_are_one_function() {
        let lowered = lowered(
            "int add(int a, int b);\n\nint main(void) {\n    return add(1, 2);\n}\n\nint add(int a, int b) {\n    return a + b;\n}\n",
        );
        assert_eq!(codes(&lowered), Vec::<String>::new());
        assert_eq!(lowered.unit.functions().len(), 2);

        let add = function(&lowered, "add");
        assert!(add.is_defined());

        let main = function(&lowered, "main");
        let entry = main.blocks().next().expect("a block");
        let Terminator::Call { callee, .. } = &entry.terminator else {
            panic!("{:?}", entry.terminator);
        };
        assert!(lowered.unit.function(*callee).is_defined());
    }

    /// A name defined twice keeps the first body and is reported.
    ///
    /// C17 6.9 p5 allows one external definition and nothing before this stage
    /// checks it, so the check that would panic is answered here instead.
    ///
    /// Mutation: fill the function with the second definition anyway. The
    /// assertion in `fill_function` panics, which is a different failure and
    /// the reason this arm exists.
    #[test]
    fn a_name_defined_twice_keeps_the_first_body() {
        let lowered =
            lowered("int f(void) {\n    return 1;\n}\n\nint f(void) {\n    return 2;\n}\n");
        assert_eq!(codes(&lowered), ["SC0304"]);

        let f = function(&lowered, "f");
        assert!(f.is_defined());
        assert_eq!(
            assigns(f.blocks().next().expect("a block"))[0].value,
            Rvalue::Use(Operand::Constant(1))
        );
    }

    /// The value of an assignment is what was assigned, and a call cannot
    /// change it afterwards.
    ///
    /// C17 6.5.16 p3 fixes the value at the assignment, and 6.5.2.2 p10 makes a
    /// callee's execution indeterminately sequenced with the rest of the
    /// expression, so `(b = 1) + g(&b)` is one plus whatever `g` answers
    /// however `g` treats `b`.
    ///
    /// Mutation: push the assigned place itself as the value rather than a copy
    /// of it. The addition reads `b` after the call and this fails.
    #[test]
    fn the_value_of_an_assignment_is_taken_before_a_call_can_change_it() {
        let lowered = lowered(
            "int g(int *p);\n\nint f(void) {\n    int b;\n    return (b = 1) + g(&b);\n}\n",
        );
        assert_eq!(codes(&lowered), Vec::<String>::new());

        let f = function(&lowered, "f");
        let blocks: Vec<_> = f.blocks().collect();
        let Terminator::Call { .. } = &blocks[0].terminator else {
            panic!("{:?}", blocks[0].terminator);
        };

        // Whatever the addition reads for the left operand was written before
        // the call, which is what the block boundary says.
        let added = assigns(blocks[1])
            .into_iter()
            .find_map(|operation| match &operation.value {
                Rvalue::Binary {
                    op: BinOp::Add,
                    lhs,
                    ..
                } => Some(lhs.clone()),
                _ => None,
            })
            .expect("an addition");
        let Operand::Copy(read) = added else {
            panic!("{added:?}");
        };
        assert!(
            assigns(blocks[0])
                .into_iter()
                .any(|operation| operation.place == read),
            "the left operand is a place written before the call"
        );
    }

    /// An expression that names no place is reported where it is written.
    ///
    /// `1 = 2` type-checks today, because `types.rs` leaves the
    /// modifiable-lvalue constraint of C17 6.5.16 p2 to a later phase, so this
    /// stage is the first thing that has to say anything about it.
    ///
    /// Mutation: refuse it without reporting. Nothing is said about a program
    /// nobody can compile and this fails.
    #[test]
    fn an_expression_that_names_no_place_is_reported() {
        let lowered = lowered("int f(void) {\n    1 = 2;\n    return 0;\n}\n");
        assert_eq!(codes(&lowered), ["SC0304"]);
        assert!(!function(&lowered, "f").is_defined());
    }

    /// A call to something that is not a function is reported.
    ///
    /// The report comes from the type check having no type for the call rather
    /// than from `callee`, which is what `callee`'s own doc comment says: a
    /// callee that could name a pointer is refused at its declaration, because
    /// the IR has no function type to give it.
    ///
    /// Mutation: refuse an untyped expression without reporting. This fails.
    #[test]
    fn a_call_to_something_that_is_not_a_function_is_reported() {
        let lowered = lowered("int f(int p) {\n    return p(1);\n}\n");
        assert_eq!(codes(&lowered), ["SC0304"]);
        assert!(!function(&lowered, "f").is_defined());
    }

    /// A call to a function whose signature was refused says nothing more.
    ///
    /// The declaration is where the problem is and where it is reported; a
    /// second diagnostic on an ordinary call would be blaming code that is
    /// fine.
    ///
    /// Mutation: report at the call as well. Two codes come back and this
    /// fails.
    #[test]
    fn a_call_to_a_refused_function_is_not_reported_twice() {
        let lowered = lowered("int g(int a[3]);\n\nint f(void) {\n    return g(0);\n}\n");
        assert_eq!(codes(&lowered), ["SC0304"]);
    }

    /// An expression deeper than the parser's own nesting limit still lowers.
    ///
    /// A chain folded by a loop adds a level to the tree per operator and none
    /// to the parser's count, which is RK-008 in the review knowledge bank and
    /// what killed the tree printer once. Ten thousand terms is far past
    /// `parser::MAX_NESTING` and nowhere near the native stack.
    ///
    /// Mutation: walk an expression by recursion. The process dies rather than
    /// failing, which is why this test exists at all: a stack overflow is not a
    /// panic anything can catch.
    #[test]
    fn an_expression_deeper_than_the_parser_nests_still_lowers() {
        let terms = 10_000;
        let mut program = String::from("int f(int a) {\n    return a");
        for _ in 0..terms {
            program.push_str(" + a");
        }
        program.push_str(";\n}\n");

        let lowered = lowered(&program);
        assert_eq!(codes(&lowered), Vec::<String>::new());

        let f = function(&lowered, "f");
        let operations: usize = f.blocks().map(|block| assigns(block).len()).sum();
        // One per operator, and one more writing the answer into the return
        // place.
        assert_eq!(operations, terms + 1);
    }
}
