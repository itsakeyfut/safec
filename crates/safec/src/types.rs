//! The type of every expression, and the three constraints C puts on one.
//!
//! The second half of the stage `docs/architecture.md` draws between the AST
//! and the typed AST. `sema.rs` says which declaration a name means; this says
//! what type each expression has, and reports an assignment, a `return` and a
//! call whose types C17 forbids.
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
    Ast, BinOp, Expr, ExprId, Item, Parameters, Stmt, StmtId, Type, TypeId, UnOp, spell_type,
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
const CONSTANT: Code = Code::new("SC0106");

/// A constant this compiler has no type for, C17 6.4.4 p2.
///
/// One code for a value too large and for a floating constant, because they
/// are one rule met two ways: the value has to be in the range of its type,
/// and the only types here are `int` and `char`. Same arrangement as
/// `MISMATCH` below, for the same reason.
const NO_TYPE: Code = Code::new("SC0305");

/// A value of the wrong type, C17 6.5.16.1 p1.
///
/// One code for an assignment and for a `return`, because 6.8.6.4 p3 makes
/// them one rule: a returned value is converted as if it were assigned to an
/// object of the return type. What differs is the message, which is the same
/// reason `parser.rs`'s `EXPECTED` is one code for every shape of syntax
/// error.
const MISMATCH: Code = Code::new("SC0302");

/// A call whose argument count is not the parameter count, C17 6.5.2.2 p2.
const ARGUMENTS: Code = Code::new("SC0303");

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

/// Give every expression a type, and report the three constraints.
///
/// Takes `&mut Ast` because two of the types it works out are types nobody
/// wrote: the `int` of an integer constant, and the pointer that `&x` has.
/// They go in the same arena as the rest, so that there is one representation
/// of a type in this compiler rather than two.
///
/// Takes `int_range` rather than a whole [`safec_ir::target::Target`] because
/// that is all it reads: whether an integer constant's value is in the range
/// of the one integer type this compiler has. ADR-0013 puts the target below
/// the frontend on the grounds that the lowering would otherwise need it to
/// build a type at all; no type is built from this one, so the same argument
/// is what limits it to a range.
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
        returns: HashMap::new(),
    };

    checker.collect_returns(ast);

    for id in ast.expr_ids().collect::<Vec<_>>() {
        let ty = checker.type_of(ast, id, diagnostics);
        checker.types[id.index()] = ty;
        checker.check_return(ast, id, diagnostics);
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
    /// The value of each `return` that has one, and the function it is in.
    ///
    /// Collected before the walk so that the walk stays in one order. A
    /// `return` is a statement and the types are worked out over the arena, so
    /// without this the returns would be reported after every expression
    /// rather than among them, and a reader would find them out of order.
    returns: HashMap<ExprId, Returning>,
}

impl Checker<'_> {
    /// Find every `return` with a value, and what it has to be assignable to.
    fn collect_returns(&mut self, ast: &Ast) {
        for item in ast.items() {
            let Item::Function(function) = item else {
                continue;
            };
            let Type::Function { returns, .. } = ast.ty(function.ty) else {
                // A definition whose declarator derived something else is a
                // constraint violation of 6.9.1 p2 that nothing reports yet.
                continue;
            };

            self.returns_in(
                ast,
                function.body,
                Returning {
                    ty: *returns,
                    name: function.name,
                },
            );
        }
    }

    /// Every `return` under one statement.
    ///
    /// A recursion, bounded by `parser::MAX_NESTING` the way every statement
    /// walk here is, and exhaustive so that a statement kind that can hold
    /// another has to be answered for rather than silently dropping the
    /// returns inside it.
    fn returns_in(&mut self, ast: &Ast, id: StmtId, returning: Returning) {
        match ast.stmt(id) {
            Stmt::Return { value, .. } => {
                if let Some(value) = *value {
                    self.returns.insert(value, returning);
                }
            }
            Stmt::Compound { body, .. } => {
                for &statement in body {
                    self.returns_in(ast, statement, returning);
                }
            }
            Stmt::If {
                then, otherwise, ..
            } => {
                self.returns_in(ast, *then, returning);
                if let Some(otherwise) = *otherwise {
                    self.returns_in(ast, otherwise, returning);
                }
            }
            Stmt::While { body, .. } | Stmt::For { body, .. } => {
                self.returns_in(ast, *body, returning);
            }
            Stmt::Declaration { .. } | Stmt::Expression { .. } | Stmt::Error { .. } => {}
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
            Expr::Binary { op, lhs, rhs, .. } => self.binary(ast, op, lhs, rhs),
            Expr::Assign {
                op, place, value, ..
            } => {
                // Only a plain `=`. A compound assignment is 6.5.16.2, whose
                // constraints are its own: `p += 1` is legal for a pointer and
                // would be reported by the rule below.
                if op.is_none() {
                    self.assignment(ast, place, value, diagnostics);
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

    fn binary(&mut self, ast: &mut Ast, op: BinOp, lhs: ExprId, rhs: ExprId) -> Option<TypeId> {
        match op {
            BinOp::Add | BinOp::Sub => self.additive(ast, op, lhs, rhs),
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
    /// result's.
    ///
    /// An operand this stage could not type makes the result one it cannot
    /// type either. Everything below turns on whether an operand is a
    /// pointer, and answering `int` for a type nobody knows is how a report
    /// about a program nobody understood reaches a user: before this said
    /// `None`, `p = (1 ? p : v) + 1` was rejected on the strength of a guess.
    fn additive(&mut self, ast: &mut Ast, op: BinOp, lhs: ExprId, rhs: ExprId) -> Option<TypeId> {
        let lhs = self.types[lhs.index()]?;
        let rhs = self.types[rhs.index()]?;
        let lhs = self.decayed(ast, lhs);
        let rhs = self.decayed(ast, rhs);

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

    /// The type an operand has after the conversions C17 6.3.2.1 makes.
    ///
    /// p3 turns an array into a pointer to its first element and p4 turns a
    /// function into a pointer to itself, before any operator sees either. A
    /// stage that skips this reads `a + 1` as arithmetic on something that is
    /// not a pointer and answers `int`, and `p = a + 1` is then a diagnostic
    /// about ordinary C. The pointer it makes is a type nobody wrote, so it is
    /// pushed.
    ///
    /// Only [`Checker::additive`] asks. `assignable` deliberately does not:
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

    /// C17 6.8.6.4 p3, which is 6.5.16.1 p1 with the return type as the place.
    fn check_return(&mut self, ast: &Ast, value: ExprId, diagnostics: &mut DiagnosticSink) {
        let Some(returning) = self.returns.get(&value).copied() else {
            return;
        };
        let Some(source) = self.types[value.index()] else {
            return;
        };

        if self.assignable(
            ast,
            returning.ty,
            source,
            self.is_null_pointer_constant(value),
        ) != Some(false)
        {
            return;
        }

        diagnostics.report(
            Diagnostic::error(format!(
                "cannot return `{}` from a function returning `{}`",
                self.spelled(ast, source),
                self.spelled(ast, returning.ty)
            ))
            .with_code(MISMATCH)
            .with_label(Label::primary(
                ast.expr(value).span(),
                format!("this is `{}`", self.spelled(ast, source)),
            ))
            .with_label(Label::secondary(
                returning.name,
                format!("declared to return `{}`", self.spelled(ast, returning.ty)),
            )),
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
    /// Both at once, because `read` answers both from one pass over the
    /// spelling and because a constant with no type has no value either.
    ///
    /// The type is `int` whenever the value fits one. 6.4.4.1 p5 asks for the
    /// first type in a list that holds it, and this compiler has no other
    /// integer type to offer: there is no `unsigned int` and no `long` in
    /// [`Type`] or in [`Integer`], so a suffix is validated and then
    /// discarded. `docs/frontend.md` records that divergence and what it
    /// costs, which is that `4294967295u` and `2147483648` are refused here
    /// and compiled by `clang`.
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
        // `Read` is matched exhaustively so that a fifth answer has to be
        // given a report rather than falling into one written for another.
        let reported = match read(self.sources.snippet(span)) {
            Read::Integer(value) => {
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
            Read::TooLarge => TOO_LARGE,
            Read::Floating => FLOATING,
            Read::Malformed(why) => Reported {
                code: CONSTANT,
                message: "this is not a constant C allows",
                label: why,
                note: "C17 6.4.4.1 p1 gives the three bases their digits and lists the suffixes",
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

/// What reading the text of a numeric constant comes to, C17 6.4.4.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Read {
    /// The value, with no type attached. Which type C would give it is
    /// 6.4.4.1 p5's table, and this compiler has only `int` to offer.
    Integer(u128),
    /// More than a `u128` holds, which no type here could.
    TooLarge,
    /// A well-formed floating constant, C17 6.4.4.2.
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
/// Everything else: a digit of no base, or a suffix C does not have.
const NOT_A_CONSTANT: &str =
    "this is not a digit of the constant's base, and not a suffix C allows";

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
fn read(text: &str) -> Read {
    // 6.4.4.2 p1 gives a floating constant a `.`, or an exponent: `e`/`E` for
    // a decimal and `p`/`P` for a hexadecimal. The base has to be known before
    // the marker can be looked for, because `e` is a hexadecimal digit and
    // `0xe1` is 225.
    let hex = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X"));
    let exponent = if hex.is_some() {
        ['p', 'P']
    } else {
        ['e', 'E']
    };
    if text.contains('.') || hex.unwrap_or(text).contains(exponent) {
        return Read::Floating;
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
        return Read::Malformed(if radix == 8 && suffix.starts_with(['8', '9']) {
            NOT_OCTAL
        } else {
            NOT_A_CONSTANT
        });
    }

    if digits.is_empty() {
        // `0` alone, and `0u`: an octal constant whose digits are just the
        // leading zero. `0x` alone has nothing after its prefix and is not a
        // constant at all.
        return if radix == 8 {
            Read::Integer(0)
        } else {
            Read::Malformed(NO_DIGITS)
        };
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
            None => return Read::TooLarge,
        };
    }

    Read::Integer(value)
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
    }

    /// One program per spelling, so that a row is about one constant.
    ///
    /// `return` and not a declaration, because an initializer is not checked
    /// against the assignment constraint yet (#205) and a row that reported
    /// twice would be reporting about that rather than about the constant.
    fn returning(spelling: &str) -> Checked {
        checked(&format!("int f(void) {{\n    return {spelling};\n}}\n"))
    }

    /// An integer constant is worth what its base and its suffix say,
    /// C17 6.4.4.1 p1.
    ///
    /// The values are written out rather than computed from the spellings,
    /// which is what RK-001 asks: a table that works the expectation out the
    /// way the code does agrees with the code however wrong both are.
    ///
    /// Mutation: give the hexadecimal arm radix ten. `0x10` is ten and this
    /// fails. Mutation: give the octal arm radix ten. `010` is ten and this
    /// fails, along with most of the suite, because `0` stops being readable
    /// too.
    #[test]
    fn an_integer_constant_is_worth_what_its_base_and_suffix_say() {
        for (spelling, value) in [
            ("0", 0),
            ("16", 16),
            ("010", 8),
            ("0777", 511),
            ("0x10", 16),
            ("0X10u", 16),
            ("1u", 1),
            ("1ul", 1),
            ("1llu", 1),
            ("0L", 0),
        ] {
            let checked = returning(spelling);
            assert_eq!(checked.messages(), Vec::<&str>::new(), "{spelling}");
            assert_eq!(checked.value_of(spelling), Some(value), "{spelling}");
            assert_eq!(checked.spelling(spelling), "int", "{spelling}");
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
    /// both arms the same markers. `0xe1` is reported as a floating constant,
    /// this fails, and nothing else in the suite does.
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
    /// The first two are constants `clang 20.1.6 -std=c17 -pedantic-errors
    /// --target=x86_64-pc-windows-msvc` accepts, which is why the message
    /// blames this compiler rather than the program: what is missing is
    /// `long`, not a well-formed constant. `docs/frontend.md` carries the
    /// divergence.
    ///
    /// **The fourth is `u128::MAX` written out, and it is here for one line.**
    /// `read` can return it, and a `u128` that large converted with `as`
    /// rather than asked whether it fits an `i128` is -1, which `int` holds.
    /// So the constant would be accepted, silently, as minus one: the same
    /// shape as `010` lowering to ten, which is a plausible wrong number
    /// rather than a refusal. Nothing else in the suite reaches that line,
    /// because every other oversized spelling overflows the accumulation
    /// first and is `Read::TooLarge` before any conversion happens.
    ///
    /// Mutation: answer `true` from `Integer::holds`. The first two are
    /// accepted with a value `int` does not hold and this fails. Mutation:
    /// answer zero rather than `Read::TooLarge` on an overflow. The forty
    /// nines become zero, nothing is reported, and this fails. Mutation:
    /// replace `i128::try_from(value).ok()` with `Some(value as i128)`. The
    /// fourth becomes `Constant -1` with nothing reported and this fails.
    #[test]
    fn a_constant_no_type_here_can_hold_is_reported_and_has_no_value() {
        for spelling in [
            "2147483648",
            "4294967295u",
            "9999999999999999999999999999999999999999",
            "340282366920938463463374607431768211455",
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
    /// fails.
    #[test]
    fn a_floating_constant_is_reported_as_one_this_compiler_does_not_read_yet() {
        for spelling in ["1.5", "0.0", "1e5"] {
            let checked = returning(spelling);
            assert_eq!(
                checked.messages(),
                ["this compiler does not read floating constants yet"],
                "{spelling}"
            );
            assert_eq!(checked.codes(), ["SC0305"], "{spelling}");
            assert_eq!(checked.value_of(spelling), None, "{spelling}");
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
    /// this fails, and nothing else in the suite does, which is what says the
    /// label is guarded and not only the code.
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
            assert_eq!(
                checked
                    .diagnostics
                    .diagnostics()
                    .iter()
                    .flat_map(Diagnostic::labels)
                    .map(Label::message)
                    .collect::<Vec<_>>(),
                [label],
                "{spelling}"
            );
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

        assert_eq!(checked.messages(), Vec::<&str>::new());

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
    /// Mutation: have `is_null_pointer_constant` answer `false` always. The
    /// first program starts reporting and this fails. Mutation: have it answer
    /// `true` always. The second stops and this fails.
    #[test]
    fn a_pointer_takes_a_zero_and_not_a_one() {
        for zero in ["0", "0x0", "0X0", "00", "0u", "0L", "0UL"] {
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
    /// Mutation: check every `Assign` rather than only the plain one. This
    /// fails with a diagnostic about correct C.
    #[test]
    fn a_compound_assignment_is_not_the_rule_for_a_plain_one() {
        let checked = checked("int main(void) { int *p; p += 1; return 0; }\n");

        assert_eq!(checked.messages(), Vec::<&str>::new());
    }

    /// Every place a `return` can be written, which is every place a statement
    /// can hold another.
    ///
    /// Mutation: drop any arm of `Checker::returns_in` that recurses. The
    /// `return` under it stops being checked, the list is short by one, and
    /// this fails.
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
