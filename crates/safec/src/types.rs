//! The type of every expression, and the constraints C puts on one that this
//! stage checks.
//!
//! The second half of the stage `docs/architecture.md` draws between the AST
//! and the typed AST. `sema.rs` says which declaration a name means; this says
//! what type each expression has, and reports an assignment, a compound
//! assignment, an initializer, a `return`, a call, a binary operator, an
//! increment and a subscript whose types C17 forbids.
//!
//! **A type this stage cannot work out is `None`, and `None` reports nothing.**
//! The other constraints of 6.5 are not checked here, so plenty of expressions
//! have a type that is a guess or is missing, and a check that fired on a
//! missing half would be this compiler saying something about a program it did
//! not understand. Silence is the direction to be wrong in, and it is what
//! keeps the deferred half of 6.5 from arriving as false reports.
//!
//! The walk is the arena rather than the tree. An expression is pushed only
//! once its children are, because a parent is built from their ids, so reading
//! `Ast::expr_ids` in order visits every child before its parent and visits
//! every expression there is. A walk of the tree would have to know every place
//! a child can hang, and a place it did not know would be a silent hole.

use std::cmp::Ordering;
use std::collections::HashMap;

use crate::ast::{
    Ast, BinOp, Expr, ExprId, InitDeclarator, Item, Parameters, Stmt, StmtId, Type, TypeId, UnOp,
    spell_type,
};
use crate::diagnostics::{Code, Diagnostic, DiagnosticSink, Label};
use crate::sema::Resolution;
use safec_ir::source::{SourceMap, Span};
use safec_ir::target::Integer;

/// A spelling that is not a constant C allows, C17 6.4.4.1 p1.
///
/// `SC01xx` and not `SC03xx` although this stage is the one that emits it:
/// `docs/diagnostics.md` hands out a code by the topic of the problem rather
/// than by the pass that finds it, and what is wrong with `09` is the shape of
/// the token.
const MALFORMED_CONSTANT: Code = Code::new("SC0106");

/// A constant this compiler has no type for, C17 6.4.4 p2.
///
/// One code for three messages, because they are one rule met three ways:
/// 6.4.4.1 p5 picks a constant's type from a list, and this compiler has only
/// `int` on it. A value too large, a suffix, and a floating constant are the
/// three ways to ask for an entry that is not there. Same arrangement as
/// `MISMATCH` below, for the same reason.
///
/// Every one of the three is a program `clang` compiles, which is what makes
/// the code's wording this compiler's own gap rather than the program's
/// fault. The spelling that is no constant at all is the other code.
const NO_TYPE: Code = Code::new("SC0305");

/// A value of the wrong type, C17 6.5.16.1 p1.
///
/// One code for an assignment, an initializer, a `return` and an argument,
/// because C17 makes them one rule: 6.7.9 p11 gives an initializer "the same
/// type constraints and conversions as for simple assignment", 6.8.6.4 p3
/// converts a returned value as if it were assigned to an object of the
/// return type, and 6.5.2.2 p2 requires an argument's value to be assignable
/// to an object of its parameter's type. What differs is the message, which
/// is the same reason `parser.rs`'s `EXPECTED` is one code for every shape of
/// syntax error.
const MISMATCH: Code = Code::new("SC0302");

/// A call whose argument count is not the parameter count, C17 6.5.2.2 p2.
const ARGUMENTS: Code = Code::new("SC0303");

/// An operand an operator does not take, C17 6.5.5 p2 to 6.5.14 p2 for a
/// binary operator, 6.5.16.2 p1 and p2 for a compound assignment, 6.5.2.4 p2
/// and 6.5.3.1 p2 for an increment or a decrement, 6.5.2.1 p1 for a
/// subscript, 6.5.3.2 p2 for `*`, 6.5.3.3 p1 for unary `+`, `-`, `~` and `!`,
/// 6.5.2.2 p1 for a callee, the operand of a call's `()`, 6.5.15 p2,
/// 6.8.4.1 p1 and 6.8.5 p2 for the condition of `?:`, `if`, `while` and
/// `for`, and 6.5.15 p3 for the pair of a conditional's arms.
///
/// Not `MISMATCH`, which is a value of the wrong type for a place: `p *= 2`
/// is refused because `*=` does not take a pointer, whatever `p` holds. A
/// code is never reassigned, so the two are kept apart from the start.
const OPERANDS: Code = Code::new("SC0306");

/// A `return` that does not match whether its function returns a value,
/// the two constraints of C17 6.8.6.4 p1: `return;` where the return type is
/// not `void`, and `return e;` where it is, `e` a `void` expression such as
/// `h()` included.
///
/// Not `MISMATCH`, which is a value of the wrong type for a place: this is
/// whether there is an expression at all, as `OPERANDS` is kept apart from
/// it. `clang` 20 refuses `return;` and `return 1;` by default and accepts
/// `return h();` in a `void` function unless `-pedantic-errors` is given;
/// C forbids it, and it is refused here as `docs/frontend.md` lists.
const RETURN_SHAPE: Code = Code::new("SC0308");

/// A declarator that derives a type C forbids, C17 6.7.6.2 so far: an array
/// whose element is not a complete object type, or whose length is not an
/// integer, or is a constant that is not greater than zero (p1); and, at file
/// scope, an array length without a value, which is either a constant
/// expression that overflows or divides by zero (6.6 p4) or not a constant
/// at all (p2).
///
/// A constant whose value [`Checker::evaluate`] gives: `[-1]` and `[1 - 1]`
/// are asked p1's question. One whose evaluation overflows is asked p2's
/// at file scope and nothing in a block, where a length need not be a
/// constant. A zero length is refused though `clang` takes it as an
/// extension, which `docs/frontend.md` records.
///
/// Not `OPERANDS`, which is an operator's: a declarator is not an operator,
/// and a code is never reassigned. 6.7.6.3 p1, a function returning an array
/// or a function, is the same kind of fault and would go here.
const DECLARATOR: Code = Code::new("SC0309");

/// Something initialized that C17 6.7.9 p3 says cannot be: a function, an
/// object of incomplete type, or a variable length array. Only an array of
/// unknown size or a complete object type that is not a variable length
/// array may have an initializer.
///
/// Not `MISMATCH`, because no value of any type would make `int f(void) = 1;`
/// right: what has to change is the `=` or the declaration. Not `DECLARATOR`,
/// because `int f(void);` is a correct declarator, and the fault is
/// initializing it.
const INITIALIZED: Code = Code::new("SC0310");

/// The type of every expression in one translation unit.
#[derive(Clone, Debug)]
pub struct Types {
    of: Vec<Option<TypeId>>,
    value: Vec<Option<i128>>,
}

impl Types {
    /// The type of the expression `id`, or `None` where this stage could not
    /// say what it is.
    ///
    /// `None` is not `void`: it means nothing here worked the type out, either
    /// because a name did not resolve or because the expression is one of the
    /// shapes this stage does not type yet. A later phase has to answer for it
    /// rather than assume.
    ///
    /// # Panics
    ///
    /// If `id` came from a different [`Ast`].
    pub fn of(&self, id: ExprId) -> Option<TypeId> {
        self.of[id.index()]
    }

    /// What the integer constant expression `id` is worth, or `None` where it
    /// is not one, is a literal this stage could not read, or is one whose
    /// operation C leaves without a defined result.
    ///
    /// The cases are one answer on purpose: a caller has nothing to do
    /// differently. A literal that could not be read was reported where it
    /// was read; an expression that is not a constant has no value to ask
    /// for; and one that overflows or divides by zero is, to every caller but
    /// one, an expression that is not a constant. All are "do not build an
    /// operand out of this", which `Checker::evaluate` says more of. The one
    /// that tells them apart is the type checker's own report on an array
    /// length at file scope, which this table does not carry.
    ///
    /// # Panics
    ///
    /// If `id` came from a different [`Ast`].
    pub fn value(&self, id: ExprId) -> Option<i128> {
        self.value[id.index()]
    }
}

/// Give every expression a type, and report the constraints this stage checks.
///
/// Takes `&mut Ast` because two of the types it works out are types nobody
/// wrote: the `int` of an integer constant, and the pointer that `&x` has.
/// They go in the same arena as the rest, so that there is one representation
/// of a type in this compiler rather than two.
///
/// Takes `int_range` rather than a whole [`safec_ir::target::Target`] because
/// that is all it reads: whether an integer constant's value is in the range
/// of the one integer type this compiler has. ADR-0013 asks that a stage be
/// given only what it reads, and its *Only what something reads* section names
/// this stage as the second reader of `int`'s width. A diagnostic that turns
/// on a width is a divergence from `docs/architecture.md`'s output table,
/// recorded there.
pub fn check(
    sources: &SourceMap,
    ast: &mut Ast,
    resolution: &Resolution,
    int_range: Integer,
    diagnostics: &mut DiagnosticSink,
) -> Types {
    let mut checker = Checker {
        sources,
        resolution,
        types: vec![None; ast.expr_ids().count()],
        values: vec![None; ast.expr_ids().count()],
        undefined: vec![false; ast.expr_ids().count()],
        // Pushed once. A new node per constant would fill the arena with
        // copies of `int` and make nothing truer.
        int: ast.push_type(Type::Int),
        int_range,
        receivers: HashMap::new(),
    };

    checker.collect_receivers(ast, diagnostics);

    for id in ast.expr_ids().collect::<Vec<_>>() {
        let ty = checker.type_of(ast, id, diagnostics);
        checker.types[id.index()] = ty;
        // A literal's value is `constant`'s, set while it was typed; every
        // other expression's is worked out from its operands' here, before
        // anything that reads it as a null pointer constant is asked.
        if ty.is_some() && checker.values[id.index()].is_none() {
            checker.values[id.index()] = checker.evaluate(ast, id);
            checker.undefined[id.index()] =
                checker.values[id.index()].is_none() && checker.constant_shaped(ast, id);
        }
        checker.check_received(ast, id, diagnostics);
    }

    // After every expression is typed, because a length's type and value are
    // what two of the three questions ask.
    checker.check_declarators(ast, diagnostics);
    checker.check_initialized(ast, diagnostics);

    Types {
        of: checker.types,
        value: checker.values,
    }
}

/// What a `return` in this function has to be assignable to.
#[derive(Clone, Copy)]
struct Returning {
    /// The function's return type.
    ty: TypeId,
    /// The span of its name, for a label that says where that was decided.
    name: Span,
}

/// What a value found by walking the statements has to satisfy, and what a
/// report about it says.
///
/// Mostly what it has to be assignable to: a `return` and an initializer are
/// one rule in C17 (see `MISMATCH`). A statement's condition is another rule,
/// that it be a scalar, and is here because the same walk finds it: a second
/// walk would be a second recursion over every statement, and an arm dropped
/// from either would be a silence that the other's test could not see.
#[derive(Clone, Copy)]
enum Receiving {
    /// C17 6.8.6.4 p3.
    Return(Returning),
    /// An expression returned from a function that returns `void`, which
    /// C17 6.8.6.4 p1 forbids whatever its type. See `RETURN_SHAPE`.
    ReturnFromVoid(Returning),
    /// C17 6.7.9 p11.
    Initializer {
        /// The type the declarator derived.
        ty: TypeId,
        /// The span of the declarator's name, which is the place.
        name: Span,
    },
    /// The condition of `if`, C17 6.8.4.1 p1, or of `while` or `for`, 6.8.5
    /// p2, which has to be a scalar. Not a place a value is assigned to,
    /// and collected by the same walk because that walk is the one that
    /// visits every statement.
    Condition {
        /// The statement's keyword, which is what the report names.
        keyword: &'static str,
    },
}

struct Checker<'a> {
    sources: &'a SourceMap,
    resolution: &'a Resolution,
    types: Vec<Option<TypeId>>,
    /// What each integer constant is worth, filled by the same walk that
    /// fills `types` and empty everywhere else.
    values: Vec<Option<i128>>,
    /// Whether each expression is a constant expression in shape whose
    /// evaluation overflows or has no defined result, so that
    /// [`Checker::evaluate`] gave it no value. Set by `check` beside the
    /// value, from [`Checker::constant_shaped`]. What tells `int a[1 << 31];`
    /// (C17 6.6 p4) from `int a[n];` (6.7.6.2 p2) when a length at file scope
    /// has no value.
    undefined: Vec<bool>,
    int: TypeId,
    /// The range of `int` on the target this run is for.
    int_range: Integer,
    /// The value of each `return` that has one and of each initializer, and
    /// what it has to be assignable to.
    ///
    /// Collected before the walk so that the walk stays in one order. A
    /// `return` and a declaration are statements and the types are worked out
    /// over the arena, so without this they would be reported after every
    /// expression rather than among them, and a reader would find them out of
    /// order. One map is enough because no expression is both.
    ///
    /// **A `return;` that owes a value is reported while this is
    /// collected**, before every expression's report, since it has no
    /// expression to be reported beside: a type error on an earlier line is
    /// printed after it. One with an expression where the function returns
    /// `void` is recorded here and reported beside its expression, in order.
    /// See `RETURN_SHAPE`.
    receivers: HashMap<ExprId, Receiving>,
}

impl Checker<'_> {
    /// Hold every declarator to C17 6.7.6.2 p1, and report the outermost array
    /// in each that breaks it under [`DECLARATOR`].
    ///
    /// Every declaration in the unit, at file scope, in a block, and as a
    /// parameter of any function type written anywhere, so `int (*fp)(void
    /// a[])` is reached. Each is asked of its [`Declaration::written`] type,
    /// because a parameter's `ty` is the adjusted pointer and has lost the
    /// array (C17 6.7.6.3 p7). One report per declaration, the outermost
    /// array first, so `int z[0][0]` is one fault and not two.
    ///
    /// [`Declaration::written`]: crate::ast::Declaration::written
    fn check_declarators(&self, ast: &Ast, diagnostics: &mut DiagnosticSink) {
        // What was declared, where to point, the type as written, and whether
        // it is at file scope, where C17 6.7.6.2 p2 wants every array length
        // a constant: only an identifier with block or prototype scope may
        // have a variably modified type, so an object there and a function's
        // own type, `int (*f(void))[n]`, may not, whether declared or
        // defined. A parameter, reached through any function type, has
        // prototype or block scope.
        let mut declared: Vec<(Span, TypeId, bool)> = Vec::new();
        let push = |declarators: &[InitDeclarator],
                    at_file_scope: bool,
                    declared: &mut Vec<(Span, TypeId, bool)>| {
            for declarator in declarators {
                let declaration = &declarator.declaration;
                declared.push((
                    declaration.name.unwrap_or(declaration.span),
                    declaration.written,
                    at_file_scope,
                ));
            }
        };
        for item in ast.items() {
            match item {
                Item::Function(function) => declared.push((function.name, function.ty, true)),
                Item::Declaration { declarators, .. } => push(declarators, true, &mut declared),
                Item::Error { .. } => {}
            }
        }
        for id in ast.stmt_ids() {
            if let Stmt::Declaration { declarators, .. } = ast.stmt(id) {
                push(declarators, false, &mut declared);
            }
        }

        // Grows as it is read: every function type met on the way down hands
        // its parameters on as declarations of their own.
        let mut next = 0;
        while let Some(&(at, written, at_file_scope)) = declared.get(next) {
            next += 1;
            let mut reported = false;
            let mut current = written;
            loop {
                match ast.ty(current) {
                    Type::Array {
                        element,
                        length,
                        written: _,
                    } => {
                        let (element, length) = (*element, *length);
                        if !reported {
                            reported = self.report_array(
                                ast,
                                at,
                                element,
                                length,
                                at_file_scope,
                                diagnostics,
                            );
                        }
                        current = element;
                    }
                    Type::Pointer(pointee) => current = *pointee,
                    Type::Function {
                        returns,
                        parameters,
                        ..
                    } => {
                        if let Parameters::Prototype(parameters) = parameters {
                            for parameter in parameters {
                                declared.push((
                                    parameter.name.unwrap_or(parameter.span),
                                    parameter.written,
                                    false,
                                ));
                            }
                        }
                        current = *returns;
                    }
                    Type::Int | Type::Char | Type::Void => break,
                }
            }
        }
    }

    /// Hold every declarator that has an initializer to C17 6.7.9 p3, and
    /// report one that initializes what cannot be under [`INITIALIZED`].
    ///
    /// At file scope and in a block alike, since p3 is about the entity and
    /// not where it is. `assignable` answers nothing for a function, `void` or
    /// an array target, so without this `int f(void) = 1;` was accepted in
    /// silence and its initializer dropped. A variable length array is one
    /// whose length has no value; at file scope the same declarator is also
    /// 6.7.6.2 p2's, and both are reported, being two constraints.
    fn check_initialized(&self, ast: &Ast, diagnostics: &mut DiagnosticSink) {
        let mut declared: Vec<&InitDeclarator> = Vec::new();
        for item in ast.items() {
            if let Item::Declaration { declarators, .. } = item {
                declared.extend(declarators);
            }
        }
        for id in ast.stmt_ids() {
            if let Stmt::Declaration { declarators, .. } = ast.stmt(id) {
                declared.extend(declarators);
            }
        }

        for declarator in declared {
            let Some(init) = declarator.init else {
                continue;
            };
            let declaration = &declarator.declaration;
            let ty = declaration.ty;
            let name = declaration.name.unwrap_or(declaration.span);
            let called = self.sources.snippet(name);
            // Every variant named, so that a type added later is asked here.
            let message = match ast.ty(ty) {
                Type::Function { .. } => {
                    format!("`{called}` is a function, and only an object can be initialized")
                }
                Type::Void => format!(
                    "`{called}` has type `void`, which is incomplete, and only a complete object can be initialized"
                ),
                Type::Array {
                    length: Some(length),
                    ..
                } if self.values[length.index()].is_none() => {
                    format!("`{called}` is a variable length array, which cannot be initialized")
                }
                Type::Array { .. } | Type::Int | Type::Char | Type::Pointer(_) => continue,
            };
            diagnostics.report(
                Diagnostic::error(message)
                    .with_code(INITIALIZED)
                    .with_label(Label::primary(
                        name,
                        format!("declared as `{}`", self.spelled(ast, ty)),
                    ))
                    .with_label(Label::secondary(ast.expr(init).span(), "this initializer"))
                    .with_note("C17 6.7.9 p3"),
            );
        }
    }

    /// Report one array of a declarator at `at` that C17 6.7.6.2 p1 forbids,
    /// or, `at_file_scope`, whose length p2 and 6.6 p4 forbid, and say
    /// whether it was one.
    fn report_array(
        &self,
        ast: &Ast,
        at: Span,
        element: TypeId,
        length: Option<ExprId>,
        at_file_scope: bool,
        diagnostics: &mut DiagnosticSink,
    ) -> bool {
        // "The element type shall not be an incomplete or function type."
        // `void` and an array of unknown length are the incomplete types this
        // compiler has.
        let incomplete = match ast.ty(element) {
            Type::Void | Type::Array { length: None, .. } | Type::Function { .. } => true,
            Type::Int
            | Type::Char
            | Type::Pointer(_)
            | Type::Array {
                length: Some(_), ..
            } => false,
        };
        if incomplete {
            let spelled = self.spelled(ast, element);
            diagnostics.report(
                Diagnostic::error(format!("an array cannot have `{spelled}` as its element"))
                    .with_code(DECLARATOR)
                    .with_label(Label::primary(at, "declared here"))
                    .with_note(
                        "the element of an array is a complete object type (C17 6.7.6.2 p1)",
                    ),
            );
            return true;
        }

        let Some(length) = length else {
            return false;
        };
        // "the expression shall have an integer type", and an untyped one,
        // `p - q`, is asked nothing, as `subscript` asks nothing of it.
        let ty = self.types[length.index()];
        if let Some(ty) = ty.filter(|&ty| !matches!(ast.ty(ty), Type::Int | Type::Char)) {
            let spelled = self.spelled(ast, ty);
            diagnostics.report(
                Diagnostic::error(format!("the length of an array cannot be `{spelled}`"))
                    .with_code(DECLARATOR)
                    .with_label(Label::primary(
                        ast.expr(length).span(),
                        format!("this is `{spelled}`"),
                    ))
                    .with_note("the length of an array is an integer (C17 6.7.6.2 p1)"),
            );
            return true;
        }
        // "If the expression is a constant expression, it shall have a value
        // greater than zero." Of a constant whose value this stage knows; see
        // `DECLARATOR`.
        if let Some(value) = self.values[length.index()].filter(|&value| value <= 0) {
            diagnostics.report(
                Diagnostic::error("the length of an array is greater than zero")
                    .with_code(DECLARATOR)
                    .with_label(Label::primary(
                        ast.expr(length).span(),
                        format!("this is {value}"),
                    ))
                    .with_note("a constant length is greater than zero (C17 6.7.6.2 p1)"),
            );
            return true;
        }
        // At file scope a length has to be a constant with a value. Untyped,
        // it was asked nothing above and is asked nothing here: an undeclared
        // name has been reported already, and `p - q`, the one untyped length
        // a valid program can write, goes unasked for want of a `ptrdiff_t`.
        if at_file_scope && ty.is_some() && self.values[length.index()].is_none() {
            let at = ast.expr(length).span();
            diagnostics.report(if self.undefined[length.index()] {
                Diagnostic::error("the length of an array has no defined value")
                    .with_code(DECLARATOR)
                    .with_label(Label::primary(at, "this overflows `int` or divides by zero"))
                    .with_note(
                        "a constant expression evaluates to a value its type can hold (C17 6.6 p4)",
                    )
            } else {
                Diagnostic::error(
                    "a declaration at file scope cannot have an array length that is not a constant",
                )
                .with_code(DECLARATOR)
                .with_label(Label::primary(at, "this is not a constant"))
                .with_note(
                    "a variably modified type is allowed only at block scope or in a prototype (C17 6.7.6.2 p2)",
                )
            });
            return true;
        }
        false
    }

    /// Whether `id` is a constant expression in shape: an operator
    /// [`Checker::evaluate`] reads, over operands that each have a value or
    /// are themselves constant in shape and undefined. So `1 << 31` and
    /// `(1 << 31) + 1` are, and `n + 1` and a call are not. Asked only of an
    /// expression `evaluate` gave no value, where it says why: `1 + 2` is
    /// constant in shape too, and has its value.
    fn constant_shaped(&self, ast: &Ast, id: ExprId) -> bool {
        let known = |id: ExprId| self.values[id.index()].is_some() || self.undefined[id.index()];
        match *ast.expr(id) {
            Expr::Unary { op, operand, .. } => {
                matches!(op, UnOp::Plus | UnOp::Minus | UnOp::BitNot | UnOp::Not) && known(operand)
            }
            Expr::Binary { lhs, rhs, .. } => known(lhs) && known(rhs),
            Expr::Conditional {
                condition,
                then,
                otherwise,
                ..
            } => known(condition) && known(then) && known(otherwise),
            Expr::Number { .. }
            | Expr::Identifier { .. }
            | Expr::Assign { .. }
            | Expr::Comma { .. }
            | Expr::Subscript { .. }
            | Expr::Call { .. }
            | Expr::Error { .. } => false,
        }
    }

    /// Find every `return` with a value and every initializer, and what each
    /// has to be assignable to, and report every `return` that has a value
    /// where it may not or lacks one where it must.
    fn collect_receivers(&mut self, ast: &Ast, diagnostics: &mut DiagnosticSink) {
        for item in ast.items() {
            let function = match item {
                Item::Function(function) => function,
                Item::Declaration { declarators, .. } => {
                    self.initializers(declarators);
                    continue;
                }
                Item::Error { .. } => continue,
            };
            let Type::Function { returns, .. } = ast.ty(function.ty) else {
                // A definition whose declarator derived something else is a
                // constraint violation of 6.9.1 p2 that nothing reports yet.
                continue;
            };

            self.receivers_in(
                ast,
                function.body,
                Returning {
                    ty: *returns,
                    name: function.name,
                },
                diagnostics,
            );
        }
    }

    /// Every `return` and every initializer under one statement.
    ///
    /// A recursion, bounded by `parser::MAX_NESTING` the way every statement
    /// walk here is, and exhaustive so that a statement kind that can hold
    /// another has to be answered for rather than silently dropping the
    /// returns and initializers inside it.
    fn receivers_in(
        &mut self,
        ast: &Ast,
        id: StmtId,
        returning: Returning,
        diagnostics: &mut DiagnosticSink,
    ) {
        match ast.stmt(id) {
            Stmt::Return { value, span } => {
                let void = matches!(ast.ty(returning.ty), Type::Void);
                match (*value, void) {
                    (Some(value), false) => {
                        self.receivers.insert(value, Receiving::Return(returning));
                    }
                    (None, true) => {}
                    (None, false) => {
                        let spelled = self.spelled(ast, returning.ty);
                        diagnostics.report(
                            Diagnostic::error(format!(
                                "`return` without a value in a function returning `{spelled}`"
                            ))
                            .with_code(RETURN_SHAPE)
                            .with_label(Label::primary(*span, "this returns nothing"))
                            .with_label(Label::secondary(
                                returning.name,
                                format!("declared to return `{spelled}`"),
                            )),
                        );
                    }
                    (Some(value), true) => {
                        self.receivers
                            .insert(value, Receiving::ReturnFromVoid(returning));
                    }
                }
            }
            Stmt::Declaration { declarators, .. } => self.initializers(declarators),
            Stmt::Compound { body, .. } => {
                for &statement in body {
                    self.receivers_in(ast, statement, returning, diagnostics);
                }
            }
            Stmt::If {
                condition,
                then,
                otherwise,
                ..
            } => {
                self.receivers
                    .insert(*condition, Receiving::Condition { keyword: "if" });
                self.receivers_in(ast, *then, returning, diagnostics);
                if let Some(otherwise) = *otherwise {
                    self.receivers_in(ast, otherwise, returning, diagnostics);
                }
            }
            Stmt::While {
                condition, body, ..
            } => {
                self.receivers
                    .insert(*condition, Receiving::Condition { keyword: "while" });
                self.receivers_in(ast, *body, returning, diagnostics);
            }
            Stmt::For {
                condition, body, ..
            } => {
                if let Some(condition) = *condition {
                    self.receivers
                        .insert(condition, Receiving::Condition { keyword: "for" });
                }
                self.receivers_in(ast, *body, returning, diagnostics);
            }
            Stmt::Expression { .. } | Stmt::Error { .. } => {}
        }
    }

    /// The initializer of each declarator that has one.
    ///
    /// The name falls back to the whole declaration where there is none. The
    /// parser always gives an init-declarator one, and the field is an
    /// `Option` only because a parameter may be abstract, but answering
    /// `None` by skipping the declarator would be a check that goes silent on
    /// a case nobody can see.
    fn initializers(&mut self, declarators: &[InitDeclarator]) {
        for declarator in declarators {
            let Some(init) = declarator.init else {
                continue;
            };
            let declaration = &declarator.declaration;
            self.receivers.insert(
                init,
                Receiving::Initializer {
                    ty: declaration.ty,
                    name: declaration.name.unwrap_or(declaration.span),
                },
            );
        }
    }

    /// The type of one expression, and the constraints that are its own.
    ///
    /// Every child already has its type, because the arena holds a child
    /// before its parent.
    fn type_of(
        &mut self,
        ast: &mut Ast,
        id: ExprId,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<TypeId> {
        // Cloned rather than borrowed, because working out the type can push
        // into the arena that the borrow would be from. A `Call`'s argument
        // list is the only part of this that allocates.
        let expr = ast.expr(id).clone();

        match expr {
            Expr::Number { span } => self.constant(id, span, diagnostics),
            Expr::Identifier { .. } => {
                Some(self.resolution.binding(self.resolution.resolved(id)?).ty)
            }
            Expr::Unary { op, operand, .. } => self.unary(ast, op, operand, diagnostics),
            Expr::Binary { op, lhs, rhs, .. } => self.binary(ast, op, lhs, rhs, diagnostics),
            Expr::Assign {
                op, place, value, ..
            } => {
                // Two rules, because C17 has two: a plain `=` is 6.5.16.1,
                // and a compound assignment is 6.5.16.2, whose constraints
                // are its own. `p += 1` is legal for a pointer and the first
                // rule would report it.
                match op {
                    None => self.assignment(ast, place, value, diagnostics),
                    Some(op) => self.compound_assignment(ast, op, place, value, diagnostics),
                }
                // 6.5.16 p3: the type is "the type the left operand would have
                // after lvalue conversion", which is the place's own while
                // there are no qualifiers to drop.
                self.types[place.index()]
            }
            // 6.5.17 p2: the result of a comma is the value of the right
            // operand, and its type.
            Expr::Comma { rhs, .. } => self.types[rhs.index()],
            Expr::Subscript { base, index, .. } => self.subscript(ast, base, index, diagnostics),
            Expr::Call {
                callee, arguments, ..
            } => self.call(ast, id, callee, &arguments, diagnostics),
            Expr::Conditional {
                condition,
                then,
                otherwise,
                ..
            } => {
                // 6.5.15 p2. Checked here rather than collected by the
                // statement walk, because a refused conditional has to have
                // no type, as a refused binary operation has none, and only
                // this arm can withhold it: kept, `int *q = h() ? 1 : 2;`
                // would be reported a second time. An untyped condition was
                // reported by whatever left it untyped.
                if let Some(asked) = self.types[condition.index()] {
                    if self.refuse_a_void_condition(ast, "?:", condition, asked, diagnostics) {
                        return None;
                    }
                }
                self.conditional(ast, then, otherwise, diagnostics)
            }
            Expr::Error { .. } => None,
        }
    }

    fn unary(
        &mut self,
        ast: &mut Ast,
        op: UnOp,
        operand: ExprId,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<TypeId> {
        let operand_ty = self.types[operand.index()];

        match op {
            // 6.5.3.2 p3: `&x` is a pointer to the type of `x`. The type
            // itself is one nobody wrote, so it is pushed.
            UnOp::AddrOf => Some(ast.push_type(Type::Pointer(operand_ty?))),
            // p4: `*p` is what `p` points at.
            UnOp::Deref => match ast.ty(operand_ty?) {
                Type::Pointer(pointee) => Some(*pointee),
                // An operand of function type is a function designator.
                // 6.3.2.1 p4 converts it to a pointer to the function, and
                // 6.5.3.2 p4 makes `*` of that pointer a designator of the
                // same function, so `*g` has `g`'s type and `(*g)(x)` is
                // checked as `g(x)` is. `*fp` reaches the function through
                // the arm above.
                Type::Function { .. } => operand_ty,
                // Valid C: 6.3.2.1 p3 makes `a` a pointer to its first
                // element, and `*` does not ask `decayed` for it. Having no
                // type here is this compiler's gap, so it is not reported as
                // the program's.
                Type::Array { .. } => None,
                // p2: the operand of `*` shall have pointer type. Reported,
                // and `None`: there is no type `*x` would have had. Nothing
                // after the type check reads the tree once it has reported,
                // so no later stage speaks about the missing type.
                Type::Int | Type::Char | Type::Void => {
                    let spelled = self.spelled(ast, operand_ty?);
                    diagnostics.report(
                        Diagnostic::error(format!("`*` cannot take `{spelled}`"))
                            .with_code(OPERANDS)
                            .with_label(Label::primary(
                                ast.expr(operand).span(),
                                format!("this is `{spelled}`"),
                            )),
                    );
                    None
                }
            },
            // Each form cites its own paragraph, the one that sends it to the
            // additive operators, and the arms are split so that the clause is
            // chosen where the variant is matched.
            UnOp::PostInc | UnOp::PostDec => {
                self.increment(ast, op, operand, "C17 6.5.2.4 p2", diagnostics)
            }
            UnOp::PreInc | UnOp::PreDec => {
                self.increment(ast, op, operand, "C17 6.5.3.1 p2", diagnostics)
            }
            // 6.5.3.3: the integer promotions of 6.3.1.1 make the result of
            // `+`, `-` and `~` an `int` for every operand p1 lets them take,
            // and p5 makes `!` an `int` outright. Still `None` for an operand
            // nothing typed, because an operand that is not a number at all
            // makes an expression with no type rather than an `int`: an `int`
            // there would have `p = -nowhere` reported a second time, as an
            // `int` given to a pointer.
            UnOp::Plus | UnOp::Minus | UnOp::Not | UnOp::BitNot => {
                // Decayed first, as `binary` does, so an array or a function
                // is the pointer 6.3.2.1 p3 and p4 make it, and is spelled as
                // one in the report.
                let ty = self.decayed(ast, operand_ty?);
                // p1: `+` and `-` take an arithmetic operand, `~` an integer
                // one, and `!` a scalar one. One arm answers both of the
                // first two, because every arithmetic type this compiler has
                // is an integer type; a floating type would split it.
                let takes = match ast.ty(ty) {
                    Type::Int | Type::Char => true,
                    // `decayed` answers neither an array nor a function, and
                    // each would be the pointer it decays to if it did.
                    Type::Pointer(_) | Type::Array { .. } | Type::Function { .. } => {
                        matches!(op, UnOp::Not)
                    }
                    Type::Void => false,
                };
                if !takes {
                    let spelled = self.spelled(ast, ty);
                    diagnostics.report(
                        Diagnostic::error(format!("`{}` cannot take `{spelled}`", op.as_str()))
                            .with_code(OPERANDS)
                            .with_label(Label::primary(
                                ast.expr(operand).span(),
                                format!("this is `{spelled}`"),
                            )),
                    );
                    // No type, as for a refused binary operation: `-p` has
                    // none C would give it, as `*x` has none, and an `int`
                    // here would have `*+p` reported a second time at the
                    // `*`.
                    return None;
                }
                Some(self.int)
            }
        }
    }

    /// C17 6.5.2.4 and 6.5.3.1: the type of an increment or a decrement of
    /// `operand`, and whether a pointer there can take the step. `clause` is
    /// the paragraph of the form that was written.
    ///
    /// The type is the operand's, so `p++` on a pointer is a pointer and not
    /// an `int`. It is answered whether or not the increment is refused,
    /// because C gives it from the operand rather than from the step.
    ///
    /// p2 of either clause sends an increment to 6.5.6 and 6.5.16.2 for its
    /// constraints, because `++E` is `E += 1`, so a pointer `v += 1` refuses
    /// is refused here by the same function, [`unsteppable`]. It is not
    /// reported as that `+=`, though: the message would name an operator and
    /// an `int` the reader never wrote, and the `int` would have nowhere to
    /// put its label. One type and one label, because there is one operand.
    ///
    /// The operand is not decayed: an array or a function there breaks p1's
    /// modifiable lvalue, which is not this rule, and decaying it would report
    /// it in this rule's words.
    fn increment(
        &self,
        ast: &Ast,
        op: UnOp,
        operand: ExprId,
        clause: &str,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<TypeId> {
        let ty = self.types[operand.index()]?;

        if let Type::Pointer(pointee) = ast.ty(ty) {
            if let Some(why) = unsteppable(ast, *pointee) {
                let spelled = self.spelled(ast, ty);
                diagnostics.report(
                    Diagnostic::error(format!("`{}` cannot take `{spelled}`", op.as_str()))
                        .with_code(OPERANDS)
                        .with_label(Label::primary(
                            ast.expr(operand).span(),
                            format!("this is `{spelled}`"),
                        ))
                        .with_note(stepping_note(why, clause)),
                );
            }
        }

        Some(ty)
    }

    /// C17 6.5.2.1: the type of `base[index]`, and whether a pointer in it
    /// points to something a step can be taken over.
    ///
    /// The type is what the pointer points at, whichever operand it is,
    /// because p2 makes `E1[E2]` mean `*((E1)+(E2))` and an addition does not
    /// care which side its pointer is on: `1[p]` is `p[1]`. It is answered
    /// whether or not the pointer is refused for what it points to, because C
    /// gives it from the pointer rather than from the step, so `1[v]` is
    /// `void` as `v[1]` is, and `g[1]` is a function as `fp[1]` is.
    ///
    /// p1 wants one operand a pointer to a complete object type and the other
    /// an integer. A pointer beside an integer is asked [`unsteppable`], the
    /// function `v + 1` asks, whichever side it was written on. Any other
    /// pairing, `x[0]`, `p[q]` or a `void` operand, is refused and answers no
    /// type. All of that is asked only where both operands are typed: `x[p -
    /// q]` is an integer subscripted by a pointer difference this compiler
    /// has no type for, and goes unreported here and untyped into the
    /// lowering, for want of a `ptrdiff_t` rather than of a rule. The
    /// operands are decayed first, because 6.3.2.1 converts an
    /// array or a function before `[]` sees it, and a function `g` is refused
    /// in `g[1]` only as the pointer it becomes.
    fn subscript(
        &mut self,
        ast: &mut Ast,
        base: ExprId,
        index: ExprId,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<TypeId> {
        if let (Some(left), Some(right)) = (self.types[base.index()], self.types[index.index()]) {
            let (left, right) = (self.decayed(ast, left), self.decayed(ast, right));
            // C17 6.5.2.1 p1: one operand a pointer to a complete object type
            // and the other an integer. Anything else is refused and answers
            // no type, which keeps `x[0]` from reaching the lowering untyped
            // and being called a gap in this compiler, and keeps `int *r =
            // p[q];` from a second report about an `int` nobody wrote.
            let refused = match (OperandClass::of(ast, left), OperandClass::of(ast, right)) {
                (Some(OperandClass::Pointer(pointee)), Some(OperandClass::Arithmetic)) => {
                    pointee_steppable(ast, pointee, true).err()
                }
                (Some(OperandClass::Arithmetic), Some(OperandClass::Pointer(pointee))) => {
                    pointee_steppable(ast, pointee, false).err()
                }
                _ => Some(Refused::Pairing),
            };
            if let Some(refused) = refused {
                self.report_subscript(ast, (base, left), (index, right), refused, diagnostics);
                if matches!(refused, Refused::Pairing) {
                    return None;
                }
            }
        }

        // Whichever operand is the pointer, decayed. Where one operand has no
        // type nothing above was asked, and the other is still read: `p[i]`
        // beside an untyped `i` is what `p` points at.
        [base, index].into_iter().find_map(|operand| {
            let ty = self.types[operand.index()]?;
            let ty = self.decayed(ast, ty);
            match ast.ty(ty) {
                Type::Pointer(pointee) => Some(*pointee),
                Type::Int
                | Type::Char
                | Type::Void
                | Type::Array { .. }
                | Type::Function { .. } => None,
            }
        })
    }

    /// Report a subscript whose pointer [`unsteppable`] refused, or whose
    /// operands are not one pointer and one integer. Each operand is its
    /// expression and its type after [`Checker::decayed`].
    ///
    /// The shape of [`Checker::report_operands`]: both types in the order they
    /// were written, and the primary label on what is wrong. A pointer
    /// refused for what it points to is wrong alone, whichever side it is on;
    /// labelling the base instead would point at an innocent `1` in `1[v]`.
    /// Where the pairing is refused, the operand that cannot stand beside the
    /// other is.
    fn report_subscript(
        &self,
        ast: &Ast,
        (base, left): (ExprId, TypeId),
        (index, right): (ExprId, TypeId),
        refused: Refused,
        diagnostics: &mut DiagnosticSink,
    ) {
        let left_spelled = self.spelled(ast, left);
        let right_spelled = self.spelled(ast, right);
        // The pointer where it is refused for what it points to. Where the
        // pairing is refused, the operand beside a pointer is what does not
        // fit, as in `p[g()]` and the `q` of `p[q]`; with no pointer, a `void`
        // operand, as the `g()` of `1[g()]`; with neither, as in `x[0]`, the
        // base.
        let on_left = match refused {
            Refused::Pointee { on_left, .. } => on_left,
            Refused::Pairing => match (ast.ty(left), ast.ty(right)) {
                (Type::Pointer(_), _) => false,
                (_, Type::Pointer(_)) => true,
                (_, Type::Void) => false,
                _ => true,
            },
        };
        let (primary, secondary) = if on_left {
            ((base, &left_spelled), (index, &right_spelled))
        } else {
            ((index, &right_spelled), (base, &left_spelled))
        };

        diagnostics.report(
            Diagnostic::error(format!(
                "`[]` cannot take `{left_spelled}` and `{right_spelled}`"
            ))
            .with_code(OPERANDS)
            .with_label(Label::primary(
                ast.expr(primary.0).span(),
                format!("this is `{}`", primary.1),
            ))
            .with_label(Label::secondary(
                ast.expr(secondary.0).span(),
                format!("this is `{}`", secondary.1),
            ))
            .with_note(match refused {
                Refused::Pointee { why, .. } => stepping_note(why, "C17 6.5.2.1 p1"),
                Refused::Pairing => {
                    "one operand of `[]` is a pointer and the other an integer (C17 6.5.2.1 p1)"
                        .to_owned()
                }
            }),
        );
    }

    /// C17 6.5.5 to 6.5.14: the type of a binary operation, and whether its
    /// operands are ones its operator takes.
    ///
    /// No type for an operation it refuses, which this reports: that is the
    /// answer `*`, a call and the unary operators give, and an `int` there
    /// had `p = p * 1` reported a second time, as an `int` given to a pointer
    /// the program never had.
    ///
    /// Beside an operand this stage could not type, every operator but `+`
    /// and `-` is still `int`, because an operand can be untyped with
    /// nothing reported: `p - q` is valid C, and this compiler has no type
    /// for its `ptrdiff_t`. No type there would skip the check around it, so
    /// `int *r = (p - q) * 2;` would lose its `SC0302`. The cost is that
    /// `p = nowhere * 1` is reported as an undeclared name and then as an
    /// `int` given to a pointer.
    fn binary(
        &mut self,
        ast: &mut Ast,
        op: BinOp,
        lhs: ExprId,
        rhs: ExprId,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<TypeId> {
        let operands = match (self.types[lhs.index()], self.types[rhs.index()]) {
            (Some(left), Some(right)) => Some((self.decayed(ast, left), self.decayed(ast, right))),
            _ => None,
        };

        let Some((left, right)) = operands else {
            return (!matches!(op, BinOp::Add | BinOp::Sub)).then_some(self.int);
        };
        let nulls = (
            self.is_null_pointer_constant(lhs),
            self.is_null_pointer_constant(rhs),
        );
        if let Some(Err(refused)) = self.binary_operable(ast, op, left, right, nulls) {
            self.report_operands(ast, op, refused, (lhs, left), (rhs, right), diagnostics);
            return None;
        }

        match op {
            BinOp::Add | BinOp::Sub => self.additive(ast, op, left, right),
            // Everything else this compiler reads is arithmetic on operands
            // the integer promotions make `int`, or a comparison, which
            // 6.5.8 p6 and 6.5.9 p3 make `int`, or `&&` and `||`, which
            // 6.5.13 p3 and 6.5.14 p3 make `int`.
            BinOp::Mul
            | BinOp::Div
            | BinOp::Rem
            | BinOp::Shl
            | BinOp::Shr
            | BinOp::Lt
            | BinOp::Gt
            | BinOp::Le
            | BinOp::Ge
            | BinOp::Eq
            | BinOp::Ne
            | BinOp::BitAnd
            | BinOp::BitXor
            | BinOp::BitOr
            | BinOp::LogAnd
            | BinOp::LogOr => Some(self.int),
        }
    }

    /// C17 6.5.6, which is the one place an operand's type decides the
    /// result's. `lhs` and `rhs` are the operands' types after
    /// [`Checker::decayed`].
    ///
    /// Everything below turns on whether an operand is a pointer, and
    /// answering `int` for a type nobody knows is how a report about a program
    /// nobody understood reaches a user: before an untyped operand made this
    /// `None`, `p = (1 ? p : v) + 1` was rejected on the strength of a guess.
    /// [`Checker::binary`] is where that operand is turned away now.
    fn additive(&self, ast: &Ast, op: BinOp, lhs: TypeId, rhs: TypeId) -> Option<TypeId> {
        let pointers = (
            matches!(ast.ty(lhs), Type::Pointer(_)),
            matches!(ast.ty(rhs), Type::Pointer(_)),
        );

        match pointers {
            // p9 makes a pointer minus a pointer `ptrdiff_t`, which this
            // compiler has no name for. Two pointers added, which p2 forbids,
            // are refused by `binary_operable` before they reach here, and
            // the arm answers the same for them.
            (true, true) => None,
            // p2 and p3 both allow the pointer on the left.
            (true, false) => Some(lhs),
            // p3 allows it only there: `i - p` is a constraint violation,
            // refused by `binary_operable` before it reaches here, and no type
            // if it did.
            (false, true) if op == BinOp::Sub => None,
            // p2 lets an addition be written the other way round.
            (false, true) => Some(rhs),
            (false, false) => Some(self.int),
        }
    }

    /// Whether a binary operator's operands meet the constraints of its own
    /// clause, C17 6.5.5 p2 to 6.5.14 p2, where the operands are a `left` and
    /// a `right` after [`Checker::decayed`] and `nulls` says which of them is
    /// a null pointer constant.
    ///
    /// Every arithmetic type this compiler has is an integer type, so the
    /// arms that want an integer and the arms that want an arithmetic type
    /// agree today; they are kept apart because a floating type would make
    /// them differ, and this is where it would have to.
    ///
    /// A refusal says why, because a pointer to `void` is refused by `+` for
    /// what it points to and not for being a pointer, and a report that does
    /// not say so contradicts what its reader knows about `+`.
    ///
    /// `None` where this does not answer. An array or a function never
    /// arrives, because `decayed` converted it; one that did would be a
    /// conversion missed, and answering it would be a report about ordinary C.
    fn binary_operable(
        &self,
        ast: &Ast,
        op: BinOp,
        left: TypeId,
        right: TypeId,
        (left_is_null, right_is_null): (bool, bool),
    ) -> Option<Result<(), Refused>> {
        let (Some(left), Some(right)) = (OperandClass::of(ast, left), OperandClass::of(ast, right))
        else {
            return None;
        };

        match op {
            // 6.5.5 p2.
            BinOp::Mul | BinOp::Div => Some(pairing(matches!(
                (left, right),
                (OperandClass::Arithmetic, OperandClass::Arithmetic)
            ))),
            // 6.5.5 p2 for `%`, 6.5.7 p2 and 6.5.10 p2 to 6.5.12 p2.
            BinOp::Rem | BinOp::Shl | BinOp::Shr | BinOp::BitAnd | BinOp::BitXor | BinOp::BitOr => {
                Some(pairing(matches!(
                    (left, right),
                    (OperandClass::Arithmetic, OperandClass::Arithmetic)
                )))
            }
            // 6.5.6 p2.
            BinOp::Add => match (left, right) {
                (OperandClass::Arithmetic, OperandClass::Arithmetic) => Some(Ok(())),
                (OperandClass::Pointer(pointee), OperandClass::Arithmetic) => {
                    Some(pointee_steppable(ast, pointee, true))
                }
                (OperandClass::Arithmetic, OperandClass::Pointer(pointee)) => {
                    Some(pointee_steppable(ast, pointee, false))
                }
                _ => Some(pairing(false)),
            },
            // 6.5.6 p3, which allows the pointer only on the left.
            BinOp::Sub => match (left, right) {
                (OperandClass::Arithmetic, OperandClass::Arithmetic) => Some(Ok(())),
                (OperandClass::Pointer(pointee), OperandClass::Arithmetic) => {
                    Some(pointee_steppable(ast, pointee, true))
                }
                (OperandClass::Pointer(left), OperandClass::Pointer(right)) => {
                    // Both pointees are asked: `int[3]` is compatible with
                    // `int[]`, and only one of them is complete.
                    if ast.compatible(left, right) {
                        Some(
                            pointee_steppable(ast, left, true)
                                .and(pointee_steppable(ast, right, false)),
                        )
                    } else {
                        Some(pairing(false))
                    }
                }
                _ => Some(pairing(false)),
            },
            // 6.5.8 p2: an object type, so two pointers to functions are
            // refused however alike they are, and an integer is refused beside
            // a pointer even when it is zero.
            BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => Some(pairing(match (left, right) {
                (OperandClass::Arithmetic, OperandClass::Arithmetic) => true,
                (OperandClass::Pointer(left), OperandClass::Pointer(right)) => {
                    ast.compatible(left, right) && !matches!(ast.ty(left), Type::Function { .. })
                }
                _ => false,
            })),
            // 6.5.9 p2. The `void *` case wants an object type on the other
            // side, which C17 6.2.5 p1 makes every type but a function.
            BinOp::Eq | BinOp::Ne => Some(pairing(match (left, right) {
                (OperandClass::Arithmetic, OperandClass::Arithmetic) => true,
                (OperandClass::Pointer(left), OperandClass::Pointer(right)) => {
                    ast.compatible(left, right)
                        || is_void_beside_an_object(ast, left, right)
                        || is_void_beside_an_object(ast, right, left)
                }
                (OperandClass::Pointer(_), OperandClass::Arithmetic) => right_is_null,
                (OperandClass::Arithmetic, OperandClass::Pointer(_)) => left_is_null,
                _ => false,
            })),
            // 6.5.13 p2 and 6.5.14 p2: each operand a scalar, whatever the
            // other is.
            BinOp::LogAnd | BinOp::LogOr => Some(pairing(
                !matches!(left, OperandClass::Void) && !matches!(right, OperandClass::Void),
            )),
        }
    }

    /// C17 6.5.15 p3 to p6: whether a conditional's arms are a pair C allows,
    /// and the type it gives them, after [`Checker::decayed`] as `binary`
    /// reads its operands.
    ///
    /// Two arithmetic arms are `int`, the usual arithmetic conversions of
    /// every pair this compiler has. Two `void` arms are `void`. Two pointers
    /// to compatible types are the first arm's type: p6 asks for their
    /// composite, which is the same type only where the two are written
    /// alike, and `int ()` beside `int (int)` or `int (*)[]` beside
    /// `int (*)[3]` gets whichever came first. A pointer to an object type
    /// beside a pointer to `void` is the pointer to `void`, and a pointer
    /// beside a null pointer constant is itself. Any other pair is refused as
    /// `OPERANDS`, in `binary`'s words, and has no type, as a refused binary
    /// operation has none, a pointer to a function beside a pointer to `void`
    /// among them. An arm with no type makes no type in silence: either
    /// something reported it, or it is `p - q`, which this compiler has no
    /// type for.
    ///
    /// Refused whether or not the value is used: `c ? h() : 1;` breaks p3 as
    /// a statement too, which `clang` accepts unless asked to be pedantic.
    fn conditional(
        &mut self,
        ast: &mut Ast,
        then: ExprId,
        otherwise: ExprId,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<TypeId> {
        let left = self.decayed(ast, self.types[then.index()]?);
        let right = self.decayed(ast, self.types[otherwise.index()]?);
        let nulls = (
            self.is_null_pointer_constant(then),
            self.is_null_pointer_constant(otherwise),
        );

        let paired = match (ast.ty(left), ast.ty(right)) {
            (Type::Int | Type::Char, Type::Int | Type::Char) => Some(self.int),
            (Type::Void, Type::Void) => Some(left),
            (Type::Pointer(left_pointee), Type::Pointer(right_pointee)) => {
                // p3 pairs a pointer to `void` only with a pointer to an
                // object type, which a function is not.
                let left_void = matches!(ast.ty(*left_pointee), Type::Void);
                let right_void = matches!(ast.ty(*right_pointee), Type::Void);
                let left_function = matches!(ast.ty(*left_pointee), Type::Function { .. });
                let right_function = matches!(ast.ty(*right_pointee), Type::Function { .. });
                if ast.compatible(left, right) || (left_void && !right_function) {
                    Some(left)
                } else if right_void && !left_function {
                    Some(right)
                } else {
                    None
                }
            }
            (Type::Pointer(_), Type::Int | Type::Char) => nulls.1.then_some(left),
            (Type::Int | Type::Char, Type::Pointer(_)) => nulls.0.then_some(right),
            // `decayed` answers neither an array nor a function; each would
            // be the pointer it decays to if it did.
            (Type::Int | Type::Char | Type::Pointer(_), Type::Void)
            | (Type::Void, Type::Int | Type::Char | Type::Pointer(_))
            | (Type::Array { .. } | Type::Function { .. }, _)
            | (_, Type::Array { .. } | Type::Function { .. }) => None,
        };

        if paired.is_none() {
            let left_spelled = self.spelled(ast, left);
            let right_spelled = self.spelled(ast, right);
            diagnostics.report(
                Diagnostic::error(format!(
                    "`?:` cannot take `{left_spelled}` and `{right_spelled}`"
                ))
                .with_code(OPERANDS)
                .with_label(Label::primary(
                    ast.expr(then).span(),
                    format!("this is `{left_spelled}`"),
                ))
                .with_label(Label::secondary(
                    ast.expr(otherwise).span(),
                    format!("this is `{right_spelled}`"),
                )),
            );
        }
        paired
    }

    /// Report a binary operator given operands [`Checker::binary_operable`]
    /// refused. Each operand is its expression and its type after
    /// [`Checker::decayed`], which is what is spelled, because it is what the
    /// operator was given: `a * 1` on an `int[2]` says `int *`.
    ///
    /// The primary label is the operand the rule refuses, as in
    /// [`Checker::compound_assignment`]. The left one where the operator takes
    /// its type in no pairing at all, and otherwise the right one, read
    /// against the left the way `assignment` reads a value against its place:
    /// in `p == 1` neither operand is wrong alone, and `1` is what does not
    /// fit beside `p`. A pointer refused for what it points to is wrong alone,
    /// whichever side it is on, and the note says why.
    fn report_operands(
        &self,
        ast: &Ast,
        op: BinOp,
        refused: Refused,
        (lhs, left): (ExprId, TypeId),
        (rhs, right): (ExprId, TypeId),
        diagnostics: &mut DiagnosticSink,
    ) {
        let left_spelled = self.spelled(ast, left);
        let right_spelled = self.spelled(ast, right);
        let left_is_refused = match refused {
            Refused::Pointee { on_left, .. } => on_left,
            Refused::Pairing => match ast.ty(left) {
                Type::Void => true,
                Type::Pointer(_) => !takes_a_pointer(op),
                Type::Int | Type::Char | Type::Array { .. } | Type::Function { .. } => false,
            },
        };
        let (primary, secondary) = if left_is_refused {
            ((lhs, &left_spelled), (rhs, &right_spelled))
        } else {
            ((rhs, &right_spelled), (lhs, &left_spelled))
        };

        let mut diagnostic = Diagnostic::error(format!(
            "`{}` cannot take `{left_spelled}` and `{right_spelled}`",
            op.as_str()
        ))
        .with_code(OPERANDS)
        .with_label(Label::primary(
            ast.expr(primary.0).span(),
            format!("this is `{}`", primary.1),
        ))
        .with_label(Label::secondary(
            ast.expr(secondary.0).span(),
            format!("this is `{}`", secondary.1),
        ));
        if let Refused::Pointee { why, .. } = refused {
            // p2 is the addition's paragraph and p3 the subtraction's, and
            // each says "complete object type" for itself.
            let clause = if op == BinOp::Sub {
                "C17 6.5.6 p3"
            } else {
                "C17 6.5.6 p2"
            };
            diagnostic = diagnostic.with_note(stepping_note(why, clause));
        }

        diagnostics.report(diagnostic);
    }

    /// The type an operand has after the conversions C17 6.3.2.1 makes.
    ///
    /// p3 turns an array into a pointer to its first element and p4 turns a
    /// function into a pointer to itself, before any operator sees either. A
    /// stage that skips this reads `a + 1` as arithmetic on something that is
    /// not a pointer and answers `int`, and `p = a + 1` is then a diagnostic
    /// about ordinary C. The pointer it makes is a type nobody wrote, so it is
    /// pushed.
    ///
    /// Only [`Checker::binary`], [`Checker::subscript`] and
    /// [`Checker::conditional`] ask. `assignable`
    /// deliberately does not:
    /// an array source there is silence today, and converting it would add a
    /// check this issue was not asked for rather than remove a false one.
    fn decayed(&mut self, ast: &mut Ast, ty: TypeId) -> TypeId {
        match ast.ty(ty) {
            Type::Array { element, .. } => {
                let element = *element;
                ast.push_type(Type::Pointer(element))
            }
            Type::Function { .. } => ast.push_type(Type::Pointer(ty)),
            Type::Int | Type::Char | Type::Void | Type::Pointer(_) => ty,
        }
    }

    /// C17 6.5.16.1 p1, for a plain `=`.
    fn assignment(
        &mut self,
        ast: &Ast,
        place: ExprId,
        value: ExprId,
        diagnostics: &mut DiagnosticSink,
    ) {
        let (Some(target), Some(source)) = (self.types[place.index()], self.types[value.index()])
        else {
            return;
        };

        if self.assignable(ast, target, source, self.is_null_pointer_constant(value)) != Some(false)
        {
            return;
        }

        diagnostics.report(
            Diagnostic::error(format!(
                "cannot assign `{}` to `{}`",
                self.spelled(ast, source),
                self.spelled(ast, target)
            ))
            .with_code(MISMATCH)
            .with_label(Label::primary(
                ast.expr(value).span(),
                format!("this is `{}`", self.spelled(ast, source)),
            ))
            .with_label(Label::secondary(
                ast.expr(place).span(),
                format!("this holds `{}`", self.spelled(ast, target)),
            )),
        );
    }

    /// C17 6.5.16.2, for a compound assignment.
    ///
    /// The primary label is the operand the rule refuses: the value when it
    /// is not arithmetic, and the place otherwise. That is exact while `int`
    /// and `char` are the only arithmetic types, because a refused pair with
    /// an arithmetic value is then always one whose place is what p1 or p2
    /// does not take.
    fn compound_assignment(
        &mut self,
        ast: &Ast,
        op: BinOp,
        place: ExprId,
        value: ExprId,
        diagnostics: &mut DiagnosticSink,
    ) {
        let (Some(target), Some(source)) = (self.types[place.index()], self.types[value.index()])
        else {
            return;
        };

        if self.compound_assignable(ast, op, target, source) != Some(false) {
            return;
        }

        // p1 takes a pointer with an integer for `+=` and `-=`, so a refused
        // pair of that shape was refused for what the pointer points to, and
        // is the one refusal that needs saying why.
        let why = match (ast.ty(target), ast.ty(source)) {
            (Type::Pointer(pointee), Type::Int | Type::Char)
                if matches!(op, BinOp::Add | BinOp::Sub) =>
            {
                unsteppable(ast, *pointee)
            }
            _ => None,
        };

        let target_spelled = self.spelled(ast, target);
        let source_spelled = self.spelled(ast, source);
        let (primary, secondary) = if !matches!(ast.ty(source), Type::Int | Type::Char) {
            ((value, &source_spelled), (place, &target_spelled))
        } else {
            ((place, &target_spelled), (value, &source_spelled))
        };

        let mut diagnostic = Diagnostic::error(format!(
            "`{}=` cannot take `{target_spelled}` and `{source_spelled}`",
            op.as_str()
        ))
        .with_code(OPERANDS)
        .with_label(Label::primary(
            ast.expr(primary.0).span(),
            format!("this is `{}`", primary.1),
        ))
        .with_label(Label::secondary(
            ast.expr(secondary.0).span(),
            format!("this is `{}`", secondary.1),
        ));
        if let Some(why) = why {
            diagnostic = diagnostic.with_note(stepping_note(why, "C17 6.5.16.2 p1"));
        }

        diagnostics.report(diagnostic);
    }

    /// Whether `place op= value` meets C17 6.5.16.2's constraints, where the
    /// place is a `target` and the value a `source`.
    ///
    /// p1 lets `+=` and `-=` take a pointer to a complete object type on the
    /// left and an integer on the right, and p2 wants both operands of every
    /// other compound assignment arithmetic and "consistent with" the binary
    /// operator's own constraints, which for `%=`, the shifts and the bitwise
    /// operators means integer. Every arithmetic type this compiler has is an
    /// integer type, so the two readings of p2 agree today; a floating type
    /// would make them differ, and this is where it would have to.
    ///
    /// `void` is neither arithmetic nor a pointer, so `*v += 1` on a `void *`
    /// breaks p1 or p2 whichever operator it is. So does an array or a
    /// function as the value, which 6.3.2.1 p3 and p4 turn into a pointer
    /// before either paragraph looks at it.
    ///
    /// A pointer to `void`, to a function or to an array of unknown length is
    /// not a pointer to a complete object type, so `v += 1` breaks p1, and
    /// `v + 1` breaks 6.5.6 p2 by the same sentence; [`unsteppable`] answers
    /// both.
    ///
    /// `None` where this does not answer. An array or a function as the place
    /// is refused by 6.5.16 p2, which wants a modifiable lvalue, and that is
    /// not this rule; `assignable` leaves it for the same reason.
    fn compound_assignable(
        &self,
        ast: &Ast,
        op: BinOp,
        target: TypeId,
        source: TypeId,
    ) -> Option<bool> {
        match (ast.ty(target), ast.ty(source)) {
            (Type::Int | Type::Char, Type::Int | Type::Char) => Some(true),
            (Type::Pointer(pointee), Type::Int | Type::Char) => match op {
                BinOp::Add | BinOp::Sub => Some(unsteppable(ast, *pointee).is_none()),
                // A comparison and a logical operator have no compound
                // form, so the parser never builds one here. They are listed
                // rather than caught by a wildcard so that a new operator
                // has to be answered for.
                BinOp::Mul
                | BinOp::Div
                | BinOp::Rem
                | BinOp::Shl
                | BinOp::Shr
                | BinOp::Lt
                | BinOp::Gt
                | BinOp::Le
                | BinOp::Ge
                | BinOp::Eq
                | BinOp::Ne
                | BinOp::BitAnd
                | BinOp::BitXor
                | BinOp::BitOr
                | BinOp::LogAnd
                | BinOp::LogOr => Some(false),
            },
            (Type::Array { .. } | Type::Function { .. }, _) => None,
            (
                Type::Int | Type::Char | Type::Pointer(_) | Type::Void,
                Type::Pointer(_) | Type::Void | Type::Array { .. } | Type::Function { .. },
            )
            | (Type::Void, Type::Int | Type::Char) => Some(false),
        }
    }

    /// A condition that is not a scalar, refused under `OPERANDS` in the
    /// words `!` and `&&` use, and whether it was. `word` is what read it:
    /// `if`, `while`, `for` or `?:`.
    ///
    /// Only `void` is refused, which is right only because every other type
    /// this compiler has is a scalar or becomes one: an array or a function
    /// is a pointer by 6.3.2.1 p3 and p4 before C reads it as a condition.
    /// Nothing here converts it; it is not `void`, so it passes. A structure
    /// type, when one lands, is not a scalar and has to be refused here
    /// too.
    fn refuse_a_void_condition(
        &self,
        ast: &Ast,
        word: &str,
        condition: ExprId,
        ty: TypeId,
        diagnostics: &mut DiagnosticSink,
    ) -> bool {
        if !matches!(ast.ty(ty), Type::Void) {
            return false;
        }
        diagnostics.report(
            Diagnostic::error(format!("`{word}` cannot take `void`"))
                .with_code(OPERANDS)
                .with_label(Label::primary(ast.expr(condition).span(), "this is `void`")),
        );
        true
    }

    /// What each value the statement walk found has to satisfy, once it is
    /// typed. For a `return` and an initializer that is C17 6.8.6.4 p3 and
    /// 6.7.9 p11, each 6.5.16.1 p1 with another place: the return type, or
    /// the declarator being initialized. For a statement's condition it is
    /// [`Checker::refuse_a_void_condition`].
    fn check_received(&mut self, ast: &Ast, value: ExprId, diagnostics: &mut DiagnosticSink) {
        let Some(receiving) = self.receivers.get(&value).copied() else {
            return;
        };
        if let Receiving::ReturnFromVoid(returning) = receiving {
            diagnostics.report(
                Diagnostic::error("`return` with an expression in a function returning `void`")
                    .with_code(RETURN_SHAPE)
                    .with_label(Label::primary(
                        ast.expr(value).span(),
                        "this is an expression",
                    ))
                    .with_label(Label::secondary(
                        returning.name,
                        "declared to return `void`",
                    )),
            );
            return;
        }
        let Some(source) = self.types[value.index()] else {
            return;
        };
        let target = match receiving {
            Receiving::Return(returning) | Receiving::ReturnFromVoid(returning) => returning.ty,
            Receiving::Initializer { ty, .. } => ty,
            Receiving::Condition { keyword } => {
                self.refuse_a_void_condition(ast, keyword, value, source, diagnostics);
                return;
            }
        };

        if self.assignable(ast, target, source, self.is_null_pointer_constant(value)) != Some(false)
        {
            return;
        }

        let source_spelled = self.spelled(ast, source);
        let target_spelled = self.spelled(ast, target);
        let (message, place, place_label) = match receiving {
            Receiving::Return(returning) | Receiving::ReturnFromVoid(returning) => (
                format!(
                    "cannot return `{source_spelled}` from a function returning `{target_spelled}`"
                ),
                returning.name,
                format!("declared to return `{target_spelled}`"),
            ),
            // The words of `assignment`'s place label, because the name is
            // the place. The message is the declaration's own, so that it
            // describes what was written rather than an `=` expression.
            Receiving::Initializer { name, .. } => (
                format!("cannot initialize `{target_spelled}` with `{source_spelled}`"),
                name,
                format!("this holds `{target_spelled}`"),
            ),
            // Answered where the target was asked for, above.
            Receiving::Condition { .. } => return,
        };

        diagnostics.report(
            Diagnostic::error(message)
                .with_code(MISMATCH)
                .with_label(Label::primary(
                    ast.expr(value).span(),
                    format!("this is `{source_spelled}`"),
                ))
                .with_label(Label::secondary(place, place_label)),
        );
    }

    /// C17 6.5.2.2 p2: the number of arguments against the number of
    /// parameters, and where they agree, each argument against its parameter
    /// in `check_argument`.
    ///
    /// A callee of pointer-to-function type is asked as the function it
    /// points at. 6.5.2.2 p1 makes every callee a pointer to a function, a
    /// designator becoming one by 6.3.2.1 p4, so `fp(x)` and `(&g)(x)` are
    /// held to the same constraint as `g(x)`. Any other callee breaks p1 and
    /// is reported as `OPERANDS`, with no type for the call, because there is
    /// no return type to give it.
    ///
    /// Only where the callee's type includes a prototype, which is what that
    /// paragraph conditions the constraint on. `()` is never one: 6.7.6.3 p14
    /// makes it an empty identifier list, and a call to a function declared
    /// that way is p6's undefined behaviour rather than a constraint anybody
    /// has to diagnose. `clang` warns there and errors here, and this reports
    /// the half C calls a constraint.
    fn call(
        &mut self,
        ast: &Ast,
        call: ExprId,
        callee: ExprId,
        arguments: &[ExprId],
        diagnostics: &mut DiagnosticSink,
    ) -> Option<TypeId> {
        let mut called = self.types[callee.index()]?;
        if let Type::Pointer(pointee) = ast.ty(called) {
            called = *pointee;
        }
        let Type::Function {
            returns,
            parameters,
        } = ast.ty(called)
        else {
            // 6.5.2.2 p1 again: what is called is a pointer to a function.
            // Spelled as the callee was declared, so `pp` with
            // `int (**pp)(int)` is named `int (**)(int)` rather than the
            // type one level in.
            let spelled = self.spelled(ast, self.types[callee.index()]?);
            diagnostics.report(
                Diagnostic::error(format!("cannot call `{spelled}`"))
                    .with_code(OPERANDS)
                    .with_label(Label::primary(
                        ast.expr(callee).span(),
                        format!("this is `{spelled}`"),
                    )),
            );
            return None;
        };
        let returns = *returns;

        let Parameters::Prototype(parameters) = parameters else {
            return Some(returns);
        };

        // A `match` on the ordering rather than two comparisons: the worst
        // defect this project has had is a gate written as two of those with a
        // case answered by neither.
        let (what, expected, found) = match arguments.len().cmp(&parameters.len()) {
            Ordering::Less => ("too few", parameters.len(), arguments.len()),
            Ordering::Greater => ("too many", parameters.len(), arguments.len()),
            Ordering::Equal => {
                // Unchecked, a pointer passed for an `int` parameter arrived
                // in the callee's body as a local declared `int`, which breaks
                // what the memory check needs of a local that may hold an
                // allocation. See ADR-0030.
                for (&argument, parameter) in arguments.iter().zip(parameters) {
                    let declared = parameter.name.unwrap_or(parameter.span);
                    self.check_argument(ast, argument, parameter.ty, declared, diagnostics);
                }
                return Some(returns);
            }
        };

        let mut diagnostic = Diagnostic::error(format!(
            "{what} arguments: expected {expected}, found {found}"
        ))
        .with_code(ARGUMENTS)
        .with_label(Label::primary(ast.expr(call).span(), "this call"));

        // Only a callee that is a name has a declaration to point at, so
        // `(*g)(1, 2)` and `(&g)(1, 2)` are reported without one.
        if let Some(binding) = self.resolution.resolved(callee) {
            diagnostic = diagnostic.with_label(Label::secondary(
                self.resolution.binding(binding).name,
                format!("declared with {expected} parameters here"),
            ));
        }

        diagnostics.report(diagnostic);
        Some(returns)
    }

    /// One argument against its parameter, C17 6.5.2.2 p2, asked of
    /// `assignable` as an initializer is and refused with `MISMATCH`.
    ///
    /// Not a `Receiving` variant beside the initializer and the `return`:
    /// those are collected from statements before anything is typed, and an
    /// argument's target is only known once the callee is. `None` from
    /// `assignable`, an array or a function on either side, is passed over
    /// as it is there.
    ///
    /// The primary label is the initializer's; the message and the secondary
    /// label are this one's own, and the secondary points at the parameter's
    /// name, or at its declaration where it has none.
    fn check_argument(
        &self,
        ast: &Ast,
        argument: ExprId,
        parameter: TypeId,
        declared: Span,
        diagnostics: &mut DiagnosticSink,
    ) {
        let Some(source) = self.types[argument.index()] else {
            return;
        };
        let null = self.is_null_pointer_constant(argument);
        if self.assignable(ast, parameter, source, null) != Some(false) {
            return;
        }
        let source_spelled = self.spelled(ast, source);
        let target_spelled = self.spelled(ast, parameter);
        diagnostics.report(
            Diagnostic::error(format!(
                "cannot pass `{source_spelled}` to a parameter of type `{target_spelled}`"
            ))
            .with_code(MISMATCH)
            .with_label(Label::primary(
                ast.expr(argument).span(),
                format!("this is `{source_spelled}`"),
            ))
            .with_label(Label::secondary(
                declared,
                format!("declared `{target_spelled}` here"),
            )),
        );
    }

    /// Whether a value of type `source` may be assigned to a place of type
    /// `target`, C17 6.5.16.1 p1.
    ///
    /// `None` where this stage does not answer: a `void` target, or an array
    /// or a function on either side, is a different constraint, and 6.5.16
    /// p2's requirement that the place be a modifiable lvalue is another.
    /// Answering `false` for them would be reporting them badly. A `void`
    /// source onto an object type is answered, `false`, since a `void`
    /// expression has no value to be assigned.
    fn assignable(
        &self,
        ast: &Ast,
        target: TypeId,
        source: TypeId,
        source_is_null: bool,
    ) -> Option<bool> {
        match (ast.ty(target), ast.ty(source)) {
            // Both arithmetic: the value is converted, and `char c; c = 1;` is
            // ordinary C rather than a mismatch.
            (Type::Int | Type::Char, Type::Int | Type::Char) => Some(true),
            // p1's pointer case, with the `void *` half of it. Qualifiers are
            // a thing this compiler cannot yet write.
            (Type::Pointer(target_pointee), Type::Pointer(source_pointee)) => Some(
                ast.compatible(target, source)
                    || matches!(ast.ty(*target_pointee), Type::Void)
                    || matches!(ast.ty(*source_pointee), Type::Void),
            ),
            // 6.3.2.3 p3: only a null pointer constant may be assigned to a
            // pointer from an integer.
            (Type::Pointer(_), Type::Int | Type::Char) => Some(source_is_null),
            (Type::Int | Type::Char, Type::Pointer(_)) => Some(false),
            // 6.5.16.1 p1 lists what may be assigned to an object, and a
            // `void` expression has no value to be any of it; 6.8.6.4 p3 and
            // 6.7.9 p11 hold a returned value and an initializer to the same
            // list. Answering `None` here let `return h();` from a function
            // returning `int` build in silence.
            (Type::Int | Type::Char | Type::Pointer(_), Type::Void) => Some(false),
            (Type::Void | Type::Array { .. } | Type::Function { .. }, _)
            | (_, Type::Array { .. } | Type::Function { .. }) => None,
        }
    }

    /// A type spelled for a message rather than for the artifact.
    ///
    /// `spell_type` splices an array length's own source text, so a spelled
    /// type can carry whatever the file wrote between the brackets, and a
    /// comment can carry a newline. `render.rs::shown` keeps a newline on
    /// purpose, because a message that runs to two lines is ordinary, so
    /// without this a source file could print a line of its own invention
    /// into this compiler's report. Every run of whitespace becomes one
    /// space, which leaves `int (*)(int, char)` spelled exactly as it was.
    fn spelled(&self, ast: &Ast, ty: TypeId) -> String {
        spell_type(self.sources, ast, ty)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Whether `value` is a null pointer constant, C17 6.3.2.3 p3.
    ///
    /// That paragraph says "an integer constant expression with the value 0",
    /// and [`Checker::evaluate`] is what gives one its value, so `0`, `0x0`,
    /// `-0` and `1 - 1` are null pointer constants and `i - i` is not.
    ///
    /// Read off the value rather than off the text: `constant` below is the
    /// one place a spelling is turned into a number, so `0x0`, `00` and `0u`
    /// are zero here because they are zero there. A second reading of the
    /// text would be a second answer to drift from the first.
    fn is_null_pointer_constant(&self, value: ExprId) -> bool {
        self.values[value.index()] == Some(0)
    }

    /// The value of an integer constant expression, C17 6.6 p6, or `None`.
    ///
    /// Asked of each expression once its operands have theirs, which the
    /// order `check` walks in gives: a parent is pushed after its operands.
    /// An expression has a value only when every operand has one and its
    /// operator is one 6.6 lets a constant expression hold, so an identifier,
    /// an assignment, a call, `++`, `*`, `&`, a subscript and a comma (6.6
    /// p3) have none, and nor does anything built on them.
    ///
    /// Computed at `int`, the only integer type with a range here, and given
    /// no value where C gives the operation no defined result: a result
    /// outside `int` (6.6 p4 for a constant, 6.5 p5 at run time), a division
    /// or remainder by zero (6.5.5 p5), a shift by a negative amount or by the
    /// width or more, and a left shift of a negative value or out of range.
    /// Such an expression has no value and nothing is said about it here;
    /// `check` marks it in `Checker::undefined`, which is what reports an
    /// array length at file scope that overflows. An operand C does not
    /// evaluate, the right of a deciding `&&` or `||` and the arm a `?:` does
    /// not take, is not asked for a value at all, only to be a constant. A null pointer constant that
    /// overflows is an integer given to a pointer.
    ///
    /// A right shift of a negative value is implementation-defined (6.5.7
    /// p5), not undefined, and this implementation has chosen: the backend
    /// writes `ashr` and the interpreter shifts arithmetically, so `-8 >> 1`
    /// is `-4` here as it is when the program runs.
    fn evaluate(&self, ast: &Ast, id: ExprId) -> Option<i128> {
        let value = |id: ExprId| self.values[id.index()];
        // A constant expression in shape, with a value or without one. What
        // an operand C does not evaluate has to be (6.6 p3, p6), since it is
        // never asked for a value.
        let known = |id: ExprId| self.values[id.index()].is_some() || self.undefined[id.index()];
        let int = self.int_range;
        let fits = |result: i128| int.holds(result).then_some(result);
        match *ast.expr(id) {
            Expr::Unary { op, operand, .. } => {
                let operand = value(operand)?;
                match op {
                    UnOp::Plus => Some(operand),
                    UnOp::Minus => fits(-operand),
                    UnOp::BitNot => Some(!operand),
                    UnOp::Not => Some(i128::from(operand == 0)),
                    UnOp::Deref
                    | UnOp::AddrOf
                    | UnOp::PreInc
                    | UnOp::PreDec
                    | UnOp::PostInc
                    | UnOp::PostDec => None,
                }
            }
            Expr::Binary { op, lhs, rhs, .. } => {
                let left = value(lhs)?;
                // 6.5.13 p4 and 6.5.14 p4: where the left operand decides,
                // the right is not evaluated, so `1 || 1 / 0` is 1. It still
                // has to be a constant in shape, so `1 || x` has no value.
                let decided = match op {
                    BinOp::LogAnd => (left == 0).then_some(0),
                    BinOp::LogOr => (left != 0).then_some(1),
                    _ => None,
                };
                if let Some(answer) = decided {
                    return known(rhs).then_some(answer);
                }
                let (lhs, rhs) = (left, value(rhs)?);
                let truth = |holds: bool| Some(i128::from(holds));
                let width = i128::from(int.bits());
                match op {
                    BinOp::Mul => fits(lhs * rhs),
                    BinOp::Div => (rhs != 0).then(|| lhs / rhs).and_then(fits),
                    // 6.5.5 p6 defines `a % b` only where `a / b` is
                    // representable, so `INT_MIN % -1` has no value though
                    // its remainder, zero, would fit.
                    BinOp::Rem => (rhs != 0)
                        .then(|| lhs % rhs)
                        .and_then(|r| fits(lhs / rhs).map(|_| r)),
                    BinOp::Add => fits(lhs + rhs),
                    BinOp::Sub => fits(lhs - rhs),
                    BinOp::Shl => ((0..width).contains(&rhs) && lhs >= 0)
                        .then(|| lhs << rhs)
                        .and_then(fits),
                    // Arithmetic, as the backend's `ashr` is; see above.
                    BinOp::Shr => (0..width).contains(&rhs).then(|| lhs >> rhs),
                    BinOp::Lt => truth(lhs < rhs),
                    BinOp::Gt => truth(lhs > rhs),
                    BinOp::Le => truth(lhs <= rhs),
                    BinOp::Ge => truth(lhs >= rhs),
                    BinOp::Eq => truth(lhs == rhs),
                    BinOp::Ne => truth(lhs != rhs),
                    BinOp::BitAnd => Some(lhs & rhs),
                    BinOp::BitXor => Some(lhs ^ rhs),
                    BinOp::BitOr => Some(lhs | rhs),
                    BinOp::LogAnd => truth(lhs != 0 && rhs != 0),
                    BinOp::LogOr => truth(lhs != 0 || rhs != 0),
                }
            }
            // 6.5.15 p4 evaluates one arm, so only that one is asked for a
            // value: `1 ? 2 : 1 / 0` is 2. The other is not evaluated and
            // need only be a constant in shape (6.6 p6), so `1 ? 2 : x` has
            // no value.
            Expr::Conditional {
                condition,
                then,
                otherwise,
                ..
            } => {
                let (taken, skipped) = if value(condition)? != 0 {
                    (then, otherwise)
                } else {
                    (otherwise, then)
                };
                if !known(skipped) {
                    return None;
                }
                value(taken)
            }
            Expr::Number { .. }
            | Expr::Identifier { .. }
            | Expr::Assign { .. }
            | Expr::Comma { .. }
            | Expr::Subscript { .. }
            | Expr::Call { .. }
            | Expr::Error { .. } => None,
        }
    }

    /// The type and the value of an integer constant, C17 6.4.4.1.
    ///
    /// Both at once, because `read_number` answers both from one pass over
    /// the spelling and because a constant with no type has no value either.
    ///
    /// The type is `int`, and only ever `int`. 6.4.4.1 p5 asks for the first
    /// type in a list that holds the value, and this compiler has no other
    /// integer type to offer: there is no `unsigned int` and no `long` in
    /// [`Type`] or in [`Integer`]. So a constant that p5's table would give
    /// one of those is refused rather than read as an `int`, which is what
    /// [`Reading::Suffixed`] is for: the suffix is not decoration, it decides
    /// what arithmetic on the constant means, and C computes `-6 / 3u` as an
    /// unsigned division. `docs/frontend.md` records the divergence.
    ///
    /// Nothing that reaches a diagnostic below is the file's own text, which
    /// is what every new message owes, since source text is content: the
    /// span is what points at the spelling, and the words are this
    /// compiler's.
    fn constant(
        &mut self,
        id: ExprId,
        span: Span,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<TypeId> {
        // `Reading` is matched exhaustively so that a sixth answer has to be
        // given a report rather than falling into one written for another.
        let reported = match read_number(self.sources.snippet(span)) {
            Reading::Integer(value) => {
                // `holds` takes an `i128`, and a value that does not fit one
                // does not fit `int` either, so the conversion failing is the
                // same answer as the range check failing.
                match i128::try_from(value)
                    .ok()
                    .filter(|value| self.int_range.holds(*value))
                {
                    Some(value) => {
                        self.values[id.index()] = Some(value);
                        return Some(self.int);
                    }
                    None => TOO_LARGE,
                }
            }
            Reading::Suffixed => SUFFIXED,
            Reading::TooLarge => TOO_LARGE,
            Reading::Floating => FLOATING,
            Reading::Malformed(why) => Reported {
                code: MALFORMED_CONSTANT,
                message: "this is not a constant C allows",
                label: why,
                note: "C17 6.4.4.1 p1 gives the three bases their digits and lists the suffixes, \
                       and 6.4.4.2 p1 spells a floating constant",
            },
        };

        diagnostics.report(
            Diagnostic::error(reported.message)
                .with_code(reported.code)
                .with_label(Label::primary(span, reported.label))
                .with_note(reported.note),
        );
        None
    }
}

/// Which of C17 6.2.5's classes an operand is in, as far as a binary operator
/// asks.
#[derive(Clone, Copy)]
enum OperandClass {
    /// 6.2.5 p18. Every one this compiler has is an integer type too.
    Arithmetic,
    /// 6.2.5 p20, with what it points to.
    Pointer(TypeId),
    /// 6.2.5 p19: no value, and so an operand of nothing.
    Void,
}

impl OperandClass {
    /// `None` for an array or a function, which 6.3.2.1 converts before an
    /// operator sees it, so that one arriving here is not answered.
    fn of(ast: &Ast, ty: TypeId) -> Option<Self> {
        match ast.ty(ty) {
            Type::Int | Type::Char => Some(Self::Arithmetic),
            Type::Pointer(pointee) => Some(Self::Pointer(*pointee)),
            Type::Void => Some(Self::Void),
            Type::Array { .. } | Type::Function { .. } => None,
        }
    }
}

/// Why C17 6.5.6 does not let a pointer to `pointee` take a step, or `None`
/// where it does: only a pointer to a complete object type may.
///
/// One function for `v + 1`, `v += 1`, `v++` and `v[1]`, so that the four
/// spellings of one rule cannot be given different answers: C17 6.5.2.4 p2
/// and 6.5.3.1 p2 define an increment as `+= 1`, and 6.5.2.1 p1 gives a
/// subscript the same constraint. The reason is written here rather
/// than spelled from `pointee`, because a spelled type can carry the file's
/// own bytes, which are content, and none of the three needs the type to be
/// read.
fn unsteppable(ast: &Ast, pointee: TypeId) -> Option<&'static str> {
    match ast.ty(pointee) {
        Type::Void => Some("`void` has no size"),
        Type::Function { .. } => Some("a function is not an object"),
        Type::Array { length: None, .. } => Some("an array of unknown length has no size"),
        Type::Int | Type::Char | Type::Pointer(_) | Type::Array { .. } => None,
    }
}

/// The note for a pointer refused for what it points to: why, and the
/// paragraph that says so.
fn stepping_note(why: &str, clause: &str) -> String {
    format!("a pointer steps by the size of what it points to, and {why} ({clause})")
}

/// Why [`Checker::binary_operable`] or [`Checker::subscript`] refused a pair
/// of operands.
#[derive(Clone, Copy)]
enum Refused {
    /// The operator's clause takes no such pairing of operand types.
    Pairing,
    /// C17 6.5.6 or 6.5.2.1 takes the pairing, and a pointer in it points to something
    /// it cannot step over. `on_left` says which operand, and `why` is
    /// [`unsteppable`]'s answer.
    Pointee { on_left: bool, why: &'static str },
}

/// `Ok` where the operator's clause takes the pairing of operand types, and
/// [`Refused::Pairing`] where it does not.
fn pairing(taken: bool) -> Result<(), Refused> {
    if taken { Ok(()) } else { Err(Refused::Pairing) }
}

/// `Ok` where a pointer to `pointee` can take the step 6.5.6 or 6.5.2.1 would
/// make, and [`Refused::Pointee`] where it cannot, naming the operand by
/// `on_left`.
fn pointee_steppable(ast: &Ast, pointee: TypeId, on_left: bool) -> Result<(), Refused> {
    match unsteppable(ast, pointee) {
        None => Ok(()),
        Some(why) => Err(Refused::Pointee { on_left, why }),
    }
}

/// Whether `void_side` is `void` and `other` is an object type, the `void *`
/// case of C17 6.5.9 p2.
fn is_void_beside_an_object(ast: &Ast, void_side: TypeId, other: TypeId) -> bool {
    matches!(ast.ty(void_side), Type::Void) && !matches!(ast.ty(other), Type::Function { .. })
}

/// Whether `op` takes a pointer operand in some pairing, which is what makes
/// a pointer operand refused alone rather than refused beside the other one.
///
/// `-` takes one on the right only beside another on the left, and is `true`
/// here for the left's sake: a report about `1 - p` names `p` anyway, because
/// the left is arithmetic and so not what is refused.
fn takes_a_pointer(op: BinOp) -> bool {
    match op {
        BinOp::Add
        | BinOp::Sub
        | BinOp::Lt
        | BinOp::Gt
        | BinOp::Le
        | BinOp::Ge
        | BinOp::Eq
        | BinOp::Ne
        | BinOp::LogAnd
        | BinOp::LogOr => true,
        BinOp::Mul
        | BinOp::Div
        | BinOp::Rem
        | BinOp::Shl
        | BinOp::Shr
        | BinOp::BitAnd
        | BinOp::BitXor
        | BinOp::BitOr => false,
    }
}

/// What a constant this stage could not read is reported as.
///
/// A value rather than four arguments, so that the arm that picks one is a
/// choice between whole diagnostics and cannot pair one message with another
/// one's note.
#[derive(Clone, Copy)]
struct Reported {
    code: Code,
    message: &'static str,
    label: &'static str,
    note: &'static str,
}

/// A value outside the range of the only integer type this compiler has.
const TOO_LARGE: Reported = Reported {
    code: NO_TYPE,
    message: "no integer type this compiler has can hold this constant",
    label: "this needs a type wider than `int`",
    note: "C17 6.4.4.1 p5 gives a constant the first type in a list that holds it; this compiler \
           has only `int`, and 6.4.4 p2 requires a constant's value to be in the range of its \
           type",
};

/// A constant with a fractional part or an exponent, C17 6.4.4.2.
const FLOATING: Reported = Reported {
    code: NO_TYPE,
    message: "this compiler does not read floating constants yet",
    label: "this is a floating constant",
    note: "C17 6.4.4.2 gives it `double`, `float` or `long double`, and this compiler has no \
           floating type",
};

/// A well-formed integer constant whose suffix asks for a type there is none
/// of here.
///
/// A refusal and not a silent `int`. The suffix is not decoration: 6.4.4.1 p5
/// makes `3u` an `unsigned int`, and 6.3.1.8's conversions then make `-6 / 3u`
/// an unsigned division, which is 1431655763 rather than -2. Reading it as an
/// `int` would compile that program to a signed division and report nothing,
/// which is the one thing this stage exists to stop.
const SUFFIXED: Reported = Reported {
    code: NO_TYPE,
    message: "this compiler has no type for a suffixed constant",
    label: "this suffix asks for `unsigned int`, `long` or `long long`",
    note: "C17 6.4.4.1 p5 gives a suffixed constant a type from a list this compiler has only \
           `int` from, and the suffix decides what arithmetic on it means: C computes `-6 / 3u` \
           as an unsigned division",
};

/// What reading the text of a numeric constant comes to, C17 6.4.4.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reading {
    /// The value of a constant with no suffix, which 6.4.4.1 p5 gives the
    /// first type in a list that holds it. `int` is the only entry here.
    Integer(u128),
    /// A well-formed constant whose suffix asks for a type off the end of
    /// that list. See [`SUFFIXED`].
    Suffixed,
    /// More than a `u128` holds, which no type here could.
    TooLarge,
    /// A well-formed floating constant, C17 6.4.4.2. Checked against that
    /// paragraph's grammar rather than guessed at from a `.`, because `1e`
    /// and `0x1.8` have the shape and are not constants at all.
    Floating,
    /// None of those, so 6.4.4 p2 has no type to give it. The string says
    /// which part of the spelling is wrong, and is this compiler's own words
    /// rather than any of the file's.
    Malformed(&'static str),
}

/// An octal digit that is not one.
const NOT_OCTAL: &str = "an octal constant has digits `0` to `7`";
/// A base prefix with nothing after it.
const NO_DIGITS: &str = "a hexadecimal constant needs at least one digit after the `0x`";
/// A `.` or an exponent, on something C17 6.4.4.2 p1 does not spell that way.
const NOT_FLOATING: &str = "an exponent needs a digit, and a hexadecimal floating constant needs \
                            an exponent";
/// Everything else: a digit of no base, or a suffix C does not have.
const NOT_A_DIGIT: &str = "this is not a digit of the constant's base, and not a suffix C allows";

/// Read what a numeric constant is worth, C17 6.4.4.1.
///
/// `text` is a preprocessing number as `lexer.rs::scan_number` delimits one,
/// which is wider than the set of valid constants: deciding which of these a
/// spelling is, is exactly this function's job.
///
/// Every digit test is `char::is_digit`, which is ASCII whatever the radix:
/// `to_digit` beneath it reads `0`-`9`, `a`-`z` and `A`-`Z` and nothing else.
/// That is said out loud rather than assumed, because the `char::is_*`
/// family next to it
/// answers for Unicode where C means ASCII.
fn read_number(text: &str) -> Reading {
    // 6.4.4.2 p1 gives a floating constant a `.`, or an exponent: `e`/`E` for
    // a decimal and `p`/`P` for a hexadecimal. The base has to be known before
    // the marker can be looked for, because `e` is a hexadecimal digit and
    // `0xe1` is 225.
    //
    // Having one of those is what makes a spelling *meant* as a floating
    // constant; whether it is one is `is_floating`'s question, and the two
    // are separate because the answers are blamed on different people. `1.5`
    // is a constant C gives a type and this compiler has none for; `1e` is
    // not a constant at all, and telling its author that floating constants
    // are unsupported would be a false reason for a true refusal.
    let hex = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X"));
    let exponent = if hex.is_some() {
        ['p', 'P']
    } else {
        ['e', 'E']
    };
    if text.contains('.') || hex.unwrap_or(text).contains(exponent) {
        return if is_floating(text) {
            Reading::Floating
        } else {
            Reading::Malformed(NOT_FLOATING)
        };
    }

    // 6.4.4.1 p1: the prefix decides the base, and a leading `0` with nothing
    // to prefix is the octal constant zero.
    let (digits, radix) = match hex {
        Some(digits) => (digits, 16),
        None => match text.strip_prefix('0') {
            Some(digits) => (digits, 8),
            None => (text, 10),
        },
    };

    // Whatever tail is not a digit of the base has to be a suffix, so the two
    // are one split rather than two scans. `find` answers in bytes at a
    // character boundary, which is what `split_at` needs.
    let split = digits
        .find(|c: char| !c.is_digit(radix))
        .unwrap_or(digits.len());
    let (digits, suffix) = digits.split_at(split);

    if !matches!(
        suffix,
        "" | "u"
            | "U"
            | "l"
            | "L"
            | "ll"
            | "LL"
            | "ul"
            | "uL"
            | "Ul"
            | "UL"
            | "lu"
            | "lU"
            | "Lu"
            | "LU"
            | "ull"
            | "uLL"
            | "Ull"
            | "ULL"
            | "llu"
            | "llU"
            | "LLu"
            | "LLU"
    ) {
        // `08` and `09` land here, because `8` is not an octal digit and so
        // is read as the start of a suffix. Naming the real mistake is worth
        // three lines: a leading zero on a written-out number is the way this
        // one gets made.
        return Reading::Malformed(if radix == 8 && suffix.starts_with(['8', '9']) {
            NOT_OCTAL
        } else {
            NOT_A_DIGIT
        });
    }

    if digits.is_empty() && radix != 8 {
        // `0x` alone has nothing after its prefix and is not a constant at
        // all. A leading `0` with nothing after it is the octal constant
        // zero, which falls through to the accumulation below.
        return Reading::Malformed(NO_DIGITS);
    }

    // Answered after the spelling is known to be well formed, so that `1lL`
    // is reported as the mistake it is rather than as a type this compiler
    // does not have.
    if !suffix.is_empty() {
        return Reading::Suffixed;
    }

    let mut value: u128 = 0;
    for byte in digits.bytes() {
        let digit = char::from(byte)
            .to_digit(radix)
            .expect("a digit of the base, which is what the split above left here");
        // Checked, so that nothing wraps and nothing clamps: a constant this
        // cannot accumulate is one no type here could hold, and saying so is
        // the whole point of reading it.
        value = match value
            .checked_mul(u128::from(radix))
            .and_then(|shifted| shifted.checked_add(u128::from(digit)))
        {
            Some(value) => value,
            None => return Reading::TooLarge,
        };
    }

    Reading::Integer(value)
}

/// Whether `text` is a floating constant, C17 6.4.4.2 p1.
///
/// Called only for a spelling that has a `.` or an exponent marker, which is
/// the whole of what makes one a candidate. What is left to answer is the
/// grammar, and three of its clauses are the ones a reader loses:
///
/// * an `exponent-part` is a marker, an optional sign, and **at least one**
///   digit, so `1e` and `1E+` are not constants;
/// * a `hexadecimal-floating-constant` has a `binary-exponent-part` in both
///   of its forms, so `0x1.8` is not one although `1.8` is;
/// * the exponent's digits are decimal whatever the mantissa's base, so the
///   `3` in `0x1p3` is three and not an invitation to read hexadecimal.
///
/// Measured against `clang 20.1.6 -std=c17 -pedantic-errors`, which reports
/// `1e` as "exponent has no digits" and `0x1.8` as "hexadecimal floating
/// constant requires an exponent".
fn is_floating(text: &str) -> bool {
    // 6.4.4.2 p1's `floating-suffix` is one of `f`, `F`, `l`, `L`, and there
    // is at most one. Taken off first so that what remains is the number.
    let body = text.strip_suffix(['f', 'F', 'l', 'L']).unwrap_or(text);

    let (body, radix, marker) = match body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        Some(body) => (body, 16, ['p', 'P']),
        None => (body, 10, ['e', 'E']),
    };

    let (mantissa, exponent) = match body.find(marker) {
        Some(at) => (&body[..at], Some(&body[at + 1..])),
        None => (body, None),
    };

    // The mantissa is a digit-sequence, or one with a single `.` in it.
    let mut parts = mantissa.split('.');
    let whole = parts.next().unwrap_or("");
    let fraction = parts.next();
    if parts.next().is_some() {
        return false;
    }

    let digits = |part: &str| part.chars().all(|c| c.is_digit(radix));
    if !digits(whole) || !fraction.is_none_or(digits) {
        return false;
    }
    if whole.is_empty() && fraction.is_none_or(str::is_empty) {
        return false;
    }

    match exponent {
        Some(exponent) => {
            let exponent = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
            !exponent.is_empty() && exponent.bytes().all(|byte| byte.is_ascii_digit())
        }
        // No exponent at all: legal for a decimal constant that has the `.`
        // instead, and for no hexadecimal one.
        None => radix == 10 && fraction.is_some(),
    }
}

#[cfg(test)]
mod tests;
