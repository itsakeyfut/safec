//! The type of every expression, and the constraints C puts on one that this
//! stage checks.
//!
//! The second half of the stage `docs/architecture.md` draws between the AST
//! and the typed AST. `sema.rs` says which declaration a name means; this says
//! what type each expression has, and reports an assignment, a compound
//! assignment, an initializer, a `return`, a call and a binary operator whose
//! types C17 forbids.
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
/// One code for an assignment, an initializer and a `return`, because C17
/// makes them one rule: 6.7.9 p11 gives an initializer "the same type
/// constraints and conversions as for simple assignment", and 6.8.6.4 p3
/// converts a returned value as if it were assigned to an object of the
/// return type. What differs is the message, which is the same reason
/// `parser.rs`'s `EXPECTED` is one code for every shape of syntax error.
const MISMATCH: Code = Code::new("SC0302");

/// A call whose argument count is not the parameter count, C17 6.5.2.2 p2.
const ARGUMENTS: Code = Code::new("SC0303");

/// An operand an operator does not take, C17 6.5.5 p2 to 6.5.14 p2 for a
/// binary operator and 6.5.16.2 p1 and p2 for a compound assignment.
///
/// Not `MISMATCH`, which is a value of the wrong type for a place: `p *= 2`
/// is refused because `*=` does not take a pointer, whatever `p` holds. A
/// code is never reassigned, so the two are kept apart from the start.
const OPERANDS: Code = Code::new("SC0306");

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

    /// What the integer constant `id` is worth, or `None` where it is not a
    /// constant or is one this stage could not read.
    ///
    /// The two cases are one answer on purpose: a caller has nothing to do
    /// differently, because a constant this stage could not read was reported
    /// where it was read, and an expression that is not a constant has no
    /// value to ask for. Both are "do not build an operand out of this".
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
        // Pushed once. A new node per constant would fill the arena with
        // copies of `int` and make nothing truer.
        int: ast.push_type(Type::Int),
        int_range,
        receivers: HashMap::new(),
    };

    checker.collect_receivers(ast);

    for id in ast.expr_ids().collect::<Vec<_>>() {
        let ty = checker.type_of(ast, id, diagnostics);
        checker.types[id.index()] = ty;
        checker.check_received(ast, id, diagnostics);
    }

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

/// What a value has to be assignable to, and what a report about it says.
///
/// Two variants of one thing rather than two maps, because C17 makes them
/// one rule (see `MISMATCH`) and because one walk finds both: a second walk
/// would be a second recursion over every statement, and an arm dropped from
/// either would be a silence that the other's test could not see.
#[derive(Clone, Copy)]
enum Receiving {
    /// C17 6.8.6.4 p3.
    Return(Returning),
    /// C17 6.7.9 p11.
    Initializer {
        /// The type the declarator derived.
        ty: TypeId,
        /// The span of the declarator's name, which is the place.
        name: Span,
    },
}

struct Checker<'a> {
    sources: &'a SourceMap,
    resolution: &'a Resolution,
    types: Vec<Option<TypeId>>,
    /// What each integer constant is worth, filled by the same walk that
    /// fills `types` and empty everywhere else.
    values: Vec<Option<i128>>,
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
    receivers: HashMap<ExprId, Receiving>,
}

impl Checker<'_> {
    /// Find every `return` with a value and every initializer, and what each
    /// has to be assignable to.
    fn collect_receivers(&mut self, ast: &Ast) {
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
            );
        }
    }

    /// Every `return` and every initializer under one statement.
    ///
    /// A recursion, bounded by `parser::MAX_NESTING` the way every statement
    /// walk here is, and exhaustive so that a statement kind that can hold
    /// another has to be answered for rather than silently dropping the
    /// returns and initializers inside it.
    fn receivers_in(&mut self, ast: &Ast, id: StmtId, returning: Returning) {
        match ast.stmt(id) {
            Stmt::Return { value, .. } => {
                if let Some(value) = *value {
                    self.receivers.insert(value, Receiving::Return(returning));
                }
            }
            Stmt::Declaration { declarators, .. } => self.initializers(declarators),
            Stmt::Compound { body, .. } => {
                for &statement in body {
                    self.receivers_in(ast, statement, returning);
                }
            }
            Stmt::If {
                then, otherwise, ..
            } => {
                self.receivers_in(ast, *then, returning);
                if let Some(otherwise) = *otherwise {
                    self.receivers_in(ast, otherwise, returning);
                }
            }
            Stmt::While { body, .. } | Stmt::For { body, .. } => {
                self.receivers_in(ast, *body, returning);
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
            Expr::Unary { op, operand, .. } => self.unary(ast, op, operand),
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
            // 6.5.2.1 p2 makes `a[i]` mean `*(a + i)`, so its type is what the
            // base points at or holds.
            Expr::Subscript { base, .. } => match ast.ty(self.types[base.index()]?) {
                Type::Pointer(pointee) => Some(*pointee),
                Type::Array { element, .. } => Some(*element),
                Type::Int | Type::Char | Type::Void | Type::Function { .. } => None,
            },
            Expr::Call {
                callee, arguments, ..
            } => self.call(ast, id, callee, &arguments, diagnostics),
            // 6.5.15's rule is the usual arithmetic conversions and a page of
            // pointer cases. What is answered here is the half that needs
            // neither: two arms of one type make that type.
            Expr::Conditional {
                then, otherwise, ..
            } => {
                let then = self.types[then.index()]?;
                let otherwise = self.types[otherwise.index()]?;
                ast.compatible(then, otherwise).then_some(then)
            }
            Expr::Error { .. } => None,
        }
    }

    fn unary(&mut self, ast: &mut Ast, op: UnOp, operand: ExprId) -> Option<TypeId> {
        let operand = self.types[operand.index()];

        match op {
            // 6.5.3.2 p3: `&x` is a pointer to the type of `x`. The type
            // itself is one nobody wrote, so it is pushed.
            UnOp::AddrOf => Some(ast.push_type(Type::Pointer(operand?))),
            // p4: `*p` is what `p` points at.
            UnOp::Deref => match ast.ty(operand?) {
                Type::Pointer(pointee) => Some(*pointee),
                Type::Int
                | Type::Char
                | Type::Void
                | Type::Array { .. }
                | Type::Function { .. } => None,
            },
            // 6.5.2.4 p2 and 6.5.3.1 p2: an increment is the value of its
            // operand, so `p++` on a pointer is a pointer and not an `int`.
            UnOp::PreInc | UnOp::PreDec | UnOp::PostInc | UnOp::PostDec => operand,
            // 6.5.3.3: the integer promotions of 6.3.1.1 make the result of
            // `+`, `-` and `~` an `int` for every operand this compiler can
            // write, and p5 makes `!` an `int` outright. Still `None` for an
            // operand nothing typed, because an operand that is not a number
            // at all makes an expression with no type rather than an `int`,
            // and `p = -nowhere` reported twice while this said otherwise.
            UnOp::Plus | UnOp::Minus | UnOp::Not | UnOp::BitNot => operand.map(|_| self.int),
        }
    }

    /// C17 6.5.5 to 6.5.14: the type of a binary operation, and whether its
    /// operands are ones its operator takes.
    ///
    /// An operand this stage could not type is checked against nothing, and
    /// every operator but `+` and `-` is still `int` beside it. `None` there
    /// hands the lowering an expression it cannot type, and its `SC0304`
    /// joins `SC0305` in `a_suffixed_constant_has_no_type_here`, whose point is
    /// that there is one report; that case fails if this answers `None`. The
    /// cost is kept from before: `p = nowhere * 1` is reported as an
    /// undeclared name and then as an `int` given to a pointer, an `int` this
    /// stage made up.
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

        if let Some((left, right)) = operands {
            let nulls = (
                self.is_null_pointer_constant(lhs),
                self.is_null_pointer_constant(rhs),
            );
            if let Some(Err(refused)) = self.binary_operable(ast, op, left, right, nulls) {
                self.report_operands(ast, op, refused, (lhs, left), (rhs, right), diagnostics);
            }
        }

        // **A refused operation keeps the type it would have had**, which is
        // `compound_assignment`'s answer too. `None` would hand the lowering
        // an expression it cannot type, and its `SC0304` says the program is
        // fine and this compiler is not, which is false here. The cost is
        // that `p = p * 1` is reported twice, the second time as an `int`
        // given to a pointer, and both reports are about a program that is
        // wrong. `+` and `-` are the exception, because `additive` has no
        // type to give `n - p` or `p + q` whether or not it is refused, so
        // those two do get the `SC0304` and its false note.
        match op {
            BinOp::Add | BinOp::Sub => {
                let (left, right) = operands?;
                self.additive(ast, op, left, right)
            }
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
            // compiler has no name for, and p2 forbids adding two pointers at
            // all. Neither has a type to give.
            (true, true) => None,
            // p2 and p3 both allow the pointer on the left.
            (true, false) => Some(lhs),
            // p3 allows it only there: `i - p` is a constraint violation, and
            // typing it as a pointer made this compiler report the assignment
            // around it instead.
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
                    Some(stepping(ast, pointee, true))
                }
                (OperandClass::Arithmetic, OperandClass::Pointer(pointee)) => {
                    Some(stepping(ast, pointee, false))
                }
                _ => Some(pairing(false)),
            },
            // 6.5.6 p3, which allows the pointer only on the left.
            BinOp::Sub => match (left, right) {
                (OperandClass::Arithmetic, OperandClass::Arithmetic) => Some(Ok(())),
                (OperandClass::Pointer(pointee), OperandClass::Arithmetic) => {
                    Some(stepping(ast, pointee, true))
                }
                (OperandClass::Pointer(left), OperandClass::Pointer(right)) => {
                    // Both pointees are asked: `int[3]` is compatible with
                    // `int[]`, and only one of them is complete.
                    if ast.compatible(left, right) {
                        Some(stepping(ast, left, true).and(stepping(ast, right, false)))
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
            Refused::Pointee { left, .. } => left,
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
    /// Only [`Checker::binary`] asks. `assignable` deliberately does not:
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

    /// C17 6.8.6.4 p3 and 6.7.9 p11, each of which is 6.5.16.1 p1 with
    /// another place: the return type, or the declarator being initialized.
    fn check_received(&mut self, ast: &Ast, value: ExprId, diagnostics: &mut DiagnosticSink) {
        let Some(receiving) = self.receivers.get(&value).copied() else {
            return;
        };
        let Some(source) = self.types[value.index()] else {
            return;
        };
        let target = match receiving {
            Receiving::Return(returning) => returning.ty,
            Receiving::Initializer { ty, .. } => ty,
        };

        if self.assignable(ast, target, source, self.is_null_pointer_constant(value)) != Some(false)
        {
            return;
        }

        let source_spelled = self.spelled(ast, source);
        let target_spelled = self.spelled(ast, target);
        let (message, place, place_label) = match receiving {
            Receiving::Return(returning) => (
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

    /// C17 6.5.2.2 p2: the number of arguments and the number of parameters.
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
        let Type::Function {
            returns,
            parameters,
        } = ast.ty(self.types[callee.index()]?)
        else {
            return None;
        };
        let returns = *returns;

        let Parameters::Prototype(parameters) = parameters else {
            return Some(returns);
        };

        // A `match` on the ordering rather than two comparisons: the shape
        // `CLAUDE.md` records as the worst defect this project has had is a
        // gate written as two of those with a case answered by neither.
        let (what, expected, found) = match arguments.len().cmp(&parameters.len()) {
            Ordering::Less => ("too few", parameters.len(), arguments.len()),
            Ordering::Greater => ("too many", parameters.len(), arguments.len()),
            Ordering::Equal => return Some(returns),
        };

        let mut diagnostic = Diagnostic::error(format!(
            "{what} arguments: expected {expected}, found {found}"
        ))
        .with_code(ARGUMENTS)
        .with_label(Label::primary(ast.expr(call).span(), "this call"));

        if let Some(binding) = self.resolution.resolved(callee) {
            diagnostic = diagnostic.with_label(Label::secondary(
                self.resolution.binding(binding).name,
                format!("declared with {expected} parameters here"),
            ));
        }

        diagnostics.report(diagnostic);
        Some(returns)
    }

    /// Whether a value of type `source` may be assigned to a place of type
    /// `target`, C17 6.5.16.1 p1.
    ///
    /// `None` where this stage does not answer: an array, a function or `void`
    /// on either side is a different constraint, and 6.5.16 p2's requirement
    /// that the place be a modifiable lvalue is another. Reporting those is
    /// not this issue's, and answering `false` for them would be reporting
    /// them badly.
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
            (Type::Void | Type::Array { .. } | Type::Function { .. }, _)
            | (_, Type::Void | Type::Array { .. } | Type::Function { .. }) => None,
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
    /// and nothing here evaluates a constant expression, so this answers for
    /// the literal: `0`, `0x0` and `00` are null pointer constants and
    /// `1 - 1` is one that this reports as a mistake.
    ///
    /// The alternative was not reporting an integer assigned to a pointer at
    /// all, which is the mistake this check exists for. Between missing every
    /// `p = i` and reporting a `p = 1 - 1` nobody writes, the second costs
    /// less, and it stops costing anything the day a constant expression can
    /// be evaluated.
    ///
    /// Read off the value rather than off the text: `constant` below is the
    /// one place a spelling is turned into a number, so `0x0`, `00` and `0u`
    /// are zero here because they are zero there. A second reading of the
    /// text would be a second answer to drift from the first.
    fn is_null_pointer_constant(&self, value: ExprId) -> bool {
        self.values[value.index()] == Some(0)
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
    /// is what RK-002 in the review knowledge bank asks of a new message: the
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
/// One function for `v + 1` and `v += 1`, so that the two spellings of one
/// rule cannot be given different answers. The reason is written here rather
/// than spelled from `pointee`, because a spelled type can carry the file's
/// own bytes (RK-002) and none of the three needs the type to be read.
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

/// Why [`Checker::binary_operable`] refused a pair of operands.
#[derive(Clone, Copy)]
enum Refused {
    /// The operator's clause takes no such pairing of operand types.
    Pairing,
    /// C17 6.5.6 takes the pairing, and a pointer in it points to something
    /// it cannot step over. `left` says which operand, and `why` is
    /// [`unsteppable`]'s answer.
    Pointee { left: bool, why: &'static str },
}

/// `Ok` where the operator's clause takes the pairing of operand types, and
/// [`Refused::Pairing`] where it does not.
fn pairing(taken: bool) -> Result<(), Refused> {
    if taken { Ok(()) } else { Err(Refused::Pairing) }
}

/// Whether a pointer to `pointee`, on the `left` or not, can take the step
/// 6.5.6 would make.
fn stepping(ast: &Ast, pointee: TypeId, left: bool) -> Result<(), Refused> {
    match unsteppable(ast, pointee) {
        None => Ok(()),
        Some(why) => Err(Refused::Pointee { left, why }),
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
/// RK-004 in the review knowledge bank is what asks for that to be said out
/// loud rather than assumed, because the `char::is_*` family next to it
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
mod tests {
    use super::*;
    use crate::ast::spell_type;
    use crate::lexer::lex;
    use crate::parser::parse;
    use crate::sema::resolve;
    use safec_ir::target::Target;

    struct Checked {
        ast: Ast,
        sources: SourceMap,
        types: Types,
        diagnostics: DiagnosticSink,
    }

    /// Scan, parse, resolve and check, the way the driver does.
    fn checked(text: &str) -> Checked {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", text);
        let mut diagnostics = DiagnosticSink::new();
        let tokens = lex(file, sources.file(file), &mut diagnostics);
        let mut ast = parse(file, &tokens, &mut diagnostics);
        assert!(
            !diagnostics.has_errors(),
            "the input did not parse: {:?}",
            diagnostics.diagnostics()
        );
        let resolution = resolve(&sources, &ast, &mut diagnostics);
        // Named rather than taken from the host: what a constant's type is
        // turns on the range of `int`, so a test that did not name a target
        // would be asserting about whichever machine ran it. RK-011 is the
        // entry. `int` is 32 bits on every row of `Target::ALL`, which is
        // what makes one triple enough here.
        let target = Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple");
        let types = check(
            &sources,
            &mut ast,
            &resolution,
            target.int(),
            &mut diagnostics,
        );

        Checked {
            ast,
            sources,
            types,
            diagnostics,
        }
    }

    impl Checked {
        /// How the expression written as `text` is spelled as a type.
        ///
        /// Found by its own text among the expressions, so a test names one
        /// the way a reader would. It must be the only expression written
        /// that way: `x` in this file is two of them, and a helper that
        /// quietly took the first would let a row ask about something other
        /// than what it says. A declarator is not an expression, so the `*p`
        /// in `int *p;` is not a candidate.
        ///
        /// `None` is spelled `?`, so that a row asserting silence looks
        /// different from a row asserting a type.
        fn spelling(&self, text: &str) -> String {
            let written: Vec<ExprId> = self
                .ast
                .expr_ids()
                .filter(|&id| self.sources.snippet(self.ast.expr(id).span()) == text)
                .collect();

            let [id] = written[..] else {
                panic!("{text:?} is {} expressions, not one", written.len());
            };

            match self.types.of(id) {
                Some(ty) => spell_type(&self.sources, &self.ast, ty),
                None => "?".to_owned(),
            }
        }

        /// What the constant written as `text` is worth.
        ///
        /// Found the same way and under the same rule as `spelling`: it has
        /// to be the only expression written that way.
        fn value_of(&self, text: &str) -> Option<i128> {
            let written: Vec<ExprId> = self
                .ast
                .expr_ids()
                .filter(|&id| self.sources.snippet(self.ast.expr(id).span()) == text)
                .collect();

            let [id] = written[..] else {
                panic!("{text:?} is {} expressions, not one", written.len());
            };

            self.types.value(id)
        }

        fn messages(&self) -> Vec<&str> {
            self.diagnostics
                .diagnostics()
                .iter()
                .map(Diagnostic::message)
                .collect()
        }

        fn codes(&self) -> Vec<&str> {
            self.diagnostics
                .diagnostics()
                .iter()
                .filter_map(|diagnostic| diagnostic.code().map(Code::as_str))
                .collect()
        }

        /// Every label's message, so that a test can hold what a caret says
        /// and not only which code said it.
        fn labels(&self) -> Vec<&str> {
            self.diagnostics
                .diagnostics()
                .iter()
                .flat_map(Diagnostic::labels)
                .map(Label::message)
                .collect()
        }

        fn notes(&self) -> Vec<&str> {
            self.diagnostics
                .diagnostics()
                .iter()
                .flat_map(Diagnostic::notes)
                .map(String::as_str)
                .collect()
        }
    }

    /// One program per spelling, so that a row is about one constant.
    fn returning(spelling: &str) -> Checked {
        checked(&format!("int f(void) {{\n    return {spelling};\n}}\n"))
    }

    /// An integer constant is worth what its base says, C17 6.4.4.1 p1.
    ///
    /// The values are written out rather than computed from the spellings,
    /// which is what RK-001 asks: a table that works the expectation out the
    /// way the code does agrees with the code however wrong both are.
    ///
    /// Every row is unsuffixed, because a suffix is refused rather than read:
    /// `a_suffix_asks_for_a_type_this_compiler_does_not_have` is that half.
    ///
    /// Mutation: give the hexadecimal arm radix ten. `0x10` is ten and this
    /// fails. Mutation: give the octal arm radix ten. `010` is ten and this
    /// fails, along with most of the suite, because `0` stops being readable
    /// too.
    #[test]
    fn an_integer_constant_is_worth_what_its_base_says() {
        for (spelling, value) in [
            ("0", 0),
            ("00", 0),
            ("16", 16),
            ("010", 8),
            ("0777", 511),
            ("0x10", 16),
            ("0X10", 16),
            ("2147483647", 2147483647),
        ] {
            let checked = returning(spelling);
            assert_eq!(checked.messages(), Vec::<&str>::new(), "{spelling}");
            assert_eq!(checked.value_of(spelling), Some(value), "{spelling}");
            assert_eq!(checked.spelling(spelling), "int", "{spelling}");
        }
    }

    /// Every suffix C17 6.4.4.1 p1 allows is refused, and refused as a type
    /// this compiler lacks rather than as a spelling C lacks.
    ///
    /// The whole table is walked, written out as literals, because a suffix
    /// arm is the contents of a table and nothing else checks them: a typo
    /// turning `ULL` into `ULK` makes `1ULL` a malformed spelling rather than
    /// an unsupported type, which is this compiler blaming the program for
    /// its own gap. Measured: that typo passed the entire suite before this
    /// test existed.
    ///
    /// **Refused rather than read.** A suffix decides what arithmetic on the
    /// constant means. With `3u` read as an `int`, `-6 / 3u` compiles to a
    /// signed division, and C17 6.3.1.8 makes it an unsigned one whose answer
    /// is 1431655763 rather than -2. Measured against `clang 20.1.6 -std=c17
    /// -pedantic-errors`, whose constant evaluator agrees.
    ///
    /// Mutation: read a suffixed constant as an `int` instead of answering
    /// `Reading::Suffixed`. Every row stops being reported and this fails.
    #[test]
    fn a_suffix_asks_for_a_type_this_compiler_does_not_have() {
        for suffix in [
            "u", "U", "l", "L", "ll", "LL", "ul", "uL", "Ul", "UL", "lu", "lU", "Lu", "LU", "ull",
            "uLL", "Ull", "ULL", "llu", "llU", "LLu", "LLU",
        ] {
            let spelling = format!("1{suffix}");
            let checked = returning(&spelling);
            assert_eq!(
                checked.messages(),
                ["this compiler has no type for a suffixed constant"],
                "{spelling}"
            );
            assert_eq!(checked.codes(), ["SC0305"], "{spelling}");
            assert_eq!(checked.value_of(&spelling), None, "{spelling}");
        }

        // The zero of each base with a suffix on it, because those are the
        // ones `a_pointer_takes_a_zero_and_not_a_one` used to accept as null
        // pointer constants and no longer does.
        for spelling in ["0L", "0u", "0UL", "0x0u"] {
            let checked = returning(spelling);
            assert_eq!(checked.codes(), ["SC0305"], "{spelling}");
        }
    }

    /// `e` is a hexadecimal digit before it is an exponent marker.
    ///
    /// The half of `read` a reader would get wrong: 6.4.4.2 p1 spells a
    /// decimal's exponent `e` and a hexadecimal's `p`, so the base has to be
    /// settled before the marker is looked for. Both directions are here,
    /// because a rule that only refused would pass against one that refused
    /// everything.
    ///
    /// Mutation: look for `e`/`E` in a hexadecimal constant too, by giving
    /// both arms the same markers. `0xe1` is reported as a floating constant
    /// and this fails, along with
    /// `a_floating_constant_is_reported_as_one_this_compiler_does_not_read_yet`,
    /// whose `0x1p3` row stops being read as one.
    #[test]
    fn a_hexadecimal_digit_e_is_not_an_exponent() {
        let checked = returning("0xe1");
        assert_eq!(checked.messages(), Vec::<&str>::new());
        assert_eq!(checked.value_of("0xe1"), Some(225));

        for spelling in ["1e5", "0e1", "0x1p3"] {
            let checked = returning(spelling);
            assert_eq!(
                checked.messages(),
                ["this compiler does not read floating constants yet"],
                "{spelling}"
            );
        }
    }

    /// A value outside `int` is reported rather than wrapped, C17 6.4.4 p2.
    ///
    /// The first is a constant `clang 20.1.6 -std=c17 -pedantic-errors
    /// --target=x86_64-pc-windows-msvc` accepts, which is why the message
    /// blames this compiler rather than the program: what is missing is
    /// `long`, not a well-formed constant. `docs/frontend.md` carries the
    /// divergence.
    ///
    /// **The last two are the two ways the carrier itself can be overrun, and
    /// each is here for one line.**
    ///
    /// `u128::MAX` written out is what `read_number` returns when the
    /// accumulation lands exactly on the top. Converted with `as` rather than
    /// asked whether it fits an `i128` it is -1, which `int` holds, so the
    /// constant would be accepted silently as minus one.
    ///
    /// `2^128` is one more, and it is congruent to zero. A wrapping
    /// accumulator reaches the end of it holding zero, which `int` also
    /// holds, so the constant would be accepted silently as nought. The forty
    /// nines cannot catch that: they wrap to a number still outside `int`, so
    /// they are reported either way. Both are the `010`-lowers-to-ten shape,
    /// a plausible wrong number rather than a refusal.
    ///
    /// Mutation: answer `true` from `Integer::holds`. The first is accepted
    /// with a value `int` does not hold and this fails. Mutation: accumulate
    /// with `wrapping_mul` and `wrapping_add`. `2^128` becomes zero, nothing
    /// is reported, and this fails. Mutation: replace
    /// `i128::try_from(value).ok()` with `Some(value as i128)`. `u128::MAX`
    /// becomes `Constant -1` with nothing reported and this fails.
    #[test]
    fn a_constant_no_type_here_can_hold_is_reported_and_has_no_value() {
        for spelling in [
            "2147483648",
            "9999999999999999999999999999999999999999",
            "340282366920938463463374607431768211455",
            "340282366920938463463374607431768211456",
        ] {
            let checked = returning(spelling);
            assert_eq!(
                checked.messages(),
                ["no integer type this compiler has can hold this constant"],
                "{spelling}"
            );
            assert_eq!(checked.codes(), ["SC0305"], "{spelling}");
            assert_eq!(checked.value_of(spelling), None, "{spelling}");
            assert_eq!(checked.spelling(spelling), "?", "{spelling}");
        }

        // The edge is the edge: `int` is 32 bits on the named target, so one
        // less than the reported value is an ordinary constant.
        let checked = returning("2147483647");
        assert_eq!(checked.messages(), Vec::<&str>::new());
        assert_eq!(checked.value_of("2147483647"), Some(2147483647));
    }

    /// A floating constant is this compiler's gap, not the program's fault.
    ///
    /// `1.5` is typed `int` today with nothing said, which is the silent
    /// wrong answer this replaces.
    ///
    /// Mutation: report a floating constant as `TOO_LARGE`. The message is
    /// about a width rather than about a type that does not exist, and this
    /// fails. Mutation: change `FLOATING`'s label. The label assertion below
    /// fails, and nothing else in the suite does, which is why the label is
    /// asserted here rather than left to a corpus case that does not exist.
    #[test]
    fn a_floating_constant_is_reported_as_one_this_compiler_does_not_read_yet() {
        for spelling in [
            "1.5", "0.0", ".5", "1.", "1e5", "1E+5", "0x1p3", "1.5f", "0x1.8p0",
        ] {
            let checked = returning(spelling);
            assert_eq!(
                checked.messages(),
                ["this compiler does not read floating constants yet"],
                "{spelling}"
            );
            assert_eq!(checked.codes(), ["SC0305"], "{spelling}");
            assert_eq!(
                checked.labels(),
                ["this is a floating constant"],
                "{spelling}"
            );
            assert_eq!(checked.value_of(spelling), None, "{spelling}");
        }
    }

    /// A spelling with a `.` or an exponent that C17 6.4.4.2 p1 does not
    /// spell that way is the program's fault, not this compiler's gap.
    ///
    /// The distinction the whole two-code arrangement rests on, at the one
    /// place it is easiest to lose: `1.5` is a constant C gives a type to and
    /// this compiler has none for, so `SC0305` is honest; `1e` is not a
    /// constant at all, and telling its author that floating constants are
    /// unsupported would be a false reason for a true refusal.
    ///
    /// Every row is an error under `clang 20.1.6 -std=c17 -pedantic-errors
    /// --target=x86_64-unknown-linux-gnu`, which reports `1e` as "exponent
    /// has no digits" and `0x1.8` as "hexadecimal floating constant requires
    /// an exponent". Measured, not recalled.
    ///
    /// Mutation: answer `Reading::Floating` for anything with a `.` or an
    /// exponent marker, without checking the grammar. Every row starts
    /// reporting `SC0305` and this fails.
    #[test]
    fn a_spelling_that_only_looks_like_a_floating_constant_is_the_programs_fault() {
        for spelling in [
            "1e", "1E+", "1e-", "0x1p", "0x1.8", "1.2.3", "0xp1", "1.5ll",
        ] {
            let checked = returning(spelling);
            assert_eq!(
                checked.messages(),
                ["this is not a constant C allows"],
                "{spelling}"
            );
            assert_eq!(checked.codes(), ["SC0106"], "{spelling}");
        }
    }

    /// A spelling that is no constant at all is the program's fault.
    ///
    /// Each row was measured against `clang 20.1.6 -std=c17
    /// -pedantic-errors --target=x86_64-pc-windows-msvc`, which reports every
    /// one of them as an error; `-pedantic-errors` is the flag that makes the
    /// answer C's rather than clang's, which is RK-032.
    ///
    /// Mutation: accept any suffix. `1lL`, `1uu`, `123abc` and `0b101` stop
    /// being reported and this fails. Mutation: answer `NOT_A_CONSTANT` for
    /// an octal digit out of range. `09` keeps its code and loses its label,
    /// and this fails along with the corpus case
    /// `a_spelling_that_is_not_a_constant`, whose blessed stderr holds the
    /// same label. Two guards, and both are about the label rather than the
    /// code, which is what says the label is guarded at all.
    #[test]
    fn a_spelling_that_is_not_a_constant_is_the_programs_fault() {
        for (spelling, label) in [
            (
                "123abc",
                "this is not a digit of the constant's base, and not a suffix C allows",
            ),
            (
                "1lL",
                "this is not a digit of the constant's base, and not a suffix C allows",
            ),
            (
                "1uu",
                "this is not a digit of the constant's base, and not a suffix C allows",
            ),
            (
                "0b101",
                "this is not a digit of the constant's base, and not a suffix C allows",
            ),
            ("09", "an octal constant has digits `0` to `7`"),
            (
                "0x",
                "a hexadecimal constant needs at least one digit after the `0x`",
            ),
        ] {
            let checked = returning(spelling);
            assert_eq!(
                checked.messages(),
                ["this is not a constant C allows"],
                "{spelling}"
            );
            assert_eq!(checked.codes(), ["SC0106"], "{spelling}");
            assert_eq!(checked.labels(), [label], "{spelling}");
        }
    }

    /// What each shape of expression is worth, spelled the way C declares it.
    ///
    /// The spellings are written out rather than derived from the types, for
    /// the reason RK-001 gives.
    ///
    /// Mutation: give `UnOp::AddrOf` the operand's type rather than a pointer
    /// to it, or `UnOp::Deref` the operand's. Either fails. Mutation: type
    /// `BinOp::Add` as `int` whatever its operands are; `p + 1` stops being a
    /// pointer and this fails.
    #[test]
    fn every_shape_of_expression_has_the_type_c_gives_it() {
        let checked = checked(
            "int f(int a) { return a; }\n\nint main(void) {\n    int x;\n    char c;\n    int *p;\n    int *r;\n    int q[3];\n    p = &x;\n    q[0] = *p + f(2) + x++;\n    p = p + 1;\n    p = q + 1;\n    p++;\n    -c;\n    p - r;\n    1 - p;\n    p - 1;\n    return 0;\n}\n",
        );

        // `1 - p` is the one expression here C refuses, and it is here for
        // its type rather than for the report.
        assert_eq!(checked.messages(), ["`-` cannot take `int` and `int *`"]);

        for (text, spelling) in [
            // An array's length is an expression like any other, and is one
            // no walk of the statements would reach.
            ("3", "int"),
            ("2", "int"),
            ("&x", "int *"),
            ("*p", "int"),
            ("q[0]", "int"),
            ("f(2)", "int"),
            ("x++", "int"),
            // An increment keeps its operand's type: 6.5.2.4 p2 makes `p++`
            // a pointer, and typing it `int` broke nothing before this row.
            ("p++", "int *"),
            // 6.3.1.1's promotions make a unary operator's result `int` even
            // when its operand is a `char`.
            ("-c", "int"),
            ("p + 1", "int *"),
            ("p = p + 1", "int *"),
            // 6.3.2.1 p3 converts the array first, so this is pointer
            // arithmetic rather than arithmetic on something that is not a
            // pointer. Before that conversion existed, `p = q + 1` was
            // reported against a program `clang` compiles.
            ("q + 1", "int *"),
            ("p - 1", "int *"),
            // 6.5.6 p9: a pointer minus a pointer is `ptrdiff_t`, which this
            // compiler cannot name. p3 allows the pointer only on the left,
            // so `1 - p` is not an expression C gives a type to at all.
            ("p - r", "?"),
            ("1 - p", "?"),
        ] {
            assert_eq!(checked.spelling(text), spelling, "{text}");
        }
    }

    /// C17 6.3.2.3 p3 lets a pointer take a null pointer constant and nothing
    /// else that is an integer.
    ///
    /// Both halves in one test, because a test for the silence alone passes
    /// against a compiler that checks nothing.
    ///
    /// The suffixed zeros `0u`, `0L` and `0UL` are C's null pointer constants
    /// too and are not here, because a suffix is refused before this rule is
    /// reached: `a_suffix_asks_for_a_type_this_compiler_does_not_have` holds
    /// that, and holds those three spellings by name so that what left this
    /// list is findable from where it went.
    ///
    /// Mutation: have `is_null_pointer_constant` answer `false` always. The
    /// first program starts reporting and this fails. Mutation: have it answer
    /// `true` always. The second stops and this fails.
    #[test]
    fn a_pointer_takes_a_zero_and_not_a_one() {
        for zero in ["0", "0x0", "0X0", "00", "000"] {
            let checked = checked(&format!(
                "int main(void) {{ int *p; p = {zero}; return 0; }}\n"
            ));
            assert_eq!(checked.messages(), Vec::<&str>::new(), "{zero}");
        }

        let checked = checked("int main(void) { int *p; p = 1; return 0; }\n");
        assert_eq!(checked.messages(), ["cannot assign `int` to `int *`"]);
    }

    /// Which assignments between pointers C17 6.5.16.1 p1 allows.
    ///
    /// One program with all four, so that the three silences are asserted
    /// against a run that does report something and cannot pass by checking
    /// nothing.
    ///
    /// Mutation: drop the `void` half of the pointer arm of `assignable`. The
    /// two `void *` lines start reporting and this fails. Mutation: have
    /// `Ast::compatible` answer `true` always. The `char *` line stops
    /// reporting and this fails, which is the only place in the suite that
    /// holds the comparison against a program rather than against a table.
    #[test]
    fn a_pointer_takes_the_same_pointer_or_a_void_one() {
        let checked = checked(
            "int main(void) {
    int *p;
    char *c;
    void *v;
    p = p;
    p = v;
    v = p;
    p = c;
    return 0;
}
",
        );

        assert_eq!(checked.messages(), ["cannot assign `char *` to `int *`"]);
    }

    /// C17 6.5.2.2 p2 makes the argument count a constraint only where the
    /// callee's type includes a prototype, and 6.7.6.3 p14 makes `()` an empty
    /// identifier list rather than one.
    ///
    /// Both halves in one test, because the silence alone passes against a
    /// compiler that counts nothing.
    ///
    /// Mutation: treat `Parameters::Unspecified` as a list of no parameters.
    /// The first program starts reporting three arguments too many, against a
    /// program `clang` compiles, and this fails.
    #[test]
    fn an_empty_parameter_list_says_nothing_about_the_count() {
        let nothing_said = checked(
            "int f();

int main(void) { return f(1, 2, 3); }
",
        );
        assert_eq!(nothing_said.messages(), Vec::<&str>::new());

        let a_prototype = checked(
            "int f(void);

int main(void) { return f(1, 2, 3); }
",
        );
        assert_eq!(
            a_prototype.messages(),
            ["too many arguments: expected 0, found 3"]
        );
    }

    /// C17 6.5.15's rule for a conditional, as far as this stage answers it.
    ///
    /// Two arms of one type make that type; the rest of p5 needs the usual
    /// arithmetic conversions and is not answered.
    ///
    /// Mutation: have the `Expr::Conditional` arm answer `None`. The first two
    /// rows lose their type and this fails. Before it existed the whole arm
    /// could be deleted with the suite green: the three corpus cases that
    /// write `?:` never assign one or return one, so nothing observed its
    /// type.
    #[test]
    fn a_conditional_has_a_type_when_both_its_arms_agree() {
        let checked = checked(
            "int main(void) {
    int *p;
    void *v;
    int x;
    x = 1 ? 2 : 3;
    p = 1 ? p : p;
    1 ? p : v;
    return 0;
}
",
        );

        assert_eq!(checked.messages(), Vec::<&str>::new());

        for (text, spelling) in [
            ("1 ? 2 : 3", "int"),
            ("1 ? p : p", "int *"),
            ("1 ? p : v", "?"),
        ] {
            assert_eq!(checked.spelling(text), spelling, "{text}");
        }
    }

    /// C17 6.7.6.3 p15 makes a prototype and an empty identifier list
    /// compatible when the default argument promotions leave every parameter
    /// alone, so the first assignment is ordinary C and `clang` accepts it.
    ///
    /// Mutation: answer `false` for `(Prototype, Unspecified)` in
    /// `Ast::compatible_parameters`, which is what reading 6.7.6.3 p10 and p14
    /// alone gives. The first line starts reporting and this fails.
    #[test]
    fn a_function_pointer_takes_one_of_a_compatible_type() {
        let checked = checked(
            "int main(void) {
    int (*p)(void);
    int (*q)();
    int (*r)(char);
    p = q;
    r = q;
    return 0;
}
",
        );

        assert_eq!(
            checked.messages(),
            ["cannot assign `int (*)()` to `int (*)(char)`"]
        );
    }

    /// A type in a message is one line, however the file wrote it.
    ///
    /// An array's length is spelled with the source's own bytes, so a comment
    /// inside one can carry a newline, and `render.rs::shown` keeps a newline
    /// on purpose. Without `Checker::spelled` a file could print a line of its
    /// own invention into this compiler's report, which is what the input
    /// below tries to do.
    ///
    /// Mutation: spell the types in `assignment` with `spell_type` directly.
    /// The message and the labels carry the comment's newlines and this fails.
    #[test]
    fn a_type_in_a_message_is_one_line_however_it_was_written() {
        let checked = checked(
            "int main(void) {
    int (*r)[1 /*
error[SC0302]: no problems found
*/ + 2];
    int *p;
    p = r;
    return 0;
}
",
        );

        let reported = checked.diagnostics.diagnostics();
        let [reported] = reported else {
            panic!("{reported:?}");
        };

        assert!(
            !reported.message().contains('\n'),
            "{:?}",
            reported.message()
        );
        for label in reported.labels() {
            assert!(!label.message().contains('\n'), "{:?}", label.message());
        }
    }

    /// A compound assignment is C17 6.5.16.2, whose constraints are its own:
    /// `p += 1` is how a pointer is advanced.
    ///
    /// Mutation: have the compound arm of `type_of` call `assignment` rather
    /// than `compound_assignment`. This fails with a diagnostic about correct
    /// C.
    #[test]
    fn a_compound_assignment_is_not_the_rule_for_a_plain_one() {
        let checked = checked("int main(void) { int *p; p += 1; return 0; }\n");

        assert_eq!(checked.messages(), Vec::<&str>::new());
    }

    /// C17 6.5.16.2 refuses a pointer only where it says so: `+=` and `-=`
    /// take one on the left with an integer on the right, and nothing else
    /// takes one at all.
    ///
    /// One program with the silences and a report together, so the silences
    /// are asserted against a run that does report. `v += 1` reports, because
    /// `void` is not a complete object type and p1 asks for a pointer to one.
    ///
    /// Mutation: have `compound_assignable` answer `Some(false)` for a
    /// pointer with `+=` or `-=`. `p += 1` and `p -= 1` start reporting.
    /// Mutation: have it answer `Some(true)` for a pointer with `+=` or `-=`
    /// whatever it points to. `v += 1` goes silent. Mutation: give the report
    /// `MISMATCH`. The code fails.
    #[test]
    fn a_compound_assignment_refuses_a_pointer_only_where_c_does() {
        let checked = checked(
            "int main(void) {
    int *p;
    int i;
    char c;
    void *v;
    p += 1;
    p -= 1;
    i += 1;
    c *= 2;
    i <<= 1;
    v += 1;
    p *= 2;
    return 0;
}
",
        );

        assert_eq!(
            checked.messages(),
            [
                "`+=` cannot take `void *` and `int`",
                "`*=` cannot take `int *` and `int`"
            ]
        );
        assert_eq!(checked.codes(), ["SC0306", "SC0306"]);
    }

    /// Every operator and every pair of operand types 6.5.16.2 answers for,
    /// each in a program of its own, with the message and the primary label
    /// written out rather than worked out (RK-001).
    ///
    /// A row is `(statement, message, primary label)`, and an empty message is
    /// a statement that must stay silent. The primary label is the first one,
    /// and it names the operand the rule refuses.
    ///
    /// Mutation: move any of `/`, `%`, `>>`, `&`, `^` or `|` into the arm
    /// that lets a pointer take an integer. Its row goes silent. Mutation:
    /// answer `None` or `Some(true)` for a `char` place and a pointer value.
    /// The `c += p` row goes silent, and that is a local declared `char`
    /// holding an addition over a pointer, which ADR-0030 drops. Mutation:
    /// refuse a pointer to a pointer or to an array of known length. Their
    /// rows start reporting. Mutation: have `unsteppable` answer `None` for
    /// an array of unknown length, `void` or a function. The `q += 1`,
    /// `v -= 1` or `fp -= 1` row goes silent. Mutation: answer `None` for a `void` place or a
    /// `void`, array or function value. Those rows go silent. Mutation: pick
    /// the primary label by whether the value is a pointer, as this did
    /// before. The `void` and array value rows name the place.
    #[test]
    fn a_compound_assignment_answers_for_every_operator_and_operand() {
        for (statement, message, primary) in [
            (
                "p /= 2;",
                "`/=` cannot take `int *` and `int`",
                "this is `int *`",
            ),
            (
                "p %= 2;",
                "`%=` cannot take `int *` and `int`",
                "this is `int *`",
            ),
            (
                "p >>= 1;",
                "`>>=` cannot take `int *` and `int`",
                "this is `int *`",
            ),
            (
                "p &= 1;",
                "`&=` cannot take `int *` and `int`",
                "this is `int *`",
            ),
            (
                "p ^= 1;",
                "`^=` cannot take `int *` and `int`",
                "this is `int *`",
            ),
            (
                "p |= 1;",
                "`|=` cannot take `int *` and `int`",
                "this is `int *`",
            ),
            (
                "c += p;",
                "`+=` cannot take `char` and `int *`",
                "this is `int *`",
            ),
            (
                "*v += 1;",
                "`+=` cannot take `void` and `int`",
                "this is `void`",
            ),
            (
                "i += g();",
                "`+=` cannot take `int` and `void`",
                "this is `void`",
            ),
            (
                "i += a;",
                "`+=` cannot take `int` and `int[2]`",
                "this is `int[2]`",
            ),
            ("pp += 1;", "", ""),
            ("pa -= 1;", "", ""),
            (
                "q += 1;",
                "`+=` cannot take `int (*)[]` and `int`",
                "this is `int (*)[]`",
            ),
            (
                "v -= 1;",
                "`-=` cannot take `void *` and `int`",
                "this is `void *`",
            ),
            (
                "fp -= 1;",
                "`-=` cannot take `void (*)(void)` and `int`",
                "this is `void (*)(void)`",
            ),
        ] {
            let checked = checked(&format!(
                "void g(void);
int main(void) {{
    int *p;
    char c;
    void *v;
    int i;
    int a[2];
    int **pp;
    int (*pa)[2];
    int (*q)[];
    void (*fp)(void);
    {statement}
    return 0;
}}
"
            ));

            if message.is_empty() {
                assert_eq!(checked.messages(), Vec::<&str>::new(), "{statement}");
            } else {
                assert_eq!(checked.messages(), [message], "{statement}");
                assert_eq!(checked.labels().first(), Some(&primary), "{statement}");
            }
        }
    }

    /// Every binary operator against the constraint of its own clause, C17
    /// 6.5.5 p2 to 6.5.14 p2, each in a program of its own, with the message
    /// and the primary label written out rather than worked out (RK-001).
    ///
    /// A row is `(expression, message, primary label)`, and an empty message
    /// is an expression that must stay silent. The silences are every pairing
    /// C allows, so a clause answered too strictly fails here as surely as one
    /// answered too loosely.
    ///
    /// Mutation: have `binary` stop calling `binary_operable`. Every reporting
    /// row goes silent. Mutation: have the `==` arm answer `false` for a
    /// pointer beside an integer. `p == 0` and `0 == p` start reporting;
    /// answer `left_is_null` on both sides and `p == 0` alone does. Mutation:
    /// drop `compatible` from `-`, `<` or `==`. `p - c`, `p < c` or `p == c`
    /// goes silent. Mutation: drop the function test from the relational arm.
    /// `g < g` goes silent. Mutation: have `is_void_beside_an_object` ignore
    /// `other`. `v == g` goes silent. Mutation: answer `Void` as a scalar for
    /// `&&`. `g() && 1` goes silent. Mutation: have `report_operands` put the
    /// primary label on the left always. `1 >> p`, `n - p` and `p == 1` name
    /// the wrong operand; on the right always, and `p * 1` and `g() * 1` do.
    /// Mutation: spell the undecayed types. The `a * 1` row names `int[2]`.
    ///
    /// Every operator has a row that it refuses, because the four relational
    /// operators share one arm and a mutation that splits them is otherwise
    /// seen only through `<`. Mutation: answer `Some(true)` for `>`, `<=` and
    /// `>=` alone. `p > 1`, `p >= c` and `g <= g` go silent. Mutation: answer
    /// `Some(true)` for a `void` operand of a relational operator. `g() < 1`
    /// goes silent. Mutation: move `>>` into `takes_a_pointer`'s `true` arm,
    /// or `!=`, `&&` and `||` into its `false` arm. `p >> 1`, `p != 1`,
    /// `p && g()` and `p || g()` name the wrong operand. Mutation: refuse a
    /// `char` on the left alone. `c[0] * p` names `c[0]`.
    ///
    /// A pointer to something with no size is refused by `+` and `-`, C17
    /// 6.5.6 p2 and p3, and named whichever side it is on. Mutation: have the
    /// `+` arm answer `Ok` for a pointer and an integer whatever it points to.
    /// `v + 1`, `1 + v`, `g + 1` and `u + 1` go silent. Mutation: ask only the
    /// left pointee of a subtraction of two pointers. `pa - u` goes silent.
    /// Mutation: pick the primary label by `Refused::Pairing`'s rule for a
    /// `Refused::Pointee` too. `v + 1` and `u - pa` name the wrong operand.
    #[test]
    fn a_binary_operator_answers_for_every_operator_and_operand() {
        for (expression, message, primary) in [
            (
                "p * 1",
                "`*` cannot take `int *` and `int`",
                "this is `int *`",
            ),
            (
                "p / 1",
                "`/` cannot take `int *` and `int`",
                "this is `int *`",
            ),
            (
                "p % 2",
                "`%` cannot take `int *` and `int`",
                "this is `int *`",
            ),
            (
                "p << 1",
                "`<<` cannot take `int *` and `int`",
                "this is `int *`",
            ),
            (
                "1 >> p",
                "`>>` cannot take `int` and `int *`",
                "this is `int *`",
            ),
            (
                "p & 1",
                "`&` cannot take `int *` and `int`",
                "this is `int *`",
            ),
            (
                "p ^ 1",
                "`^` cannot take `int *` and `int`",
                "this is `int *`",
            ),
            (
                "p | 1",
                "`|` cannot take `int *` and `int`",
                "this is `int *`",
            ),
            (
                "a * 1",
                "`*` cannot take `int *` and `int`",
                "this is `int *`",
            ),
            (
                "n - p",
                "`-` cannot take `int` and `int *`",
                "this is `int *`",
            ),
            (
                "p + q",
                "`+` cannot take `int *` and `int *`",
                "this is `int *`",
            ),
            (
                "p - c",
                "`-` cannot take `int *` and `char *`",
                "this is `char *`",
            ),
            (
                "p < c",
                "`<` cannot take `int *` and `char *`",
                "this is `char *`",
            ),
            (
                "p == c",
                "`==` cannot take `int *` and `char *`",
                "this is `char *`",
            ),
            (
                "p < 0",
                "`<` cannot take `int *` and `int`",
                "this is `int`",
            ),
            (
                "p < n",
                "`<` cannot take `int *` and `int`",
                "this is `int`",
            ),
            (
                "p == 1",
                "`==` cannot take `int *` and `int`",
                "this is `int`",
            ),
            (
                "1 != p",
                "`!=` cannot take `int` and `int *`",
                "this is `int *`",
            ),
            (
                "g < g",
                "`<` cannot take `void (*)(void)` and `void (*)(void)`",
                "this is `void (*)(void)`",
            ),
            (
                "v == g",
                "`==` cannot take `void *` and `void (*)(void)`",
                "this is `void (*)(void)`",
            ),
            (
                "g() * 1",
                "`*` cannot take `void` and `int`",
                "this is `void`",
            ),
            (
                "1 * g()",
                "`*` cannot take `int` and `void`",
                "this is `void`",
            ),
            (
                "g() && 1",
                "`&&` cannot take `void` and `int`",
                "this is `void`",
            ),
            (
                "n || g()",
                "`||` cannot take `int` and `void`",
                "this is `void`",
            ),
            (
                "g() == 1",
                "`==` cannot take `void` and `int`",
                "this is `void`",
            ),
            (
                "g() < 1",
                "`<` cannot take `void` and `int`",
                "this is `void`",
            ),
            (
                "p >> 1",
                "`>>` cannot take `int *` and `int`",
                "this is `int *`",
            ),
            (
                "c[0] * p",
                "`*` cannot take `char` and `int *`",
                "this is `int *`",
            ),
            (
                "p > 1",
                "`>` cannot take `int *` and `int`",
                "this is `int`",
            ),
            (
                "p >= c",
                "`>=` cannot take `int *` and `char *`",
                "this is `char *`",
            ),
            (
                "g <= g",
                "`<=` cannot take `void (*)(void)` and `void (*)(void)`",
                "this is `void (*)(void)`",
            ),
            (
                "p != 1",
                "`!=` cannot take `int *` and `int`",
                "this is `int`",
            ),
            (
                "p && g()",
                "`&&` cannot take `int *` and `void`",
                "this is `void`",
            ),
            (
                "p || g()",
                "`||` cannot take `int *` and `void`",
                "this is `void`",
            ),
            ("n * c[0]", "", ""),
            ("n % 2 << 1 & 3 ^ 4 | 5", "", ""),
            ("p - q", "", ""),
            ("p + 1", "", ""),
            ("1 + p", "", ""),
            ("p - 1", "", ""),
            ("a + 1", "", ""),
            ("p < q", "", ""),
            ("p > q", "", ""),
            ("p <= q", "", ""),
            ("v >= v", "", ""),
            ("p == q", "", ""),
            ("p == 0", "", ""),
            ("0 == p", "", ""),
            ("p != v", "", ""),
            ("a == p", "", ""),
            ("g == g", "", ""),
            ("p && q", "", ""),
            ("p || n", "", ""),
            (
                "v + 1",
                "`+` cannot take `void *` and `int`",
                "this is `void *`",
            ),
            (
                "1 + v",
                "`+` cannot take `int` and `void *`",
                "this is `void *`",
            ),
            (
                "v - 1",
                "`-` cannot take `void *` and `int`",
                "this is `void *`",
            ),
            (
                "v - v",
                "`-` cannot take `void *` and `void *`",
                "this is `void *`",
            ),
            (
                "g + 1",
                "`+` cannot take `void (*)(void)` and `int`",
                "this is `void (*)(void)`",
            ),
            (
                "u + 1",
                "`+` cannot take `int (*)[]` and `int`",
                "this is `int (*)[]`",
            ),
            (
                "pa - u",
                "`-` cannot take `int (*)[2]` and `int (*)[]`",
                "this is `int (*)[]`",
            ),
            (
                "u - pa",
                "`-` cannot take `int (*)[]` and `int (*)[2]`",
                "this is `int (*)[]`",
            ),
            ("pa - pa", "", ""),
        ] {
            let checked = checked(&format!(
                "void g(void);
int main(void) {{
    int *p;
    int *q;
    char *c;
    void *v;
    int n;
    int a[2];
    int (*pa)[2];
    int (*u)[];
    {expression};
    return 0;
}}
"
            ));

            if message.is_empty() {
                assert_eq!(checked.messages(), Vec::<&str>::new(), "{expression}");
            } else {
                assert_eq!(checked.messages(), [message], "{expression}");
                assert_eq!(checked.codes(), ["SC0306"], "{expression}");
                assert_eq!(checked.labels().first(), Some(&primary), "{expression}");
            }
        }
    }

    /// A pointer refused for what it points to says so, with the paragraph of
    /// the spelling that refused it, and a pointer refused for the pairing it
    /// is in says nothing more than the pairing: `p - v` is two pointers that
    /// are not compatible, and a note about `void` having no size would be a
    /// reason that is false there.
    ///
    /// Mutation: attach the note to every refusal with a pointer operand. The
    /// `p - v`, `v + v` and `n - v` rows gain one. Mutation: cite p2 for `-`.
    /// The `v - 1` row fails. Mutation: drop the note from
    /// `compound_assignment`. The `+=` and `-=` rows lose theirs. Mutation:
    /// drop the `+=` and `-=` test from `compound_assignment`'s note. `v *= 2`
    /// gains one, and `*=` refuses a pointer whatever it points to. Mutation:
    /// swap two of `unsteppable`'s reasons. The rows for those two fail.
    #[test]
    fn a_pointer_refused_for_what_it_points_to_says_why() {
        for (code, note) in [
            (
                "v + 1;",
                "a pointer steps by the size of what it points to, and `void` has no size (C17 6.5.6 p2)",
            ),
            (
                "v - 1;",
                "a pointer steps by the size of what it points to, and `void` has no size (C17 6.5.6 p3)",
            ),
            (
                "g + 1;",
                "a pointer steps by the size of what it points to, and a function is not an object (C17 6.5.6 p2)",
            ),
            (
                "u + 1;",
                "a pointer steps by the size of what it points to, and an array of unknown length has no size (C17 6.5.6 p2)",
            ),
            (
                "v += 1;",
                "a pointer steps by the size of what it points to, and `void` has no size (C17 6.5.16.2 p1)",
            ),
            (
                "u -= 1;",
                "a pointer steps by the size of what it points to, and an array of unknown length has no size (C17 6.5.16.2 p1)",
            ),
            ("p - v;", ""),
            ("v + v;", ""),
            ("n - v;", ""),
            ("v *= 2;", ""),
        ] {
            let checked = checked(&format!(
                "void g(void);
int main(void) {{
    int *p;
    void *v;
    int n;
    int (*u)[];
    {code}
    return 0;
}}
"
            ));

            assert_eq!(checked.codes(), ["SC0306"], "{code}");
            if note.is_empty() {
                assert_eq!(checked.notes(), Vec::<&str>::new(), "{code}");
            } else {
                assert_eq!(checked.notes(), [note], "{code}");
            }
        }
    }

    /// An initializer is held to the rule for a plain `=`, C17 6.7.9 p11, at
    /// file scope and in a block, with the words a declaration was written in.
    ///
    /// One program with the silences and the reports together, so that the
    /// silences are asserted against a run that does report and cannot pass by
    /// checking nothing. The silences are a null pointer constant, and a
    /// `void *` both ways, which is the implicit conversion RK-068 is about.
    ///
    /// Mutation: have `collect_receivers` skip an `Item::Declaration`. The
    /// file-scope report goes and this fails; nothing else in the suite
    /// declares at file scope with a mismatch. Mutation: pass `false` for
    /// `source_is_null` in `check_received`. `int *g = 0;` starts reporting
    /// and this fails. Mutation: give the initializer the `Return` arm's
    /// place label. The labels fail.
    #[test]
    fn an_initializer_in_either_scope_is_held_to_the_rule_for_assignment() {
        let checked = checked(
            "int h;
int *g = 0;
void *v = 0;
int *w = v;
int *q = h;
int main(void) {
    char c = 1;
    int *p = w;
    char *s = p;
    return 0;
}
",
        );

        assert_eq!(
            checked.messages(),
            [
                "cannot initialize `int *` with `int`",
                "cannot initialize `char *` with `int *`",
            ]
        );
        assert_eq!(
            checked.labels(),
            [
                "this is `int`",
                "this holds `int *`",
                "this is `int *`",
                "this holds `char *`",
            ]
        );
    }

    /// Every declarator of a declaration is checked, not only the first, and
    /// one with no initializer does not end the search.
    ///
    /// `m` is written first on purpose: it is the trivial value a list of one
    /// would pass every other test with, which is RK-076's shape.
    ///
    /// Mutation: have `initializers` read `declarators.iter().take(1)`.
    /// Mutation: have it `break` rather than `continue` at a declarator with
    /// no initializer. Either way `n` stops being checked and this fails.
    #[test]
    fn every_declarator_of_a_declaration_has_its_initializer_checked() {
        let checked = checked(
            "int main(void) { int *p = 0; int m, n = p; return 0; }
",
        );

        assert_eq!(checked.messages(), ["cannot initialize `int` with `int *`"]);
    }

    /// An initializer and a `return` are reported where they are written,
    /// among the other diagnostics, rather than after all of them.
    ///
    /// Mutation: run `check_received` in a loop of its own after the walk that
    /// works out the types. The assignment moves ahead of the initializer and
    /// this fails.
    #[test]
    fn an_initializer_and_a_return_are_reported_in_the_order_they_are_written() {
        let checked = checked(
            "int main(void) {
    int *p = 0;
    int n = p;
    n = p;
    return p;
}
",
        );

        assert_eq!(
            checked.messages(),
            [
                "cannot initialize `int` with `int *`",
                "cannot assign `int *` to `int`",
                "cannot return `int *` from a function returning `int`",
            ]
        );
    }

    /// Every place a `return` can be written, which is every place a statement
    /// can hold another.
    ///
    /// Mutation: drop any arm of `Checker::receivers_in` that recurses. The
    /// `return` under it stops being checked, the list is short by one, and
    /// this fails. Every initializer under that arm stops being checked with
    /// it, which is why the two are found by one walk: see `Receiving`.
    #[test]
    fn a_return_is_found_wherever_it_is_written() {
        let checked = checked(
            "int *g(void) {\n    if (1) return 1; else return 1;\n    while (1) return 1;\n    for (;;) return 1;\n    { return 1; }\n    return 1;\n}\n",
        );

        assert_eq!(
            checked.messages(),
            ["cannot return `int` from a function returning `int *`"; 6]
        );
    }

    /// A name nothing declares has no type, and an expression with no type is
    /// not reported on.
    ///
    /// Mutation: report when either side of an assignment is `None`. The
    /// undeclared name gains a second diagnostic about a type nobody knows and
    /// this fails. Mutation: have `additive` or `unary` answer `int` for an
    /// operand nothing typed. The rows with an operator in them gain the same
    /// second diagnostic, which is what they are here for: the bare name alone
    /// passed against a compiler that guessed.
    #[test]
    fn a_name_that_resolved_to_nothing_is_reported_once() {
        for value in ["nowhere", "nowhere + 1", "1 + nowhere", "-nowhere"] {
            let checked = checked(&format!(
                "int main(void) {{ int *p; p = {value}; return 0; }}\n"
            ));

            assert_eq!(
                checked.messages(),
                ["use of undeclared identifier `nowhere`"],
                "{value}"
            );
        }
    }
}
