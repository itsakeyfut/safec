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
use crate::source::{SourceMap, Span};

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
}

/// Give every expression a type, and report the three constraints.
///
/// Takes `&mut Ast` because two of the types it works out are types nobody
/// wrote: the `int` of an integer constant, and the pointer that `&x` has.
/// They go in the same arena as the rest, so that there is one representation
/// of a type in this compiler rather than two.
pub fn check(
    sources: &SourceMap,
    ast: &mut Ast,
    resolution: &Resolution,
    diagnostics: &mut DiagnosticSink,
) -> Types {
    let mut checker = Checker {
        sources,
        resolution,
        types: vec![None; ast.expr_ids().count()],
        // Pushed once. A new node per constant would fill the arena with
        // copies of `int` and make nothing truer.
        int: ast.push_type(Type::Int),
        returns: HashMap::new(),
    };

    checker.collect_returns(ast);

    for id in ast.expr_ids().collect::<Vec<_>>() {
        let ty = checker.type_of(ast, id, diagnostics);
        checker.types[id.index()] = ty;
        checker.check_return(ast, id, diagnostics);
    }

    Types { of: checker.types }
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
    int: TypeId,
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
            Stmt::Declaration(_) | Stmt::Expression { .. } | Stmt::Error { .. } => {}
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
            // C17 6.4.4.1 p5 gives an integer constant the first type in a
            // list that fits it. Nothing here reads a suffix or knows a
            // target's widths, and `int` is the first of that list, so it is
            // the answer for every constant this compiler can read.
            Expr::Number { .. } => Some(self.int),
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
                ast.same_type(then, otherwise).then_some(then)
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
            // write, and p5 makes `!` an `int` outright.
            UnOp::Plus | UnOp::Minus | UnOp::Not | UnOp::BitNot => Some(self.int),
        }
    }

    fn binary(&mut self, ast: &Ast, op: BinOp, lhs: ExprId, rhs: ExprId) -> Option<TypeId> {
        let lhs = self.types[lhs.index()];
        let rhs = self.types[rhs.index()];

        match op {
            // 6.5.6 p8: a pointer plus an integer is that pointer's type. Not
            // answering this would type `p + 1` as `int` and make `q = p + 1`
            // a diagnostic about correct C.
            BinOp::Add | BinOp::Sub => {
                let pointer = [lhs, rhs]
                    .into_iter()
                    .flatten()
                    .find(|&ty| matches!(ast.ty(ty), Type::Pointer(_)));

                match pointer {
                    // 6.5.6 p9 makes a pointer minus a pointer `ptrdiff_t`,
                    // which this compiler has no name for, so it is not
                    // answered.
                    Some(_) if op == BinOp::Sub && lhs.is_some() && rhs.is_some() => {
                        let both = [lhs, rhs]
                            .into_iter()
                            .flatten()
                            .filter(|&ty| matches!(ast.ty(ty), Type::Pointer(_)))
                            .count();
                        if both == 2 { None } else { pointer }
                    }
                    Some(pointer) => Some(pointer),
                    None => Some(self.int),
                }
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

        if self.assignable(
            ast,
            target,
            source,
            self.is_null_pointer_constant(ast, value),
        ) != Some(false)
        {
            return;
        }

        diagnostics.report(
            Diagnostic::error(format!(
                "cannot assign `{}` to `{}`",
                spell_type(self.sources, ast, source),
                spell_type(self.sources, ast, target)
            ))
            .with_code(MISMATCH)
            .with_label(Label::primary(
                ast.expr(value).span(),
                format!("this is `{}`", spell_type(self.sources, ast, source)),
            ))
            .with_label(Label::secondary(
                ast.expr(place).span(),
                format!("this holds `{}`", spell_type(self.sources, ast, target)),
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
            self.is_null_pointer_constant(ast, value),
        ) != Some(false)
        {
            return;
        }

        diagnostics.report(
            Diagnostic::error(format!(
                "cannot return `{}` from a function returning `{}`",
                spell_type(self.sources, ast, source),
                spell_type(self.sources, ast, returning.ty)
            ))
            .with_code(MISMATCH)
            .with_label(Label::primary(
                ast.expr(value).span(),
                format!("this is `{}`", spell_type(self.sources, ast, source)),
            ))
            .with_label(Label::secondary(
                returning.name,
                format!(
                    "declared to return `{}`",
                    spell_type(self.sources, ast, returning.ty)
                ),
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
                ast.same_type(target, source)
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
    fn is_null_pointer_constant(&self, ast: &Ast, value: ExprId) -> bool {
        let Expr::Number { span } = ast.expr(value) else {
            return false;
        };

        let text = self.sources.snippet(*span);
        let digits = text.trim_end_matches(['u', 'U', 'l', 'L']);
        let digits = digits
            .strip_prefix("0x")
            .or_else(|| digits.strip_prefix("0X"))
            .unwrap_or(digits);

        !digits.is_empty() && digits.bytes().all(|digit| digit == b'0')
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::spell_type;
    use crate::lexer::lex;
    use crate::parser::parse;
    use crate::sema::resolve;

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
        let types = check(&sources, &mut ast, &resolution, &mut diagnostics);

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

        fn messages(&self) -> Vec<&str> {
            self.diagnostics
                .diagnostics()
                .iter()
                .map(Diagnostic::message)
                .collect()
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
            "int f(int a) { return a; }\n\nint main(void) {\n    int x;\n    int *p;\n    int q[3];\n    p = &x;\n    q[0] = *p + f(2) + x++;\n    p = p + 1;\n    return 0;\n}\n",
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
            ("p + 1", "int *"),
            ("p = p + 1", "int *"),
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
        for zero in ["0", "0x0", "00"] {
            let checked = checked(&format!(
                "int main(void) {{ int *p; p = {zero}; return 0; }}\n"
            ));
            assert_eq!(checked.messages(), Vec::<&str>::new(), "{zero}");
        }

        let checked = checked("int main(void) { int *p; p = 1; return 0; }\n");
        assert_eq!(checked.messages(), ["cannot assign `int` to `int *`"]);
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
    /// Mutation: drop any arm of `Resolver::returns_in` that recurses. The
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
    /// this fails.
    #[test]
    fn a_name_that_resolved_to_nothing_is_reported_once() {
        let checked = checked("int main(void) { int *p; p = nowhere; return 0; }\n");

        assert_eq!(
            checked.messages(),
            ["use of undeclared identifier `nowhere`"]
        );
    }
}
